//! Prelude module for convenient imports.
//!
//! This module re-exports the most commonly used types from the crate.
//!
//! # Example
//!
//! ```rust
//! use clmm_lp_domain::prelude::*;
//! ```

// Entities
pub use crate::entities::pool::Pool;
pub use crate::entities::position::{Position, PositionId};
pub use crate::entities::price_candle::PriceCandle;
pub use crate::entities::token::Token;

// Enums
pub use crate::enums::{OptimizationObjective, PoolType, PositionStatus, Protocol, TimeHorizon};

// Fees
pub use crate::fees::{FeeAccumulation, FeeTier};

// Math functions
pub use crate::math::concentrated_liquidity::{
    get_amount0_delta, get_amount1_delta, get_liquidity_for_amount0, get_liquidity_for_amount1,
};
pub use crate::math::constant_product::{calculate_k, calculate_out_amount, calculate_spot_price};
pub use crate::math::fee_math::{
    FeeTier as MathFeeTier, bps_to_decimal, calculate_effective_fee_rate, calculate_fee_amount,
    calculate_lp_fee_share, decimal_to_bps, estimate_position_fees_24h,
};
pub use crate::math::price_impact::{
    calculate_execution_price, calculate_slippage, estimate_max_swap_for_impact,
    estimate_price_impact_clmm, estimate_price_impact_constant_product,
};
pub use crate::math::price_tick::{price_to_tick, tick_to_price};

// Metrics
pub use crate::metrics::fees::{
    FeeProjectionModel, analyze_fee_sustainability, apr_to_apy, calculate_apy,
    calculate_breakeven_days, calculate_fee_efficiency, calculate_pool_fees,
    calculate_required_fee_rate, project_fees,
};
pub use crate::metrics::impermanent_loss::{
    calculate_il_concentrated, calculate_il_constant_product,
};
pub use crate::metrics::{APY, ImpermanentLoss, PnL};

// Value objects
pub use crate::position_fee_checkpoint::{
    CheckpointSource, PositionFeeCheckpoint, PositionTruthMode,
};
pub use crate::value_objects::amount::Amount;
pub use crate::value_objects::optimization_result::OptimizationResult;
pub use crate::value_objects::percentage::Percentage;
pub use crate::value_objects::price::Price;
pub use crate::value_objects::price_range::PriceRange;
pub use crate::value_objects::simulation_result::SimulationResult;
pub use crate::value_objects::{
    FeeEarnings, ImpermanentLossResult, PoolMetrics, RiskMetrics, VolatilityEstimate,
};

pub use crate::agent_decision::AgentDecision;
pub use crate::optimize_result::{
    OptimizeResultFile, OptimizeRetouchRepeat, OptimizeTrackerSummary, OptimizeWinner,
};
