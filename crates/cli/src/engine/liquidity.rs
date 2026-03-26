use crate::backtest_engine::StepData;
use crate::engine::pricing::{
    clamp_quote_usd, from_base_units, price_ab_human_to_raw, price_to_q64, price_to_sqrt_q64,
    to_base_units,
};
use clmm_lp_domain::math::concentrated_liquidity::q64_64;
use primitive_types::U256;
use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};

/// Optional anchors for [`estimate_position_liquidity`].
///
/// By default, liquidity is estimated using **`step_data[0]`** (entry quote USD, entry A/B, entry
/// A/USD). After a rebalance, callers build `lower_usd` / `upper_usd` with the **current** step's
/// `quote_usd`; without overrides, dividing those USD bounds by the **entry** quote mis-scales the
/// implied A/B range and can yield `L = 0` or nonsense — which then zeros the HODL benchmark.
#[derive(Clone, Copy, Debug, Default)]
pub struct LiquidityEstimateOverrides {
    /// Quote token USD price for converting `lower_usd` / `upper_usd` / `capital_usd` to A/B.
    pub quote_usd: Option<Decimal>,
    /// A/B price used as the in-range anchor for `max_liquidity_for_value_in_range`.
    pub price_ab: Option<Decimal>,
    /// Token A price in USD for the normalize-to-capital step (`amt_a * price_a_usd + amt_b * quote`).
    pub price_a_usd: Option<Decimal>,
}

