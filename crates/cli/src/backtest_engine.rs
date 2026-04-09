//! Shared backtest logic: step data building and grid execution.
//!
//! Used by both `backtest` (single run) and `backtest-optimize` (grid + rolling windows).

use crate::engine::indicators;
use crate::engine::{fees as fee_engine, hodl, liquidity};
use clmm_lp_data::swaps::SwapEvent;
use clmm_lp_domain::prelude::{Amount, Price, PriceCandle};
use clmm_lp_protocols::prelude::price_to_tick;
use clmm_lp_simulation::prelude::*;
use primitive_types::U256;
use rayon::prelude::*;
use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

/// Match [`liquidity::estimate_position_liquidity`]: CLMM sqrt math uses **raw** A/B (token atoms),
/// not UI `price_ab`. Calling `price_to_sqrt_q64` on human prices breaks when `dec_a != dec_b`
/// (e.g. SOL 9 / USDC 6) → nonsense token amounts and huge bogus PnL.
#[inline]
fn sqrt_q64_from_price_ab_human(
    price_ab_human: Decimal,
    token_a_decimals: u32,
    token_b_decimals: u32,
) -> u128 {
    let raw = crate::engine::pricing::price_ab_human_to_raw(
        price_ab_human,
        token_a_decimals,
        token_b_decimals,
    );
    crate::engine::pricing::price_to_sqrt_q64(raw)
}

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
    /// Pool active liquidity (CLMM on-chain units), available for snapshot-derived steps.
    /// Used for dynamic LiquidityShare fee attribution.
    pub liquidity_active_raw: Option<u128>,
    /// Current tick index at this step (Orca/Raydium snapshot-derived).
    /// When set, can be used for tick-aligned in-range accounting.
    pub tick_current: Option<i32>,
    /// Candle start timestamp (seconds).
    pub start_timestamp: u64,
}

pub type StepData = StepDataPoint;

/// Strategy variant for grid search.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StratConfig {
    Static,
    Threshold(f64),
    /// Steps between rebalances (label uses `h` suffix historically; value is **step count**).
    Periodic(u64),
    /// Bollinger: last `window` closes (A/B), bands `SMA ± k·σ`; rebalance every `rebalance_steps` steps.
    Bollinger {
        window: u32,
        k: f64,
        rebalance_steps: u64,
    },
    /// Anchor on close of last completed candle of `candle_steps` steps; rebalance every `rebalance_steps` steps.
    LastCandle {
        candle_steps: u64,
        rebalance_steps: u64,
    },
    /// Snapshot-friendly: candle and rebalance defined in wall-clock seconds.
    /// Candle bounds use the last fully closed time bucket `[t-candle_seconds, t)`.
    LastCandleTime {
        candle_seconds: u64,
        rebalance_seconds: u64,
    },
}

