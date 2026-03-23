//! Parameter optimization for rebalancing strategies.
//!
//! This module provides optimization for strategy parameters such as
//! rebalancing thresholds, intervals, and IL limits.

use crate::constraints::RebalanceConstraints;
use crate::objective::ObjectiveFunction;
use crate::optimizer::OptimizationConfig;
use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};

/// Parameters for a threshold-based rebalancing strategy.
#[derive(Debug, Clone)]
pub struct ThresholdParams {
    /// Price deviation threshold to trigger rebalance.
    pub price_threshold: Decimal,
    /// IL threshold to trigger rebalance.
    pub il_threshold: Decimal,
    /// Whether to rebalance when out of range.
    pub rebalance_on_out_of_range: bool,
}

impl Default for ThresholdParams {
    fn default() -> Self {
        Self {
            price_threshold: Decimal::from_f64(0.05).unwrap(), // 5%
            il_threshold: Decimal::from_f64(0.03).unwrap(),    // 3%
            rebalance_on_out_of_range: true,
        }
    }
}

/// Parameters for a periodic rebalancing strategy.
#[derive(Debug, Clone)]
pub struct PeriodicParams {
    /// Interval between rebalances in steps.
    pub interval: u64,
    /// Whether to only rebalance when out of range.
    pub only_when_out_of_range: bool,
}

impl Default for PeriodicParams {
    fn default() -> Self {
        Self {
            interval: 24, // Daily
            only_when_out_of_range: false,
        }
    }
}

/// Parameters for an IL-limit strategy.
#[derive(Debug, Clone)]
pub struct ILLimitParams {
    /// Maximum IL before rebalancing.
    pub max_il: Decimal,
    /// Maximum IL before closing position.
    pub close_il: Option<Decimal>,
    /// Grace period in steps before acting.
    pub grace_period: u64,
}

impl Default for ILLimitParams {
    fn default() -> Self {
        Self {
            max_il: Decimal::from_f64(0.05).unwrap(),         // 5%
            close_il: Some(Decimal::from_f64(0.10).unwrap()), // 10%
            grace_period: 0,
        }
    }
}

/// Result of parameter optimization.
#[derive(Debug, Clone)]
pub struct ParameterOptimizationResult {
    /// Baseline static range candidate.
    pub static_range: Option<StaticRangeCandidate>,
    /// Best threshold parameters found.
    pub threshold_params: Option<ThresholdParams>,
    /// Best periodic parameters found.
    pub periodic_params: Option<PeriodicParams>,
    /// Best IL limit parameters found.
    pub il_limit_params: Option<ILLimitParams>,
    /// Best retouch-shift parameters found.
    pub retouch_shift_params: Option<RetouchShiftParams>,
    /// Expected performance metrics.
    pub expected_fees: Decimal,
    /// Expected IL.
    pub expected_il: Decimal,
    /// Expected net PnL.
    pub expected_net_pnl: Decimal,
    /// Number of expected rebalances.
    pub expected_rebalances: u32,
    /// Score from objective function.
    pub score: Decimal,
}

/// Optimizer for rebalancing strategy parameters.
#[derive(Debug, Clone)]
pub struct ParameterOptimizer {
    /// Constraints for parameter search.
    pub constraints: RebalanceConstraints,
    /// Grid of price thresholds to search.
    price_thresholds: Vec<Decimal>,
    /// Grid of IL thresholds to search.
    il_thresholds: Vec<Decimal>,
    /// Grid of intervals to search.
    intervals: Vec<u64>,
    /// Grid of retouch cooldown intervals in steps.
    retouch_cooldowns: Vec<u64>,
}

impl Default for ParameterOptimizer {
    fn default() -> Self {
        Self::new()
    }
}

