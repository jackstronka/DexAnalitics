//! On-chain Orca Whirlpool reads (RPC via `orca_whirlpools`, same family as `orca-positions-list` CLI).

use crate::error::{ApiError, ApiResult};
use crate::models::{OrcaOwnerPositionEntry, OrcaOwnerPositionsResponse};
use crate::state::AppState;
use axum::{Json, extract::Query, extract::State};
use orca_whirlpools::{
    PositionOrBundle, WhirlpoolsConfigInput, fetch_positions_for_owner,
    set_whirlpools_config_address,
};
use serde::Deserialize;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

#[derive(Debug, Deserialize)]
pub struct OrcaPositionsByOwnerQuery {
    /// Owner wallet pubkey (base58).
    pub owner: String,
}

/// List Whirlpool LP positions for an owner on-chain (RPC). Does not use `PositionMonitor`.
#[utoipa::path(
    get,
    path = "/orca/positions-by-owner",
    tag = "Orca",
    params(
        ("owner" = String, Query, description = "Solana wallet pubkey (base58)")
    ),
    responses(
        (status = 200, description = "Positions from chain", body = OrcaOwnerPositionsResponse),
        (status = 400, description = "Invalid owner pubkey")
    )
)]
pub async fn orca_positions_by_owner(
    State(state): State<AppState>,
    Query(q): Query<OrcaPositionsByOwnerQuery>,
) -> ApiResult<Json<OrcaOwnerPositionsResponse>> {
    let owner_trim = q.owner.trim();
    if owner_trim.is_empty() {
        return Err(ApiError::bad_request("query parameter `owner` is required"));
    }
    let owner_pk =
        Pubkey::from_str(owner_trim).map_err(|_| ApiError::bad_request("invalid owner pubkey"))?;

    let endpoint = state.provider.current_endpoint().await;
    let config = if endpoint.contains("devnet") {
        WhirlpoolsConfigInput::SolanaDevnet
    } else {
        WhirlpoolsConfigInput::SolanaMainnet
    };
    set_whirlpools_config_address(config)
        .map_err(|e| ApiError::internal(format!("orca config: {e}")))?;

    let rpc = RpcClient::new(endpoint.clone());
    let raw = fetch_positions_for_owner(&rpc, owner_pk)
        .await
        .map_err(|e| ApiError::internal(format!("fetch_positions_for_owner: {e}")))?;

    let mut entries = Vec::new();
    for p in raw {
        match p {
            PositionOrBundle::Position(h) => {
                let d = &h.data;
                entries.push(OrcaOwnerPositionEntry {
                    kind: "position".to_string(),
                    position_address: h.address.to_string(),
                    pool_address: d.whirlpool.to_string(),
                    tick_lower: d.tick_lower_index,
                    tick_upper: d.tick_upper_index,
                    liquidity: d.liquidity.to_string(),
                    position_mint: Some(d.position_mint.to_string()),
                    position_bundle_address: None,
                });
            }
            PositionOrBundle::PositionBundle(b) => {
                let bundle_addr = b.address.to_string();
                for bp in &b.positions {
                    let d = &bp.data;
                    entries.push(OrcaOwnerPositionEntry {
                        kind: "bundled_position".to_string(),
                        position_address: bp.address.to_string(),
                        pool_address: d.whirlpool.to_string(),
                        tick_lower: d.tick_lower_index,
                        tick_upper: d.tick_upper_index,
                        liquidity: d.liquidity.to_string(),
                        position_mint: Some(d.position_mint.to_string()),
                        position_bundle_address: Some(bundle_addr.clone()),
                    });
                }
            }
        }
    }

    let total = entries.len();
    Ok(Json(OrcaOwnerPositionsResponse {
        owner: owner_pk.to_string(),
        rpc_url: endpoint,
        total,
        entries,
    }))
}
