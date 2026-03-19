use anyhow::Result;
use reqwest::Client;
use rust_decimal::Decimal;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use crate::swaps::SwapEvent;

/// Basic TVL point from Dune.
#[derive(Debug, Clone)]
pub struct TvlPoint {
    pub date: String,
    pub tvl_usd: Decimal,
}

/// Basic volume/fees point from Dune.
#[derive(Debug, Clone)]
pub struct VolumePoint {
    pub date: String,
    pub volume_usd: Decimal,
    pub fees_usd: Decimal,
}

/// Lightweight client for Dune HTTP API.
///
/// It expects the following env vars to be set:
/// - `DUNE_API_KEY`
/// - `DUNE_TVL_QUERY_ID`
/// - `DUNE_VOLUME_QUERY_ID`
pub struct DuneClient {
    client: Client,
    api_key: String,
    tvl_query_id: String,
    volume_query_id: String,
}

impl DuneClient {
    pub fn from_env() -> Result<Self> {
        let api_key = std::env::var("DUNE_API_KEY")?;
        let tvl_query_id = std::env::var("DUNE_TVL_QUERY_ID")?;
        let volume_query_id = std::env::var("DUNE_VOLUME_QUERY_ID")?;

        Ok(Self {
            client: Client::new(),
            api_key,
            tvl_query_id,
            volume_query_id,
        })
    }

    /// Build a client that can only call `fetch_swaps` (no TVL/volume).
    /// Requires only `DUNE_API_KEY`. Use for backtest swap-based fees when
    /// TVL/volume are not needed or come from another source.
    pub fn from_env_swaps_only() -> Result<Self> {
        let api_key = std::env::var("DUNE_API_KEY")?;
        Ok(Self {
            client: Client::new(),
            api_key,
            tvl_query_id: String::new(),
            volume_query_id: String::new(),
        })
    }

    async fn fetch_rows_inner<T: for<'de> Deserialize<'de>>(
        &self,
        query_id: &str,
        skip_cache_read: bool,
    ) -> Result<Vec<T>> {
        /// Simple on-disk cache to avoid re-fetching identical Dune results.
        ///
        /// Cache layout:
        ///   data/dune-cache/{query_id}.json
        ///
        /// If the file exists and deserializes correctly, we use it instead of
        /// calling the HTTP API again. To refresh data manually, the user can
        /// delete the corresponding cache file.
        fn cache_path(query_id: &str) -> PathBuf {
            let mut path = PathBuf::from("data");
            path.push("dune-cache");
            path.push(format!("{query_id}.json"));
            path
        }

        #[derive(Deserialize)]
        struct DuneRows<T> {
            rows: Vec<T>,
        }

        #[derive(Deserialize)]
        struct DuneResult<T> {
            result: DuneRows<T>,
        }

        let path = cache_path(query_id);

        // Try cache first unless user explicitly disables it or forces refresh.
        let disable_cache = std::env::var("DUNE_DISABLE_CACHE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        if !skip_cache_read
            && !disable_cache
            && path.exists()
            && let Ok(bytes) = fs::read(&path)
            && let Ok(wrapper) = serde_json::from_slice::<DuneResult<T>>(&bytes)
        {
            return Ok(wrapper.result.rows);
        }

        let url = format!("https://api.dune.com/api/v1/query/{}/results", query_id);

        let resp = self
            .client
            .get(&url)
            .header("X-Dune-API-Key", &self.api_key)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if status.as_u16() == 402 {
                return Err(anyhow::anyhow!(
                    "Dune API 402 Payment Required: this request would exceed your per-request credit limit. \
                    The query may return too many rows. Options: (1) In Dune, duplicate the query and add a date filter (e.g. last 30 days), then use that query ID with --query-id. \
                    (2) Upgrade your plan at https://dune.com/settings/billing. \
                    Raw: {}",
                    text
                ));
            }
            return Err(anyhow::anyhow!("Dune API error: {} - {}", status, text));
        }

        let bytes = resp.bytes().await?;

        // Best-effort: cache raw response for future runs (avoids requiring T: Serialize).
        if !disable_cache {
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let _ = fs::write(&path, &bytes);
        }

        let wrapper: DuneResult<T> = serde_json::from_slice(&bytes)?;
        Ok(wrapper.result.rows)
    }

