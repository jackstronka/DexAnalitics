//! Phase-1 decision layer: run curated `data-health-check` logic and append one JSONL row
//! compatible with `data/agent/agent_decisions.jsonl` (same shape as API `AgentDecisionRow`).

use anyhow::{Context, Result};
use chrono::Utc;
use serde::Serialize;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// One JSONL line compatible with API `AgentDecisionRow` / `GET /data/agent/decisions`.
#[derive(Debug, Serialize)]
struct AgentJsonlRow {
    ts_utc: String,
    source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    strategy_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    position_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    chain_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    decision: serde_json::Value,
}

/// Payload inside `AgentDecisionRow.decision` for orchestrator gate runs (schema v1).
#[derive(Debug, Clone, Serialize)]
pub struct OrchestratorRunV1 {
    pub schema_version: u32,
    pub run_id: String,
    pub kind: &'static str,
    pub tools_invoked: Vec<String>,
    pub data_quality: serde_json::Value,
    pub outcome: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_go_reason: Option<String>,
    pub inputs: serde_json::Value,
    /// Curated dataset file stats from `health_check_curated_all_collect` (phase 0 `inputs_ref`).
    pub inputs_ref: serde_json::Value,
}

/// Append one line to `agent_decisions.jsonl` (API-compatible row).
pub fn append_agent_decision_row(path: &Path, row: &serde_json::Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create_dir_all {}", parent.display()))?;
    }
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("open {}", path.display()))?;
    let line = serde_json::to_string(row).context("serialize agent decision row")?;
    writeln!(f, "{line}").with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

/// Run health collect, build `OrchestratorRunV1`, append to JSONL, optionally exit non-zero on `no_go`.
pub fn run_gate_and_log(
    max_age_minutes: i64,
    min_decode_ok_pct: f64,
    exit_non_zero_on_no_go: bool,
    jsonl_path: &Path,
    source: &str,
    chain_id: Option<&str>,
) -> Result<()> {
    let summary =
        crate::swap_sync::health_check_curated_all_collect(max_age_minutes, min_decode_ok_pct)?;
    let outcome = if summary.alerts.is_empty() {
        "ok"
    } else {
        "no_go"
    };
    let no_go_reason = if summary.alerts.is_empty() {
        None
    } else {
        Some(format!(
            "{} curated pool health alert(s); see data_quality.alerts",
            summary.alerts.len()
        ))
    };

    let run_id = format!("gate-{}", uuid::Uuid::new_v4());
    let data_quality = serde_json::json!({
        "alerts": summary.alerts,
        "alert_count": summary.alerts.len(),
        "health_report_path": summary.report_path,
        "summary_ts_utc": summary.ts_utc,
    });
    let inputs = serde_json::json!({
        "max_age_minutes": max_age_minutes,
        "min_decode_ok_pct": min_decode_ok_pct,
    });

    let decision = OrchestratorRunV1 {
        schema_version: 1,
        run_id,
        kind: "gate_health",
        tools_invoked: vec!["health_check_curated_all_collect".to_string()],
        data_quality,
        outcome,
        no_go_reason,
        inputs,
        inputs_ref: summary.inputs_ref.clone(),
    };
    let decision_value = serde_json::to_value(&decision).context("serialize orchestrator run")?;

    let row = AgentJsonlRow {
        ts_utc: Utc::now().to_rfc3339(),
        source: source.to_string(),
        strategy_id: None,
        position_id: None,
        chain_id: chain_id.map(str::to_string),
        session_id: None,
        decision: decision_value,
    };
    let row_value = serde_json::to_value(&row).context("serialize agent jsonl row")?;

    append_agent_decision_row(jsonl_path, &row_value)?;
    println!(
        "🧭 orchestrator-gate: appended row to {} (outcome={})",
        jsonl_path.display(),
        outcome
    );

    if exit_non_zero_on_no_go && outcome == "no_go" {
        anyhow::bail!(
            "orchestrator-gate: outcome=no_go (see JSONL row and optional health report)"
        );
    }
    Ok(())
}

/// Default path for agent decisions log (same default as API handler).
pub fn default_agent_decisions_path() -> PathBuf {
    PathBuf::from("data")
        .join("agent")
        .join("agent_decisions.jsonl")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_row_produces_one_json_line() {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "agent_decisions_test_{}.jsonl",
            uuid::Uuid::new_v4()
        ));
        let row = serde_json::json!({
            "ts_utc": "2026-05-14T12:00:00Z",
            "source": "test",
            "decision": {"schema_version": 1, "kind": "gate_health", "outcome": "ok"}
        });
        append_agent_decision_row(&p, &row).expect("append");
        let txt = std::fs::read_to_string(&p).expect("read");
        assert_eq!(txt.lines().count(), 1);
        let parsed: serde_json::Value =
            serde_json::from_str(txt.lines().next().unwrap()).expect("json");
        assert_eq!(parsed["source"], "test");
        let _ = std::fs::remove_file(&p);
    }
}
