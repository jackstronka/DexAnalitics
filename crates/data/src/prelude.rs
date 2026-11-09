//! Prelude module for convenient imports.
//!
//! This module re-exports the most commonly used types from the crate.
//!
//! # Example
//!
//! ```rust
//! use clmm_lp_data::prelude::*;
//! ```

// Traits
pub use crate::MarketDataProvider;

// Cache
pub use crate::cache::{
    Cache, CacheEntry, CacheKeyBuilder, CachedProvider, FileCache, MemoryCache,
};

// Pool state
pub use crate::pool_state::{PoolStateHistory, PoolStateSnapshot};

// Providers
pub use crate::providers::csv_provider::write_candles_to_csv;
pub use crate::providers::{
    BirdeyeProvider, CsvProvider, JupiterProvider, JupiterTriggerClient, MockMarketDataProvider,
};

// Database repositories
pub use crate::repositories::{
    Database, OptimizationRecord, PoolRecord, PoolRepository, PriceRecord, PriceRepository,
    SimulationRecord, SimulationRepository, SimulationResultRecord,
};

// In-memory repository
pub use crate::repository::{SimulationDataRepository, SimulationDataRepositoryBuilder};

// Time series
pub use crate::timeseries::{OhlcvCandle, TimeSeries};
