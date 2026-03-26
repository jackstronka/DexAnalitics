use borsh::{BorshDeserialize, BorshSerialize};
use solana_sdk::pubkey::Pubkey;

/// Reward slot in Whirlpool account (128 bytes each × 3 = 384).
#[derive(BorshDeserialize, BorshSerialize, Debug, Clone, Copy)]
pub struct WhirlpoolRewardInfoLayout {
    pub mint: Pubkey,
    pub vault: Pubkey,
    pub extension: [u8; 32],
    pub emissions_per_second_x64: u128,
    pub growth_global_x64: u128,
}

/// Full Whirlpool account body after the 8-byte Anchor discriminator (645 bytes).
/// Matches on-chain layout in `orca-so/whirlpools` (Whirlpool::LEN = 653 including disc).
#[derive(BorshDeserialize, BorshSerialize, Debug, Clone)]
pub struct WhirlpoolAccountBody {
    pub whirlpools_config: Pubkey,
    pub whirlpool_bump: [u8; 1],
    pub tick_spacing: u16,
    pub fee_tier_index_seed: [u8; 2],
    pub fee_rate: u16,
    pub protocol_fee_rate: u16,
    pub liquidity: u128,
    pub sqrt_price: u128,
    pub tick_current_index: i32,
    pub protocol_fee_owed_a: u64,
    pub protocol_fee_owed_b: u64,
    pub token_mint_a: Pubkey,
    pub token_vault_a: Pubkey,
    pub fee_growth_global_a: u128,
    pub token_mint_b: Pubkey,
    pub token_vault_b: Pubkey,
    pub fee_growth_global_b: u128,
    pub reward_last_updated_timestamp: u64,
    pub reward_infos: [WhirlpoolRewardInfoLayout; 3],
}

/// Parse a Whirlpool account using Borsh (preferred). `data` must include the 8-byte discriminator.
#[must_use]
pub fn parse_whirlpool_account_borsh(data: &[u8]) -> Option<WhirlpoolMinimal> {
    const WHIRLPOOL_TOTAL: usize = 653;
    if data.len() < WHIRLPOOL_TOTAL {
        return None;
    }
    let body = data.get(8..WHIRLPOOL_TOTAL)?;
    let parsed = WhirlpoolAccountBody::try_from_slice(body).ok()?;
    Some(WhirlpoolMinimal {
        tick_spacing: parsed.tick_spacing,
        fee_rate: parsed.fee_rate,
        protocol_fee_rate: parsed.protocol_fee_rate,
        liquidity: parsed.liquidity,
        sqrt_price: parsed.sqrt_price,
        tick_current_index: parsed.tick_current_index,
        protocol_fee_owed_a: parsed.protocol_fee_owed_a,
        protocol_fee_owed_b: parsed.protocol_fee_owed_b,
        token_mint_a: parsed.token_mint_a,
        token_vault_a: parsed.token_vault_a,
        fee_growth_global_a: parsed.fee_growth_global_a,
        token_mint_b: parsed.token_mint_b,
        token_vault_b: parsed.token_vault_b,
        fee_growth_global_b: parsed.fee_growth_global_b,
    })
}

// Simplification of Whirlpool Account Layout
// In reality, we would use the anchor-generated structs or a complete copy of the layout.
// For MVP, we define enough to read ticks and liquidity.

/// Represents an Orca Whirlpool account.
#[derive(BorshDeserialize, BorshSerialize, Debug, Clone)]
pub struct Whirlpool {
    /// Discriminator to identify the account type.
    pub discriminator: [u8; 8],
    /// The whirlpools config account.
    pub whirlpools_config: Pubkey,
    /// The bump seed for the whirlpool.
    pub whirlpool_bump: [u8; 1],
    /// The tick spacing.
    pub tick_spacing: u16,
    /// The tick spacing seed.
    pub tick_spacing_seed: [u8; 2],
    /// The fee rate.
    pub fee_rate: u16,
    /// The protocol fee rate.
    pub protocol_fee_rate: u16,
    /// The liquidity amount.
    pub liquidity: u128,
    /// The square root price.
    pub sqrt_price: u128,
    /// The current tick index.
    pub tick_current_index: i32,
    /// Protocol fee owed for token A.
    pub protocol_fee_owed_a: u64,
    /// Protocol fee owed for token B.
    pub protocol_fee_owed_b: u64,
    /// The mint of token A.
    pub token_mint_a: Pubkey,
    /// The vault for token A.
    pub token_vault_a: Pubkey,
    /// The fee growth global for token A.
    pub fee_growth_global_a: u128,
    /// The mint of token B.
    pub token_mint_b: Pubkey,
    /// The vault for token B.
    pub token_vault_b: Pubkey,
    /// The fee growth global for token B.
    pub fee_growth_global_b: u128,
    /// The last updated timestamp for rewards.
    pub reward_last_updated_timestamp: u64,
    // ... there are more fields (rewards, etc.)
    // Borsh deserialization fails if struct doesn't match exact bytes.
    // So we usually need the FULL struct or use a manual parser (unsafe pointer cast or byte slicing).
    // For safety in Rust, using the Anchor deserializer is best if we have the IDL.
    // Or we can skip bytes if we know offsets.
}

