use borsh::{BorshDeserialize, BorshSerialize};
use solana_sdk::pubkey::Pubkey;

/// Orca Whirlpool TickArray account layout (Anchor/Borsh) after discriminator.
///
/// We only need a small subset of tick fields (fee growth outside), but we deserialize the full
/// tick entries for correctness.
#[derive(BorshDeserialize, BorshSerialize, Debug, Clone)]
pub struct TickArrayAccountBody {
    /// Whirlpool address this tick array belongs to.
    pub whirlpool: Pubkey,
    /// Start tick index covered by this array.
    pub start_tick_index: i32,
    /// Tick entries (fixed-size array).
    pub ticks: [Tick; 88],
}

#[derive(BorshDeserialize, BorshSerialize, Debug, Clone, Copy)]
pub struct Tick {
    pub initialized: bool,
    pub liquidity_gross: u128,
    pub liquidity_net: i128,
    pub fee_growth_outside_a: u128,
    pub fee_growth_outside_b: u128,
    pub reward_growths_outside: [u128; 3],
}

#[derive(Debug, Clone, Copy)]
pub struct TickBoundaryState {
    pub fee_growth_outside_a: u128,
    pub fee_growth_outside_b: u128,
    pub initialized: bool,
}

/// Returns the tick array start index that contains `tick_index`.
#[must_use]
pub fn tick_array_start_index(tick_index: i32, tick_spacing: u16) -> i32 {
    let spacing = tick_spacing as i32;
    let array_size = spacing * 88;
    // floor division for negatives
    let q = if tick_index >= 0 {
        tick_index / array_size
    } else {
        -((-tick_index + array_size - 1) / array_size)
    };
    q * array_size
}

/// Returns the position inside the tick array (0..88) for `tick_index`.
#[must_use]
pub fn tick_array_offset(
    tick_index: i32,
    start_tick_index: i32,
    tick_spacing: u16,
) -> Option<usize> {
    let spacing = tick_spacing as i32;
    if spacing <= 0 {
        return None;
    }
    let rel = tick_index.checked_sub(start_tick_index)?;
    if rel % spacing != 0 {
        return None;
    }
    let idx = rel / spacing;
    if (0..88).contains(&idx) {
        Some(idx as usize)
    } else {
        None
    }
}

/// Compute `fee_growth_inside` for Whirlpool (single token) using global and boundary `fee_growth_outside`.
///
/// Semantics follow Orca Whirlpool:
/// - lower tick inclusive, upper tick exclusive
/// - computation branches on `tick_current` relative to boundaries
#[must_use]
pub fn compute_fee_growth_inside_single(
    fee_growth_global: u128,
    fee_growth_outside_lower: u128,
    fee_growth_outside_upper: u128,
    tick_current: i32,
    tick_lower: i32,
    tick_upper: i32,
) -> u128 {
    let fee_growth_below = if tick_current < tick_lower {
        fee_growth_global.wrapping_sub(fee_growth_outside_lower)
    } else {
        fee_growth_outside_lower
    };
    let fee_growth_above = if tick_current >= tick_upper {
        fee_growth_global.wrapping_sub(fee_growth_outside_upper)
    } else {
        fee_growth_outside_upper
    };
    fee_growth_global
        .wrapping_sub(fee_growth_below)
        .wrapping_sub(fee_growth_above)
}
