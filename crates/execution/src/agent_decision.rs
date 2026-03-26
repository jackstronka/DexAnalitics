//! Validate [`AgentDecision`](clmm_lp_domain::agent_decision::AgentDecision) before applying to the executor.

use clmm_lp_domain::agent_decision::AgentDecision;
use clmm_lp_domain::optimize_result::OptimizeResultFile;
use thiserror::Error;

/// Validation errors for agent decisions.
#[derive(Debug, Error)]
pub enum AgentDecisionValidationError {
    #[error("unsupported agent decision schema version: {0}")]
    UnsupportedSchemaVersion(u32),
    #[error("approved agent decision must include optimize_result")]
    MissingOptimizeResult,
    #[error(
        "proposed width_pct {proposed} differs from baseline {baseline} by more than max_width_pct_delta {max_delta}"
    )]
    WidthDeltaExceeded {
        baseline: f64,
        proposed: f64,
        max_delta: f64,
    },
}

/// Validate schema, approval semantics, and optional width delta vs baseline.
pub fn validate_agent_decision(
    decision: &AgentDecision,
    baseline: Option<&OptimizeResultFile>,
    max_width_pct_delta: Option<f64>,
) -> Result<(), AgentDecisionValidationError> {
    if decision.schema_version != AgentDecision::CURRENT_SCHEMA_VERSION {
        return Err(AgentDecisionValidationError::UnsupportedSchemaVersion(
            decision.schema_version,
        ));
    }
    if !decision.approved {
        return Ok(());
    }
    let opt = decision
        .optimize_result
        .as_ref()
        .ok_or(AgentDecisionValidationError::MissingOptimizeResult)?;

    if let (Some(delta), Some(base)) = (max_width_pct_delta, baseline) {
        let bw = base.winner.width_pct;
        let nw = opt.winner.width_pct;
        if (nw - bw).abs() > delta {
            return Err(AgentDecisionValidationError::WidthDeltaExceeded {
                baseline: bw,
                proposed: nw,
                max_delta: delta,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clmm_lp_domain::optimize_result::{OptimizeTrackerSummary, OptimizeWinner};

    fn minimal_file(width: f64) -> OptimizeResultFile {
        OptimizeResultFile {
            schema_version: OptimizeResultFile::CURRENT_SCHEMA_VERSION,
            computed_at: "2025-01-01T00:00:00Z".to_string(),
            objective: "Pnl".to_string(),
            pair_label: "A/B".to_string(),
            mint_a: "a".to_string(),
            mint_b: "b".to_string(),
            token_a_decimals: 9,
            token_b_decimals: 6,
            pool_address: None,
            price_path_source: "snapshots".to_string(),
            fee_source: "snapshots".to_string(),
            windows: 1,
            winner: OptimizeWinner {
                strategy_label: "static".to_string(),
                strategy_kind: "static".to_string(),
                width_pct: width,
                range_lower_usd: 1.0,
                range_upper_usd: 2.0,
                periodic_interval_hours: None,
                threshold_ratio: None,
                il_max_ratio: None,
                il_close_ratio: None,
                il_grace_steps: None,
            },
            score: "0".to_string(),
            tracker_summary: OptimizeTrackerSummary {
                final_value: "0".to_string(),
                final_pnl: "0".to_string(),
                final_il_pct: "0".to_string(),
                total_fees: "0".to_string(),
                time_in_range_pct: "0".to_string(),
                rebalance_count: 0,
                total_rebalance_cost: "0".to_string(),
                max_drawdown: "0".to_string(),
                hodl_value: "0".to_string(),
                vs_hodl: "0".to_string(),
            },
            retouch_repeat: None,
        }
    }

    #[test]
    fn rejected_skips_width_check() {
        let d = AgentDecision {
            schema_version: 1,
            approved: false,
            reason: Some("no".to_string()),
            optimize_result: None,
        };
        validate_agent_decision(&d, None, Some(0.001)).unwrap();
    }

    #[test]
    fn approved_requires_result() {
        let d = AgentDecision {
            schema_version: 1,
            approved: true,
            reason: None,
            optimize_result: None,
        };
        assert!(validate_agent_decision(&d, None, None).is_err());
    }

    #[test]
    fn width_delta_enforced() {
        let base = minimal_file(0.1);
        let proposed = minimal_file(0.2);
        let d = AgentDecision {
            schema_version: 1,
            approved: true,
            reason: None,
            optimize_result: Some(proposed),
        };
        validate_agent_decision(&d, Some(&base), Some(0.05)).unwrap_err();
        validate_agent_decision(&d, Some(&base), Some(0.15)).unwrap();
    }
}
