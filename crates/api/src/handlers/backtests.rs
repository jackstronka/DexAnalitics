//! Backtest endpoints (spawn CLI backtest runs).

use crate::error::{ApiError, ApiResult};
use crate::models::{
    BacktestFromClosedPositionRequest, BacktestJobResponse, BacktestJobStatusResponse,
};
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::Json;
use clmm_lp_protocols::ledger::position_registry::registry_path;
use clmm_lp_domain::math::price_tick::tick_to_price;
use crate::services::price_fetch::fetch_mint_prices_usd;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use serde_json::Value;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::process::Stdio;
use std::sync::LazyLock;
use tokio::process::Command;
use tokio::sync::RwLock;
use uuid::Uuid;

static JOBS: LazyLock<RwLock<HashMap<String, BacktestJobResponse>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

fn registry_lookup(position_pubkey: &str) -> Option<(Option<String>, Option<String>, String, Option<String>, Option<Value>)> {
    let p = registry_path();
    let f = std::fs::File::open(p).ok()?;
    let r = BufReader::new(f);

    let mut opened_ts: Option<String> = None;
    let mut closed_ts: Option<String> = None;
    let mut pool: Option<String> = None;
    let mut sid: Option<String> = None;
    let mut details: Option<Value> = None;

    for line in r.lines().filter_map(Result::ok) {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(t) else {
            continue;
        };
        let ev = v.get("event").and_then(|x| x.as_str()).unwrap_or("").trim();
        if ev != "registry_open" && ev != "registry_close" {
            continue;
        }
        let pos = v
            .get("position_pubkey")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .trim();
        if pos != position_pubkey {
            continue;
        }
        pool = v
            .get("pool_address")
            .and_then(|x| x.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or(pool);
        sid = v
            .get("rebalance_session_id")
            .and_then(|x| x.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or(sid);
        details = v.get("details").cloned().or(details);
        let ts = v.get("ts_utc").and_then(|x| x.as_str()).map(|s| s.trim().to_string());
        match ev {
            "registry_open" => opened_ts = ts.or(opened_ts),
            "registry_close" => closed_ts = ts.or(closed_ts),
            _ => {}
        }
    }

    let pool = pool?;
    Some((opened_ts, closed_ts, pool, sid, details))
}

fn iso_date(ts_utc: &str) -> Option<String> {
    let t = ts_utc.trim();
    if t.len() >= 10 {
        return Some(t[..10].to_string());
    }
    None
}

/// Spawn a historical backtest for a closed position (best-effort) using CLI `clmm-lp-cli backtest`.
#[utoipa::path(
    post,
    path = "/backtests/from-closed-position",
    tag = "Analytics",
    request_body = BacktestFromClosedPositionRequest,
    responses(
        (status = 200, description = "Backtest job started", body = BacktestJobStatusResponse),
        (status = 400, description = "Invalid request")
    )
)]
pub async fn backtest_from_closed_position(
    State(state): State<AppState>,
    Json(req): Json<BacktestFromClosedPositionRequest>,
) -> ApiResult<Json<BacktestJobStatusResponse>> {
    let position = req.position_address.trim();
    if position.is_empty() {
        return Err(ApiError::bad_request("position_address is required"));
    }

    let (opened_ts, closed_ts, pool_address, open_sid, details) = registry_lookup(position).ok_or_else(|| {
        ApiError::not_found("Position not found in registry (missing open/close rows)")
    })?;

    // Resolve dates (best-effort). End date is exclusive in CLI; we still pass close date (may undercount partial day).
    let start_date = req
        .start_date
        .clone()
        .or_else(|| opened_ts.as_deref().and_then(iso_date));
    let end_date = req
        .end_date
        .clone()
        .or_else(|| closed_ts.as_deref().and_then(iso_date));

    // Derive range bounds from open ticks stored in registry details (preferred).
    let (mut lower, mut upper) = (req.lower, req.upper);
    if lower.is_none() || upper.is_none() {
        let tick_lower = details
            .as_ref()
            .and_then(|d| d.get("tick_lower"))
            .and_then(|x| x.as_i64())
            .map(|n| n as i32);
        let tick_upper = details
            .as_ref()
            .and_then(|d| d.get("tick_upper"))
            .and_then(|x| x.as_i64())
            .map(|n| n as i32);
        if let (Some(tl), Some(tu)) = (tick_lower, tick_upper) {
            let lp = tick_to_price(tl).ok().and_then(|d| d.to_f64());
            let up = tick_to_price(tu).ok().and_then(|d| d.to_f64());
            if lower.is_none() {
                lower = lp;
            }
            if upper.is_none() {
                upper = up;
            }
        }
    }
    let lower = lower.unwrap_or(0.0);
    let upper = upper.unwrap_or(0.0);
    if lower <= 0.0 || upper <= 0.0 || lower >= upper {
        return Err(ApiError::bad_request(
            "Could not derive lower/upper from registry details; provide lower/upper overrides",
        ));
    }

    // Derive capital from open-session fee payer token deltas (best-effort).
    let capital = if let Some(c) = req.capital {
        c.max(0.0)
    } else if let Some(sid) = open_sid.as_deref() {
        let ledger = clmm_lp_protocols::ledger::tx_lifecycle::ledger_read_path();
        if !ledger.exists() {
            0.0
        } else {
            let txt = std::fs::read_to_string(&ledger).unwrap_or_default();
            let mut mint_deltas: HashMap<String, Decimal> = HashMap::new();
            for line in txt.lines() {
                let t = line.trim();
                if t.is_empty() {
                    continue;
                }
                let Ok(v) = serde_json::from_str::<Value>(t) else {
                    continue;
                };
                if v.get("rebalance_session_id").and_then(|x| x.as_str()).map(str::trim)
                    != Some(sid)
                {
                    continue;
                }
                let Some(obj) = v.get("fee_payer_token_deltas").and_then(|x| x.as_object()) else {
                    continue;
                };
                for (mint, dv) in obj {
                    if let Some(s) = dv.as_str() {
                        if let Ok(d) = Decimal::from_str_exact(s.trim()) {
                            *mint_deltas.entry(mint.clone()).or_insert(Decimal::ZERO) += d;
                        }
                    }
                }
            }

            // Convert only pool leg mints (A and B) to USD at current free prices.
            let pool_state = clmm_lp_protocols::prelude::WhirlpoolReader::new(state.provider.clone())
                .get_pool_state(&pool_address)
                .await
                .map_err(|e| ApiError::internal(format!("get_pool_state failed: {e}")))?;
            let mint_a = pool_state.token_mint_a.to_string();
            let mint_b = pool_state.token_mint_b.to_string();
            let mut mints = std::collections::BTreeSet::new();
            mints.insert(mint_a.clone());
            mints.insert(mint_b.clone());
            let (px, _src) = fetch_mint_prices_usd(&mints).await;
            let pa = px.get(&mint_a).copied().unwrap_or(0.0);
            let pb = px.get(&mint_b).copied().unwrap_or(0.0);
            let da = mint_deltas.get(&mint_a).cloned().unwrap_or(Decimal::ZERO);
            let db = mint_deltas.get(&mint_b).cloned().unwrap_or(Decimal::ZERO);
            // For capital, we want the USD value *spent* to open: negative deltas → spend.
            let spend_a = (-da).max(Decimal::ZERO);
            let spend_b = (-db).max(Decimal::ZERO);
            let usd = spend_a * Decimal::from_f64_retain(pa).unwrap_or(Decimal::ZERO)
                + spend_b * Decimal::from_f64_retain(pb).unwrap_or(Decimal::ZERO);
            usd.to_f64().unwrap_or(0.0)
        }
    } else {
        0.0
    };
    if capital <= 0.0 {
        return Err(ApiError::bad_request(
            "Could not derive capital from open session; provide capital override or open with cost_session_id",
        ));
    }

    // Fetch pool mints from RPC (Orca snapshot path requires cross-pair).
    let pool_state = clmm_lp_protocols::prelude::WhirlpoolReader::new(state.provider.clone())
        .get_pool_state(&pool_address)
        .await
        .map_err(|e| ApiError::internal(format!("get_pool_state failed: {e}")))?;
    let mint_a = pool_state.token_mint_a.to_string();
    let mint_b = pool_state.token_mint_b.to_string();

    let id = Uuid::new_v4().to_string();
    let job = BacktestJobResponse {
        id: id.clone(),
        position_address: position.to_string(),
        pool_address: pool_address.clone(),
        status: "running".to_string(),
        started_ts_utc: chrono::Utc::now().to_rfc3339(),
        finished_ts_utc: None,
        exit_code: None,
        stdout: None,
        stderr: None,
        note: Some("Runs `clmm-lp-cli backtest` as a subprocess; output is best-effort and may depend on local snapshot data availability.".to_string()),
    };

    {
        let mut w = JOBS.write().await;
        w.insert(id.clone(), job.clone());
    }

    let strategy = req.strategy.clone().unwrap_or_else(|| "static".to_string());
    let fee_source = req.fee_source.clone().unwrap_or_else(|| "snapshots".to_string());
    let price_path_source = req
        .price_path_source
        .clone()
        .unwrap_or_else(|| "snapshots".to_string());
    let snapshot_protocol = req
        .snapshot_protocol
        .clone()
        .unwrap_or_else(|| "orca".to_string());

    let job_id = id.clone();
    tokio::spawn(async move {
        let mut cmd = Command::new("clmm-lp-cli");
        cmd.arg("backtest");
        cmd.arg("--symbol-a").arg("A");
        cmd.arg("--mint-a").arg(&mint_a);
        cmd.arg("--symbol-b").arg("B");
        cmd.arg("--mint-b").arg(&mint_b);
        cmd.arg("--lower").arg(format!("{lower}"));
        cmd.arg("--upper").arg(format!("{upper}"));
        cmd.arg("--capital").arg(format!("{capital}"));
        cmd.arg("--strategy").arg(strategy);
        cmd.arg("--fee-source").arg(fee_source);
        cmd.arg("--price-path-source").arg(price_path_source);
        cmd.arg("--snapshot-protocol").arg(snapshot_protocol);
        cmd.arg("--snapshot-pool-address").arg(&pool_address);
        if let Some(sd) = start_date.as_deref() {
            cmd.arg("--start-date").arg(sd);
        }
        if let Some(ed) = end_date.as_deref() {
            cmd.arg("--end-date").arg(ed);
        }

        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let out = cmd.output().await;
        let mut w = JOBS.write().await;
        let Some(mut cur) = w.get(&job_id).cloned() else {
            return;
        };
        match out {
            Ok(o) => {
                cur.exit_code = o.status.code();
                cur.status = if o.status.success() {
                    "succeeded".to_string()
                } else {
                    "failed".to_string()
                };
                cur.stdout = Some(String::from_utf8_lossy(&o.stdout).to_string());
                cur.stderr = Some(String::from_utf8_lossy(&o.stderr).to_string());
                cur.finished_ts_utc = Some(chrono::Utc::now().to_rfc3339());
            }
            Err(e) => {
                cur.status = "failed".to_string();
                cur.stderr = Some(format!("failed to spawn backtest subprocess: {e}"));
                cur.finished_ts_utc = Some(chrono::Utc::now().to_rfc3339());
            }
        }
        w.insert(job_id.clone(), cur);
    });

    Ok(Json(BacktestJobStatusResponse {
        id,
        status: job.status,
        note: job.note,
    }))
}

/// Get job status and outputs.
#[utoipa::path(
    get,
    path = "/backtests/{id}",
    tag = "Analytics",
    params(
        ("id" = String, Path, description = "Backtest job id")
    ),
    responses(
        (status = 200, description = "Job record", body = BacktestJobResponse),
        (status = 404, description = "Not found")
    )
)]
pub async fn get_backtest_job(
    State(_state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<BacktestJobResponse>> {
    let r = JOBS.read().await;
    let j = r.get(id.trim()).cloned().ok_or_else(|| ApiError::not_found("Job not found"))?;
    Ok(Json(j))
}

