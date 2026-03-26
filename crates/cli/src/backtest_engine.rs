//! Shared backtest logic: step data building and grid execution.
//!
//! Used by both `backtest` (single run) and `backtest-optimize` (grid + rolling windows).

use crate::engine::pricing::{from_base_units, price_ab_human_to_raw, price_to_sqrt_q64};
use crate::engine::{fees as fee_engine, liquidity};
use clmm_lp_data::swaps::SwapEvent;
use clmm_lp_domain::prelude::{Amount, Price, PriceCandle};
use clmm_lp_simulation::prelude::*;
use primitive_types::U256;
use rayon::prelude::*;
use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

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

/// How `StratConfig::Periodic(hours)` measures elapsed time since the last rebalance.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PeriodicTimeBasis {
    /// Elapsed = `steps_since_rebalance * step_seconds` (legacy; synthetic step series).
    StepsTimesStepSeconds,
    /// Elapsed = current step `start_timestamp - anchor_timestamp` (wall clock; Birdeye candles, pool snapshots).
    #[default]
    WallClockSeconds,
}

/// Optional repeat policy for [`StratConfig::RetouchShift`] while price stays out of range.
///
/// After the **first** retouch in an out-of-range episode, further retouches are allowed only when
/// wall time since the last retouch is at least [`Self::cooldown_secs`], and **either**
/// - at least [`Self::rearm_after_secs`] have passed since that retouch, **or**
/// - price has moved at least [`Self::extra_move_pct`] further in the adverse direction vs the
///   A/B price at the last retouch (`extra_move_pct` is a fraction, e.g. `0.003` = 0.3%).
///
/// When `None`, behavior is legacy: at most one retouch per continuous OOR episode until price
/// returns in range.
#[derive(Clone, Copy, Debug)]
pub struct RetouchRepeatConfig {
    pub cooldown_secs: u64,
    pub rearm_after_secs: u64,
    pub extra_move_pct: f64,
}

/// Signed extension of A/B price vs `ref_ab` in the direction of the current OOR side, as a fraction of `ref_ab`.
fn retouch_signed_extra_move_pct(
    price_ab: Decimal,
    ref_ab: Decimal,
    lower: Decimal,
    upper: Decimal,
) -> Decimal {
    let ref_d = if ref_ab > Decimal::ZERO {
        ref_ab
    } else {
        return Decimal::ZERO;
    };
    if price_ab > upper {
        if price_ab > ref_ab {
            (price_ab - ref_ab) / ref_d
        } else {
            Decimal::ZERO
        }
    } else if price_ab < lower {
        if price_ab < ref_ab {
            (ref_ab - price_ab) / ref_d
        } else {
            Decimal::ZERO
        }
    } else {
        Decimal::ZERO
    }
}

