//! Unit tests for amount-based backtest engine (`run_single`).
//!
//! Kept in sync with `crate::backtest_engine::run_single` (flat args, no legacy `GridRunParams`).
//!
//! **Birdeye vs snapshots (CLI):** default price path uses Birdeye OHLCV → `step_volume_usd` and fees
//! `volume * fee_rate` (when swaps/snapshot fee map absent). `--price-path-source snapshots` sets
//! `step_volume_usd = 0` and supplies pool fees via `snapshot_pool_fees_usd`. When per-step **pool**
//! fees USD are the same, both modes must match — see `birdeye_volume_fees_match_equivalent_snapshot_fee_index`.

#[cfg(test)]
#[allow(clippy::module_inception)]
mod tests {
    use crate::backtest_engine::StepDataPoint;
    use crate::backtest_engine::{StratConfig, run_single};
    use crate::engine::hodl;
    use crate::engine::liquidity;
    use clmm_lp_domain::prelude::Price;
    use rust_decimal::Decimal;
    use rust_decimal::prelude::ToPrimitive;
    use rust_decimal_macros::dec;
    use std::collections::BTreeMap;

    fn step(price_ab: Decimal, quote_usd: Decimal) -> StepDataPoint {
        StepDataPoint {
            price_usd: Price::new(price_ab * quote_usd),
            price_ab: Price::new(price_ab),
            step_volume_usd: dec!(1000),
            quote_usd,
            lp_share: dec!(0.0001),
            liquidity_active_raw: None,
            tick_current: None,
            start_timestamp: 0,
        }
    }

    #[test]
    fn hodl_value_matches_engine_hodl_benchmark() {
        let steps = vec![step(dec!(20), dec!(100)), step(dec!(22), dec!(90))];
        let capital = dec!(7000);
        let token_a_decimals: u32 = 9;
        let token_b_decimals: u32 = 9;
        let width_pct = 0.20;

        let (_lo, _hi, _name, summary) = run_single(
            &steps,
            Price::new(dec!(2000)),
            2000.0,
            width_pct,
            StratConfig::Static,
            capital,
            dec!(0),
            dec!(0.0),
            None,
            token_a_decimals,
            token_b_decimals,
            None,
            None,
        );

        let expected_hodl_value = hodl::hodl_value_50_50_usd(&steps, capital);
        assert!((summary.hodl_value - expected_hodl_value).abs() < dec!(0.0001));
    }

    #[test]
    fn liquidity_increases_when_range_is_narrower() {
        let steps = vec![step(dec!(20), dec!(100))];
        let capital = dec!(7000);

        let l_wide =
            liquidity::estimate_position_liquidity(&steps, dec!(1500), dec!(2500), capital, 9, 9);
        let l_narrow =
            liquidity::estimate_position_liquidity(&steps, dec!(1800), dec!(2200), capital, 9, 9);
        assert!(l_narrow >= l_wide);
    }

    #[test]
    fn periodic_hourly_rebalance_costs_are_charged_once() {
        let mut steps = Vec::new();
        for i in 0..5u64 {
            let mut s = step(dec!(20), dec!(100));
            s.step_volume_usd = Decimal::ZERO;
            s.start_timestamp = i * 3600;
            steps.push(s);
        }

        let capital = dec!(1000);
        let tx_cost = dec!(2);

        let (_lo, _hi, _name, summary) = run_single(
            &steps,
            Price::new(dec!(2000)),
            2000.0,
            0.20,
            StratConfig::Periodic(1),
            capital,
            tx_cost,
            dec!(0.0),
            None,
            9,
            9,
            None,
            None,
        );

        assert_eq!(summary.total_fees, Decimal::ZERO);
        // First step has no elapsed wall-clock yet; then each 1h step fires.
        assert_eq!(summary.rebalance_count, 4);
        assert_eq!(summary.total_rebalance_cost, tx_cost * Decimal::from(4u32));

        assert!(
            (summary.final_value - (capital - summary.total_rebalance_cost)).abs() < dec!(0.0001)
        );
    }