pub fn amounts_from_liquidity_at_price(
    liquidity: u128,
    sqrt_l: u128,
    sqrt_p: u128,
    sqrt_u: u128,
) -> (u64, u64) {
    // token0= A, token1= B. Q64.64 formulas (Uniswap v3 style).
    const Q64: u128 = 1u128 << 64;
    let liq = U256::from(liquidity);
    if liquidity == 0 {
        return (0, 0);
    }
    if sqrt_p <= sqrt_l {
        let num = liq * U256::from(sqrt_u.saturating_sub(sqrt_l)) * U256::from(Q64);
        let den = U256::from(sqrt_u) * U256::from(sqrt_l);
        let a0 = if den.is_zero() {
            U256::zero()
        } else {
            num / den
        };
        (a0.min(U256::from(u64::MAX)).as_u64(), 0u64)
    } else if sqrt_p >= sqrt_u {
        let num = liq * U256::from(sqrt_u.saturating_sub(sqrt_l));
        let a1 = (num / U256::from(Q64)).min(U256::from(u64::MAX)).as_u64();
        (0u64, a1)
    } else {
        let num0 = liq * U256::from(sqrt_u.saturating_sub(sqrt_p)) * U256::from(Q64);
        let den0 = U256::from(sqrt_u) * U256::from(sqrt_p);
        let a0 = if den0.is_zero() {
            U256::zero()
        } else {
            num0 / den0
        };
        let num1 = liq * U256::from(sqrt_p.saturating_sub(sqrt_l));
        let a1 = num1 / U256::from(Q64);
        (
            a0.min(U256::from(u64::MAX)).as_u64(),
            a1.min(U256::from(u64::MAX)).as_u64(),
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn normalize_liquidity_to_capital(
    liquidity: u128,
    sqrt_l: u128,
    sqrt_p: u128,
    sqrt_u: u128,
    price_a_usd: Decimal,
    quote_usd: Decimal,
    capital_usd: Decimal,
    token_a_decimals: u32,
    token_b_decimals: u32,
) -> u128 {
    if liquidity == 0 {
        return 0;
    }
    let (a0, a1) = amounts_from_liquidity_at_price(liquidity, sqrt_l, sqrt_p, sqrt_u);
    let amt_a = from_base_units(a0, token_a_decimals);
    let amt_b = from_base_units(a1, token_b_decimals);
    let value = (amt_a * price_a_usd) + (amt_b * quote_usd);
    if value <= Decimal::ZERO {
        return liquidity;
    }
    // Scale liquidity linearly so reconstructed value matches capital.
    // We clamp the scaling factor to avoid numerical blow-ups from bad inputs.
    let ratio_f = (capital_usd / value)
        .to_f64()
        .unwrap_or(1.0)
        .clamp(1e-18, 1e18);
    (Decimal::from(liquidity) * Decimal::from_f64(ratio_f).unwrap_or(Decimal::ONE))
        .round()
        .to_u128()
        .unwrap_or(liquidity)
}

/// Estimates initial position liquidity (L) for a given USD bounds and capital in USD.
///
/// Uses A/B prices for liquidity math by converting USD bounds to A/B via quote USD (entry step by
/// default; see [`LiquidityEstimateOverrides`]).
pub fn estimate_position_liquidity(
    step_data: &[StepData],
    lower_usd: Decimal,
    upper_usd: Decimal,
    capital_usd: Decimal,
    token_a_decimals: u32,
    token_b_decimals: u32,
) -> u128 {
    estimate_position_liquidity_with_overrides(
        step_data,
        lower_usd,
        upper_usd,
        capital_usd,
        token_a_decimals,
        token_b_decimals,
        LiquidityEstimateOverrides::default(),
    )
}

/// `max_liquidity_for_value_in_range` assumes an **interior** spot vs ticks; at an exact tick
/// boundary `min(L0,L1)` becomes zero. Nudge √P slightly inside `(√L, √U)` when needed.
fn clamp_sqrt_price_inside_range(sqrt_l: u128, sqrt_p: u128, sqrt_u: u128) -> u128 {
    if sqrt_u <= sqrt_l {
        return sqrt_p;
    }
    let span = sqrt_u.saturating_sub(sqrt_l);
    if span <= 1 {
        return sqrt_p;
    }
    let bump = (span / 1000).max(1);
    if sqrt_p <= sqrt_l {
        sqrt_l.saturating_add(bump).min(sqrt_u.saturating_sub(bump))
    } else if sqrt_p >= sqrt_u {
        sqrt_u.saturating_sub(bump).max(sqrt_l.saturating_add(bump))
    } else {
        sqrt_p
    }
}

/// Like [`estimate_position_liquidity`], but anchors quote / price to a specific step (e.g. rebalance).
pub fn estimate_position_liquidity_with_overrides(
    step_data: &[StepData],
    lower_usd: Decimal,
    upper_usd: Decimal,
    capital_usd: Decimal,
    token_a_decimals: u32,
    token_b_decimals: u32,
    overrides: LiquidityEstimateOverrides,
) -> u128 {
    let Some(first) = step_data.first() else {
        return 0;
    };
    let quote_usd = clamp_quote_usd(overrides.quote_usd.unwrap_or(first.quote_usd));
    let capital_in_token_b = capital_usd / quote_usd;
    let value_b_base = to_base_units(capital_in_token_b, token_b_decimals);

    let lower_ab = lower_usd / quote_usd;
    let upper_ab = upper_usd / quote_usd;
    let lower_ab_raw = price_ab_human_to_raw(lower_ab, token_a_decimals, token_b_decimals);
    let upper_ab_raw = price_ab_human_to_raw(upper_ab, token_a_decimals, token_b_decimals);
    let anchor_ab = overrides.price_ab.unwrap_or(first.price_ab.value);
    let entry_ab_raw = price_ab_human_to_raw(anchor_ab, token_a_decimals, token_b_decimals);

    let sqrt_l = price_to_sqrt_q64(lower_ab_raw);
    let sqrt_u = price_to_sqrt_q64(upper_ab_raw);
    let sqrt_p = price_to_sqrt_q64(entry_ab_raw);
    let sqrt_p_fit = clamp_sqrt_price_inside_range(sqrt_l, sqrt_p, sqrt_u);
    let p_q64 = price_to_q64(entry_ab_raw);
    let (l_raw, _, _) =
        q64_64::max_liquidity_for_value_in_range(value_b_base, p_q64, sqrt_l, sqrt_p_fit, sqrt_u);

    let price_a_usd = overrides.price_a_usd.unwrap_or(first.price_usd.value);

    // Anti-odlot: verify that liquidity corresponds to the intended USD capital at entry,
    // and scale if necessary.
    normalize_liquidity_to_capital(
        l_raw,
        sqrt_l,
        sqrt_p_fit,
        sqrt_u,
        price_a_usd,
        quote_usd,
        capital_usd,
        token_a_decimals,
        token_b_decimals,
    )
}

/// Estimates LP end amounts (token A and token B) from estimated position liquidity (L).
///
/// Returns (amount_a, amount_b, liquidity_L) in human units.
pub fn estimate_lp_end_amounts(
    step_data: &[StepData],
    lower_usd: Decimal,
    upper_usd: Decimal,
    capital_usd: Decimal,
    token_a_decimals: u32,
    token_b_decimals: u32,
) -> (Decimal, Decimal, u128) {
    let (Some(first), Some(last)) = (step_data.first(), step_data.last()) else {
        return (Decimal::ZERO, Decimal::ZERO, 0);
    };
    let quote_usd = clamp_quote_usd(first.quote_usd);
    let lower_ab = lower_usd / quote_usd;
    let upper_ab = upper_usd / quote_usd;
    let l_pos = estimate_position_liquidity(
        step_data,
        lower_usd,
        upper_usd,
        capital_usd,
        token_a_decimals,
        token_b_decimals,
    );
    if l_pos == 0 {
        return (Decimal::ZERO, Decimal::ZERO, 0);
    }

    let lower_ab_raw = price_ab_human_to_raw(lower_ab, token_a_decimals, token_b_decimals);
    let upper_ab_raw = price_ab_human_to_raw(upper_ab, token_a_decimals, token_b_decimals);
    let last_ab_raw =
        price_ab_human_to_raw(last.price_ab.value, token_a_decimals, token_b_decimals);

    let sqrt_l = price_to_sqrt_q64(lower_ab_raw);
    let sqrt_u = price_to_sqrt_q64(upper_ab_raw);
    let sqrt_p = price_to_sqrt_q64(last_ab_raw);

    let (amt0_base, amt1_base) = amounts_from_liquidity_at_price(l_pos, sqrt_l, sqrt_p, sqrt_u);

    (
        from_base_units(amt0_base, token_a_decimals),
        from_base_units(amt1_base, token_b_decimals),
        l_pos,
    )
}
