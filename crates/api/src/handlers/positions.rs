//! Position handlers.

use crate::error::{ApiError, ApiResult};
use crate::models::{
    DecreaseLiquidityRequest, ListPositionsResponse, MessageResponse, OpenPositionRequest,
    PnLResponse, PositionResponse, PositionStatus, RebalanceRequest,
};
use crate::state::{AppState, PositionUpdate};
use axum::{
    Json,
    extract::{Path, State},
};
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;
use tracing::info;

use crate::services::PositionService;

/// List all positions.
#[utoipa::path(
    get,
    path = "/positions",
    tag = "Positions",
    responses(
        (status = 200, description = "List of positions", body = ListPositionsResponse)
    )
)]
pub async fn list_positions(
    State(state): State<AppState>,
) -> ApiResult<Json<ListPositionsResponse>> {
    let positions = state.monitor.get_positions().await;

    let responses: Vec<PositionResponse> = positions
        .iter()
        .map(|p| PositionResponse {
            address: p.address.to_string(),
            pool_address: p.pool.to_string(),
            owner: p.on_chain.owner.to_string(),
            tick_lower: p.on_chain.tick_lower,
            tick_upper: p.on_chain.tick_upper,
            liquidity: p.on_chain.liquidity.to_string(),
            in_range: p.in_range,
            value_usd: p.pnl.current_value_usd,
            pnl: PnLResponse {
                unrealized_pnl_usd: p.pnl.net_pnl_usd,
                unrealized_pnl_pct: p.pnl.net_pnl_pct,
                fees_earned_a: p.pnl.fees_earned_a,
                fees_earned_b: p.pnl.fees_earned_b,
                fees_earned_usd: p.pnl.fees_usd,
                il_pct: p.pnl.il_pct,
                net_pnl_usd: p.pnl.net_pnl_usd,
                net_pnl_pct: p.pnl.net_pnl_pct,
            },
            status: if p.in_range {
                PositionStatus::Active
            } else {
                PositionStatus::OutOfRange
            },
            created_at: None,
        })
        .collect();

    Ok(Json(ListPositionsResponse {
        total: responses.len(),
        positions: responses,
    }))
}

/// Get a specific position.
#[utoipa::path(
    get,
    path = "/positions/{address}",
    tag = "Positions",
    params(
        ("address" = String, Path, description = "Position address")
    ),
    responses(
        (status = 200, description = "Position details", body = PositionResponse),
        (status = 404, description = "Position not found")
    )
)]
pub async fn get_position(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> ApiResult<Json<PositionResponse>> {
    let pubkey = Pubkey::from_str(&address)
        .map_err(|_| ApiError::bad_request("Invalid position address"))?;

    let positions = state.monitor.get_positions().await;
    let position = positions
        .iter()
        .find(|p| p.address == pubkey)
        .ok_or_else(|| ApiError::not_found("Position not found"))?;

    let response = PositionResponse {
        address: position.address.to_string(),
        pool_address: position.pool.to_string(),
        owner: position.on_chain.owner.to_string(),
        tick_lower: position.on_chain.tick_lower,
        tick_upper: position.on_chain.tick_upper,
        liquidity: position.on_chain.liquidity.to_string(),
        in_range: position.in_range,
        value_usd: position.pnl.current_value_usd,
        pnl: PnLResponse {
            unrealized_pnl_usd: position.pnl.net_pnl_usd,
            unrealized_pnl_pct: position.pnl.net_pnl_pct,
            fees_earned_a: position.pnl.fees_earned_a,
            fees_earned_b: position.pnl.fees_earned_b,
            fees_earned_usd: position.pnl.fees_usd,
            il_pct: position.pnl.il_pct,
            net_pnl_usd: position.pnl.net_pnl_usd,
            net_pnl_pct: position.pnl.net_pnl_pct,
        },
        status: if position.in_range {
            PositionStatus::Active
        } else {
            PositionStatus::OutOfRange
        },
        created_at: None,
    };

    Ok(Json(response))
}

/// Open a new position.
#[utoipa::path(
    post,
    path = "/positions",
    tag = "Positions",
    request_body = OpenPositionRequest,
    responses(
        (status = 201, description = "Position opened", body = PositionResponse),
        (status = 400, description = "Invalid request")
    )
)]
pub async fn open_position(
    State(state): State<AppState>,
    Json(request): Json<OpenPositionRequest>,
) -> ApiResult<Json<MessageResponse>> {
    info!(
        pool = %request.pool_address,
        tick_lower = request.tick_lower,
        tick_upper = request.tick_upper,
        dry_run = state.dry_run,
        "Opening position"
    );

    let mut svc = PositionService::new(state.clone());
    svc.set_dry_run(state.dry_run);

    // Non-dry-run: use any available strategy executor (if present).
    if !state.dry_run {
        if let Some(exec) = state.executors.read().await.values().next().cloned() {
            svc.set_executor(exec);
        }
    }

    let op = svc.open_position(&request).await?;
    if op.success {
        if let Some(m) = op
            .data
            .as_ref()
            .and_then(|d| d.get("message"))
            .and_then(|v| v.as_str())
        {
            return Ok(Json(MessageResponse::new(m.to_string())));
        }
        if let Some(pda) = op
            .data
            .as_ref()
            .and_then(|d| d.get("position_pda"))
            .and_then(|v| v.as_str())
        {
            return Ok(Json(MessageResponse::new(format!(
                "Position opened. PDA: {pda}"
            ))));
        }
        return Ok(Json(MessageResponse::new("Position opened".to_string())));
    }

    Err(ApiError::ServiceUnavailable(
        op.error
            .unwrap_or_else(|| "Position opening failed".to_string()),
    ))
}

