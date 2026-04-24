//! Helpers for backtest-optimize command: data fetching and grid defaults.

use crate::engine::token_meta::fetch_mint_decimals;
use anyhow::Result;
use clmm_lp_data::providers::DuneClient;
use clmm_lp_data::swaps::SwapEvent;
use clmm_lp_domain::math::fee_math::calculate_effective_fee_rate;
use clmm_lp_protocols::orca::pool_reader::WhirlpoolReader;
use clmm_lp_protocols::rpc::RpcProvider;
use rust_decimal::Decimal;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::backtest_engine::StratConfig;

/// Resolve Dune swaps query ID from protocol name or raw ID.
pub fn dune_swaps_query_id(name: &str) -> &str {
    match name.to_lowercase().as_str() {
        "orca" => "6848259",
        "meteora" => "6848336",
        "raydium" => "6848343",
        _ => name,
    }
}

/// Fetch Dune TVL and volume maps for a pool. Returns (None, None) if empty or missing.
pub async fn fetch_dune_tvl_volume(
    pool: &str,
) -> Result<(
    Option<HashMap<String, Decimal>>,
    Option<HashMap<String, Decimal>>,
)> {
    let dune = DuneClient::from_env()?;
    let (tvl_map, vol_map) = dune.fetch_tvl_volume_maps(pool).await?;
    Ok(if tvl_map.is_empty() || vol_map.is_empty() {
        (None, None)
    } else {
        (Some(tvl_map), Some(vol_map))
    })
}

/// Fetch on-chain Orca Whirlpool state: liquidity, effective fee rate, token decimals.
pub async fn fetch_pool_state(
    pool: &str,
    _token_a_decimals_guess: u8,
    _token_b_decimals_guess: u8,
    use_cross_pair: bool,
) -> Result<(
    Option<u128>,
    Option<Decimal>,
    u8,
    u8,
    Option<String>,
    Option<String>,
)> {
    let rpc = Arc::new(RpcProvider::mainnet());
    let reader = WhirlpoolReader::new(rpc.clone());
    let state = reader.get_pool_state(pool).await?;
    let base_fee = state.fee_rate();
    let protocol_fee_pct = Decimal::from(state.protocol_fee_rate_bps) / Decimal::from(10_000);
    let eff = calculate_effective_fee_rate(base_fee, protocol_fee_pct);
    let dec_a = fetch_mint_decimals(rpc.as_ref(), &state.token_mint_a.to_string()).await?;
    let dec_b = fetch_mint_decimals(rpc.as_ref(), &state.token_mint_b.to_string()).await?;
    Ok((
        Some(state.liquidity),
        Some(eff),
        dec_a,
        if use_cross_pair { dec_b } else { 6 },
        Some(state.token_vault_a.to_string()),
        Some(state.token_vault_b.to_string()),
    ))
}

/// Filter Dune swap events down to a specific pool, using vaults when available.
pub fn filter_swaps_for_pool(
    swaps: Vec<SwapEvent>,
    token_vault_a: Option<&str>,
    token_vault_b: Option<&str>,
    token_mint_a: &str,
    token_mint_b: &str,
) -> Vec<SwapEvent> {
    let va = token_vault_a.unwrap_or_default();
    let vb = token_vault_b.unwrap_or_default();
    let use_vaults = !va.is_empty()
        && !vb.is_empty()
        && va != "11111111111111111111111111111111"
        && vb != "11111111111111111111111111111111";

    swaps
        .into_iter()
        .filter(|s| {
            if use_vaults {
                (s.token_sold_vault == va && s.token_bought_vault == vb)
                    || (s.token_sold_vault == vb && s.token_bought_vault == va)
            } else {
                (s.token_sold_mint_address == token_mint_a
                    && s.token_bought_mint_address == token_mint_b)
                    || (s.token_sold_mint_address == token_mint_b
                        && s.token_bought_mint_address == token_mint_a)
            }
        })
        .collect()
}

/// Fetch Dune swap events for fee calculation. Returns None if dune_swaps arg is None.
pub async fn fetch_swaps_for_optimize(query_arg: &str) -> Result<Option<Vec<SwapEvent>>> {
    let query_id = dune_swaps_query_id(query_arg);
    let dune = DuneClient::from_env_swaps_only()?;
    println!(
        "📡 Fetching Dune swaps (query {}) for fee calculation...",
        query_id
    );
    Ok(Some(dune.fetch_swaps(query_id).await?))
}

