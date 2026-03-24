//! Position handlers.

use crate::error::{ApiError, ApiResult};
use crate::models::{
    ListPositionsResponse, MessageResponse, OpenPositionRequest, PnLResponse, PositionResponse,
    PositionStatus, RebalanceRequest,
};
use crate::state::{AlertUpdate, AppState, PositionUpdate};
use axum::{
    Json,
    extract::{Path, State},
};
use clmm_lp_execution::prelude::{RebalanceData, RebalanceReason};
use clmm_lp_protocols::prelude::WhirlpoolReader;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;
use tracing::{info, warn};

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

    // Validate tick range
    if request.tick_lower >= request.tick_upper {
        return Err(ApiError::Validation(
            "tick_lower must be less than tick_upper".to_string(),
        ));
    }

    // Validate pool exists
    let pool_reader = WhirlpoolReader::new(state.provider.clone());
    let pool_state = pool_reader
        .get_pool_state(&request.pool_address)
        .await
        .map_err(|e| ApiError::not_found(format!("Pool not found: {}", e)))?;

    // Validate tick spacing
    let tick_spacing = pool_state.tick_spacing as i32;
    if request.tick_lower % tick_spacing != 0 || request.tick_upper % tick_spacing != 0 {
        return Err(ApiError::Validation(format!(
            "Tick bounds must be multiples of tick spacing ({})",
            tick_spacing
        )));
    }

    if state.dry_run {
        info!("Dry-run mode: would open position");
        return Ok(Json(MessageResponse::new(format!(
            "[DRY-RUN] Would open position in pool {} with range [{}, {}]",
            request.pool_address, request.tick_lower, request.tick_upper
        ))));
    }

    // Actual execution requires wallet configuration
    warn!("Position opening requires wallet configuration");
    Ok(Json(MessageResponse::new(
        "Position opening requires wallet configuration. Set up wallet first.",
    )))
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
        state.broadcast_position_update(PositionUpdate {
            update_type: "close_simulated".to_string(),
            position_address: address.clone(),
            timestamp: chrono::Utc::now(),
            data: serde_json::json!({
                "liquidity": position.on_chain.liquidity.to_string(),
                "dry_run": true
            }),
        });

        return Ok(Json(MessageResponse::new(format!(
            "[DRY-RUN] Would close position {} with liquidity {}",
            address, position.on_chain.liquidity
        ))));
    }

    // Actual execution requires wallet configuration
    warn!("Position closing requires wallet configuration");
    Ok(Json(MessageResponse::new(
        "Position closing requires wallet configuration. Set up wallet first.",
    )))
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
        state.broadcast_position_update(PositionUpdate {
            update_type: "fees_collected_simulated".to_string(),
            position_address: address.clone(),
            timestamp: chrono::Utc::now(),
            data: serde_json::json!({
                "fees_a": position.pnl.fees_earned_a,
                "fees_b": position.pnl.fees_earned_b,
                "dry_run": true
            }),
        });

        return Ok(Json(MessageResponse::new(format!(
            "[DRY-RUN] Would collect fees from position {}: {} token A, {} token B",
            address, position.pnl.fees_earned_a, position.pnl.fees_earned_b
        ))));
    }

    // Actual execution requires wallet configuration
    warn!("Fee collection requires wallet configuration");
    Ok(Json(MessageResponse::new(
        "Fee collection requires wallet configuration. Set up wallet first.",
    )))
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
    let pubkey = Pubkey::from_str(&address)
        .map_err(|_| ApiError::bad_request("Invalid position address"))?;

    info!(
        position = %address,
        new_tick_lower = request.new_tick_lower,
        new_tick_upper = request.new_tick_upper,
        dry_run = state.dry_run,
        "Rebalancing position"
    );

    // Validate tick range
    if request.new_tick_lower >= request.new_tick_upper {
        return Err(ApiError::Validation(
            "new_tick_lower must be less than new_tick_upper".to_string(),
        ));
    }

    // Verify position exists
    let positions = state.monitor.get_positions().await;
    let position = positions
        .iter()
        .find(|p| p.address == pubkey)
        .ok_or_else(|| ApiError::not_found("Position not found"))?;

    // Fetch pool state for validation
    let pool_reader = WhirlpoolReader::new(state.provider.clone());
    let pool_state = pool_reader
        .get_pool_state(&position.pool.to_string())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to fetch pool state: {}", e)))?;

    // Validate tick spacing
    let tick_spacing = pool_state.tick_spacing as i32;
    if request.new_tick_lower % tick_spacing != 0 || request.new_tick_upper % tick_spacing != 0 {
        return Err(ApiError::Validation(format!(
            "Tick bounds must be multiples of tick spacing ({})",
            tick_spacing
        )));
    }

    if state.dry_run {
        info!("Dry-run mode: would rebalance position");

        // Broadcast simulated update
        state.broadcast_position_update(PositionUpdate {
            update_type: "rebalance_simulated".to_string(),
            position_address: address.clone(),
            timestamp: chrono::Utc::now(),
            data: serde_json::json!({
                "old_range": [position.on_chain.tick_lower, position.on_chain.tick_upper],
                "new_range": [request.new_tick_lower, request.new_tick_upper],
                "dry_run": true
            }),
        });

        return Ok(Json(MessageResponse::new(format!(
            "[DRY-RUN] Would rebalance position {} from [{}, {}] to [{}, {}]",
            address,
            position.on_chain.tick_lower,
            position.on_chain.tick_upper,
            request.new_tick_lower,
            request.new_tick_upper
        ))));
    }

    // Record rebalance intent in lifecycle tracker
    state
        .lifecycle
        .record_rebalance(
            pubkey,
            position.pool,
            RebalanceData {
                old_tick_lower: position.on_chain.tick_lower,
                old_tick_upper: position.on_chain.tick_upper,
                new_tick_lower: request.new_tick_lower,
                new_tick_upper: request.new_tick_upper,
                old_liquidity: position.on_chain.liquidity,
                new_liquidity: position.on_chain.liquidity,
                tx_cost_lamports: 0,
                il_at_rebalance: position.pnl.il_pct,
                reason: RebalanceReason::Manual,
                amount_a_before: None,
                amount_b_before: None,
                amount_a_after: None,
                amount_b_after: None,
                price_ab_before: None,
                price_ab_after: None,
                fees_a_collected: None,
                fees_b_collected: None,
                optimization_run_id: None,
            },
        )
        .await;

    // Broadcast update
    state.broadcast_position_update(PositionUpdate {
        update_type: "rebalance_initiated".to_string(),
        position_address: address.clone(),
        timestamp: chrono::Utc::now(),
        data: serde_json::json!({
            "old_range": [position.on_chain.tick_lower, position.on_chain.tick_upper],
            "new_range": [request.new_tick_lower, request.new_tick_upper]
        }),
    });

    // Broadcast alert
    state.broadcast_alert(AlertUpdate {
        level: "info".to_string(),
        message: format!("Rebalance initiated for position {}", address),
        timestamp: chrono::Utc::now(),
        position_address: Some(address.clone()),
    });

    // Actual execution requires wallet configuration
    warn!("Rebalance recorded - actual execution requires wallet configuration");
    Ok(Json(MessageResponse::new(
        "Rebalance recorded. Actual execution requires wallet configuration.",
    )))
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
