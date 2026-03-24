//! Machine-readable output from `clmm-lp-cli backtest-optimize` (for bots / API).

use serde::{Deserialize, Serialize};

/// Root document written to `--optimize-result-json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizeResultFile {
    /// Schema version; bump when fields change.
    pub schema_version: u32,
    /// RFC3339 timestamp when the file was produced.
    pub computed_at: String,
    /// Objective name (e.g. `VsHodl`).
    pub objective: String,
    /// Human pair label, e.g. `SOL/USDC`.
    pub pair_label: String,
    pub mint_a: String,
    pub mint_b: String,
    pub token_a_decimals: u8,
    pub token_b_decimals: u8,
    /// Orca whirlpool or snapshot pool address used for calibration, if any.
    pub pool_address: Option<String>,
    pub price_path_source: String,
    pub fee_source: String,
    pub windows: usize,
    /// Grid winner.
    pub winner: OptimizeWinner,
    /// Objective score for the winner (stringified decimal).
    pub score: String,
    pub tracker_summary: OptimizeTrackerSummary,
    /// RetouchShift hybrid repeat policy from the CLI run (if enabled).
    pub retouch_repeat: Option<OptimizeRetouchRepeat>,
}

/// Winning strategy + range from the grid.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizeWinner {
    /// Simulator label, e.g. `periodic_24h`, `threshold_5%`.
    pub strategy_label: String,
    /// One of: `static`, `periodic`, `threshold`, `il_limit`, `retouch_shift`, `oor_recenter`.
    pub strategy_kind: String,
    /// Total range width as a **fraction** (matches backtest grid / `DecisionConfig.range_width_pct`), e.g. `0.1` = 10%.
    pub width_pct: f64,
    pub range_lower_usd: f64,
    pub range_upper_usd: f64,
    pub periodic_interval_hours: Option<u64>,
    /// Threshold ratio for `StrategyMode::Threshold`, e.g. `0.05` = 5% midpoint deviation.
    pub threshold_ratio: Option<f64>,
    /// IL-limit rebalance threshold ratio (backtest `max_il`).
    pub il_max_ratio: Option<f64>,
    pub il_close_ratio: Option<f64>,
    pub il_grace_steps: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizeTrackerSummary {
    pub final_value: String,
    pub final_pnl: String,
    pub final_il_pct: String,
    pub total_fees: String,
    pub time_in_range_pct: String,
    pub rebalance_count: u32,
    pub total_rebalance_cost: String,
    pub max_drawdown: String,
    pub hodl_value: String,
    pub vs_hodl: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizeRetouchRepeat {
    pub cooldown_secs: u64,
    pub rearm_after_secs: u64,
    pub extra_move_pct: f64,
}

impl OptimizeResultFile {
    /// Current schema version consumed by `clmm-lp-execution::optimize_profile`.
    pub const CURRENT_SCHEMA_VERSION: u32 = 1;
}
