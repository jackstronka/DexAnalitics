//! Target **USD notional** for an **in-range** Orca deposit: derive `token_max_a/b` from liquidity math.
//!
//! The UI used to split budget 50/50 in USD and send those as caps; the curve often only consumes
//! ~one leg, so on-chain value looked ~half of the entered budget. Here we pick liquidity `L` so
//! `amount_a(L) * p_a + amount_b(L) * p_b ≈ target_usd` (from below, integer `L`).

use primitive_types::U256;

/// Q64.64 sqrt price from tick (same convention as [`super::position_reader`]).
fn tick_to_sqrt_price(tick: i32) -> u128 {
    let base: f64 = 1.0001;
    let sqrt_price = base.powi(tick).sqrt() * (1u128 << 64) as f64;
    sqrt_price as u128
}

/// Token A amount for in-range liquidity: `L * (1/√Pc - 1/√Pu)` with Q64.64 `√P` values.
///
/// Using `floor(2^64/√Pc) - floor(2^64/√Pu)` collapses to 0 when `√Pc` and `√Pu` map to the same
/// inverse floor (common for modest tick distance). Exact form:
/// `L * (√Pu - √Pc) * 2^64 / (√Pc * √Pu)`.
fn amount_a_in_range_q64(liquidity: u128, sqrt_current: u128, sqrt_upper: u128) -> u64 {
    if sqrt_current == 0 || sqrt_upper == 0 || sqrt_upper <= sqrt_current {
        return 0;
    }
    let l = U256::from(liquidity);
    let sc = U256::from(sqrt_current);
    let su = U256::from(sqrt_upper);
    let diff = su - sc;
    if diff.is_zero() {
        return 0;
    }
    let num = l * diff * U256::from(1u128 << 64);
    let den = sc * su;
    if den.is_zero() {
        return 0;
    }
    let q = num / den;
    let cap = U256::from(u64::MAX);
    if q > cap { u64::MAX } else { q.as_u64() }
}

/// Token amounts (raw) for liquidity `L` when spot is inside `[tick_lower, tick_upper)`.
fn token_amounts_in_range(
    liquidity: u128,
    sqrt_price: u128,
    tick_lower: i32,
    tick_upper: i32,
) -> (u64, u64) {
    let sqrt_price_upper = tick_to_sqrt_price(tick_upper);
    let amount_a = amount_a_in_range_q64(liquidity, sqrt_price, sqrt_price_upper);

    let sqrt_price_lower = tick_to_sqrt_price(tick_lower);
    let delta_b = sqrt_price.saturating_sub(sqrt_price_lower);
    let amount_b = ((liquidity.saturating_mul(delta_b)) >> 64) as u64;

    (amount_a, amount_b)
}

fn raw_pair_usd(
    a: u64,
    b: u64,
    decimals_a: u8,
    decimals_b: u8,
    price_a_usd: f64,
    price_b_usd: f64,
) -> f64 {
    let a_ui = a as f64 / 10f64.powi(i32::from(decimals_a));
    let b_ui = b as f64 / 10f64.powi(i32::from(decimals_b));
    a_ui * price_a_usd + b_ui * price_b_usd
}

/// Small bump so `token_max` clears rounding vs Whirlpool SDK (still caps, unused refunded).
fn bump_raw_cap(x: u64) -> u64 {
    let extra = (x / 200).max(1);
    x.saturating_add(extra)
}

/// Quote result: use `token_max_*` as Orca `token_max_a` / `token_max_b`.
#[derive(Debug, Clone, PartialEq)]
pub struct DepositBudgetQuote {
    /// Raw amounts at chosen `liquidity` (before small cap bump).
    pub amount_a: u64,
    pub amount_b: u64,
    pub token_max_a: u64,
    pub token_max_b: u64,
    /// Estimated USD at quoted prices (slightly below `target_usd` due to discrete `L`).
    pub estimated_value_usd: f64,
    pub liquidity: u128,
}

