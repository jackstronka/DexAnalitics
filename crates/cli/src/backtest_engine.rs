//! Shared backtest logic: step data building and grid execution.
//!
//! Used by both `backtest` (single run) and `backtest-optimize` (grid + rolling windows).

use clmm_lp_domain::prelude::{Amount, Price, PriceCandle};
use clmm_lp_simulation::prelude::*;
use clmm_lp_data::swaps::SwapEvent;
use primitive_types::U256;
use rayon::prelude::*;
use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use crate::engine::{fees as fee_engine, hodl, liquidity};
use crate::engine::pricing::price_ab_human_to_raw;

/// Per-step data used by simulations.
#[derive(Clone, Copy, Debug)]
pub struct StepDataPoint {
    /// Price of token A in USD (A/USD). For cross-pairs derived as (A/B) * (B/USD).
    pub price_usd: Price,
    /// Price of token A denominated in token B (A/B). Used for liquidity math.
    pub price_ab: Price,
    /// Step volume in USD (already scaled/distributed).
    pub step_volume_usd: Decimal,
    /// Quote token (B) price in USD at this step (1.0 if quote is USDC).
    pub quote_usd: Decimal,
    /// LP share proxy (legacy; replaced by liquidity-share model when available).
    pub lp_share: Decimal,
    /// Active pool liquidity (from protocol state) at this step.
    /// Only available in snapshot-only price path mode.
    pub pool_liquidity_active: Option<u128>,
    /// Candle start timestamp (seconds).
    pub start_timestamp: u64,
}

pub type StepData = StepDataPoint;

/// Strategy variant for grid search.
#[derive(Clone, Copy, Debug)]
pub enum StratConfig {
    Static,
    Threshold(f64),
    Periodic(u64),
}

/// Shared parameters for a single backtest or grid run (capital, fees, pool, decimals).
#[derive(Clone, Debug)]
pub struct GridRunParams {
    pub capital_dec: Decimal,
    pub tx_cost_dec: Decimal,
    /// Optional realistic rebalance cost model:
    /// cost = fixed_cost_usd + notional_usd * slippage_bps / 10_000.
    pub rebalance_cost_model: Option<RebalanceCostModel>,
    pub fee_rate: Decimal,
    pub pool_active_liquidity: Option<u128>,
    pub token_a_decimals: u32,
    pub token_b_decimals: u32,
    /// Step duration in seconds (candle resolution), e.g. 3600 for 1H.
    pub step_seconds: i64,
    /// If true, use liquidity-based fee share when pool liquidity is known.
    /// If false, always use `StepDataPoint.lp_share` (e.g. when `--lp-share` is provided).
    pub use_liquidity_share: bool,
}

/// Realistic rebalance cost model.
#[derive(Clone, Copy, Debug)]
pub struct RebalanceCostModel {
    /// Fixed USD component charged on each rebalance
    /// (network + priority + optional tip + extra fixed overhead).
    pub fixed_cost_usd: Decimal,
    /// Slippage in basis points applied to current rebalanced notional.
    pub slippage_bps: Decimal,
}

impl RebalanceCostModel {
    #[must_use]
    pub fn cost_for_notional(&self, notional_usd: Decimal) -> Decimal {
        let slip = if self.slippage_bps > Decimal::ZERO && notional_usd > Decimal::ZERO {
            notional_usd * self.slippage_bps / Decimal::from(10_000u32)
        } else {
            Decimal::ZERO
        };
        self.fixed_cost_usd + slip
    }
}

