use crate::backtest_engine::StepData;
use crate::engine::pricing::{clamp_quote_usd, from_base_units, price_to_q64, price_to_sqrt_q64, to_base_units};
use clmm_lp_domain::math::concentrated_liquidity::q64_64;
use primitive_types::U256;
use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};

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
        let a0 = if den.is_zero() { U256::zero() } else { num / den };
        (a0.min(U256::from(u64::MAX)).as_u64(), 0u64)
    } else if sqrt_p >= sqrt_u {
        let num = liq * U256::from(sqrt_u.saturating_sub(sqrt_l));
        let a1 = (num / U256::from(Q64)).min(U256::from(u64::MAX)).as_u64();
        (0u64, a1)
    } else {
        let num0 = liq * U256::from(sqrt_u.saturating_sub(sqrt_p)) * U256::from(Q64);
        let den0 = U256::from(sqrt_u) * U256::from(sqrt_p);
        let a0 = if den0.is_zero() { U256::zero() } else { num0 / den0 };
        let num1 = liq * U256::from(sqrt_p.saturating_sub(sqrt_l));
        let a1 = num1 / U256::from(Q64);
        (
            a0.min(U256::from(u64::MAX)).as_u64(),
            a1.min(U256::from(u64::MAX)).as_u64(),
        )
    }
}

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
    // If reconstructed value deviates a lot, scale liquidity linearly to match capital.
    let ratio = (capital_usd / value).to_f64().unwrap_or(1.0);
    if !(0.01..=100.0).contains(&ratio) {
        // extreme mismatch, keep original (and let outer sanity prints show the issue)
        return liquidity;
    }
    let scaled = (Decimal::from(liquidity) * Decimal::from_f64(ratio).unwrap_or(Decimal::ONE))
        .round()
        .to_u128()
        .unwrap_or(liquidity);
    scaled
}

/// Estimates initial position liquidity (L) for a given USD bounds and capital in USD.
///
/// Uses A/B prices for liquidity math by converting USD bounds to A/B via entry quote USD.
pub fn estimate_position_liquidity(
    step_data: &[StepData],
    lower_usd: Decimal,
    upper_usd: Decimal,
    capital_usd: Decimal,
    token_a_decimals: u32,
    token_b_decimals: u32,
) -> u128 {
    let Some(first) = step_data.first() else {
        return 0;
    };
    let quote_usd = clamp_quote_usd(first.quote_usd);
    let capital_in_token_b = capital_usd / quote_usd;
    let value_b_base = to_base_units(capital_in_token_b, token_b_decimals);

    let lower_ab = lower_usd / quote_usd;
    let upper_ab = upper_usd / quote_usd;
    let sqrt_l = price_to_sqrt_q64(lower_ab);
    let sqrt_u = price_to_sqrt_q64(upper_ab);
    let sqrt_p = price_to_sqrt_q64(first.price_ab.value);
    let p_q64 = price_to_q64(first.price_ab.value);
    let (l_raw, _, _) = q64_64::max_liquidity_for_value_in_range(value_b_base, p_q64, sqrt_l, sqrt_p, sqrt_u);

    // Anti-odlot: verify that liquidity corresponds to the intended USD capital at entry,
    // and scale if necessary.
    let l = normalize_liquidity_to_capital(
        l_raw,
        sqrt_l,
        sqrt_p,
        sqrt_u,
        first.price_usd.value,
        quote_usd,
        capital_usd,
        token_a_decimals,
        token_b_decimals,
    );
    l
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
    let l_pos = estimate_position_liquidity(step_data, lower_usd, upper_usd, capital_usd, token_a_decimals, token_b_decimals);
    if l_pos == 0 {
        return (Decimal::ZERO, Decimal::ZERO, 0);
    }

    let sqrt_l = price_to_sqrt_q64(lower_ab);
    let sqrt_u = price_to_sqrt_q64(upper_ab);
    let sqrt_p = price_to_sqrt_q64(last.price_ab.value);

    let (amt0_base, amt1_base) = amounts_from_liquidity_at_price(l_pos, sqrt_l, sqrt_p, sqrt_u);

    (
        from_base_units(amt0_base, token_a_decimals),
        from_base_units(amt1_base, token_b_decimals),
        l_pos,
    )
}

