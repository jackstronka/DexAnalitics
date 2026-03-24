//! Strategy handlers.

use crate::error::{ApiError, ApiResult};
use crate::models::{
    CreateStrategyRequest, ListStrategiesResponse, MessageResponse, StrategyParameters,
    StrategyPerformanceResponse, StrategyResponse, StrategyType,
};
use crate::state::{AlertUpdate, AppState, StrategyState};
use axum::{
    Json,
    extract::{Path, State},
};
use clmm_lp_execution::prelude::{DecisionConfig, ExecutorConfig, StrategyExecutor};
use rust_decimal::Decimal;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

/// List all strategies.
#[utoipa::path(
    get,
    path = "/strategies",
    tag = "Strategies",
    responses(
        (status = 200, description = "List of strategies", body = ListStrategiesResponse)
    )
)]
pub async fn list_strategies(
    State(state): State<AppState>,
) -> ApiResult<Json<ListStrategiesResponse>> {
    let strategies = state.strategies.read().await;

    let responses: Vec<StrategyResponse> = strategies
        .values()
        .map(|s| {
            let params: StrategyParameters = s
                .config
                .get("parameters")
                .and_then(|p| serde_json::from_value(p.clone()).ok())
                .unwrap_or(StrategyParameters {
                    tick_width: None,
                    range_width_pct: None,
                    rebalance_threshold_pct: None,
                    max_il_pct: None,
                    eval_interval_secs: None,
                    min_rebalance_interval_hours: None,
                    optimize_on_start: None,
                    optimize_interval_secs: None,
                    optimize_command: None,
                    optimize_result_json_path: None,
                    il_ledger_path: None,
                });

            StrategyResponse {
                id: s.id.clone(),
                name: s.name.clone(),
                pool_address: s
                    .config
                    .get("pool_address")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                strategy_type: s
                    .config
                    .get("strategy_type")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or(StrategyType::StaticRange),
                parameters: params,
                running: s.running,
                dry_run: s
                    .config
                    .get("dry_run")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                created_at: s.created_at,
                updated_at: s.updated_at,
            }
        })
        .collect();

    Ok(Json(ListStrategiesResponse {
        total: responses.len(),
        strategies: responses,
    }))
}

/// Get a specific strategy.
#[utoipa::path(
    get,
    path = "/strategies/{id}",
    tag = "Strategies",
    params(
        ("id" = String, Path, description = "Strategy ID")
    ),
    responses(
        (status = 200, description = "Strategy details", body = StrategyResponse),
        (status = 404, description = "Strategy not found")
    )
)]
pub async fn get_strategy(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<StrategyResponse>> {
    let strategies = state.strategies.read().await;
    let strategy = strategies
        .get(&id)
        .ok_or_else(|| ApiError::not_found("Strategy not found"))?;

    let params: StrategyParameters = strategy
        .config
        .get("parameters")
        .and_then(|p| serde_json::from_value(p.clone()).ok())
        .unwrap_or(StrategyParameters {
            tick_width: None,
            range_width_pct: None,
            rebalance_threshold_pct: None,
            max_il_pct: None,
            eval_interval_secs: None,
            min_rebalance_interval_hours: None,
            optimize_on_start: None,
            optimize_interval_secs: None,
            optimize_command: None,
            optimize_result_json_path: None,
            il_ledger_path: None,
        });

    let response = StrategyResponse {
        id: strategy.id.clone(),
        name: strategy.name.clone(),
        pool_address: strategy
            .config
            .get("pool_address")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        strategy_type: strategy
            .config
            .get("strategy_type")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or(StrategyType::StaticRange),
        parameters: params,
        running: strategy.running,
        dry_run: strategy
            .config
            .get("dry_run")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        created_at: strategy.created_at,
        updated_at: strategy.updated_at,
    };

    Ok(Json(response))
}

/// Create a new strategy.
#[utoipa::path(
    post,
    path = "/strategies",
    tag = "Strategies",
    request_body = CreateStrategyRequest,
    responses(
        (status = 201, description = "Strategy created", body = StrategyResponse),
        (status = 400, description = "Invalid request")
    )
)]
pub async fn create_strategy(
    State(state): State<AppState>,
    Json(request): Json<CreateStrategyRequest>,
) -> ApiResult<Json<StrategyResponse>> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now();

    let config = serde_json::json!({
        "pool_address": request.pool_address,
        "strategy_type": request.strategy_type,
        "parameters": request.parameters,
        "auto_execute": request.auto_execute,
        "dry_run": request.dry_run,
    });

    let strategy_state = StrategyState {
        id: id.clone(),
        name: request.name.clone(),
        running: false,
        config: config.clone(),
        created_at: now,
        updated_at: now,
    };

    state
        .strategies
        .write()
        .await
        .insert(id.clone(), strategy_state);

    info!(id = %id, name = %request.name, "Strategy created");

    let response = StrategyResponse {
        id,
        name: request.name,
        pool_address: request.pool_address,
        strategy_type: request.strategy_type,
        parameters: request.parameters,
        running: false,
        dry_run: request.dry_run,
        created_at: now,
        updated_at: now,
    };

    Ok(Json(response))
}