/// Strategy variant for grid search.
#[derive(Clone, Copy, Debug)]
pub enum StratConfig {
    Static,
    Threshold(f64),
    Periodic(u64),
    /// Rebalance/close when IL-like drag vs HODL exceeds thresholds.
    ILLimit {
        max_il: f64,
        close_il: Option<f64>,
        grace_steps: u64,
    },
    /// Shift only the exiting edge of the range towards current price,
    /// keeping the original range width, with "once until back in range" gating.
    ///
    /// **Economics:** On-chain this is still “remove liquidity + open a new position” with a new
    /// range (same as any rebalance): the simulator charges `tx_cost` / `rebalance_cost_model`,
    /// increments `rebalance_count`, and redeploys `position_value − cost` into the shifted range.
    RetouchShift,
    /// Symmetric band around spot using grid `width_pct` (`half = width_pct/2` multiplicative).
    ///
    /// Rebalance **only** when A/B exits `[lower, upper]`. On rebalance, opens a new position
    /// centered on the current price with the same relative width — i.e. “follow spot, ±half until
    /// OOR”, with **no** in-range recenters from mid-deviation (unlike [`StratConfig::Threshold`]).
    OorRecenter,
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
    /// Wall-clock basis matches real `start_timestamp` gaps (required for dense snapshot paths).
    pub periodic_time_basis: PeriodicTimeBasis,
    /// Hybrid time / % repeat policy for [`StratConfig::RetouchShift`]; `None` = once per OOR episode.
    pub retouch_repeat: Option<RetouchRepeatConfig>,
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
    let mut vol_model =
        ConstantVolume::from_amount(Amount::new(U256::from(1_000_000_000_000u64), 6));
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
                        (capital_dec / daily_tvl)
                            .min(Decimal::ONE)
                            .max(Decimal::ZERO)
                    });
                    let day_total = birdeye_day_total
                        .get(&date_key)
                        .copied()
                        .unwrap_or(Decimal::ZERO);
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
    let (total_vol, weighted_vol) = step_data
        .iter()
        .fold((Decimal::ZERO, Decimal::ZERO), |(tv, wv), p| {
            (tv + p.step_volume_usd, wv + p.step_volume_usd * p.lp_share)
        });
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
                final_il_segment_pct: None,
                final_il_vs_hodl_ex_fees_pct: Decimal::ZERO,
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

    // HODL benchmark: hold the *same* initial token amounts that correspond to the LP
    // position opened with `capital_dec` and the initial range at the entry price.
    //
    // This fixes cases where the real token split (value-weighted) is not 50/50 USD.
    let initial_liquidity_l = liquidity_l;
    let lower_ab_raw_for_hodl = price_ab_human_to_raw(lower_ab, token_a_decimals, token_b_decimals);
    let upper_ab_raw_for_hodl = price_ab_human_to_raw(upper_ab, token_a_decimals, token_b_decimals);
    let entry_ab_raw_for_hodl =
        price_ab_human_to_raw(first.price_ab.value, token_a_decimals, token_b_decimals);
    let sqrt_l_hodl = price_to_sqrt_q64(lower_ab_raw_for_hodl);
    let sqrt_u_hodl = price_to_sqrt_q64(upper_ab_raw_for_hodl);
    let sqrt_p_hodl = price_to_sqrt_q64(entry_ab_raw_for_hodl);
    let (hodl_a_entry_base, hodl_b_entry_base) = liquidity::amounts_from_liquidity_at_price(
        initial_liquidity_l,
        sqrt_l_hodl,
        sqrt_p_hodl,
        sqrt_u_hodl,
    );
    let mut hodl_a_entry = from_base_units(hodl_a_entry_base, token_a_decimals);
    let mut hodl_b_entry = from_base_units(hodl_b_entry_base, token_b_decimals);
    let hodl_a_initial = hodl_a_entry;
    let hodl_b_initial = hodl_b_entry;

    let mut total_fees = Decimal::ZERO;
    let mut total_rebalance_cost = Decimal::ZERO;
    let mut rebalance_count: u32 = 0;
    let mut steps_since_rebalance: u64 = 0;
    // Anchor for `PeriodicTimeBasis::WallClockSeconds` (last open / rebalance wall time).
    let mut periodic_anchor_ts: u64 = first.start_timestamp;
    let mut in_range_steps: u64 = 0;

    // equity curve for max drawdown
    let mut peak_equity = capital_dec;
    let mut max_drawdown = Decimal::ZERO;
    let is_retouch = matches!(strat, StratConfig::RetouchShift);
    let mut retouch_armed = true;
    // Hybrid retouch: wall time and ref A/B price at last retouch (`None` = no retouch this OOR episode yet).
    let mut retouch_last_ts: Option<u64> = None;
    let mut retouch_ref_ab: Option<Decimal> = None;
    let mut position_closed = false;
    let mut closed_cash_value_usd = Decimal::ZERO;

    let strat_name = match strat {
        StratConfig::Static => "static".to_string(),
        StratConfig::Threshold(p) => format!("threshold_{:.0}%", p * 100.0),
        StratConfig::Periodic(h) => format!("periodic_{}h", h),
        StratConfig::ILLimit {
            max_il,
            close_il,
            grace_steps,
        } => match close_il {
            Some(c) => format!(
                "il_limit_{:.0}%_close_{:.0}%_grace_{}",
                max_il * 100.0,
                c * 100.0,
                grace_steps
            ),
            None => format!("il_limit_{:.0}%_grace_{}", max_il * 100.0, grace_steps),
        },
        StratConfig::RetouchShift => "retouch_shift".to_string(),
        StratConfig::OorRecenter => "oor_recenter".to_string(),
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
        if position_closed {
            let equity = closed_cash_value_usd + total_fees;
            if equity > peak_equity {
                peak_equity = equity;
            }
            if peak_equity > Decimal::ZERO {
                let dd = (peak_equity - equity) / peak_equity;
                if dd > max_drawdown {
                    max_drawdown = dd;
                }
            }
            continue;
        }
        let price_ab = p.price_ab.value;
        let in_range = price_ab >= current_lower_ab && price_ab <= current_upper_ab;
        if in_range {
            in_range_steps += 1;
            if is_retouch {
                if params.retouch_repeat.is_some() {
                    // New in-range episode: next OOR may do an immediate first retouch again.
                    retouch_last_ts = None;
                    retouch_ref_ab = None;
                } else {
                    // Legacy: re-arm single retouch after price has returned inside the range.
                    retouch_armed = true;
                }
            }
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
        let lower_ab_raw =
            price_ab_human_to_raw(current_lower_ab, token_a_decimals, token_b_decimals);
        let upper_ab_raw =
            price_ab_human_to_raw(current_upper_ab, token_a_decimals, token_b_decimals);
        let price_ab_raw = price_ab_human_to_raw(price_ab, token_a_decimals, token_b_decimals);

        let sqrt_l = crate::engine::pricing::price_to_sqrt_q64(lower_ab_raw);
        let sqrt_u = crate::engine::pricing::price_to_sqrt_q64(upper_ab_raw);
        let sqrt_p = crate::engine::pricing::price_to_sqrt_q64(price_ab_raw);
        let (amt_a_base, amt_b_base) =
            liquidity::amounts_from_liquidity_at_price(liquidity_l, sqrt_l, sqrt_p, sqrt_u);
        let amt_a = crate::engine::pricing::from_base_units(amt_a_base, token_a_decimals);
        let amt_b = crate::engine::pricing::from_base_units(amt_b_base, token_b_decimals);
        let position_value_usd = (amt_a * p.price_usd.value) + (amt_b * p.quote_usd);
        let hodl_value_step =
            (hodl_a_entry * p.price_usd.value) + (hodl_b_entry * p.quote_usd.max(Decimal::ZERO));
        let il_like_step = if capital_dec > Decimal::ZERO {
            (position_value_usd - hodl_value_step) / capital_dec
        } else {
            Decimal::ZERO
        };

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
                let interval_sec = interval_hours.saturating_mul(3600);
                match params.periodic_time_basis {
                    PeriodicTimeBasis::WallClockSeconds => {
                        p.start_timestamp.saturating_sub(periodic_anchor_ts) >= interval_sec
                    }
                    PeriodicTimeBasis::StepsTimesStepSeconds => {
                        let elapsed = (steps_since_rebalance as i64) * params.step_seconds.max(1);
                        elapsed as u64 >= interval_sec
                    }
                }
            }
            StratConfig::ILLimit {
                max_il,
                close_il: _,
                grace_steps,
            } => {
                let step_idx = (i as u64) + 1;
                if step_idx <= grace_steps {
                    false
                } else if !in_range {
                    true
                } else {
                    il_like_step.abs() >= Decimal::from_f64(max_il).unwrap_or(Decimal::ZERO).abs()
                }
            }
            StratConfig::OorRecenter => !in_range,
            StratConfig::RetouchShift => {
                if in_range {
                    false
                } else if let Some(cfg) = params.retouch_repeat {
                    // First OOR step of an episode: shift immediately. Later: cooldown, then time OR %.
                    match retouch_last_ts {
                        None => true,
                        Some(last_ts) => {
                            let dt = p.start_timestamp.saturating_sub(last_ts);
                            if dt < cfg.cooldown_secs {
                                false
                            } else if dt >= cfg.rearm_after_secs {
                                true
                            } else {
                                let ref_ab = retouch_ref_ab.unwrap_or(price_ab);
                                let pct_move = retouch_signed_extra_move_pct(
                                    price_ab,
                                    ref_ab,
                                    current_lower_ab,
                                    current_upper_ab,
                                );
                                let threshold =
                                    Decimal::from_f64(cfg.extra_move_pct).unwrap_or(Decimal::ZERO);
                                pct_move >= threshold
                            }
                        }
                    }
                } else {
                    !in_range && retouch_armed
                }
            }
        };
        let should_close = match strat {
            StratConfig::ILLimit {
                max_il: _,
                close_il: Some(close_il),
                grace_steps,
            } => {
                let step_idx = (i as u64) + 1;
                step_idx > grace_steps
                    && il_like_step.abs()
                        >= Decimal::from_f64(close_il).unwrap_or(Decimal::ZERO).abs()
            }
            _ => false,
        };

        if should_close && liquidity_l > 0 {
            let close_cost = rebalance_cost_model.map_or(tx_cost_dec, |model| {
                model.cost_for_notional(position_value_usd.max(Decimal::ZERO))
            });
            total_rebalance_cost += close_cost;
            rebalance_count += 1;
            steps_since_rebalance = 0;
            periodic_anchor_ts = p.start_timestamp;
            closed_cash_value_usd = (position_value_usd - close_cost).max(Decimal::ZERO);
            liquidity_l = 0;
            position_closed = true;
        } else if should_rebalance && liquidity_l > 0 {
            // Includes `RetouchShift`: same accounting as other rebalances (cost, `rebalance_count`,
            // redeploy NAV minus cost into the new range); only the *geometry* of the new bounds differs.
            //
            // Benchmark (HODL) is updated at rebalance time as well:
            // we "rebalance" the benchmark holdings to match the LP token composition
            // for the new range, without paying tx costs.
            let benchmark_capital_now_usd = (hodl_a_entry * p.price_usd.value)
                + (hodl_b_entry * p.quote_usd.max(Decimal::ZERO));

            // Estimate slippage not from whole position value,
            // but from the delta in token amounts implied by relocating the range.
            // This is a more realistic proxy for "how much you have to swap" at rebalance.
            let (new_lower_ab, new_upper_ab) = if is_retouch {
                if price_ab > current_upper_ab {
                    let overflow = price_ab - current_upper_ab;
                    (current_lower_ab + overflow, price_ab)
                } else {
                    // price_ab < current_lower_ab (out-of-range lower side)
                    let overflow = current_lower_ab - price_ab;
                    (price_ab, current_upper_ab - overflow)
                }
            } else {
                let center_ab_now = price_ab.to_f64().unwrap_or(1.0);
                (
                    Decimal::from_f64(center_ab_now * (1.0 - half)).unwrap(),
                    Decimal::from_f64(center_ab_now * (1.0 + half)).unwrap(),
                )
            };

            let rebalance_cost = if let Some(model) = rebalance_cost_model {
                let capital_usd_for_delta = position_value_usd.max(Decimal::ZERO);

                // Target token split for the new range at the rebalance price.
                let new_lower_usd_for_delta = new_lower_ab * p.quote_usd;
                let new_upper_usd_for_delta = new_upper_ab * p.quote_usd;

                let liquidity_l_for_delta = liquidity::estimate_position_liquidity_with_overrides(
                    step_data,
                    new_lower_usd_for_delta,
                    new_upper_usd_for_delta,
                    capital_usd_for_delta,
                    token_a_decimals,
                    token_b_decimals,
                    liquidity::LiquidityEstimateOverrides {
                        quote_usd: Some(p.quote_usd),
                        price_ab: Some(price_ab),
                        price_a_usd: Some(p.price_usd.value),
                    },
                );

                let lower_ab_raw_for_delta =
                    price_ab_human_to_raw(new_lower_ab, token_a_decimals, token_b_decimals);
                let upper_ab_raw_for_delta =
                    price_ab_human_to_raw(new_upper_ab, token_a_decimals, token_b_decimals);

                let sqrt_l_for_delta = price_to_sqrt_q64(lower_ab_raw_for_delta);
                let sqrt_u_for_delta = price_to_sqrt_q64(upper_ab_raw_for_delta);

                let (tgt_a_base, tgt_b_base) = liquidity::amounts_from_liquidity_at_price(
                    liquidity_l_for_delta,
                    sqrt_l_for_delta,
                    sqrt_p,
                    sqrt_u_for_delta,
                );
                let tgt_a = from_base_units(tgt_a_base, token_a_decimals);
                let tgt_b = from_base_units(tgt_b_base, token_b_decimals);

                // Approx proxy notional: the larger USD-side of what must change.
                let notional_a_usd = (tgt_a - amt_a).abs() * p.price_usd.value;
                let notional_b_usd = (tgt_b - amt_b).abs() * p.quote_usd.max(Decimal::ZERO);
                let delta_notional_usd = notional_a_usd.max(notional_b_usd);

                // Retouch = withdraw entire position and mint a new one with shifted ticks. Slippage
                // should scale with full position notional, not only the ideal token-delta swap size.
                let notional_for_cost = if is_retouch {
                    position_value_usd.max(Decimal::ZERO)
                } else if delta_notional_usd > Decimal::ZERO {
                    delta_notional_usd
                } else {
                    position_value_usd.max(Decimal::ZERO)
                };

                model.cost_for_notional(notional_for_cost)
            } else {
                tx_cost_dec
            };

            total_rebalance_cost += rebalance_cost;
            rebalance_count += 1;
            steps_since_rebalance = 0;
            periodic_anchor_ts = p.start_timestamp;

            // Re-deploy current position value minus rebalance cost; fees are NOT compounded here.
            let capital_usd_now = (position_value_usd - rebalance_cost).max(Decimal::ZERO);
            current_lower_ab = new_lower_ab;
            current_upper_ab = new_upper_ab;
            if is_retouch {
                if params.retouch_repeat.is_some() {
                    retouch_last_ts = Some(p.start_timestamp);
                    retouch_ref_ab = Some(price_ab);
                } else {
                    // Legacy: at most one retouch until back in range.
                    retouch_armed = false;
                }
            }

            // Convert AB bounds to USD using current quote USD for liquidity estimation.
            let new_lower_usd = current_lower_ab * p.quote_usd;
            let new_upper_usd = current_upper_ab * p.quote_usd;
            liquidity_l = liquidity::estimate_position_liquidity_with_overrides(
                step_data,
                new_lower_usd,
                new_upper_usd,
                capital_usd_now,
                token_a_decimals,
                token_b_decimals,
                liquidity::LiquidityEstimateOverrides {
                    quote_usd: Some(p.quote_usd),
                    price_ab: Some(price_ab),
                    price_a_usd: Some(p.price_usd.value),
                },
            );

            // Update benchmark token amounts to the new segment start.
            // Token amounts scale linearly with capital for a fixed range and price,
            // so we derive LP's token amounts after rebalance and scale them up to
            // match `benchmark_capital_now_usd` (i.e. ignore tx costs for the benchmark).
            if capital_usd_now > Decimal::ZERO && liquidity_l > 0 {
                let lower_ab_raw_for_bench =
                    price_ab_human_to_raw(current_lower_ab, token_a_decimals, token_b_decimals);
                let upper_ab_raw_for_bench =
                    price_ab_human_to_raw(current_upper_ab, token_a_decimals, token_b_decimals);
                let price_ab_raw_for_bench =
                    price_ab_human_to_raw(price_ab, token_a_decimals, token_b_decimals);

                let sqrt_l_for_bench = price_to_sqrt_q64(lower_ab_raw_for_bench);
                let sqrt_u_for_bench = price_to_sqrt_q64(upper_ab_raw_for_bench);
                let sqrt_p_for_bench = price_to_sqrt_q64(price_ab_raw_for_bench);

                let (amt_a_base_bench, amt_b_base_bench) =
                    liquidity::amounts_from_liquidity_at_price(
                        liquidity_l,
                        sqrt_l_for_bench,
                        sqrt_p_for_bench,
                        sqrt_u_for_bench,
                    );
                let lp_a = from_base_units(amt_a_base_bench, token_a_decimals);
                let lp_b = from_base_units(amt_b_base_bench, token_b_decimals);

                // If L or amounts degenerate to zero, keep the previous HODL benchmark instead of
                // zeroing it (avoids absurd `hodl_value == 0` rows in optimize tables).
                if !lp_a.is_zero() || !lp_b.is_zero() {
                    let scale = benchmark_capital_now_usd / capital_usd_now;
                    hodl_a_entry = lp_a * scale;
                    hodl_b_entry = lp_b * scale;
                }
            }

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
    let position_value_usd = if position_closed {
        closed_cash_value_usd
    } else {
        let lower_ab_raw =
            price_ab_human_to_raw(current_lower_ab, token_a_decimals, token_b_decimals);
        let upper_ab_raw =
            price_ab_human_to_raw(current_upper_ab, token_a_decimals, token_b_decimals);
        let last_ab_raw =
            price_ab_human_to_raw(last.price_ab.value, token_a_decimals, token_b_decimals);

        let sqrt_l = crate::engine::pricing::price_to_sqrt_q64(lower_ab_raw);
        let sqrt_u = crate::engine::pricing::price_to_sqrt_q64(upper_ab_raw);
        let sqrt_p = crate::engine::pricing::price_to_sqrt_q64(last_ab_raw);
        let (amt_a_base, amt_b_base) =
            liquidity::amounts_from_liquidity_at_price(liquidity_l, sqrt_l, sqrt_p, sqrt_u);
        let amt_a = crate::engine::pricing::from_base_units(amt_a_base, token_a_decimals);
        let amt_b = crate::engine::pricing::from_base_units(amt_b_base, token_b_decimals);
        (amt_a * last.price_usd.value) + (amt_b * last.quote_usd)
    };

    let final_value = position_value_usd + total_fees;
    let final_pnl = final_value - capital_dec;
    let hodl_value = {
        let v = (hodl_a_entry * last.price_usd.value)
            + (hodl_b_entry * last.quote_usd.max(Decimal::ZERO));
        if hodl_a_entry.is_zero()
            && hodl_b_entry.is_zero()
            && (!hodl_a_initial.is_zero() || !hodl_b_initial.is_zero())
        {
            (hodl_a_initial * last.price_usd.value)
                + (hodl_b_initial * last.quote_usd.max(Decimal::ZERO))
        } else {
            v
        }
    };
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
        final_il_segment_pct: None,
        final_il_vs_hodl_ex_fees_pct: il_like_pct,
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

fn pct_token_to_ratio(s: &str) -> Option<f64> {
    s.strip_suffix('%')?.parse::<f64>().ok().map(|x| x / 100.0)
}

fn parse_il_limit_label(label: &str) -> Option<StratConfig> {
    let rest = label.strip_prefix("il_limit_")?;
    let (mid, grace_s) = rest.split_once("_grace_")?;
    let grace_steps: u64 = grace_s.parse().ok()?;
    if let Some((max_s, close_s)) = mid.split_once("_close_") {
        let max_il = pct_token_to_ratio(max_s)?;
        let close_il = pct_token_to_ratio(close_s)?;
        Some(StratConfig::ILLimit {
            max_il,
            close_il: Some(close_il),
            grace_steps,
        })
    } else {
        let max_il = pct_token_to_ratio(mid)?;
        Some(StratConfig::ILLimit {
            max_il,
            close_il: None,
            grace_steps,
        })
    }
}

/// Parse grid strategy name (from `run_single`) back to [`StratConfig`].
pub fn parse_strategy_label(label: &str) -> Option<StratConfig> {
    match label {
        "static" => return Some(StratConfig::Static),
        "retouch_shift" => return Some(StratConfig::RetouchShift),
        "oor_recenter" => return Some(StratConfig::OorRecenter),
        _ => {}
    }
    if let Some(h) = label
        .strip_prefix("periodic_")
        .and_then(|s| s.strip_suffix('h'))
    {
        return h.parse::<u64>().ok().map(StratConfig::Periodic);
    }
    if let Some(p) = label
        .strip_prefix("threshold_")
        .and_then(|s| s.strip_suffix('%'))
    {
        return p
            .parse::<f64>()
            .ok()
            .map(|pct| StratConfig::Threshold(pct / 100.0));
    }
    if label.starts_with("il_limit_") {
        return parse_il_limit_label(label);
    }
    None
}

#[cfg(test)]
mod strat_label_tests {
    use super::{StratConfig, parse_strategy_label};

    #[test]
    fn parse_periodic_threshold_il() {
        assert!(matches!(
            parse_strategy_label("periodic_24h"),
            Some(StratConfig::Periodic(24))
        ));
        match parse_strategy_label("threshold_5%") {
            Some(StratConfig::Threshold(p)) => assert!((p - 0.05).abs() < 1e-9),
            other => panic!("unexpected {other:?}"),
        }
        match parse_strategy_label("il_limit_5%_grace_0").unwrap() {
            StratConfig::ILLimit {
                max_il,
                close_il: None,
                grace_steps: 0,
            } => assert!((max_il - 0.05).abs() < 1e-9),
            other => panic!("unexpected {other:?}"),
        }
    }
}
