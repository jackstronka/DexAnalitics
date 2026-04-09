//! Shared USD price lookup for SPL mints (positions, wallet UI, `/prices/jupiter`).
//!
//! **Sources (best-effort, no paid vendor required):**
//! - USDC / USDT → $1
//! - [**GeckoTerminal**](https://apiguide.geckoterminal.com/) — public DEX aggregate by mint (no key; batch-friendly)
//! - Jupiter Price API v2 (optional `JUPITER_API_KEY`; without key often 401)
//! - legacy `price.jup.ag/v4` if DNS works
//! - DexPaprika SSE per mint
//! - Dexscreener `token-pairs` (same as CLI snapshot helpers)
//!
//! **Not implemented here (but valid product directions):** USD from **your own** swap logs /
//! decoded pool state (vault balances × external quote) — see `doc/` and CLI snapshot tooling.

use clmm_lp_data::providers::{DexChain, DexscreenerClient};
use serde::Deserialize;
use serde_json::Value as JsonValue;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;
use std::time::Duration;
use tracing::warn;

static HTTP: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::limited(10))
        .user_agent("clmm-lp-api/0.1 (prices)")
        .build()
        .expect("reqwest client for price_fetch")
});

/// SPL mints we treat as ~1 USD without hitting external feeds (same idea as snapshot tooling).
const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
const USDT_MINT: &str = "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB";
/// Wrapped SOL — feeds sometimes omit this mint; see `position_valuation` + CoinGecko fallback below.
const WSOL_MINT: &str = "So11111111111111111111111111111111111111112";

fn stablecoin_usd(mint: &str) -> Option<f64> {
    if mint.eq_ignore_ascii_case(USDC_MINT) || mint.eq_ignore_ascii_case(USDT_MINT) {
        Some(1.0)
    } else {
        None
    }
}

fn jupiter_price_v2_url() -> String {
    std::env::var("JUPITER_PRICE_API_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "https://api.jup.ag/price/v2".to_string())
}

fn jupiter_api_key() -> Option<String> {
    std::env::var("JUPITER_API_KEY")
        .ok()
        .filter(|s| !s.trim().is_empty())
}

#[derive(Debug, Deserialize)]
struct JupiterV2Wire {
    #[serde(default)]
    data: BTreeMap<String, JupiterV2Row>,
}

#[derive(Debug, Deserialize)]
struct JupiterV2Row {
    /// v2 returns price as a decimal string.
    #[serde(default)]
    price: String,
}

#[derive(Debug, Deserialize)]
struct JupiterLegacyWire {
    #[serde(default)]
    data: BTreeMap<String, JupiterLegacyRow>,
}

#[derive(Debug, Deserialize)]
struct JupiterLegacyRow {
    price: f64,
}

#[derive(Debug, Deserialize)]
struct DexPaprikaSseData {
    #[allow(dead_code)]
    a: String,
    c: String,
    p: String,
}

/// Chunk size for GeckoTerminal `token_price/{addr,addr,...}` (avoid huge URLs).
const GECKO_TERMINAL_MINT_CHUNK: usize = 25;

#[derive(Debug, Deserialize)]
struct GeckoTerminalWire {
    data: GeckoTerminalData,
}

#[derive(Debug, Deserialize)]
struct GeckoTerminalData {
    attributes: GeckoTerminalAttrs,
}

#[derive(Debug, Deserialize)]
struct GeckoTerminalAttrs {
    #[serde(default)]
    token_prices: BTreeMap<String, String>,
}

/// GeckoTerminal simple token price (aggregated DEX liquidity). Free tier, no API key.
async fn fetch_geckoterminal_prices(mints: &[String]) -> BTreeMap<String, f64> {
    if mints.is_empty() {
        return BTreeMap::new();
    }

    let mut out = BTreeMap::new();
    for chunk in mints.chunks(GECKO_TERMINAL_MINT_CHUNK) {
        let tail = chunk.join(",");
        let url = format!(
            "https://api.geckoterminal.com/api/v2/simple/networks/solana/token_price/{tail}"
        );
        let resp = match HTTP.get(&url).send().await {
            Ok(r) => r,
            Err(e) => {
                warn!(error = %e, "GeckoTerminal price request failed");
                continue;
            }
        };
        if !resp.status().is_success() {
            warn!(status = %resp.status(), "GeckoTerminal price non-success");
            continue;
        }
        let text = match resp.text().await {
            Ok(t) => t,
            Err(e) => {
                warn!(error = %e, "GeckoTerminal read body failed");
                continue;
            }
        };
        let wire: GeckoTerminalWire = match serde_json::from_str(&text) {
            Ok(w) => w,
            Err(e) => {
                warn!(error = %e, "GeckoTerminal JSON parse failed");
                continue;
            }
        };
        for (mint, s) in wire.data.attributes.token_prices {
            if let Ok(p) = s.parse::<f64>() {
                if p.is_finite() && p > 0.0 {
                    out.insert(mint, p);
                }
            }
        }
    }
    out
}

async fn fetch_jupiter_v2_prices(ids: &str) -> BTreeMap<String, f64> {
    let base = jupiter_price_v2_url();
    let mut req = HTTP.get(&base).query(&[("ids", ids)]);
    if let Some(ref key) = jupiter_api_key() {
        req = req.header("x-api-key", key);
    }
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            warn!(error = %e, url = %base, "Jupiter v2 price request failed");
            return BTreeMap::new();
        }
    };
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        warn!(%status, body = %body, "Jupiter v2 price non-success");
        return BTreeMap::new();
    }
    let text = match resp.text().await {
        Ok(t) => t,
        Err(e) => {
            warn!(error = %e, "Jupiter v2 price read body failed");
            return BTreeMap::new();
        }
    };
    let wire: JupiterV2Wire = match serde_json::from_str(&text) {
        Ok(w) => w,
        Err(e) => {
            warn!(error = %e, "Jupiter v2 price JSON parse failed");
            return BTreeMap::new();
        }
    };
    let mut out = BTreeMap::new();
    for (mint, row) in wire.data {
        if let Ok(p) = row.price.parse::<f64>() {
            if p.is_finite() && p > 0.0 {
                out.insert(mint, p);
            }
        }
    }
    out
}