/// Build step data (price, volume, share) for each candle.
///
/// **Volume:** When Dune TVL/volume is present we use **hybrid** volume:
/// - Per-candle USD volume from Birdeye (`volume_token_a * close`) gives the **intraday distribution**
///   (high volume hours get more volume; often those are volatile hours when price may be out of range).
/// - Dune daily volume for the pool gives the **scale** so the day total matches the pool.
/// - So: `step_vol_usd = dune_daily_vol * (candle_vol_usd / birdeye_day_total)`.
///
/// When Birdeye has no volume for a day we fall back to uniform `daily_vol / 24`.
/// Without Dune we use Birdeye candle volume as-is (realistic distribution, scale from lp_share).
pub fn build_step_data(
    candle_slice: &[PriceCandle],
    dune_tvl: Option<&HashMap<String, Decimal>>,
    dune_vol: Option<&HashMap<String, Decimal>>,
    quote_usd_map: Option<&HashMap<u64, Decimal>>,
    capital_dec: Decimal,
    lp_share_override: Option<Decimal>,
    steps_per_day: Decimal,
) -> (Vec<StepData>, Price, f64) {
    let mut vol_model = ConstantVolume::from_amount(Amount::new(U256::from(1_000_000_000_000u64), 6));
    // Determine entry price in USD (for cross-pairs multiply by quote USD).
    let entry_ab = candle_slice
        .first()
        .map(|c| c.close)
        .unwrap_or_else(|| Price::new(Decimal::ONE));
    let entry_quote_usd = candle_slice
        .first()
        .and_then(|c| quote_usd_map.and_then(|m| m.get(&c.start_timestamp).copied()))
        .unwrap_or(Decimal::ONE);
    let entry = Price::new(entry_ab.value * entry_quote_usd);
    let center = entry.value.to_f64().unwrap_or(1.0);

    // Per-candle USD volume from Birdeye (distribution); per-day totals for scaling
    let candle_vol_usd: Vec<Decimal> = candle_slice
        .iter()
        .map(|c| {
            let quote_usd = quote_usd_map
                .and_then(|m| m.get(&c.start_timestamp).copied())
                .unwrap_or(Decimal::ONE);
            let price_usd = c.close.value * quote_usd;
            c.volume_token_a.to_decimal() * price_usd
        })
        .collect();
    let mut birdeye_day_total: HashMap<String, Decimal> = HashMap::new();
    for (candle, vol) in candle_slice.iter().zip(candle_vol_usd.iter()) {
        let date_key = chrono::DateTime::from_timestamp(candle.start_timestamp as i64, 0)
            .unwrap_or_default()
            .format("%Y-%m-%d")
            .to_string();
        *birdeye_day_total.entry(date_key).or_insert(Decimal::ZERO) += *vol;
    }

    let data: Vec<StepData> = candle_slice
        .iter()
        .zip(candle_vol_usd.iter())
        .map(|(candle, candle_vol_usd)| {
            let date_key = chrono::DateTime::from_timestamp(candle.start_timestamp as i64, 0)
                .unwrap_or_default()
                .format("%Y-%m-%d")
                .to_string();

            let (step_vol, share) = if let (Some(tvl_map), Some(vol_map)) = (dune_tvl, dune_vol) {
                let daily_tvl = tvl_map.get(&date_key).cloned().unwrap_or(Decimal::ZERO);
                let daily_vol = vol_map.get(&date_key).cloned().unwrap_or(Decimal::ZERO);
                if daily_tvl.is_zero() || daily_vol.is_zero() {
                    (
                        vol_model.next_volume().to_decimal(),
                        lp_share_override.unwrap_or_else(|| Decimal::from_f64(0.01).unwrap()),
                    )
                } else {
                    let share = lp_share_override.unwrap_or_else(|| {
                        (capital_dec / daily_tvl).min(Decimal::ONE).max(Decimal::ZERO)
                    });
                    let day_total = birdeye_day_total.get(&date_key).copied().unwrap_or(Decimal::ZERO);
                    let step_vol = if day_total > Decimal::ZERO && *candle_vol_usd > Decimal::ZERO {
                        daily_vol * (*candle_vol_usd / day_total)
                    } else {
                        daily_vol / steps_per_day
                    };
                    (step_vol, share)
                }
            } else {
                let share = lp_share_override.unwrap_or_else(|| Decimal::from_f64(0.01).unwrap());
                let step_vol = if *candle_vol_usd > Decimal::ZERO {
                    *candle_vol_usd
                } else {
                    vol_model.next_volume().to_decimal()
                };
                (step_vol, share)
            };
            let quote_usd = quote_usd_map
                .and_then(|m| m.get(&candle.start_timestamp).copied())
                .unwrap_or(Decimal::ONE);
            let price_ab = candle.close;
            let price_usd = Price::new(price_ab.value * quote_usd);
            StepDataPoint {
                price_usd,
                price_ab,
                step_volume_usd: step_vol,
                quote_usd,
                lp_share: share,
                pool_liquidity_active: None,
                start_timestamp: candle.start_timestamp,
            }
        })
        .collect();
    (data, entry, center)
}