impl ParameterOptimizer {
    /// Creates a new parameter optimizer with default grids.
    #[must_use]
    pub fn new() -> Self {
        Self {
            constraints: RebalanceConstraints::default(),
            price_thresholds: vec![0.02, 0.03, 0.05, 0.07, 0.10, 0.15]
                .into_iter()
                .filter_map(Decimal::from_f64)
                .collect(),
            il_thresholds: vec![0.01, 0.02, 0.03, 0.05, 0.07, 0.10]
                .into_iter()
                .filter_map(Decimal::from_f64)
                .collect(),
            intervals: vec![6, 12, 24, 48, 72, 168], // 6h to 1 week
            retouch_cooldowns: vec![1, 3, 6, 12, 24],
        }
    }

    /// Sets custom price threshold grid.
    #[must_use]
    pub fn with_price_thresholds(mut self, thresholds: Vec<Decimal>) -> Self {
        self.price_thresholds = thresholds;
        self
    }

    /// Sets custom IL threshold grid.
    #[must_use]
    pub fn with_il_thresholds(mut self, thresholds: Vec<Decimal>) -> Self {
        self.il_thresholds = thresholds;
        self
    }

    /// Sets custom interval grid.
    #[must_use]
    pub fn with_intervals(mut self, intervals: Vec<u64>) -> Self {
        self.intervals = intervals;
        self
    }

    /// Sets custom retouch cooldown grid.
    #[must_use]
    pub fn with_retouch_cooldowns(mut self, cooldowns: Vec<u64>) -> Self {
        self.retouch_cooldowns = cooldowns;
        self
    }

    /// Sets constraints.
    #[must_use]
    pub fn with_constraints(mut self, constraints: RebalanceConstraints) -> Self {
        self.constraints = constraints;
        self
    }

    /// Optimizes threshold strategy parameters.
    pub fn optimize_threshold<O: ObjectiveFunction>(
        &self,
        config: &OptimizationConfig,
        range_width: Decimal,
        objective: &O,
    ) -> Vec<ThresholdCandidate> {
        let mut candidates = Vec::new();

        for &price_threshold in &self.price_thresholds {
            if !self.constraints.is_valid_price_threshold(price_threshold) {
                continue;
            }

            for &il_threshold in &self.il_thresholds {
                if !self.constraints.is_valid_il_threshold(il_threshold) {
                    continue;
                }

                for rebalance_on_oor in [true, false] {
                    let params = ThresholdParams {
                        price_threshold,
                        il_threshold,
                        rebalance_on_out_of_range: rebalance_on_oor,
                    };

                    let result = self.estimate_threshold_performance(&params, config, range_width);

                    let sim_result = create_sim_result(&result);
                    let score = objective.evaluate(&sim_result);

                    candidates.push(ThresholdCandidate {
                        params,
                        expected_fees: result.0,
                        expected_il: result.1,
                        expected_rebalances: result.2,
                        score,
                    });
                }
            }
        }

        // Sort by score descending
        candidates.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        candidates
    }

    /// Optimizes periodic strategy parameters.
    pub fn optimize_periodic<O: ObjectiveFunction>(
        &self,
        config: &OptimizationConfig,
        range_width: Decimal,
        objective: &O,
    ) -> Vec<PeriodicCandidate> {
        let mut candidates = Vec::new();

        for &interval in &self.intervals {
            if !self.constraints.is_valid_interval(interval) {
                continue;
            }

            for only_oor in [true, false] {
                let params = PeriodicParams {
                    interval,
                    only_when_out_of_range: only_oor,
                };

                let result = self.estimate_periodic_performance(&params, config, range_width);

                let sim_result = create_sim_result(&result);
                let score = objective.evaluate(&sim_result);

                candidates.push(PeriodicCandidate {
                    params,
                    expected_fees: result.0,
                    expected_il: result.1,
                    expected_rebalances: result.2,
                    score,
                });
            }
        }

        candidates.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        candidates
    }

