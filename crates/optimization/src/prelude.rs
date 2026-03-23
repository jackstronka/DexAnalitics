//! Prelude module for convenient imports.
//!
//! This module re-exports the most commonly used types from the crate.
//!
//! # Example
//!
//! ```rust
//! use clmm_lp_optimization::prelude::*;
//! ```

// Constraints
pub use crate::constraints::{OptimizationConstraints, PositionConstraints, RebalanceConstraints};

// Objective functions
pub use crate::objective::{
    CompositeObjective, CompositeWeights, MaximizeFees, MaximizeNetPnL, MaximizeSharpeRatio,
    MaximizeTimeInRange, MinimizeIL, ObjectiveFunction, RiskAdjustedReturn,
};

// Optimizer
pub use crate::optimizer::{
    AnalyticalOptimizer, CandidateResult, GridSearchOptimizer, OptimizationConfig, Optimizer,
};

// Parameter optimizer
pub use crate::parameter_optimizer::{
    ILLimitCandidate, ILLimitParams, ParameterOptimizationResult, ParameterOptimizer,
    PeriodicCandidate, PeriodicParams, RetouchShiftCandidate, RetouchShiftParams,
    StaticRangeCandidate, ThresholdCandidate, ThresholdParams,
};

// Range optimizer
pub use crate::range_optimizer::RangeOptimizer;