    async fn fetch_rows<T: for<'de> Deserialize<'de>>(&self, query_id: &str) -> Result<Vec<T>> {
        self.fetch_rows_inner::<T>(query_id, false).await
    }

    /// Fetch swap events for a given swaps query id (e.g. 6848259 for Orca).
    ///
    /// The underlying HTTP call is cached on disk by `fetch_rows` in
    /// `data/dune-cache/{query_id}.json`, so repeated calls with the same
    /// query id will reuse local data without consuming extra Dune credits.
    pub async fn fetch_swaps(&self, swaps_query_id: &str) -> Result<Vec<SwapEvent>> {
        self.fetch_rows(swaps_query_id).await
    }

    /// Fetch swap events from Dune API and write to cache, skipping any existing cache file.
    /// Use this to pre-fill or refresh the local cache (e.g. `dune-sync-swaps` command).
    /// Cache path: `data/dune-cache/{query_id}.json`.
    pub async fn fetch_swaps_force(&self, swaps_query_id: &str) -> Result<Vec<SwapEvent>> {
        self.fetch_rows_inner(swaps_query_id, true).await
    }

    /// Fetch daily TVL for all pools, caller filters by `pool_address`.
    pub async fn fetch_tvl(&self, pool_address: &str) -> Result<Vec<TvlPoint>> {
        if self.tvl_query_id.is_empty() {
            return Err(anyhow::anyhow!("DuneClient: TVL query ID not set (use from_env() for TVL/volume)"));
        }
        #[derive(Deserialize)]
        struct Row {
            date: String,
            pool_address: String,
            total_tvl_usd: Decimal,
        }

        let rows: Vec<Row> = self.fetch_rows(&self.tvl_query_id).await?;
        let mut out = Vec::new();

        for r in rows {
            if r.pool_address == pool_address {
                // Normalize date to YYYY-MM-DD so we can safely join
                let date = r.date.split_whitespace().next().unwrap_or(&r.date).to_string();

                out.push(TvlPoint {
                    date,
                    tvl_usd: r.total_tvl_usd,
                });
            }
        }

        out.sort_by(|a, b| a.date.cmp(&b.date));
        Ok(out)
    }

    /// Fetch daily volume and fees for all pools, caller filters by `pool_address`.
    pub async fn fetch_volume_fees(&self, pool_address: &str) -> Result<Vec<VolumePoint>> {
        if self.volume_query_id.is_empty() {
            return Err(anyhow::anyhow!("DuneClient: volume query ID not set (use from_env() for TVL/volume)"));
        }
        #[derive(Deserialize)]
        struct Row {
            trade_date: String,
            whirlpool_address: String,
            volume_usd_daily: Decimal,
            fees_usd_daily: Decimal,
        }

        let rows: Vec<Row> = self.fetch_rows(&self.volume_query_id).await?;
        let mut out = Vec::new();

        for r in rows {
            if r.whirlpool_address == pool_address {
                // Normalize date to YYYY-MM-DD so joins with TVL work
                let date = r.trade_date.split_whitespace().next().unwrap_or(&r.trade_date).to_string();

                out.push(VolumePoint {
                    date,
                    volume_usd: r.volume_usd_daily,
                    fees_usd: r.fees_usd_daily,
                });
            }
        }

        out.sort_by(|a, b| a.date.cmp(&b.date));
        Ok(out)
    }

    /// Fetch daily TVL and volume for a pool and return (date -> tvl_usd, date -> volume_usd).
    /// Use for backtest/optimize to avoid duplicating fetch and map-building in callers.
    pub async fn fetch_tvl_volume_maps(
        &self,
        pool_address: &str,
    ) -> Result<(HashMap<String, Decimal>, HashMap<String, Decimal>)> {
        let tvl_series = self.fetch_tvl(pool_address).await?;
        let vol_series = self.fetch_volume_fees(pool_address).await?;
        let tvl_map: HashMap<String, Decimal> = tvl_series.into_iter().map(|p| (p.date, p.tvl_usd)).collect();
        let vol_map: HashMap<String, Decimal> = vol_series.into_iter().map(|v| (v.date, v.volume_usd)).collect();
        Ok((tvl_map, vol_map))
    }
}