    /// Optimizes IL limit strategy parameters.
    pub fn optimize_il_limit<O: ObjectiveFunction>(
        &self,
        config: &OptimizationConfig,
        range_width: Decimal,
        objective: &O,
    ) -> Vec<ILLimitCandidate> {
        let mut candidates = Vec::new();

        for &max_il in &self.il_thresholds {
            if !self.constraints.is_valid_il_threshold(max_il) {
                continue;
            }

            // Close IL options: None, 2x max_il, 3x max_il
            let close_options = vec![
                None,
                Some(max_il * Decimal::from(2)),
                Some(max_il * Decimal::from(3)),
            ];

            for close_il in close_options {
                for grace_period in [0, 3, 6] {
                    let params = ILLimitParams {
                        max_il,
                        close_il,
                        grace_period,
                    };

                    let result = self.estimate_il_limit_performance(&params, config, range_width);

                    let sim_result = create_sim_result(&result);
                    let score = objective.evaluate(&sim_result);

                    candidates.push(ILLimitCandidate {
                        params,
                        expected_fees: result.0,
                        expected_il: result.1,
                        expected_rebalances: result.2,
                        score,
                    });
                }
            }
        }

        candidates.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        candidates
    }

    /// Baseline candidate for static range (no rebalances).
    pub fn optimize_static_range<O: ObjectiveFunction>(
        &self,
        config: &OptimizationConfig,
        range_width: Decimal,
        objective: &O,
    ) -> StaticRangeCandidate {
        let time_in_range = estimate_time_in_range(range_width, config.volatility, 0);
        let fees = estimate_base_fees(config, range_width, time_in_range);
        let il = estimate_base_il(range_width, config.volatility);
        let sim_result = create_sim_result(&(fees, il, 0));
        let score = objective.evaluate(&sim_result);
        StaticRangeCandidate {
            expected_fees: fees,
            expected_il: il,
            expected_rebalances: 0,
            score,
        }
    }

    /// Optimizes retouch-shift style parameters.
    pub fn optimize_retouch_shift<O: ObjectiveFunction>(
        &self,
        config: &OptimizationConfig,
        range_width: Decimal,
        objective: &O,
    ) -> Vec<RetouchShiftCandidate> {
        let mut candidates = Vec::new();
        for &price_threshold in &self.price_thresholds {
            if !self.constraints.is_valid_price_threshold(price_threshold) {
                continue;
            }
            for &cooldown_steps in &self.retouch_cooldowns {
                let params = RetouchShiftParams {
                    price_threshold,
                    cooldown_steps,
                };
                let result = self.estimate_retouch_shift_performance(&params, config, range_width);
                let sim_result = create_sim_result(&result);
                let score = objective.evaluate(&sim_result);
                candidates.push(RetouchShiftCandidate {
                    params,
                    expected_fees: result.0,
                    expected_il: result.1,
                    expected_rebalances: result.2,
                    score,
                });
            }
        }
        candidates.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        candidates
    }

    /// Estimates performance for threshold strategy.
    fn estimate_threshold_performance(
        &self,
        params: &ThresholdParams,
        config: &OptimizationConfig,
        range_width: Decimal,
    ) -> (Decimal, Decimal, u32) {
        // Estimate number of rebalances based on volatility and thresholds
        let vol_dec = Decimal::from_f64(config.volatility).unwrap_or(Decimal::ZERO);
        let steps = config.simulation_steps as u32;

        // Higher volatility + lower threshold = more rebalances
        let rebalance_rate = vol_dec / params.price_threshold;
        let expected_rebalances = (Decimal::from(steps) * rebalance_rate / Decimal::from(10))
            .to_u32()
            .unwrap_or(0)
            .min(steps);

        // Fees: more rebalances = more time in optimal range = more fees
        let time_in_range =
            estimate_time_in_range(range_width, config.volatility, expected_rebalances);
        let base_fees = estimate_base_fees(config, range_width, time_in_range);

        // IL: rebalancing reduces IL but costs tx fees
        let base_il = estimate_base_il(range_width, config.volatility);
        let il_reduction = Decimal::from(expected_rebalances) * Decimal::from_f64(0.01).unwrap();
        let effective_il = (base_il - il_reduction).max(Decimal::ZERO);

        // Subtract tx costs
        let tx_costs = Decimal::from(expected_rebalances) * config.tx_cost;
        let net_fees = base_fees - tx_costs;

        (net_fees, effective_il, expected_rebalances)
    }

