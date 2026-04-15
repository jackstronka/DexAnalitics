//! Minimal mint→USD for two-token pools (GeckoTerminal + stablecoin peg).
//! Shared by event-time pool pricing so `clmm-lp-execution` does not depend on `clmm-lp-api`.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::time::Duration;
use tracing::warn;

const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
const USDT_MINT: &str = "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB";
const USDC_DEVNET: &str = "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU";

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

/// Best-effort USD prices for Solana SPL mints (GeckoTerminal batch).
pub async fn fetch_gecko_solana_mint_prices_usd(mints: &[String]) -> BTreeMap<String, f64> {
    if mints.is_empty() {
        return BTreeMap::new();
    }
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .user_agent("clmm-lp-protocols/0.1 (event_pool_prices)")
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            warn!(error = %e, "reqwest client build failed");
            return BTreeMap::new();
        }
    };

    let tail = mints.join(",");
    let url = format!(
        "https://api.geckoterminal.com/api/v2/simple/networks/solana/token_price/{tail}"
    );
    let resp = match client.get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            warn!(error = %e, "GeckoTerminal price request failed");
            return BTreeMap::new();
        }
    };
    if !resp.status().is_success() {
        warn!(status = %resp.status(), "GeckoTerminal price non-success");
        return BTreeMap::new();
    }
    let text = match resp.text().await {
        Ok(t) => t,
        Err(e) => {
            warn!(error = %e, "GeckoTerminal read body failed");
            return BTreeMap::new();
        }
    };
    let wire: GeckoTerminalWire = match serde_json::from_str(&text) {
        Ok(w) => w,
        Err(e) => {
            warn!(error = %e, "GeckoTerminal JSON parse failed");
            return BTreeMap::new();
        }
    };
    let mut out = BTreeMap::new();
    for (mint, s) in wire.data.attributes.token_prices {
        if let Ok(p) = s.parse::<f64>()
            && p.is_finite()
            && p > 0.0
        {
            out.insert(mint, p);
        }
    }
    out
}

/// Apply ~1 USD for known stable mints when missing from Gecko.
pub fn stablecoin_usd_if_applicable(mint: &str) -> Option<f64> {
    if mint.eq_ignore_ascii_case(USDC_MINT)
        || mint.eq_ignore_ascii_case(USDT_MINT)
        || mint.eq_ignore_ascii_case(USDC_DEVNET)
    {
        Some(1.0)
    } else {
        None
    }
}