/// Default strategy set for grid search.
pub fn default_strategies(
    static_only: bool,
    indicator_strategies: bool,
    snapshot_mode: bool,
    il_max_pct: f64,
    il_close_pct: Option<f64>,
    il_grace_steps: u64,
    threshold_grid: Option<&[f64]>,
    threshold_min_rebalance_interval_hours: u64,
    threshold_rebalance_on_range_exit_immediately: bool,
    retouch_offset_pct: f64,
    periodic_grid: Option<&[u64]>,
    bollinger_windows: Option<&[u64]>,
    bollinger_k: Option<&[f64]>,
    bollinger_rebalance_steps: Option<&[u64]>,
    last_candle_steps: Option<&[u64]>,
    last_candle_rebalance_steps: Option<&[u64]>,
    last_candle_seconds: Option<&[u64]>,
    last_candle_rebalance_seconds: Option<&[u64]>,
) -> Vec<StratConfig> {
    if static_only {
        vec![StratConfig::Static]
    } else {
        let threshold_grid = threshold_grid.unwrap_or(&[2.0, 3.0, 5.0, 7.0, 10.0, 15.0]);
        // Despite historical flag name `--periodic-grid-steps`, values represent wall-clock hours
        // for `StratConfig::Periodic` (bot-like semantics).
        let periodic_grid = periodic_grid.unwrap_or(&[12, 24, 48, 72]);
        let mut v = vec![StratConfig::Static, StratConfig::OorRecenter];
        for t in threshold_grid {
            if *t > 0.0 {
                v.push(StratConfig::Threshold {
                    threshold_pct: *t / 100.0,
                    min_rebalance_interval_hours: threshold_min_rebalance_interval_hours,
                    rebalance_on_range_exit_immediately:
                        threshold_rebalance_on_range_exit_immediately,
                });
            }
        }
        for p in periodic_grid {
            if *p > 0 {
                v.push(StratConfig::Periodic(*p));
            }
        }
        v.push(StratConfig::IlLimit {
            max_il_pct: il_max_pct / 100.0,
            close_il_pct: il_close_pct.map(|v| v / 100.0),
            grace_steps: il_grace_steps,
        });
        v.push(StratConfig::RetouchShift { retouch_offset_pct });
        if indicator_strategies {
            let bollinger_windows = bollinger_windows.unwrap_or(&[20]);
            let bollinger_k = bollinger_k.unwrap_or(&[1.5_f64, 2.0, 2.5]);
            let bollinger_rebalance_steps = bollinger_rebalance_steps.unwrap_or(&[24, 48]);
            // Three σ-width presets (`k` in `SMA ± k·σ`): narrower (1.5σ), classic (2σ), wider (2.5σ).
            // Each paired with two rebalance cadences (`rebalance_steps`; same timebase as the path).
            for window in bollinger_windows {
                for k in bollinger_k {
                    for rebalance_steps in bollinger_rebalance_steps {
                        if *window > 0 && *k > 0.0 && *rebalance_steps > 0 {
                            v.push(StratConfig::Bollinger {
                                window: *window as u32,
                                k: *k,
                                rebalance_steps: *rebalance_steps,
                            });
                        }
                    }
                }
            }
            if snapshot_mode {
                // Snapshot mode has irregular step spacing; prefer wall-clock buckets.
                // Candles: 15m, 30m, 45m, 1h. Rebalance: 15m, 30m, 45m, 1h, 4h, 12h.
                let candles = last_candle_seconds.unwrap_or(&[15 * 60, 30 * 60, 45 * 60, 60 * 60]);
                let rebals = last_candle_rebalance_seconds.unwrap_or(&[
                    15 * 60,
                    30 * 60,
                    45 * 60,
                    60 * 60,
                    4 * 3600,
                    12 * 3600,
                ]);
                for &c in candles {
                    for &r in rebals {
                        v.push(StratConfig::LastCandleTime {
                            candle_seconds: c,
                            rebalance_seconds: r,
                        });
                    }
                }
            } else {
                // Step-based last-candle (regular candle path).
                if let (Some(candles), Some(rebals)) = (last_candle_steps, last_candle_rebalance_steps) {
                    for &candle_steps in candles {
                        for &rebalance_steps in rebals {
                            if candle_steps > 0 && rebalance_steps > 0 {
                                v.push(StratConfig::LastCandle {
                                    candle_steps,
                                    rebalance_steps,
                                });
                            }
                        }
                    }
                } else {
                    for &(candle_steps, rebalance_steps) in LAST_CANDLE_OPTIMIZE_GRID {
                        v.push(StratConfig::LastCandle {
                            candle_steps,
                            rebalance_steps,
                        });
                    }
                }
            }
        }
        v
    }
}

