use prettytable::{Table, row};
use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};

use crate::backtest_engine::StepData;
use clmm_lp_simulation::prelude::TrackerSummary;

pub fn round2(d: Decimal) -> Decimal {
    (d * Decimal::from(100)).round() / Decimal::from(100)
}

fn pct(x: Decimal) -> f64 {
    (x * Decimal::from(100)).to_f64().unwrap_or(0.0)
}

fn quote_usd(step_data: Option<&Vec<StepData>>) -> Option<Decimal> {
    step_data
        .and_then(|v| v.first().map(|p| p.quote_usd))
        .filter(|q| *q > Decimal::ZERO)
}

fn price_low_high_ab(step_data: &[StepData]) -> Option<(Decimal, Decimal)> {
    let mut low: Option<Decimal> = None;
    let mut high: Option<Decimal> = None;
    for p in step_data.iter() {
        let v = p.price_ab.value;
        low = Some(low.map_or(v, |l| l.min(v)));
        high = Some(high.map_or(v, |h| h.max(v)));
    }
    match (low, high) {
        (Some(l), Some(h)) if l > Decimal::ZERO && h > Decimal::ZERO => Some((l, h)),
        _ => None,
    }
}

fn bounds_in_quote_units(lower_usd: f64, upper_usd: f64, quote_usd: Decimal) -> (Option<Decimal>, Option<Decimal>) {
    if quote_usd <= Decimal::ZERO {
        return (None, None);
    }
    let lo = Decimal::from_f64(lower_usd).map(|d| d / quote_usd);
    let up = Decimal::from_f64(upper_usd).map(|d| d / quote_usd);
    (lo, up)
}

