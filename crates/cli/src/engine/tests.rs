#[cfg(test)]
#[allow(clippy::module_inception)]
mod tests {
    use crate::backtest_engine::{run_single, GridRunParams, StratConfig};
    use crate::engine::liquidity;
    use crate::engine::pricing::{from_base_units, price_ab_human_to_raw, price_to_sqrt_q64};
    use crate::backtest_engine::StepDataPoint;
    use clmm_lp_domain::prelude::Price;
    use rust_decimal::Decimal;
    use rust_decimal::prelude::{FromPrimitive, ToPrimitive};
    use rust_decimal_macros::dec;

    fn step(price_ab: Decimal, quote_usd: Decimal) -> StepDataPoint {
        StepDataPoint {
            price_usd: Price::new(price_ab * quote_usd),
            price_ab: Price::new(price_ab),
            step_volume_usd: dec!(1000),
            quote_usd,
            lp_share: dec!(0.0001),
            pool_liquidity_active: None,
            start_timestamp: 0,
        }
    }

    #[test]
    fn hodl_value_matches_initial_lp_entry_tokens() {
        // Entry: A/B=20, B/USD=100 => A/USD=2000
        // Final: A/B=22, B/USD=90  => A/USD=1980
        let steps = vec![step(dec!(20), dec!(100)), step(dec!(22), dec!(90))];
        let capital = dec!(7000);
        let token_a_decimals: u32 = 9;
        let token_b_decimals: u32 = 9;

        let params = GridRunParams {
            capital_dec: capital,
            tx_cost_dec: dec!(0),
            rebalance_cost_model: None,
            fee_rate: dec!(0.0),
            pool_active_liquidity: None,
            token_a_decimals,
            token_b_decimals,
            step_seconds: 3600,
            use_liquidity_share: false,
        };

        // Static => no rebalances => `hodl_value` is solely derived from entry tokens.
        let width_pct = 0.20;
        let (_lo, _hi, _name, summary) = run_single(
            &steps,
            Price::new(dec!(2000)), // unused for bounds computation
            2000.0,                 // unused for bounds computation
            width_pct,
            StratConfig::Static,
            &params,
            None::<&[_]>,
            None,
        );

        let first = steps.first().unwrap();
        let last = steps.last().unwrap();

        let center_ab = first.price_ab.value.to_f64().unwrap_or(1.0);
        let half = width_pct / 2.0;
        let lower_ab = Decimal::from_f64(center_ab * (1.0 - half)).unwrap();
        let upper_ab = Decimal::from_f64(center_ab * (1.0 + half)).unwrap();
        let lower_usd = lower_ab * first.quote_usd;
        let upper_usd = upper_ab * first.quote_usd;

        let l_pos = liquidity::estimate_position_liquidity(
            &steps,
            lower_usd,
            upper_usd,
            capital,
            token_a_decimals,
            token_b_decimals,
        );

        let lower_ab_raw = price_ab_human_to_raw(lower_ab, token_a_decimals, token_b_decimals);
        let upper_ab_raw = price_ab_human_to_raw(upper_ab, token_a_decimals, token_b_decimals);
        let entry_ab_raw =
            price_ab_human_to_raw(first.price_ab.value, token_a_decimals, token_b_decimals);

        let sqrt_l = price_to_sqrt_q64(lower_ab_raw);
        let sqrt_u = price_to_sqrt_q64(upper_ab_raw);
        let sqrt_p = price_to_sqrt_q64(entry_ab_raw);

        let (hodl_a_base, hodl_b_base) =
            liquidity::amounts_from_liquidity_at_price(l_pos, sqrt_l, sqrt_p, sqrt_u);

        let hodl_a = from_base_units(hodl_a_base, token_a_decimals);
        let hodl_b = from_base_units(hodl_b_base, token_b_decimals);

        let expected_hodl_value = (hodl_a * last.price_usd.value)
            + (hodl_b * last.quote_usd.max(Decimal::ZERO));

        assert!((summary.hodl_value - expected_hodl_value).abs() < dec!(0.0001));
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
        let params = GridRunParams {
            capital_dec: capital,
            tx_cost_dec: tx_cost,
            rebalance_cost_model: None,
            fee_rate: dec!(0.0),
            pool_active_liquidity: None,
            token_a_decimals: 9,
            token_b_decimals: 9,
            step_seconds: 3600,
            use_liquidity_share: false,
        };

        // Periodic(1) => rebalance on every step.
        let (_lo, _hi, _name, summary) = run_single(
            &steps,
            Price::new(dec!(2000)), // entry A/USD (unused)
            2000.0,                 // center (USD) (only for fallback; bounds returned for reporting)
            0.20,                   // 20% width
            StratConfig::Periodic(1),
            &params,
            None::<&[_]>,
            None,
        );

        assert_eq!(summary.total_fees, Decimal::ZERO);
        assert_eq!(summary.rebalance_count, 5);
        assert_eq!(summary.total_rebalance_cost, tx_cost * Decimal::from(5u32));

        // Capital should be reduced exactly by total_rebalance_cost (not double-counted).
        assert!((summary.final_value - (capital - summary.total_rebalance_cost)).abs() < dec!(0.0001));
    }

