//! On-chain Orca Whirlpool reads (RPC via `orca_whirlpools`, same family as `orca-positions-list` CLI).

use crate::error::{ApiError, ApiResult};
use crate::models::{OrcaOwnerPositionEntry, OrcaOwnerPositionsResponse};
use crate::services::position_valuation::enrich_pool_ticks_for_display;
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
use std::sync::Arc;

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
                    range_lower_usdc: None,
                    range_upper_usdc: None,
                    range_usdc_quote: None,
                    range_lower_price: None,
                    range_upper_price: None,
                    range_price_quote: None,
                    liquidity: d.liquidity.to_string(),
                    position_mint: Some(d.position_mint.to_string()),
                    position_bundle_address: None,
                    in_range: false,
                    token_a_label: None,
                    token_b_label: None,
                    token_mint_a: None,
                    token_mint_b: None,
                    token_price_a_usd: None,
                    token_price_b_usd: None,
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
                        range_lower_usdc: None,
                        range_upper_usdc: None,
                        range_usdc_quote: None,
                        range_lower_price: None,
                        range_upper_price: None,
                        range_price_quote: None,
                        liquidity: d.liquidity.to_string(),
                        position_mint: Some(d.position_mint.to_string()),
                        position_bundle_address: Some(bundle_addr.clone()),
                        in_range: false,
                        token_a_label: None,
                        token_b_label: None,
                        token_mint_a: None,
                        token_mint_b: None,
                        token_price_a_usd: None,
                        token_price_b_usd: None,
                    });
                }
            }
        }
    }

    let provider = state.provider.clone();
    let mut enriched: Vec<OrcaOwnerPositionEntry> = Vec::with_capacity(entries.len());
    for e in entries {
        let enrich = match Pubkey::from_str(&e.pool_address) {
            Ok(pk) => {
                enrich_pool_ticks_for_display(
                    Arc::clone(&provider),
                    &pk,
                    e.tick_lower,
                    e.tick_upper,
                )
                .await
            }
            Err(_) => crate::services::position_valuation::PoolTicksEnrichment::default(),
        };
        let range = enrich.range_usdc;
        let range_price = enrich.range_price;
        enriched.push(OrcaOwnerPositionEntry {
            range_lower_usdc: range.as_ref().map(|r| r.lower),
            range_upper_usdc: range.as_ref().map(|r| r.upper),
            range_usdc_quote: range.as_ref().map(|r| r.quote.clone()),
            range_lower_price: range_price.as_ref().map(|r| r.lower),
            range_upper_price: range_price.as_ref().map(|r| r.upper),
            range_price_quote: range_price.as_ref().map(|r| r.quote.clone()),
            in_range: enrich.in_range,
            token_a_label: enrich.token_a_label,
            token_b_label: enrich.token_b_label,
            token_mint_a: enrich.token_mint_a,
            token_mint_b: enrich.token_mint_b,
            token_price_a_usd: enrich.token_price_a_usd,
            token_price_b_usd: enrich.token_price_b_usd,
            ..e
        });
    }

    let total = enriched.len();
    Ok(Json(OrcaOwnerPositionsResponse {
        owner: owner_pk.to_string(),
        rpc_url: endpoint,
        total,
        entries: enriched,
    }))
}
