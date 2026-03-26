use crate::token::TokenAmount;
use primitive_types::U256;
use rust_decimal::Decimal;
use rust_decimal::prelude::*;

const Q64: u128 = 1u128 << 64;

/// Calculates the amount of token0 (x) given liquidity and price range.
/// delta_x = L * (1/sqrt(P_a) - 1/sqrt(P_b))
/// where P_a < P_b
pub fn get_amount0_delta(
    liquidity: u128,
    sqrt_price_a: Decimal,
    sqrt_price_b: Decimal,
) -> Result<TokenAmount, &'static str> {
    if sqrt_price_a <= Decimal::ZERO || sqrt_price_b <= Decimal::ZERO {
        return Err("Sqrt price must be positive");
    }

    let (lower, upper) = if sqrt_price_a < sqrt_price_b {
        (sqrt_price_a, sqrt_price_b)
    } else {
        (sqrt_price_b, sqrt_price_a)
    };

    // delta_x = L * ( (upper - lower) / (lower * upper) )
    // using rust_decimal for precision

    let liquidity_dec = Decimal::from(liquidity);

    let num = upper - lower;
    let den = lower * upper;

    if den.is_zero() {
        return Err("Denominator zero");
    }

    let factor = num / den;
    let amount = liquidity_dec * factor;

    let amount_u128 = amount.to_u128().ok_or("Overflow converting amount")?;
    Ok(TokenAmount::from(amount_u128))
}

/// Calculates the amount of token1 (y) given liquidity and price range.
/// delta_y = L * (sqrt(P_b) - sqrt(P_a))
/// where P_a < P_b
pub fn get_amount1_delta(
    liquidity: u128,
    sqrt_price_a: Decimal,
    sqrt_price_b: Decimal,
) -> Result<TokenAmount, &'static str> {
    let (lower, upper) = if sqrt_price_a < sqrt_price_b {
        (sqrt_price_a, sqrt_price_b)
    } else {
        (sqrt_price_b, sqrt_price_a)
    };

    let liquidity_dec = Decimal::from(liquidity);
    let diff = upper - lower;

    let amount = liquidity_dec * diff;

    let amount_u128 = amount.to_u128().ok_or("Overflow converting amount")?;
    Ok(TokenAmount::from(amount_u128))
}

/// Calculates liquidity for a given amount of token0 and price range
/// L = amount0 * (sqrt(P_a) * sqrt(P_b)) / (sqrt(P_b) - sqrt(P_a))
pub fn get_liquidity_for_amount0(
    amount0: TokenAmount,
    sqrt_price_a: Decimal,
    sqrt_price_b: Decimal,
) -> Result<u128, &'static str> {
    let (lower, upper) = if sqrt_price_a < sqrt_price_b {
        (sqrt_price_a, sqrt_price_b)
    } else {
        (sqrt_price_b, sqrt_price_a)
    };

    let amount0_dec = Decimal::from_str(&amount0.0.to_string()).map_err(|_| "Conversion error")?;

    let num = amount0_dec * lower * upper;
    let den = upper - lower;

    if den.is_zero() {
        return Err("Range too small");
    }

    let liquidity = num / den;
    liquidity.to_u128().ok_or("Overflow")
}

/// Calculates liquidity for a given amount of token1 and price range
/// L = amount1 / (sqrt(P_b) - sqrt(P_a))
pub fn get_liquidity_for_amount1(
    amount1: TokenAmount,
    sqrt_price_a: Decimal,
    sqrt_price_b: Decimal,
) -> Result<u128, &'static str> {
    let (lower, upper) = if sqrt_price_a < sqrt_price_b {
        (sqrt_price_a, sqrt_price_b)
    } else {
        (sqrt_price_b, sqrt_price_a)
    };

    let amount1_dec = Decimal::from_str(&amount1.0.to_string()).map_err(|_| "Conversion error")?;

    let den = upper - lower;
    if den.is_zero() {
        return Err("Range too small");
    }

    let liquidity = amount1_dec / den;
    liquidity.to_u128().ok_or("Overflow")
}

