//! Orca Whirlpool position reader.
//!
//! Reads position state from on-chain accounts.

use crate::events::OnChainPosition;
use crate::rpc::RpcProvider;
use anyhow::{Context, Result};
use borsh::BorshDeserialize;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;
use std::sync::Arc;
use tracing::{debug, info};

/// Whirlpool position account structure.
#[derive(BorshDeserialize, Debug, Clone)]
pub struct PositionRewardInfo {
    /// Reward growth checkpoint inside the position range.
    pub growth_inside_checkpoint: u128,
    /// Reward amount owed to the position.
    pub amount_owed: u64,
}

/// Whirlpool position account structure.
#[derive(BorshDeserialize, Debug, Clone)]
pub struct WhirlpoolPosition {
    /// Account discriminator.
    pub discriminator: [u8; 8],
    /// The whirlpool this position belongs to.
    pub whirlpool: Pubkey,
    /// The position mint (NFT).
    pub position_mint: Pubkey,
    /// The liquidity in this position.
    pub liquidity: u128,
    /// Lower tick index.
    pub tick_lower_index: i32,
    /// Upper tick index.
    pub tick_upper_index: i32,
    /// Fee growth checkpoint for token A.
    pub fee_growth_checkpoint_a: u128,
    /// Fee owed for token A.
    pub fee_owed_a: u64,
    /// Fee growth checkpoint for token B.
    pub fee_growth_checkpoint_b: u128,
    /// Fee owed for token B.
    pub fee_owed_b: u64,
    /// Reward infos for each configured reward.
    /// Orca Whirlpool uses NUM_REWARDS=3 in `orca_whirlpools_core`.
    pub reward_infos: [PositionRewardInfo; 3],
}

/// Reads Orca Whirlpool positions from on-chain.
pub struct PositionReader {
    /// RPC provider.
    provider: Arc<RpcProvider>,
}

impl PositionReader {
    /// Creates a new position reader.
    pub fn new(provider: Arc<RpcProvider>) -> Self {
        Self { provider }
    }

    /// Gets a position by its address.
    pub async fn get_position(&self, position_address: &str) -> Result<OnChainPosition> {
        let pubkey = Pubkey::from_str(position_address).context("Invalid position address")?;

        info!(position = position_address, "Fetching position state");

        let account = self.provider.get_account(&pubkey).await?;
        let position = WhirlpoolPosition::try_from_slice(&account.data)
            .context("Failed to deserialize position account")?;

        debug!(
            liquidity = %position.liquidity,
            tick_lower = position.tick_lower_index,
            tick_upper = position.tick_upper_index,
            "Parsed position state"
        );

        Ok(OnChainPosition {
            address: pubkey,
            pool: position.whirlpool,
            owner: Pubkey::default(), // Owner needs to be fetched from token account
            tick_lower: position.tick_lower_index,
            tick_upper: position.tick_upper_index,
            liquidity: position.liquidity,
            fee_growth_inside_a: position.fee_growth_checkpoint_a,
            fee_growth_inside_b: position.fee_growth_checkpoint_b,
            fees_owed_a: position.fee_owed_a,
            fees_owed_b: position.fee_owed_b,
        })
    }

    /// Gets all positions for a given owner.
    ///
    /// This requires scanning token accounts for position NFTs.
    pub async fn get_positions_by_owner(&self, owner: &str) -> Result<Vec<OnChainPosition>> {
        let _owner_pubkey = Pubkey::from_str(owner).context("Invalid owner address")?;

        info!(owner = owner, "Fetching positions for owner");

        // TODO: Implement by:
        // 1. Get all token accounts for owner
        // 2. Filter for position NFT mints
        // 3. Derive position PDAs from mints
        // 4. Fetch position accounts

        Ok(vec![])
    }

    /// Gets positions for a specific pool.
    pub async fn get_positions_for_pool(
        &self,
        _pool_address: &str,
        _owner: &str,
    ) -> Result<Vec<OnChainPosition>> {
        // TODO: Implement by filtering owner positions by pool
        Ok(vec![])
    }