    #[test]
    fn total_il_is_zero_with_multiple_periodic_rebalances_and_constant_price() {
        let mut steps = Vec::new();
        for i in 0..6u64 {
            let mut s = step(dec!(20), dec!(100));
            s.step_volume_usd = Decimal::ZERO;
            s.start_timestamp = i * 3600;
            steps.push(s);
        }

        let capital = dec!(1000);
        let tx_cost = dec!(2);

        let (_lo, _hi, _name, summary) = run_single(
            &steps,
            Price::new(dec!(2000)),
            2000.0,
            0.20,
            StratConfig::Periodic(1),
            capital,
            tx_cost,
            dec!(0.0),
            None,
            9,
            9,
            None,
            None,
        );

        assert!(summary.total_fees.is_zero());
        assert!(summary.rebalance_count >= 5);
        assert!(summary.final_il_pct.abs() < dec!(0.0001));
    }

    #[test]
    fn periodic_hourly_respects_wall_clock_on_irregular_steps() {
        let mut steps = Vec::new();
        for (i, ts) in [0_u64, 60, 120, 180, 240, 3600, 7200]
            .iter()
            .enumerate()
        {
            let mut s = step(dec!(20), dec!(100));
            s.step_volume_usd = Decimal::ZERO;
            s.start_timestamp = *ts;
            s.tick_current = Some(i as i32);
            steps.push(s);
        }

        let (_lo, _hi, _name, summary) = run_single(
            &steps,
            Price::new(dec!(2000)),
            2000.0,
            0.20,
            StratConfig::Periodic(1),
            dec!(1000),
            dec!(0),
            dec!(0.0),
            None,
            9,
            9,
            None,
            None,
        );
        // Only at ts=3600 and ts=7200.
        assert_eq!(summary.rebalance_count, 2);
    }

    #[test]
    fn periodic_steps_legacy_still_rebalances_each_step() {
        let mut steps = Vec::new();
        for i in 0..5u64 {
            let mut s = step(dec!(20), dec!(100));
            s.step_volume_usd = Decimal::ZERO;
            s.start_timestamp = i * 3600;
            steps.push(s);
        }

        let (_lo, _hi, _name, summary) = run_single(
            &steps,
            Price::new(dec!(2000)),
            2000.0,
            0.20,
            StratConfig::PeriodicSteps(1),
            dec!(1000),
            dec!(0),
            dec!(0.0),
            None,
            9,
            9,
            None,
            None,
        );
        assert_eq!(summary.rebalance_count, 5);
    }