/// Fee realism: total period volume (USD) and expected fees if 100% TIR (volume × share × fee_tier).
/// Use with simulated fees to check: simulated / expected_100_tir ≈ fee-weighted time-in-range.
pub fn fee_realism(step_data: &[StepData], fee_rate: Decimal) -> (Decimal, Decimal) {
    let (total_vol, weighted_vol) = step_data.iter().fold(
        (Decimal::ZERO, Decimal::ZERO),
        |(tv, wv), p| (tv + p.step_volume_usd, wv + p.step_volume_usd * p.lp_share),
    );
    let expected_fees_100_tir = weighted_vol * fee_rate;
    (total_vol, expected_fees_100_tir)
}

/// Estimates initial position liquidity (L) for a given range and capital in USD.
///
/// Requires quote token USD price at entry (from `step_data[0].quote_usd`) and token B decimals.
pub fn estimate_position_liquidity(
    step_data: &[StepData],
    lower: Decimal,
    upper: Decimal,
    capital_usd: Decimal,
    token_a_decimals: u32,
    token_b_decimals: u32,
) -> u128 {
    liquidity::estimate_position_liquidity(
        step_data,
        lower,
        upper,
        capital_usd,
        token_a_decimals,
        token_b_decimals,
    )
}

/// Index swap events by step index. Step duration assumed 3600s (1h). Swaps whose block_time
/// falls in [step_start, step_start + 3600) are assigned to that step.
fn index_swaps_by_step<'a>(
    swaps: &'a [SwapEvent],
    step_data: &[StepData],
    step_seconds: i64,
) -> BTreeMap<usize, Vec<&'a SwapEvent>> {
    let mut map: BTreeMap<usize, Vec<&SwapEvent>> = BTreeMap::new();
    if step_data.is_empty() {
        return map;
    }
    let start_ts = step_data[0].start_timestamp as i64;
    for s in swaps {
        if let Some(dt) = s.block_time_utc() {
            let delta = dt.timestamp() - start_ts;
            if delta >= 0 {
                let idx = (delta / step_seconds) as usize;
                map.entry(idx).or_default().push(s);
            }
        }
    }
    map
}

