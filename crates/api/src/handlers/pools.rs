//! Pool handlers.

use crate::error::{ApiError, ApiResult};
use crate::models::{ListPoolsResponse, PoolResponse, PoolStateResponse};
use crate::state::AppState;
use axum::{
    Json,
    extract::{Path, State},
};
use clmm_lp_data::providers::{OrcaListPoolsQuery, OrcaRestClient};
use clmm_lp_protocols::prelude::WhirlpoolReader;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

/// List available pools.
#[utoipa::path(
    get,
    path = "/pools",
    tag = "Pools",
    responses(
        (status = 200, description = "List of pools", body = ListPoolsResponse)
    )
)]
pub async fn list_pools(State(_state): State<AppState>) -> ApiResult<Json<ListPoolsResponse>> {
    let base_url = std::env::var("ORCA_PUBLIC_API_BASE_URL")
        .unwrap_or_else(|_| "https://api.orca.so/v2/solana".to_string());
    let client = OrcaRestClient::new(base_url);

    let q = OrcaListPoolsQuery {
        size: Some(50),
        stats: Some("24h".to_string()),
        ..Default::default()
    };
    let paged = client
        .list_pools(q)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let pools = paged
        .data
        .into_iter()
        .map(|p| PoolResponse {
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
        })
        .collect::<Vec<_>>();

    Ok(Json(ListPoolsResponse {
        total: pools.len(),
        pools,
    }))
}

/// Get pool details.
#[utoipa::path(
    get,
    path = "/pools/{address}",
    tag = "Pools",
    params(
        ("address" = String, Path, description = "Pool address")
    ),
    responses(
        (status = 200, description = "Pool details", body = PoolResponse),
        (status = 404, description = "Pool not found")
    )
)]
pub async fn get_pool(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> ApiResult<Json<PoolResponse>> {
    let _pubkey =
        Pubkey::from_str(&address).map_err(|_| ApiError::bad_request("Invalid pool address"))?;

    let reader = WhirlpoolReader::new(state.provider.clone());

    let pool_state = reader
        .get_pool_state(&address)
        .await
        .map_err(|e| ApiError::not_found(format!("Pool not found: {}", e)))?;

    let response = PoolResponse {
        address: pool_state.address,
        protocol: "orca_whirlpool".to_string(),
        token_mint_a: pool_state.token_mint_a.to_string(),
        token_mint_b: pool_state.token_mint_b.to_string(),
        current_tick: pool_state.tick_current,
        tick_spacing: pool_state.tick_spacing as i32,
        price: pool_state.price,
        liquidity: pool_state.liquidity.to_string(),
        fee_rate_bps: pool_state.fee_rate_bps,
        volume_24h_usd: None,
        tvl_usd: None,
        apy_estimate: None,
    };

    Ok(Json(response))
}

/// Get current pool state.
#[utoipa::path(
    get,
    path = "/pools/{address}/state",
    tag = "Pools",
    params(
        ("address" = String, Path, description = "Pool address")
    ),
    responses(
        (status = 200, description = "Current pool state", body = PoolStateResponse),
        (status = 404, description = "Pool not found")
    )
)]
pub async fn get_pool_state(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> ApiResult<Json<PoolStateResponse>> {
    let _pubkey =
        Pubkey::from_str(&address).map_err(|_| ApiError::bad_request("Invalid pool address"))?;

    let reader = WhirlpoolReader::new(state.provider.clone());

    let pool_state = reader
        .get_pool_state(&address)
        .await
        .map_err(|e| ApiError::not_found(format!("Pool not found: {}", e)))?;

    let response = PoolStateResponse {
        address: pool_state.address,
        current_tick: pool_state.tick_current,
        sqrt_price: pool_state.sqrt_price.to_string(),
        price: pool_state.price,
        liquidity: pool_state.liquidity.to_string(),
        fee_growth_global_a: pool_state.fee_growth_global_a.to_string(),
        fee_growth_global_b: pool_state.fee_growth_global_b.to_string(),
        timestamp: chrono::Utc::now(),
    };

    Ok(Json(response))
}
