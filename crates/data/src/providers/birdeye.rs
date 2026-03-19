//! Birdeye API provider for market data.

use crate::MarketDataProvider;
use anyhow::Result;
use async_trait::async_trait;
use clmm_lp_domain::entities::price_candle::PriceCandle;
use clmm_lp_domain::entities::token::Token;
use clmm_lp_domain::value_objects::{amount::Amount, price::Price};
use reqwest::Client;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Deserialize, Debug)]
struct BirdeyeOhlcvResponse {
    data: BirdeyeData,
    success: bool,
}

#[derive(Deserialize, Debug)]
struct BirdeyeData {
    items: Vec<BirdeyeCandle>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
struct BirdeyeCandle {
    o: f64,
    h: f64,
    l: f64,
    c: f64,
    v: f64,
    #[serde(rename = "unix_time")]
    unix_time: u64,
}

/// Incremental on-disk cache for Birdeye OHLCV.
///
/// Stored as one file per (mint, resolution), and extended over time as new ranges are requested.
#[derive(Deserialize, Serialize, Debug)]
struct BirdeyeOhlcvCacheFile {
    mint: String,
    resolution: String,
    start_time: u64,
    end_time: u64,
    items: Vec<BirdeyeCandle>,
}

/// Provider for Birdeye API.
pub struct BirdeyeProvider {
    /// The HTTP client.
    pub client: Client,
    /// The API key.
    pub api_key: String,
}

impl BirdeyeProvider {
    /// Creates a new BirdeyeProvider.
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
        }
    }

    fn map_resolution(&self, seconds: u64) -> &'static str {
        match seconds {
            1 => "1s",
            15 => "15s",
            30 => "30s",
            60 => "1m",
            180 => "3m",
            300 => "5m",
            900 => "15m",
            1800 => "30m",
            3600 => "1H",
            7200 => "2H",
            14400 => "4H",
            21600 => "6H",
            28800 => "8H",
            43200 => "12H",
            86400 => "1D",
            259200 => "3D",
            604800 => "1W",
            2_592_000 => "1M",
            _ => "1H", // Default fallback
        }
    }

    async fn fetch_usd_candles(
        &self,
        token: &Token,
        start_time: u64,
        end_time: u64,
        resolution: u64,
    ) -> Result<Vec<BirdeyeCandle>> {
        let resolution_str = self.map_resolution(resolution);

        // Normalize range to resolution boundaries so cache keys are stable.
        let req_start = start_time - (start_time % resolution);
        let req_end = end_time - (end_time % resolution);

        /// Incremental cache path.
        ///
        /// Cache layout:
        ///   data/birdeye-cache/ohlcv_{mint}_{type}_usd.json
        fn cache_path(mint: &str, resolution_str: &str) -> PathBuf {
            let mut path = PathBuf::from("data");
            path.push("birdeye-cache");
            path.push(format!("ohlcv_{}_{}_usd.json", mint, resolution_str));
            path
        }

        let path = cache_path(&token.mint_address, resolution_str);

        // Try cache first unless user explicitly disables it via env.
        let disable_cache = std::env::var("BIRDEYE_DISABLE_CACHE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        let mut cache: Option<BirdeyeOhlcvCacheFile> = None;
        if !disable_cache
            && path.exists()
            && let Ok(bytes) = fs::read(&path)
            && let Ok(c) = serde_json::from_slice::<BirdeyeOhlcvCacheFile>(&bytes)
            && c.mint == token.mint_address
            && c.resolution == resolution_str
        {
            cache = Some(c);
        }

        // Fetch a range from the API with retry/backoff (no cache).
        // NOTE: Birdeye may cap the number of candles returned for a single "range" call.
        // To guarantee full coverage (especially for 5m/1m), we fetch in time chunks.
        let fetch_range_once = |from: u64, to: u64| async move {
            let url = format!(
                "https://public-api.birdeye.so/defi/v3/ohlcv?address={}&type={}&time_from={}&time_to={}&currency=usd&mode=range",
                token.mint_address, resolution_str, from, to
            );
            let mut attempts = 0u32;
            loop {
                attempts += 1;
                let resp = self
                    .client
                    .get(&url)
                    .header("X-API-KEY", &self.api_key)
                    .header("x-chain", "solana")
                    .header("accept", "application/json")
                    .send()
                    .await?;

                if resp.status().as_u16() == 429 && attempts < 3 {
                    let delay_ms = 500 * attempts;
                    tracing::warn!(
                        "Birdeye API returned 429 Too Many Requests (attempt {}). Retrying in {} ms...",
                        attempts,
                        delay_ms
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(delay_ms as u64)).await;
                    continue;
                }

                if !resp.status().is_success() {
                    let status = resp.status();
                    let text = resp.text().await?;
                    return Err(anyhow::anyhow!("Birdeye API error: {} - {}", status, text));
                }

                let data: BirdeyeOhlcvResponse = resp.json().await?;
                if !data.success {
                    return Err(anyhow::anyhow!("Birdeye API returned success=false"));
                }
                return Ok::<Vec<BirdeyeCandle>, anyhow::Error>(data.data.items);
            }
        };

        let fetch_range = |from: u64, to: u64| async move {
            if from >= to {
                return Ok::<Vec<BirdeyeCandle>, anyhow::Error>(Vec::new());
            }
            // Conservative chunk size to stay below any server-side item cap.
            // (e.g. 5m: 900 candles ~= 3.1 days; 1m: 900 candles ~= 15h)
            let max_candles_per_call: u64 = 900;
            let chunk_span = resolution.saturating_mul(max_candles_per_call).max(resolution);

            let mut cur = from;
            let mut all: Vec<BirdeyeCandle> = Vec::new();
            while cur < to {
                let next = (cur.saturating_add(chunk_span)).min(to);
                let mut part = fetch_range_once(cur, next).await?;
                all.append(&mut part);
                // Avoid infinite loops if API returns empty for a chunk.
                cur = next.max(cur.saturating_add(resolution));
            }
            Ok::<Vec<BirdeyeCandle>, anyhow::Error>(all)
        };

        let (mut cached_start, mut cached_end, mut items): (u64, u64, Vec<BirdeyeCandle>) = if let Some(c) = cache {
            (c.start_time, c.end_time, c.items)
        } else {
            (u64::MAX, 0, Vec::new())
        };

        let mut fetched_any = false;
        if items.is_empty() {
            tracing::info!(
                "Birdeye OHLCV cache miss: {} ({} {}..{})",
                token.mint_address,
                resolution_str,
                req_start,
                req_end
            );
            let mut new_items = fetch_range(req_start, req_end).await?;
            new_items.sort_by_key(|c| c.unix_time);
            items = new_items;
            cached_start = items.first().map(|c| c.unix_time).unwrap_or(req_start);
            cached_end = items.last().map(|c| c.unix_time).unwrap_or(req_end);
            fetched_any = true;
        } else {
            // Extend backwards if needed.
            if req_start < cached_start {
                tracing::info!(
                    "Birdeye OHLCV cache partial (missing before): {} ({} {}..{})",
                    token.mint_address,
                    resolution_str,
                    req_start,
                    cached_start
                );
                let mut before = fetch_range(req_start, cached_start).await?;
                items.append(&mut before);
                fetched_any = true;
            }
            // Extend forwards if needed.
            if req_end > cached_end {
                tracing::info!(
                    "Birdeye OHLCV cache partial (missing after): {} ({} {}..{})",
                    token.mint_address,
                    resolution_str,
                    cached_end,
                    req_end
                );
                let mut after = fetch_range(cached_end, req_end).await?;
                items.append(&mut after);
                fetched_any = true;
            }
        }

        // De-duplicate by unix_time and sort.
        if fetched_any {
            let mut by_time: std::collections::BTreeMap<u64, BirdeyeCandle> = std::collections::BTreeMap::new();
            for c in items.into_iter() {
                by_time.insert(c.unix_time, c);
            }
            items = by_time.into_values().collect();
            cached_start = items.first().map(|c| c.unix_time).unwrap_or(cached_start);
            cached_end = items.last().map(|c| c.unix_time).unwrap_or(cached_end);

            // Retention: keep only the last N days to prevent unbounded cache growth.
            // Default is 180 days, configurable via `BIRDEYE_CACHE_MAX_DAYS`.
            //
            // IMPORTANT: Never prune away data required for the current request range.
            let max_days_cfg: u64 = std::env::var("BIRDEYE_CACHE_MAX_DAYS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(180);
            if max_days_cfg > 0 && cached_end > 0 {
                // Prune relative to the newest cached data, but NEVER prune away anything
                // needed to satisfy the current request range.
                let retention_min_ts = cached_end.saturating_sub(max_days_cfg * 24 * 3600);
                let min_ts = retention_min_ts.min(req_start);
                let before_len = items.len();
                items.retain(|c| c.unix_time >= min_ts);
                if items.len() != before_len {
                    cached_start = items.first().map(|c| c.unix_time).unwrap_or(cached_start);
                    tracing::info!(
                        "Birdeye OHLCV cache pruned: {} kept {} days ({} -> {} items)",
                        token.mint_address,
                        max_days_cfg,
                        before_len,
                        items.len()
                    );
                }
            }

            if !disable_cache {
                if let Some(parent) = path.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                let wrapper = BirdeyeOhlcvCacheFile {
                    mint: token.mint_address.clone(),
                    resolution: resolution_str.to_string(),
                    start_time: cached_start,
                    end_time: cached_end,
                    items: items.clone(),
                };
                let _ = fs::write(&path, serde_json::to_vec_pretty(&wrapper).unwrap_or_default());
                tracing::info!(
                    "Birdeye OHLCV cache updated: {} -> {} ({}..{}, {} items)",
                    token.mint_address,
                    path.display(),
                    cached_start,
                    cached_end,
                    wrapper.items.len()
                );
            }
        } else {
            tracing::info!(
                "Birdeye OHLCV cache hit: {} ({} {}..{})",
                token.mint_address,
                resolution_str,
                req_start,
                req_end
            );
        }

        Ok(items
            .into_iter()
            .filter(|c| c.unix_time >= req_start && c.unix_time <= req_end)
            .collect())
    }

    /// Fetches price history for a cross pair (base/quote), e.g. whETH/SOL.
    pub async fn get_cross_pair_price_history(
        &self,
        base: &Token,
        quote: &Token,
        start_time: u64,
        end_time: u64,
        resolution: u64,
    ) -> Result<Vec<PriceCandle>> {
        use std::collections::HashMap;

        // Fetch base/USD and quote/USD candles
        let base_items = self
            .fetch_usd_candles(base, start_time, end_time, resolution)
            .await?;

        // Be gentle with the rate limit: small pause between requests
        tokio::time::sleep(std::time::Duration::from_millis(2000)).await;

        let quote_items = self
            .fetch_usd_candles(quote, start_time, end_time, resolution)
            .await?;

        // Map quote candles by timestamp for quick lookup
        let mut quote_by_time: HashMap<u64, &BirdeyeCandle> = HashMap::new();
        for c in &quote_items {
            quote_by_time.insert(c.unix_time, c);
        }

        let mut candles = Vec::new();

        for base_candle in base_items {
            if let Some(quote_candle) = quote_by_time.get(&base_candle.unix_time) {
                let base_close = Decimal::from_f64(base_candle.c).unwrap_or(Decimal::ZERO);
                let quote_close = Decimal::from_f64(quote_candle.c).unwrap_or(Decimal::ZERO);

                if base_close.is_zero() || quote_close.is_zero() {
                    continue;
                }

                let price_base_quote = base_close / quote_close;

                let vol_usd = Decimal::from_f64(base_candle.v).unwrap_or(Decimal::ZERO);
                let vol_token = if base_close.is_zero() {
                    Decimal::ZERO
                } else {
                    vol_usd / base_close
                };
                let vol_amount = Amount::from_decimal(vol_token, base.decimals);

                candles.push(PriceCandle {
                    token_a: base.clone(),
                    token_b: quote.clone(),
                    start_timestamp: base_candle.unix_time,
                    duration_seconds: resolution,
                    // For cross pair we primarily care about close price; reuse it for OHLC.
                    open: Price::new(price_base_quote),
                    high: Price::new(price_base_quote),
                    low: Price::new(price_base_quote),
                    close: Price::new(price_base_quote),
                    volume_token_a: vol_amount,
                });
            }
        }

        Ok(candles)
    }
}

#[async_trait]
impl MarketDataProvider for BirdeyeProvider {
    async fn get_price_history(
        &self,
        token_a: &Token,
        token_b: &Token,
        start_time: u64,
        end_time: u64,
        resolution: u64,
    ) -> Result<Vec<PriceCandle>> {
        let is_token_b_usd = token_b.symbol.to_uppercase().contains("USD");

        if !is_token_b_usd {
            tracing::warn!(
                "Cross-pair requested through MarketDataProvider; returning {}/USD prices. \
Consider using get_cross_pair_price_history for non-USD quotes.",
                token_a.symbol
            );
        }

        let items = self
            .fetch_usd_candles(token_a, start_time, end_time, resolution)
            .await?;

        let candles = items
            .into_iter()
            .map(|item| {
                let open = Decimal::from_f64(item.o).unwrap_or(Decimal::ZERO);
                let high = Decimal::from_f64(item.h).unwrap_or(Decimal::ZERO);
                let low = Decimal::from_f64(item.l).unwrap_or(Decimal::ZERO);
                let close = Decimal::from_f64(item.c).unwrap_or(Decimal::ZERO);

                let vol_usd = Decimal::from_f64(item.v).unwrap_or(Decimal::ZERO);
                let vol_token = if close.is_zero() {
                    Decimal::ZERO
                } else {
                    vol_usd / close
                };

                let vol_amount = Amount::from_decimal(vol_token, token_a.decimals);

                PriceCandle {
                    token_a: token_a.clone(),
                    token_b: token_b.clone(),
                    start_timestamp: item.unix_time,
                    duration_seconds: resolution,
                    open: Price::new(open),
                    high: Price::new(high),
                    low: Price::new(low),
                    close: Price::new(close),
                    volume_token_a: vol_amount,
                }
            })
            .collect();

        Ok(candles)
    }
}