    #[test]
    fn threshold_rebalances_more_than_static_on_swing_path() {
        let mut steps = Vec::new();
        for (i, p) in [
            dec!(20.0),
            dec!(20.5),
            dec!(19.0),
            dec!(18.5),
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

        let capital = dec!(1000);
        let tx_cost = dec!(2);

        let (_lo_s, _hi_s, _name_s, static_summary) = run_single(
            &steps,
            Price::new(dec!(2000)),
            2000.0,
            0.20,
            StratConfig::Static,
            capital,
            tx_cost,
            dec!(0.0),
            None,
            9,
            9,
            None,
            None,
        );
        let (_lo_t, _hi_t, _name_t, thr_summary) = run_single(
            &steps,
            Price::new(dec!(2000)),
            2000.0,
            0.20,
            StratConfig::Threshold {
                threshold_pct: 0.01,
                min_rebalance_interval_hours: 0,
                rebalance_on_range_exit_immediately: true,
            },
            capital,
            tx_cost,
            dec!(0.0),
            None,
            9,
            9,
            None,
            None,
        );

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

        assert_eq!(static_summary.rebalance_count, 0);
        assert!(thr_summary.rebalance_count > static_summary.rebalance_count);
    }

    #[test]
    fn birdeye_volume_fees_match_equivalent_snapshot_fee_index() {
        // Contract: candle path (step_volume * fee_rate) vs snapshot index (explicit pool USD/step).
        let n = 5u64;
        let fee_rate = dec!(0.0004);
        let vol_per_step = dec!(1_000_000);
        let lp = dec!(0.1);
        let p_ab = dec!(100);
        let q_usd = dec!(1);

        let mut steps_birdeye = Vec::new();
        let mut steps_snap = Vec::new();
        for i in 0..n {
            let b = StepDataPoint {
                price_usd: Price::new(p_ab * q_usd),
                price_ab: Price::new(p_ab),
                step_volume_usd: vol_per_step,
                quote_usd: q_usd,
                lp_share: lp,
                liquidity_active_raw: None,
                tick_current: None,
                start_timestamp: i * 3600,
            };
            let mut s = b;
            s.step_volume_usd = Decimal::ZERO;
            steps_birdeye.push(b);
            steps_snap.push(s);
        }

        let pool_fee_per_step = vol_per_step * fee_rate;
        let mut snap: BTreeMap<usize, Decimal> = BTreeMap::new();
        for i in 0..n as usize {
            snap.insert(i, pool_fee_per_step);
        }

        let capital = dec!(10_000);
        let args = (
            Price::new(p_ab * q_usd),
            100.0_f64,
            0.25_f64,
            StratConfig::Static,
            capital,
            Decimal::ZERO,
            fee_rate,
            None,
            9_u32,
            9_u32,
            None,
        );

        let (_l1, _h1, _n1, sum_b) = run_single(
            &steps_birdeye,
            args.0,
            args.1,
            args.2,
            args.3,
            args.4,
            args.5,
            args.6,
            args.7,
            args.8,
            args.9,
            args.10,
            None,
        );
        let (_l2, _h2, _n2, sum_s) = run_single(
            &steps_snap,
            args.0,
            args.1,
            args.2,
            args.3,
            args.4,
            args.5,
            args.6,
            args.7,
            args.8,
            args.9,
            args.10,
            Some(&snap),
        );

        assert!(
            (sum_b.total_fees - sum_s.total_fees).abs() < dec!(0.000_000_1),
            "total_fees birdeye {} vs snapshot {}",
            sum_b.total_fees,
            sum_s.total_fees
        );
        assert!(
            (sum_b.final_value - sum_s.final_value).abs() < dec!(0.000_000_1),
            "final_value birdeye {} vs snapshot {}",
            sum_b.final_value,
            sum_s.final_value
        );
    }

    #[test]
    fn run_single_sol_usdc_decimals_position_value_sane_at_flat_price() {
        // Regression: human price_ab with dec_a=9, dec_b=6 must use raw encoding in sqrt math
        // (same as estimate_position_liquidity); otherwise final_value blows up vs capital.
        let p_ab = dec!(130.0);
        let q_usd = dec!(1.0);
        let steps = vec![
            StepDataPoint {
                price_usd: Price::new(p_ab * q_usd),
                price_ab: Price::new(p_ab),
                step_volume_usd: Decimal::ZERO,
                quote_usd: q_usd,
                lp_share: dec!(0.01),
                liquidity_active_raw: None,
                tick_current: None,
                start_timestamp: 0,
            };
            20
        ];
        let capital = dec!(7000);
        let (_lo, _hi, _name, summary) = run_single(
            &steps,
            Price::new(p_ab * q_usd),
            130.0,
            0.20,
            StratConfig::Static,
            capital,
            dec!(0),
            dec!(0.000348),
            None,
            9,
            6,
            None,
            None,
        );
        let diff = (summary.final_value - capital).abs();
        assert!(
            diff < dec!(500),
            "final_value {} too far from capital {} (diff {})",
            summary.final_value,
            capital,
            diff
        );
    }

    #[test]
    fn snapshot_pool_fee_index_accrues_lp_share_when_in_range() {
        let mut steps = Vec::new();
        for i in 0..3u64 {
            let mut s = step(dec!(100), dec!(1));
            s.step_volume_usd = Decimal::ZERO;
            s.lp_share = dec!(0.1);
            s.start_timestamp = i * 60;
            steps.push(s);
        }
        let mut snap = BTreeMap::new();
        snap.insert(1, dec!(100));
        snap.insert(2, dec!(50));
        let (_lo, _hi, _name, summary) = run_single(
            &steps,
            Price::new(dec!(100)),
            100.0,
            0.50,
            StratConfig::Static,
            dec!(1000),
            dec!(0),
            dec!(0.0004),
            None,
            9,
            9,
            None,
            Some(&snap),
        );
        assert!((summary.total_fees - dec!(15)).abs() < dec!(0.0001));
    }

    #[test]
    fn snapshot_pool_fee_dynamic_liquidity_active_scales_fees() {
        // When snapshot-fees are used and per-step `liquidity_active_raw` is present,
        // we attribute fees via:
        //   step_fees = pool_fees_usd * (position_liquidity / liquidity_active_at_step)
        // So changing liquidity_active in a step should scale the earned fees inversely.

        let price_ab = dec!(100);
        let quote_usd = dec!(1);
        let token_a_decimals: u32 = 9;
        let token_b_decimals: u32 = 6;

        let make_steps = |liq1: u128, liq2: u128| -> Vec<StepDataPoint> {
            vec![
                StepDataPoint {
                    price_usd: Price::new(price_ab * quote_usd),
                    price_ab: Price::new(price_ab),
                    step_volume_usd: Decimal::ZERO,
                    quote_usd,
                    lp_share: dec!(0.01), // not used in dynamic snapshot share
                    liquidity_active_raw: Some(liq1),
                    tick_current: None,
                    start_timestamp: 0,
                },
                StepDataPoint {
                    price_usd: Price::new(price_ab * quote_usd),
                    price_ab: Price::new(price_ab),
                    step_volume_usd: Decimal::ZERO,
                    quote_usd,
                    lp_share: dec!(0.01),
                    liquidity_active_raw: Some(liq2),
                    tick_current: None,
                    start_timestamp: 3600,
                },
            ]
        };

        let mut snap = BTreeMap::new();
        snap.insert(0usize, dec!(100));
        snap.insert(1usize, dec!(100));

        let capital = dec!(10_000);
        let width_pct = 0.20; // bounds around entry price: [90,110]

        // Pool active L must be **much larger** than estimated position L so fee share stays in
        // (0, 1); otherwise `pos/pool` hits the safety clamp and the ratio below collapses to 1.
        let pool_l: u128 = 1_000_000_000_000_000;
        let (_l1, _u1, _n1, s1) = run_single(
            &make_steps(pool_l, pool_l),
            Price::new(price_ab * quote_usd),
            price_ab.to_f64().unwrap_or(100.0),
            width_pct,
            StratConfig::Static,
            capital,
            dec!(0),
            dec!(0.0),
            None,
            token_a_decimals,
            token_b_decimals,
            None,
            Some(&snap),
        );

        let (_l2, _u2, _n2, s2) = run_single(
            &make_steps(pool_l, pool_l * 2),
            Price::new(price_ab * quote_usd),
            price_ab.to_f64().unwrap_or(100.0),
            width_pct,
            StratConfig::Static,
            capital,
            dec!(0),
            dec!(0.0),
            None,
            token_a_decimals,
            token_b_decimals,
            None,
            Some(&snap),
        );

        assert!(s1.total_fees > Decimal::ZERO);
        let ratio = s2.total_fees / s1.total_fees;
        // Expected ratio (same L_pos; only per-step pool L differs):
        //   case1 ∝ 100/L + 100/L = 200/L
        //   case2 ∝ 100/L + 100/(2L) = 150/L  →  ratio = 150/200 = 0.75
        assert!(
            (ratio - dec!(0.75)).abs() < dec!(0.000_001),
            "ratio={} expected=0.75 (fees1={} fees2={})",
            ratio,
            s1.total_fees,
            s2.total_fees
        );
    }

    #[test]
    fn parse_strategy_label_bollinger_and_last_candle() {
        use crate::backtest_engine::parse_strategy_label;
        assert_eq!(
            parse_strategy_label("oor_recenter"),
            Some(StratConfig::OorRecenter)
        );
        assert_eq!(
            parse_strategy_label("retouch_shift"),
            Some(StratConfig::RetouchShift {
                retouch_offset_pct: 0.0,
            })
        );
        assert_eq!(
            parse_strategy_label("retouch_shift_off0.1000pct"),
            Some(StratConfig::RetouchShift {
                retouch_offset_pct: 0.001,
            })
        );
        assert_eq!(
            parse_strategy_label("il_limit_5%_grace_0"),
            Some(StratConfig::IlLimit {
                max_il_pct: 0.05,
                close_il_pct: None,
                grace_steps: 0,
            })
        );
        assert_eq!(
            parse_strategy_label("il_limit_5%_close_12%_grace_3"),
            Some(StratConfig::IlLimit {
                max_il_pct: 0.05,
                close_il_pct: Some(0.12),
                grace_steps: 3,
            })
        );
        assert_eq!(
            parse_strategy_label("bollinger_w20_k2_r12"),
            Some(StratConfig::Bollinger {
                window: 20,
                k: 2.0,
                rebalance_steps: 12,
            })
        );
        assert_eq!(
            parse_strategy_label("bollinger_w20_k1.5_r24"),
            Some(StratConfig::Bollinger {
                window: 20,
                k: 1.5,
                rebalance_steps: 24,
            })
        );
        assert_eq!(
            parse_strategy_label("last_candle_c4_r24"),
            Some(StratConfig::LastCandle {
                candle_steps: 4,
                rebalance_steps: 24,
            })
        );
        assert_eq!(
            parse_strategy_label("last_candle_c1_r1"),
            Some(StratConfig::LastCandle {
                candle_steps: 1,
                rebalance_steps: 1,
            })
        );
        assert_eq!(
            parse_strategy_label("last_candle_t3600_r14400"),
            Some(StratConfig::LastCandleTime {
                candle_seconds: 3600,
                rebalance_seconds: 14400,
            })
        );
        assert_eq!(
            parse_strategy_label("periodic_steps_3"),
            Some(StratConfig::PeriodicSteps(3))
        );
    }

    #[test]
    fn bollinger_flat_price_rebalances_with_fallback_band() {
        let mut steps = Vec::new();
        for i in 0..30u64 {
            let mut s = step(dec!(100), dec!(1));
            s.start_timestamp = i * 3600;
            steps.push(s);
        }
        let (_lo, _hi, name, summary) = run_single(
            &steps,
            Price::new(dec!(100)),
            100.0,
            0.10,
            StratConfig::Bollinger {
                window: 20,
                k: 2.0,
                rebalance_steps: 25,
            },
            dec!(1000),
            dec!(0),
            dec!(0),
            None,
            9,
            9,
            None,
            None,
        );
        assert!(name.contains("bollinger"));
        assert!(summary.rebalance_count >= 1);
    }

    #[test]
    fn last_candle_triggers_one_rebalance_on_schedule() {
        let mut steps = Vec::new();
        for i in 0..8u64 {
            let mut s = step(dec!(100) + Decimal::from(i), dec!(1));
            s.start_timestamp = i * 3600;
            steps.push(s);
        }
        let (_lo, _hi, _name, summary) = run_single(
            &steps,
            Price::new(dec!(100)),
            100.0,
            0.20,
            StratConfig::LastCandle {
                candle_steps: 4,
                rebalance_steps: 8,
            },
            dec!(1000),
            dec!(0),
            dec!(0),
            None,
            9,
            9,
            None,
            None,
        );
        assert_eq!(summary.rebalance_count, 1);
    }

    /// `OorRecenter` vs `RetouchShift` share the same code path until the first OOR event; with no
    /// OOR they match `Static` (no rebalances). With a single sustained OOR plateau they often both
    /// rebalance **once** — metrics can look identical after rounding even though post-rebalance
    /// geometries differ (symmetric % band vs edge-preserving shift).
    #[test]
    fn oor_recenter_matches_retouch_shift_when_price_never_leaves_initial_band() {
        let mut steps = Vec::new();
        for i in 0..12u64 {
            let mut s = step(dec!(100), dec!(1));
            s.start_timestamp = i * 3600;
            s.step_volume_usd = Decimal::ZERO;
            steps.push(s);
        }
        let cap = dec!(10_000);
        let tx = dec!(1);
        let w = 0.10;
        let (_, _, _, s_oor) = run_single(
            &steps,
            Price::new(dec!(100)),
            100.0,
            w,
            StratConfig::OorRecenter,
            cap,
            tx,
            dec!(0.0),
            None,
            9,
            6,
            None,
            None,
        );
        let (_, _, _, s_ret) = run_single(
            &steps,
            Price::new(dec!(100)),
            100.0,
            w,
            StratConfig::RetouchShift {
                retouch_offset_pct: 0.0,
            },
            cap,
            tx,
            dec!(0.0),
            None,
            9,
            6,
            None,
            None,
        );
        assert_eq!(s_oor.rebalance_count, 0);
        assert_eq!(s_ret.rebalance_count, 0);
        assert!((s_oor.final_value - s_ret.final_value).abs() < dec!(0.0001));
    }

    /// After the first OOR fix, `OorRecenter` can fire again on the **next** step if the price path
    /// immediately leaves the freshly centered band. `RetouchShift` arms only after an in-range
    /// step, so a monotonic climb typically produces **fewer** retouches than oor recenters.
    #[test]
    fn oor_recenter_rebalances_more_often_than_retouch_on_monotonic_climb_after_oor() {
        let mut steps = Vec::new();
        for i in 0..3u64 {
            let mut s = step(dec!(100), dec!(1));
            s.start_timestamp = i;
            s.step_volume_usd = Decimal::ZERO;
            steps.push(s);
        }
        let climb = [
            dec!(111),
            dec!(120),
            dec!(130),
            dec!(141),
            dec!(153),
            dec!(166),
            dec!(180),
            dec!(195),
        ];
        for (j, px) in climb.iter().enumerate() {
            let mut s = step(*px, dec!(1));
            s.start_timestamp = 10 + j as u64;
            s.step_volume_usd = Decimal::ZERO;
            steps.push(s);
        }
        let cap = dec!(10_000);
        let tx = dec!(5);
        let w = 0.10;
        let (_, _, _, s_oor) = run_single(
            &steps,
            Price::new(dec!(100)),
            100.0,
            w,
            StratConfig::OorRecenter,
            cap,
            tx,
            dec!(0.0),
            None,
            9,
            6,
            None,
            None,
        );
        let (_, _, _, s_ret) = run_single(
            &steps,
            Price::new(dec!(100)),
            100.0,
            w,
            StratConfig::RetouchShift {
                retouch_offset_pct: 0.0,
            },
            cap,
            tx,
            dec!(0.0),
            None,
            9,
            6,
            None,
            None,
        );
        assert!(
            s_oor.rebalance_count > s_ret.rebalance_count,
            "expected more oor recenters than retouches on monotonic climb; oor={} retouch={}",
            s_oor.rebalance_count,
            s_ret.rebalance_count
        );
    }
}
