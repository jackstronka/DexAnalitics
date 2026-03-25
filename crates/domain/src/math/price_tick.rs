//! Tick ↔ human price helpers (Uniswap v3-style **multiplicative** tick spacing).
//!
//! On-chain pools store **`sqrtPriceX96`** (fixed-point), not `1.0001^tick` directly. The **ratio**
//! between adjacent ticks is still **1.0001** per tick, matching [`TickMath`](https://docs.uniswap.org/contracts/v3/reference/core/libraries/TickMath) semantics.
//!
//! # Precision (`f64` vs chain)
//!
//! [`tick_to_price`] and [`price_to_tick`] use `f64` exponentiation/log for simplicity. For typical
//! CLMM ticks used in liquid pairs (roughly \(|\text{tick}| \ll 10^6\)), error vs a `Decimal`
//! reference is small — see unit tests. For **edge** ticks near protocol min/max or when matching
//! **exact** on-chain `sqrt_price` / tick math, prefer the protocol’s Q64.96 / integer path (e.g.
//! Orca reader) instead of this module.
use rust_decimal::Decimal;
use rust_decimal::prelude::*;

/// Returns the price corresponding to a given tick.
/// P = 1.0001 ^ tick
pub fn tick_to_price(tick: i32) -> Result<Decimal, &'static str> {
    let base = 1.0001f64;
    let price_f64 = base.powi(tick);
    Decimal::from_f64(price_f64).ok_or("Overflow converting price")
}

/// Returns the tick corresponding to a given price.
/// tick = log_1.0001(P)
pub fn price_to_tick(price: Decimal) -> Result<i32, &'static str> {
    if price <= Decimal::ZERO {
        return Err("Price must be positive");
    }
    let price_f64 = price.to_f64().ok_or("Overflow converting price")?;
    let base = 1.0001f64;
    let tick = price_f64.log(base);
    Ok(tick.round() as i32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::prelude::MathematicalOps;

    fn price_from_tick_decimal_reference(tick: i32) -> Decimal {
        let base = Decimal::from_str_exact("1.0001").expect("constant");
        base.powi(tick as i64)
    }

    #[test]
    fn tick_to_price_near_decimal_reference_for_typical_ticks() {
        for tick in [-50_000i32, -10_000, -1_000, 0, 1_000, 10_000, 50_000] {
            let p_f64 = tick_to_price(tick).unwrap();
            let p_ref = price_from_tick_decimal_reference(tick);
            let diff = (p_f64 - p_ref).abs();
            let scale = p_ref.abs().max(Decimal::ONE);
            let rel = diff / scale;
            assert!(
                rel < Decimal::new(1, 6),
                "tick={tick} p_f64={p_f64} p_ref={p_ref} rel={rel}"
            );
        }
    }

    #[test]
    fn test_tick_to_price() {
        // Tick 0 -> Price 1
        let p = tick_to_price(0).unwrap();
        assert_eq!(p, Decimal::from(1));

        // Tick 100 -> 1.0001^100 ~= 1.010049
        let p100 = tick_to_price(100).unwrap();
        // Allow small error due to f64
        let expected = 1.01004966;
        let diff = (p100.to_f64().unwrap() - expected).abs();
        assert!(diff < 0.000001);
    }

    #[test]
    fn test_price_to_tick() {
        let t = price_to_tick(Decimal::from(1)).unwrap();
        assert_eq!(t, 0);

        let t2 = price_to_tick(Decimal::from_f64(1.01004966).unwrap()).unwrap();
        assert_eq!(t2, 100);
    }
}
