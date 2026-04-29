//! Decision engine for strategy execution.

use super::Decision;
use crate::monitor::MonitoredPosition;
use clmm_lp_protocols::prelude::WhirlpoolState;
use rust_decimal::Decimal;
use std::sync::RwLock;
use tracing::debug;

/// Which strategy semantics to apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrategyMode {
    /// No rebalancing; only optional fee collection.
    StaticRange,
    /// Rebalance periodically regardless of range status.
    Periodic,
    /// Rebalance when out of range OR when deviation from range midpoint exceeds threshold.
    Threshold,
    /// Rebalance on interval using rolling Bollinger bands as the target LP range.
    Bollinger,
    /// Rebalance only when price exits the range (matches backtest `OorRecenter`; no in-range midpoint recenters).
    OorRecenter,
    /// Shift only the exiting edge towards current price, once per out-of-range episode.
    RetouchShift,
    /// Rebalance on range-exit using low/high from the last closed candle.
    LastCandle,
    /// Rebalance on a time interval using the last closed candle band (or width fallback).
    LastCandlePeriodic,
    /// IL-based close/rebalance (legacy / future).
    IlLimit,
}

/// Configuration for the decision engine.
#[derive(Debug, Clone)]
pub struct DecisionConfig {
    /// Strategy semantics.
    pub strategy_mode: StrategyMode,
    /// IL threshold for rebalancing (as percentage).
    pub il_rebalance_threshold: Decimal,
    /// IL threshold for closing (as percentage).
    pub il_close_threshold: Decimal,
    /// Minimum time between rebalances in minutes.
    pub min_rebalance_interval_minutes: u64,
    /// For `Periodic`: rebalance every N minutes.
    pub periodic_interval_minutes: u64,
    /// For `Periodic`: if true, periodic rebalance triggers only when the position is out of range.
    /// This avoids an automatic close+open while in-range just because the timer elapsed.
    pub periodic_requires_out_of_range: bool,
    /// If true, exiting the range may trigger a rebalance immediately (subject to strategy mode),
    /// instead of waiting for `min_rebalance_interval_minutes` / `periodic_interval_minutes`.
    ///
    /// Default is false: range exit is *observed*, but the rebalance happens only on the schedule.
    pub rebalance_on_range_exit_immediately: bool,
    /// For `Threshold`: deviation from range midpoint that triggers rebalance.
    /// Expressed as a ratio (e.g. 0.05 = 5%).
    pub threshold_pct: Decimal,
    /// For `Bollinger`: rolling window length in points/samples.
    pub bollinger_window_points: u64,
    /// For `Bollinger`: standard deviation multiplier (k).
    pub bollinger_k: Decimal,
    /// For `RetouchShift`: shift full retouched band by this ratio (0.001 = +0.1%).
    pub retouch_offset_pct: Decimal,
    /// Range width for new positions (as percentage).
    pub range_width_pct: Decimal,
    /// Candle size in seconds for `LastCandle` mode.
    pub last_candle_seconds: u64,
    /// Whether to auto-collect fees.
    pub auto_collect_fees: bool,
    /// Minimum fees to collect in USD.
    pub min_fees_to_collect: Decimal,
}

impl Default for DecisionConfig {
    fn default() -> Self {
        Self {
            strategy_mode: StrategyMode::IlLimit,
            il_rebalance_threshold: Decimal::new(5, 2), // 5%
            il_close_threshold: Decimal::new(15, 2),    // 15%
            min_rebalance_interval_minutes: 24 * 60,
            periodic_interval_minutes: 24 * 60,
            // Backward-compatible defaults: old behavior was "rebalance immediately on range exit"
            // and `Periodic` was strictly timer-based regardless of in-range status.
            periodic_requires_out_of_range: false,
            rebalance_on_range_exit_immediately: true,
            threshold_pct: Decimal::new(5, 3),    // 0.5% by default
            bollinger_window_points: 20,
            bollinger_k: Decimal::new(2, 0),
            retouch_offset_pct: Decimal::ZERO,
            range_width_pct: Decimal::new(10, 2), // 10%
            last_candle_seconds: 3600,
            auto_collect_fees: true,
            min_fees_to_collect: Decimal::new(10, 0), // $10
        }
    }
}

