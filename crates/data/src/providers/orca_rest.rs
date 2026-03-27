//! Orca Public REST API client (read-only).
//!
//! Base URL (Solana): `https://api.orca.so/v2/solana`

use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct OrcaRestClient {
    client: Client,
    base_url: String,
}

impl OrcaRestClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
        }
    }

    pub async fn list_pools(&self, q: ListPoolsQuery) -> Result<Paged<OrcaPoolSummary>> {
        let url = format!("{}/pools", self.base_url);
        let resp = self
            .client
            .get(&url)
            .query(&q)
            .send()
            .await
            .context("orca rest GET /pools")?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("orca rest /pools error: {} - {}", status, text);
        }
        resp.json::<Paged<OrcaPoolSummary>>()
            .await
            .context("orca rest /pools json")
    }

    pub async fn search_pools(&self, q: SearchPoolsQuery) -> Result<Paged<OrcaPoolSummary>> {
        let url = format!("{}/pools/search", self.base_url);
        let resp = self
            .client
            .get(&url)
            .query(&q)
            .send()
            .await
            .context("orca rest GET /pools/search")?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("orca rest /pools/search error: {} - {}", status, text);
        }
        resp.json::<Paged<OrcaPoolSummary>>()
            .await
            .context("orca rest /pools/search json")
    }

    pub async fn get_pool(&self, address: &str) -> Result<Wrapped<OrcaPoolSummary>> {
        let url = format!("{}/pools/{}", self.base_url, address.trim());
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .context("orca rest GET /pools/{address}")?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("orca rest /pools/{{address}} error: {} - {}", status, text);
        }
        resp.json::<Wrapped<OrcaPoolSummary>>()
            .await
            .context("orca rest /pools/{address} json")
    }

    pub async fn get_lock_info(&self, address: &str) -> Result<Vec<OrcaLockInfo>> {
        let url = format!("{}/lock/{}", self.base_url, address.trim());
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .context("orca rest GET /lock/{address}")?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("orca rest /lock/{{address}} error: {} - {}", status, text);
        }
        resp.json::<Vec<OrcaLockInfo>>()
            .await
            .context("orca rest /lock/{address} json")
    }

    pub async fn list_tokens(&self, q: ListTokensQuery) -> Result<Paged<OrcaTokenSummary>> {
        let url = format!("{}/tokens", self.base_url);
        let resp = self
            .client
            .get(&url)
            .query(&q)
            .send()
            .await
            .context("orca rest GET /tokens")?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("orca rest /tokens error: {} - {}", status, text);
        }
        resp.json::<Paged<OrcaTokenSummary>>()
            .await
            .context("orca rest /tokens json")
    }

    pub async fn search_tokens(&self, q: SearchTokensQuery) -> Result<Paged<OrcaTokenSummary>> {
        let url = format!("{}/tokens/search", self.base_url);
        let resp = self
            .client
            .get(&url)
            .query(&q)
            .send()
            .await
            .context("orca rest GET /tokens/search")?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("orca rest /tokens/search error: {} - {}", status, text);
        }
        resp.json::<Paged<OrcaTokenSummary>>()
            .await
            .context("orca rest /tokens/search json")
    }

    pub async fn get_token(&self, mint: &str) -> Result<Wrapped<OrcaTokenSummary>> {
        let url = format!("{}/tokens/{}", self.base_url, mint.trim());
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .context("orca rest GET /tokens/{mint}")?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("orca rest /tokens/{{mint}} error: {} - {}", status, text);
        }
        resp.json::<Wrapped<OrcaTokenSummary>>()
            .await
            .context("orca rest /tokens/{mint} json")
    }

    pub async fn get_protocol(&self) -> Result<Wrapped<OrcaProtocolStats>> {
        let url = format!("{}/protocol", self.base_url);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .context("orca rest GET /protocol")?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("orca rest /protocol error: {} - {}", status, text);
        }
        let text = resp.text().await.context("orca rest /protocol body")?;
        if let Ok(w) = serde_json::from_str::<Wrapped<OrcaProtocolStats>>(&text) {
            return Ok(w);
        }
        let data =
            serde_json::from_str::<OrcaProtocolStats>(&text).context("orca rest /protocol json")?;
        Ok(Wrapped {
            data,
            meta: PageMeta {
                next: None,
                previous: None,
            },
        })
    }
}

