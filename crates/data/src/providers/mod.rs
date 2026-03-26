//! Market data and pool data providers.
//!
//! This module provides different data sources for historical price data,
//! including API providers and file-based providers, as well as
//! utilities for working with on-chain pool metadata.

mod birdeye;
/// CSV provider module for file-based data loading.
pub mod csv_provider;
mod defillama;
mod dexscreener;
mod orca_rest;
pub mod dune;
/// Jupiter Price API provider.
pub mod jupiter;
mod mock;
pub mod pool_info;

pub use birdeye::BirdeyeProvider;
pub use csv_provider::CsvProvider;
pub use defillama::{DailyTvlPoint, DefiLlamaChartPoint, DefiLlamaClient, DefiLlamaYieldPool};
pub use dexscreener::{DexChain, DexPair, DexscreenerClient};
pub use orca_rest::{
    ListPoolsQuery as OrcaListPoolsQuery, ListTokensQuery as OrcaListTokensQuery, OrcaLockInfo,
    OrcaPoolSummary, OrcaProtocolStats, OrcaRestClient, OrcaTokenSummary, Paged as OrcaPaged,
    SearchPoolsQuery as OrcaSearchPoolsQuery, SearchTokensQuery as OrcaSearchTokensQuery,
    Wrapped as OrcaWrapped,
};
pub use dune::{DuneClient, TvlPoint, VolumePoint};
pub use jupiter::JupiterProvider;
pub use mock::MockMarketDataProvider;