/// `period_label` should match the fetched span, e.g. `last 24 hour(s)` or `last 30 day(s)`.
#[allow(clippy::too_many_arguments)]
pub fn print_best_block(
    pair_label: &str,
    period_label: &str,
    capital: &f64,
    windows: &usize,
    objective_label: &str,
    min_range_pct: &f64,
    max_range_pct: &f64,
    strategies_len: usize,
    min_time_in_range: &Option<f64>,
    max_drawdown: &Option<f64>,
    best: &(f64, f64, f64, String, TrackerSummary, Decimal),
    capital_dec: Decimal,
    effective_fee_rate: Decimal,
    pool_active_liquidity: Option<u128>,
    audit_step_data: Option<&Vec<StepData>>,
    token_a_decimals: u32,
    token_b_decimals: u32,
    symbol_a: &str,
    symbol_b: Option<&str>,
) {
    println!();
    println!(
        "📊 BACKTEST OPTIMIZE: {} ({} — capital ${}, {} window(s))",
        pair_label, period_label, capital, windows
    );
    println!(
        "   Objective: {}   Range: {:.1}%-{:.1}%   Strategies: {}   Filters: TIR>={:?}%  DD<={:?}%",
        objective_label, min_range_pct, max_range_pct, strategies_len, min_time_in_range, max_drawdown
    );
    println!();
    println!(
        "🏆 BEST: width {:.2}%   range ${:.2} - ${:.2}   strategy {}   Score: {}   PnL: {}   vs HODL: {}",
        best.0 * 100.0,
        best.1,
        best.2,
        best.3,
        round2(best.5),
        round2(best.4.final_pnl),
        round2(best.4.vs_hodl),
    );
    println!(
        "   Final value: ${:.2}   Fees: ${:.2}   Drag(ex-fees vs HODL): {:.2}%   TIR: {:.1}%   Rebalances: {} (${:.2})",
        best.4.final_value,
        best.4.total_fees,
        best.4.final_il_pct * Decimal::from(100),
        best.4.time_in_range_pct * Decimal::from(100),
        best.4.rebalance_count,
        best.4.total_rebalance_cost,
    );
    let hodl_pnl = best.4.hodl_value - capital_dec;
    println!(
        "   HODL: final ${:.2}   PnL {:+.2}",
        round2(best.4.hodl_value),
        round2(hodl_pnl)
    );

    if let (Some(pool_l), Some(step_data)) = (pool_active_liquidity, audit_step_data)
        && pool_l > 0
    {
        use crate::engine::liquidity as liq_engine;
        use crate::engine::pricing::{from_base_units, price_ab_human_to_raw, price_to_sqrt_q64};

        // HODL benchmark: hold the same initial token amounts that correspond to
        // the LP entry liquidity for the BEST initial range.
        let first = step_data.first().expect("step_data must be non-empty");
        let lower_usd = Decimal::from_f64(best.1).unwrap_or(Decimal::ZERO);
        let upper_usd = Decimal::from_f64(best.2).unwrap_or(Decimal::ZERO);
        let quote_usd = first.quote_usd;

        let lower_ab = if quote_usd > Decimal::ZERO {
            lower_usd / quote_usd
        } else {
            Decimal::ZERO
        };
        let upper_ab = if quote_usd > Decimal::ZERO {
            upper_usd / quote_usd
        } else {
            Decimal::ZERO
        };

        let l_pos_hodl = liq_engine::estimate_position_liquidity(
            step_data,
            lower_usd,
            upper_usd,
            capital_dec,
            token_a_decimals,
            token_b_decimals,
        );

        let lower_ab_raw =
            price_ab_human_to_raw(lower_ab, token_a_decimals, token_b_decimals);
        let upper_ab_raw =
            price_ab_human_to_raw(upper_ab, token_a_decimals, token_b_decimals);
        let entry_ab_raw = price_ab_human_to_raw(
            first.price_ab.value,
            token_a_decimals,
            token_b_decimals,
        );

        let sqrt_l = price_to_sqrt_q64(lower_ab_raw);
        let sqrt_u = price_to_sqrt_q64(upper_ab_raw);
        let sqrt_p = price_to_sqrt_q64(entry_ab_raw);

        let (hodl_a_base, hodl_b_base) =
            liq_engine::amounts_from_liquidity_at_price(l_pos_hodl, sqrt_l, sqrt_p, sqrt_u);
        let hodl_a = from_base_units(hodl_a_base, token_a_decimals);
        let hodl_b_amt = from_base_units(hodl_b_base, token_b_decimals);
        // LP end amounts for static range can be reconstructed from L.
        // For rebalancing strategies, end amounts depend on the rebalance path and must come
        // directly from the simulator state (not available here), so we skip the misleading estimate.
        let (lp_a, lp_b, l_pos) = if best.3 == "static" {
            liq_engine::estimate_lp_end_amounts(
                step_data,
                Decimal::from_f64(best.1).unwrap(),
                Decimal::from_f64(best.2).unwrap(),
                capital_dec,
                token_a_decimals,
                token_b_decimals,
            )
        } else {
            (Decimal::ZERO, Decimal::ZERO, 0u128)
        };
        let share_pct = if l_pos > 0 {
            (Decimal::from(l_pos) / Decimal::from(pool_l) * Decimal::from(100))
                .to_f64()
                .unwrap_or(0.0)
        } else {
            0.0
        };
        println!(
            "   Liquidity share (approx): L_pos={}  L_pool={}  share≈{:.6}%",
            l_pos, pool_l, share_pct
        );

        // Sanity: compute in-range volume (USD) for BEST range, and implied share from earned fees.
        // IMPORTANT: "in range" is defined in quote units (A/B), not USD.
        let lo_usd = Decimal::from_f64(best.1).unwrap();
        let up_usd = Decimal::from_f64(best.2).unwrap();
        let q0 = step_data.first().map(|p| p.quote_usd).unwrap_or(Decimal::ZERO);
        let lo_ab = if q0 > Decimal::ZERO { lo_usd / q0 } else { Decimal::ZERO };
        let up_ab = if q0 > Decimal::ZERO { up_usd / q0 } else { Decimal::ZERO };
        let mut in_range_vol = Decimal::ZERO;
        let mut in_range_steps: u64 = 0;
        let mut sum_lp_share = Decimal::ZERO;
        for p in step_data.iter() {
            let px = p.price_ab.value;
            if px >= lo_ab && px <= up_ab {
                in_range_vol += p.step_volume_usd;
                in_range_steps += 1;
                sum_lp_share += p.lp_share;
            }
        }
        if in_range_vol > Decimal::ZERO && effective_fee_rate > Decimal::ZERO {
            let implied_share = best.4.total_fees / (in_range_vol * effective_fee_rate);
            let avg_lp_share = if in_range_steps > 0 {
                sum_lp_share / Decimal::from(in_range_steps)
            } else {
                Decimal::ZERO
            };
            println!(
                "   Sanity: in-range volume ${:.0}, implied share≈{:.6}%, avg lp_share≈{:.6}%",
                in_range_vol,
                pct(implied_share),
                pct(avg_lp_share),
            );
        }
        if let Some(bsym) = symbol_b {
            println!(
                "   HODL amounts (entry LP-derived): {:.6} {} + {:.6} {}",
                hodl_a, symbol_a, hodl_b_amt, bsym
            );
            if best.3 == "static" {
                println!(
                    "   LP amounts (approx, end): {:.6} {} + {:.6} {}",
                    lp_a, symbol_a, lp_b, bsym
                );
            } else {
                println!("   LP amounts (end): (skipped for rebalancing strategy; path-dependent)");
            }
        }

        if let Some(first) = step_data.first()
            && first.quote_usd > Decimal::ZERO
            && symbol_b.is_some()
        {
            let lo_b = Decimal::from_f64(best.1).unwrap() / first.quote_usd;
            let up_b = Decimal::from_f64(best.2).unwrap() / first.quote_usd;
            println!(
                "   Range (quote units): {:.3} - {:.3} {} (using quote≈${:.2})",
                lo_b,
                up_b,
                symbol_b.unwrap(),
                first.quote_usd
            );
        }
    }
}

