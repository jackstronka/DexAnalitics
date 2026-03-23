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
) -> Result<(Option<HashMap<String, Decimal>>, Option<HashMap<String, Decimal>>)> {
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
) -> Result<(Option<u128>, Option<Decimal>, u8, u8, Option<String>, Option<String>)> {
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
    let use_vaults = !va.is_empty() && !vb.is_empty() && va != "11111111111111111111111111111111" && vb != "11111111111111111111111111111111";

    swaps
        .into_iter()
        .filter(|s| {
            if use_vaults {
                (s.token_sold_vault == va && s.token_bought_vault == vb)
                    || (s.token_sold_vault == vb && s.token_bought_vault == va)
            } else {
                (s.token_sold_mint_address == token_mint_a && s.token_bought_mint_address == token_mint_b)
                    || (s.token_sold_mint_address == token_mint_b && s.token_bought_mint_address == token_mint_a)
            }
        })
        .collect()
}

/// Fetch Dune swap events for fee calculation. Returns None if dune_swaps arg is None.
pub async fn fetch_swaps_for_optimize(query_arg: &str) -> Result<Option<Vec<SwapEvent>>> {
    let query_id = dune_swaps_query_id(query_arg);
    let dune = DuneClient::from_env_swaps_only()?;
    println!("📡 Fetching Dune swaps (query {}) for fee calculation...", query_id);
    Ok(Some(dune.fetch_swaps(query_id).await?))
}

/// Default strategy set for grid search.
pub fn default_strategies(
    static_only: bool,
    il_max_pct: f64,
    il_close_pct: Option<f64>,
    il_grace_steps: u64,
) -> Vec<StratConfig> {
    if static_only {
        vec![StratConfig::Static]
    } else {
        vec![
            StratConfig::Static,
            StratConfig::Threshold(0.02),
            StratConfig::Threshold(0.03),
            StratConfig::Threshold(0.05),
            StratConfig::Threshold(0.07),
            StratConfig::Threshold(0.10),
            StratConfig::Threshold(0.15),
            StratConfig::Periodic(12),
            StratConfig::Periodic(24),
            StratConfig::Periodic(48),
            StratConfig::Periodic(72),
            StratConfig::ILLimit {
                max_il: il_max_pct / 100.0,
                close_il: il_close_pct.map(|v| v / 100.0),
                grace_steps: il_grace_steps,
            },
            StratConfig::RetouchShift,
        ]
    }
}
