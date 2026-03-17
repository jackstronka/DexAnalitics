//! Orca Whirlpool pool reader.
//!
//! Reads pool state from on-chain accounts.

use super::whirlpool::{Whirlpool, parse_whirlpool_minimal};
use crate::rpc::RpcProvider;
use anyhow::{Context, Result};
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;
use std::sync::Arc;
use tracing::{debug, info};

/// Orca Whirlpool program ID.
pub const WHIRLPOOL_PROGRAM_ID: &str = "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc";

/// Reads Orca Whirlpool pool state from on-chain.
pub struct WhirlpoolReader {
    /// RPC provider.
    provider: Arc<RpcProvider>,
}

impl WhirlpoolReader {
    /// Creates a new Whirlpool reader.
    pub fn new(provider: Arc<RpcProvider>) -> Self {
        Self { provider }
    }

    /// Gets the pool state for a given pool address.
    ///
    /// # Arguments
    /// * `pool_address` - The pool account address
    ///
    /// # Returns
    /// The deserialized Whirlpool state
    pub async fn get_pool_state(&self, pool_address: &str) -> Result<WhirlpoolState> {
        let pubkey = Pubkey::from_str(pool_address).context("Invalid pool address")?;

        info!(pool = pool_address, "Fetching Whirlpool state");

        let account = self.provider.get_account(&pubkey).await?;
        // Prefer manual parsing for stable offsets of required fields; fall back to Borsh if needed.
        let minimal = parse_whirlpool_minimal(&account.data)
            .context("Failed to parse Whirlpool account (minimal)")?;
        let whirlpool = Whirlpool {
            discriminator: [0u8; 8],
            whirlpools_config: Pubkey::default(),
            whirlpool_bump: [0u8; 1],
            tick_spacing: minimal.tick_spacing,
            tick_spacing_seed: [0u8; 2],
            fee_rate: minimal.fee_rate,
            protocol_fee_rate: minimal.protocol_fee_rate,
            liquidity: minimal.liquidity,
            sqrt_price: minimal.sqrt_price,
            tick_current_index: minimal.tick_current_index,
            protocol_fee_owed_a: 0,
            protocol_fee_owed_b: 0,
            token_mint_a: minimal.token_mint_a,
            token_vault_a: Pubkey::default(),
            fee_growth_global_a: 0,
            token_mint_b: minimal.token_mint_b,
            token_vault_b: Pubkey::default(),
            fee_growth_global_b: 0,
            reward_last_updated_timestamp: 0,
        };

        debug!(
            tick = whirlpool.tick_current_index,
            liquidity = %whirlpool.liquidity,
            "Parsed Whirlpool state"
        );

        Ok(WhirlpoolState::from_whirlpool(&whirlpool, pool_address))
    }

    /// Gets the current price from a pool.
    pub async fn get_current_price(&self, pool_address: &str) -> Result<Decimal> {
        let state = self.get_pool_state(pool_address).await?;
        Ok(state.price)
    }

    /// Gets the current tick from a pool.
    pub async fn get_current_tick(&self, pool_address: &str) -> Result<i32> {
        let state = self.get_pool_state(pool_address).await?;
        Ok(state.tick_current)
    }

    /// Gets the liquidity at the current tick.
    pub async fn get_liquidity(&self, pool_address: &str) -> Result<u128> {
        let state = self.get_pool_state(pool_address).await?;
        Ok(state.liquidity)
    }

    /// Gets multiple pool states in a single batch.
    pub async fn get_multiple_pools(&self, addresses: &[&str]) -> Result<Vec<WhirlpoolState>> {
        let pubkeys: Vec<Pubkey> = addresses
            .iter()
            .filter_map(|a| Pubkey::from_str(a).ok())
            .collect();

        let accounts = self.provider.get_multiple_accounts(&pubkeys).await?;

        let mut states = Vec::new();
        for (i, account_opt) in accounts.into_iter().enumerate() {
            let Some(account) = account_opt else { continue };
            if let Some(minimal) = parse_whirlpool_minimal(&account.data) {
                let whirlpool = Whirlpool {
                    discriminator: [0u8; 8],
                    whirlpools_config: Pubkey::default(),
                    whirlpool_bump: [0u8; 1],
                    tick_spacing: minimal.tick_spacing,
                    tick_spacing_seed: [0u8; 2],
                    fee_rate: minimal.fee_rate,
                    protocol_fee_rate: minimal.protocol_fee_rate,
                    liquidity: minimal.liquidity,
                    sqrt_price: minimal.sqrt_price,
                    tick_current_index: minimal.tick_current_index,
                    protocol_fee_owed_a: 0,
                    protocol_fee_owed_b: 0,
                    token_mint_a: minimal.token_mint_a,
                    token_vault_a: Pubkey::default(),
                    fee_growth_global_a: 0,
                    token_mint_b: minimal.token_mint_b,
                    token_vault_b: Pubkey::default(),
                    fee_growth_global_b: 0,
                    reward_last_updated_timestamp: 0,
                };
                states.push(WhirlpoolState::from_whirlpool(&whirlpool, addresses[i]));
            }
        }

        Ok(states)
    }
}