fn strategy_family_name(s: &StratConfig) -> &'static str {
    match s {
        StratConfig::Static => "static",
        StratConfig::OorRecenter => "oor_recenter",
        StratConfig::Threshold { .. } => "threshold",
        StratConfig::Periodic(_) | StratConfig::PeriodicSteps(_) => "periodic",
        StratConfig::IlLimit { .. } => "il_limit",
        StratConfig::RetouchShift { .. } => "retouch_shift",
        StratConfig::Bollinger { .. } => "bollinger",
        StratConfig::LastCandle { .. } | StratConfig::LastCandleTime { .. } => "last_candle",
    }
}

/// Keep only selected strategy families (e.g. `static,threshold,il_limit`).
pub fn filter_strategies_by_families(
    strategies: Vec<StratConfig>,
    include_families: Option<&[String]>,
) -> Vec<StratConfig> {
    let Some(raw) = include_families else {
        return strategies;
    };
    let wanted: HashSet<String> = raw
        .iter()
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    if wanted.is_empty() {
        return strategies;
    }
    strategies
        .into_iter()
        .filter(|s| wanted.contains(strategy_family_name(s)))
        .collect()
}

/// Presets for `backtest-optimize --indicator-strategies`: wall-clock labels assume **15 min / step** (`--resolution-seconds 900`).
const LAST_CANDLE_OPTIMIZE_GRID: &[(u64, u64)] = &[
    // 15-minute candle (1 step)
    (1, 1),
    (1, 2),
    (1, 3),
    (1, 4),
    (1, 16),
    (1, 48),
    // 30-minute candle (2 steps)
    (2, 2),
    (2, 3),
    (2, 4),
    (2, 16),
    (2, 48),
    // 45-minute candle (3 steps @ 900s/step)
    (3, 1),
    (3, 2),
    (3, 3),
    (3, 4),
    (3, 16),
    (3, 48),
    // 1-hour candle (4 steps)
    (4, 4),
    (4, 16),
    (4, 48),
];

#[cfg(test)]
mod tests {
    use super::{default_strategies, filter_strategies_by_families};
    use crate::backtest_engine::StratConfig;

    #[test]
    fn default_grid_includes_documented_non_indicator_strategies() {
        let v = default_strategies(
            false,
            false,
            true,
            5.0,
            Some(12.0),
            3,
            None,
            0,
            true,
            0.0,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );
        assert!(v.contains(&StratConfig::Static));
        assert!(v.contains(&StratConfig::OorRecenter));
        assert!(v.contains(&StratConfig::RetouchShift {
            retouch_offset_pct: 0.0,
        }));
        assert!(v.contains(&StratConfig::IlLimit {
            max_il_pct: 0.05,
            close_il_pct: Some(0.12),
            grace_steps: 3,
        }));
    }

    #[test]
    fn filter_strategies_by_family_keeps_requested_only() {
        let v = default_strategies(
            false,
            true,
            true,
            5.0,
            Some(12.0),
            3,
            None,
            0,
            true,
            0.0,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );
        let include = vec!["static".to_string(), "il_limit".to_string()];
        let filtered = filter_strategies_by_families(v, Some(&include));
        assert!(filtered.contains(&StratConfig::Static));
        assert!(filtered.iter().any(|s| matches!(s, StratConfig::IlLimit { .. })));
        assert!(!filtered
            .iter()
            .any(|s| matches!(s, StratConfig::Threshold { .. })));
        assert!(!filtered.iter().any(|s| matches!(s, StratConfig::Bollinger { .. })));
    }
}
