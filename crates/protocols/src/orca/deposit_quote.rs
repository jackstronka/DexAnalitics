//! Target **USD notional** for an **in-range** Orca deposit: derive `token_max_a/b` from liquidity math.
//!
//! The UI used to split budget 50/50 in USD and send those as caps; the curve often only consumes
//! ~one leg, so on-chain value looked ~half of the entered budget. Here we pick liquidity `L` so
//! `amount_a(L) * p_a + amount_b(L) * p_b ≈ target_usd` (from below, integer `L`).

/// Q64.64 sqrt price from tick (same convention as [`super::position_reader`]).
fn tick_to_sqrt_price(tick: i32) -> u128 {
    let base: f64 = 1.0001;
    let sqrt_price = base.powi(tick).sqrt() * (1u128 << 64) as f64;
    sqrt_price as u128
}

/// Token amounts (raw) for liquidity `L` when spot is inside `[tick_lower, tick_upper)`.
fn token_amounts_in_range(
    liquidity: u128,
    sqrt_price: u128,
    tick_lower: i32,
    tick_upper: i32,
) -> (u64, u64) {
    let sqrt_price_upper = tick_to_sqrt_price(tick_upper);
    let inv_current = (1u128 << 64) / sqrt_price;
    let inv_upper = (1u128 << 64) / sqrt_price_upper;
    let delta_a = inv_current.saturating_sub(inv_upper);
    let amount_a = ((liquidity.saturating_mul(delta_a)) >> 64) as u64;

    let sqrt_price_lower = tick_to_sqrt_price(tick_lower);
    let delta_b = sqrt_price.saturating_sub(sqrt_price_lower);
    let amount_b = ((liquidity.saturating_mul(delta_b)) >> 64) as u64;

    (amount_a, amount_b)
}

fn raw_pair_usd(a: u64, b: u64, decimals_a: u8, decimals_b: u8, price_a_usd: f64, price_b_usd: f64) -> f64 {
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
#[must_use]
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
        return Err("current_tick must satisfy tick_lower <= current_tick < tick_upper (price in range)");
    }
    if !(target_usd.is_finite() && target_usd > 0.0) {
        return Err("target_usd must be a finite positive number");
    }
    if !(price_a_usd.is_finite() && price_b_usd.is_finite() && price_a_usd > 0.0 && price_b_usd > 0.0) {
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

    let liquidity = lo;
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
        assert!(q.token_max_a > 0 && q.token_max_b > 0);
    }
}