    /// Estimates performance for periodic strategy.
    fn estimate_periodic_performance(
        &self,
        params: &PeriodicParams,
        config: &OptimizationConfig,
        range_width: Decimal,
    ) -> (Decimal, Decimal, u32) {
        let steps = config.simulation_steps as u32;
        let expected_rebalances = steps / params.interval as u32;

        let time_in_range =
            estimate_time_in_range(range_width, config.volatility, expected_rebalances);
        let base_fees = estimate_base_fees(config, range_width, time_in_range);

        let base_il = estimate_base_il(range_width, config.volatility);
        let il_reduction = Decimal::from(expected_rebalances) * Decimal::from_f64(0.008).unwrap();
        let effective_il = (base_il - il_reduction).max(Decimal::ZERO);

        let tx_costs = Decimal::from(expected_rebalances) * config.tx_cost;
        let net_fees = base_fees - tx_costs;

        (net_fees, effective_il, expected_rebalances)
    }

    /// Estimates performance for IL limit strategy.
    fn estimate_il_limit_performance(
        &self,
        params: &ILLimitParams,
        config: &OptimizationConfig,
        range_width: Decimal,
    ) -> (Decimal, Decimal, u32) {
        let vol_dec = Decimal::from_f64(config.volatility).unwrap_or(Decimal::ZERO);
        let steps = config.simulation_steps as u32;

        // Rebalances triggered when IL exceeds threshold
        let il_trigger_rate = vol_dec * vol_dec / params.max_il;
        let expected_rebalances = (Decimal::from(steps) * il_trigger_rate / Decimal::from(5))
            .to_u32()
            .unwrap_or(0)
            .min(steps);

        let time_in_range =
            estimate_time_in_range(range_width, config.volatility, expected_rebalances);
        let base_fees = estimate_base_fees(config, range_width, time_in_range);

        // IL is capped at max_il due to rebalancing
        let effective_il = params.max_il;

        let tx_costs = Decimal::from(expected_rebalances) * config.tx_cost;
        let net_fees = base_fees - tx_costs;

        (net_fees, effective_il, expected_rebalances)
    }

    /// Estimates performance for retouch-shift style strategy.
    fn estimate_retouch_shift_performance(
        &self,
        params: &RetouchShiftParams,
        config: &OptimizationConfig,
        range_width: Decimal,
    ) -> (Decimal, Decimal, u32) {
        let vol_dec = Decimal::from_f64(config.volatility).unwrap_or(Decimal::ZERO);
        let steps = config.simulation_steps as u32;
        let trigger_rate = if params.price_threshold > Decimal::ZERO {
            vol_dec / params.price_threshold
        } else {
            Decimal::ZERO
        };
        let cooldown_penalty = Decimal::from(params.cooldown_steps.max(1));
        let expected_rebalances = (Decimal::from(steps) * trigger_rate / cooldown_penalty)
            .to_u32()
            .unwrap_or(0)
            .min(steps);

        let time_in_range =
            estimate_time_in_range(range_width, config.volatility, expected_rebalances);
        let base_fees = estimate_base_fees(config, range_width, time_in_range);
        let base_il = estimate_base_il(range_width, config.volatility);
        let il_reduction = Decimal::from(expected_rebalances) * Decimal::from_f64(0.006).unwrap();
        let effective_il = (base_il - il_reduction).max(Decimal::ZERO);
        let tx_costs = Decimal::from(expected_rebalances) * config.tx_cost;
        let net_fees = base_fees - tx_costs;
        (net_fees, effective_il, expected_rebalances)
    }
}

