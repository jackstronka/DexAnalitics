//! Domain models and logic for the CLMM Liquidity Provider.

/// Prelude module for convenient imports.
pub mod prelude;

pub mod entities;
/// Enumerations used across the domain.
pub mod enums;
/// Fee related structures and logic.
pub mod fees;
/// Mathematical functions and utilities.
pub mod math;
/// Metrics for analysis.
pub mod metrics;
/// Pool entities and logic.
pub mod pool;
/// Position entities and logic.
pub mod position;
/// Token entities and logic.
pub mod token;
pub mod value_objects;
/// JSON artifact from CLI `backtest-optimize` (`--optimize-result-json`).
pub mod optimize_result;