/// Returns caps targeting up to `target_usd` when the pool price sits **inside** the position range.
///
/// Requires positive finite USD prices for **both** tokens (caller should pin stables to ~1).
#[allow(clippy::too_many_arguments)]
pub fn quote_deposit_budget_in_range(
    tick_lower: i32,
    tick_upper: i32,
    tick_current: i32,
    sqrt_price: u128,
    decimals_a: u8,
    decimals_b: u8,
    price_a_usd: f64,
    price_b_usd: f64,
    target_usd: f64,
) -> Result<DepositBudgetQuote, &'static str> {
    if tick_lower >= tick_upper {
        return Err("tick_lower must be < tick_upper");
    }
    if !(tick_current >= tick_lower && tick_current < tick_upper) {
        return Err(
            "current_tick must satisfy tick_lower <= current_tick < tick_upper (price in range)",
        );
    }
    if !(target_usd.is_finite() && target_usd > 0.0) {
        return Err("target_usd must be a finite positive number");
    }
    if !(price_a_usd.is_finite()
        && price_b_usd.is_finite()
        && price_a_usd > 0.0
        && price_b_usd > 0.0)
    {
        return Err("price_a_usd and price_b_usd must be finite and positive");
    }

    let usd_for_l = |l: u128| -> f64 {
        let (a, b) = token_amounts_in_range(l, sqrt_price, tick_lower, tick_upper);
        raw_pair_usd(a, b, decimals_a, decimals_b, price_a_usd, price_b_usd)
    };

    let u_at_1 = usd_for_l(1);
    if u_at_1.is_finite() && u_at_1 > target_usd {
        return Err("target_usd too small for this range (increase budget or widen range)");
    }

    // Exponential upper bound: `(L * delta) >> 64` can be 0 for tiny `L`, so `hi` may need to grow.
    let mut hi: u128 = 1;
    let mut saw_positive_usd = false;
    for _ in 0..130 {
        let u = usd_for_l(hi);
        if !u.is_finite() {
            return Err("USD estimate overflow");
        }
        if u > 0.0 {
            saw_positive_usd = true;
        }
        if u >= target_usd {
            break;
        }
        let next = hi.checked_mul(2).ok_or("liquidity search overflow")?;
        if next == hi {
            return Err("liquidity search overflow");
        }
        hi = next;
    }
    if !saw_positive_usd {
        return Err("degenerate liquidity step (check ticks/sqrt_price)");
    }
    if usd_for_l(hi) < target_usd {
        return Err("target_usd too large to bracket at this range/prices (try smaller target)");
    }

    // Largest L with USD <= target_usd
    let mut lo: u128 = 0;
    let mut hi_bracket = hi;
    while lo + 1 < hi_bracket {
        let mid = lo + (hi_bracket - lo) / 2;
        let (a, b) = token_amounts_in_range(mid, sqrt_price, tick_lower, tick_upper);
        let u = raw_pair_usd(a, b, decimals_a, decimals_b, price_a_usd, price_b_usd);
        if u <= target_usd {
            lo = mid;
        } else {
            hi_bracket = mid;
        }
    }

    // Integer `(L * delta) >> 64` can be 0 on one side for small `L` while USD still fits the
    // budget (the other leg carries all notional). Whirlpool needs both legs > 0 for a normal
    // in-range deposit — raise `L` to the smallest value where both raw amounts are positive.
    let both_positive = |l: u128| -> bool {
        let (a, b) = token_amounts_in_range(l, sqrt_price, tick_lower, tick_upper);
        a > 0 && b > 0
    };

    let mut lo2 = lo;
    if !both_positive(lo) {
        let mut h = hi_bracket.max(2);
        let mut extended = 0u32;
        loop {
            let probe = h.saturating_sub(1).max(1);
            let (a, b) = token_amounts_in_range(probe, sqrt_price, tick_lower, tick_upper);
            if a > 0 && b > 0 {
                break;
            }
            extended += 1;
            if extended > 130 {
                return Err("quote: cannot achieve two-sided deposit (range or sqrt precision)");
            }
            let next = h.checked_mul(2).ok_or("liquidity search overflow")?;
            if next == h {
                return Err("liquidity search overflow");
            }
            h = next;
        }
        let hi_cap = h.saturating_sub(1).max(lo.saturating_add(1));
        let (a_hi, b_hi) = token_amounts_in_range(hi_cap, sqrt_price, tick_lower, tick_upper);
        if a_hi == 0 || b_hi == 0 {
            return Err("quote: cannot achieve two-sided deposit (range or sqrt precision)");
        }
        let mut left = lo;
        let mut right = hi_cap;
        while left + 1 < right {
            let mid = left + (right - left) / 2;
            if both_positive(mid) {
                right = mid;
            } else {
                left = mid;
            }
        }
        lo2 = right;
        if !both_positive(lo2) {
            return Err("quote: cannot achieve two-sided deposit (range or sqrt precision)");
        }
    }

    const MAX_USD_SLIP_FOR_TWO_SIDED: f64 = 1.03;
    let max_usd = target_usd * MAX_USD_SLIP_FOR_TWO_SIDED;

    let (a_floor, b_floor) = token_amounts_in_range(lo2, sqrt_price, tick_lower, tick_upper);
    let u_floor = raw_pair_usd(
        a_floor,
        b_floor,
        decimals_a,
        decimals_b,
        price_a_usd,
        price_b_usd,
    );
    if u_floor > max_usd {
        return Err(
            "target_usd too small for minimum two-sided deposit at this range (raise target ~5% or widen ticks)",
        );
    }

    // As much liquidity as possible under `target_usd`, but never below `lo2` (two-sided grid).
    let mut liquidity = lo2;
    if u_floor <= target_usd {
        let mut left = lo2;
        let mut right = hi_bracket;
        while left + 1 < right {
            let mid = left + (right - left) / 2;
            let (a, b) = token_amounts_in_range(mid, sqrt_price, tick_lower, tick_upper);
            if a == 0 || b == 0 {
                left = mid;
                continue;
            }
            let u = raw_pair_usd(a, b, decimals_a, decimals_b, price_a_usd, price_b_usd);
            if u <= target_usd {
                left = mid;
            } else {
                right = mid;
            }
        }
        liquidity = left;
    }

    let (a_lo, b_lo) = token_amounts_in_range(liquidity, sqrt_price, tick_lower, tick_upper);
    let estimated = raw_pair_usd(a_lo, b_lo, decimals_a, decimals_b, price_a_usd, price_b_usd);

    Ok(DepositBudgetQuote {
        amount_a: a_lo,
        amount_b: b_lo,
        token_max_a: bump_raw_cap(a_lo),
        token_max_b: bump_raw_cap(b_lo),
        estimated_value_usd: estimated,
        liquidity,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amount_a_nonzero_when_inv_floor_collapses() {
        // `floor(2^64/sqrt(Pc))` can equal `floor(2^64/sqrt(Pu))` for modest tick distance; the
        // U256 `L*(√Pu-√Pc)*2^64/(√Pc*√Pu)` path still attributes token A.
        let tl = 100i32;
        let tu = 500i32;
        let tc = 300i32;
        let sp = tick_to_sqrt_price(tc);
        let l = 1u128 << 50;
        let (a, b) = token_amounts_in_range(l, sp, tl, tu);
        assert!(a > 0 && b > 0, "a={a} b={b}");
    }

    #[test]
    fn quote_monotonic_and_near_target() {
        let tick_lower = -25_300;
        let tick_upper = -25_200;
        let tick_current = -25_250;
        let sqrt_price = tick_to_sqrt_price(tick_current);
        let target = 100.0_f64;
        let q = quote_deposit_budget_in_range(
            tick_lower,
            tick_upper,
            tick_current,
            sqrt_price,
            9,
            6,
            100.0,
            1.0,
            target,
        )
        .expect("quote");
        assert!(q.estimated_value_usd <= target);
        assert!(q.estimated_value_usd > target * 0.85);
        assert!(q.amount_a > 0 && q.amount_b > 0);
        assert!(q.token_max_a > 0 && q.token_max_b > 0);
    }

    /// Small USD targets used to round one raw leg to 0 under the old "max L under budget" rule.
    #[test]
    fn quote_small_target_both_legs_nonzero() {
        let tick_lower = -25_300;
        let tick_upper = -25_200;
        let tick_current = -25_250;
        let sqrt_price = tick_to_sqrt_price(tick_current);
        let q = quote_deposit_budget_in_range(
            tick_lower,
            tick_upper,
            tick_current,
            sqrt_price,
            9,
            6,
            150.0,
            1.0,
            3.0,
        )
        .expect("quote");
        assert!(q.amount_a > 0 && q.amount_b > 0);
        assert!(q.estimated_value_usd <= 3.0 * 1.03);
    }
}
