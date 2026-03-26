use crate::backtest_engine::StepData;
use rust_decimal::Decimal;

/// HODL benchmark definition for cross-pairs:
/// - start with 50/50 USD split between token A and token B at entry
/// - hold fixed amounts until the end
/// - value both legs in USD at the end using (A/USD) and (B/USD)
pub fn hodl_amounts_50_50_usd(step_data: &[StepData], capital_usd: Decimal) -> (Decimal, Decimal) {
    let Some(first) = step_data.first() else {
        return (Decimal::ZERO, Decimal::ZERO);
    };
    let half = capital_usd / Decimal::from(2);
    let a_entry_usd = first.price_usd.value;
    let b_entry_usd = first.quote_usd;
    if a_entry_usd <= Decimal::ZERO || b_entry_usd <= Decimal::ZERO {
        return (Decimal::ZERO, Decimal::ZERO);
    }
    (half / a_entry_usd, half / b_entry_usd)
}

pub fn hodl_value_50_50_usd(step_data: &[StepData], capital_usd: Decimal) -> Decimal {
    let (Some(_first), Some(last)) = (step_data.first(), step_data.last()) else {
        return capital_usd;
    };
    let (amt_a, amt_b) = hodl_amounts_50_50_usd(step_data, capital_usd);
    (amt_a * last.price_usd.value) + (amt_b * last.quote_usd.max(Decimal::ZERO))
}
