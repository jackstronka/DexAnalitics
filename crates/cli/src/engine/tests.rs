#[cfg(test)]
#[allow(clippy::module_inception)]
mod tests {
    use crate::backtest_engine::StepDataPoint;
    use crate::backtest_engine::{
        GridRunParams, PeriodicTimeBasis, RetouchRepeatConfig, StratConfig, run_single,
    };
    use crate::engine::liquidity;
    use crate::engine::pricing::{from_base_units, price_ab_human_to_raw, price_to_sqrt_q64};
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
            periodic_time_basis: PeriodicTimeBasis::WallClockSeconds,
            retouch_repeat: None,
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

        let expected_hodl_value =
            (hodl_a * last.price_usd.value) + (hodl_b * last.quote_usd.max(Decimal::ZERO));

        assert!((summary.hodl_value - expected_hodl_value).abs() < dec!(0.0001));
    }

    #[test]
    fn liquidity_increases_when_range_is_narrower() {
        // Use a simple cross-pair scenario at entry.
        let steps = vec![step(dec!(20), dec!(100))];
        let capital = dec!(7000);

        // Narrower USD bounds correspond to narrower A/B bounds at entry.
        let l_wide =
            liquidity::estimate_position_liquidity(&steps, dec!(1500), dec!(2500), capital, 9, 9);
        let l_narrow =
            liquidity::estimate_position_liquidity(&steps, dec!(1800), dec!(2200), capital, 9, 9);
        assert!(l_narrow >= l_wide);
    }

    #[test]
    fn periodic_rebalance_costs_are_charged_once() {
        // Constant price, zero volume: only tx costs should reduce final value.
        let mut steps = Vec::new();
        for i in 0..5u64 {
            let mut s = step(dec!(20), dec!(100));
            s.step_volume_usd = Decimal::ZERO;
            // One wall-clock hour between steps so `Periodic(1)` matches real 1h intervals.
            s.start_timestamp = i * 3600;
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
            periodic_time_basis: PeriodicTimeBasis::WallClockSeconds,
            retouch_repeat: None,
            use_liquidity_share: false,
        };

        // Periodic(1h): first step is entry; then one rebalance per subsequent hour (4 total).
        let (_lo, _hi, _name, summary) = run_single(
            &steps,
            Price::new(dec!(2000)), // entry A/USD (unused)
            2000.0, // center (USD) (only for fallback; bounds returned for reporting)
            0.20,   // 20% width
            StratConfig::Periodic(1),
            &params,
            None::<&[_]>,
            None,
        );

        assert_eq!(summary.total_fees, Decimal::ZERO);
        assert_eq!(summary.rebalance_count, 4);
        assert_eq!(summary.total_rebalance_cost, tx_cost * Decimal::from(4u32));

        // Capital should be reduced exactly by total_rebalance_cost (not double-counted).
        assert!(
            (summary.final_value - (capital - summary.total_rebalance_cost)).abs() < dec!(0.0001)
        );
    }

    #[test]
    fn total_il_is_zero_with_multiple_rebalances_and_constant_price() {
        // Constant price + rebalanced ranges centered on current price => no IL movement,
        // even if we pay tx costs. Our IL metric excludes rebalance costs.
        let mut steps = Vec::new();
        for i in 0..6u64 {
            let mut s = step(dec!(20), dec!(100));
            s.step_volume_usd = Decimal::ZERO;
            s.start_timestamp = i * 3600;
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
            periodic_time_basis: PeriodicTimeBasis::WallClockSeconds,
            retouch_repeat: None,
            use_liquidity_share: false,
        };

        let (_lo, _hi, _name, summary) = run_single(
            &steps,
            Price::new(dec!(2000)),   // unused for bounds computation
            2000.0, // center (USD) (only for fallback; bounds returned for reporting)
            0.20,   // 20% width
            StratConfig::Periodic(1), // rebalance each wall-clock hour after entry
            &params,
            None::<&[_]>,
            None,
        );

        assert!(summary.total_fees.is_zero());
        // 6 hourly steps => rebalance at hours 1..5 after entry (5 rebalances).
        assert!(summary.rebalance_count >= 5);
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
            periodic_time_basis: PeriodicTimeBasis::WallClockSeconds,
            retouch_repeat: None,
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
        assert!(
            l0 > 0,
            "expected non-zero liquidity_l for retouch test, got {l0}"
        );

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
        // Regression: HODL benchmark must not collapse to zero after retouch (bad L estimate).
        assert!(
            summary.hodl_value > dec!(100),
            "hodl_value={} (expected >> 0)",
            summary.hodl_value
        );
    }

    /// If `quote_usd` changes between steps, post-retouch `lower_usd`/`upper_usd` use step-1 quote
    /// but liquidity estimation must not still divide by step-0 quote (that produced L=0 and
    /// zeroed HODL in optimize tables).
    #[test]
    fn retouch_shift_hodl_nonzero_when_step_quote_usd_differs() {
        let mut s0 = step(dec!(25), dec!(100));
        s0.start_timestamp = 0;
        let mut s1 = step(dec!(28), dec!(95));
        s1.start_timestamp = 3600;
        let steps = vec![s0, s1];
        let capital = dec!(7000);
        let params = GridRunParams {
            capital_dec: capital,
            tx_cost_dec: dec!(0),
            rebalance_cost_model: None,
            fee_rate: dec!(0.0),
            pool_active_liquidity: None,
            token_a_decimals: 9,
            token_b_decimals: 9,
            step_seconds: 3600,
            periodic_time_basis: PeriodicTimeBasis::WallClockSeconds,
            retouch_repeat: None,
            use_liquidity_share: false,
        };
        let width_pct = 0.20;
        let (_lo, _hi, _name, summary) = run_single(
            &steps,
            Price::new(dec!(2500)),
            2500.0,
            width_pct,
            StratConfig::RetouchShift,
            &params,
            None::<&[_]>,
            None,
        );
        assert_eq!(summary.rebalance_count, 1);
        assert!(
            summary.hodl_value > dec!(1000),
            "hodl_value={} (expected non-degenerate HODL)",
            summary.hodl_value
        );
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
            periodic_time_basis: PeriodicTimeBasis::WallClockSeconds,
            retouch_repeat: None,
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
        assert!(
            l0 > 0,
            "expected non-zero liquidity_l for retouch test, got {l0}"
        );

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
    fn retouch_shift_hybrid_repeat_fires_on_time_while_still_out_of_range() {
        // Start in range, exit up, retouch; price keeps climbing so we stay OOR; after rearm_secs
        // wall time, a second retouch is allowed.
        let mut a = step(dec!(25), dec!(100));
        a.start_timestamp = 0;
        a.step_volume_usd = Decimal::ZERO;
        let mut b = step(dec!(32), dec!(100));
        b.start_timestamp = 1;
        b.step_volume_usd = Decimal::ZERO;
        let mut c = step(dec!(33), dec!(100));
        c.start_timestamp = 10;
        c.step_volume_usd = Decimal::ZERO;
        let mut d = step(dec!(33), dec!(100));
        d.start_timestamp = 70;
        d.step_volume_usd = Decimal::ZERO;
        let steps = vec![a, b, c, d];

        let params = GridRunParams {
            capital_dec: dec!(1000),
            tx_cost_dec: dec!(1),
            rebalance_cost_model: None,
            fee_rate: dec!(0.0),
            pool_active_liquidity: None,
            token_a_decimals: 9,
            token_b_decimals: 9,
            step_seconds: 3600,
            periodic_time_basis: PeriodicTimeBasis::WallClockSeconds,
            retouch_repeat: Some(RetouchRepeatConfig {
                cooldown_secs: 5,
                rearm_after_secs: 50,
                extra_move_pct: 0.05,
            }),
            use_liquidity_share: false,
        };

        let (_lo, _hi, _name, summary) = run_single(
            &steps,
            Price::new(dec!(1)),
            1.0,
            0.20,
            StratConfig::RetouchShift,
            &params,
            None::<&[_]>,
            None,
        );
        assert!(
            summary.rebalance_count >= 2,
            "expected >=2 retouches (initial + time-based repeat), got {}",
            summary.rebalance_count
        );
    }

    #[test]
    fn retouch_shift_hybrid_repeat_fires_on_extra_move_pct() {
        let mut a = step(dec!(25), dec!(100));
        a.start_timestamp = 0;
        a.step_volume_usd = Decimal::ZERO;
        let mut b = step(dec!(40), dec!(100));
        b.start_timestamp = 1;
        b.step_volume_usd = Decimal::ZERO;
        let mut c = step(dec!(40.5), dec!(100));
        c.start_timestamp = 10;
        c.step_volume_usd = Decimal::ZERO;
        let mut d = step(dec!(41), dec!(100));
        d.start_timestamp = 20;
        d.step_volume_usd = Decimal::ZERO;
        let steps = vec![a, b, c, d];

        let params = GridRunParams {
            capital_dec: dec!(1000),
            tx_cost_dec: dec!(1),
            rebalance_cost_model: None,
            fee_rate: dec!(0.0),
            pool_active_liquidity: None,
            token_a_decimals: 9,
            token_b_decimals: 9,
            step_seconds: 3600,
            periodic_time_basis: PeriodicTimeBasis::WallClockSeconds,
            retouch_repeat: Some(RetouchRepeatConfig {
                cooldown_secs: 5,
                rearm_after_secs: 86_400,
                extra_move_pct: 0.02,
            }),
            use_liquidity_share: false,
        };

        let (_lo, _hi, _name, summary) = run_single(
            &steps,
            Price::new(dec!(1)),
            1.0,
            0.20,
            StratConfig::RetouchShift,
            &params,
            None::<&[_]>,
            None,
        );
        assert!(
            summary.rebalance_count >= 2,
            "expected >=2 retouches (pct path before rearm time), got {}",
            summary.rebalance_count
        );
    }

    #[test]
    fn il_metrics_are_consistent_across_static_and_threshold_paths() {
        // Same market path for both strategies.
        let mut steps = Vec::new();
        for (i, p) in [
            dec!(20),
            dec!(20.5),
            dec!(21.2),
            dec!(20.1),
            dec!(19.7),
            dec!(20.0),
        ]
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
            periodic_time_basis: PeriodicTimeBasis::WallClockSeconds,
            retouch_repeat: None,
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

    /// [`StratConfig::OorRecenter`] never recenters while still in-range; [`StratConfig::Threshold`]
    /// can rebalance on mid-deviation even without OOR.
    #[test]
    fn oor_recenter_skips_in_range_mid_rebalance_that_threshold_fires() {
        let mut s0 = step(dec!(25), dec!(100));
        s0.start_timestamp = 0;
        s0.step_volume_usd = Decimal::ZERO;
        let mut s1 = step(dec!(26.5), dec!(100));
        s1.start_timestamp = 3600;
        s1.step_volume_usd = Decimal::ZERO;
        let steps = vec![s0, s1];
        let params = GridRunParams {
            capital_dec: dec!(1000),
            tx_cost_dec: dec!(0),
            rebalance_cost_model: None,
            fee_rate: dec!(0.0),
            pool_active_liquidity: None,
            token_a_decimals: 9,
            token_b_decimals: 9,
            step_seconds: 3600,
            periodic_time_basis: PeriodicTimeBasis::WallClockSeconds,
            retouch_repeat: None,
            use_liquidity_share: false,
        };
        let width_pct = 0.20;
        let (_lo_o, _hi_o, _name_o, oor_summary) = run_single(
            &steps,
            Price::new(dec!(2500)),
            2500.0,
            width_pct,
            StratConfig::OorRecenter,
            &params,
            None::<&[_]>,
            None,
        );
        let (_lo_t, _hi_t, _name_t, th_summary) = run_single(
            &steps,
            Price::new(dec!(2500)),
            2500.0,
            width_pct,
            StratConfig::Threshold(0.02),
            &params,
            None::<&[_]>,
            None,
        );
        assert_eq!(oor_summary.rebalance_count, 0);
        assert_eq!(th_summary.rebalance_count, 1);
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
            periodic_time_basis: PeriodicTimeBasis::WallClockSeconds,
            retouch_repeat: None,
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
            periodic_time_basis: PeriodicTimeBasis::WallClockSeconds,
            retouch_repeat: None,
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
