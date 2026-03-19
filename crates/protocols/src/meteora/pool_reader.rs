//! Meteora DLMM pool-state parsing helpers.
//!
//! Meteora DLMM is bin-based. The on-chain `lb_pair` account contains:
//! - active bin id (`active_id`)
//! - token mints and vault/reserve accounts
//! - fee parameters (`protocol_fee.amount_x/y`)
//!
//! Note: DLMM global fee-growth accumulators are typically tracked at bin
//! level, so for a full fee-growth approach we will likely need bin-array
//! neighborhood snapshots in a later step.

use anyhow::{Context, Result};
use borsh::BorshDeserialize;
use carbon_meteora_dlmm_decoder::accounts::lb_pair::LbPair;
use solana_sdk::pubkey::Pubkey;

#[derive(Debug, Clone)]
pub struct MeteoraLbPairMinimal {
    pub active_id: i32,
    pub bin_step: u16,

    pub token_mint_x: Pubkey,
    pub token_mint_y: Pubkey,
    pub reserve_x: Pubkey,
    pub reserve_y: Pubkey,

    // Protocol fees tracked/owed at the pair level.
    pub protocol_fee_amount_x: u64,
    pub protocol_fee_amount_y: u64,
}

/// Parses Meteora DLMM `lb_pair` from raw account bytes.
pub fn parse_lb_pair(data: &[u8]) -> Result<MeteoraLbPairMinimal> {
    // `try_from_slice` is strict (borsh requires consuming the entire slice).
    // Some accounts contain trailing bytes beyond the minimal lb_pair layout,
    // so we parse via a reader and allow leftover bytes.
    let mut cursor = std::io::Cursor::new(data);
    let pair = LbPair::deserialize_reader(&mut cursor)
        .context("Meteora LbPair deserialization failed")?;

    Ok(MeteoraLbPairMinimal {
        active_id: pair.active_id,
        bin_step: pair.bin_step,
        token_mint_x: pair.token_x_mint,
        token_mint_y: pair.token_y_mint,
        reserve_x: pair.reserve_x,
        reserve_y: pair.reserve_y,
        protocol_fee_amount_x: pair.protocol_fee.amount_x,
        protocol_fee_amount_y: pair.protocol_fee.amount_y,
    })
}

