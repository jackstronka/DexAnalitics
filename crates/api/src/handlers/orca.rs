//! Orca Public REST proxy handlers (`/orca/*`).

use crate::error::{ApiError, ApiResult};
use crate::models::{
    ListPoolsResponse, OrcaLockInfoResponse, OrcaLockResponse, OrcaProtocolResponse,
    OrcaTokenListResponse, OrcaTokenResponse, PoolResponse,
};
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::Json;
use clmm_lp_data::providers::{
    OrcaListPoolsQuery, OrcaListTokensQuery, OrcaRestClient, OrcaSearchPoolsQuery,
    OrcaSearchTokensQuery,
};
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;

fn rest_client(state: &AppState) -> OrcaRestClient {
    let base_url = state
        .config
        .orca_public_api_base_url
        .clone()
        .or_else(|| std::env::var("ORCA_PUBLIC_API_BASE_URL").ok())
        .unwrap_or_else(|| "https://api.orca.so/v2/solana".to_string());
    OrcaRestClient::new(base_url)
}

fn map_pool(p: clmm_lp_data::providers::OrcaPoolSummary) -> PoolResponse {
    PoolResponse {
        address: p.address,
        protocol: "orca_whirlpool".to_string(),
        token_mint_a: p.token_mint_a,
        token_mint_b: p.token_mint_b,
        current_tick: p.tick_current_index,
        tick_spacing: p.tick_spacing as i32,
        price: Decimal::from_str_exact(&p.price).unwrap_or(Decimal::ZERO),
        liquidity: p.liquidity,
        fee_rate_bps: p.fee_rate,
        volume_24h_usd: None,
        tvl_usd: p.tvl_usdc.parse::<f64>().ok().and_then(Decimal::from_f64),
        apy_estimate: None,
    }
}

fn parse_decimal_opt(v: Option<String>) -> Option<Decimal> {
    v.and_then(|s| s.parse::<f64>().ok())
        .and_then(Decimal::from_f64)
}

fn map_token(t: clmm_lp_data::providers::OrcaTokenSummary) -> OrcaTokenResponse {
    let symbol = t
        .symbol
        .clone()
        .or_else(|| t.metadata.as_ref().and_then(|m| m.symbol.clone()));
    let name = t
        .name
        .clone()
        .or_else(|| t.metadata.as_ref().and_then(|m| m.name.clone()));
    OrcaTokenResponse {
        mint: if t.mint.is_empty() {
            t.address.unwrap_or_default()
        } else {
            t.mint
        },
        symbol,
        name,
        decimals: t.decimals,
        verified: t.verified,
        price_usdc: parse_decimal_opt(t.price_usdc),
    }
}

/// Proxy: Orca REST `GET /pools`.
#[utoipa::path(
    get,
    path = "/orca/pools",
    tag = "Orca",
    responses(
        (status = 200, description = "List of Orca pools (REST)", body = ListPoolsResponse)
    )
)]
pub async fn orca_list_pools(
    State(state): State<AppState>,
    Query(q): Query<OrcaListPoolsQuery>,
) -> ApiResult<Json<ListPoolsResponse>> {
    let client = rest_client(&state);
    let paged = client.list_pools(q).await.map_err(|e| ApiError::internal(e.to_string()))?;
    let pools = paged.data.into_iter().map(map_pool).collect::<Vec<_>>();
    Ok(Json(ListPoolsResponse {
        total: pools.len(),
        pools,
    }))
}

/// Proxy: Orca REST `GET /pools/search`.
#[utoipa::path(
    get,
    path = "/orca/pools/search",
    tag = "Orca",
    params(
        ("q" = String, Query, description = "Search query (e.g. SOL-USDC)"),
        ("size" = Option<u32>, Query, description = "Page size"),
        ("next" = Option<String>, Query, description = "Cursor")
    ),
    responses(
        (status = 200, description = "Search results (REST)", body = ListPoolsResponse)
    )
)]
pub async fn orca_search_pools(
    State(state): State<AppState>,
    Query(q): Query<OrcaSearchPoolsQuery>,
) -> ApiResult<Json<ListPoolsResponse>> {
    let client = rest_client(&state);
    let paged = client
        .search_pools(q)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let pools = paged.data.into_iter().map(map_pool).collect::<Vec<_>>();
    Ok(Json(ListPoolsResponse {
        total: pools.len(),
        pools,
    }))
}

