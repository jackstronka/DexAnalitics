//! Strategy execution for automated position management.
//!
//! Provides automated strategy execution including:
//! - Decision engine
//! - Rebalancing logic
//! - Position lifecycle management

mod decision;
mod executor;
mod pending_open;
mod rebalance;
mod types;

pub use decision::*;
pub use executor::*;
pub use pending_open::{PendingOpenItem, PendingOpenStore};
pub use rebalance::*;
pub use types::Decision;