/// Context for making decisions.
#[derive(Debug, Clone)]
pub struct DecisionContext {
    /// Current position state.
    pub position: MonitoredPosition,
    /// Current pool state.
    pub pool: WhirlpoolState,
    /// Minutes since last rebalance.
    pub minutes_since_rebalance: u64,
    /// For `RetouchShift`: whether we are allowed to retouch given the current out-of-range episode.
    pub retouch_armed: Option<bool>,
    /// Optional tick band derived from last closed candle low/high.
    pub last_candle_ticks: Option<(i32, i32)>,
    /// Optional tick band derived from rolling Bollinger window.
    pub bollinger_ticks: Option<(i32, i32)>,
}

/// Decision engine for automated strategy execution.
pub struct DecisionEngine {
    /// Configuration.
    config: RwLock<DecisionConfig>,
}

impl DecisionEngine {
    /// Creates a new decision engine.
    #[must_use]
    pub fn new(config: DecisionConfig) -> Self {
        Self {
            config: RwLock::new(config),
        }
    }

    /// Makes a decision for a position.
    pub fn decide(&self, context: &DecisionContext) -> Decision {
        let position = &context.position;
        let pool = &context.pool;

        let cfg = self.config.read().expect("decision config lock");

        debug!(
            position = %position.address,
            in_range = position.in_range,
            il_pct = %position.pnl.il_pct,
            "Evaluating position"
        );

        // Strategy-specific decision first. `CollectFees` is applied only when the strategy
        // would otherwise `Hold`, so Periodic / OorRecenter / Threshold / RetouchShift / IlLimit
        // are not starved by fee collection.
        let strategy_decision = match cfg.strategy_mode {
            StrategyMode::StaticRange => Decision::Hold,

            StrategyMode::Periodic => {
                if context.minutes_since_rebalance >= cfg.periodic_interval_minutes
                    && (!cfg.periodic_requires_out_of_range || !position.in_range)
                {
                    let (new_lower, new_upper) = self.calculate_new_range(pool);
                    debug!(
                        new_lower = new_lower,
                        new_upper = new_upper,
                        "Periodic rebalance"
                    );
                    return Decision::Rebalance {
                        new_tick_lower: new_lower,
                        new_tick_upper: new_upper,
                    };
                }
                Decision::Hold
            }

            StrategyMode::OorRecenter => {
                if !position.in_range {
                    if !cfg.rebalance_on_range_exit_immediately
                        && context.minutes_since_rebalance < cfg.min_rebalance_interval_minutes
                    {
                        debug!(
                            minutes_since_rebalance = context.minutes_since_rebalance,
                            min_rebalance_interval_minutes = cfg.min_rebalance_interval_minutes,
                            "OorRecenter: out of range but waiting for rebalance interval"
                        );
                        return Decision::Hold;
                    }
                    let (new_lower, new_upper) = self.calculate_new_range(pool);
                    debug!(
                        new_lower = new_lower,
                        new_upper = new_upper,
                        "OorRecenter: out of range"
                    );
                    return Decision::Rebalance {
                        new_tick_lower: new_lower,
                        new_tick_upper: new_upper,
                    };
                }
                Decision::Hold
            }

            StrategyMode::Threshold => {
                if !position.in_range {
                    if !cfg.rebalance_on_range_exit_immediately
                        && context.minutes_since_rebalance < cfg.min_rebalance_interval_minutes
                    {
                        debug!(
                            minutes_since_rebalance = context.minutes_since_rebalance,
                            min_rebalance_interval_minutes = cfg.min_rebalance_interval_minutes,
                            "Threshold: out of range but waiting for rebalance interval"
                        );
                        return Decision::Hold;
                    }
                    let (new_lower, new_upper) = self.calculate_new_range(pool);
                    debug!(
                        new_lower = new_lower,
                        new_upper = new_upper,
                        "Threshold: out of range"
                    );
                    return Decision::Rebalance {
                        new_tick_lower: new_lower,
                        new_tick_upper: new_upper,
                    };
                }

                // In-range: rebalance only if we are far enough from midpoint.
                let lower_price =
                    clmm_lp_protocols::prelude::tick_to_price(position.on_chain.tick_lower);
                let upper_price =
                    clmm_lp_protocols::prelude::tick_to_price(position.on_chain.tick_upper);
                let mid = (lower_price + upper_price) / Decimal::from(2u32);
                if mid.is_zero() {
                    return Decision::Hold;
                }
                let change = (pool.price - mid).abs() / mid;
                if change >= cfg.threshold_pct {
                    let (new_lower, new_upper) = self.calculate_new_range(pool);
                    debug!(
                        new_lower = new_lower,
                        new_upper = new_upper,
                        change = %change,
                        "Threshold: midpoint deviation"
                    );
                    return Decision::Rebalance {
                        new_tick_lower: new_lower,
                        new_tick_upper: new_upper,
                    };
                }

                Decision::Hold
            }

            StrategyMode::Bollinger => {
                if context.minutes_since_rebalance >= cfg.min_rebalance_interval_minutes {
                    if let Some((new_lower, new_upper)) = context.bollinger_ticks {
                        debug!(
                            new_lower = new_lower,
                            new_upper = new_upper,
                            window_points = cfg.bollinger_window_points,
                            k = %cfg.bollinger_k,
                            "Bollinger interval rebalance"
                        );
                        return Decision::Rebalance {
                            new_tick_lower: new_lower,
                            new_tick_upper: new_upper,
                        };
                    }
                    debug!(
                        window_points = cfg.bollinger_window_points,
                        "Bollinger: interval reached but not enough rolling points yet"
                    );
                }
                Decision::Hold
            }

            StrategyMode::RetouchShift => {
                if position.in_range {
                    return Decision::Hold;
                }
                let armed = context.retouch_armed.unwrap_or(false);
                if !armed {
                    return Decision::Hold;
                }
                if !cfg.rebalance_on_range_exit_immediately
                    && context.minutes_since_rebalance < cfg.min_rebalance_interval_minutes
                {
                    debug!(
                        minutes_since_rebalance = context.minutes_since_rebalance,
                        min_rebalance_interval_minutes = cfg.min_rebalance_interval_minutes,
                        "RetouchShift: out of range but waiting for rebalance interval"
                    );
                    return Decision::Hold;
                }

                let (new_lower, new_upper) = self.calculate_retouch_range(position, pool);
                debug!(
                    new_lower = new_lower,
                    new_upper = new_upper,
                    "RetouchShift: rebalance range edge"
                );
                return Decision::Rebalance {
                    new_tick_lower: new_lower,
                    new_tick_upper: new_upper,
                };
            }
            StrategyMode::LastCandle => {
                if !position.in_range {
                    if !cfg.rebalance_on_range_exit_immediately
                        && context.minutes_since_rebalance < cfg.min_rebalance_interval_minutes
                    {
                        debug!(
                            minutes_since_rebalance = context.minutes_since_rebalance,
                            min_rebalance_interval_minutes = cfg.min_rebalance_interval_minutes,
                            "LastCandle: out of range but waiting for rebalance interval"
                        );
                        return Decision::Hold;
                    }
                    let (new_lower, new_upper) = context
                        .last_candle_ticks
                        .unwrap_or_else(|| self.calculate_new_range(pool));
                    debug!(
                        new_lower = new_lower,
                        new_upper = new_upper,
                        has_last_candle_ticks = context.last_candle_ticks.is_some(),
                        "LastCandle: out of range"
                    );
                    return Decision::Rebalance {
                        new_tick_lower: new_lower,
                        new_tick_upper: new_upper,
                    };
                }
                Decision::Hold
            }
            StrategyMode::LastCandlePeriodic => {
                if context.minutes_since_rebalance >= cfg.min_rebalance_interval_minutes {
                    let (new_lower, new_upper) = context
                        .last_candle_ticks
                        .unwrap_or_else(|| self.calculate_new_range(pool));
                    debug!(
                        new_lower = new_lower,
                        new_upper = new_upper,
                        has_last_candle_ticks = context.last_candle_ticks.is_some(),
                        "LastCandlePeriodic: interval elapsed"
                    );
                    return Decision::Rebalance {
                        new_tick_lower: new_lower,
                        new_tick_upper: new_upper,
                    };
                }
                Decision::Hold
            }

            StrategyMode::IlLimit => {
                // Out-of-range recenter **before** IL-close: after an OOR move, `il_pct` often spikes
                // above `il_close_threshold`; if we tested close first, we'd `Close` (no auto re-open)
                // instead of `Rebalance` (close + open new range).
                if !position.in_range
                    && (cfg.rebalance_on_range_exit_immediately
                        || context.minutes_since_rebalance >= cfg.min_rebalance_interval_minutes)
                {
                    let (new_lower, new_upper) = self.calculate_new_range(pool);
                    debug!(
                        new_lower = new_lower,
                        new_upper = new_upper,
                        il_pct = %position.pnl.il_pct,
                        "IlLimit: out of range with rebalance cooldown OK — recenter"
                    );
                    return Decision::Rebalance {
                        new_tick_lower: new_lower,
                        new_tick_upper: new_upper,
                    };
                }

                // Critical IL while we are not taking an OOR recenter: full exit (no new position).
                if position.pnl.il_pct.abs() > cfg.il_close_threshold {
                    debug!(
                        il_pct = %position.pnl.il_pct,
                        threshold = %cfg.il_close_threshold,
                        in_range = position.in_range,
                        "IlLimit: IL exceeds close threshold — close only"
                    );
                    return Decision::Close;
                }

                // In-range (or OOR but rebalance cooldown not met): IL-based rebalance
                if position.pnl.il_pct.abs() > cfg.il_rebalance_threshold
                    && context.minutes_since_rebalance >= cfg.min_rebalance_interval_minutes
                {
                    let (new_lower, new_upper) = self.calculate_new_range(pool);
                    debug!(
                        il_pct = %position.pnl.il_pct,
                        "IlLimit: IL exceeds rebalance threshold"
                    );
                    return Decision::Rebalance {
                        new_tick_lower: new_lower,
                        new_tick_upper: new_upper,
                    };
                }

                Decision::Hold
            }
        };

        match strategy_decision {
            // Policy: fee collection should happen on close/rebalance flows, not as a standalone loop action.
            Decision::Hold => Decision::Hold,
            d => d,
        }
    }