/// Proxy: Orca REST `GET /pools/{address}`.
#[utoipa::path(
    get,
    path = "/orca/pools/{address}",
    tag = "Orca",
    params(
        ("address" = String, Path, description = "Whirlpool address")
    ),
    responses(
        (status = 200, description = "Orca pool details (REST)", body = PoolResponse),
        (status = 404, description = "Pool not found")
    )
)]
pub async fn orca_get_pool(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> ApiResult<Json<PoolResponse>> {
    let client = rest_client(&state);
    let wrapped = client
        .get_pool(&address)
        .await
        .map_err(|e| ApiError::not_found(e.to_string()))?;
    Ok(Json(map_pool(wrapped.data)))
}

/// Proxy: Orca REST `GET /lock/{address}`.
#[utoipa::path(
    get,
    path = "/orca/lock/{address}",
    tag = "Orca",
    params(
        ("address" = String, Path, description = "Whirlpool address")
    ),
    responses(
        (status = 200, description = "Lock info (REST)", body = OrcaLockResponse),
        (status = 404, description = "Not found")
    )
)]
pub async fn orca_get_lock_info(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> ApiResult<Json<OrcaLockResponse>> {
    let client = rest_client(&state);
    let locks = client
        .get_lock_info(&address)
        .await
        .map_err(|e| ApiError::not_found(e.to_string()))?;
    let mapped = locks
        .into_iter()
        .map(|l| OrcaLockInfoResponse {
            name: l.name,
            locked_percentage: l.locked_percentage,
        })
        .collect::<Vec<_>>();
    Ok(Json(OrcaLockResponse {
        address,
        locks: mapped,
    }))
}

/// Proxy: Orca REST `GET /tokens`.
#[utoipa::path(
    get,
    path = "/orca/tokens",
    tag = "Orca",
    responses(
        (status = 200, description = "List of Orca tokens (REST)", body = OrcaTokenListResponse)
    )
)]
pub async fn orca_list_tokens(
    State(state): State<AppState>,
    Query(q): Query<OrcaListTokensQuery>,
) -> ApiResult<Json<OrcaTokenListResponse>> {
    let client = rest_client(&state);
    let paged = client
        .list_tokens(q)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let tokens = paged.data.into_iter().map(map_token).collect::<Vec<_>>();
    Ok(Json(OrcaTokenListResponse {
        total: tokens.len(),
        tokens,
    }))
}

/// Proxy: Orca REST `GET /tokens/search`.
#[utoipa::path(
    get,
    path = "/orca/tokens/search",
    tag = "Orca",
    params(
        ("q" = String, Query, description = "Search query (e.g. ORCA)"),
        ("size" = Option<u32>, Query, description = "Page size"),
        ("next" = Option<String>, Query, description = "Cursor")
    ),
    responses(
        (status = 200, description = "Search token results (REST)", body = OrcaTokenListResponse)
    )
)]
pub async fn orca_search_tokens(
    State(state): State<AppState>,
    Query(q): Query<OrcaSearchTokensQuery>,
) -> ApiResult<Json<OrcaTokenListResponse>> {
    let client = rest_client(&state);
    let paged = client
        .search_tokens(q)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let tokens = paged.data.into_iter().map(map_token).collect::<Vec<_>>();
    Ok(Json(OrcaTokenListResponse {
        total: tokens.len(),
        tokens,
    }))
}

/// Proxy: Orca REST `GET /tokens/{mint}`.
#[utoipa::path(
    get,
    path = "/orca/tokens/{mint}",
    tag = "Orca",
    params(
        ("mint" = String, Path, description = "Token mint address")
    ),
    responses(
        (status = 200, description = "Orca token details (REST)", body = OrcaTokenResponse),
        (status = 404, description = "Token not found")
    )
)]
pub async fn orca_get_token(
    State(state): State<AppState>,
    Path(mint): Path<String>,
) -> ApiResult<Json<OrcaTokenResponse>> {
    let client = rest_client(&state);
    let wrapped = client
        .get_token(&mint)
        .await
        .map_err(|e| ApiError::not_found(e.to_string()))?;
    Ok(Json(map_token(wrapped.data)))
}

/// Proxy: Orca REST `GET /protocol`.
#[utoipa::path(
    get,
    path = "/orca/protocol",
    tag = "Orca",
    responses(
        (status = 200, description = "Orca protocol stats (REST)", body = OrcaProtocolResponse)
    )
)]
pub async fn orca_get_protocol(
    State(state): State<AppState>,
) -> ApiResult<Json<OrcaProtocolResponse>> {
    let client = rest_client(&state);
    let wrapped = client
        .get_protocol()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(OrcaProtocolResponse {
        tvl_usdc: parse_decimal_opt(wrapped.data.tvl_usdc.or(wrapped.data.tvl)),
        volume_24h_usdc: parse_decimal_opt(wrapped.data.volume_24h_usdc),
        volume_7d_usdc: parse_decimal_opt(wrapped.data.volume_7d_usdc),
    }))
}