/// Integer CLMM math using Whirlpool-style Q64.64 sqrt prices.
///
/// These helpers are designed so the resulting liquidity `L` is comparable to
/// on-chain Whirlpool liquidity (u128), enabling realistic fee share:
/// `fee_share = position_L / pool_active_L`.
pub mod q64_64 {
    use super::{Q64, U256};

    /// Computes liquidity from amount0 (token A) for a full range [sqrt_lower, sqrt_upper].
    ///
    /// Formula (Uniswap v3):
    /// \( amount0 = L * (sqrtU - sqrtL) / (sqrtU * sqrtL) \)
    /// => \( L = amount0 * (sqrtU * sqrtL) / (sqrtU - sqrtL) \)
    ///
    /// `sqrt_*` are Q64.64, `amount0` is in base units.
    pub fn liquidity_from_amount0(amount0: u64, sqrt_lower: u128, sqrt_upper: u128) -> u128 {
        if amount0 == 0 || sqrt_lower == 0 || sqrt_upper == 0 {
            return 0;
        }
        let (sl, su) = if sqrt_lower < sqrt_upper {
            (sqrt_lower, sqrt_upper)
        } else {
            (sqrt_upper, sqrt_lower)
        };
        let delta = su.saturating_sub(sl);
        if delta == 0 {
            return 0;
        }
        let num = U256::from(amount0) * U256::from(sl) * U256::from(su);
        let liq = num / U256::from(delta);
        liq.min(U256::from(u128::MAX)).as_u128()
    }

    /// Computes liquidity from amount1 (token B) for a full range [sqrt_lower, sqrt_upper].
    ///
    /// Formula:
    /// \( amount1 = L * (sqrtU - sqrtL) \)
    /// In Q64.64: amount1 = L * (sqrtU - sqrtL) / Q64
    /// => L = amount1 * Q64 / (sqrtU - sqrtL)
    pub fn liquidity_from_amount1(amount1: u64, sqrt_lower: u128, sqrt_upper: u128) -> u128 {
        if amount1 == 0 {
            return 0;
        }
        let (sl, su) = if sqrt_lower < sqrt_upper {
            (sqrt_lower, sqrt_upper)
        } else {
            (sqrt_upper, sqrt_lower)
        };
        let delta = su.saturating_sub(sl);
        if delta == 0 {
            return 0;
        }
        let num = U256::from(amount1) * U256::from(Q64);
        let liq = num / U256::from(delta);
        liq.min(U256::from(u128::MAX)).as_u128()
    }

    /// Computes liquidity from amount0 given current sqrt price inside the range.
    ///
    /// Formula:
    /// amount0 = L * (sqrtU - sqrtP) / (sqrtU * sqrtP)
    /// => L = amount0 * sqrtU * sqrtP / (sqrtU - sqrtP)
    pub fn liquidity_from_amount0_in_range(amount0: u64, sqrt_p: u128, sqrt_upper: u128) -> u128 {
        if amount0 == 0 || sqrt_p == 0 || sqrt_upper == 0 {
            return 0;
        }
        let (sp, su) = if sqrt_p < sqrt_upper {
            (sqrt_p, sqrt_upper)
        } else {
            (sqrt_upper, sqrt_p)
        };
        let delta = su.saturating_sub(sp);
        if delta == 0 {
            return 0;
        }
        let num = U256::from(amount0) * U256::from(sp) * U256::from(su);
        let liq = num / U256::from(delta);
        liq.min(U256::from(u128::MAX)).as_u128()
    }

    /// Computes liquidity from amount1 given current sqrt price inside the range.
    ///
    /// Formula:
    /// amount1 = L * (sqrtP - sqrtL) / Q64
    /// => L = amount1 * Q64 / (sqrtP - sqrtL)
    pub fn liquidity_from_amount1_in_range(amount1: u64, sqrt_lower: u128, sqrt_p: u128) -> u128 {
        if amount1 == 0 {
            return 0;
        }
        let (sl, sp) = if sqrt_lower < sqrt_p {
            (sqrt_lower, sqrt_p)
        } else {
            (sqrt_p, sqrt_lower)
        };
        let delta = sp.saturating_sub(sl);
        if delta == 0 {
            return 0;
        }
        let num = U256::from(amount1) * U256::from(Q64);
        let liq = num / U256::from(delta);
        liq.min(U256::from(u128::MAX)).as_u128()
    }

