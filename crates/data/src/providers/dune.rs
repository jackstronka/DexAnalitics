use anyhow::Result;
use reqwest::Client;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
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

    async fn fetch_rows<T: for<'de> Deserialize<'de> + Serialize>(
        &self,
        query_id: &str,
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

        #[derive(Deserialize, Serialize)]
        struct DuneRows<T> {
            rows: Vec<T>,
        }

        #[derive(Deserialize, Serialize)]
        struct DuneResult<T> {
            result: DuneRows<T>,
        }

        let path = cache_path(query_id);

        // Try cache first unless user explicitly disables it via env.
        let disable_cache = std::env::var("DUNE_DISABLE_CACHE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        if !disable_cache
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
            return Err(anyhow::anyhow!("Dune API error: {} - {}", status, text));
        }

        let wrapper: DuneResult<T> = resp.json().await?;

        // Best-effort: cache to disk for future runs.
        if !disable_cache {
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let _ = fs::write(&path, serde_json::to_vec_pretty(&wrapper).unwrap_or_default());
        }

        Ok(wrapper.result.rows)
    }

    /// Fetch swap events for a given swaps query id (e.g. 6848259 for Orca).
    ///
    /// The underlying HTTP call is cached on disk by `fetch_rows` in
    /// `data/dune-cache/{query_id}.json`, so repeated calls with the same
    /// query id will reuse local data without consuming extra Dune credits.
    pub async fn fetch_swaps(&self, swaps_query_id: &str) -> Result<Vec<SwapEvent>> {
        self.fetch_rows(swaps_query_id).await
    }

    /// Fetch daily TVL for all pools, caller filters by `pool_address`.
    pub async fn fetch_tvl(&self, pool_address: &str) -> Result<Vec<TvlPoint>> {
        #[derive(Deserialize, Serialize)]
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
        #[derive(Deserialize, Serialize)]
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