/// Query params for `/pools` (subset; extend as needed).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListPoolsQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "sortBy")]
    pub sort_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "sortDirection")]
    pub sort_direction: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "minTvl")]
    pub min_tvl: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats: Option<String>,
}

/// Query params for `/pools/search`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchPoolsQuery {
    pub q: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "minTvl")]
    pub min_tvl: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "minVolume")]
    pub min_volume: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "verifiedOnly")]
    pub verified_only: Option<bool>,
}

/// Query params for `/tokens`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListTokensQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
}

/// Query params for `/tokens/search`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchTokensQuery {
    pub q: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Paged<T> {
    pub data: Vec<T>,
    pub meta: PageMeta,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Wrapped<T> {
    pub data: T,
    pub meta: PageMeta,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PageMeta {
    pub next: Option<String>,
    pub previous: Option<String>,
}

/// Pool summary payload from Orca REST (`/pools`).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrcaPoolSummary {
    pub address: String,
    pub tick_spacing: u16,
    pub fee_rate: u16,
    pub liquidity: String,
    pub sqrt_price: String,
    pub tick_current_index: i32,
    pub token_mint_a: String,
    pub token_mint_b: String,
    pub price: String,
    pub tvl_usdc: String,
}

/// Lock info payload from Orca REST (`/lock/{address}`).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrcaLockInfo {
    pub name: String,
    pub locked_percentage: String,
}

/// Token payload from Orca REST (`/tokens*`).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrcaTokenSummary {
    #[serde(default)]
    pub mint: String,
    #[serde(default)]
    pub address: Option<String>,
    pub symbol: Option<String>,
    pub name: Option<String>,
    pub decimals: Option<u8>,
    pub verified: Option<bool>,
    pub price_usdc: Option<String>,
    #[serde(default)]
    pub metadata: Option<OrcaTokenMetadata>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrcaTokenMetadata {
    pub symbol: Option<String>,
    pub name: Option<String>,
}

