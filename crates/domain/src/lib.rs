//! Domain models and logic for the Bociarz LP Strategy Lab (derived from CLMM Liquidity Provider).

/// Prelude module for convenient imports.
pub mod prelude;

/// AI / agent approval layer on top of `OptimizeResultFile`.
pub mod agent_decision;
pub mod entities;
/// Enumerations used across the domain.
pub mod enums;
/// Fee related structures and logic.
pub mod fees;
/// Mathematical functions and utilities.
pub mod math;
/// Metrics for analysis.
pub mod metrics;
/// JSON artifact from CLI `backtest-optimize` (`--optimize-result-json`).
pub mod optimize_result;
/// Pool entities and logic.
pub mod pool;
/// Position entities and logic.
pub mod position;
/// Position-level fee checkpoints for Tier3 mode.
pub mod position_fee_checkpoint;
/// Token entities and logic.
pub mod token;
pub mod value_objects;
