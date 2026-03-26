//! Position tracking for simulations.
//!
//! This module provides functionality to track position state over time,
//! recording snapshots and computing metrics at each step.

use crate::strategies::{RebalanceAction, RebalanceStrategy, StrategyContext};
use clmm_lp_domain::metrics::impermanent_loss::calculate_il_concentrated;
use clmm_lp_domain::value_objects::price::Price;
use clmm_lp_domain::value_objects::price_range::PriceRange;
use rust_decimal::Decimal;

/// A snapshot of position state at a point in time.
#[derive(Debug, Clone)]
pub struct PositionSnapshot {
    /// Step number in the simulation.
    pub step: u64,
    /// Current price at this step.
    pub price: Price,
    /// Current position range.
    pub range: PriceRange,
    /// Whether price is in range.
    pub in_range: bool,
    /// Cumulative fees earned up to this step.
    pub cumulative_fees: Decimal,
    /// Current impermanent loss percentage.
    pub il_pct: Decimal,
    /// Current position value in USD.
    pub position_value_usd: Decimal,
    /// Net PnL at this step.
    pub net_pnl: Decimal,
    /// Action taken at this step (if any).
    pub action: Option<RebalanceAction>,
}

/// Tracks position state throughout a simulation.
///
/// Uses **segment-based** IL: after each rebalance, "entry" and capital for IL
/// are reset to the price and position value at rebalance time. This makes IL
/// and position value correct for a sequence of ranges (continuous rebalancing
/// vs one wide range is comparable).
#[derive(Debug)]
pub struct PositionTracker {
    /// Initial capital in USD (never changed; used for net PnL vs start).
    pub initial_capital: Decimal,
    /// Entry price at very first step (used for strategy context and HODL).
    pub entry_price: Price,
    /// Current range.
    pub current_range: PriceRange,
    /// All recorded snapshots.
    pub snapshots: Vec<PositionSnapshot>,
    /// Steps since last rebalance.
    pub steps_since_rebalance: u64,
    /// Total rebalance count.
    pub rebalance_count: u32,
    /// Total transaction costs from rebalancing.
    pub total_rebalance_cost: Decimal,
    /// Cost per rebalance in USD.
    pub rebalance_cost: Decimal,
    /// Cumulative fees earned.
    cumulative_fees: Decimal,
    /// Current step.
    current_step: u64,

    // Segment state (reset on each rebalance for correct path-dependent IL)
    /// Entry price for the current segment (price when this range was opened).
    segment_entry_price: Price,
    /// Capital at start of current segment (position value at last rebalance).
    segment_capital: Decimal,
    /// Cumulative fees at start of current segment.
    segment_cumulative_fees: Decimal,
    /// Total rebalance cost at start of current segment.
    segment_rebalance_cost: Decimal,
}

impl PositionTracker {
    /// Creates a new position tracker.
    ///
    /// # Arguments
    ///
    /// * `initial_capital` - Starting capital in USD
    /// * `entry_price` - Price at position entry
    /// * `initial_range` - Initial price range
    /// * `rebalance_cost` - Cost per rebalance transaction in USD
    #[must_use]
    pub fn new(
        initial_capital: Decimal,
        entry_price: Price,
        initial_range: PriceRange,
        rebalance_cost: Decimal,
    ) -> Self {
        Self {
            initial_capital,
            entry_price,
            current_range: initial_range,
            snapshots: Vec::new(),
            steps_since_rebalance: 0,
            rebalance_count: 0,
            total_rebalance_cost: Decimal::ZERO,
            rebalance_cost,
            cumulative_fees: Decimal::ZERO,
            current_step: 0,
            segment_entry_price: entry_price,
            segment_capital: initial_capital,
            segment_cumulative_fees: Decimal::ZERO,
            segment_rebalance_cost: Decimal::ZERO,
        }
    }

