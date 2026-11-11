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
pub mod dune;
/// Jupiter Price API provider.
pub mod jupiter;
/// Jupiter Trigger Order API V2 (auth + vault/deposit + price + DCA).
pub mod jupiter_trigger;
mod mock;
mod orca_rest;
pub mod pool_info;

pub use birdeye::BirdeyeProvider;
pub use csv_provider::CsvProvider;
pub use defillama::{DailyTvlPoint, DefiLlamaChartPoint, DefiLlamaClient, DefiLlamaYieldPool};
pub use dexscreener::{DexChain, DexPair, DexscreenerClient};
pub use dune::{DuneClient, TvlPoint, VolumePoint};
pub use jupiter::JupiterProvider;
pub use jupiter_trigger::{
    JupiterTriggerClient, OrderHistoryQuery, TriggerAuthTokens, TriggerChallenge,
    TriggerDcaHistoryItem, TriggerDcaHistoryResponse, TriggerDcaOrderCancelResponse,
    TriggerDcaOrderCreateRequest, TriggerDcaOrderTxResponse, TriggerDepositCraftRequest,
    TriggerDepositCraftResponse, TriggerOrderHistoryItem, TriggerOrderHistoryResponse,
    TriggerPriceOrderCancelResponse, TriggerPriceOrderConfirmCancelRequest,
    TriggerPriceOrderConfirmCancelResponse, TriggerPriceOrderCreateRequest,
    TriggerPriceOrderCreateResponse, TriggerPriceOrderPatchRequest, TriggerPriceOrderPatchResponse,
    TriggerVault, filter_open_buy_dips,
};
pub use mock::MockMarketDataProvider;
pub use orca_rest::{
    ListPoolsQuery as OrcaListPoolsQuery, ListTokensQuery as OrcaListTokensQuery, OrcaLockInfo,
    OrcaPoolStats, OrcaPoolSummary, OrcaProtocolStats, OrcaRestClient, OrcaTokenSummary,
    Paged as OrcaPaged, SearchPoolsQuery as OrcaSearchPoolsQuery,
    SearchTokensQuery as OrcaSearchTokensQuery, Wrapped as OrcaWrapped,
};
