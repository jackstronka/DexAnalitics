//! Phase-2 decision layer (API-first): optional gate → `POST /api/v1/backtests/data-readiness`
//! (aligned with `pool_ids` / `snapshot_variants` from the FULL request file) → `POST /backtests/full` → poll → audit row.

use anyhow::{Context, Result, bail};
use chrono::Utc;
use reqwest::Client;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Trim trailing slash for joining `/api/v1/...`.
pub fn normalize_api_base(raw: &str) -> String {
    raw.trim().trim_end_matches('/').to_string()
}

fn api_v1_url(base: &str, tail: &str) -> String {
    let b = normalize_api_base(base);
    format!("{b}/api/v1{tail}")
}

fn count_metric_rows(job: &Value) -> usize {
    let Some(results) = job.get("results").and_then(|r| r.as_array()) else {
        return 0;
    };
    results
        .iter()
        .filter_map(|w| w.get("metrics").and_then(|m| m.as_array()))
        .map(|m| m.len())
        .sum()
}

fn stderr_preview(job: &Value, max: usize) -> Option<Value> {
    let s = job.get("stderr").and_then(|v| v.as_str())?;
    if s.len() <= max {
        return Some(Value::String(s.to_string()));
    }
    Some(Value::String(format!("{}…(truncated)", &s[..max])))
}

fn data_readiness_body_from_full_request(full: &Value) -> Value {
    let mut o = json!({});
    if let Some(p) = full.get("pool_ids") {
        o["pool_ids"] = p.clone();
    }
    if let Some(v) = full.get("snapshot_variants") {
        o["snapshot_variants"] = v.clone();
    }
    o
}

