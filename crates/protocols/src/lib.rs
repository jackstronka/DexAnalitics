//! Protocol adapters and interfaces.
//!
//! This crate provides adapters for interacting with Solana CLMM protocols:
//! - Orca Whirlpools
//! - Raydium CLMM
//! - Meteora DLMM (planned)

/// Prelude module for convenient imports.
pub mod prelude;

/// Event fetching and parsing.
pub mod events;
/// Orca protocol adapter.
pub mod orca;
/// Data parsers.
pub mod parsers;
/// Raydium protocol adapter.
pub mod raydium;
/// Meteora DLMM protocol adapter (bin-based).
pub mod meteora;
/// RPC provider with health checks and fallback.
pub mod rpc;
/// Solana client wrapper.
pub mod solana_client;

use anyhow::Result;
use async_trait::async_trait;
use clmm_lp_domain::entities::pool::Pool;

/// Trait for fetching pool data.
#[async_trait]
pub trait PoolFetcher {
    /// Fetches pool data by address.
    async fn fetch_pool(&self, pool_address: &str) -> Result<Pool>;
}
