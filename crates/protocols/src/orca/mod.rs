//! Orca Whirlpool protocol adapter.
//!
//! This module provides functionality to interact with Orca Whirlpool pools:
//! - Read pool state
//! - Read position state
//! - Execute LP operations
//! - Calculate token amounts

/// Executor for on-chain operations.
pub mod executor;
/// In-range deposit sizing from a USD budget (token caps for Whirlpool).
pub mod deposit_quote;
/// Pool reader for on-chain state.
pub mod pool_reader;
/// Position reader for on-chain state.
pub mod position_reader;
/// Orca pool provider.
pub mod provider;
/// Tick array reader and fee-growth helpers.
pub mod tick_array;
/// Tick boundary fetcher.
pub mod tick_reader;
/// Orca whirlpool account structures.
pub mod whirlpool;
