//! Compute best-effort USD valuations for positions.
//!
//! The monitor tracks on-chain state and IL/fees in raw token units, but it does not compute USD.
//! For dashboard/UI we derive USD value from:
//! - pool state (tick/sqrt_price) + position liquidity -> token amounts (raw u64)
//! - mint decimals (on-chain SPL mint)
//! - free USD prices (Jupiter v2 + fallbacks — see `price_fetch`)

use crate::error::ApiError;
use crate::services::price_fetch::fetch_mint_prices_usd;
use anyhow::Context;
use clmm_lp_execution::monitor::{MonitoredPosition, PositionPnL};
use clmm_lp_protocols::orca::pool_reader::WhirlpoolReader;
use clmm_lp_protocols::orca::position_reader::PositionReader;
use clmm_lp_protocols::rpc::RpcProvider;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use solana_sdk::pubkey::Pubkey;
use spl_token::solana_program::program_pack::Pack;
use spl_token::state::Mint;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct PositionUsdValuation {
    pub value_usd: Decimal,
    pub fees_usd: Decimal,
    pub amount_a_raw: u64,
    pub amount_b_raw: u64,
}

async fn fetch_mint_decimals(provider: &RpcProvider, mint: &Pubkey) -> anyhow::Result<u8> {
    let account = provider
        .get_account(mint)
        .await
        .context("fetch mint account")?;
    let mint_state = Mint::unpack(&account.data).context("unpack SPL Mint")?;
    Ok(mint_state.decimals)
}

fn ui_amount(raw: u64, decimals: u8) -> f64 {
    if decimals == 0 {
        return raw as f64;
    }
    let denom = 10f64.powi(i32::from(decimals));
    (raw as f64) / denom
}

/// Build a [`MonitoredPosition`] from RPC when the address is **not** in the in-memory monitor
/// (e.g. opened before `monitor.add_position`, or API restarted). Used by `GET /positions/:address`.
pub async fn monitored_position_from_chain(
    provider: Arc<RpcProvider>,
    position_address: &Pubkey,
) -> Result<MonitoredPosition, ApiError> {
    let position_reader = PositionReader::new(provider.clone());
    let on_chain = position_reader
        .get_position(&position_address.to_string())
        .await
        .map_err(|e| ApiError::not_found(format!("Position not found: {e}")))?;

    let pool_reader = WhirlpoolReader::new(provider.clone());
    let pool_state = pool_reader
        .get_pool_state(&on_chain.pool.to_string())
        .await
        .map_err(|e| ApiError::internal(format!("pool state: {e}")))?;
    let in_range = pool_state.is_tick_in_range(on_chain.tick_lower, on_chain.tick_upper);

    let pnl = PositionPnL {
        fees_earned_a: on_chain.fees_owed_a,
        fees_earned_b: on_chain.fees_owed_b,
        ..PositionPnL::default()
    };

    Ok(MonitoredPosition {
        address: on_chain.address,
        pool: on_chain.pool,
        on_chain,
        pnl,
        in_range,
        last_updated: chrono::Utc::now(),
    })
}

/// Compute USD valuation for a single monitored position (best-effort).
pub async fn compute_position_usd_valuation(
    provider: Arc<RpcProvider>,
    position: &MonitoredPosition,
    prices_usd: &BTreeMap<String, f64>,
) -> Result<PositionUsdValuation, ApiError> {
    let pool_reader = WhirlpoolReader::new(provider.clone());
    let position_reader = PositionReader::new(provider.clone());

    let pool_state = pool_reader
        .get_pool_state(&position.pool.to_string())
        .await
        .map_err(|e| ApiError::internal(format!("pool state fetch failed: {e}")))?;

    let (amount_a_raw, amount_b_raw) = position_reader.calculate_token_amounts(
        &position.on_chain,
        pool_state.tick_current,
        pool_state.sqrt_price,
    );

    let dec_a = fetch_mint_decimals(provider.as_ref(), &pool_state.token_mint_a)
        .await
        .map_err(|e| ApiError::internal(format!("mint decimals fetch failed (A): {e}")))?;
    let dec_b = fetch_mint_decimals(provider.as_ref(), &pool_state.token_mint_b)
        .await
        .map_err(|e| ApiError::internal(format!("mint decimals fetch failed (B): {e}")))?;

    let pa = prices_usd
        .get(&pool_state.token_mint_a.to_string())
        .copied()
        .unwrap_or(0.0);
    let pb = prices_usd
        .get(&pool_state.token_mint_b.to_string())
        .copied()
        .unwrap_or(0.0);

    let a_ui = ui_amount(amount_a_raw, dec_a);
    let b_ui = ui_amount(amount_b_raw, dec_b);
    let fees_a_ui = ui_amount(position.on_chain.fees_owed_a, dec_a);
    let fees_b_ui = ui_amount(position.on_chain.fees_owed_b, dec_b);

    let value_usd_f = a_ui * pa + b_ui * pb;
    let fees_usd_f = fees_a_ui * pa + fees_b_ui * pb;

    Ok(PositionUsdValuation {
        value_usd: Decimal::from_f64(value_usd_f).unwrap_or(Decimal::ZERO),
        fees_usd: Decimal::from_f64(fees_usd_f).unwrap_or(Decimal::ZERO),
        amount_a_raw,
        amount_b_raw,
    })
}

/// Build a mint->price map for all pools referenced by the given positions.
pub async fn fetch_prices_for_positions(
    provider: Arc<RpcProvider>,
    positions: &[MonitoredPosition],
) -> BTreeMap<String, f64> {
    let pool_reader = WhirlpoolReader::new(provider.clone());

    let mut mints: BTreeSet<String> = BTreeSet::new();
    for p in positions {
        if let Ok(pool_state) = pool_reader.get_pool_state(&p.pool.to_string()).await {
            mints.insert(pool_state.token_mint_a.to_string());
            mints.insert(pool_state.token_mint_b.to_string());
        }
    }

    fetch_mint_prices_usd(&mints).await.0
}