    /// Calculates a new range centered on current price.
    fn calculate_new_range(&self, pool: &WhirlpoolState) -> (i32, i32) {
        let cfg = self.config.read().expect("decision config lock");
        clmm_lp_protocols::prelude::calculate_tick_range(
            pool.tick_current,
            cfg.range_width_pct,
            pool.tick_spacing,
        )
    }

    /// RetouchShift: shift only the exiting edge, keeping the original price-width.
    fn calculate_retouch_range(
        &self,
        position: &MonitoredPosition,
        pool: &WhirlpoolState,
    ) -> (i32, i32) {
        let spacing = pool.tick_spacing as i32;

        let lower_price = clmm_lp_protocols::prelude::tick_to_price(position.on_chain.tick_lower);
        let upper_price = clmm_lp_protocols::prelude::tick_to_price(position.on_chain.tick_upper);
        let current_price = pool.price;

        let (new_lower_price, new_upper_price) = if current_price > upper_price {
            let overflow = current_price - upper_price;
            (lower_price + overflow, current_price)
        } else {
            // current_price < lower_price
            let overflow = lower_price - current_price;
            (current_price, upper_price - overflow)
        };
        let cfg = self.config.read().expect("decision config lock");
        let shift = Decimal::ONE + cfg.retouch_offset_pct;
        let (new_lower_price, new_upper_price) = if shift > Decimal::ZERO {
            (new_lower_price * shift, new_upper_price * shift)
        } else {
            (new_lower_price, new_upper_price)
        };

        let mut new_lower_tick =
            clmm_lp_protocols::prelude::price_to_tick(new_lower_price.max(Decimal::ZERO));
        let mut new_upper_tick =
            clmm_lp_protocols::prelude::price_to_tick(new_upper_price.max(Decimal::ZERO));

        // Round to nearest allowed tick spacing.
        if spacing > 0 {
            new_lower_tick = ((new_lower_tick as f64) / (spacing as f64)).round() as i32 * spacing;
            new_upper_tick = ((new_upper_tick as f64) / (spacing as f64)).round() as i32 * spacing;
        }

        // Ensure sane ordering after rounding.
        if new_upper_tick <= new_lower_tick {
            new_upper_tick = new_lower_tick + spacing.max(1);
        }

        (new_lower_tick, new_upper_tick)
    }

