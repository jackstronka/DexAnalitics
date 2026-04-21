//! Strategy service for managing automated strategies.

use crate::error::ApiError;
use crate::models::OptimizeApplyPolicy;
use crate::models::StrategyType;
use crate::position_registry_seed::registry_open_position_pubkeys;
use crate::services::optimization_runner::{
    apply_optimize_result_json, merge_optimize_result_json_arg, run_optimize_cycle,
    run_optimize_subprocess,
};
use crate::state::{AlertUpdate, AppState};
use clmm_lp_domain::prelude::PositionTruthMode;
use clmm_lp_execution::prelude::{DecisionConfig, ExecutorConfig, StrategyExecutor, StrategyMode};
use rust_decimal::Decimal;
use solana_sdk::pubkey::Pubkey;
use std::collections::HashSet;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::interval;
use tracing::{info, warn};

fn json_f64(v: &serde_json::Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse::<f64>().ok()))
}

/// Parse `parameters.min_rebalance_interval_hours` from JSON (integer or float).
pub(crate) fn min_rebalance_interval_hours_from_json(value: &serde_json::Value) -> Option<u64> {
    if let Some(u) = value.as_u64() {
        return Some(u);
    }
    if let Some(f) = value.as_f64()
        && f.is_finite()
        && f >= 0.0
    {
        return Some(f.floor() as u64);
    }
    value
        .as_str()
        .and_then(|s| s.trim().parse::<f64>().ok())
        .filter(|f| f.is_finite() && *f >= 0.0)
        .map(|f| f.floor() as u64)
}

/// Applies optional interval semantics to a decision config.
///
/// Rules:
/// - `Some(n)` sets the minimum interval gate to `n` for all modes.
/// - For `Periodic`, `Some(0)` is clamped to `1` defensively to avoid
///   rebalance-every-eval-tick loops from direct API payloads.
/// - `None` means "optional not set":
///   - `Periodic`: disable timer-triggering by setting an unreachable interval.
///   - non-`Periodic`: remove spacing gate (`min_rebalance_interval_hours=0`).
pub(crate) fn apply_optional_interval_to_decision_config(
    decision_config: &mut DecisionConfig,
    maybe_min_hours: Option<u64>,
) {
    match maybe_min_hours {
        Some(min_hours) => {
            let periodic_hours = if matches!(decision_config.strategy_mode, StrategyMode::Periodic)
                && min_hours == 0
            {
                warn!(
                    "periodic strategy received min_rebalance_interval_hours=0; clamping to 1h to avoid rebalance every eval tick"
                );
                1
            } else {
                min_hours
            };
            decision_config.periodic_interval_hours = periodic_hours;
            decision_config.min_rebalance_interval_hours = min_hours;
        }
        None => {
            if matches!(decision_config.strategy_mode, StrategyMode::Periodic) {
                decision_config.periodic_interval_hours = u64::MAX;
            } else {
                decision_config.min_rebalance_interval_hours = 0;
            }
        }
    }
}

/// Positions the strategy executor may act on: registry-open PDAs filtered by configured links.
///
/// - If `parameters.position_addresses` is **missing** or not a JSON array: use all
///   `registry_open` pubkeys (legacy strategies without an explicit link list).
/// - If it is an **empty array**: treat as an explicit operator choice to manage **no** PDAs
///   (cleared links), not “everything in registry”.
/// - If non-empty: intersection(`registry_open`, configured).
fn managed_allowlist_pubkeys_for_strategy_parameters(
    parameters: Option<&serde_json::Value>,
    registry_open: Vec<solana_sdk::pubkey::Pubkey>,
) -> Vec<solana_sdk::pubkey::Pubkey> {
    let Some(params) = parameters else {
        return registry_open;
    };
    let addr_field = params.get("position_addresses");
    let Some(arr) = addr_field.and_then(|v| v.as_array()) else {
        return registry_open;
    };
    let configured: Vec<solana_sdk::pubkey::Pubkey> = arr
        .iter()
        .filter_map(|v| v.as_str())
        .filter_map(|s| solana_sdk::pubkey::Pubkey::from_str(s.trim()).ok())
        .collect();
    if configured.is_empty() {
        return Vec::new();
    }
    let set: std::collections::HashSet<_> = configured.into_iter().collect();
    registry_open
        .into_iter()
        .filter(|p| set.contains(p))
        .collect()
}

