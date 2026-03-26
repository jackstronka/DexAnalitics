use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};

fn pow10_u64(exp: u32) -> u64 {
    10u64.saturating_pow(exp.min(18))
}

/// Convert a human A/B price into a "raw units" price used by CLMM math.
///
/// If `price_ab_human` is tokenB per tokenA in UI units, then:
/// `price_ab_raw = price_ab_human * 10^(dec_b - dec_a)`.
pub fn price_ab_human_to_raw(
    price_ab_human: Decimal,
    token_a_decimals: u32,
    token_b_decimals: u32,
) -> Decimal {
    let da = token_a_decimals.min(18) as i32;
    let db = token_b_decimals.min(18) as i32;
    let exp = db - da;
    if exp == 0 {
        return price_ab_human;
    }
    let scale = Decimal::from(pow10_u64(exp.unsigned_abs() as u32));
    if exp > 0 {
        price_ab_human * scale
    } else {
        price_ab_human / scale
    }
}

/// Converts a Decimal price into Q64.64 (u128).
pub fn price_to_q64(price: Decimal) -> u128 {
    let f = price.to_f64().unwrap_or(0.0);
    if f <= 0.0 {
        return 0;
    }
    (f * (1u128 << 64) as f64) as u128
}

/// Converts a Decimal price into sqrt(Q64.64) (u128).
pub fn price_to_sqrt_q64(price: Decimal) -> u128 {
    let f = price.to_f64().unwrap_or(0.0);
    if f <= 0.0 {
        return 0;
    }
    (f.sqrt() * (1u128 << 64) as f64) as u128
}

pub fn to_base_units(amount: Decimal, decimals: u32) -> u64 {
    if amount <= Decimal::ZERO {
        return 0;
    }
    let scale = Decimal::from(10u64.pow(decimals.min(18)));
    (amount * scale).round().to_u64().unwrap_or(0)
}

pub fn from_base_units(amount: u64, decimals: u32) -> Decimal {
    if decimals == 0 {
        return Decimal::from(amount);
    }
    let scale = Decimal::from(10u64.pow(decimals.min(18)));
    Decimal::from(amount) / scale
}

/// Safe clamp for quote USD used in divisions.
pub fn clamp_quote_usd(q: Decimal) -> Decimal {
    q.max(Decimal::from_f64(1e-9).unwrap())
}