    /// Computes the maximum liquidity attainable for a given total value in token1 units
    /// when the current price is inside the range.
    ///
    /// Inputs:
    /// - `value_token1`: total capital denominated in token1 **base units** (e.g. SOL lamports).
    /// - `price_x_in_y_q64`: current price as Q64.64 where `P = token1/token0`.
    /// - `sqrt_lower/sqrt_p/sqrt_upper`: Q64.64 sqrt prices.
    ///
    /// Returns: (liquidity_L, amount0_used, amount1_used)
    ///
    /// We binary-search the token1 allocation `amount1` so that liquidity derived from
    /// amount0 equals liquidity derived from amount1 (maximizing min(L0,L1)).
    pub fn max_liquidity_for_value_in_range(
        value_token1: u64,
        price_x_in_y_q64: u128,
        sqrt_lower: u128,
        sqrt_p: u128,
        sqrt_upper: u128,
    ) -> (u128, u64, u64) {
        if value_token1 == 0 || price_x_in_y_q64 == 0 {
            return (0, 0, 0);
        }

        // Convert token1 value into token0 budget via x = (V - y) / P.
        // P is Q64.64 => x = ((V - y) * Q64) / P_q64
        let mut lo_y: u64 = 0;
        let mut hi_y: u64 = value_token1;

        let mut best = (0u128, 0u64, 0u64);
        for _ in 0..64 {
            let mid_y = lo_y + ((hi_y - lo_y) / 2);
            let rem_y = value_token1 - mid_y;
            let x = ((U256::from(rem_y) * U256::from(Q64)) / U256::from(price_x_in_y_q64))
                .min(U256::from(u64::MAX))
                .as_u64();

            let l0 = liquidity_from_amount0_in_range(x, sqrt_p, sqrt_upper);
            let l1 = liquidity_from_amount1_in_range(mid_y, sqrt_lower, sqrt_p);
            let l = l0.min(l1);
            if l > best.0 {
                best = (l, x, mid_y);
            }

            // Move towards equality l0 ~= l1.
            if l0 > l1 {
                // Too much token0 (or too little token1) -> increase y
                lo_y = mid_y.saturating_add(1);
            } else {
                hi_y = mid_y;
            }
            if lo_y >= hi_y {
                break;
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_amount_deltas() {
        // Example: Liquidity 1000
        // Price goes from 1 to 4 (sqrt: 1 to 2)
        // delta_y = 1000 * (2 - 1) = 1000
        // delta_x = 1000 * (1/1 - 1/2) = 1000 * 0.5 = 500

        let liquidity = 1000u128;
        let sqrt_p_a = Decimal::from(1);
        let sqrt_p_b = Decimal::from(2);

        let dy = get_amount1_delta(liquidity, sqrt_p_a, sqrt_p_b).unwrap();
        assert_eq!(dy.as_u256().as_u64(), 1000);

        let dx = get_amount0_delta(liquidity, sqrt_p_a, sqrt_p_b).unwrap();
        assert_eq!(dx.as_u256().as_u64(), 500);
    }

    #[test]
    fn test_get_liquidity() {
        let sqrt_p_a = Decimal::from(1);
        let sqrt_p_b = Decimal::from(2);

        // From previous test: if dx = 500, L should be 1000
        let dx = TokenAmount::from(500u64);
        let l = get_liquidity_for_amount0(dx, sqrt_p_a, sqrt_p_b).unwrap();
        assert_eq!(l, 1000);

        // If dy = 1000, L should be 1000
        let dy = TokenAmount::from(1000u64);
        let l2 = get_liquidity_for_amount1(dy, sqrt_p_a, sqrt_p_b).unwrap();
        assert_eq!(l2, 1000);
    }
}
