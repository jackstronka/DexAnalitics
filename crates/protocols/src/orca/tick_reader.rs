use super::pool_reader::WHIRLPOOL_PROGRAM_ID;
use super::tick_array::{
    TickArrayAccountBody, TickBoundaryState, tick_array_offset, tick_array_start_index,
};
use crate::rpc::RpcProvider;
use anyhow::{Context, Result};
use borsh::BorshDeserialize;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;
use std::sync::Arc;

pub struct WhirlpoolTickReader {
    provider: Arc<RpcProvider>,
}

impl WhirlpoolTickReader {
    pub fn new(provider: Arc<RpcProvider>) -> Self {
        Self { provider }
    }

    pub fn derive_tick_array_address(whirlpool: &Pubkey, start_tick_index: i32) -> Result<Pubkey> {
        let program_id =
            Pubkey::from_str(WHIRLPOOL_PROGRAM_ID).context("invalid whirlpool program id")?;
        let seed_prefix = b"tick_array";
        let start_bytes = start_tick_index.to_le_bytes();
        let (pda, _bump) = Pubkey::find_program_address(
            &[seed_prefix, whirlpool.as_ref(), &start_bytes],
            &program_id,
        );
        Ok(pda)
    }

    pub async fn get_tick_boundary_state(
        &self,
        whirlpool: &Pubkey,
        tick_index: i32,
        tick_spacing: u16,
    ) -> Result<TickBoundaryState> {
        let start = tick_array_start_index(tick_index, tick_spacing);
        let addr = Self::derive_tick_array_address(whirlpool, start)?;
        let account = self.provider.get_account(&addr).await?;

        let body = account
            .data
            .get(8..) // skip discriminator
            .context("tick array account too small")?;
        let parsed = TickArrayAccountBody::try_from_slice(body)
            .context("failed to deserialize tick array")?;

        if parsed.start_tick_index != start {
            // This should never happen if PDA derivation matches program; keep explicit for audit.
            anyhow::bail!(
                "tick array start mismatch: expected {start}, got {}",
                parsed.start_tick_index
            );
        }

        let off = tick_array_offset(tick_index, parsed.start_tick_index, tick_spacing)
            .context("tick index not in tick array")?;
        let t = parsed.ticks[off];
        Ok(TickBoundaryState {
            fee_growth_outside_a: t.fee_growth_outside_a,
            fee_growth_outside_b: t.fee_growth_outside_b,
            initialized: t.initialized,
        })
    }
}