/// Update a strategy.
#[utoipa::path(
    put,
    path = "/strategies/{id}",
    tag = "Strategies",
    params(
        ("id" = String, Path, description = "Strategy ID")
    ),
    request_body = CreateStrategyRequest,
    responses(
        (status = 200, description = "Strategy updated", body = StrategyResponse),
        (status = 404, description = "Strategy not found")
    )
)]
pub async fn update_strategy(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<CreateStrategyRequest>,
) -> ApiResult<Json<StrategyResponse>> {
    let mut strategies = state.strategies.write().await;
    let strategy = strategies
        .get_mut(&id)
        .ok_or_else(|| ApiError::not_found("Strategy not found"))?;

    let now = chrono::Utc::now();

    let config = serde_json::json!({
        "pool_address": request.pool_address,
        "strategy_type": request.strategy_type,
        "parameters": request.parameters,
        "auto_execute": request.auto_execute,
        "dry_run": request.dry_run,
    });

    strategy.name = request.name.clone();
    strategy.config = config;
    strategy.updated_at = now;

    info!(id = %id, "Strategy updated");

    let response = StrategyResponse {
        id,
        name: request.name,
        pool_address: request.pool_address,
        strategy_type: request.strategy_type,
        parameters: request.parameters,
        running: strategy.running,
        dry_run: request.dry_run,
        created_at: strategy.created_at,
        updated_at: now,
    };

    Ok(Json(response))
}

