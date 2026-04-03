//! Position handlers.

use crate::error::{ApiError, ApiResult};
use crate::handlers::strategies::ensure_strategy_running_after_position_link;
use crate::models::{
    DecreaseLiquidityRequest, ListPositionsResponse, MessageResponse, OpenPositionRequest,
    PnLResponse, PositionOpenResponse, PositionResponse, PositionStatus, RebalanceRequest,
    SwapBeforeOpenRequest, SwapBeforeOpenResponse,
};
use crate::services::strategy_service::append_position_address_to_strategy;
use crate::state::{AppState, PositionUpdate};
use axum::{
    Json,
    extract::{Path, State},
};
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;
use tracing::{info, warn};

use crate::position_registry_seed::registry_open_position_pubkeys;
use crate::services::PositionService;
use crate::services::position_executor::resolve_executor_for_position_ops;
use crate::services::position_valuation::{
    compute_position_usd_valuation, fetch_prices_for_positions, monitored_position_from_chain,
};
use std::collections::HashSet;

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
    let mut positions = state.monitor.get_positions().await;
    let monitored: HashSet<Pubkey> = positions.iter().map(|p| p.address).collect();

    // Registry remembers opens across restarts; monitor can be empty or miss a PDA. Merge chain state
    // for registry opens not yet in monitor so `GET /positions` matches what users see on-chain.
    for pk in registry_open_position_pubkeys() {
        if monitored.contains(&pk) {
            continue;
        }
        match monitored_position_from_chain(state.provider.clone(), &pk).await {
            Ok(p) => {
                positions.push(p);
            }
            Err(e) => {
                warn!(
                    position = %pk,
                    error = %e,
                    "list_positions: registry open but not on-chain or RPC error; skipping"
                );
            }
        }
    }

    let prices = fetch_prices_for_positions(state.provider.clone(), &positions).await;

    let mut responses: Vec<PositionResponse> = Vec::with_capacity(positions.len());
    for p in &positions {
        let valuation =
            match compute_position_usd_valuation(state.provider.clone(), p, &prices).await {
                Ok(v) => Some(v),
                Err(e) => {
                    warn!(
                        position = %p.address,
                        pool = %p.pool,
                        error = %e,
                        "USD valuation failed; falling back to monitor zeros"
                    );
                    None
                }
            };

        let value_usd = valuation
            .as_ref()
            .map(|v| v.value_usd)
            .unwrap_or(p.pnl.current_value_usd);
        let fees_usd = valuation
            .as_ref()
            .map(|v| v.fees_usd)
            .unwrap_or(p.pnl.fees_usd);

        responses.push(PositionResponse {
            address: p.address.to_string(),
            pool_address: p.pool.to_string(),
            owner: p.on_chain.owner.to_string(),
            tick_lower: p.on_chain.tick_lower,
            tick_upper: p.on_chain.tick_upper,
            liquidity: p.on_chain.liquidity.to_string(),
            in_range: p.in_range,
            value_usd,
            pnl: PnLResponse {
                unrealized_pnl_usd: p.pnl.net_pnl_usd,
                unrealized_pnl_pct: p.pnl.net_pnl_pct,
                fees_earned_a: p.pnl.fees_earned_a,
                fees_earned_b: p.pnl.fees_earned_b,
                fees_earned_usd: fees_usd,
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
        });
    }

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
    let position = if let Some(p) = positions.iter().find(|p| p.address == pubkey) {
        p.clone()
    } else {
        let p = monitored_position_from_chain(state.provider.clone(), &pubkey).await?;
        let st = state.clone();
        let addr = address.clone();
        tokio::spawn(async move {
            if let Err(e) = st.monitor.add_position(&addr).await {
                warn!(
                    error = %e,
                    position = %addr,
                    "get_position fallback: monitor.add_position failed (detail still returned)"
                );
            }
        });
        p
    };

    let prices =
        fetch_prices_for_positions(state.provider.clone(), std::slice::from_ref(&position)).await;
    let valuation = compute_position_usd_valuation(state.provider.clone(), &position, &prices)
        .await
        .ok();
    let value_usd = valuation
        .as_ref()
        .map(|v| v.value_usd)
        .unwrap_or(position.pnl.current_value_usd);
    let fees_usd = valuation
        .as_ref()
        .map(|v| v.fees_usd)
        .unwrap_or(position.pnl.fees_usd);

    let response = PositionResponse {
        address: position.address.to_string(),
        pool_address: position.pool.to_string(),
        owner: position.on_chain.owner.to_string(),
        tick_lower: position.on_chain.tick_lower,
        tick_upper: position.on_chain.tick_upper,
        liquidity: position.on_chain.liquidity.to_string(),
        in_range: position.in_range,
        value_usd,
        pnl: PnLResponse {
            unrealized_pnl_usd: position.pnl.net_pnl_usd,
            unrealized_pnl_pct: position.pnl.net_pnl_pct,
            fees_earned_a: position.pnl.fees_earned_a,
            fees_earned_b: position.pnl.fees_earned_b,
            fees_earned_usd: fees_usd,
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

/// Executes an Orca Whirlpool swap (ExactIn) inside the same pool (SWAP-only step).
#[utoipa::path(
    post,
    path = "/positions/swap-before-open",
    tag = "Positions",
    request_body = SwapBeforeOpenRequest,
    responses(
        (status = 200, description = "Swap executed", body = SwapBeforeOpenResponse),
        (status = 400, description = "Invalid request")
    )
)]
pub async fn swap_before_open(
    State(state): State<AppState>,
    Json(request): Json<SwapBeforeOpenRequest>,
) -> ApiResult<Json<SwapBeforeOpenResponse>> {
    info!(
        pool = %request.pool_address,
        specified_mint = %request.specified_mint,
        amount_in = request.amount_in,
        dry_run = state.dry_run,
        "Swapping before open (swap-only step)"
    );

    let mut svc = PositionService::new(state.clone());
    svc.set_dry_run(state.dry_run);

    if !state.dry_run {
        if let Some(exec) = resolve_executor_for_position_ops(&state).await {
            svc.set_executor(exec);
        }
    }

    let op = svc.swap_before_open_exact_in(&request).await?;

    if op.success {
        let data = op.data.as_ref();
        let message = data
            .and_then(|d| d.get("message"))
            .and_then(|v| v.as_str())
            .unwrap_or("Swap executed")
            .to_string();

        let swap_signature = data
            .and_then(|d| d.get("swap_signature"))
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string);

        let cost_session_id = request
            .cost_session_id
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        Ok(Json(SwapBeforeOpenResponse {
            message,
            swap_signature,
            cost_session_id,
        }))
    } else {
        Err(ApiError::ServiceUnavailable(
            op.error.unwrap_or_else(|| "Swap failed".to_string()),
        ))
    }
}

