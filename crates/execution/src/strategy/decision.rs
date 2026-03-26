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
    /// Rebalance only when price exits the range (matches backtest `OorRecenter`; no in-range midpoint recenters).
    OorRecenter,
    /// Shift only the exiting edge towards current price, once per out-of-range episode.
    RetouchShift,
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
    /// Minimum time between rebalances in hours.
    pub min_rebalance_interval_hours: u64,
    /// For `Periodic`: rebalance every N hours.
    pub periodic_interval_hours: u64,
    /// For `Threshold`: deviation from range midpoint that triggers rebalance.
    /// Expressed as a ratio (e.g. 0.05 = 5%).
    pub threshold_pct: Decimal,
    /// Range width for new positions (as percentage).
    pub range_width_pct: Decimal,
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
            min_rebalance_interval_hours: 24,
            periodic_interval_hours: 24,
            threshold_pct: Decimal::new(5, 3),    // 0.5% by default
            range_width_pct: Decimal::new(10, 2), // 10%
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
    /// Hours since last rebalance.
    pub hours_since_rebalance: u64,
    /// For `RetouchShift`: whether we are allowed to retouch given the current out-of-range episode.
    pub retouch_armed: Option<bool>,
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
                if context.hours_since_rebalance >= cfg.periodic_interval_hours {
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

            StrategyMode::RetouchShift => {
                if position.in_range {
                    return Decision::Hold;
                }
                let armed = context.retouch_armed.unwrap_or(false);
                if !armed {
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

            StrategyMode::IlLimit => {
                // Check for critical IL - close position
                if position.pnl.il_pct.abs() > cfg.il_close_threshold {
                    debug!("IL exceeds close threshold, recommending close");
                    return Decision::Close;
                }

                // Check if out of range
                if !position.in_range {
                    // Check if enough time has passed since last rebalance
                    if context.hours_since_rebalance >= cfg.min_rebalance_interval_hours {
                        let (new_lower, new_upper) = self.calculate_new_range(pool);
                        debug!(
                            new_lower = new_lower,
                            new_upper = new_upper,
                            "Position out of range, recommending rebalance"
                        );
                        return Decision::Rebalance {
                            new_tick_lower: new_lower,
                            new_tick_upper: new_upper,
                        };
                    }
                }

                // Check for IL-based rebalancing
                if position.pnl.il_pct.abs() > cfg.il_rebalance_threshold
                    && context.hours_since_rebalance >= cfg.min_rebalance_interval_hours
                {
                    let (new_lower, new_upper) = self.calculate_new_range(pool);
                    debug!(
                        il_pct = %position.pnl.il_pct,
                        "IL exceeds threshold, recommending rebalance"
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
            Decision::Hold => {
                if cfg.auto_collect_fees && position.pnl.fees_usd > cfg.min_fees_to_collect {
                    debug!("Fees exceed threshold, recommending collection");
                    Decision::CollectFees
                } else {
                    Decision::Hold
                }
            }
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
            hours_since_rebalance: 48,
            retouch_armed: None,
        }
    }

    fn engine_with_mode(mode: StrategyMode) -> DecisionEngine {
        let mut cfg = DecisionConfig::default();
        cfg.strategy_mode = mode;
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
}