/// Candidate result for threshold optimization.
#[derive(Debug, Clone)]
pub struct ThresholdCandidate {
    /// The parameters.
    pub params: ThresholdParams,
    /// Expected fees.
    pub expected_fees: Decimal,
    /// Expected IL.
    pub expected_il: Decimal,
    /// Expected number of rebalances.
    pub expected_rebalances: u32,
    /// Objective score.
    pub score: Decimal,
}

/// Candidate result for periodic optimization.
#[derive(Debug, Clone)]
pub struct PeriodicCandidate {
    /// The parameters.
    pub params: PeriodicParams,
    /// Expected fees.
    pub expected_fees: Decimal,
    /// Expected IL.
    pub expected_il: Decimal,
    /// Expected number of rebalances.
    pub expected_rebalances: u32,
    /// Objective score.
    pub score: Decimal,
}

/// Candidate result for IL limit optimization.
#[derive(Debug, Clone)]
pub struct ILLimitCandidate {
    /// The parameters.
    pub params: ILLimitParams,
    /// Expected fees.
    pub expected_fees: Decimal,
    /// Expected IL.
    pub expected_il: Decimal,
    /// Expected number of rebalances.
    pub expected_rebalances: u32,
    /// Objective score.
    pub score: Decimal,
}

/// Candidate result for static-range baseline.
#[derive(Debug, Clone)]
pub struct StaticRangeCandidate {
    /// Expected fees.
    pub expected_fees: Decimal,
    /// Expected IL.
    pub expected_il: Decimal,
    /// Expected number of rebalances.
    pub expected_rebalances: u32,
    /// Objective score.
    pub score: Decimal,
}

/// Parameters for a retouch-shift strategy.
#[derive(Debug, Clone)]
pub struct RetouchShiftParams {
    /// Price threshold that arms a retouch.
    pub price_threshold: Decimal,
    /// Minimum cooldown between retouches (in steps).
    pub cooldown_steps: u64,
}

/// Candidate result for retouch-shift optimization.
#[derive(Debug, Clone)]
pub struct RetouchShiftCandidate {
    /// The parameters.
    pub params: RetouchShiftParams,
    /// Expected fees.
    pub expected_fees: Decimal,
    /// Expected IL.
    pub expected_il: Decimal,
    /// Expected number of rebalances.
    pub expected_rebalances: u32,
    /// Objective score.
    pub score: Decimal,
}

// Helper functions

fn estimate_time_in_range(width: Decimal, volatility: f64, rebalances: u32) -> Decimal {
    let vol_factor = Decimal::from_f64(1.0 - volatility.min(0.9)).unwrap_or(Decimal::ONE);
    let width_factor = width * Decimal::from(2);
    let rebalance_bonus = Decimal::from(rebalances) * Decimal::from_f64(0.5).unwrap();

    let base_time = Decimal::from(50);
    let time = base_time + (width_factor * Decimal::from(100) * vol_factor) + rebalance_bonus;

    time.min(Decimal::from(100))
}

fn estimate_base_fees(
    config: &OptimizationConfig,
    width: Decimal,
    time_in_range: Decimal,
) -> Decimal {
    let volume_factor = Decimal::from(config.pool_liquidity) / Decimal::from(1_000_000_000u64);
    let width_factor = if width.is_zero() {
        Decimal::ONE
    } else {
        Decimal::ONE / width
    };

    let steps = Decimal::from(config.simulation_steps);
    let base_fees = config.fee_rate * volume_factor * steps;

    base_fees * width_factor * time_in_range / Decimal::from(100)
}

fn estimate_base_il(width: Decimal, volatility: f64) -> Decimal {
    let vol_dec = Decimal::from_f64(volatility).unwrap_or(Decimal::ZERO);
    let vol_squared = vol_dec * vol_dec;

    if width.is_zero() {
        return vol_squared;
    }

    vol_squared / width / Decimal::from(10)
}