/// Run optional curated gate, API data-readiness, `POST /backtests/full` → poll → audit row.
#[allow(clippy::too_many_arguments)]
pub async fn run_backtests_full_via_api(
    api_base: &str,
    request_json_path: &Path,
    skip_gate: bool,
    fail_on_gate_no_go: bool,
    gate_max_age_minutes: i64,
    gate_min_decode_ok_pct: f64,
    skip_data_readiness: bool,
    fail_on_data_readiness: bool,
    poll_interval: Duration,
    poll_timeout: Duration,
    save_job_json: Option<&Path>,
    decisions_via_http: bool,
    jsonl_path: Option<&Path>,
    source: &str,
    chain_id: Option<&str>,
    fail_on_job_failed: bool,
    fail_on_job_partial: bool,
    decision_include_full_job: bool,
) -> Result<()> {
    let mut gate_summary: Option<crate::swap_sync::HealthCheckCuratedSummary> = None;
    if !skip_gate {
        let summary = crate::swap_sync::health_check_curated_all_collect(
            gate_max_age_minutes,
            gate_min_decode_ok_pct,
        )?;
        if !summary.alerts.is_empty() && fail_on_gate_no_go {
            bail!(
                "orchestrator-backtests-full: gate reported {} alert(s); refusing POST /backtests/full (run orchestrator-gate or fix data)",
                summary.alerts.len()
            );
        }
        gate_summary = Some(summary);
    }

    let req_text = std::fs::read_to_string(request_json_path)
        .with_context(|| format!("read request json {}", request_json_path.display()))?;
    let request_body: Value =
        serde_json::from_str(&req_text).context("parse BacktestFullRequest JSON")?;

    let client = Client::builder()
        .user_agent("clmm-lp-cli/orchestrator-backtests-full")
        .build()
        .context("build HTTP client")?;

    let mut data_readiness_response: Option<Value> = None;
    if !skip_data_readiness {
        let readiness_url = api_v1_url(api_base, "/backtests/data-readiness");
        let readiness_body = data_readiness_body_from_full_request(&request_body);
        let dr = client
            .post(&readiness_url)
            .json(&readiness_body)
            .send()
            .await
            .with_context(|| format!("POST {readiness_url}"))?;
        let dr_status = dr.status();
        if !dr_status.is_success() {
            let body = dr.text().await.unwrap_or_default();
            bail!(
                "POST /backtests/data-readiness failed HTTP {}: {}",
                dr_status,
                body
            );
        }
        let dr_json: Value = dr
            .json()
            .await
            .context("decode POST /backtests/data-readiness JSON")?;
        let agg_status = dr_json
            .pointer("/aggregate/status")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if fail_on_data_readiness && agg_status != "ok" {
            bail!(
                "orchestrator-backtests-full: data-readiness aggregate.status={agg_status:?} (not ok); refusing POST /backtests/full (fix snapshots or use --skip-data-readiness / --fail-on-data-readiness=false)"
            );
        }
        println!(
            "📊 orchestrator-backtests-full: data-readiness aggregate.status={}",
            if agg_status.is_empty() {
                "(empty)"
            } else {
                agg_status
            }
        );
        data_readiness_response = Some(dr_json);
    }

    let post_url = api_v1_url(api_base, "/backtests/full");
    let start = client
        .post(&post_url)
        .json(&request_body)
        .send()
        .await
        .with_context(|| format!("POST {post_url}"))?;
    let start_status = start.status();
    if !start_status.is_success() {
        let body = start.text().await.unwrap_or_default();
        bail!(
            "POST /backtests/full failed HTTP {}: {}",
            start_status,
            body
        );
    }
    let start_json: Value = start
        .json()
        .await
        .context("decode POST /backtests/full JSON")?;
    let job_id = start_json["id"]
        .as_str()
        .context("POST /backtests/full: missing string field \"id\"")?
        .to_string();

    let get_url = api_v1_url(api_base, &format!("/backtests/full/{job_id}"));
    let deadline = Instant::now() + poll_timeout;
    let mut job: Value = json!({});
    loop {
        if Instant::now() > deadline {
            bail!(
                "poll timeout after {:?} waiting for job {} (last GET {})",
                poll_timeout,
                job_id,
                get_url
            );
        }
        let r = client
            .get(&get_url)
            .send()
            .await
            .with_context(|| format!("GET {get_url}"))?;
        let st = r.status();
        if !st.is_success() {
            let body = r.text().await.unwrap_or_default();
            bail!("GET {} failed HTTP {}: {}", get_url, st, body);
        }
        job = r.json().await.context("decode GET job JSON")?;
        let status = job["status"].as_str().unwrap_or("");
        if status != "running" {
            break;
        }
        tokio::time::sleep(poll_interval).await;
    }

    if let Some(out) = save_job_json {
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create_dir_all {}", parent.display()))?;
        }
        std::fs::write(out, serde_json::to_string_pretty(&job)?)
            .with_context(|| format!("write {}", out.display()))?;
        println!(
            "📝 orchestrator-backtests-full: wrote full job JSON to {}",
            out.display()
        );
    }

    let job_status = job["status"].as_str().unwrap_or("unknown").to_string();
    let metric_rows = count_metric_rows(&job);
    let window_count = job
        .get("results")
        .and_then(|r| r.as_array())
        .map(|a| a.len())
        .unwrap_or(0);

    let outcome = match job_status.as_str() {
        "succeeded" => "ok",
        "partial" => {
            if fail_on_job_partial {
                "no_go"
            } else {
                "ok"
            }
        }
        "failed" => "no_go",
        _ => "no_go",
    };

    let no_go_reason: Option<String> = match job_status.as_str() {
        "failed" => Some(format!(
            "backtests/full job status=failed; see stderr / {}",
            save_job_json
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "GET response".into())
        )),
        "partial" if fail_on_job_partial => {
            Some("backtests/full job status=partial (some pool/window errors); see stderr".into())
        }
        s if s != "succeeded" && s != "partial" => Some(format!("unexpected terminal status: {s}")),
        _ => None,
    };

    let run_id = format!("full-api-{}", uuid::Uuid::new_v4());
    let mut data_quality = json!({
        "job_id": job_id,
        "job_status": job_status,
        "metric_rows": metric_rows,
        "window_count": window_count,
        "stderr_preview": stderr_preview(&job, 8000),
        "finished_ts_utc": job.get("finished_ts_utc").cloned(),
    });
    if let Some(gs) = gate_summary.as_ref() {
        data_quality["gate"] = json!({
            "alert_count": gs.alerts.len(),
            "alerts": &gs.alerts,
            "health_report_path": &gs.report_path,
            "summary_ts_utc": &gs.ts_utc,
        });
    }
    if let Some(dr) = data_readiness_response.as_ref() {
        data_quality["data_readiness"] = dr.clone();
    }

    let mut tools = vec![
        "POST /api/v1/backtests/data-readiness".to_string(),
        "POST /api/v1/backtests/full".to_string(),
        "GET /api/v1/backtests/full/{id}".to_string(),
    ];
    if skip_data_readiness {
        tools.retain(|t| *t != "POST /api/v1/backtests/data-readiness");
    }

    let mut decision_payload = json!({
        "schema_version": 1,
        "run_id": run_id,
        "kind": "api_backtests_full",
        "tools_invoked": tools,
        "data_quality": data_quality,
        "outcome": outcome,
        "no_go_reason": no_go_reason,
        "inputs": request_body,
        "inputs_ref": {
            "schema_version": 1,
            "role": "api_backtests_full_request",
            "request_json_path": request_json_path.to_string_lossy(),
            "api_base": normalize_api_base(api_base),
        },
    });
    if decision_include_full_job
        && let Some(m) = decision_payload.as_object_mut()
    {
        m.insert("job".to_string(), job.clone());
    }

    if decisions_via_http {
        let dec_url = api_v1_url(api_base, "/data/agent/decisions");
        let mut post_body = json!({
            "source": source,
            "decision": decision_payload,
        });
        if let Some(c) = chain_id {
            post_body["chain_id"] = json!(c);
        }
        let post_dec = client
            .post(&dec_url)
            .json(&post_body)
            .send()
            .await
            .with_context(|| format!("POST {dec_url}"))?;
        let dec_status = post_dec.status();
        if !dec_status.is_success() {
            let body = post_dec.text().await.unwrap_or_default();
            bail!(
                "POST /data/agent/decisions failed HTTP {}: {}",
                dec_status,
                body
            );
        }
        println!(
            "🧭 orchestrator-backtests-full: logged via API {} (outcome={})",
            dec_url, outcome
        );
    } else {
        let mut row = json!({
            "ts_utc": Utc::now().to_rfc3339(),
            "source": source,
            "decision": decision_payload,
        });
        if let Some(c) = chain_id {
            row["chain_id"] = json!(c);
        }
        let path = jsonl_path
            .map(PathBuf::from)
            .unwrap_or_else(crate::orchestrator_gate::default_agent_decisions_path);
        crate::orchestrator_gate::append_agent_decision_row(&path, &row)
            .with_context(|| format!("append {}", path.display()))?;
        println!(
            "🧭 orchestrator-backtests-full: appended row to {} (outcome={})",
            path.display(),
            outcome
        );
    }

    if fail_on_job_failed && job_status == "failed" {
        bail!("orchestrator-backtests-full: job status=failed (see stderr / saved JSON)");
    }
    if fail_on_job_failed && job_status != "succeeded" && job_status != "partial" {
        bail!(
            "orchestrator-backtests-full: unexpected job status={job_status} (--fail-on-job-failed)"
        );
    }
    if fail_on_job_partial && job_status == "partial" {
        bail!("orchestrator-backtests-full: job status=partial (--fail-on-job-partial)");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn data_readiness_body_copies_pool_and_variant_fields() {
        let full = json!({
            "windows_hours": [24],
            "pool_ids": ["A"],
            "snapshot_variants": ["5m"],
            "capital_usd": 1.0
        });
        let b = data_readiness_body_from_full_request(&full);
        assert_eq!(b["pool_ids"], json!(["A"]));
        assert_eq!(b["snapshot_variants"], json!(["5m"]));
        assert!(b.get("windows_hours").is_none());
    }

    #[test]
    fn normalize_api_base_trims_slash() {
        assert_eq!(
            normalize_api_base("http://127.0.0.1:8081/"),
            "http://127.0.0.1:8081"
        );
    }
}