/// Open a new position.
#[utoipa::path(
    post,
    path = "/positions",
    tag = "Positions",
    request_body = OpenPositionRequest,
    responses(
        (status = 201, description = "Position opened", body = PositionOpenResponse),
        (status = 400, description = "Invalid request")
    )
)]
pub async fn open_position(
    State(state): State<AppState>,
    Json(request): Json<OpenPositionRequest>,
) -> ApiResult<Json<PositionOpenResponse>> {
    let strategy_id = request.strategy_id.clone();
    info!(
        pool = %request.pool_address,
        tick_lower = request.tick_lower,
        tick_upper = request.tick_upper,
        dry_run = state.dry_run,
        strategy_id = ?strategy_id.as_deref(),
        "Opening position"
    );

    if let Some(ref sid) = strategy_id {
        let strategies = state.strategies.read().await;
        strategies
            .get(sid)
            .ok_or_else(|| ApiError::not_found("Strategy not found"))?;
    }

    let mut svc = PositionService::new(state.clone());
    svc.set_dry_run(state.dry_run);

    // Non-dry-run: strategy executor or lazy KEYPAIR_PATH executor (swap/open work without a running strategy).
    if !state.dry_run {
        if let Some(exec) = resolve_executor_for_position_ops(&state).await {
            svc.set_executor(exec);
        }
    }

    let cost_session_id = request
        .cost_session_id
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let op = svc.open_position(&request).await?;
    if op.success {
        let data = op.data.as_ref();
        let position_pda_opt = data
            .and_then(|d| d.get("position_pda"))
            .and_then(|v| v.as_str());

        // New positions exist on-chain but `GET /positions/:addr` reads the in-memory monitor first.
        // Without this, the dashboard shows "Position not found" until API restart + registry seed.
        if !state.dry_run {
            if let Some(pda) = position_pda_opt {
                if let Err(e) = state.monitor.add_position(pda).await {
                    warn!(
                        error = %e,
                        position = %pda,
                        "open_position: monitor.add_position failed (detail may 404 until retry)"
                    );
                }
            }
        }

        // Legacy: response with only `message` and no PDA (avoid swallowing idempotent replay which has both).
        if let Some(m) = data.and_then(|d| d.get("message")).and_then(|v| v.as_str()) {
            if position_pda_opt.is_none() {
                return Ok(Json(PositionOpenResponse {
                    message: m.to_string(),
                    position_pda: None,
                    swap_signature: None,
                    cost_session_id,
                }));
            }
        }
        if let Some(pda) = position_pda_opt
        {
            let swap_signature = data
                .and_then(|d| d.get("swap_signature"))
                .and_then(|v| v.as_str())
                .map(std::string::ToString::to_string);
            let resp_cost_session = data
                .and_then(|d| d.get("cost_session_id"))
                .and_then(|v| v.as_str())
                .map(std::string::ToString::to_string)
                .or_else(|| cost_session_id.clone());

            if let Some(ref sid) = strategy_id {
                append_position_address_to_strategy(&state, sid, pda).await?;
                match ensure_strategy_running_after_position_link(&state, sid, pda).await {
                    Ok(()) => {
                        let mut msg = format!("Position opened. PDA: {pda}");
                        msg.push_str(" — linked to strategy; automation started.");
                        return Ok(Json(PositionOpenResponse {
                            message: msg,
                            position_pda: Some(pda.to_string()),
                            swap_signature,
                            cost_session_id: resp_cost_session,
                        }));
                    }
                    Err(e) => {
                        warn!(
                            error = %e,
                            strategy_id = %sid,
                            "Could not start strategy automation after position link"
                        );
                        let mut msg = format!("Position opened. PDA: {pda}");
                        msg.push_str(&format!(
                            " — linked to strategy; automation could not start: {}.",
                            e
                        ));
                        return Ok(Json(PositionOpenResponse {
                            message: msg,
                            position_pda: Some(pda.to_string()),
                            swap_signature,
                            cost_session_id: resp_cost_session,
                        }));
                    }
                }
            }
            let msg = data
                .and_then(|d| d.get("message"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("Position opened. PDA: {pda}"));
            return Ok(Json(PositionOpenResponse {
                message: msg,
                position_pda: Some(pda.to_string()),
                swap_signature,
                cost_session_id: resp_cost_session,
            }));
        }
        return Ok(Json(PositionOpenResponse {
            message: "Position opened".to_string(),
            position_pda: None,
            swap_signature: None,
            cost_session_id,
        }));
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
    if let Some(exec) = resolve_executor_for_position_ops(&state).await {
        svc.set_executor(exec);
    }

    let op = svc.close_position(&address).await?;
    if op.success {
        // Remove immediately so UI doesn't keep showing stale monitored entry.
        state.monitor.remove_position(&pubkey).await;
        state
            .broadcast_position_update(PositionUpdate {
                update_type: "closed".to_string(),
                position_address: address.clone(),
                timestamp: chrono::Utc::now(),
                data: serde_json::json!({}),
            })
            .await;

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
    if let Some(exec) = resolve_executor_for_position_ops(&state).await {
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
        if let Some(exec) = resolve_executor_for_position_ops(&state).await {
            svc.set_executor(exec);
        }
    }

    let liquidity_amount: u128 = request.liquidity_amount.trim().parse().map_err(|_| {
        ApiError::bad_request("liquidity_amount must be a non-negative decimal integer string")
    })?;

    let op = svc.decrease_liquidity(&address, liquidity_amount).await?;
    if op.success {
        if state.dry_run {
            let msg = op
                .data
                .as_ref()
                .and_then(|d| d.get("message"))
                .and_then(|v| v.as_str())
                .unwrap_or("Dry-run: liquidity decrease simulated");
            return Ok(Json(MessageResponse::new(format!("[DRY-RUN] {msg}"))));
        }
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
        if let Some(exec) = resolve_executor_for_position_ops(&state).await {
            svc.set_executor(exec);
        }
    }

    let op = svc.rebalance_position(&address, &request).await?;
    if op.success {
        if state.dry_run {
            let msg = op
                .data
                .as_ref()
                .and_then(|d| d.get("message"))
                .and_then(|v| v.as_str())
                .unwrap_or("Dry-run: rebalance simulated");
            return Ok(Json(MessageResponse::new(format!("[DRY-RUN] {msg}"))));
        }
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