fn create_sim_result(
    result: &(Decimal, Decimal, u32),
) -> clmm_lp_domain::value_objects::simulation_result::SimulationResult {
    clmm_lp_domain::value_objects::simulation_result::SimulationResult {
        final_position_value: Decimal::ZERO,
        total_fees_earned: result.0,
        total_il: result.1,
        net_pnl: result.0 - result.1,
        max_drawdown: result.1,
        time_in_range_percentage: Decimal::ZERO,
        sharpe_ratio: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objective::MaximizeNetPnL;

    #[test]
    fn test_parameter_optimizer_creation() {
        let optimizer = ParameterOptimizer::new();
        assert!(!optimizer.price_thresholds.is_empty());
        assert!(!optimizer.il_thresholds.is_empty());
        assert!(!optimizer.intervals.is_empty());
    }

    #[test]
    fn test_optimize_threshold() {
        let optimizer = ParameterOptimizer::new();
        let config = OptimizationConfig::default();
        let range_width = Decimal::from_f64(0.10).unwrap();

        let candidates = optimizer.optimize_threshold(&config, range_width, &MaximizeNetPnL);

        assert!(!candidates.is_empty());
        // Should be sorted by score descending
        for i in 1..candidates.len() {
            assert!(candidates[i - 1].score >= candidates[i].score);
        }
    }

    #[test]
    fn test_optimize_periodic() {
        let optimizer = ParameterOptimizer::new();
        let config = OptimizationConfig::default();
        let range_width = Decimal::from_f64(0.10).unwrap();

        let candidates = optimizer.optimize_periodic(&config, range_width, &MaximizeNetPnL);

        assert!(!candidates.is_empty());
        for i in 1..candidates.len() {
            assert!(candidates[i - 1].score >= candidates[i].score);
        }
    }

    #[test]
    fn test_optimize_il_limit() {
        let optimizer = ParameterOptimizer::new();
        let config = OptimizationConfig::default();
        let range_width = Decimal::from_f64(0.10).unwrap();

        let candidates = optimizer.optimize_il_limit(&config, range_width, &MaximizeNetPnL);

        assert!(!candidates.is_empty());
        for i in 1..candidates.len() {
            assert!(candidates[i - 1].score >= candidates[i].score);
        }
    }

    #[test]
    fn test_optimize_static_range() {
        let optimizer = ParameterOptimizer::new();
        let config = OptimizationConfig::default();
        let range_width = Decimal::from_f64(0.10).unwrap();
        let candidate = optimizer.optimize_static_range(&config, range_width, &MaximizeNetPnL);
        assert_eq!(candidate.expected_rebalances, 0);
    }

    #[test]
    fn test_optimize_retouch_shift() {
        let optimizer = ParameterOptimizer::new();
        let config = OptimizationConfig::default();
        let range_width = Decimal::from_f64(0.10).unwrap();
        let candidates = optimizer.optimize_retouch_shift(&config, range_width, &MaximizeNetPnL);
        assert!(!candidates.is_empty());
        for i in 1..candidates.len() {
            assert!(candidates[i - 1].score >= candidates[i].score);
        }
    }

    #[test]
    fn test_threshold_params_default() {
        let params = ThresholdParams::default();
        assert_eq!(params.price_threshold, Decimal::from_f64(0.05).unwrap());
        assert!(params.rebalance_on_out_of_range);
    }

    #[test]
    fn test_periodic_params_default() {
        let params = PeriodicParams::default();
        assert_eq!(params.interval, 24);
    }

    #[test]
    fn test_il_limit_params_default() {
        let params = ILLimitParams::default();
        assert_eq!(params.max_il, Decimal::from_f64(0.05).unwrap());
        assert!(params.close_il.is_some());
    }
}