/// Close a position.
#[utoipa::path(
    delete,
    path = "/positions/{address}",
    tag = "Positions",
    params(
        ("address" = String, Path, description = "Position address")
    ),
    responses(
        (status = 200, description = "Position closed", body = MessageResponse),
        (status = 404, description = "Position not found")
    )
)]
pub async fn close_position(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> ApiResult<Json<MessageResponse>> {
    let pubkey = Pubkey::from_str(&address)
        .map_err(|_| ApiError::bad_request("Invalid position address"))?;

    info!(position = %address, dry_run = state.dry_run, "Closing position");

    // Verify position exists
    let positions = state.monitor.get_positions().await;
    let position = positions
        .iter()
        .find(|p| p.address == pubkey)
        .ok_or_else(|| ApiError::not_found("Position not found"))?;

    if state.dry_run {
        info!("Dry-run mode: would close position");

        // Broadcast simulated update
        state
            .broadcast_position_update(PositionUpdate {
            update_type: "close_simulated".to_string(),
            position_address: address.clone(),
            timestamp: chrono::Utc::now(),
            data: serde_json::json!({
                "liquidity": position.on_chain.liquidity.to_string(),
                "dry_run": true
            }),
            })
            .await;

        return Ok(Json(MessageResponse::new(format!(
            "[DRY-RUN] Would close position {} with liquidity {}",
            address, position.on_chain.liquidity
        ))));
    }

    let mut svc = PositionService::new(state.clone());
    svc.set_dry_run(false);
    if let Some(exec) = state.executors.read().await.values().next().cloned() {
        svc.set_executor(exec);
    }

    let op = svc.close_position(&address).await?;
    if op.success {
        Ok(Json(MessageResponse::new(format!(
            "Position closed: {address}"
        ))))
    } else {
        Err(ApiError::ServiceUnavailable(
            op.error
                .unwrap_or_else(|| "Position closing failed".to_string()),
        ))
    }
}

/// Collect fees from a position.
#[utoipa::path(
    post,
    path = "/positions/{address}/collect",
    tag = "Positions",
    params(
        ("address" = String, Path, description = "Position address")
    ),
    responses(
        (status = 200, description = "Fees collected", body = MessageResponse),
        (status = 404, description = "Position not found")
    )
)]
pub async fn collect_fees(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> ApiResult<Json<MessageResponse>> {
    let pubkey = Pubkey::from_str(&address)
        .map_err(|_| ApiError::bad_request("Invalid position address"))?;

    info!(position = %address, dry_run = state.dry_run, "Collecting fees");

    // Verify position exists
    let positions = state.monitor.get_positions().await;
    let position = positions
        .iter()
        .find(|p| p.address == pubkey)
        .ok_or_else(|| ApiError::not_found("Position not found"))?;

    if state.dry_run {
        info!("Dry-run mode: would collect fees");

        // Broadcast simulated update
        state
            .broadcast_position_update(PositionUpdate {
            update_type: "fees_collected_simulated".to_string(),
            position_address: address.clone(),
            timestamp: chrono::Utc::now(),
            data: serde_json::json!({
                "fees_a": position.pnl.fees_earned_a,
                "fees_b": position.pnl.fees_earned_b,
                "dry_run": true
            }),
            })
            .await;

        return Ok(Json(MessageResponse::new(format!(
            "[DRY-RUN] Would collect fees from position {}: {} token A, {} token B",
            address, position.pnl.fees_earned_a, position.pnl.fees_earned_b
        ))));
    }

    let mut svc = PositionService::new(state.clone());
    svc.set_dry_run(false);
    if let Some(exec) = state.executors.read().await.values().next().cloned() {
        svc.set_executor(exec);
    }

    let op = svc.collect_fees(&address).await?;
    if op.success {
        Ok(Json(MessageResponse::new(format!(
            "Fees collected from position: {address}"
        ))))
    } else {
        Err(ApiError::ServiceUnavailable(
            op.error
                .unwrap_or_else(|| "Fee collection failed".to_string()),
        ))
    }
}

/// Decrease liquidity from a position.
#[utoipa::path(
    post,
    path = "/positions/{address}/decrease",
    tag = "Positions",
    params(
        ("address" = String, Path, description = "Position address")
    ),
    request_body = DecreaseLiquidityRequest,
    responses(
        (status = 200, description = "Liquidity decreased", body = MessageResponse),
        (status = 404, description = "Position not found")
    )
)]
pub async fn decrease_liquidity(
    State(state): State<AppState>,
    Path(address): Path<String>,
    Json(request): Json<DecreaseLiquidityRequest>,
) -> ApiResult<Json<MessageResponse>> {
    let mut svc = PositionService::new(state.clone());
    svc.set_dry_run(state.dry_run);

    if !state.dry_run {
        if let Some(exec) = state.executors.read().await.values().next().cloned() {
            svc.set_executor(exec);
        }
    }

    let op = svc
        .decrease_liquidity(&address, request.liquidity_amount)
        .await?;
    if op.success {
        Ok(Json(MessageResponse::new(format!(
            "Liquidity decreased for position: {address}"
        ))))
    } else {
        Err(ApiError::ServiceUnavailable(
            op.error
                .unwrap_or_else(|| "Decrease liquidity failed".to_string()),
        ))
    }
}

