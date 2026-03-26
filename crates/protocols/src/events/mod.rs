//! Event fetching and parsing for CLMM protocols.
//!
//! This module provides functionality to fetch and parse on-chain events
//! from CLMM protocol transactions.

mod fetcher;
pub mod meteora_swap_event;
mod parser;
pub mod raydium_swap_event;
mod types;
pub mod whirlpool_traded;

pub use fetcher::*;
pub use meteora_swap_event::{
    METEORA_SWAP_EVENT_DISCRIMINATOR, MeteoraDlmmSwapEvent, parse_meteora_swap_event_for_pool,
};
pub use parser::*;
pub use raydium_swap_event::{
    SWAP_EVENT_DISCRIMINATOR as RAYDIUM_SWAP_EVENT_DISCRIMINATOR, parse_raydium_swap_event_for_pool,
};
pub use types::*;
pub use whirlpool_traded::{
    TRADED_EVENT_DISCRIMINATOR, WhirlpoolTradedEvent, parse_traded_event_for_pool,
};
