//! Strategy handlers.

use crate::error::{ApiError, ApiResult};
use crate::models::{
    ApplyOptimizeResultRequest, CreateStrategyRequest, ListStrategiesResponse, MessageResponse,
    OptimizeApplyPolicy, StrategyParameters, StrategyPerformanceResponse,
    StrategyPositionExecutorRequest, StrategyResponse, StrategyType,
};
use crate::services::optimization_runner::{
    apply_optimize_result_parsed, end_optimize_busy, try_begin_optimize_busy,
};
use crate::services::position_executor::load_wallet_from_env;
use crate::state::{AlertUpdate, AppState, StrategyState};
use axum::{
    Json,
    extract::{Path, State},
};
use clmm_lp_domain::prelude::PositionTruthMode;
use clmm_lp_execution::prelude::{
    DecisionConfig, ExecutorConfig, StrategyExecutor, validate_agent_decision,
};
use rust_decimal::Decimal;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Deserialize `parameters` from persisted strategy JSON. Overlays `position_addresses` and
/// `executor_disabled_position_addresses` from the **raw** `parameters` object so linked PDAs are
/// still returned when strict [`StrategyParameters`] deserialization fails (e.g. odd `Decimal`
/// encodings in older configs) — otherwise `GET /strategies` dropped them and the UI showed
/// "None linked" after Open Position.
fn strategy_parameters_from_stored_config(
    parameters: Option<&serde_json::Value>,
) -> StrategyParameters {
    let Some(p) = parameters else {
        return StrategyParameters::default();
    };
    let mut params: StrategyParameters = serde_json::from_value(p.clone()).unwrap_or_default();
    if let Some(arr) = p.get("position_addresses").and_then(|v| v.as_array()) {
        let addrs: Vec<String> = arr
            .iter()
            .filter_map(|x| x.as_str().map(std::string::ToString::to_string))
            .collect();
        if !addrs.is_empty() {
            params.position_addresses = Some(addrs);
        }
    }
    if let Some(arr) = p
        .get("executor_disabled_position_addresses")
        .and_then(|v| v.as_array())
    {
        let addrs: Vec<String> = arr
            .iter()
            .filter_map(|x| x.as_str().map(std::string::ToString::to_string))
            .collect();
        if !addrs.is_empty() {
            params.executor_disabled_position_addresses = Some(addrs);
        }
    }
    params
}

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
            let params = strategy_parameters_from_stored_config(s.config.get("parameters"));

            StrategyResponse {
                id: s.id.clone(),
                name: s.name.clone(),
                pool_address: s
                    .config
                    .get("pool_address")
                    .and_then(|v| v.as_str())
                    .filter(|p| !p.is_empty())
                    .map(str::to_string),
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
                auto_execute: s
                    .config
                    .get("auto_execute")
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

    let params = strategy_parameters_from_stored_config(strategy.config.get("parameters"));

    let response = StrategyResponse {
        id: strategy.id.clone(),
        name: strategy.name.clone(),
        pool_address: strategy
            .config
            .get("pool_address")
            .and_then(|v| v.as_str())
            .filter(|p| !p.is_empty())
            .map(str::to_string),
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
        auto_execute: strategy
            .config
            .get("auto_execute")
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

    let mut config = serde_json::json!({
        "strategy_type": request.strategy_type,
        "parameters": request.parameters,
        "auto_execute": request.auto_execute,
        "dry_run": request.dry_run,
    });
    if let Some(ref p) = request.pool_address {
        let t = p.trim();
        if !t.is_empty() {
            if let Some(obj) = config.as_object_mut() {
                obj.insert(
                    "pool_address".to_string(),
                    serde_json::Value::String(t.to_string()),
                );
            }
        }
    }

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

    // Persist strategy config so it survives API restarts.
    let snapshot = state.strategies.read().await.clone();
    crate::state::try_persist_strategies_best_effort(&snapshot);

    info!(id = %id, name = %request.name, "Strategy created");

    let response = StrategyResponse {
        id,
        name: request.name,
        pool_address: None,
        strategy_type: request.strategy_type,
        parameters: request.parameters,
        running: false,
        dry_run: request.dry_run,
        auto_execute: request.auto_execute,
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
    let was_running = {
        let strategies = state.strategies.read().await;
        strategies.get(&id).map(|s| s.running).unwrap_or(false)
    };

    let response = {
        let mut strategies = state.strategies.write().await;
        let strategy = match strategies.get_mut(&id) {
            Some(s) => s,
            None => return Err(ApiError::not_found("Strategy not found")),
        };

        let now = chrono::Utc::now();

        let old_position_addrs = strategy
            .config
            .get("parameters")
            .and_then(|p| p.get("position_addresses"))
            .cloned();
        let old_executor_disabled = strategy
            .config
            .get("parameters")
            .and_then(|p| p.get("executor_disabled_position_addresses"))
            .cloned();
        let old_pool_addr = strategy.config.get("pool_address").cloned();

        let mut config = serde_json::json!({
            "strategy_type": request.strategy_type,
            "parameters": request.parameters,
            "auto_execute": request.auto_execute,
            "dry_run": request.dry_run,
        });

        match &request.pool_address {
            Some(p) if !p.trim().is_empty() => {
                if let Some(obj) = config.as_object_mut() {
                    obj.insert(
                        "pool_address".to_string(),
                        serde_json::Value::String(p.trim().to_string()),
                    );
                }
            }
            Some(_) => { /* clear legacy pool — do not copy old */ }
            None => {
                if let Some(p) = old_pool_addr {
                    if let Some(obj) = config.as_object_mut() {
                        obj.insert("pool_address".to_string(), p);
                    }
                }
            }
        }

        if let Some(addrs) = old_position_addrs {
            if let Some(params) = config.get_mut("parameters").and_then(|p| p.as_object_mut()) {
                params.insert("position_addresses".to_string(), addrs);
            }
        }
        if let Some(disabled) = old_executor_disabled {
            if let Some(params) = config.get_mut("parameters").and_then(|p| p.as_object_mut()) {
                if !params.contains_key("executor_disabled_position_addresses") {
                    params.insert("executor_disabled_position_addresses".to_string(), disabled);
                }
            }
        }

        strategy.name = request.name.clone();
        strategy.config = config;
        strategy.updated_at = now;

        info!(id = %id, "Strategy updated");

        let params = strategy_parameters_from_stored_config(strategy.config.get("parameters"));

        StrategyResponse {
            id: id.clone(),
            name: request.name,
            pool_address: strategy
                .config
                .get("pool_address")
                .and_then(|v| v.as_str())
                .filter(|p| !p.is_empty())
                .map(str::to_string),
            strategy_type: request.strategy_type,
            parameters: params,
            running: strategy.running,
            dry_run: request.dry_run,
            auto_execute: request.auto_execute,
            created_at: strategy.created_at,
            updated_at: now,
        }
    };

    sync_executor_disabled_from_config(&state, &id).await?;

    // Persist strategy config so it survives API restarts.
    let snapshot = state.strategies.read().await.clone();
    crate::state::try_persist_strategies_best_effort(&snapshot);

    // If strategy is running, it must be restarted to pick up executor-level flags like
    // `auto_execute`, `dry_run`, and `eval_interval_secs` (they are read only on executor start).
    if was_running {
        // Stop current executor (if any) and remove it from the map.
        {
            let exec_opt = { state.executors.read().await.get(&id).cloned() };
            if let Some(exec) = exec_opt {
                exec.read().await.stop();
            }
        }
        {
            let mut execs = state.executors.write().await;
            execs.remove(&id);
        }

        // Start a fresh executor using the updated persisted config.
        let cfg = {
            let strategies = state.strategies.read().await;
            strategies
                .get(&id)
                .map(|s| s.config.clone())
                .unwrap_or(serde_json::json!({}))
        };
        let _ = start_strategy_executor_core(&state, &id, cfg).await?;
    }

    Ok(Json(response))
}

/// Apply a parsed `OptimizeResultFile` (or agent envelope) to a running strategy executor without running `backtest-optimize`.
///
/// Request JSON: either a full [`OptimizeResultFile`](clmm_lp_domain::optimize_result::OptimizeResultFile), or
/// `{ "decision": { ... }, "baseline_optimize_result": ... }` with [`AgentDecision`](clmm_lp_domain::agent_decision::AgentDecision).
#[utoipa::path(
    post,
    path = "/strategies/{id}/apply-optimize-result",
    tag = "Strategies",
    params(
        ("id" = String, Path, description = "Strategy ID")
    ),
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Applied or no-op", body = MessageResponse),
        (status = 404, description = "Strategy not found"),
        (status = 409, description = "Strategy not running, optimize_apply_policy blocks HTTP apply, or busy")
    )
)]
pub async fn apply_optimize_result(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult<Json<MessageResponse>> {
    let body: ApplyOptimizeResultRequest = serde_json::from_value(body)
        .map_err(|e| ApiError::bad_request(format!("invalid apply-optimize-result JSON: {e}")))?;

    let (params, max_delta) = {
        let strategies = state.strategies.read().await;
        let strategy = strategies
            .get(&id)
            .ok_or_else(|| ApiError::not_found("Strategy not found"))?;
        if !strategy.running {
            return Err(ApiError::Conflict(
                "Strategy is not running; apply-optimize-result requires an active executor"
                    .to_string(),
            ));
        }
        let params = strategy_parameters_from_stored_config(strategy.config.get("parameters"));
        let max_delta = params.agent_max_width_pct_delta;
        (params, max_delta)
    };

    let executor = {
        let executors = state.executors.read().await;
        executors
            .get(&id)
            .cloned()
            .ok_or_else(|| ApiError::not_found("Strategy executor not found"))?
    };

    match &body {
        ApplyOptimizeResultRequest::Agent(env) => {
            validate_agent_decision(
                &env.decision,
                env.baseline_optimize_result.as_ref(),
                max_delta,
            )?;
            if !env.decision.approved {
                return Ok(Json(MessageResponse::new(
                    "Agent decision: not approved; executor unchanged",
                )));
            }
        }
        ApplyOptimizeResultRequest::Direct(_) => {}
    }

    if params.optimize_apply_policy == OptimizeApplyPolicy::PeriodicSubprocess {
        return Err(ApiError::Conflict(
            "parameters.optimize_apply_policy is periodic_subprocess; HTTP apply is disabled"
                .to_string(),
        ));
    }

    let busy = {
        let mut m = state.optimization_busy.write().await;
        m.entry(id.clone())
            .or_insert_with(|| Arc::new(AtomicBool::new(false)))
            .clone()
    };

    if !try_begin_optimize_busy(&busy) {
        return Err(ApiError::Conflict(
            "Another optimization or apply-optimize-result is in progress for this strategy"
                .to_string(),
        ));
    }

    let apply_result = async {
        match &body {
            ApplyOptimizeResultRequest::Direct(file) => {
                apply_optimize_result_parsed(file, executor.as_ref()).await
            }
            ApplyOptimizeResultRequest::Agent(env) => {
                let file = env.decision.optimize_result.as_ref().ok_or_else(|| {
                    ApiError::bad_request("approved agent decision missing optimize_result")
                })?;
                apply_optimize_result_parsed(file, executor.as_ref()).await
            }
        }
    }
    .await;

    end_optimize_busy(&busy);
    apply_result?;

    info!(strategy_id = %id, "apply-optimize-result applied");
    Ok(Json(MessageResponse::new("Optimize result applied")))
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
    {
        let mut strategies = state.strategies.write().await;
        let Some(strategy) = strategies.get_mut(&id) else {
            return Err(ApiError::not_found("Strategy not found"));
        };
        if strategy.running {
            strategy.running = false;
            strategy.updated_at = chrono::Utc::now();
        }
    }

    {
        let executors = state.executors.read().await;
        if let Some(executor) = executors.get(&id) {
            let executor_guard = executor.read().await;
            executor_guard.stop();
            info!(strategy_id = %id, "Strategy executor stopped before delete");
        }
    }
    {
        let mut executors = state.executors.write().await;
        executors.remove(&id);
    }
    {
        let mut busy = state.optimization_busy.write().await;
        busy.remove(&id);
    }

    {
        let mut strategies = state.strategies.write().await;
        if strategies.remove(&id).is_none() {
            return Err(ApiError::not_found("Strategy not found"));
        }
    }

    // Persist strategies store (deleted strategy must disappear after restart).
    let snapshot = state.strategies.read().await.clone();
    crate::state::try_persist_strategies_best_effort(&snapshot);

    info!(id = %id, "Strategy deleted");

    Ok(Json(MessageResponse::new("Strategy deleted")))
}

/// Push `executor_disabled_position_addresses` from config into the running executor (if any).
pub(crate) async fn sync_executor_disabled_from_config(
    state: &AppState,
    strategy_id: &str,
) -> ApiResult<()> {
    let addrs: Vec<String> = {
        let strategies = state.strategies.read().await;
        let Some(strategy) = strategies.get(strategy_id) else {
            return Ok(());
        };
        strategy
            .config
            .get("parameters")
            .and_then(|p| p.get("executor_disabled_position_addresses"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str().map(std::string::ToString::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    let executors = state.executors.read().await;
    let Some(exec) = executors.get(strategy_id) else {
        return Ok(());
    };
    let g = exec.read().await;
    g.set_skip_evaluation_for_addresses(&addrs).await;
    Ok(())
}

/// Starts the executor loop (caller must set `strategy.running = true` and pass current `config` JSON).
///
/// Returns `Ok(Some(warning))` when one or more `position_addresses` could not be added to the
/// in-memory monitor (typically RPC `get_account` failure); the executor still starts so the
/// strategy link in config is not blocked by transient RPC issues.
async fn start_strategy_executor_core(
    state: &AppState,
    id: &str,
    strategy_config: serde_json::Value,
) -> ApiResult<Option<String>> {
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

    let mut monitor_failures: Vec<String> = Vec::new();
    if let Some(addrs) = strategy_config
        .get("parameters")
        .and_then(|p| p.get("position_addresses"))
        .and_then(|v| v.as_array())
    {
        for a in addrs {
            if let Some(s) = a.as_str() {
                if let Err(e) = state.monitor.add_position(s).await {
                    warn!(
                        position = %s,
                        error = %e,
                        "start_strategy: monitor.add_position failed (RPC); continuing without this PDA in monitor"
                    );
                    monitor_failures.push(format!("{s}: {e}"));
                }
            } else {
                return Err(ApiError::bad_request(
                    "parameters.position_addresses must be array of strings",
                ));
            }
        }
    }

    let monitor_note = if monitor_failures.is_empty() {
        None
    } else {
        Some(format!(
            "Monitor could not load {} position(s) from RPC (strategy still started): {}",
            monitor_failures.len(),
            monitor_failures.join("; ")
        ))
    };

    let executor_config = ExecutorConfig {
        eval_interval_secs,
        auto_execute,
        require_confirmation: !auto_execute,
        max_slippage_pct: Decimal::new(5, 3), // 0.5%
        dry_run,
        fee_mode: PositionTruthMode::Heuristic,
    };

    let mut executor = StrategyExecutor::new(
        state.provider.clone(),
        state.monitor.clone(),
        state.tx_manager.clone(),
        executor_config,
    );

    executor.set_position_fee_ledger_path(Some(std::path::PathBuf::from(
        "data/position-fee-checkpoints.jsonl",
    )));

    if !dry_run {
        match load_wallet_from_env() {
            Ok(Some(wallet)) => executor.set_wallet(wallet),
            Ok(None) => {
                if auto_execute {
                    return Err(ApiError::bad_request(
                        "auto_execute=true requires KEYPAIR_PATH or SOLANA_KEYPAIR_PATH",
                    ));
                }
            }
            Err(e) => return Err(e),
        }
    }

    if let Some(params) = strategy_config.get("parameters") {
        let mut decision_config = DecisionConfig::default();
        let strategy_type: StrategyType = strategy_config
            .get("strategy_type")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or(StrategyType::StaticRange);

        decision_config.strategy_mode = match strategy_type {
            StrategyType::StaticRange => clmm_lp_execution::prelude::StrategyMode::StaticRange,
            StrategyType::Periodic => clmm_lp_execution::prelude::StrategyMode::Periodic,
            StrategyType::Threshold => clmm_lp_execution::prelude::StrategyMode::Threshold,
            StrategyType::OorRecenter => clmm_lp_execution::prelude::StrategyMode::OorRecenter,
            StrategyType::IlLimit => clmm_lp_execution::prelude::StrategyMode::IlLimit,
            StrategyType::RetouchShift => clmm_lp_execution::prelude::StrategyMode::RetouchShift,
        };

        // Common: width and periodic / thresholds.
        if let Some(range_width_pct) = params.get("range_width_pct").and_then(|v| v.as_f64()) {
            decision_config.range_width_pct = Decimal::from_f64_retain(range_width_pct / 100.0)
                .unwrap_or(decision_config.range_width_pct);
        }

        if let Some(threshold) = params.get("rebalance_threshold_pct")
            && let Some(val) = threshold.as_f64()
        {
            decision_config.threshold_pct =
                Decimal::from_f64_retain(val / 100.0).unwrap_or(decision_config.threshold_pct);
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
            // Align Periodic interval with the same knob used in the UI ("1h", "4h", ...).
            // For non-periodic modes this value is still used as the minimum rebalance interval gate.
            decision_config.periodic_interval_hours = val;
        }

        // IL-specific knobs (only meaningful for IlLimit strategy mode).
        if decision_config.strategy_mode == clmm_lp_execution::prelude::StrategyMode::IlLimit {
            if let Some(max_il) = params.get("max_il_pct").and_then(|v| v.as_f64()) {
                decision_config.il_close_threshold = Decimal::from_f64_retain(max_il / 100.0)
                    .unwrap_or(decision_config.il_close_threshold);
            }
            if let Some(threshold) = params
                .get("rebalance_threshold_pct")
                .and_then(|v| v.as_f64())
            {
                decision_config.il_rebalance_threshold =
                    Decimal::from_f64_retain(threshold / 100.0)
                        .unwrap_or(decision_config.il_rebalance_threshold);
            }
        }

        executor.set_decision_config(decision_config);
    }

    let executor = Arc::new(RwLock::new(executor));

    let disabled_addrs: Vec<String> = strategy_config
        .get("parameters")
        .and_then(|p| p.get("executor_disabled_position_addresses"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(std::string::ToString::to_string))
                .collect()
        })
        .unwrap_or_default();
    {
        let g = executor.read().await;
        g.set_skip_evaluation_for_addresses(&disabled_addrs).await;
    }

    {
        let mut executors = state.executors.write().await;
        executors.insert(id.to_string(), executor.clone());
    }

    let executor_clone = executor.clone();
    let id_owned = id.to_string();
    let alert_sender = state.alert_updates.clone();

    tokio::spawn(async move {
        info!(strategy_id = %id_owned, "Strategy executor task started");

        let executor_guard = executor_clone.read().await;
        executor_guard.start().await;

        let _ = alert_sender.send(AlertUpdate {
            level: "info".to_string(),
            message: format!("Strategy {} stopped", id_owned),
            timestamp: chrono::Utc::now(),
            position_address: None,
        });
    });

    state
        .broadcast_alert(AlertUpdate {
            level: "info".to_string(),
            message: format!("Strategy {} started", id),
            timestamp: chrono::Utc::now(),
            position_address: None,
        })
        .await;

    info!(
        id = %id,
        dry_run = dry_run,
        auto_execute = auto_execute,
        "Strategy started"
    );

    Ok(monitor_note)
}

/// After linking a position to a strategy, ensure the strategy executor is running and the PDA is monitored.
///
/// On RPC failure while adding the position to the monitor, returns `Ok(Some(warning))` so callers
/// can surface a success response with a note (link is already persisted).
pub async fn ensure_strategy_running_after_position_link(
    state: &AppState,
    strategy_id: &str,
    position_pda: &str,
) -> ApiResult<Option<String>> {
    let need_full_start = {
        let mut strategies = state.strategies.write().await;
        let strategy = strategies
            .get_mut(strategy_id)
            .ok_or_else(|| ApiError::not_found("Strategy not found"))?;
        if strategy.running {
            false
        } else {
            strategy.running = true;
            strategy.updated_at = chrono::Utc::now();
            true
        }
    };

    if !need_full_start {
        let mut note = None;
        if let Err(e) = state.monitor.add_position(position_pda).await {
            warn!(
                position = %position_pda,
                error = %e,
                "ensure_strategy_running_after_position_link: monitor.add_position failed (RPC); link is already saved"
            );
            note = Some(format!(
                "Monitor could not load this position from RPC: {e}"
            ));
        }
        sync_executor_disabled_from_config(state, strategy_id).await?;
        return Ok(note);
    }

    let strategy_config = {
        let strategies = state.strategies.read().await;
        let strategy = strategies
            .get(strategy_id)
            .ok_or_else(|| ApiError::not_found("Strategy not found"))?;
        strategy.config.clone()
    };

    start_strategy_executor_core(state, strategy_id, strategy_config).await
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

    let dry_run = strategy_config
        .get("dry_run")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let auto_execute = strategy_config
        .get("auto_execute")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let note = start_strategy_executor_core(&state, &id, strategy_config).await?;

    let mut msg = format!(
        "Strategy started (dry_run={}, auto_execute={})",
        dry_run, auto_execute
    );
    if let Some(w) = note {
        msg.push_str(". ");
        msg.push_str(&w);
    }
    Ok(Json(MessageResponse::new(msg)))
}

/// Enable or disable strategy automation for a single linked position.
#[utoipa::path(
    post,
    path = "/strategies/{id}/position-executor",
    tag = "Strategies",
    params(
        ("id" = String, Path, description = "Strategy ID")
    ),
    request_body = StrategyPositionExecutorRequest,
    responses(
        (status = 200, description = "Updated", body = MessageResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Strategy not found")
    )
)]
pub async fn set_strategy_position_executor(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<StrategyPositionExecutorRequest>,
) -> ApiResult<Json<MessageResponse>> {
    Pubkey::from_str(body.position_address.trim())
        .map_err(|_| ApiError::bad_request("Invalid position_address"))?;

    {
        let mut strategies = state.strategies.write().await;
        let strategy = strategies
            .get_mut(&id)
            .ok_or_else(|| ApiError::not_found("Strategy not found"))?;

        let params = strategy
            .config
            .get_mut("parameters")
            .and_then(|p| p.as_object_mut())
            .ok_or_else(|| ApiError::bad_request("strategy parameters missing"))?;

        let arr_val = params
            .entry("executor_disabled_position_addresses".to_string())
            .or_insert_with(|| serde_json::json!([]));
        let list = arr_val.as_array_mut().ok_or_else(|| {
            ApiError::bad_request("executor_disabled_position_addresses must be a JSON array")
        })?;

        let addr = body.position_address.trim().to_string();
        if body.enabled {
            list.retain(|v| v.as_str() != Some(addr.as_str()));
        } else if !list.iter().any(|v| v.as_str() == Some(addr.as_str())) {
            list.push(serde_json::Value::String(addr));
        }

        strategy.updated_at = chrono::Utc::now();
    }

    sync_executor_disabled_from_config(&state, &id).await?;

    // Persist updated strategy parameters.
    let snapshot = state.strategies.read().await.clone();
    crate::state::try_persist_strategies_best_effort(&snapshot);

    Ok(Json(MessageResponse::new(if body.enabled {
        "Automation enabled for this position".to_string()
    } else {
        "Automation disabled for this position".to_string()
    })))
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

    {
        let mut busy = state.optimization_busy.write().await;
        busy.remove(&id);
    }

    // Broadcast alert
    state
        .broadcast_alert(AlertUpdate {
            level: "info".to_string(),
            message: format!("Strategy {} stopped", id),
            timestamp: chrono::Utc::now(),
            position_address: None,
        })
        .await;

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
