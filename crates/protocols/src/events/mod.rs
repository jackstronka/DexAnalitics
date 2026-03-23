//! Event fetching and parsing for CLMM protocols.
//!
//! This module provides functionality to fetch and parse on-chain events
//! from CLMM protocol transactions.

mod fetcher;
mod parser;
mod types;
pub mod meteora_swap_event;
pub mod raydium_swap_event;
pub mod whirlpool_traded;

pub use fetcher::*;
pub use parser::*;
pub use types::*;
pub use meteora_swap_event::{
    parse_meteora_swap_event_for_pool, MeteoraDlmmSwapEvent, METEORA_SWAP_EVENT_DISCRIMINATOR,
};
pub use raydium_swap_event::{parse_raydium_swap_event_for_pool, SWAP_EVENT_DISCRIMINATOR as RAYDIUM_SWAP_EVENT_DISCRIMINATOR};
pub use whirlpool_traded::{parse_traded_event_for_pool, WhirlpoolTradedEvent, TRADED_EVENT_DISCRIMINATOR};
