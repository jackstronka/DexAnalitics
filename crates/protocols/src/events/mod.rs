//! Event fetching and parsing for CLMM protocols.
//!
//! This module provides functionality to fetch and parse on-chain events
//! from CLMM protocol transactions.

mod fetcher;
mod parser;
mod types;
pub mod whirlpool_traded;

pub use fetcher::*;
pub use parser::*;
pub use types::*;
pub use whirlpool_traded::{parse_traded_event_for_pool, WhirlpoolTradedEvent, TRADED_EVENT_DISCRIMINATOR};
