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
use serde::Deserialize;

#[derive(Deserialize, Debug)]
struct BirdeyeOhlcvResponse {
    data: BirdeyeData,
    success: bool,
}

#[derive(Deserialize, Debug)]
struct BirdeyeData {
    items: Vec<BirdeyeCandle>,
}

#[derive(Deserialize, Debug)]
struct BirdeyeCandle {
    o: f64,
    h: f64,
    l: f64,
    c: f64,
    v: f64,
    #[serde(rename = "unix_time")]
    unix_time: u64,
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

        let url = format!(
            "https://public-api.birdeye.so/defi/v3/ohlcv?address={}&type={}&time_from={}&time_to={}&currency=usd&mode=range",
            token.mint_address, resolution_str, start_time, end_time
        );

        // Simple retry with backoff for rate limiting (429)
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
                // Too many requests - wait a bit and retry
                let delay_ms = 500 * attempts; // 0.5s, 1s
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

            return Ok(data.data.items);
        }
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