/// Managed allowlist + reopen hook: every executor start path must call this so bot rotations
/// update `parameters.position_addresses` and the executor does not widen beyond linked PDAs.
pub async fn wire_executor_allowlist_and_reopen_hook(
    executor: &StrategyExecutor,
    state: &AppState,
    strategy_id: &str,
    parameters: Option<&serde_json::Value>,
) {
    let managed_allow = managed_allowlist_pubkeys_for_strategy_parameters(
        parameters,
        registry_open_position_pubkeys(),
    );
    executor.set_managed_allowlist(managed_allow).await;

    let st = state.clone();
    let sid = strategy_id.to_string();
    executor
        .set_reopen_hook(Some(Arc::new(move |old, new| {
            let st = st.clone();
            let sid = sid.clone();
            tokio::spawn(async move {
                let _ = replace_position_address_in_strategy(
                    &st,
                    &sid,
                    &old.to_string(),
                    &new.to_string(),
                )
                .await;
            });
        })))
        .await;
}

/// Result of a strategy operation.
#[derive(Debug, Clone)]
pub struct StrategyOperationResult {
    /// Whether the operation was successful.
    pub success: bool,
    /// Error message if failed.
    pub error: Option<String>,
}

impl StrategyOperationResult {
    /// Creates a successful result.
    pub fn success() -> Self {
        Self {
            success: true,
            error: None,
        }
    }

    /// Creates a failed result.
    pub fn failure(error: impl Into<String>) -> Self {
        Self {
            success: false,
            error: Some(error.into()),
        }
    }
}

/// Service for strategy operations.
pub struct StrategyService {
    /// Application state (includes the shared strategy executor map).
    state: AppState,
}

impl StrategyService {
    /// Creates a new strategy service.
    pub fn new(state: AppState) -> Self {
        Self { state }
    }