/// Protocol payload from Orca REST (`/protocol`).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrcaProtocolStats {
    pub tvl_usdc: Option<String>,
    pub volume_24h_usdc: Option<String>,
    pub volume_7d_usdc: Option<String>,
    pub fees_24h_usdc: Option<String>,
    pub revenue_24h_usdc: Option<String>,
    pub tvl: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::Method::GET;
    use httpmock::MockServer;

    #[tokio::test]
    async fn list_pools_parses_wrapper_and_data() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/v2/solana/pools");
            then.status(200).json_body(serde_json::json!({
                "data": [{
                    "address": "POOL1",
                    "whirlpoolsConfig": "CFG",
                    "tickSpacing": 64,
                    "feeRate": 2000,
                    "protocolFeeRate": 300,
                    "liquidity": "123",
                    "sqrtPrice": "456",
                    "tickCurrentIndex": 7,
                    "tokenMintA": "MINTA",
                    "tokenMintB": "MINTB",
                    "price": "1.23",
                    "tvlUsdc": "999.0"
                }],
                "meta": { "next": null, "previous": null }
            }));
        });

        let c = OrcaRestClient::new(server.base_url() + "/v2/solana");
        let res = c
            .list_pools(ListPoolsQuery {
                size: Some(1),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(res.data.len(), 1);
        assert_eq!(res.data[0].address, "POOL1");
        assert_eq!(res.data[0].tick_spacing, 64);
    }

    #[tokio::test]
    async fn search_pools_hits_search_endpoint() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/v2/solana/pools/search");
            then.status(200).json_body(serde_json::json!({
                "data": [{
                    "address": "POOL2",
                    "tickSpacing": 8,
                    "feeRate": 300,
                    "liquidity": "1",
                    "sqrtPrice": "1",
                    "tickCurrentIndex": 0,
                    "tokenMintA": "A",
                    "tokenMintB": "B",
                    "price": "1.0",
                    "tvlUsdc": "10.0"
                }],
                "meta": { "next": null, "previous": null }
            }));
        });

        let c = OrcaRestClient::new(server.base_url() + "/v2/solana");
        let res = c
            .search_pools(SearchPoolsQuery {
                q: "SOL-USDC".to_string(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(res.data.len(), 1);
        assert_eq!(res.data[0].address, "POOL2");
    }

    #[tokio::test]
    async fn get_pool_parses_wrapped_data() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/v2/solana/pools/POOLX");
            then.status(200).json_body(serde_json::json!({
                "data": {
                    "address": "POOLX",
                    "tickSpacing": 64,
                    "feeRate": 300,
                    "liquidity": "1",
                    "sqrtPrice": "1",
                    "tickCurrentIndex": 0,
                    "tokenMintA": "A",
                    "tokenMintB": "B",
                    "price": "1.0",
                    "tvlUsdc": "10.0"
                },
                "meta": { "next": null, "previous": null }
            }));
        });

        let c = OrcaRestClient::new(server.base_url() + "/v2/solana");
        let w = c.get_pool("POOLX").await.unwrap();
        assert_eq!(w.data.address, "POOLX");
    }

    #[tokio::test]
    async fn get_lock_info_parses_array() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/v2/solana/lock/POOLX");
            then.status(200).json_body(serde_json::json!([
                { "name": "TestLock", "lockedPercentage": "45.5" }
            ]));
        });

        let c = OrcaRestClient::new(server.base_url() + "/v2/solana");
        let v = c.get_lock_info("POOLX").await.unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].name, "TestLock");
        assert_eq!(v[0].locked_percentage, "45.5");
    }

    #[tokio::test]
    async fn list_tokens_parses_payload() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/v2/solana/tokens");
            then.status(200).json_body(serde_json::json!({
                "data": [{
                    "mint": "MINT1",
                    "symbol": "AAA",
                    "name": "Token AAA",
                    "decimals": 6,
                    "verified": true,
                    "priceUsdc": "1.01"
                }],
                "meta": { "next": null, "previous": null }
            }));
        });

        let c = OrcaRestClient::new(server.base_url() + "/v2/solana");
        let v = c.list_tokens(ListTokensQuery::default()).await.unwrap();
        assert_eq!(v.data.len(), 1);
        assert_eq!(v.data[0].mint, "MINT1");
    }

    #[tokio::test]
    async fn search_tokens_hits_endpoint() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/v2/solana/tokens/search");
            then.status(200).json_body(serde_json::json!({
                "data": [{
                    "mint": "MINT2",
                    "symbol": "BBB"
                }],
                "meta": { "next": null, "previous": null }
            }));
        });

        let c = OrcaRestClient::new(server.base_url() + "/v2/solana");
        let v = c
            .search_tokens(SearchTokensQuery {
                q: "BBB".to_string(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(v.data.len(), 1);
        assert_eq!(v.data[0].mint, "MINT2");
    }

    #[tokio::test]
    async fn get_token_parses_wrapped_token() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/v2/solana/tokens/MINTX");
            then.status(200).json_body(serde_json::json!({
                "data": {
                    "mint": "MINTX",
                    "symbol": "XXX",
                    "priceUsdc": "0.5"
                },
                "meta": { "next": null, "previous": null }
            }));
        });

        let c = OrcaRestClient::new(server.base_url() + "/v2/solana");
        let v = c.get_token("MINTX").await.unwrap();
        assert_eq!(v.data.mint, "MINTX");
    }

    #[tokio::test]
    async fn get_protocol_parses_stats() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/v2/solana/protocol");
            then.status(200).json_body(serde_json::json!({
                "data": {
                    "tvlUsdc": "123456.7",
                    "volume24hUsdc": "1111.2",
                    "volume7dUsdc": "7777.8"
                },
                "meta": { "next": null, "previous": null }
            }));
        });

        let c = OrcaRestClient::new(server.base_url() + "/v2/solana");
        let v = c.get_protocol().await.unwrap();
        assert_eq!(v.data.tvl_usdc.as_deref(), Some("123456.7"));
    }
}