/// Rebalance a position.
#[utoipa::path(
    post,
    path = "/positions/{address}/rebalance",
    tag = "Positions",
    params(
        ("address" = String, Path, description = "Position address")
    ),
    request_body = RebalanceRequest,
    responses(
        (status = 200, description = "Position rebalanced", body = MessageResponse),
        (status = 404, description = "Position not found")
    )
)]
pub async fn rebalance_position(
    State(state): State<AppState>,
    Path(address): Path<String>,
    Json(request): Json<RebalanceRequest>,
) -> ApiResult<Json<MessageResponse>> {
    let mut svc = PositionService::new(state.clone());
    svc.set_dry_run(state.dry_run);

    if !state.dry_run {
        if let Some(exec) = state.executors.read().await.values().next().cloned() {
            svc.set_executor(exec);
        }
    }

    let op = svc.rebalance_position(&address, &request).await?;
    if op.success {
        Ok(Json(MessageResponse::new(
            "Rebalance requested".to_string(),
        )))
    } else {
        Err(ApiError::ServiceUnavailable(
            op.error.unwrap_or_else(|| "Rebalance failed".to_string()),
        ))
    }
}

/// Get position PnL details.
#[utoipa::path(
    get,
    path = "/positions/{address}/pnl",
    tag = "Positions",
    params(
        ("address" = String, Path, description = "Position address")
    ),
    responses(
        (status = 200, description = "Position PnL", body = PnLResponse),
        (status = 404, description = "Position not found")
    )
)]
pub async fn get_position_pnl(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> ApiResult<Json<PnLResponse>> {
    let pubkey = Pubkey::from_str(&address)
        .map_err(|_| ApiError::bad_request("Invalid position address"))?;

    let positions = state.monitor.get_positions().await;
    let position = positions
        .iter()
        .find(|p| p.address == pubkey)
        .ok_or_else(|| ApiError::not_found("Position not found"))?;

    let response = PnLResponse {
        unrealized_pnl_usd: position.pnl.net_pnl_usd,
        unrealized_pnl_pct: position.pnl.net_pnl_pct,
        fees_earned_a: position.pnl.fees_earned_a,
        fees_earned_b: position.pnl.fees_earned_b,
        fees_earned_usd: position.pnl.fees_usd,
        il_pct: position.pnl.il_pct,
        net_pnl_usd: position.pnl.net_pnl_usd,
        net_pnl_pct: position.pnl.net_pnl_pct,
    };

    Ok(Json(response))
}