    /// Calculates the token amounts for a position.
    pub fn calculate_token_amounts(
        &self,
        position: &OnChainPosition,
        current_tick: i32,
        sqrt_price: u128,
    ) -> (u64, u64) {
        // If position is out of range, all liquidity is in one token
        if current_tick < position.tick_lower {
            // All in token A
            let amount_a =
                self.get_amount_a(position.liquidity, position.tick_lower, position.tick_upper);
            return (amount_a, 0);
        }

        if current_tick >= position.tick_upper {
            // All in token B
            let amount_b =
                self.get_amount_b(position.liquidity, position.tick_lower, position.tick_upper);
            return (0, amount_b);
        }

        // Position is in range - split between tokens
        let amount_a = self.get_amount_a_in_range(
            position.liquidity,
            current_tick,
            position.tick_upper,
            sqrt_price,
        );
        let amount_b = self.get_amount_b_in_range(
            position.liquidity,
            position.tick_lower,
            current_tick,
            sqrt_price,
        );

        (amount_a, amount_b)
    }

    /// Gets amount of token A when position is below current tick.
    fn get_amount_a(&self, liquidity: u128, tick_lower: i32, tick_upper: i32) -> u64 {
        let sqrt_price_lower = tick_to_sqrt_price(tick_lower);
        let sqrt_price_upper = tick_to_sqrt_price(tick_upper);

        // amount_a = L * (1/sqrt_price_lower - 1/sqrt_price_upper)
        let inv_lower = (1u128 << 64) / sqrt_price_lower;
        let inv_upper = (1u128 << 64) / sqrt_price_upper;

        let delta = inv_lower.saturating_sub(inv_upper);
        ((liquidity * delta) >> 64) as u64
    }

    /// Gets amount of token B when position is above current tick.
    fn get_amount_b(&self, liquidity: u128, tick_lower: i32, tick_upper: i32) -> u64 {
        let sqrt_price_lower = tick_to_sqrt_price(tick_lower);
        let sqrt_price_upper = tick_to_sqrt_price(tick_upper);

        // amount_b = L * (sqrt_price_upper - sqrt_price_lower)
        let delta = sqrt_price_upper.saturating_sub(sqrt_price_lower);
        ((liquidity * delta) >> 64) as u64
    }

    /// Gets amount of token A when in range.
    fn get_amount_a_in_range(
        &self,
        liquidity: u128,
        _current_tick: i32,
        tick_upper: i32,
        sqrt_price: u128,
    ) -> u64 {
        let sqrt_price_upper = tick_to_sqrt_price(tick_upper);

        let inv_current = (1u128 << 64) / sqrt_price;
        let inv_upper = (1u128 << 64) / sqrt_price_upper;

        let delta = inv_current.saturating_sub(inv_upper);
        ((liquidity * delta) >> 64) as u64
    }

    /// Gets amount of token B when in range.
    fn get_amount_b_in_range(
        &self,
        liquidity: u128,
        tick_lower: i32,
        _current_tick: i32,
        sqrt_price: u128,
    ) -> u64 {
        let sqrt_price_lower = tick_to_sqrt_price(tick_lower);

        let delta = sqrt_price.saturating_sub(sqrt_price_lower);
        ((liquidity * delta) >> 64) as u64
    }
}

/// Converts a tick to sqrt_price (Q64.64).
fn tick_to_sqrt_price(tick: i32) -> u128 {
    // sqrt_price = sqrt(1.0001^tick) * 2^64
    let base: f64 = 1.0001;
    let sqrt_price = base.powi(tick).sqrt() * (1u128 << 64) as f64;
    sqrt_price as u128
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tick_to_sqrt_price() {
        // tick 0 should give sqrt_price = 2^64
        let sqrt_price = tick_to_sqrt_price(0);
        let expected = 1u128 << 64;
        // Allow some floating point error
        assert!((sqrt_price as i128 - expected as i128).abs() < 1000);
    }
}
