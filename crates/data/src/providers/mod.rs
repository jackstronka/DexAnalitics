//! Market data and pool data providers.
//!
//! This module provides different data sources for historical price data,
//! including API providers and file-based providers, as well as
//! utilities for working with on-chain pool metadata.

mod birdeye;
/// CSV provider module for file-based data loading.
pub mod csv_provider;
/// Jupiter Price API provider.
pub mod jupiter;
mod mock;
pub mod pool_info;
pub mod dune;

pub use birdeye::BirdeyeProvider;
pub use csv_provider::CsvProvider;
pub use jupiter::JupiterProvider;
pub use mock::MockMarketDataProvider;
pub use dune::{DuneClient, TvlPoint, VolumePoint};
