//! Map CLI `backtest-optimize` JSON (`OptimizeResultFile`) to live `DecisionConfig`.

use crate::strategy::{DecisionConfig, StrategyMode};
use clmm_lp_domain::optimize_result::OptimizeResultFile;
use rust_decimal::Decimal;
use thiserror::Error;

/// Errors when applying an optimize-result file to the executor.
#[derive(Debug, Error)]
pub enum OptimizeProfileError {
    #[error("unsupported optimize result schema version: {0}")]
    UnsupportedSchemaVersion(u32),
    #[error("unknown strategy_kind: {0}")]
    UnknownStrategyKind(String),
    #[error("invalid decimal field {0}: {1}")]
    InvalidDecimal(&'static str, String),
    #[error("IL-limit profile missing il_max_ratio")]
    IlLimitMissingMax,
    #[error("periodic strategy missing periodic_interval_hours")]
    PeriodicMissingHours,
    #[error("threshold strategy missing threshold_ratio")]
    ThresholdMissingRatio,
}

/// Parse JSON text into [`OptimizeResultFile`].
pub fn parse_optimize_result_json(json: &str) -> Result<OptimizeResultFile, serde_json::Error> {
    serde_json::from_str(json)
}

/// Build a [`DecisionConfig`] from the winner in an optimize-result file.
///
/// Preserves fee-collection defaults from `DecisionConfig::default()` where not specified.
pub fn decision_config_from_optimize_result(
    file: &OptimizeResultFile,
) -> Result<DecisionConfig, OptimizeProfileError> {
    if file.schema_version != OptimizeResultFile::CURRENT_SCHEMA_VERSION {
        return Err(OptimizeProfileError::UnsupportedSchemaVersion(
            file.schema_version,
        ));
    }

    let w = &file.winner;
    let mut cfg = DecisionConfig::default();

    let width = Decimal::from_f64_retain(w.width_pct)
        .ok_or_else(|| OptimizeProfileError::InvalidDecimal("width_pct", w.width_pct.to_string()))?;
    cfg.range_width_pct = width;

    match w.strategy_kind.as_str() {
        "static" => {
            cfg.strategy_mode = StrategyMode::StaticRange;
        }
        "periodic" => {
            cfg.strategy_mode = StrategyMode::Periodic;
            cfg.periodic_interval_hours = w.periodic_interval_hours.ok_or(
                OptimizeProfileError::PeriodicMissingHours,
            )?;
        }
        "threshold" => {
            cfg.strategy_mode = StrategyMode::Threshold;
            let t = w
                .threshold_ratio
                .ok_or(OptimizeProfileError::ThresholdMissingRatio)?;
            cfg.threshold_pct = Decimal::from_f64_retain(t).ok_or_else(|| {
                OptimizeProfileError::InvalidDecimal("threshold_ratio", t.to_string())
            })?;
        }
        "oor_recenter" => {
            cfg.strategy_mode = StrategyMode::OorRecenter;
        }
        "retouch_shift" => {
            cfg.strategy_mode = StrategyMode::RetouchShift;
        }
        "il_limit" => {
            cfg.strategy_mode = StrategyMode::IlLimit;
            let max_il = w
                .il_max_ratio
                .ok_or(OptimizeProfileError::IlLimitMissingMax)?;
            cfg.il_rebalance_threshold = Decimal::from_f64_retain(max_il).ok_or_else(|| {
                OptimizeProfileError::InvalidDecimal("il_max_ratio", max_il.to_string())
            })?;
            if let Some(close) = w.il_close_ratio {
                cfg.il_close_threshold = Decimal::from_f64_retain(close).ok_or_else(|| {
                    OptimizeProfileError::InvalidDecimal("il_close_ratio", close.to_string())
                })?;
            }
            if let Some(g) = w.il_grace_steps {
                // Backtest uses "steps"; live IlLimit uses hours — treat grace as 0 hours when unknown.
                let _ = g;
            }
        }
        other => {
            return Err(OptimizeProfileError::UnknownStrategyKind(other.to_string()));
        }
    }

    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clmm_lp_domain::optimize_result::{
        OptimizeResultFile, OptimizeTrackerSummary, OptimizeWinner,
    };

    fn minimal_file(winner: OptimizeWinner) -> OptimizeResultFile {
        OptimizeResultFile {
            schema_version: OptimizeResultFile::CURRENT_SCHEMA_VERSION,
            computed_at: "2026-01-01T00:00:00Z".to_string(),
            objective: "VsHodl".to_string(),
            pair_label: "A/B".to_string(),
            mint_a: "mintA".to_string(),
            mint_b: "mintB".to_string(),
            token_a_decimals: 9,
            token_b_decimals: 6,
            pool_address: None,
            price_path_source: "birdeye".to_string(),
            fee_source: "auto".to_string(),
            windows: 1,
            winner,
            score: "1".to_string(),
            tracker_summary: OptimizeTrackerSummary {
                final_value: "100".to_string(),
                final_pnl: "0".to_string(),
                final_il_pct: "0".to_string(),
                total_fees: "0".to_string(),
                time_in_range_pct: "0.5".to_string(),
                rebalance_count: 0,
                total_rebalance_cost: "0".to_string(),
                max_drawdown: "0".to_string(),
                hodl_value: "100".to_string(),
                vs_hodl: "0".to_string(),
            },
            retouch_repeat: None,
        }
    }

    #[test]
    fn maps_periodic_and_width() {
        let f = minimal_file(OptimizeWinner {
            strategy_label: "periodic_24h".to_string(),
            strategy_kind: "periodic".to_string(),
            width_pct: 0.08,
            range_lower_usd: 90.0,
            range_upper_usd: 110.0,
            periodic_interval_hours: Some(24),
            threshold_ratio: None,
            il_max_ratio: None,
            il_close_ratio: None,
            il_grace_steps: None,
        });
        let c = decision_config_from_optimize_result(&f).unwrap();
        assert_eq!(c.strategy_mode, StrategyMode::Periodic);
        assert_eq!(c.periodic_interval_hours, 24);
        assert_eq!(c.range_width_pct, Decimal::from_f64_retain(0.08).unwrap());
    }

    #[test]
    fn rejects_bad_schema() {
        let mut f = minimal_file(OptimizeWinner {
            strategy_label: "static".to_string(),
            strategy_kind: "static".to_string(),
            width_pct: 0.1,
            range_lower_usd: 1.0,
            range_upper_usd: 2.0,
            periodic_interval_hours: None,
            threshold_ratio: None,
            il_max_ratio: None,
            il_close_ratio: None,
            il_grace_steps: None,
        });
        f.schema_version = 999;
        assert!(decision_config_from_optimize_result(&f).is_err());
    }
}
