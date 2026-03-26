//! DefiLlama Yields API provider (pool discovery + historical TVL).

use anyhow::Result;
use reqwest::Client;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
pub struct DefiLlamaYieldPool {
    /// Unique pool id (UUID) used by the yields API.
    pub pool: String,
    pub chain: String,
    pub project: String,
    pub symbol: String,
    #[serde(default, rename = "tvlUsd")]
    pub tvl_usd: f64,
}

#[derive(Debug, Deserialize)]
struct PoolsResponse {
    #[allow(dead_code)]
    status: Option<String>,
    data: Vec<DefiLlamaYieldPool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefiLlamaChartPoint {
    /// Timestamp (ISO8601), e.g. "2023-02-14T23:01:48.477Z".
    #[serde(default)]
    pub timestamp: String,
    /// TVL in USD at that timestamp.
    #[serde(default, rename = "tvlUsd")]
    pub tvl_usd: f64,
    /// APY value at that timestamp (not used for now).
    #[serde(default)]
    pub apy: f64,
}

#[derive(Debug, Deserialize)]
struct ChartResponse {
    #[allow(dead_code)]
    status: Option<String>,
    data: Vec<DefiLlamaChartPoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyTvlPoint {
    /// Date in YYYY-MM-DD (UTC).
    pub date: String,
    pub tvl_usd: Decimal,
}

/// DefiLlama client for the public yields API.
pub struct DefiLlamaClient {
    client: Client,
}

impl DefiLlamaClient {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    fn cache_dir() -> PathBuf {
        let mut p = PathBuf::from("data");
        p.push("defillama-cache");
        p
    }

    fn cache_path(name: &str) -> PathBuf {
        let mut p = Self::cache_dir();
        p.push(name);
        p
    }

    fn cache_disabled() -> bool {
        std::env::var("DEFILLAMA_DISABLE_CACHE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
    }

    /// List all yield pools from DefiLlama. Use for discovery.
    pub async fn list_pools(&self) -> Result<Vec<DefiLlamaYieldPool>> {
        let url = "https://yields.llama.fi/pools";
        let resp = self.client.get(url).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "DefiLlama pools error: {} - {}",
                status,
                text
            ));
        }
        let parsed: PoolsResponse = resp.json().await?;
        Ok(parsed.data)
    }

    /// Fetch historical chart points for a given DefiLlama yield pool id.
    /// Cached to `data/defillama-cache/chart_{pool_id}.json`.
    pub async fn fetch_pool_chart(&self, pool_id: &str) -> Result<Vec<DefiLlamaChartPoint>> {
        let cache_name = format!("chart_{}.json", pool_id);
        let path = Self::cache_path(&cache_name);
        let disable_cache = Self::cache_disabled();
        if !disable_cache && path.exists() {
            if let Ok(bytes) = fs::read(&path) {
                if let Ok(points) = serde_json::from_slice::<Vec<DefiLlamaChartPoint>>(&bytes) {
                    return Ok(points);
                }
            }
        }

        let url = format!("https://yields.llama.fi/chart/{}", pool_id);
        let resp = self.client.get(&url).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "DefiLlama chart error: {} - {}",
                status,
                text
            ));
        }
        let parsed: ChartResponse = resp.json().await?;
        let points: Vec<DefiLlamaChartPoint> = parsed.data;

        if !disable_cache {
            let _ = fs::create_dir_all(Self::cache_dir());
            let _ = fs::write(
                &path,
                serde_json::to_vec_pretty(&points).unwrap_or_default(),
            );
        }

        Ok(points)
    }

    /// Convert chart points into daily TVL points (UTC), taking the last point per day.
    pub fn chart_to_daily(points: &[DefiLlamaChartPoint]) -> Vec<DailyTvlPoint> {
        use chrono::{DateTime, Utc};
        use std::collections::BTreeMap;

        let mut by_day: BTreeMap<String, Decimal> = BTreeMap::new();
        for p in points {
            let dt: DateTime<Utc> = match DateTime::parse_from_rfc3339(p.timestamp.trim()) {
                Ok(d) => d.with_timezone(&Utc),
                Err(_) => continue,
            };
            let day = dt.format("%Y-%m-%d").to_string();
            let tvl = Decimal::from_f64_retain(p.tvl_usd).unwrap_or(Decimal::ZERO);
            // Keep the last-seen value for the day (points are typically ordered).
            by_day.insert(day, tvl);
        }

        by_day
            .into_iter()
            .map(|(date, tvl_usd)| DailyTvlPoint { date, tvl_usd })
            .collect()
    }

    /// Fetch daily TVL points (UTC) for a pool id, cached to `daily_tvl_{pool_id}.json`.
    pub async fn fetch_daily_tvl(&self, pool_id: &str) -> Result<Vec<DailyTvlPoint>> {
        let cache_name = format!("daily_tvl_{}.json", pool_id);
        let path = Self::cache_path(&cache_name);
        let disable_cache = Self::cache_disabled();

        if !disable_cache && path.exists() {
            if let Ok(bytes) = fs::read(&path) {
                if let Ok(points) = serde_json::from_slice::<Vec<DailyTvlPoint>>(&bytes) {
                    return Ok(points);
                }
            }
        }

        let chart = self.fetch_pool_chart(pool_id).await?;
        let daily = Self::chart_to_daily(&chart);

        if !disable_cache {
            let _ = fs::create_dir_all(Self::cache_dir());
            let _ = fs::write(&path, serde_json::to_vec_pretty(&daily).unwrap_or_default());
        }
        Ok(daily)
    }
}