/// Legacy `price.jup.ag/v4` wire format (may still work on some networks).
async fn fetch_jupiter_legacy_v4_prices(ids: &str) -> BTreeMap<String, f64> {
    let resp = match HTTP
        .get("https://price.jup.ag/v4/price")
        .query(&[("ids", ids)])
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            warn!(error = %e, "legacy Jupiter v4 price request failed");
            return BTreeMap::new();
        }
    };
    if !resp.status().is_success() {
        return BTreeMap::new();
    }
    let wire: JupiterLegacyWire = match resp.json().await {
        Ok(w) => w,
        Err(_) => return BTreeMap::new(),
    };
    wire.data
        .into_iter()
        .filter_map(|(k, v)| {
            if v.price.is_finite() && v.price > 0.0 {
                Some((k, v.price))
            } else {
                None
            }
        })
        .collect()
}

async fn fetch_dexpaprika_price_usd(mint: &str) -> Option<f64> {
    let resp = HTTP
        .get("https://streaming.dexpaprika.com/stream")
        .query(&[
            ("method", "t_p"),
            ("chain", "solana"),
            ("address", mint),
            ("limit", "1"),
        ])
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?;
    let text = resp.text().await.ok()?;
    for line in text.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("data:") {
            let payload = rest.trim();
            let data: DexPaprikaSseData = serde_json::from_str(payload).ok()?;
            if data.c != "solana" {
                continue;
            }
            let p = data.p.parse::<f64>().ok()?;
            if p.is_finite() && p > 0.0 {
                return Some(p);
            }
        }
    }
    None
}

