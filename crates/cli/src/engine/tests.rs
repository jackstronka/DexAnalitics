#[cfg(test)]
mod tests {
    use crate::backtest_engine::{run_single, StratConfig};
    use crate::engine::{hodl, liquidity};
    use crate::backtest_engine::StepDataPoint;
    use clmm_lp_domain::prelude::Price;
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;

    fn step(price_ab: Decimal, quote_usd: Decimal) -> StepDataPoint {
        StepDataPoint {
            price_usd: Price::new(price_ab * quote_usd),
            price_ab: Price::new(price_ab),
            step_volume_usd: dec!(1000),
            quote_usd,
            lp_share: dec!(0.0001),
            start_timestamp: 0,
        }
    }

    #[test]
    fn hodl_value_is_final_value_of_fixed_amounts() {
        // Entry: A/B=20, B/USD=100 => A/USD=2000
        // Final: A/B=22, B/USD=90  => A/USD=1980
        let steps = vec![step(dec!(20), dec!(100)), step(dec!(22), dec!(90))];
        let capital = dec!(7000);

        let (amt_a, amt_b) = hodl::hodl_amounts_50_50_usd(&steps, capital);
        // Verify amounts are from entry 50/50 split
        assert!((amt_a - (capital / dec!(2) / dec!(2000))).abs() < dec!(0.0000001));
        assert!((amt_b - (capital / dec!(2) / dec!(100))).abs() < dec!(0.0000001));

        let hv = hodl::hodl_value_50_50_usd(&steps, capital);
        // Final value from fixed amounts
        let expected = amt_a * dec!(1980) + amt_b * dec!(90);
        assert!((hv - expected).abs() < dec!(0.0001));
    }

    #[test]
    fn liquidity_increases_when_range_is_narrower() {
        // Use a simple cross-pair scenario at entry.
        let steps = vec![step(dec!(20), dec!(100))];
        let capital = dec!(7000);

        // Narrower USD bounds correspond to narrower A/B bounds at entry.
        let l_wide = liquidity::estimate_position_liquidity(&steps, dec!(1500), dec!(2500), capital, 9, 9);
        let l_narrow = liquidity::estimate_position_liquidity(&steps, dec!(1800), dec!(2200), capital, 9, 9);
        assert!(l_narrow >= l_wide);
    }

    #[test]
    fn periodic_rebalance_costs_are_charged_once() {
        // Constant price, zero volume: only tx costs should reduce final value.
        let mut steps = Vec::new();
        for i in 0..5u64 {
            let mut s = step(dec!(20), dec!(100));
            s.step_volume_usd = Decimal::ZERO;
            s.start_timestamp = i;
            steps.push(s);
        }

        let capital = dec!(1000);
        let tx_cost = dec!(2);

        // Periodic(1) => rebalance on every step.
        let (_lo, _hi, _name, summary) = run_single(
            &steps,
            Price::new(dec!(2000)), // entry A/USD (unused)
            2000.0,                 // center (USD) (only for fallback; bounds returned for reporting)
            0.20,                   // 20% width
            StratConfig::Periodic(1),
            capital,
            tx_cost,
            dec!(0.0), // fee rate
            None,
            9,
            9,
        );

        assert_eq!(summary.total_fees, Decimal::ZERO);
        assert_eq!(summary.rebalance_count, 5);
        assert_eq!(summary.total_rebalance_cost, tx_cost * Decimal::from(5u32));

        // Capital should be reduced exactly by total_rebalance_cost (not double-counted).
        assert!((summary.final_value - (capital - summary.total_rebalance_cost)).abs() < dec!(0.0001));
    }
}

