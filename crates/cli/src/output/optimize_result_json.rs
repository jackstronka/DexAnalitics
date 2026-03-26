//! Build `clmm_lp_domain::optimize_result::OptimizeResultFile` after a grid run.

use crate::backtest_engine::{RetouchRepeatConfig, StratConfig, parse_strategy_label};
use clmm_lp_domain::optimize_result::{
    OptimizeResultFile, OptimizeRetouchRepeat, OptimizeTrackerSummary, OptimizeWinner,
};
use clmm_lp_simulation::prelude::TrackerSummary;
use rust_decimal::Decimal;
use std::path::Path;

#[allow(clippy::too_many_arguments)]
pub fn build_optimize_result_file(
    computed_at_rfc3339: String,
    objective: &str,
    pair_label: String,
    mint_a: String,
    mint_b: String,
    token_a_decimals: u8,
    token_b_decimals: u8,
    pool_address: Option<String>,
    price_path_source: &str,
    fee_source: &str,
    windows: usize,
    width_pct: f64,
    lower_usd: f64,
    upper_usd: f64,
    strat_name: &str,
    summary: &TrackerSummary,
    score: Decimal,
    retouch_repeat: Option<RetouchRepeatConfig>,
) -> anyhow::Result<OptimizeResultFile> {
    let strat = parse_strategy_label(strat_name).ok_or_else(|| {
        anyhow::anyhow!("could not parse strategy label for JSON: {}", strat_name)
    })?;

    let winner = optimize_winner_from_strat(strat, strat_name, width_pct, lower_usd, upper_usd);

    let rr = retouch_repeat.map(|c| OptimizeRetouchRepeat {
        cooldown_secs: c.cooldown_secs,
        rearm_after_secs: c.rearm_after_secs,
        extra_move_pct: c.extra_move_pct,
    });

    Ok(OptimizeResultFile {
        schema_version: OptimizeResultFile::CURRENT_SCHEMA_VERSION,
        computed_at: computed_at_rfc3339,
        objective: objective.to_string(),
        pair_label,
        mint_a,
        mint_b,
        token_a_decimals,
        token_b_decimals,
        pool_address,
        price_path_source: price_path_source.to_string(),
        fee_source: fee_source.to_string(),
        windows,
        winner,
        score: score.to_string(),
        tracker_summary: tracker_summary_to_serde(summary),
        retouch_repeat: rr,
    })
}

fn optimize_winner_from_strat(
    strat: StratConfig,
    strat_name: &str,
    width_pct: f64,
    lower_usd: f64,
    upper_usd: f64,
) -> OptimizeWinner {
    let strategy_kind = match strat {
        StratConfig::Static => "static",
        StratConfig::Periodic(_) => "periodic",
        StratConfig::Threshold(_) => "threshold",
        StratConfig::ILLimit { .. } => "il_limit",
        StratConfig::RetouchShift => "retouch_shift",
        StratConfig::OorRecenter => "oor_recenter",
    }
    .to_string();

    let mut periodic_interval_hours = None;
    let mut threshold_ratio = None;
    let mut il_max_ratio = None;
    let mut il_close_ratio = None;
    let mut il_grace_steps = None;

    match strat {
        StratConfig::Periodic(h) => periodic_interval_hours = Some(h),
        StratConfig::Threshold(p) => threshold_ratio = Some(p),
        StratConfig::ILLimit {
            max_il,
            close_il,
            grace_steps,
        } => {
            il_max_ratio = Some(max_il);
            il_close_ratio = close_il;
            il_grace_steps = Some(grace_steps);
        }
        _ => {}
    }

    OptimizeWinner {
        strategy_label: strat_name.to_string(),
        strategy_kind,
        width_pct,
        range_lower_usd: lower_usd,
        range_upper_usd: upper_usd,
        periodic_interval_hours,
        threshold_ratio,
        il_max_ratio,
        il_close_ratio,
        il_grace_steps,
    }
}

fn tracker_summary_to_serde(s: &TrackerSummary) -> OptimizeTrackerSummary {
    OptimizeTrackerSummary {
        final_value: s.final_value.to_string(),
        final_pnl: s.final_pnl.to_string(),
        final_il_pct: s.final_il_pct.to_string(),
        total_fees: s.total_fees.to_string(),
        time_in_range_pct: s.time_in_range_pct.to_string(),
        rebalance_count: s.rebalance_count,
        total_rebalance_cost: s.total_rebalance_cost.to_string(),
        max_drawdown: s.max_drawdown.to_string(),
        hodl_value: s.hodl_value.to_string(),
        vs_hodl: s.vs_hodl.to_string(),
    }
}

/// Serialize to pretty JSON and write `path` (creates parent dirs).
pub fn write_optimize_result_json(path: &Path, file: &OptimizeResultFile) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(file)?;
    std::fs::write(path, json)?;
    Ok(())
}

/// Write the same document under `copy_dir` as `<UTC timestamp>.json` and `latest.json` (for agent / audit history).
pub fn write_optimize_result_copy_dir(
    copy_dir: &Path,
    file: &OptimizeResultFile,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(copy_dir)?;
    let ts = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let stamped = copy_dir.join(format!("{ts}.json"));
    write_optimize_result_json(&stamped, file)?;
    let latest = copy_dir.join("latest.json");
    write_optimize_result_json(&latest, file)?;
    Ok(())
}
