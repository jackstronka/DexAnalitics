//! Raydium CLMM pool-state parsing helpers.
//!
//! We use the `raydium_clmm` crate (borsh/anchor-derived layout) to deserialize
//! the on-chain pool account and extract:
//! - `fee_growth_global*` (global fee accumulators)
//! - `liquidity` (active liquidity proxy)
//! - `tick_current`, `sqrt_price_x64`
//! - token mints + vaults

use anyhow::{Context, Result};
use borsh::BorshDeserialize;
use raydium_clmm::accounts::pool_state::PoolState;
#[derive(Debug, Clone)]
pub struct RaydiumClmmPoolStateMinimal {
    pub token_mint0: String,
    pub token_mint1: String,
    pub token_vault0: String,
    pub token_vault1: String,

    pub mint_decimals0: u8,
    pub mint_decimals1: u8,

    pub liquidity_active: u128,
    pub tick_current: i32,
    pub sqrt_price_x64: u128,

    pub fee_growth_global0_x64: u128,
    pub fee_growth_global1_x64: u128,

    pub protocol_fees_token0: u64,
    pub protocol_fees_token1: u64,
}

/// Parses Raydium CLMM `PoolState` from raw account data.
///
/// Important: this does not add any RPC cost; it only deserializes the bytes
/// you already fetched.
pub fn parse_pool_state(data: &[u8]) -> Result<RaydiumClmmPoolStateMinimal> {
    // `try_from_slice` is strict (borsh requires consuming the entire slice).
    // Some accounts may contain trailing bytes beyond the minimal pool-state layout,
    // so parse via a reader to tolerate leftover bytes.
    let mut cursor = std::io::Cursor::new(data);
    let state = PoolState::deserialize_reader(&mut cursor)
        .context("Raydium PoolState deserialization failed")?;

    Ok(RaydiumClmmPoolStateMinimal {
        token_mint0: state.token_mint0.to_string(),
        token_mint1: state.token_mint1.to_string(),
        token_vault0: state.token_vault0.to_string(),
        token_vault1: state.token_vault1.to_string(),
        mint_decimals0: state.mint_decimals0,
        mint_decimals1: state.mint_decimals1,
        liquidity_active: state.liquidity,
        tick_current: state.tick_current,
        sqrt_price_x64: state.sqrt_price_x64,
        fee_growth_global0_x64: state.fee_growth_global0_x64,
        fee_growth_global1_x64: state.fee_growth_global1_x64,
        protocol_fees_token0: state.protocol_fees_token0,
        protocol_fees_token1: state.protocol_fees_token1,
    })
}
