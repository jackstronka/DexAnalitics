//! Aerodrome Slipstream read-only handlers (Base).
//!
//! Priorytet produktowy: **najpierw komunikacja** (stabilny kontrakt HTTP + `BASE_RPC_URL`), potem
//! rozszerzanie read modelu i cięższe operacje — patrz `doc/AERODROME_SLIPSTREAM_BASE_LIVE_PLAN.md` §0.

use crate::error::{ApiError, ApiResult};
use crate::models::SlipstreamSlot0Response;
use crate::services::evm_json_rpc::{self, decode_slot0_return};
use crate::state::AppState;
use axum::{
    Json,
    extract::{Path, State},
};
use clmm_lp_protocols::aerodrome_slipstream::BASE_MAINNET_CHAIN_ID;

/// Returns `slot0()` for a Slipstream pool on Base (requires `BASE_RPC_URL`).
#[utoipa::path(
    get,
    path = "/evm/base/aerodrome-slipstream/pools/{pool}/slot0",
    tag = "Evm",
    params(
        ("pool" = String, Path, description = "Pool contract address (0x + 40 hex)")
    ),
    responses(
        (status = 200, description = "slot0 tuple", body = SlipstreamSlot0Response),
        (status = 400, description = "Invalid pool address"),
        (status = 503, description = "BASE_RPC_URL not configured"),
        (status = 502, description = "Base RPC error or bad return data")
    )
)]
pub async fn get_aerodrome_slipstream_pool_slot0(
    State(state): State<AppState>,
    Path(pool): Path<String>,
) -> ApiResult<Json<SlipstreamSlot0Response>> {
    let rpc_url = state.config.base_rpc_url.as_deref().ok_or_else(|| {
        ApiError::service_unavailable(
            "BASE_RPC_URL is not set; configure a Base JSON-RPC endpoint to use this route.",
        )
    })?;

    let pool_n = normalize_pool_address(pool.trim())?;

    let raw = evm_json_rpc::eth_call(rpc_url, &pool_n, evm_json_rpc::SLOT0_SELECTOR, "latest")
        .await
        .map_err(ApiError::bad_gateway)?;

    let d = decode_slot0_return(&raw).map_err(ApiError::bad_gateway)?;

    Ok(Json(SlipstreamSlot0Response {
        chain_id: BASE_MAINNET_CHAIN_ID,
        pool: pool_n,
        sqrt_price_x96: d.sqrt_price_x96.to_string(),
        tick: d.tick,
        observation_index: d.observation_index,
        observation_cardinality: d.observation_cardinality,
        observation_cardinality_next: d.observation_cardinality_next,
        fee_protocol: d.fee_protocol,
        unlocked: d.unlocked,
    }))
}

fn normalize_pool_address(s: &str) -> Result<String, ApiError> {
    let lower = s.to_ascii_lowercase();
    if !lower.starts_with("0x") || lower.len() != 42 {
        return Err(ApiError::bad_request(
            "pool must be a 0x-prefixed 20-byte address (42 chars)",
        ));
    }
    if !lower[2..].chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ApiError::bad_request("pool address contains non-hex"));
    }
    Ok(lower)
}