/// Delete a strategy.
#[utoipa::path(
    delete,
    path = "/strategies/{id}",
    tag = "Strategies",
    params(
        ("id" = String, Path, description = "Strategy ID")
    ),
    responses(
        (status = 200, description = "Strategy deleted", body = MessageResponse),
        (status = 404, description = "Strategy not found")
    )
)]
pub async fn delete_strategy(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<MessageResponse>> {
    let mut strategies = state.strategies.write().await;

    if strategies.remove(&id).is_none() {
        return Err(ApiError::not_found("Strategy not found"));
    }

    info!(id = %id, "Strategy deleted");

    Ok(Json(MessageResponse::new("Strategy deleted")))
}

/// Start a strategy.
#[utoipa::path(
    post,
    path = "/strategies/{id}/start",
    tag = "Strategies",
    params(
        ("id" = String, Path, description = "Strategy ID")
    ),
    responses(
        (status = 200, description = "Strategy started", body = MessageResponse),
        (status = 404, description = "Strategy not found")
    )
)]
pub async fn start_strategy(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<MessageResponse>> {
    // Get strategy configuration
    let strategy_config = {
        let mut strategies = state.strategies.write().await;
        let strategy = strategies
            .get_mut(&id)
            .ok_or_else(|| ApiError::not_found("Strategy not found"))?;

        if strategy.running {
            return Err(ApiError::Conflict(
                "Strategy is already running".to_string(),
            ));
        }

        strategy.running = true;
        strategy.updated_at = chrono::Utc::now();
        strategy.config.clone()
    };

    // Parse configuration
    let dry_run = strategy_config
        .get("dry_run")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let auto_execute = strategy_config
        .get("auto_execute")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let eval_interval_secs = strategy_config
        .get("parameters")
        .and_then(|p| p.get("eval_interval_secs"))
        .and_then(|v| v.as_u64())
        .unwrap_or(300);

    // Create executor configuration
    let executor_config = ExecutorConfig {
        eval_interval_secs,
        auto_execute,
        require_confirmation: !auto_execute,
        max_slippage_pct: Decimal::new(5, 3), // 0.5%
        dry_run,
    };

    // Create strategy executor
    let mut executor = StrategyExecutor::new(
        state.provider.clone(),
        state.monitor.clone(),
        state.tx_manager.clone(),
        executor_config,
    );

    // Configure decision engine if parameters provided
    if let Some(params) = strategy_config.get("parameters") {
        let mut decision_config = DecisionConfig::default();

        if let Some(threshold) = params.get("rebalance_threshold_pct")
            && let Some(val) = threshold.as_f64()
        {
            decision_config.il_rebalance_threshold =
                Decimal::from_f64_retain(val / 100.0).unwrap_or(Decimal::new(5, 2));
        }

        if let Some(max_il) = params.get("max_il_pct")
            && let Some(val) = max_il.as_f64()
        {
            decision_config.il_close_threshold =
                Decimal::from_f64_retain(val / 100.0).unwrap_or(Decimal::new(15, 2));
        }

        if let Some(min_hours) = params.get("min_rebalance_interval_hours")
            && let Some(val) = min_hours.as_u64()
        {
            decision_config.min_rebalance_interval_hours = val;
        }

        executor.set_decision_config(decision_config);
    }

    let executor = Arc::new(RwLock::new(executor));

    // Store executor
    {
        let mut executors = state.executors.write().await;
        executors.insert(id.clone(), executor.clone());
    }

    // Start executor in background task
    let executor_clone = executor.clone();
    let id_clone = id.clone();
    let alert_sender = state.alert_updates.clone();

    tokio::spawn(async move {
        info!(strategy_id = %id_clone, "Strategy executor task started");

        let executor_guard = executor_clone.read().await;
        executor_guard.start().await;

        // Notify when stopped
        let _ = alert_sender.send(AlertUpdate {
            level: "info".to_string(),
            message: format!("Strategy {} stopped", id_clone),
            timestamp: chrono::Utc::now(),
            position_address: None,
        });
    });

    // Broadcast alert
    state.broadcast_alert(AlertUpdate {
        level: "info".to_string(),
        message: format!("Strategy {} started", id),
        timestamp: chrono::Utc::now(),
        position_address: None,
    });

    info!(id = %id, dry_run = dry_run, auto_execute = auto_execute, "Strategy started");

    Ok(Json(MessageResponse::new(format!(
        "Strategy started (dry_run={}, auto_execute={})",
        dry_run, auto_execute
    ))))
}

/// Stop a strategy.
#[utoipa::path(
    post,
    path = "/strategies/{id}/stop",
    tag = "Strategies",
    params(
        ("id" = String, Path, description = "Strategy ID")
    ),
    responses(
        (status = 200, description = "Strategy stopped", body = MessageResponse),
        (status = 404, description = "Strategy not found")
    )
)]
pub async fn stop_strategy(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<MessageResponse>> {
    // Update strategy state
    {
        let mut strategies = state.strategies.write().await;
        let strategy = strategies
            .get_mut(&id)
            .ok_or_else(|| ApiError::not_found("Strategy not found"))?;

        if !strategy.running {
            return Err(ApiError::Conflict("Strategy is not running".to_string()));
        }

        strategy.running = false;
        strategy.updated_at = chrono::Utc::now();
    }

    // Stop the executor
    {
        let executors = state.executors.read().await;
        if let Some(executor) = executors.get(&id) {
            let executor_guard = executor.read().await;
            executor_guard.stop();
            info!(id = %id, "Strategy executor stopped");
        }
    }

    // Remove executor from map
    {
        let mut executors = state.executors.write().await;
        executors.remove(&id);
    }

    // Broadcast alert
    state.broadcast_alert(AlertUpdate {
        level: "info".to_string(),
        message: format!("Strategy {} stopped", id),
        timestamp: chrono::Utc::now(),
        position_address: None,
    });

    info!(id = %id, "Strategy stopped");

    Ok(Json(MessageResponse::new("Strategy stopped")))
}

/// Get strategy performance.
#[utoipa::path(
    get,
    path = "/strategies/{id}/performance",
    tag = "Strategies",
    params(
        ("id" = String, Path, description = "Strategy ID")
    ),
    responses(
        (status = 200, description = "Strategy performance", body = StrategyPerformanceResponse),
        (status = 404, description = "Strategy not found")
    )
)]
pub async fn get_strategy_performance(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<StrategyPerformanceResponse>> {
    let strategies = state.strategies.read().await;
    if !strategies.contains_key(&id) {
        return Err(ApiError::not_found("Strategy not found"));
    }

    // Get aggregate stats from lifecycle tracker
    let stats = state.lifecycle.get_aggregate_stats().await;

    let response = StrategyPerformanceResponse {
        strategy_id: id,
        total_pnl_usd: stats.total_pnl_usd,
        total_pnl_pct: stats.avg_pnl_pct,
        total_fees_usd: stats.total_fees_usd,
        total_il_pct: Decimal::ZERO, // Would need to track per strategy
        rebalance_count: stats.total_rebalances,
        total_tx_costs_lamports: stats.total_tx_costs_lamports,
        win_rate_pct: Decimal::ZERO, // Would need to track per strategy
    };

    Ok(Json(response))
}