pub fn build_results_table(
    results: &[(f64, f64, f64, String, TrackerSummary, Decimal)],
    top_n: usize,
    use_cross_pair: bool,
    quote_usd_for_table: Option<Decimal>,
    capital_dec: Decimal,
) -> Table {
    let mut table = Table::new();
    table.add_row(row![
        "Rank",
        "Lower($)",
        "Upper($)",
        "Lower(B)",
        "Upper(B)",
        "Strategy",
        "Score",
        "Fees",
        "Rebals",
        "RebalCost",
        "HODLValue",
        "HODLPnL",
        "FinalValue",
        "PnL",
        "vs HODL",
        "TIR%",
        "Drag%"
    ]);

    for (i, (_wp, lo, up, name, s, sc)) in results.iter().take(top_n).enumerate() {
        let (lo_b, up_b) = if use_cross_pair {
            if let Some(q) = quote_usd_for_table.filter(|q| *q > Decimal::ZERO) {
                let l = Decimal::from_f64(*lo).unwrap() / q;
                let u = Decimal::from_f64(*up).unwrap() / q;
                (format!("{:.3}", l), format!("{:.3}", u))
            } else {
                ("-".to_string(), "-".to_string())
            }
        } else {
            ("-".to_string(), "-".to_string())
        };

        table.add_row(row![
            i + 1,
            format!("{:.2}", lo),
            format!("{:.2}", up),
            lo_b,
            up_b,
            name,
            format!("{}", round2(*sc)),
            format!("{:.2}", round2(s.total_fees)),
            format!("{}", s.rebalance_count),
            format!("{:.2}", round2(s.total_rebalance_cost)),
            format!("{:.2}", round2(s.hodl_value)),
            format!("{:+.2}", round2(s.hodl_value - capital_dec)),
            format!("{:.2}", round2(s.final_value)),
            format!("{:+.2}", round2(s.final_pnl)),
            format!("{:+.2}", round2(s.vs_hodl)),
            format!("{:.1}%", s.time_in_range_pct * Decimal::from(100)),
            format!("{:.2}%", s.final_il_pct * Decimal::from(100)),
        ]);
    }
    table
}