/// Minimal Whirlpool fields we need for analytics/backtests.
#[derive(Debug, Clone)]
pub struct WhirlpoolMinimal {
    pub tick_spacing: u16,
    pub fee_rate: u16,
    pub protocol_fee_rate: u16,
    pub liquidity: u128,
    pub sqrt_price: u128,
    pub tick_current_index: i32,
    pub protocol_fee_owed_a: u64,
    pub protocol_fee_owed_b: u64,
    pub token_mint_a: Pubkey,
    pub token_vault_a: Pubkey,
    pub fee_growth_global_a: u128,
    pub token_mint_b: Pubkey,
    pub token_vault_b: Pubkey,
    pub fee_growth_global_b: u128,
}

fn read_u16_le(data: &[u8], off: usize) -> Option<u16> {
    data.get(off..off + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
}
fn read_i32_le(data: &[u8], off: usize) -> Option<i32> {
    data.get(off..off + 4)
        .map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}
fn read_u64_le(data: &[u8], off: usize) -> Option<u64> {
    data.get(off..off + 8)
        .map(|b| u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
}
fn read_u128_le(data: &[u8], off: usize) -> Option<u128> {
    data.get(off..off + 16).map(|b| {
        u128::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9], b[10], b[11], b[12], b[13],
            b[14], b[15],
        ])
    })
}
fn read_pubkey(data: &[u8], off: usize) -> Option<Pubkey> {
    let bytes = data.get(off..off + 32)?;
    let arr: [u8; 32] = bytes.try_into().ok()?;
    Some(Pubkey::new_from_array(arr))
}

/// Parses Whirlpool account bytes using fixed offsets for early fields.
///
/// This avoids relying on a fully accurate Anchor-generated struct, while remaining stable
/// for the fields we need (fee rates, liquidity, sqrt price, tick, token mints).
///
/// Offsets are derived from the documented Whirlpool struct prefix:
/// - discriminator: 8
/// - whirlpools_config: 32
/// - whirlpool_bump: 1
/// - tick_spacing: 2
/// - tick_spacing_seed: 2
/// - fee_rate: 2
/// - protocol_fee_rate: 2
/// - liquidity: 16
/// - sqrt_price: 16
/// - tick_current_index: 4
/// - protocol_fee_owed_a: 8
/// - protocol_fee_owed_b: 8
/// - token_mint_a: 32
/// - token_vault_a: 32
/// - fee_growth_global_a: 16
/// - token_mint_b: 32
pub fn parse_whirlpool_minimal(data: &[u8]) -> Option<WhirlpoolMinimal> {
    // Prefix offsets
    let off_tick_spacing = 8 + 32 + 1;
    let tick_spacing = read_u16_le(data, off_tick_spacing)?;
    let off_fee_rate = off_tick_spacing + 2 + 2;
    let fee_rate = read_u16_le(data, off_fee_rate)?;
    let protocol_fee_rate = read_u16_le(data, off_fee_rate + 2)?;
    let off_liquidity = off_fee_rate + 2 + 2;
    let liquidity = read_u128_le(data, off_liquidity)?;
    let sqrt_price = read_u128_le(data, off_liquidity + 16)?;
    let tick_current_index = read_i32_le(data, off_liquidity + 32)?;

    let off_fee_owed_a = off_liquidity + 32 + 4;
    let fee_owed_a = read_u64_le(data, off_fee_owed_a)?;
    let fee_owed_b = read_u64_le(data, off_fee_owed_a + 8)?;

    let off_token_mint_a = off_fee_owed_a + 8 + 8;
    let token_mint_a = read_pubkey(data, off_token_mint_a)?;
    let off_token_vault_a = off_token_mint_a + 32;
    let token_vault_a = read_pubkey(data, off_token_vault_a)?;
    let off_fee_growth_a = off_token_vault_a + 32;
    let fee_growth_a = read_u128_le(data, off_fee_growth_a)?;
    let off_token_mint_b = off_fee_growth_a + 16;
    let token_mint_b = read_pubkey(data, off_token_mint_b)?;
    let off_token_vault_b = off_token_mint_b + 32;
    let token_vault_b = read_pubkey(data, off_token_vault_b)?;
    let off_fee_growth_b = off_token_vault_b + 32;
    let fee_growth_b = read_u128_le(data, off_fee_growth_b)?;

    Some(WhirlpoolMinimal {
        tick_spacing,
        fee_rate,
        protocol_fee_rate,
        liquidity,
        sqrt_price,
        tick_current_index,
        protocol_fee_owed_a: fee_owed_a,
        protocol_fee_owed_b: fee_owed_b,
        token_mint_a,
        token_vault_a,
        fee_growth_global_a: fee_growth_a,
        token_mint_b,
        token_vault_b,
        fee_growth_global_b: fee_growth_b,
    })
}
