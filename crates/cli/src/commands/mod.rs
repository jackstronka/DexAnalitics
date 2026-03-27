//! CLI command implementations.
//!
//! This module contains the implementation of all CLI commands,
//! separated into logical modules for maintainability.

pub mod analyze;
pub mod backtest;
pub mod backtest_optimize;
pub mod data;
pub mod optimize;
pub mod orca_bot;
pub mod orca_position;
pub mod orca_swap;
pub mod orca_wallet;
pub mod snapshot_price_path;
pub mod studio;

pub use analyze::run_analyze;
pub use backtest::run_backtest;
pub use data::run_data;
pub use optimize::run_optimize;