    /// Records a step in the simulation.
    ///
    /// # Arguments
    ///
    /// * `price` - Current price
    /// * `step_fees` - Fees earned this step
    /// * `strategy` - Optional strategy to evaluate for rebalancing
    ///
    /// # Returns
    ///
    /// The action taken (if any)
    pub fn record_step<S: RebalanceStrategy>(
        &mut self,
        price: Price,
        step_fees: Decimal,
        strategy: Option<&S>,
    ) -> Option<RebalanceAction> {
        self.current_step += 1;
        self.steps_since_rebalance += 1;
        self.cumulative_fees += step_fees;

        // IL for current segment (entry = segment start, so correct for sequence of ranges)
        let il_pct = calculate_il_concentrated(
            self.segment_entry_price.value,
            price.value,
            self.current_range.lower_price.value,
            self.current_range.upper_price.value,
        )
        .unwrap_or(Decimal::ZERO);

        // Position value: segment capital + IL in segment + fees since segment − costs since segment
        let fees_since_segment = self.cumulative_fees - self.segment_cumulative_fees;
        let costs_since_segment = self.total_rebalance_cost - self.segment_rebalance_cost;
        let il_amount = self.segment_capital * il_pct;
        let position_value =
            self.segment_capital + il_amount + fees_since_segment - costs_since_segment;
        let net_pnl = position_value - self.initial_capital;

        // Check if in range
        let in_range = price.value >= self.current_range.lower_price.value
            && price.value <= self.current_range.upper_price.value;

        // Evaluate strategy if provided
        let action = strategy.map(|s| {
            let context = StrategyContext {
                current_price: price,
                current_range: self.current_range.clone(),
                entry_price: self.entry_price,
                steps_since_open: self.current_step,
                steps_since_rebalance: self.steps_since_rebalance,
                current_il_pct: il_pct,
                total_fees_earned: self.cumulative_fees,
            };
            s.evaluate(&context)
        });

        // Handle rebalance action (pass current price and position value for segment reset)
        let final_action = if let Some(ref act) = action {
            match act {
                RebalanceAction::Rebalance { new_range, .. } => {
                    self.execute_rebalance(new_range.clone(), price, position_value);
                    action.clone()
                }
                RebalanceAction::Close { .. } => action.clone(),
                RebalanceAction::Hold => None,
            }
        } else {
            None
        };

        // Record snapshot
        let snapshot = PositionSnapshot {
            step: self.current_step,
            price,
            range: self.current_range.clone(),
            in_range,
            cumulative_fees: self.cumulative_fees,
            il_pct,
            position_value_usd: position_value,
            net_pnl,
            action: final_action.clone(),
        };
        self.snapshots.push(snapshot);

        final_action
    }

    /// Executes a rebalance to a new range and starts a new segment for IL.
    fn execute_rebalance(
        &mut self,
        new_range: PriceRange,
        rebalance_price: Price,
        position_value_before_cost: Decimal,
    ) {
        let capital_after_cost = position_value_before_cost - self.rebalance_cost;
        self.segment_entry_price = rebalance_price;
        self.segment_capital = capital_after_cost;
        self.segment_cumulative_fees = self.cumulative_fees;
        self.segment_rebalance_cost = self.total_rebalance_cost;
        self.current_range = new_range;
        self.steps_since_rebalance = 0;
        self.rebalance_count += 1;
        self.total_rebalance_cost += self.rebalance_cost;
    }

    /// Returns summary statistics for the tracked position.
    #[must_use]
    pub fn summary(&self) -> TrackerSummary {
        let total_steps = self.snapshots.len() as u64;
        let in_range_steps = self.snapshots.iter().filter(|s| s.in_range).count() as u64;

        let time_in_range_pct = if total_steps > 0 {
            Decimal::from(in_range_steps) / Decimal::from(total_steps)
        } else {
            Decimal::ZERO
        };

        let final_snapshot = self.snapshots.last();
        let final_value = final_snapshot
            .map(|s| s.position_value_usd)
            .unwrap_or(self.initial_capital);
        let final_pnl = final_snapshot.map(|s| s.net_pnl).unwrap_or(Decimal::ZERO);
        let final_il_segment = final_snapshot.map(|s| s.il_pct);

        // Calculate max drawdown
        let mut peak = self.initial_capital;
        let mut max_drawdown = Decimal::ZERO;
        for snapshot in &self.snapshots {
            if snapshot.position_value_usd > peak {
                peak = snapshot.position_value_usd;
            }
            let drawdown = (peak - snapshot.position_value_usd) / peak;
            if drawdown > max_drawdown {
                max_drawdown = drawdown;
            }
        }

        // HODL comparison:
        // The tracker doesn't know quote/USD dynamics, so the only safe default here is the
        // **USD cash benchmark**: holding the initial USD capital constant.
        //
        // Cross-pair commands (e.g. whETH/SOL) should override `hodl_value` with a correct
        // 50/50 USD split across both legs (A and B) using their USD prices.
        let hodl_value = self.initial_capital;
        let vs_hodl = final_value - hodl_value;
        let position_value_before_fees =
            final_value - self.cumulative_fees + self.total_rebalance_cost;
        let il_vs_hodl_ex_fees_pct = if self.initial_capital > Decimal::ZERO {
            (position_value_before_fees - hodl_value) / self.initial_capital
        } else {
            Decimal::ZERO
        };

        TrackerSummary {
            total_steps,
            final_value,
            final_pnl,
            final_il_pct: il_vs_hodl_ex_fees_pct,
            final_il_segment_pct: final_il_segment,
            final_il_vs_hodl_ex_fees_pct: il_vs_hodl_ex_fees_pct,
            total_fees: self.cumulative_fees,
            time_in_range_pct,
            rebalance_count: self.rebalance_count,
            total_rebalance_cost: self.total_rebalance_cost,
            max_drawdown,
            hodl_value,
            vs_hodl,
        }
    }
}