    /// Starts a strategy.
    pub async fn start_strategy(
        &self,
        strategy_id: &str,
    ) -> Result<StrategyOperationResult, ApiError> {
        info!(strategy_id = %strategy_id, "Starting strategy");

        let config_snapshot = {
            let mut strategies = self.state.strategies.write().await;
            let strategy = strategies
                .get_mut(strategy_id)
                .ok_or_else(|| ApiError::not_found("Strategy not found"))?;
            if strategy.running {
                return Err(ApiError::Conflict(
                    "Strategy is already running".to_string(),
                ));
            }
            strategy.config.clone()
        };

        let dry_run = config_snapshot
            .get("dry_run")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let auto_execute = config_snapshot
            .get("auto_execute")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let params_json = config_snapshot
            .get("parameters")
            .cloned()
            .unwrap_or(serde_json::json!({}));

        let eval_interval_secs = params_json
            .get("eval_interval_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(300);

        let optimize_on_start = params_json
            .get("optimize_on_start")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let optimize_interval_secs = params_json
            .get("optimize_interval_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let optimize_apply_policy: OptimizeApplyPolicy = params_json
            .get("optimize_apply_policy")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        if optimize_apply_policy == OptimizeApplyPolicy::ExternalHttp && optimize_interval_secs > 0
        {
            return Err(ApiError::bad_request(
                "parameters.optimize_apply_policy is external_http but optimize_interval_secs > 0; set interval to 0 or use combined".to_string(),
            ));
        }
        let optimize_command: Option<Vec<String>> = params_json
            .get("optimize_command")
            .and_then(|v| serde_json::from_value(v.clone()).ok());
        let optimize_result_json_path: Option<String> = params_json
            .get("optimize_result_json_path")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let il_ledger_path: Option<String> = params_json
            .get("il_ledger_path")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let result_path_buf = optimize_result_json_path
            .as_ref()
            .map(std::path::PathBuf::from);

        if optimize_on_start {
            match (&optimize_command, &result_path_buf) {
                (Some(cmd), Some(rp)) => {
                    let argv = merge_optimize_result_json_arg(cmd.clone(), &rp.to_string_lossy());
                    run_optimize_subprocess(&argv).await?;
                }
                _ => {
                    return Err(ApiError::bad_request(
                        "optimize_on_start requires optimize_command and optimize_result_json_path",
                    ));
                }
            }
        }

        if let Err(e) = try_heal_stale_strategy_links_for_strategy(&self.state, strategy_id).await {
            warn!(
                strategy_id = %strategy_id,
                error = %e,
                "try_heal_stale_strategy_links_for_strategy failed before executor start"
            );
        }

        let mut strategies = self.state.strategies.write().await;
        let strategy = strategies
            .get_mut(strategy_id)
            .ok_or_else(|| ApiError::not_found("Strategy not found"))?;
        if strategy.running {
            return Err(ApiError::Conflict(
                "Strategy is already running".to_string(),
            ));
        }

        // Create executor configuration
        let executor_config = ExecutorConfig {
            eval_interval_secs,
            auto_execute,
            require_confirmation: !auto_execute,
            max_slippage_pct: Decimal::new(5, 3), // 0.5%
            dry_run,
            fee_mode: PositionTruthMode::Heuristic,
        };

        // Create strategy executor
        let executor = StrategyExecutor::new(
            self.state.provider.clone(),
            self.state.monitor.clone(),
            self.state.tx_manager.clone(),
            executor_config,
        );

        wire_executor_allowlist_and_reopen_hook(
            &executor,
            &self.state,
            strategy_id,
            strategy.config.get("parameters"),
        )
        .await;

        // Always enable Tier3 checkpoint ledger by default (append-only JSONL).
        executor.set_position_fee_ledger_path(Some(std::path::PathBuf::from(
            "data/position-fee-checkpoints.jsonl",
        )));

        if !dry_run {
            match crate::services::position_executor::load_wallet_from_env() {
                Ok(Some(wallet)) => executor.set_wallet(wallet),
                Ok(None) => {
                    if auto_execute {
                        return Err(ApiError::bad_request(
                            "auto_execute=true requires KEYPAIR_PATH, SOLANA_KEYPAIR_PATH, or WALLET_KEYPAIR_PATH (e.g. in .env at API cwd)",
                        ));
                    }
                }
                Err(e) => return Err(e),
            }
        }

        // Configure decision engine from stored strategy config.
        let strategy_type = strategy
            .config
            .get("strategy_type")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or(StrategyType::StaticRange);

        let mut decision_config = DecisionConfig {
            strategy_mode: match strategy_type {
                StrategyType::StaticRange => StrategyMode::StaticRange,
                StrategyType::Periodic => StrategyMode::Periodic,
                StrategyType::Threshold => StrategyMode::Threshold,
                StrategyType::OorRecenter => StrategyMode::OorRecenter,
                StrategyType::IlLimit => StrategyMode::IlLimit,
                StrategyType::RetouchShift => StrategyMode::RetouchShift,
                StrategyType::LastCandle => StrategyMode::LastCandle,
            },
            ..DecisionConfig::default()
        };

        if let Some(params) = strategy.config.get("parameters") {
            // Common: width and periodic / thresholds.
            if let Some(range_width_pct) = params.get("range_width_pct").and_then(json_f64) {
                decision_config.range_width_pct = Decimal::from_f64_retain(range_width_pct / 100.0)
                    .unwrap_or(decision_config.range_width_pct);
            }

            if let Some(v) = params
                .get("periodic_requires_out_of_range")
                .and_then(|v| v.as_bool())
            {
                decision_config.periodic_requires_out_of_range = v;
            }

            if let Some(v) = params
                .get("rebalance_on_range_exit_immediately")
                .and_then(|v| v.as_bool())
            {
                decision_config.rebalance_on_range_exit_immediately = v;
            }

            if let Some(threshold) = params.get("rebalance_threshold_pct").and_then(json_f64) {
                decision_config.threshold_pct = Decimal::from_f64_retain(threshold / 100.0)
                    .unwrap_or(decision_config.threshold_pct);
            }

            let maybe_min_hours = params
                .get("min_rebalance_interval_hours")
                .and_then(min_rebalance_interval_hours_from_json);
            apply_optional_interval_to_decision_config(&mut decision_config, maybe_min_hours);

            if let Some(candle_seconds) = params.get("candle_seconds").and_then(|v| v.as_u64()) {
                decision_config.last_candle_seconds = candle_seconds.max(60);
            }

            // IL-specific knobs (only meaningful for IlLimit strategy mode).
            if let StrategyMode::IlLimit = decision_config.strategy_mode {
                if let Some(max_il) = params.get("max_il_pct").and_then(json_f64) {
                    decision_config.il_close_threshold = Decimal::from_f64_retain(max_il / 100.0)
                        .unwrap_or(decision_config.il_close_threshold);
                }
                if let Some(threshold) = params.get("rebalance_threshold_pct").and_then(json_f64) {
                    decision_config.il_rebalance_threshold =
                        Decimal::from_f64_retain(threshold / 100.0)
                            .unwrap_or(decision_config.il_rebalance_threshold);
                }
            }
        }

        executor.set_decision_config(decision_config);

        let executor = Arc::new(RwLock::new(executor));

        if let Some(ref rp) = result_path_buf
            && (optimize_on_start || std::path::Path::new(rp).exists())
            && let Err(e) = apply_optimize_result_json(rp, executor.as_ref()).await
        {
            warn!(error = %e, "Could not apply optimize JSON; using static config");
        }

        if let Some(p) = il_ledger_path.as_deref() {
            executor
                .read()
                .await
                .set_il_ledger_path(Some(PathBuf::from(p)));
        }

        let busy = {
            let mut m = self.state.optimization_busy.write().await;
            m.entry(strategy_id.to_string())
                .or_insert_with(|| Arc::new(AtomicBool::new(false)))
                .clone()
        };

        // Store executor
        {
            let mut executors = self.state.executors.write().await;
            executors.insert(strategy_id.to_string(), executor.clone());
        }

        if optimize_interval_secs > 0 {
            match (&optimize_command, &result_path_buf) {
                (Some(cmd), Some(rp)) => {
                    let argv = merge_optimize_result_json_arg(cmd.clone(), &rp.to_string_lossy());
                    let sid = strategy_id.to_string();
                    let execs = self.state.executors.clone();
                    let busy_c = busy.clone();
                    let path = rp.clone();
                    tokio::spawn(async move {
                        let mut ticker = interval(Duration::from_secs(optimize_interval_secs));
                        ticker.tick().await;
                        loop {
                            ticker.tick().await;
                            let ex_opt = execs.read().await.get(&sid).cloned();
                            let Some(ex) = ex_opt else {
                                break;
                            };
                            if let Err(e) = run_optimize_cycle(&argv, &path, &ex, &busy_c).await {
                                warn!(strategy_id = %sid, error = %e, "Periodic optimization failed");
                            }
                        }
                    });
                }
                _ => {
                    warn!(strategy_id = %strategy_id, "optimize_interval_secs set but missing optimize_command or optimize_result_json_path — skipping periodic optimize");
                }
            }
        }

        // Start executor in background task
        let executor_clone = executor.clone();
        let strategy_id_clone = strategy_id.to_string();
        let alert_sender = self.state.alert_updates.clone();

        tokio::spawn(async move {
            info!(strategy_id = %strategy_id_clone, "Strategy executor task started");

            let executor_guard = executor_clone.read().await;
            executor_guard.start().await;

            // Notify when stopped
            let _ = alert_sender.send(AlertUpdate {
                level: "info".to_string(),
                message: format!("Strategy {} stopped", strategy_id_clone),
                timestamp: chrono::Utc::now(),
                position_address: None,
            });
        });

        // Update strategy state
        strategy.running = true;
        strategy.updated_at = chrono::Utc::now();

        // Broadcast alert
        self.state
            .broadcast_alert(AlertUpdate {
                level: "info".to_string(),
                message: format!("Strategy {} started", strategy_id),
                timestamp: chrono::Utc::now(),
                position_address: None,
            })
            .await;

        info!(strategy_id = %strategy_id, "Strategy started successfully");
        Ok(StrategyOperationResult::success())
    }

    /// Stops a strategy.
    pub async fn stop_strategy(
        &self,
        strategy_id: &str,
    ) -> Result<StrategyOperationResult, ApiError> {
        info!(strategy_id = %strategy_id, "Stopping strategy");

        // Get strategy
        let mut strategies = self.state.strategies.write().await;
        let strategy = strategies
            .get_mut(strategy_id)
            .ok_or_else(|| ApiError::not_found("Strategy not found"))?;

        if !strategy.running {
            return Err(ApiError::Conflict("Strategy is not running".to_string()));
        }

        // Stop executor
        {
            let executors = self.state.executors.read().await;
            if let Some(executor) = executors.get(strategy_id) {
                let executor_guard = executor.read().await;
                executor_guard.stop();
            }
        }

        // Remove executor
        {
            let mut executors = self.state.executors.write().await;
            executors.remove(strategy_id);
        }

        {
            let mut busy = self.state.optimization_busy.write().await;
            busy.remove(strategy_id);
        }

        // Update strategy state
        strategy.running = false;
        strategy.updated_at = chrono::Utc::now();

        // Broadcast alert
        self.state
            .broadcast_alert(AlertUpdate {
                level: "info".to_string(),
                message: format!("Strategy {} stopped", strategy_id),
                timestamp: chrono::Utc::now(),
                position_address: None,
            })
            .await;

        info!(strategy_id = %strategy_id, "Strategy stopped successfully");
        Ok(StrategyOperationResult::success())
    }

    /// Gets the executor for a strategy.
    pub async fn get_executor(&self, strategy_id: &str) -> Option<Arc<RwLock<StrategyExecutor>>> {
        let executors = self.state.executors.read().await;
        executors.get(strategy_id).cloned()
    }

    /// Triggers a manual evaluation for a strategy.
    pub async fn trigger_evaluation(
        &self,
        strategy_id: &str,
    ) -> Result<StrategyOperationResult, ApiError> {
        info!(strategy_id = %strategy_id, "Triggering manual evaluation");

        let executors = self.state.executors.read().await;
        let _executor = executors.get(strategy_id).ok_or_else(|| {
            ApiError::not_found("Strategy executor not found - is the strategy running?")
        })?;

        // The executor runs on its own schedule, but we can trigger by checking positions
        // For now, just verify it's running
        let strategies = self.state.strategies.read().await;
        let strategy = strategies
            .get(strategy_id)
            .ok_or_else(|| ApiError::not_found("Strategy not found"))?;

        if !strategy.running {
            return Err(ApiError::Conflict("Strategy is not running".to_string()));
        }

        info!(strategy_id = %strategy_id, "Evaluation will occur on next interval");
        Ok(StrategyOperationResult::success())
    }

    /// Gets statistics for a running strategy.
    pub async fn get_strategy_stats(
        &self,
        strategy_id: &str,
    ) -> Result<serde_json::Value, ApiError> {
        let executors = self.state.executors.read().await;

        if let Some(executor) = executors.get(strategy_id) {
            let executor_guard = executor.read().await;
            let lifecycle = executor_guard.lifecycle();
            let circuit_breaker = executor_guard.circuit_breaker();

            let stats = lifecycle.get_aggregate_stats().await;
            let cb_stats = circuit_breaker.stats().await;
            let cb_state = circuit_breaker.state().await;

            Ok(serde_json::json!({
                "lifecycle": {
                    "total_positions": stats.total_positions,
                    "open_positions": stats.open_positions,
                    "closed_positions": stats.closed_positions,
                    "total_rebalances": stats.total_rebalances,
                    "total_fees_usd": stats.total_fees_usd.to_string(),
                    "total_pnl_usd": stats.total_pnl_usd.to_string(),
                    "avg_pnl_pct": stats.avg_pnl_pct.to_string(),
                    "total_tx_costs_lamports": stats.total_tx_costs_lamports
                },
                "circuit_breaker": {
                    "state": format!("{:?}", cb_state),
                    "success_count": cb_stats.success_count,
                    "failure_count": cb_stats.failure_count,
                    "manually_tripped": cb_stats.manually_tripped,
                    "opened_at": cb_stats.opened_at.map(|t| format!("{:?}", t))
                }
            }))
        } else {
            // Strategy not running, return basic stats from lifecycle
            let stats = self.state.lifecycle.get_aggregate_stats().await;

            Ok(serde_json::json!({
                "lifecycle": {
                    "total_positions": stats.total_positions,
                    "open_positions": stats.open_positions,
                    "closed_positions": stats.closed_positions,
                    "total_rebalances": stats.total_rebalances,
                    "total_fees_usd": stats.total_fees_usd.to_string(),
                    "total_pnl_usd": stats.total_pnl_usd.to_string(),
                    "avg_pnl_pct": stats.avg_pnl_pct.to_string(),
                    "total_tx_costs_lamports": stats.total_tx_costs_lamports
                },
                "circuit_breaker": null
            }))
        }
    }
}

/// Append a position PDA to `parameters.position_addresses` for an existing strategy.
pub async fn append_position_address_to_strategy(
    state: &AppState,
    strategy_id: &str,
    position_address: &str,
) -> Result<(), ApiError> {
    let mut strategies = state.strategies.write().await;
    let strategy = strategies
        .get_mut(strategy_id)
        .ok_or_else(|| ApiError::not_found("Strategy not found"))?;

    let config_obj = strategy
        .config
        .as_object_mut()
        .ok_or_else(|| ApiError::internal("strategy config must be a JSON object"))?;

    let params = config_obj
        .entry("parameters".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !params.is_object() {
        *params = serde_json::json!({});
    }
    let params_obj = params
        .as_object_mut()
        .ok_or_else(|| ApiError::internal("parameters must be a JSON object"))?;

    let arr_val = params_obj
        .entry("position_addresses".to_string())
        .or_insert_with(|| serde_json::json!([]));
    let arr = arr_val.as_array_mut().ok_or_else(|| {
        ApiError::bad_request("parameters.position_addresses must be a JSON array")
    })?;
    if !arr.iter().any(|v| v.as_str() == Some(position_address)) {
        arr.push(serde_json::json!(position_address));
    }
    strategy.updated_at = chrono::Utc::now();
    info!(
        strategy_id = %strategy_id,
        position = %position_address,
        "Linked position to strategy (parameters.position_addresses)"
    );

    let snapshot = strategies.clone();
    drop(strategies);
    crate::state::try_persist_strategies_best_effort(&snapshot);
    Ok(())
}

/// Remove a position PDA from every strategy's `position_addresses` and
/// `executor_disabled_position_addresses` so a position can be **moved** to another strategy or fully unlinked.
pub async fn remove_position_address_from_all_strategies(
    state: &AppState,
    position_address: &str,
) -> Result<(), ApiError> {
    let pos = position_address.trim();
    if pos.is_empty() {
        return Err(ApiError::bad_request("position address empty"));
    }

    let mut strategies = state.strategies.write().await;
    let mut changed = false;
    for strategy in strategies.values_mut() {
        let Some(config_obj) = strategy.config.as_object_mut() else {
            continue;
        };
        let Some(params_val) = config_obj.get_mut("parameters") else {
            continue;
        };
        if !params_val.is_object() {
            continue;
        }
        let params_obj = params_val.as_object_mut().unwrap();
        for key in ["position_addresses", "executor_disabled_position_addresses"] {
            if let Some(arr_val) = params_obj.get_mut(key)
                && let Some(arr) = arr_val.as_array_mut()
            {
                let before = arr.len();
                arr.retain(|v| v.as_str().map(|s| s.trim()) != Some(pos));
                if arr.len() != before {
                    changed = true;
                    strategy.updated_at = chrono::Utc::now();
                }
            }
        }
    }

    if changed {
        let snapshot = strategies.clone();
        drop(strategies);
        crate::state::try_persist_strategies_best_effort(&snapshot);
        info!(
            position = %pos,
            "Removed position from all strategy parameter lists"
        );
    } else {
        drop(strategies);
    }
    Ok(())
}

/// Replace `old_position` with `new_position` in a strategy's `parameters.position_addresses`.
///
/// Used for bot-driven close→open cycles so the UI keeps showing the strategy link.
pub async fn replace_position_address_in_strategy(
    state: &AppState,
    strategy_id: &str,
    old_position: &str,
    new_position: &str,
) -> Result<(), ApiError> {
    let old_pos = old_position.trim();
    let new_pos = new_position.trim();
    if old_pos.is_empty() || new_pos.is_empty() {
        return Err(ApiError::bad_request("position address empty"));
    }
    if old_pos == new_pos {
        return Ok(());
    }

    let mut strategies = state.strategies.write().await;
    let strategy = strategies
        .get_mut(strategy_id)
        .ok_or_else(|| ApiError::not_found("Strategy not found"))?;

    let config_obj = strategy
        .config
        .as_object_mut()
        .ok_or_else(|| ApiError::internal("strategy config must be a JSON object"))?;

    let params = config_obj
        .entry("parameters".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !params.is_object() {
        *params = serde_json::json!({});
    }
    let params_obj = params
        .as_object_mut()
        .ok_or_else(|| ApiError::internal("parameters must be a JSON object"))?;

    let arr_val = params_obj
        .entry("position_addresses".to_string())
        .or_insert_with(|| serde_json::json!([]));
    let arr = arr_val.as_array_mut().ok_or_else(|| {
        ApiError::bad_request("parameters.position_addresses must be a JSON array")
    })?;

    let mut changed = false;
    let mut replaced_old = false;
    for v in arr.iter_mut() {
        if v.as_str().map(|s| s.trim()) == Some(old_pos) {
            *v = serde_json::json!(new_pos);
            changed = true;
            replaced_old = true;
        }
    }
    // Ensure new is present exactly once, but only when old was actually replaced.
    // This prevents accidental growth when called with stale/non-matching old PDA.
    if replaced_old
        && !arr
            .iter()
            .any(|v| v.as_str().map(|s| s.trim()) == Some(new_pos))
    {
        arr.push(serde_json::json!(new_pos));
        changed = true;
    }
    // Remove any remaining old occurrences.
    let before = arr.len();
    arr.retain(|v| v.as_str().map(|s| s.trim()) != Some(old_pos));
    if arr.len() != before {
        changed = true;
    }

    // Keep per-position automation skip list in sync only when old PDA was replaced.
    if replaced_old
        && let Some(arr_val) = params_obj.get_mut("executor_disabled_position_addresses")
        && let Some(arr) = arr_val.as_array_mut()
    {
        for v in arr.iter_mut() {
            if v.as_str().map(|s| s.trim()) == Some(old_pos) {
                *v = serde_json::json!(new_pos);
                changed = true;
            }
        }
        let before_d = arr.len();
        arr.retain(|v| v.as_str().map(|s| s.trim()) != Some(old_pos));
        if arr.len() != before_d {
            changed = true;
        }
    }

    if changed {
        strategy.updated_at = chrono::Utc::now();
        let snapshot = strategies.clone();
        drop(strategies);
        crate::state::try_persist_strategies_best_effort(&snapshot);
        info!(
            strategy_id = %strategy_id,
            old_position = %old_pos,
            new_position = %new_pos,
            "Replaced linked position PDA in strategy (parameters.position_addresses)"
        );
    }
    Ok(())
}

async fn strategy_ids_holding_position_address(state: &AppState, pda: &str) -> Vec<String> {
    let pda = pda.trim();
    if pda.is_empty() {
        return Vec::new();
    }
    let strategies = state.strategies.read().await;
    let mut out = Vec::new();
    for s in strategies.values() {
        let Some(arr) = s
            .config
            .get("parameters")
            .and_then(|p| p.get("position_addresses"))
            .and_then(|v| v.as_array())
        else {
            continue;
        };
        if arr.iter().any(|v| v.as_str().map(str::trim) == Some(pda)) {
            out.push(s.id.clone());
        }
    }
    out
}

/// If [`parameters.position_addresses`] references mints that are **closed** in `registry.jsonl`,
/// try each **open** registry mint with [`heal_rotated_strategy_link_best_effort`] so rotation lineage
/// can rewrite the link to the live NFT (covers missed `reopen_hook`).
pub async fn try_heal_stale_strategy_links_for_strategy(
    state: &AppState,
    strategy_id: &str,
) -> Result<(), ApiError> {
    let (has_stale, linked_count) = {
        let strategies = state.strategies.read().await;
        let Some(s) = strategies.get(strategy_id) else {
            return Ok(());
        };
        let open_set: HashSet<Pubkey> = registry_open_position_pubkeys().into_iter().collect();
        let Some(params) = s.config.get("parameters") else {
            return Ok(());
        };
        let Some(arr) = params.get("position_addresses").and_then(|v| v.as_array()) else {
            return Ok(());
        };
        let mut any_stale = false;
        let mut n = 0usize;
        for v in arr {
            let Some(addr) = v.as_str() else {
                continue;
            };
            n += 1;
            if let Ok(pk) = Pubkey::from_str(addr.trim())
                && !open_set.contains(&pk)
            {
                any_stale = true;
            }
        }
        if n == 0 {
            return Ok(());
        }
        (any_stale, n)
    };

    if !has_stale {
        return Ok(());
    }

    for cand in registry_open_position_pubkeys() {
        let healed = heal_rotated_strategy_link_best_effort(state, &cand.to_string()).await?;
        if let Some(ids) = healed
            && ids.iter().any(|x| x == strategy_id)
        {
            info!(
                strategy_id = %strategy_id,
                new_position = %cand,
                "Healed stale strategy position_addresses from registry rotation chain (executor start)"
            );
            return Ok(());
        }
    }

    warn!(
        strategy_id = %strategy_id,
        linked_slots = linked_count,
        "strategy linked PDAs are not registry_open; could not infer current mint from rotation lineage — call POST /positions/{{active_mint}}/heal-strategy-link or edit parameters"
    );
    Ok(())
}

/// Recompute managed allowlist for a **running** executor (see [`managed_allowlist_pubkeys_for_strategy_parameters`]).
pub async fn sync_managed_allowlist_from_registry_for_strategy(
    state: &AppState,
    strategy_id: &str,
) -> Result<(), ApiError> {
    let exec_opt = { state.executors.read().await.get(strategy_id).cloned() };
    let Some(exec) = exec_opt else {
        return Ok(());
    };
    let strategies = state.strategies.read().await;
    let Some(strategy) = strategies.get(strategy_id) else {
        return Ok(());
    };
    let managed_allow = managed_allowlist_pubkeys_for_strategy_parameters(
        strategy.config.get("parameters"),
        registry_open_position_pubkeys(),
    );
    drop(strategies);
    let g = exec.read().await;
    g.set_managed_allowlist(managed_allow).await;
    Ok(())
}

/// When the UI shows an active PDA but `parameters.position_addresses` still lists a **closed**
/// parent (reopen_hook missed: external bot, API restart, or spawn error), walk registry/lifecycle
/// parents and replace the first matching PDA in strategy config.
pub async fn heal_rotated_strategy_link_best_effort(
    state: &AppState,
    new_position: &str,
) -> Result<Option<Vec<String>>, ApiError> {
    let new_pos = new_position.trim();
    if new_pos.is_empty() {
        return Ok(None);
    }
    // Safety guard: heal is only for active mints currently tracked by monitor.
    let new_pk = match Pubkey::from_str(new_pos) {
        Ok(pk) => pk,
        Err(_) => return Ok(None),
    };
    if state.monitor.get_position(&new_pk).await.is_none() {
        return Ok(None);
    }

    if !strategy_ids_holding_position_address(state, new_pos)
        .await
        .is_empty()
    {
        return Ok(None);
    }

    let mut cur = new_pos.to_string();
    for _ in 0..32 {
        let Some(parent) =
            crate::services::position_stream_lineage::infer_rotation_parent_best_effort(&cur).await
        else {
            break;
        };
        let parent = parent.trim().to_string();
        if parent.is_empty() || parent == cur {
            break;
        }
        let holders = strategy_ids_holding_position_address(state, &parent).await;
        if !holders.is_empty() {
            let mut healed = Vec::new();
            for sid in &holders {
                replace_position_address_in_strategy(state, sid, &parent, new_pos).await?;
                healed.push(sid.clone());
            }
            for sid in &healed {
                if let Err(e) = sync_managed_allowlist_from_registry_for_strategy(state, sid).await
                {
                    warn!(
                        strategy_id = %sid,
                        error = %e,
                        "heal_rotated_strategy_link: sync managed allowlist failed"
                    );
                }
                if let Err(e) =
                    crate::handlers::strategies::sync_executor_disabled_from_config(state, sid)
                        .await
                {
                    warn!(
                        strategy_id = %sid,
                        error = %e,
                        "heal_rotated_strategy_link: sync executor_disabled failed"
                    );
                }
            }
            info!(
                new_pos = %new_pos,
                parent = %parent,
                strategies = ?healed,
                "Healed strategy link after rotation (position_addresses + executor allowlist)"
            );
            return Ok(Some(healed));
        }
        cur = parent;
    }
    Ok(None)
}

#[cfg(test)]
mod managed_allowlist_tests {
    use super::{
        apply_optional_interval_to_decision_config,
        managed_allowlist_pubkeys_for_strategy_parameters, min_rebalance_interval_hours_from_json,
    };
    use clmm_lp_execution::prelude::{DecisionConfig, StrategyMode};
    use solana_sdk::pubkey::Pubkey;

    #[test]
    fn min_rebalance_interval_parses_json_number_and_string() {
        assert_eq!(
            min_rebalance_interval_hours_from_json(&serde_json::json!(0)),
            Some(0)
        );
        assert_eq!(
            min_rebalance_interval_hours_from_json(&serde_json::json!("2")),
            Some(2)
        );
    }

    #[test]
    fn optional_interval_none_disables_periodic_timer_trigger() {
        let mut cfg = DecisionConfig {
            strategy_mode: StrategyMode::Periodic,
            ..DecisionConfig::default()
        };
        apply_optional_interval_to_decision_config(&mut cfg, None);
        assert_eq!(cfg.periodic_interval_hours, u64::MAX);
        assert_eq!(
            cfg.min_rebalance_interval_hours,
            DecisionConfig::default().min_rebalance_interval_hours
        );
    }

    #[test]
    fn optional_interval_none_removes_spacing_for_non_periodic_modes() {
        let mut cfg = DecisionConfig {
            strategy_mode: StrategyMode::OorRecenter,
            ..DecisionConfig::default()
        };
        apply_optional_interval_to_decision_config(&mut cfg, None);
        assert_eq!(cfg.min_rebalance_interval_hours, 0);
    }

    #[test]
    fn optional_interval_zero_is_defensively_clamped_only_for_periodic() {
        let mut periodic_cfg = DecisionConfig {
            strategy_mode: StrategyMode::Periodic,
            ..DecisionConfig::default()
        };
        apply_optional_interval_to_decision_config(&mut periodic_cfg, Some(0));
        assert_eq!(periodic_cfg.min_rebalance_interval_hours, 0);
        assert_eq!(periodic_cfg.periodic_interval_hours, 1);

        let mut oor_cfg = DecisionConfig {
            strategy_mode: StrategyMode::OorRecenter,
            ..DecisionConfig::default()
        };
        apply_optional_interval_to_decision_config(&mut oor_cfg, Some(0));
        assert_eq!(oor_cfg.min_rebalance_interval_hours, 0);
        assert_eq!(oor_cfg.periodic_interval_hours, 0);
    }

    #[test]
    fn missing_position_addresses_field_uses_registry_open() {
        let open = vec![Pubkey::new_unique()];
        let params = serde_json::json!({});
        let got = managed_allowlist_pubkeys_for_strategy_parameters(Some(&params), open.clone());
        assert_eq!(got, open);
    }

    #[test]
    fn explicit_empty_position_addresses_yields_empty_allowlist() {
        let open = vec![Pubkey::new_unique()];
        let params = serde_json::json!({ "position_addresses": [] });
        let got = managed_allowlist_pubkeys_for_strategy_parameters(Some(&params), open);
        assert!(got.is_empty());
    }
}