/// Build step data (price, volume, share) for each candle.
///
/// **Volume:** When Dune TVL/volume is present we use **hybrid** volume:
/// - Per-candle USD volume from Birdeye (`volume_token_a * close`) gives the **intraday distribution**
///   (high volume hours get more volume; often those are volatile hours when price may be out of range).
/// - Dune daily volume for the pool gives the **scale** so the day total matches the pool.
/// - So: `step_vol_usd = dune_daily_vol * (candle_vol_usd / birdeye_day_total)`.
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
    for (candle, &ref vol) in candle_slice.iter().zip(candle_vol_usd.iter()) {
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
                liquidity_active_raw: None,
                tick_current: None,
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
    let mut map: BTreeMap<usize, Vec<&'a SwapEvent>> = BTreeMap::new();
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
/// If `swaps` is provided, fees are computed from Dune swap data (step_swap_fees_usd) instead of candle volume.
/// If `snapshot_pool_fees_usd` is provided (and `swaps` is None), per-step **pool** fees in USD come from the map
/// (key = step index, same convention as `snapshot_price_path` / `fee_growth` deltas). LP share is still applied
/// via [`FeeShareModel`]. When both are absent, fees default to `step_volume_usd * fee_rate`.
pub fn run_single(
    step_data: &[StepData],
    entry_price: Price,
    center: f64,
    width_pct: f64,
    strat: StratConfig,
    capital_dec: Decimal,
    tx_cost_dec: Decimal,
    fee_rate: Decimal,
    pool_active_liquidity: Option<u128>,
    token_a_decimals: u32,
    token_b_decimals: u32,
    swaps: Option<&[SwapEvent]>,
    snapshot_pool_fees_usd: Option<&BTreeMap<usize, Decimal>>,
) -> (f64, f64, String, TrackerSummary) {
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

    let mut total_fees = Decimal::ZERO;
    let mut total_rebalance_cost = Decimal::ZERO;
    let mut rebalance_count: u32 = 0;
    let mut steps_since_rebalance: u64 = 0;
    let mut in_range_steps: u64 = 0;
    let mut secs_since_rebalance: u64 = 0;
    let mut prev_ts: Option<u64> = None;

    // equity curve for max drawdown
    let mut peak_equity = capital_dec;
    let mut max_drawdown = Decimal::ZERO;
    let strat_name = match strat {
        StratConfig::Static => "static".to_string(),
        StratConfig::Threshold(p) => format!("threshold_{:.0}%", p * 100.0),
        StratConfig::Periodic(h) => format!("periodic_{}h", h),
        StratConfig::Bollinger {
            window,
            k,
            rebalance_steps,
        } => format!("bollinger_w{}_k{}_r{}", window, k, rebalance_steps),
        StratConfig::LastCandle {
            candle_steps,
            rebalance_steps,
        } => format!("last_candle_c{}_r{}", candle_steps, rebalance_steps),
        StratConfig::LastCandleTime {
            candle_seconds,
            rebalance_seconds,
        } => format!("last_candle_t{}_r{}", candle_seconds, rebalance_seconds),
    };

    let mut fee_share_model = if let Some(pool_l) = pool_active_liquidity.filter(|v| *v > 0) {
        fee_engine::FeeShareModel::LiquidityShare {
            position_liquidity: liquidity_l,
            pool_active_liquidity: pool_l,
        }
    } else {
        fee_engine::FeeShareModel::LegacyLpShare
    };

    let swap_index = swaps.map(|s| index_swaps_by_step(s, step_data, 3600));

    // Debug aid: print fee-share mechanics for the first N in-range steps.
    // Controlled via env var:
    //   CLMM_DEBUG_STEP_LIQ_SHARE=20
    let mut debug_left: u32 = std::env::var("CLMM_DEBUG_STEP_LIQ_SHARE")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);
    let debug_enabled = debug_left > 0 && snapshot_pool_fees_usd.is_some();

    // Tick-aligned in-range: use `tick_current` (when available) instead of float price bounds.
    // Semantics follow Orca: lower tick inclusive, upper tick exclusive.
    // Enabled via:
    //   CLMM_IN_RANGE_TICK=1
    let use_tick_in_range = std::env::var("CLMM_IN_RANGE_TICK")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0)
        > 0;

    for (i, p) in step_data.iter().enumerate() {
        if let Some(prev) = prev_ts {
            secs_since_rebalance = secs_since_rebalance
                .saturating_add(p.start_timestamp.saturating_sub(prev));
        }
        prev_ts = Some(p.start_timestamp);
        steps_since_rebalance += 1;
        let price_ab = p.price_ab.value;
        let in_range_float = price_ab >= current_lower_ab && price_ab <= current_upper_ab;
        let mut in_range = in_range_float;
        if use_tick_in_range {
            if let Some(tick_current) = p.tick_current {
                let tick_lower = price_to_tick(current_lower_ab.max(Decimal::ZERO));
                let tick_upper = price_to_tick(current_upper_ab.max(Decimal::ZERO));
                in_range = tick_current >= tick_lower && tick_current < tick_upper;
            }
        }
        if in_range {
            in_range_steps += 1;
        }

        let pool_fees = if let Some(ref idx) = swap_index {
            idx.get(&i)
                .map(|swaps_here: &Vec<&SwapEvent>| {
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
        } else if let Some(snap) = snapshot_pool_fees_usd {
            snap.get(&i).copied().unwrap_or(Decimal::ZERO)
        } else {
            p.step_volume_usd * fee_rate
        };

        let step_fee_share = if in_range {
            if snapshot_pool_fees_usd.is_some() {
                if let Some(liq_active_raw) = p.liquidity_active_raw {
                    if liq_active_raw > 0 {
                        // Dynamic LiquidityShare for snapshot-fees:
                        // share = position_liquidity / pool_active_liquidity_at_step
                        let pos_l_dec = Decimal::from_u128(liquidity_l).unwrap_or(Decimal::ZERO);
                        let liq_dec = Decimal::from_u128(liq_active_raw).unwrap_or(Decimal::ONE);
                        if liq_dec > Decimal::ZERO {
                            (pos_l_dec / liq_dec).min(Decimal::ONE)
                        } else {
                            fee_share_model.step_fee_share(p)
                        }
                    } else {
                        fee_share_model.step_fee_share(p)
                    }
                } else {
                    fee_share_model.step_fee_share(p)
                }
            } else {
                fee_share_model.step_fee_share(p)
            }
        } else {
            Decimal::ZERO
        };
        let step_fees: Decimal = if in_range {
            pool_fees * step_fee_share
        } else {
            Decimal::ZERO
        };

        if debug_enabled && in_range && debug_left > 0 {
            println!(
                "DEBUG step_fee_share i={} ts={} price_ab={} in_range={} pool_fees_usd={} fee_model_kind={} lp_share={} step_fee_share={}",
                i,
                p.start_timestamp,
                price_ab,
                in_range,
                pool_fees,
                fee_share_model.kind(),
                p.lp_share,
                step_fee_share
            );
            debug_left -= 1;
        }
        total_fees += step_fees;

        // Current position valuation (excluding fees)
        let sqrt_l =
            sqrt_q64_from_price_ab_human(current_lower_ab, token_a_decimals, token_b_decimals);
        let sqrt_u =
            sqrt_q64_from_price_ab_human(current_upper_ab, token_a_decimals, token_b_decimals);
        let sqrt_p = sqrt_q64_from_price_ab_human(price_ab, token_a_decimals, token_b_decimals);
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
            StratConfig::Periodic(interval) => steps_since_rebalance >= interval,
            StratConfig::Bollinger {
                window,
                rebalance_steps,
                ..
            } => {
                let w = window as usize;
                steps_since_rebalance >= rebalance_steps && (i + 1) >= w
            }
            StratConfig::LastCandle {
                rebalance_steps, ..
            } => steps_since_rebalance >= rebalance_steps,
            StratConfig::LastCandleTime {
                rebalance_seconds,
                ..
            } => secs_since_rebalance >= rebalance_seconds,
        };

        if should_rebalance && liquidity_l > 0 {
            total_rebalance_cost += tx_cost_dec;
            rebalance_count += 1;
            steps_since_rebalance = 0;
            secs_since_rebalance = 0;

            // Re-deploy current position value minus tx cost; fees are NOT compounded here.
            let capital_usd_now = (position_value_usd - tx_cost_dec).max(Decimal::ZERO);
            let (new_lower_ab, new_upper_ab) = match strat {
                StratConfig::Bollinger { window, k, .. } => {
                    let w = window as usize;
                    let start = i + 1 - w;
                    let closes: Vec<f64> = step_data[start..=i]
                        .iter()
                        .filter_map(|p| p.price_ab.value.to_f64())
                        .collect();
                    if let Some((lo, hi)) = indicators::bollinger_lower_upper(&closes, k) {
                        (
                            Decimal::from_f64(lo).unwrap_or(current_lower_ab),
                            Decimal::from_f64(hi).unwrap_or(current_upper_ab),
                        )
                    } else {
                        let sma = closes.iter().sum::<f64>() / closes.len().max(1) as f64;
                        (
                            Decimal::from_f64(sma * (1.0 - half)).unwrap(),
                            Decimal::from_f64(sma * (1.0 + half)).unwrap(),
                        )
                    }
                }
                StratConfig::LastCandle { candle_steps, .. } => {
                    // LP range = [min(price_ab), max(price_ab)] over the last **completed** candle (OHLC-style band on the path).
                    // If the candle is flat (min == max) or no candle closed yet, fall back to ±width_pct around close/current.
                    let cs = candle_steps as usize;
                    if let Some((start, end)) = indicators::last_closed_candle_step_range(i, cs) {
                        let slice = &step_data[start..=end];
                        let mut lo_ab = slice[0].price_ab.value;
                        let mut hi_ab = lo_ab;
                        for p in &slice[1..] {
                            let v = p.price_ab.value;
                            lo_ab = lo_ab.min(v);
                            hi_ab = hi_ab.max(v);
                        }
                        if lo_ab > Decimal::ZERO && lo_ab < hi_ab {
                            (lo_ab, hi_ab)
                        } else {
                            let anchor = if lo_ab > Decimal::ZERO {
                                lo_ab
                            } else {
                                step_data[indicators::last_closed_candle_close_idx(i, cs)]
                                    .price_ab
                                    .value
                            };
                            let c = anchor.to_f64().unwrap_or(1.0);
                            (
                                Decimal::from_f64(c * (1.0 - half)).unwrap(),
                                Decimal::from_f64(c * (1.0 + half)).unwrap(),
                            )
                        }
                    } else {
                        let anchor = step_data[0].price_ab.value;
                        let c = anchor.to_f64().unwrap_or(1.0);
                        (
                            Decimal::from_f64(c * (1.0 - half)).unwrap(),
                            Decimal::from_f64(c * (1.0 + half)).unwrap(),
                        )
                    }
                }
                StratConfig::LastCandleTime {
                    candle_seconds, ..
                } => {
                    // Snapshot-friendly: use last fully closed wall-clock bucket `[t-candle_seconds, t)`.
                    if let Some((start, end)) = indicators::last_closed_time_bucket_step_range(
                        step_data,
                        i,
                        candle_seconds,
                    ) {
                        let slice = &step_data[start..=end];
                        let mut lo_ab = slice[0].price_ab.value;
                        let mut hi_ab = lo_ab;
                        for p in &slice[1..] {
                            let v = p.price_ab.value;
                            lo_ab = lo_ab.min(v);
                            hi_ab = hi_ab.max(v);
                        }
                        if lo_ab > Decimal::ZERO && lo_ab < hi_ab {
                            (lo_ab, hi_ab)
                        } else {
                            let anchor = slice.last().map(|p| p.price_ab.value).unwrap_or(price_ab);
                            let c = anchor.to_f64().unwrap_or(1.0);
                            (
                                Decimal::from_f64(c * (1.0 - half)).unwrap(),
                                Decimal::from_f64(c * (1.0 + half)).unwrap(),
                            )
                        }
                    } else {
                        let anchor = step_data[0].price_ab.value;
                        let c = anchor.to_f64().unwrap_or(1.0);
                        (
                            Decimal::from_f64(c * (1.0 - half)).unwrap(),
                            Decimal::from_f64(c * (1.0 + half)).unwrap(),
                        )
                    }
                }
                _ => {
                    let center_ab_now = price_ab.to_f64().unwrap_or(1.0);
                    (
                        Decimal::from_f64(center_ab_now * (1.0 - half)).unwrap(),
                        Decimal::from_f64(center_ab_now * (1.0 + half)).unwrap(),
                    )
                }
            };
            current_lower_ab = new_lower_ab;
            current_upper_ab = new_upper_ab;

            // Convert AB bounds to USD using current quote USD for liquidity estimation.
            let new_lower_usd = current_lower_ab * p.quote_usd;
            let new_upper_usd = current_upper_ab * p.quote_usd;
            // `estimate_position_liquidity` defaults to **entry** `quote_usd` / `price_ab` for
            // converting USD bounds → A/B and for normalizing L. After rebalance, bounds were built
            // with **this step's** quote — must pass overrides or L and fee-share vs pool blow up when
            // B/USD drifts (cross-pairs).
            liquidity_l = liquidity::estimate_position_liquidity_with_overrides(
                step_data,
                new_lower_usd,
                new_upper_usd,
                capital_usd_now,
                token_a_decimals,
                token_b_decimals,
                liquidity::LiquidityEstimateOverrides {
                    quote_usd: Some(p.quote_usd),
                    price_ab: Some(p.price_ab.value),
                    price_a_usd: Some(p.price_usd.value),
                },
            );

            if let Some(pool_l) = pool_active_liquidity.filter(|v| *v > 0) {
                fee_share_model = fee_engine::FeeShareModel::LiquidityShare {
                    position_liquidity: liquidity_l,
                    pool_active_liquidity: pool_l,
                };
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
    let sqrt_l = sqrt_q64_from_price_ab_human(current_lower_ab, token_a_decimals, token_b_decimals);
    let sqrt_u = sqrt_q64_from_price_ab_human(current_upper_ab, token_a_decimals, token_b_decimals);
    let sqrt_p =
        sqrt_q64_from_price_ab_human(last.price_ab.value, token_a_decimals, token_b_decimals);
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
/// If `swaps` is provided, fees are computed from Dune swap data per step.
/// If `snapshot_pool_fees_usd` is provided (and swaps are not), pool fees per step follow the snapshot index.
pub fn run_grid(
    step_data: &[StepData],
    entry_price: Price,
    center: f64,
    width_pcts: &[f64],
    strategies: &[StratConfig],
    capital_dec: Decimal,
    tx_cost_dec: Decimal,
    fee_rate: Decimal,
    pool_active_liquidity: Option<u128>,
    token_a_decimals: u32,
    token_b_decimals: u32,
    swaps: Option<&[SwapEvent]>,
    snapshot_pool_fees_usd: Option<BTreeMap<usize, Decimal>>,
) -> Vec<(f64, f64, f64, String, TrackerSummary)> {
    let step_data = Arc::new(step_data.to_vec());
    let swaps_arc: Option<Arc<Vec<SwapEvent>>> = swaps.map(|s| Arc::new(s.to_vec()));
    let snap_arc: Option<Arc<BTreeMap<usize, Decimal>>> = snapshot_pool_fees_usd.map(Arc::new);
    let jobs: Vec<(f64, StratConfig)> = width_pcts
        .iter()
        .flat_map(|&wp| strategies.iter().copied().map(move |s| (wp, s)))
        .collect();
    jobs.par_iter()
        .map(|(wp, strat)| {
            let swaps_ref = swaps_arc.as_deref();
            let snap_ref = snap_arc.as_deref();
            let (lower, upper, strat_name, summary) = run_single(
                step_data.as_ref(),
                entry_price,
                center,
                *wp,
                *strat,
                capital_dec,
                tx_cost_dec,
                fee_rate,
                pool_active_liquidity,
                token_a_decimals,
                token_b_decimals,
                swaps_ref.map(|v| &**v),
                snap_ref,
            );
            (*wp, lower, upper, strat_name, summary)
        })
        .collect()
}

/// Parse simulator strategy label (`run_single` output) back to [`StratConfig`] for JSON export.
#[must_use]
pub fn parse_strategy_label(name: &str) -> Option<StratConfig> {
    let name = name.trim();
    if name == "static" {
        return Some(StratConfig::Static);
    }
    if let Some(rest) = name.strip_prefix("threshold_") {
        let pct_str = rest.trim_end_matches('%').trim();
        let pct = pct_str.parse::<f64>().ok()?;
        return Some(StratConfig::Threshold(pct / 100.0));
    }
    if let Some(rest) = name.strip_prefix("periodic_") {
        let num_str = rest.trim_end_matches('h').trim();
        let steps = num_str.parse::<u64>().ok()?;
        return Some(StratConfig::Periodic(steps));
    }
    if let Some(rest) = name.strip_prefix("bollinger_") {
        let mut window: Option<u32> = None;
        let mut k: Option<f64> = None;
        let mut rebalance_steps: Option<u64> = None;
        for part in rest.split('_') {
            if let Some(n) = part.strip_prefix('w') {
                window = n.parse().ok();
            } else if let Some(n) = part.strip_prefix('k') {
                k = n.parse().ok();
            } else if let Some(n) = part.strip_prefix('r') {
                rebalance_steps = n.parse().ok();
            }
        }
        return Some(StratConfig::Bollinger {
            window: window?,
            k: k?,
            rebalance_steps: rebalance_steps?,
        });
    }
    if let Some(rest) = name.strip_prefix("last_candle_") {
        if let Some(rest) = rest.strip_prefix('t') {
            // time-based: last_candle_t{candle_seconds}_r{rebalance_seconds}
            let mut candle_seconds: Option<u64> = None;
            let mut rebalance_seconds: Option<u64> = None;
            for part in rest.split('_') {
                if candle_seconds.is_none() {
                    candle_seconds = part.parse().ok();
                } else if let Some(n) = part.strip_prefix('r') {
                    rebalance_seconds = n.parse().ok();
                }
            }
            return Some(StratConfig::LastCandleTime {
                candle_seconds: candle_seconds?,
                rebalance_seconds: rebalance_seconds?,
            });
        }
        let mut candle_steps: Option<u64> = None;
        let mut rebalance_steps: Option<u64> = None;
        for part in rest.split('_') {
            if let Some(n) = part.strip_prefix('c') {
                candle_steps = n.parse().ok();
            } else if let Some(n) = part.strip_prefix('r') {
                rebalance_steps = n.parse().ok();
            }
        }
        return Some(StratConfig::LastCandle {
            candle_steps: candle_steps?,
            rebalance_steps: rebalance_steps?,
        });
    }
    None
}