async fn fetch_coingecko_solana_usd() -> Option<f64> {
    let url = "https://api.coingecko.com/api/v3/simple/price?ids=solana&vs_currencies=usd";
    let resp = HTTP.get(url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let text = resp.text().await.ok()?;
    let v: JsonValue = serde_json::from_str(&text).ok()?;
    let u = v.get("solana")?.get("usd")?.as_f64()?;
    if u.is_finite() && u > 0.0 {
        Some(u)
    } else {
        None
    }
}

async fn fetch_dexscreener_mint_usd(mint: &str) -> Option<f64> {
    let client = DexscreenerClient::new();
    let pairs = client.token_pairs(DexChain::Solana, mint).await.ok()?;
    let best = pairs.iter().max_by(|a, b| {
        a.liquidity
            .usd
            .partial_cmp(&b.liquidity.usd)
            .unwrap_or(std::cmp::Ordering::Equal)
    })?;
    let px: f64 = best.price_usd.parse().ok()?;
    if px.is_finite() && px > 0.0 {
        Some(px)
    } else {
        None
    }
}

/// Best-effort USD price map for the given mint set (see module docs for ordering).
///
/// Returns a short `source` label for API responses (not a strict provenance per mint).
pub async fn fetch_mint_prices_usd(mints: &BTreeSet<String>) -> (BTreeMap<String, f64>, String) {
    if mints.is_empty() {
        return (BTreeMap::new(), "none".to_string());
    }

    let mut prices: BTreeMap<String, f64> = BTreeMap::new();
    let mut tags: Vec<&'static str> = Vec::new();

    for m in mints {
        if let Some(p) = stablecoin_usd(m) {
            prices.insert(m.clone(), p);
        }
    }
    if !prices.is_empty() {
        tags.push("stable");
    }

    // Prefer a known-good SOL/USD for WSOL mint.
    //
    // We observed `GeckoTerminal simple token_price` sometimes returning stale/wrong values for WSOL,
    // which then propagates to the dashboard as a misleading "Jupiter" USD estimate.
    if mints.iter().any(|m| m == WSOL_MINT) {
        if let Some(p) = fetch_coingecko_solana_usd().await {
            prices.insert(WSOL_MINT.to_string(), p);
            tags.push("coingecko_solana");
        }
    }

    let mut pending: Vec<String> = mints
        .iter()
        .filter(|m| !prices.contains_key(*m))
        .cloned()
        .collect();

    if !pending.is_empty() {
        let gt = fetch_geckoterminal_prices(&pending).await;
        if !gt.is_empty() {
            tags.push("geckoterminal");
        }
        for (k, v) in gt {
            prices.insert(k, v);
        }
    }

    pending = mints
        .iter()
        .filter(|m| !prices.contains_key(*m))
        .cloned()
        .collect();

    if !pending.is_empty() {
        let qs = pending.join(",");
        let v2 = fetch_jupiter_v2_prices(&qs).await;
        if !v2.is_empty() {
            tags.push("jupiter_v2");
        }
        for (k, v) in v2 {
            prices.insert(k, v);
        }

        let still: Vec<String> = pending
            .into_iter()
            .filter(|m| !prices.contains_key(m))
            .collect();
        if !still.is_empty() {
            let qs2 = still.join(",");
            let leg = fetch_jupiter_legacy_v4_prices(&qs2).await;
            if !leg.is_empty() {
                tags.push("jupiter_v4_legacy");
            }
            for (k, v) in leg {
                prices.insert(k, v);
            }
        }
    }

    let missing: Vec<String> = mints
        .iter()
        .filter(|m| !prices.contains_key(*m))
        .cloned()
        .collect();

    if !missing.is_empty() {
        let mut dp_any = false;
        for mint in &missing {
            if let Some(p) = fetch_dexpaprika_price_usd(mint).await {
                prices.insert(mint.clone(), p);
                dp_any = true;
            }
        }
        if dp_any {
            tags.push("dexpaprika");
        }
    }

    let missing2: Vec<String> = mints
        .iter()
        .filter(|m| !prices.contains_key(*m))
        .cloned()
        .collect();

    if !missing2.is_empty() {
        let mut ds_any = false;
        for mint in &missing2 {
            if let Some(p) = fetch_dexscreener_mint_usd(mint).await {
                prices.insert(mint.clone(), p);
                ds_any = true;
            }
        }
        if ds_any {
            tags.push("dexscreener");
        }
    }

    // WSOL fallback is handled early (prefer CoinGecko).

    let source = if tags.is_empty() {
        "none".to_string()
    } else {
        tags.join("+")
    };

    (prices, source)
}
