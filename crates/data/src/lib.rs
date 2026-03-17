//! Data ingestion and storage.

/// Prelude module for convenient imports.
pub mod prelude;

/// Caching layer for market data.
pub mod cache;
/// Historical pool state structures.
pub mod pool_state;
/// Data providers.
pub mod providers;
pub mod swaps;
/// Database repositories.
pub mod repositories;
/// In-memory data repository for simulation.
pub mod repository;
/// Time series data structures.
pub mod timeseries;

use anyhow::Result;
use async_trait::async_trait;
use clmm_lp_domain::entities::price_candle::PriceCandle;
use clmm_lp_domain::entities::token::Token;

/// Trait for providing market data.
#[async_trait]
pub trait MarketDataProvider {
    /// Fetches price history for a token pair.
    async fn get_price_history(
        &self,
        token_a: &Token,
        token_b: &Token,
        start_time: u64,
        end_time: u64,
        resolution: u64, // seconds
    ) -> Result<Vec<PriceCandle>>;
}