pub fn print_candidate_sets(
    results: &[(f64, f64, f64, String, TrackerSummary, Decimal)],
    top_n: usize,
    use_cross_pair: bool,
    audit_step_data: Option<&Vec<StepData>>,
    capital_dec: Decimal,
) {
    if results.is_empty() {
        return;
    }

    let q = quote_usd(audit_step_data);
    let low_high = audit_step_data.and_then(|v| price_low_high_ab(v));

    // Helper: pick top-K by a key
    let mut by_fees = results.to_vec();
    by_fees.sort_by(|a, b| b.4.total_fees.partial_cmp(&a.4.total_fees).unwrap_or(std::cmp::Ordering::Equal));
    let mut by_vs = results.to_vec();
    by_vs.sort_by(|a, b| b.4.vs_hodl.partial_cmp(&a.4.vs_hodl).unwrap_or(std::cmp::Ordering::Equal));
    let mut by_dd = results.to_vec();
    by_dd.sort_by(|a, b| a.4.max_drawdown.partial_cmp(&b.4.max_drawdown).unwrap_or(std::cmp::Ordering::Equal));
    let mut by_tir = results.to_vec();
    by_tir.sort_by(|a, b| b.4.time_in_range_pct.partial_cmp(&a.4.time_in_range_pct).unwrap_or(std::cmp::Ordering::Equal));

    fn print_section(
        title: &str,
        rows: &[(f64, f64, f64, String, TrackerSummary, Decimal)],
        top_n: usize,
        use_cross_pair: bool,
        q: Option<Decimal>,
        low_high: Option<(Decimal, Decimal)>,
        capital_dec: Decimal,
    ) {
        println!();
        println!("== {} ==", title);
        let mut t = Table::new();
        t.add_row(row![
            "Rank",
            "Lower(B)",
            "Upper(B)",
            "ΔLow%",
            "ΔHigh%",
            "Fees",
            "PnL",
            "vsHODL",
            "TIR%",
            "DD%",
            "Rebals"
        ]);
        for (i, (_wp, lo_usd, up_usd, name, s, _sc)) in rows.iter().take(top_n).enumerate() {
            let (lo_b, up_b) = if use_cross_pair {
                if let Some(qv) = q {
                    let (lo, up) = bounds_in_quote_units(*lo_usd, *up_usd, qv);
                    (lo.unwrap_or(Decimal::ZERO), up.unwrap_or(Decimal::ZERO))
                } else {
                    (Decimal::ZERO, Decimal::ZERO)
                }
            } else {
                (Decimal::ZERO, Decimal::ZERO)
            };
            let (dlow, dhigh) = if let (Some((low, high)), true) = (low_high, use_cross_pair) {
                let dlow = if low > Decimal::ZERO { (lo_b - low) / low } else { Decimal::ZERO };
                let dhigh = if high > Decimal::ZERO { (high - up_b) / high } else { Decimal::ZERO };
                (dlow, dhigh)
            } else {
                (Decimal::ZERO, Decimal::ZERO)
            };
            let _ = name;
            let _ = capital_dec;
            t.add_row(row![
                i + 1,
                format!("{:.3}", lo_b),
                format!("{:.3}", up_b),
                format!("{:+.1}%", pct(dlow)),
                format!("{:+.1}%", pct(dhigh)),
                format!("{:.2}", round2(s.total_fees)),
                format!("{:+.2}", round2(s.final_pnl)),
                format!("{:+.2}", round2(s.vs_hodl)),
                format!("{:.1}%", pct(s.time_in_range_pct)),
                format!("{:.1}%", pct(s.max_drawdown)),
                format!("{}", s.rebalance_count),
            ]);
        }
        t.printstd();
    }

    print_section("Max Fees", &by_fees, top_n.min(5), use_cross_pair, q, low_high, capital_dec);
    print_section("Max vs HODL", &by_vs, top_n.min(5), use_cross_pair, q, low_high, capital_dec);
    print_section("Conservative (min drawdown)", &by_dd, top_n.min(5), use_cross_pair, q, low_high, capital_dec);
    print_section("Max Time-in-Range", &by_tir, top_n.min(5), use_cross_pair, q, low_high, capital_dec);
}

