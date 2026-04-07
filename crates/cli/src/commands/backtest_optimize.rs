//! Helpers for backtest-optimize command: data fetching and grid defaults.

use crate::engine::token_meta::fetch_mint_decimals;
use anyhow::Result;
use clmm_lp_data::providers::DuneClient;
use clmm_lp_data::swaps::SwapEvent;
use clmm_lp_domain::math::fee_math::calculate_effective_fee_rate;
use clmm_lp_protocols::orca::pool_reader::WhirlpoolReader;
use clmm_lp_protocols::rpc::RpcProvider;
use rust_decimal::Decimal;
use std::collections::HashMap;
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
    _il_max_pct: f64,
    _il_close_pct: Option<f64>,
    _il_grace_steps: u64,
) -> Vec<StratConfig> {
    if static_only {
        vec![StratConfig::Static]
    } else {
        let mut v = vec![
            StratConfig::Static,
            StratConfig::Threshold(0.02),
            StratConfig::Threshold(0.03),
            StratConfig::Threshold(0.05),
            StratConfig::Threshold(0.07),
            StratConfig::Threshold(0.10),
            StratConfig::Threshold(0.15),
            // `Periodic(n)` is **steps** between rebalances (same timebase as candle/step index).
            StratConfig::Periodic(12),
            StratConfig::Periodic(24),
            StratConfig::Periodic(48),
            StratConfig::Periodic(72),
        ];
        if indicator_strategies {
            // Three σ-width presets (`k` in `SMA ± k·σ`): narrower (1.5σ), classic (2σ), wider (2.5σ).
            // Each paired with two rebalance cadences (`rebalance_steps`; same timebase as the path).
            for k in [1.5_f64, 2.0, 2.5] {
                v.push(StratConfig::Bollinger {
                    window: 20,
                    k,
                    rebalance_steps: 24,
                });
                v.push(StratConfig::Bollinger {
                    window: 20,
                    k,
                    rebalance_steps: 48,
                });
            }
            // Last-closed-candle anchor: `(candle_steps, rebalance_steps)` in **simulation steps**.
            // With `--resolution-seconds 900` (15 min/step), the grid below matches:
            // - 15m candle: rebal 15/30/45/60m, 4h, 12h  → (1,1|2|3|4|16|48)
            // - 30m candle: rebal 30/45/60m, 4h, 12h     → (2,2|3|4|16|48)
            // - 45m candle: rebal 15/30/45/60m, 4h, 12h → (3,1|2|3|4|16|48)
            // - 1h candle:  rebal 1h, 4h, 12h           → (4,4|16|48)
            // Each rebalance pays `tx_cost` in `run_single` — frequent presets stress fee drag.
            for &(candle_steps, rebalance_steps) in LAST_CANDLE_OPTIMIZE_GRID {
                v.push(StratConfig::LastCandle {
                    candle_steps,
                    rebalance_steps,
                });
            }
        }
        v
    }
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