/// Parsed Whirlpool state.
#[derive(Debug, Clone)]
pub struct WhirlpoolState {
    /// Pool address.
    pub address: String,
    /// Token A mint.
    pub token_mint_a: Pubkey,
    /// Token B mint.
    pub token_mint_b: Pubkey,
    /// Current tick index.
    pub tick_current: i32,
    /// Tick spacing.
    pub tick_spacing: u16,
    /// Current sqrt price (Q64.64).
    pub sqrt_price: u128,
    /// Current price (derived from sqrt_price).
    pub price: Decimal,
    /// Current liquidity.
    pub liquidity: u128,
    /// Fee rate in basis points.
    pub fee_rate_bps: u16,
    /// Protocol fee rate in basis points.
    pub protocol_fee_rate_bps: u16,
    /// Fee growth global for token A.
    pub fee_growth_global_a: u128,
    /// Fee growth global for token B.
    pub fee_growth_global_b: u128,
}

impl WhirlpoolState {
    /// Creates a WhirlpoolState from a deserialized Whirlpool.
    fn from_whirlpool(wp: &Whirlpool, address: &str) -> Self {
        Self {
            address: address.to_string(),
            token_mint_a: wp.token_mint_a,
            token_mint_b: wp.token_mint_b,
            tick_current: wp.tick_current_index,
            tick_spacing: wp.tick_spacing,
            sqrt_price: wp.sqrt_price,
            price: sqrt_price_to_price(wp.sqrt_price),
            liquidity: wp.liquidity,
            fee_rate_bps: wp.fee_rate,
            protocol_fee_rate_bps: wp.protocol_fee_rate,
            fee_growth_global_a: wp.fee_growth_global_a,
            fee_growth_global_b: wp.fee_growth_global_b,
        }
    }

    /// Returns the fee rate as a decimal.
    #[must_use]
    pub fn fee_rate(&self) -> Decimal {
        // In Whirlpool accounts, `fee_rate` is stored as **hundredths of a basis point**
        // (1e-6). Example: 0.05% = 5 bps = 500 hundredths-bp.
        // So: decimal = fee_rate / 1_000_000.
        Decimal::from(self.fee_rate_bps) / Decimal::from(1_000_000)
    }

    /// Checks if a tick is within the current range.
    #[must_use]
    pub fn is_tick_in_range(&self, tick_lower: i32, tick_upper: i32) -> bool {
        self.tick_current >= tick_lower && self.tick_current < tick_upper
    }
}

/// Converts sqrt_price (Q64.64) to a human-readable price.
///
/// sqrt_price is stored as a Q64.64 fixed-point number.
/// price = (sqrt_price / 2^64)^2
fn sqrt_price_to_price(sqrt_price: u128) -> Decimal {
    // sqrt_price is Q64.64, so we need to divide by 2^64
    let sqrt_price_f64 = sqrt_price as f64 / (1u128 << 64) as f64;
    let price = sqrt_price_f64 * sqrt_price_f64;

    Decimal::from_f64(price).unwrap_or(Decimal::ZERO)
}

/// Converts a tick index to a price.
///
/// price = 1.0001^tick
#[must_use]
pub fn tick_to_price(tick: i32) -> Decimal {
    let base: f64 = 1.0001;
    let price = base.powi(tick);
    Decimal::from_f64(price).unwrap_or(Decimal::ZERO)
}

/// Converts a price to the nearest tick index.
///
/// tick = log(price) / log(1.0001)
#[must_use]
pub fn price_to_tick(price: Decimal) -> i32 {
    let price_f64 = price.to_string().parse::<f64>().unwrap_or(1.0);
    let tick = price_f64.ln() / 1.0001_f64.ln();
    tick.round() as i32
}

/// Calculates the tick range for a given price and width percentage.
#[must_use]
pub fn calculate_tick_range(
    current_tick: i32,
    width_pct: Decimal,
    tick_spacing: u16,
) -> (i32, i32) {
    let width_f64 = width_pct.to_string().parse::<f64>().unwrap_or(0.1);

    // Calculate tick delta based on width
    // width_pct = (upper_price - lower_price) / current_price
    // For symmetric range: upper = current * (1 + width/2), lower = current * (1 - width/2)
    let half_width = width_f64 / 2.0;

    // tick_delta ≈ width_pct / (2 * ln(1.0001))
    let tick_delta = (half_width / 1.0001_f64.ln()).abs() as i32;

    // Align to tick spacing
    let spacing = tick_spacing as i32;
    let lower = ((current_tick - tick_delta) / spacing) * spacing;
    let upper = ((current_tick + tick_delta) / spacing + 1) * spacing;

    (lower, upper)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tick_to_price() {
        // tick 0 should give price 1.0
        let price = tick_to_price(0);
        assert!((price - Decimal::ONE).abs() < Decimal::from_f64(0.0001).unwrap());

        // Positive tick should give price > 1
        let price = tick_to_price(1000);
        assert!(price > Decimal::ONE);

        // Negative tick should give price < 1
        let price = tick_to_price(-1000);
        assert!(price < Decimal::ONE);
    }

    #[test]
    fn test_price_to_tick() {
        let tick = price_to_tick(Decimal::ONE);
        assert_eq!(tick, 0);

        let tick = price_to_tick(Decimal::from_f64(1.1052).unwrap());
        assert!(tick > 0);
    }

    #[test]
    fn test_calculate_tick_range() {
        let (lower, upper) = calculate_tick_range(0, Decimal::from_f64(0.1).unwrap(), 64);

        // Should be symmetric around 0
        assert!(lower < 0);
        assert!(upper > 0);

        // Should be aligned to tick spacing
        assert_eq!(lower % 64, 0);
        assert_eq!(upper % 64, 0);
    }
}