    #[test]
    fn total_il_is_zero_with_multiple_rebalances_and_constant_price() {
        // Constant price + rebalanced ranges centered on current price => no IL movement,
        // even if we pay tx costs. Our IL metric excludes rebalance costs.
        let mut steps = Vec::new();
        for i in 0..6u64 {
            let mut s = step(dec!(20), dec!(100));
            s.step_volume_usd = Decimal::ZERO;
            s.start_timestamp = i;
            steps.push(s);
        }

        let capital = dec!(1000);
        let tx_cost = dec!(2);
        let params = GridRunParams {
            capital_dec: capital,
            tx_cost_dec: tx_cost,
            rebalance_cost_model: None,
            fee_rate: dec!(0.0),
            pool_active_liquidity: None,
            token_a_decimals: 9,
            token_b_decimals: 9,
            step_seconds: 3600,
            use_liquidity_share: false,
        };

        let (_lo, _hi, _name, summary) = run_single(
            &steps,
            Price::new(dec!(2000)), // unused for bounds computation
            2000.0,                 // center (USD) (only for fallback; bounds returned for reporting)
            0.20,                   // 20% width
            StratConfig::Periodic(1), // rebalance every step
            &params,
            None::<&[_]>,
            None,
        );

        assert!(summary.total_fees.is_zero());
        assert!(summary.rebalance_count >= 6);
        assert!(summary.final_il_pct.abs() < dec!(0.0001));
    }

    #[test]
    fn retouch_shift_moves_exit_edge_once() {
        // Entry A/B=25, width=20% => initial bounds:
        // lower=25*(1-0.1)=22.5, upper=25*(1+0.1)=27.5
        // Step1 price=28 => overflow=0.5 => new_upper=28, new_lower=22.5+0.5=23.0
        let steps = vec![step(dec!(25), dec!(100)), step(dec!(28), dec!(100))];
        let capital = dec!(1000);
        let params = GridRunParams {
            capital_dec: capital,
            tx_cost_dec: dec!(1),
            rebalance_cost_model: None,
            fee_rate: dec!(0.0),
            pool_active_liquidity: None,
            token_a_decimals: 9,
            token_b_decimals: 9,
            step_seconds: 3600,
            use_liquidity_share: false,
        };

        let width_pct = 0.20;

        // Sanity: rebalance should not be blocked by zero liquidity.
        let first = steps.first().unwrap();
        let center_ab = first.price_ab.value.to_f64().unwrap_or(1.0);
        let half = width_pct / 2.0;
        let lower_ab = Decimal::from_f64(center_ab * (1.0 - half)).unwrap();
        let upper_ab = Decimal::from_f64(center_ab * (1.0 + half)).unwrap();
        let lower_usd = lower_ab * first.quote_usd;
        let upper_usd = upper_ab * first.quote_usd;
        let l0 = liquidity::estimate_position_liquidity(
            &steps,
            lower_usd,
            upper_usd,
            capital,
            params.token_a_decimals,
            params.token_b_decimals,
        );
        assert!(l0 > 0, "expected non-zero liquidity_l for retouch test, got {l0}");

        let (_lo, _hi, _name, summary) = run_single(
            &steps,
            Price::new(dec!(1)), // unused for bounds computation
            1.0,
            width_pct,
            StratConfig::RetouchShift,
            &params,
            None::<&[_]>,
            None,
        );
        assert_eq!(summary.rebalance_count, 1);
        assert_eq!(summary.total_rebalance_cost, dec!(1));
    }

    #[test]
    fn retouch_shift_is_gated_until_back_in_range() {
        // Entry: 25, width=20% => initial [22.5, 27.5]
        // Step1: 28 => retouch to [23.0, 28.0] (once)
        // Step2: 29 => still out-of-range on upper side; no second retouch until price
        // re-enters range.
        let steps = vec![
            step(dec!(25), dec!(100)),
            step(dec!(28), dec!(100)),
            step(dec!(29), dec!(100)),
        ];
        let capital = dec!(1000);
        let params = GridRunParams {
            capital_dec: capital,
            tx_cost_dec: dec!(1),
            rebalance_cost_model: None,
            fee_rate: dec!(0.0),
            pool_active_liquidity: None,
            token_a_decimals: 9,
            token_b_decimals: 9,
            step_seconds: 3600,
            use_liquidity_share: false,
        };

        let width_pct = 0.20;

        let first = steps.first().unwrap();
        let center_ab = first.price_ab.value.to_f64().unwrap_or(1.0);
        let half = width_pct / 2.0;
        let lower_ab = Decimal::from_f64(center_ab * (1.0 - half)).unwrap();
        let upper_ab = Decimal::from_f64(center_ab * (1.0 + half)).unwrap();
        let lower_usd = lower_ab * first.quote_usd;
        let upper_usd = upper_ab * first.quote_usd;
        let l0 = liquidity::estimate_position_liquidity(
            &steps,
            lower_usd,
            upper_usd,
            capital,
            params.token_a_decimals,
            params.token_b_decimals,
        );
        assert!(l0 > 0, "expected non-zero liquidity_l for retouch test, got {l0}");

        let (_lo, _hi, _name, summary) = run_single(
            &steps,
            Price::new(dec!(1)), // unused for bounds computation
            1.0,
            width_pct,
            StratConfig::RetouchShift,
            &params,
            None::<&[_]>,
            None,
        );
        assert_eq!(summary.rebalance_count, 1);
        assert_eq!(summary.total_rebalance_cost, dec!(1));
    }

