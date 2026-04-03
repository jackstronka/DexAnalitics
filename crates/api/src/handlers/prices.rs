//! Free price endpoints (server-side fetch to avoid browser CORS/adblock).

use crate::error::{ApiError, ApiResult};
use crate::models::JupiterPricesResponse;
use axum::extract::Query;
use axum::Json;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

static HTTP: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::limited(10))
        // Some public APIs reject requests without a User-Agent.
        .user_agent("clmm-lp-api/0.1 (prices)")
        .build()
        .expect("reqwest client for prices")
});

// Simple in-memory price cache to reduce external calls (best-effort, dev-friendly).
// Key: mint (base58), Value: (price_usd, inserted_at).
static PRICE_CACHE: LazyLock<RwLock<std::collections::HashMap<String, (f64, Instant)>>> =
    LazyLock::new(|| RwLock::new(std::collections::HashMap::new()));
const PRICE_CACHE_TTL: Duration = Duration::from_secs(90);

#[derive(Debug, Deserialize)]
pub struct JupiterPricesQuery {
    /// Comma-separated list of SPL mint addresses.
    pub ids: String,
}

#[derive(Debug, Deserialize)]
struct JupiterPriceWire {
    data: BTreeMap<String, JupiterPriceRow>,
}

#[derive(Debug, Deserialize)]
struct JupiterPriceRow {
    price: f64,
}

#[derive(Debug, Deserialize)]
struct DexPaprikaSseData {
    /// Token address
    #[allow(dead_code)]
    a: String,
    /// Chain slug
    c: String,
    /// Price in USD (numeric string)
    p: String,
}

async fn cache_get(mint: &str) -> Option<f64> {
    let map = PRICE_CACHE.read().await;
    let (p, ts) = map.get(mint)?;
    if ts.elapsed() <= PRICE_CACHE_TTL {
        Some(*p)
    } else {
        None
    }
}

async fn cache_put(mint: &str, price: f64) {
    let mut map = PRICE_CACHE.write().await;
    map.insert(mint.to_string(), (price, Instant::now()));
}

async fn fetch_dexpaprika_price_usd(mint: &str) -> Result<Option<f64>, reqwest::Error> {
    // Use SSE endpoint with `limit=1` to get a single price update and close.
    let resp = HTTP
        .get("https://streaming.dexpaprika.com/stream")
        .query(&[
            ("method", "t_p"),
            ("chain", "solana"),
            ("address", mint),
            ("limit", "1"),
        ])
        .send()
        .await?;
    let resp = resp.error_for_status()?;
    let text = resp.text().await?;

    // Parse first "data: {...}" line.
    for line in text.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("data:") {
            let payload = rest.trim();
            let data: DexPaprikaSseData = match serde_json::from_str(payload) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if data.c != "solana" {
                continue;
            }
            // Some feeds may echo a normalized address; we still return the parsed price.
            let p = data.p.parse::<f64>().ok();
            if let Some(p) = p {
                if p.is_finite() && p > 0.0 {
                    return Ok(Some(p));
                }
            }
        }
    }
    Ok(None)
}

/// Jupiter USD prices (server-side). Query: `?ids=mint1,mint2,...`
#[utoipa::path(
    get,
    path = "/prices/jupiter",
    tag = "Prices",
    params(
        ("ids" = String, Query, description = "Comma-separated SPL mint addresses")
    ),
    responses(
        (status = 200, description = "USD prices map", body = JupiterPricesResponse),
        (status = 400, description = "Missing ids")
    )
)]
pub async fn get_jupiter_prices(Query(q): Query<JupiterPricesQuery>) -> ApiResult<Json<JupiterPricesResponse>> {
    let raw = q.ids.trim();
    if raw.is_empty() {
        return Err(ApiError::bad_request("query parameter `ids` is required"));
    }

    let mut uniq_all: BTreeSet<String> = BTreeSet::new();
    for x in raw.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
        uniq_all.insert(x.to_string());
    }
    if uniq_all.is_empty() {
        return Err(ApiError::bad_request("query parameter `ids` is required"));
    }

    let requested = uniq_all.len();

    // Attempt 1: Jupiter (free, but may be blocked or may not cover all SPL mints).
    let mut prices: BTreeMap<String, f64> = BTreeMap::new();
    let mut source = "jupiter".to_string();
    let qs = uniq_all.iter().cloned().collect::<Vec<_>>().join(",");
    let jup: Result<JupiterPriceWire, reqwest::Error> = async {
        let resp = HTTP
            .get("https://price.jup.ag/v4/price")
            .query(&[("ids", qs.as_str())])
            .send()
            .await?;
        let resp = resp.error_for_status()?;
        resp.json::<JupiterPriceWire>().await
    }
    .await;

    if let Ok(ref wire) = jup {
        for (mint, row) in wire.data.iter() {
            if row.price.is_finite() && row.price > 0.0 {
                prices.insert(mint.clone(), row.price);
            }
        }
    }

    // Attempt 2: DexPaprika (free, no API key) to fill missing mints (or if Jupiter failed entirely).
    // Uses a public SSE endpoint with limit=1 (one-shot read).
    let missing: Vec<String> = uniq_all
        .iter()
        .filter(|m| !prices.contains_key(*m))
        .cloned()
        .collect();
    if jup.is_err() || !missing.is_empty() {
        source = if jup.is_err() {
            "dexpaprika".to_string()
        } else {
            "jupiter+dexpaprika".to_string()
        };

        // Fill from cache first.
        let mut still_missing: Vec<String> = Vec::new();
        for mint in missing {
            if let Some(p) = cache_get(&mint).await {
                prices.insert(mint, p);
            } else {
                still_missing.push(mint);
            }
        }

        // Best-effort fetch for remaining mints.
        for mint in &still_missing {
            match fetch_dexpaprika_price_usd(mint).await {
                Ok(Some(p)) => {
                    prices.insert(mint.clone(), p);
                    cache_put(mint, p).await;
                }
                Ok(None) => {}
                Err(_) => {}
            }
        }
    }

    Ok(Json(JupiterPricesResponse {
        source,
        requested,
        returned: prices.len(),
        prices,
    }))
}

