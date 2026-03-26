//! Dexscreener API provider (multi-DEX: Orca/Raydium/Meteora).
//!
//! This is best used for:
//! - venue discovery (which DEX has the most liquidity/volume for a given pair)
//! - lightweight comparisons for rotation decisions
//! - cached snapshots of liquidity/volume (off-chain aggregated metrics)
//!
//! It is NOT a swap-level fee truth source.

use anyhow::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DexPairToken {
    pub address: String,
    pub symbol: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DexPairLiquidity {
    #[serde(default)]
    pub usd: f64,
    #[serde(default)]
    pub base: f64,
    #[serde(default)]
    pub quote: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DexPairVolume {
    #[serde(default)]
    pub m5: f64,
    #[serde(default)]
    pub h1: f64,
    #[serde(default)]
    pub h6: f64,
    #[serde(default)]
    pub h24: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DexPairTxnsWindow {
    #[serde(default)]
    pub buys: u64,
    #[serde(default)]
    pub sells: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DexPairTxns {
    #[serde(default)]
    pub m5: DexPairTxnsWindow,
    #[serde(default)]
    pub h1: DexPairTxnsWindow,
    #[serde(default)]
    pub h6: DexPairTxnsWindow,
    #[serde(default)]
    pub h24: DexPairTxnsWindow,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DexPair {
    pub chain_id: String,
    pub dex_id: String,
    pub pair_address: String,
    pub base_token: DexPairToken,
    pub quote_token: DexPairToken,

    #[serde(default)]
    pub price_usd: String,
    #[serde(default)]
    pub liquidity: DexPairLiquidity,
    #[serde(default)]
    pub volume: DexPairVolume,
    #[serde(default)]
    pub txns: DexPairTxns,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchResponse {
    pairs: Vec<DexPair>,
}

#[derive(Debug, Clone, Copy)]
pub enum DexChain {
    Solana,
}

impl DexChain {
    pub fn as_str(&self) -> &'static str {
        match self {
            DexChain::Solana => "solana",
        }
    }
}

pub struct DexscreenerClient {
    client: Client,
}

impl DexscreenerClient {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    fn cache_dir() -> PathBuf {
        PathBuf::from("data").join("dexscreener-cache")
    }

    fn cache_disabled() -> bool {
        std::env::var("DEXSCREENER_DISABLE_CACHE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
    }

    fn ttl_seconds() -> u64 {
        std::env::var("DEXSCREENER_CACHE_TTL_SECONDS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(300)
    }

    fn cache_path(key: &str) -> PathBuf {
        Self::cache_dir().join(format!("{key}.json"))
    }

    fn is_fresh(path: &PathBuf) -> bool {
        let ttl = Self::ttl_seconds();
        if ttl == 0 {
            return false;
        }
        if let Ok(meta) = fs::metadata(path) {
            if let Ok(modified) = meta.modified() {
                if let Ok(elapsed) = modified.elapsed() {
                    return elapsed.as_secs() <= ttl;
                }
            }
        }
        false
    }

    async fn get_json_cached<T: for<'de> Deserialize<'de> + Serialize>(
        &self,
        cache_key: &str,
        url: &str,
    ) -> Result<T> {
        let disable_cache = Self::cache_disabled();
        let path = Self::cache_path(cache_key);

        if !disable_cache && path.exists() && Self::is_fresh(&path) {
            if let Ok(bytes) = fs::read(&path) {
                if let Ok(val) = serde_json::from_slice::<T>(&bytes) {
                    return Ok(val);
                }
            }
        }

        let resp = self.client.get(url).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("Dexscreener error: {} - {}", status, text));
        }
        let val: T = resp.json().await?;

        if !disable_cache {
            let _ = fs::create_dir_all(Self::cache_dir());
            let _ = fs::write(&path, serde_json::to_vec_pretty(&val).unwrap_or_default());
        }

        Ok(val)
    }

    /// Search pairs by text query (Dexscreener search endpoint).
    pub async fn search(&self, query: &str) -> Result<Vec<DexPair>> {
        let q = query.trim();
        let url = format!(
            "https://api.dexscreener.com/latest/dex/search?q={}",
            urlencoding::encode(q)
        );
        let cache_key = format!("search_{}", sanitize_cache_key(q));
        let parsed: SearchResponse = self.get_json_cached(&cache_key, &url).await?;
        Ok(parsed.pairs)
    }

    /// List all pairs for a given token mint on a chain.
    pub async fn token_pairs(&self, chain: DexChain, token_mint: &str) -> Result<Vec<DexPair>> {
        let mint = token_mint.trim();
        let url = format!(
            "https://api.dexscreener.com/token-pairs/v1/{}/{}",
            chain.as_str(),
            mint
        );
        let cache_key = format!(
            "token_pairs_{}_{}",
            chain.as_str(),
            sanitize_cache_key(mint)
        );
        self.get_json_cached(&cache_key, &url).await
    }

    /// Fetch details for a specific pair address on a chain.
    pub async fn pair(&self, chain: DexChain, pair_address: &str) -> Result<Vec<DexPair>> {
        let addr = pair_address.trim();
        let url = format!(
            "https://api.dexscreener.com/latest/dex/pairs/{}/{}",
            chain.as_str(),
            addr
        );
        let cache_key = format!("pair_{}_{}", chain.as_str(), sanitize_cache_key(addr));
        let parsed: SearchResponse = self.get_json_cached(&cache_key, &url).await?;
        Ok(parsed.pairs)
    }
}

fn sanitize_cache_key(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}