/// Run a single backtest (one range, one strategy) over step data. Returns (lower, upper, strat_name, summary).
///
/// Fee precedence per step:
/// 1) `local_pool_fees_usd` if non-empty map (decoded / timing proxy from local JSONL)
/// 2) `swaps` (Dune `SwapEvent` slice), if non-empty
/// 3) candle volume × `fee_rate`
pub fn run_single(
    step_data: &[StepData],
    entry_price: Price,
    center: f64,
    width_pct: f64,
    strat: StratConfig,
    params: &GridRunParams,
    swaps: Option<&[SwapEvent]>,
    local_pool_fees_usd: Option<&BTreeMap<usize, Decimal>>,
) -> (f64, f64, String, TrackerSummary) {
    let capital_dec = params.capital_dec;
    let tx_cost_dec = params.tx_cost_dec;
    let rebalance_cost_model = params.rebalance_cost_model;
    let fee_rate = params.fee_rate;
    let pool_active_liquidity = params.pool_active_liquidity;
    let token_a_decimals = params.token_a_decimals;
    let token_b_decimals = params.token_b_decimals;
    let _ = entry_price; // kept for API compatibility; amount-based sim derives entry from step_data[0]
    // Amount-based accounting:
    // - Range is defined in A/B (quote units) and checked against `price_ab`
    // - Position value is derived from real amounts (amountA/amountB) computed from liquidity L
    // - Rebalance closes and reopens the position at the current price, paying tx cost
    let Some(first) = step_data.first() else {
        return (
            center * (1.0 - width_pct / 2.0),
            center * (1.0 + width_pct / 2.0),
            "empty".to_string(),
            TrackerSummary {
                total_steps: 0,
                final_value: capital_dec,
                final_pnl: Decimal::ZERO,
                final_il_pct: Decimal::ZERO,
                total_fees: Decimal::ZERO,
                time_in_range_pct: Decimal::ZERO,
                rebalance_count: 0,
                total_rebalance_cost: Decimal::ZERO,
                max_drawdown: Decimal::ZERO,
                hodl_value: capital_dec,
                vs_hodl: Decimal::ZERO,
            },
        );
    };

    let half = width_pct / 2.0;
    let center_ab = first.price_ab.value.to_f64().unwrap_or(1.0);
    let lower_ab = Decimal::from_f64(center_ab * (1.0 - half)).unwrap();
    let upper_ab = Decimal::from_f64(center_ab * (1.0 + half)).unwrap();

    // For reporting only, return bounds in USD using entry quote USD.
    let entry_quote_usd = first.quote_usd;
    let lower_usd = lower_ab * entry_quote_usd;
    let upper_usd = upper_ab * entry_quote_usd;
    let lower = lower_usd.to_f64().unwrap_or(center * (1.0 - half));
    let upper = upper_usd.to_f64().unwrap_or(center * (1.0 + half));

    // Current position state
    let mut current_lower_ab = lower_ab;
    let mut current_upper_ab = upper_ab;
    let mut liquidity_l: u128 = liquidity::estimate_position_liquidity(
        step_data,
        lower_usd,
        upper_usd,
        capital_dec,
        token_a_decimals,
        token_b_decimals,
    );

    let mut total_fees = Decimal::ZERO;
    let mut total_rebalance_cost = Decimal::ZERO;
    let mut rebalance_count: u32 = 0;
    let mut steps_since_rebalance: u64 = 0;
    let mut in_range_steps: u64 = 0;

    // equity curve for max drawdown
    let mut peak_equity = capital_dec;
    let mut max_drawdown = Decimal::ZERO;
    let strat_name = match strat {
        StratConfig::Static => "static".to_string(),
        StratConfig::Threshold(p) => format!("threshold_{:.0}%", p * 100.0),
        StratConfig::Periodic(h) => format!("periodic_{}h", h),
    };

    let mut fee_share_model = if params.use_liquidity_share {
        pool_active_liquidity
            .filter(|v| *v > 0)
            .map(|pool_l| fee_engine::FeeShareModel::LiquidityShare {
                position_liquidity: liquidity_l,
                pool_active_liquidity: pool_l,
            })
            .unwrap_or(fee_engine::FeeShareModel::LegacyLpShare)
    } else {
        fee_engine::FeeShareModel::LegacyLpShare
    };

    let swap_index: Option<BTreeMap<usize, Vec<&SwapEvent>>> = swaps
        .filter(|s| !s.is_empty())
        .map(|s| index_swaps_by_step(s, step_data, params.step_seconds.max(1)));

    for (i, p) in step_data.iter().enumerate() {
        steps_since_rebalance += 1;
        let price_ab = p.price_ab.value;
        let in_range = price_ab >= current_lower_ab && price_ab <= current_upper_ab;
        if in_range {
            in_range_steps += 1;
        }

        let pool_fees = if let Some(m) = local_pool_fees_usd.filter(|m| !m.is_empty()) {
            m.get(&i).copied().unwrap_or(Decimal::ZERO)
        } else if let Some(ref idx) = swap_index {
            idx.get(&i)
                .map(|swaps_here| {
                    swaps_here.iter().fold(Decimal::ZERO, |acc, s| {
                        let f = if s.fee_usd != Decimal::ZERO {
                            s.fee_usd
                        } else {
                            s.amount_usd * s.fee_tier
                        };
                        acc + f
                    })
                })
                .unwrap_or(Decimal::ZERO)
        } else {
            p.step_volume_usd * fee_rate
        };

        let step_fees: Decimal = if in_range {
            pool_fees * fee_share_model.step_fee_share(p)
        } else {
            Decimal::ZERO
        };
        total_fees += step_fees;

        // Current position valuation (excluding fees)
        let lower_ab_raw = price_ab_human_to_raw(current_lower_ab, token_a_decimals, token_b_decimals);
        let upper_ab_raw = price_ab_human_to_raw(current_upper_ab, token_a_decimals, token_b_decimals);
        let price_ab_raw = price_ab_human_to_raw(price_ab, token_a_decimals, token_b_decimals);

        let sqrt_l = crate::engine::pricing::price_to_sqrt_q64(lower_ab_raw);
        let sqrt_u = crate::engine::pricing::price_to_sqrt_q64(upper_ab_raw);
        let sqrt_p = crate::engine::pricing::price_to_sqrt_q64(price_ab_raw);
        let (amt_a_base, amt_b_base) =
            liquidity::amounts_from_liquidity_at_price(liquidity_l, sqrt_l, sqrt_p, sqrt_u);
        let amt_a = crate::engine::pricing::from_base_units(amt_a_base, token_a_decimals);
        let amt_b = crate::engine::pricing::from_base_units(amt_b_base, token_b_decimals);
        let position_value_usd = (amt_a * p.price_usd.value) + (amt_b * p.quote_usd);

        // `position_value_usd` is already net of any rebalance costs that were paid when
        // reopening the position (we redeploy `position_value_usd - tx_cost`).
        // So for equity/final value we must NOT subtract `total_rebalance_cost` again.
        let equity = position_value_usd + total_fees;
        if equity > peak_equity {
            peak_equity = equity;
        }
        if peak_equity > Decimal::ZERO {
            let dd = (peak_equity - equity) / peak_equity;
            if dd > max_drawdown {
                max_drawdown = dd;
            }
        }

        let should_rebalance = match strat {
            StratConfig::Static => false,
            StratConfig::Threshold(th) => {
                if !in_range {
                    true
                } else {
                    let mid = (current_lower_ab + current_upper_ab) / Decimal::from(2u32);
                    if mid.is_zero() {
                        false
                    } else {
                        let change = ((price_ab - mid) / mid).abs();
                        change >= Decimal::from_f64(th).unwrap_or(Decimal::ZERO)
                    }
                }
            }
            StratConfig::Periodic(interval_hours) => {
                let elapsed = (steps_since_rebalance as i64) * params.step_seconds.max(1);
                elapsed as u64 >= interval_hours.saturating_mul(3600)
            }
        };

        if should_rebalance && liquidity_l > 0 {
            let rebalance_cost = if let Some(model) = rebalance_cost_model {
                model.cost_for_notional(position_value_usd)
            } else {
                tx_cost_dec
            };
            total_rebalance_cost += rebalance_cost;
            rebalance_count += 1;
            steps_since_rebalance = 0;

            // Re-deploy current position value minus tx cost; fees are NOT compounded here.
            let capital_usd_now = (position_value_usd - rebalance_cost).max(Decimal::ZERO);
            let center_ab_now = price_ab.to_f64().unwrap_or(1.0);
            let new_lower_ab = Decimal::from_f64(center_ab_now * (1.0 - half)).unwrap();
            let new_upper_ab = Decimal::from_f64(center_ab_now * (1.0 + half)).unwrap();
            current_lower_ab = new_lower_ab;
            current_upper_ab = new_upper_ab;

            // Convert AB bounds to USD using current quote USD for liquidity estimation.
            let new_lower_usd = current_lower_ab * p.quote_usd;
            let new_upper_usd = current_upper_ab * p.quote_usd;
            liquidity_l = liquidity::estimate_position_liquidity(
                step_data,
                new_lower_usd,
                new_upper_usd,
                capital_usd_now,
                token_a_decimals,
                token_b_decimals,
            );

            if params.use_liquidity_share {
                if let Some(pool_l) = pool_active_liquidity.filter(|v| *v > 0) {
                    fee_share_model = fee_engine::FeeShareModel::LiquidityShare {
                        position_liquidity: liquidity_l,
                        pool_active_liquidity: pool_l,
                    };
                }
            }
        }
    }

    let total_steps = step_data.len() as u64;
    let time_in_range_pct = if total_steps > 0 {
        Decimal::from(in_range_steps) / Decimal::from(total_steps)
    } else {
        Decimal::ZERO
    };

    let last = step_data.last().unwrap();
    let lower_ab_raw = price_ab_human_to_raw(current_lower_ab, token_a_decimals, token_b_decimals);
    let upper_ab_raw = price_ab_human_to_raw(current_upper_ab, token_a_decimals, token_b_decimals);
    let last_ab_raw = price_ab_human_to_raw(last.price_ab.value, token_a_decimals, token_b_decimals);

    let sqrt_l = crate::engine::pricing::price_to_sqrt_q64(lower_ab_raw);
    let sqrt_u = crate::engine::pricing::price_to_sqrt_q64(upper_ab_raw);
    let sqrt_p = crate::engine::pricing::price_to_sqrt_q64(last_ab_raw);
    let (amt_a_base, amt_b_base) =
        liquidity::amounts_from_liquidity_at_price(liquidity_l, sqrt_l, sqrt_p, sqrt_u);
    let amt_a = crate::engine::pricing::from_base_units(amt_a_base, token_a_decimals);
    let amt_b = crate::engine::pricing::from_base_units(amt_b_base, token_b_decimals);
    let position_value_usd = (amt_a * last.price_usd.value) + (amt_b * last.quote_usd);

    let final_value = position_value_usd + total_fees;
    let final_pnl = final_value - capital_dec;
    let hodl_value = hodl::hodl_value_50_50_usd(step_data, capital_dec);
    let vs_hodl = final_value - hodl_value;

    // "IL%" in amount-based mode: define as **under/over-performance vs HODL excluding fees**,
    // i.e. compare HODL to the underlying position value before fees (and before rebalance costs).
    //
    // This is not Uniswap's instantaneous IL formula; it's a backtest-end accounting metric that
    // stays consistent across static and rebalancing strategies.
    let position_value_before_fees = position_value_usd;
    let position_value_before_costs = position_value_before_fees + total_rebalance_cost;
    let il_like_pct = if capital_dec > Decimal::ZERO {
        (position_value_before_costs - hodl_value) / capital_dec
    } else {
        Decimal::ZERO
    };

    let summary = TrackerSummary {
        total_steps,
        final_value,
        final_pnl,
        final_il_pct: il_like_pct,
        total_fees,
        time_in_range_pct,
        rebalance_count,
        total_rebalance_cost,
        max_drawdown,
        hodl_value,
        vs_hodl,
    };

    (lower, upper, strat_name, summary)
}