    /// Updates the configuration.
    pub fn set_config(&self, config: DecisionConfig) {
        *self.config.write().expect("decision config lock") = config;
    }

    /// Gets the current configuration.
    #[must_use]
    pub fn config(&self) -> DecisionConfig {
        self.config.read().expect("decision config lock").clone()
    }
}

impl Default for DecisionEngine {
    fn default() -> Self {
        Self::new(DecisionConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitor::PositionPnL;
    use solana_sdk::pubkey::Pubkey;

    fn create_test_context(in_range: bool, il_pct: Decimal) -> DecisionContext {
        let position = MonitoredPosition {
            address: Pubkey::new_unique(),
            pool: Pubkey::new_unique(),
            on_chain: clmm_lp_protocols::prelude::OnChainPosition {
                address: Pubkey::new_unique(),
                pool: Pubkey::new_unique(),
                owner: Pubkey::new_unique(),
                tick_lower: -1000,
                tick_upper: 1000,
                liquidity: 1000000,
                fee_growth_inside_a: 0,
                fee_growth_inside_b: 0,
                fees_owed_a: 0,
                fees_owed_b: 0,
            },
            pnl: PositionPnL {
                il_pct,
                ..Default::default()
            },
            in_range,
            last_updated: chrono::Utc::now(),
        };

        let pool = WhirlpoolState {
            address: String::new(),
            token_mint_a: Pubkey::new_unique(),
            token_mint_b: Pubkey::new_unique(),
            token_vault_a: Pubkey::new_unique(),
            token_vault_b: Pubkey::new_unique(),
            tick_current: 0,
            tick_spacing: 64,
            sqrt_price: 1 << 64,
            price: Decimal::ONE,
            liquidity: 1000000,
            fee_rate_bps: 30,
            protocol_fee_rate_bps: 0,
            protocol_fee_owed_a: 0,
            protocol_fee_owed_b: 0,
            fee_growth_global_a: 0,
            fee_growth_global_b: 0,
        };

        DecisionContext {
            position,
            pool,
            minutes_since_rebalance: 48 * 60,
            retouch_armed: None,
            last_candle_ticks: None,
            bollinger_ticks: None,
        }
    }

    fn engine_with_mode(mode: StrategyMode) -> DecisionEngine {
        let cfg = DecisionConfig {
            strategy_mode: mode,
            ..DecisionConfig::default()
        };
        DecisionEngine::new(cfg)
    }

    #[test]
    fn test_hold_decision() {
        let engine = DecisionEngine::default();
        let context = create_test_context(true, Decimal::ZERO);

        let decision = engine.decide(&context);
        assert!(matches!(decision, Decision::Hold));
    }

    #[test]
    fn test_rebalance_on_range_exit() {
        let engine = DecisionEngine::default();
        let context = create_test_context(false, Decimal::ZERO);

        let decision = engine.decide(&context);
        assert!(matches!(decision, Decision::Rebalance { .. }));
    }

    #[test]
    fn test_close_on_high_il() {
        let engine = DecisionEngine::default();
        let context = create_test_context(true, Decimal::new(20, 2)); // 20% IL

        let decision = engine.decide(&context);
        assert!(matches!(decision, Decision::Close));
    }

    /// OOR + IL above *close* threshold used to hit `Close` first — no new position. Recenter first.
    #[test]
    fn test_oor_rebalance_before_il_close_when_cooldown_ok() {
        let engine = DecisionEngine::default();
        let mut context = create_test_context(false, Decimal::new(20, 2)); // OOR + 20% IL
        context.minutes_since_rebalance = 48 * 60;

        let decision = engine.decide(&context);
        assert!(
            matches!(decision, Decision::Rebalance { .. }),
            "expected Rebalance, got {decision:?}"
        );
    }

    #[test]
    fn test_retouch_shift_rebalances_when_armed_and_out_of_range() {
        let engine = engine_with_mode(StrategyMode::RetouchShift);
        let mut context = create_test_context(false, Decimal::ZERO);
        context.retouch_armed = Some(true);
        context.pool.price = Decimal::from(2u32); // clearly above upper tick price for test range

        let decision = engine.decide(&context);
        assert!(matches!(decision, Decision::Rebalance { .. }));
    }

    #[test]
    fn test_retouch_shift_holds_when_not_armed() {
        let engine = engine_with_mode(StrategyMode::RetouchShift);
        let mut context = create_test_context(false, Decimal::ZERO);
        context.retouch_armed = Some(false);
        context.pool.price = Decimal::from(2u32);

        let decision = engine.decide(&context);
        assert!(matches!(decision, Decision::Hold));
    }

    #[test]
    fn test_retouch_shift_holds_when_back_in_range() {
        let engine = engine_with_mode(StrategyMode::RetouchShift);
        let mut context = create_test_context(true, Decimal::ZERO);
        context.retouch_armed = Some(true);
        context.pool.price = Decimal::ONE;

        let decision = engine.decide(&context);
        assert!(matches!(decision, Decision::Hold));
    }

    #[test]
    fn test_oor_recenter_waits_for_interval_by_default() {
        let cfg = DecisionConfig {
            strategy_mode: StrategyMode::OorRecenter,
            min_rebalance_interval_minutes: 60,
            rebalance_on_range_exit_immediately: false,
            ..DecisionConfig::default()
        };
        let engine = DecisionEngine::new(cfg);

        let mut context = create_test_context(false, Decimal::ZERO);
        context.minutes_since_rebalance = 0;

        let decision = engine.decide(&context);
        assert!(
            matches!(decision, Decision::Hold),
            "expected Hold, got {decision:?}"
        );
    }

    #[test]
    fn test_last_candle_uses_candle_ticks_when_out_of_range() {
        let cfg = DecisionConfig {
            strategy_mode: StrategyMode::LastCandle,
            ..DecisionConfig::default()
        };
        let engine = DecisionEngine::new(cfg);
        let mut context = create_test_context(false, Decimal::ZERO);
        context.last_candle_ticks = Some((-512, 512));

        let decision = engine.decide(&context);
        match decision {
            Decision::Rebalance {
                new_tick_lower,
                new_tick_upper,
            } => {
                assert_eq!(new_tick_lower, -512);
                assert_eq!(new_tick_upper, 512);
            }
            other => panic!("expected Rebalance, got {other:?}"),
        }
    }

    #[test]
    fn test_last_candle_falls_back_without_candle_ticks() {
        let cfg = DecisionConfig {
            strategy_mode: StrategyMode::LastCandle,
            ..DecisionConfig::default()
        };
        let engine = DecisionEngine::new(cfg);
        let context = create_test_context(false, Decimal::ZERO);

        let decision = engine.decide(&context);
        assert!(matches!(decision, Decision::Rebalance { .. }));
    }

    #[test]
    fn test_last_candle_periodic_rebalances_on_interval_in_range() {
        let cfg = DecisionConfig {
            strategy_mode: StrategyMode::LastCandlePeriodic,
            min_rebalance_interval_minutes: 60,
            ..DecisionConfig::default()
        };
        let engine = DecisionEngine::new(cfg);
        let mut context = create_test_context(true, Decimal::ZERO);
        context.minutes_since_rebalance = 60;
        context.last_candle_ticks = Some((-256, 256));

        let decision = engine.decide(&context);
        match decision {
            Decision::Rebalance {
                new_tick_lower,
                new_tick_upper,
            } => {
                assert_eq!(new_tick_lower, -256);
                assert_eq!(new_tick_upper, 256);
            }
            other => panic!("expected Rebalance, got {other:?}"),
        }
    }

    #[test]
    fn test_last_candle_periodic_holds_before_interval() {
        let cfg = DecisionConfig {
            strategy_mode: StrategyMode::LastCandlePeriodic,
            min_rebalance_interval_minutes: 120,
            ..DecisionConfig::default()
        };
        let engine = DecisionEngine::new(cfg);
        let mut context = create_test_context(false, Decimal::ZERO);
        context.minutes_since_rebalance = 60;

        let decision = engine.decide(&context);
        assert!(matches!(decision, Decision::Hold));
    }

    #[test]
    fn test_static_range_does_not_emit_collect_fees_decision() {
        let cfg = DecisionConfig {
            strategy_mode: StrategyMode::StaticRange,
            auto_collect_fees: true,
            min_fees_to_collect: Decimal::ONE,
            ..DecisionConfig::default()
        };
        let engine = DecisionEngine::new(cfg);
        let mut context = create_test_context(true, Decimal::ZERO);
        context.position.pnl.fees_usd = Decimal::from(100u32);
        let decision = engine.decide(&context);
        assert!(matches!(decision, Decision::Hold));
    }
}
