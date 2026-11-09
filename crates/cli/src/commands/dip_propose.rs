//! CLI: append Dip propose-only decision to `agent_decisions.jsonl` (Faza C).

use anyhow::{Context, Result};
use clmm_lp_data::dip::{
    DipAuditDecisionRow, DipProposeAction, DipProposeDecision, DipProposeMode, append_dip_propose,
    default_agent_decisions_path,
};
use std::path::PathBuf;

pub fn run_dip_propose(
    action: String,
    mode: String,
    rationale: String,
    refs: Vec<String>,
    zone: Option<String>,
    sol_usd: Option<f64>,
    path: Option<PathBuf>,
) -> Result<()> {
    let action = DipProposeAction::parse(&action)?;
    let mode = DipProposeMode::parse(&mode)?;
    let mut decision = DipProposeDecision::new(action, mode, rationale, refs)?;
    decision.zone = zone.map(|z| z.trim().to_string()).filter(|s| !s.is_empty());
    decision.sol_usd = sol_usd;

    let out_path = path.unwrap_or_else(default_agent_decisions_path);
    let row = DipAuditDecisionRow {
        ts_utc: String::new(),
        source: clmm_lp_data::dip::DIP_PROPOSE_SOURCE.to_string(),
        strategy_id: None,
        position_id: None,
        chain_id: None,
        session_id: None,
        decision,
    };
    let written = append_dip_propose(&out_path, row).context("append dip propose")?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "path": out_path,
            "written": true,
            "row": written,
        }))
        .context("serialize")?
    );
    Ok(())
}