/// Run grid of (width_pct, strategy) in parallel. Returns (width_pct, lower, upper, strat_name, summary).
pub fn run_grid(
    step_data: &[StepData],
    entry_price: Price,
    center: f64,
    width_pcts: &[f64],
    strategies: &[StratConfig],
    params: &GridRunParams,
    swaps: Option<&[SwapEvent]>,
    local_pool_fees_usd: Option<Arc<BTreeMap<usize, Decimal>>>,
) -> Vec<(f64, f64, f64, String, TrackerSummary)> {
    let step_data = Arc::new(step_data.to_vec());
    let swaps_arc: Option<Arc<Vec<SwapEvent>>> = swaps.map(|s| Arc::new(s.to_vec()));
    let jobs: Vec<(f64, StratConfig)> = width_pcts
        .iter()
        .flat_map(|&wp| strategies.iter().copied().map(move |s| (wp, s)))
        .collect();
    jobs.par_iter()
        .map(|(wp, strat)| {
            let swaps_ref: Option<&[SwapEvent]> = swaps_arc.as_deref().map(|v| v.as_slice());
            let local_ref = local_pool_fees_usd.as_deref();
            let (lower, upper, strat_name, summary) = run_single(
                step_data.as_ref(),
                entry_price,
                center,
                *wp,
                *strat,
                params,
                swaps_ref,
                local_ref,
            );
            (*wp, lower, upper, strat_name, summary)
        })
        .collect()
}
