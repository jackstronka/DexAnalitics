//! Free price endpoints (server-side fetch to avoid browser CORS/adblock).

use crate::error::{ApiError, ApiResult};
use crate::models::JupiterPricesResponse;
use crate::services::price_fetch::fetch_mint_prices_usd;
use axum::Json;
use axum::extract::Query;
use serde::Deserialize;
use std::collections::BTreeSet;

#[derive(Debug, Deserialize)]
pub struct JupiterPricesQuery {
    /// Comma-separated list of SPL mint addresses.
    pub ids: String,
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
pub async fn get_jupiter_prices(
    Query(q): Query<JupiterPricesQuery>,
) -> ApiResult<Json<JupiterPricesResponse>> {
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

    let (prices, source) = fetch_mint_prices_usd(&uniq_all).await;

    Ok(Json(JupiterPricesResponse {
        source,
        requested,
        returned: prices.len(),
        prices,
    }))
}