    #[test]
    fn il_metrics_are_consistent_across_static_and_threshold_paths() {
        // Same market path for both strategies.
        let mut steps = Vec::new();
        for (i, p) in [dec!(20), dec!(20.5), dec!(21.2), dec!(20.1), dec!(19.7), dec!(20.0)]
            .iter()
            .enumerate()
        {
            let mut s = step(*p, dec!(100));
            s.step_volume_usd = Decimal::ZERO;
            s.start_timestamp = i as u64;
            steps.push(s);
        }

        let params = GridRunParams {
            capital_dec: dec!(1000),
            tx_cost_dec: dec!(2),
            rebalance_cost_model: None,
            fee_rate: dec!(0.0),
            pool_active_liquidity: None,
            token_a_decimals: 9,
            token_b_decimals: 9,
            step_seconds: 3600,
            use_liquidity_share: false,
        };

        let (_lo_s, _hi_s, _name_s, static_summary) = run_single(
            &steps,
            Price::new(dec!(2000)),
            2000.0,
            0.20,
            StratConfig::Static,
            &params,
            None::<&[_]>,
            None,
        );
        let (_lo_t, _hi_t, _name_t, thr_summary) = run_single(
            &steps,
            Price::new(dec!(2000)),
            2000.0,
            0.20,
            StratConfig::Threshold(0.01),
            &params,
            None::<&[_]>,
            None,
        );

        // In amount-based engine we intentionally expose IL-like vs HODL (ex-fees),
        // and keep the legacy field as exact alias.
        assert_eq!(
            static_summary.final_il_pct,
            static_summary.final_il_vs_hodl_ex_fees_pct
        );
        assert_eq!(
            thr_summary.final_il_pct,
            thr_summary.final_il_vs_hodl_ex_fees_pct
        );
        assert!(static_summary.final_il_segment_pct.is_none());
        assert!(thr_summary.final_il_segment_pct.is_none());

        // Threshold path should rebalance more than static on this path.
        assert_eq!(static_summary.rebalance_count, 0);
        assert!(thr_summary.rebalance_count > static_summary.rebalance_count);
    }

    #[test]
    fn il_limit_rebalances_when_drag_exceeds_threshold() {
        let steps = vec![
            step(dec!(20), dec!(100)),
            step(dec!(26), dec!(100)),
            step(dec!(27), dec!(100)),
        ];
        let params = GridRunParams {
            capital_dec: dec!(1000),
            tx_cost_dec: dec!(1),
            rebalance_cost_model: None,
            fee_rate: dec!(0.0),
            pool_active_liquidity: None,
            token_a_decimals: 9,
            token_b_decimals: 9,
            step_seconds: 3600,
            use_liquidity_share: false,
        };
        let (_lo, _hi, _name, summary) = run_single(
            &steps,
            Price::new(dec!(2000)),
            2000.0,
            0.20,
            StratConfig::ILLimit {
                max_il: 0.01,
                close_il: None,
                grace_steps: 0,
            },
            &params,
            None::<&[_]>,
            None,
        );
        assert!(summary.rebalance_count >= 1);
    }

    #[test]
    fn il_limit_close_threshold_can_flatten_position() {
        let steps = vec![
            step(dec!(20), dec!(100)),
            step(dec!(34), dec!(100)),
            step(dec!(35), dec!(100)),
            step(dec!(36), dec!(100)),
        ];
        let params = GridRunParams {
            capital_dec: dec!(1000),
            tx_cost_dec: dec!(1),
            rebalance_cost_model: None,
            fee_rate: dec!(0.0),
            pool_active_liquidity: None,
            token_a_decimals: 9,
            token_b_decimals: 9,
            step_seconds: 3600,
            use_liquidity_share: false,
        };
        let (_lo, _hi, _name, summary) = run_single(
            &steps,
            Price::new(dec!(2000)),
            2000.0,
            0.20,
            StratConfig::ILLimit {
                max_il: 0.01,
                close_il: Some(0.02),
                grace_steps: 0,
            },
            &params,
            None::<&[_]>,
            None,
        );
        assert!(summary.rebalance_count >= 1);
        assert!(summary.total_rebalance_cost >= dec!(1));
    }
}