/// Summary statistics from position tracking.
#[derive(Debug, Clone)]
pub struct TrackerSummary {
    /// Total simulation steps.
    pub total_steps: u64,
    /// Final position value in USD.
    pub final_value: Decimal,
    /// Final net PnL.
    pub final_pnl: Decimal,
    /// Backward-compatible IL field used by older reports/objectives.
    /// Equals `final_il_vs_hodl_ex_fees_pct`.
    pub final_il_pct: Decimal,
    /// Last-segment concentrated IL (entry at last rebalance, current range/price).
    pub final_il_segment_pct: Option<Decimal>,
    /// End-of-backtest IL-like metric: LP underlying (excluding fees/costs) vs HODL benchmark.
    pub final_il_vs_hodl_ex_fees_pct: Decimal,
    /// Total fees earned.
    pub total_fees: Decimal,
    /// Percentage of time in range.
    pub time_in_range_pct: Decimal,
    /// Number of rebalances executed.
    pub rebalance_count: u32,
    /// Total cost of rebalancing.
    pub total_rebalance_cost: Decimal,
    /// Maximum drawdown percentage.
    pub max_drawdown: Decimal,
    /// HODL strategy value for comparison.
    pub hodl_value: Decimal,
    /// Performance vs HODL (positive = outperformed).
    pub vs_hodl: Decimal,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strategies::StaticRange;
    use rust_decimal_macros::dec;

    #[test]
    fn test_tracker_basic() {
        let mut tracker = PositionTracker::new(
            dec!(1000),
            Price::new(dec!(100)),
            PriceRange::new(Price::new(dec!(90)), Price::new(dec!(110))),
            dec!(5),
        );

        // Record some steps
        tracker.record_step::<StaticRange>(Price::new(dec!(100)), dec!(10), None);
        tracker.record_step::<StaticRange>(Price::new(dec!(102)), dec!(10), None);
        tracker.record_step::<StaticRange>(Price::new(dec!(98)), dec!(10), None);

        assert_eq!(tracker.snapshots.len(), 3);
        assert_eq!(tracker.cumulative_fees, dec!(30));

        let summary = tracker.summary();
        assert_eq!(summary.total_steps, 3);
        assert_eq!(summary.total_fees, dec!(30));
        assert_eq!(summary.rebalance_count, 0);
    }

    #[test]
    fn test_tracker_with_strategy() {
        use crate::strategies::ThresholdRebalance;

        let mut tracker = PositionTracker::new(
            dec!(1000),
            Price::new(dec!(100)),
            PriceRange::new(Price::new(dec!(90)), Price::new(dec!(110))),
            dec!(5),
        );

        let strategy = ThresholdRebalance::new(dec!(0.05), dec!(0.2));

        // Price stays in range, no rebalance
        tracker.record_step(Price::new(dec!(100)), dec!(10), Some(&strategy));
        assert_eq!(tracker.rebalance_count, 0);

        // Price moves significantly, should trigger rebalance
        tracker.record_step(Price::new(dec!(120)), dec!(5), Some(&strategy));
        assert_eq!(tracker.rebalance_count, 1);
        assert_eq!(tracker.total_rebalance_cost, dec!(5));

        // New range should be centered on 120
        assert_eq!(tracker.current_range.lower_price.value, dec!(108)); // 120 - 12
        assert_eq!(tracker.current_range.upper_price.value, dec!(132)); // 120 + 12
    }

    #[test]
    fn test_tracker_time_in_range() {
        let mut tracker = PositionTracker::new(
            dec!(1000),
            Price::new(dec!(100)),
            PriceRange::new(Price::new(dec!(90)), Price::new(dec!(110))),
            dec!(5),
        );

        // 2 in range, 1 out of range
        tracker.record_step::<StaticRange>(Price::new(dec!(100)), dec!(10), None);
        tracker.record_step::<StaticRange>(Price::new(dec!(120)), dec!(0), None); // out
        tracker.record_step::<StaticRange>(Price::new(dec!(105)), dec!(10), None);

        let summary = tracker.summary();
        // 2/3 in range
        assert!(summary.time_in_range_pct > dec!(0.66));
        assert!(summary.time_in_range_pct < dec!(0.67));
    }

    #[test]
    fn test_il_fields_are_not_mixed_after_rebalance() {
        use crate::strategies::ThresholdRebalance;

        let mut tracker = PositionTracker::new(
            dec!(1000),
            Price::new(dec!(100)),
            PriceRange::new(Price::new(dec!(90)), Price::new(dec!(110))),
            dec!(5),
        );

        let strategy = ThresholdRebalance::new(dec!(0.05), dec!(0.2));
        tracker.record_step(Price::new(dec!(100)), Decimal::ZERO, Some(&strategy));
        tracker.record_step(Price::new(dec!(120)), Decimal::ZERO, Some(&strategy)); // rebalance
        tracker.record_step(Price::new(dec!(120)), Decimal::ZERO, Some(&strategy)); // same as segment entry

        let summary = tracker.summary();
        let seg_il = summary.final_il_segment_pct.unwrap_or(dec!(0));

        // At segment entry price IL is ~0, but IL-vs-HODL includes realized path/cost effects.
        assert!(seg_il.abs() < dec!(0.000001));
        assert!(summary.final_il_vs_hodl_ex_fees_pct < Decimal::ZERO);
        assert_eq!(summary.final_il_pct, summary.final_il_vs_hodl_ex_fees_pct);
    }
}
