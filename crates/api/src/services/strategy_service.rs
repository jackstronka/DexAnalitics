//! Strategy service for managing automated strategies.

use crate::error::ApiError;
use crate::models::OptimizeApplyPolicy;
use crate::models::StrategyType;
use crate::services::optimization_runner::{
    apply_optimize_result_json, merge_optimize_result_json_arg, run_optimize_cycle,
    run_optimize_subprocess,
};
use crate::state::{AlertUpdate, AppState};
use clmm_lp_execution::prelude::{DecisionConfig, ExecutorConfig, StrategyExecutor, StrategyMode};
use rust_decimal::Decimal;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::interval;
use tracing::{info, warn};

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
    /// Application state.
    state: AppState,
    /// Active strategy executors.
    executors: Arc<RwLock<std::collections::HashMap<String, Arc<RwLock<StrategyExecutor>>>>>,
}

impl StrategyService {
    /// Creates a new strategy service.
    pub fn new(state: AppState) -> Self {
        Self {
            state,
            executors: Arc::new(RwLock::new(std::collections::HashMap::new())),
        }
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
        };

        // Create strategy executor
        let executor = StrategyExecutor::new(
            self.state.provider.clone(),
            self.state.monitor.clone(),
            self.state.tx_manager.clone(),
            executor_config,
        );

        // Configure decision engine from stored strategy config.
        let strategy_type = strategy
            .config
            .get("strategy_type")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or(StrategyType::StaticRange);

        let mut decision_config = DecisionConfig::default();
        decision_config.strategy_mode = match strategy_type {
            StrategyType::StaticRange => StrategyMode::StaticRange,
            StrategyType::Periodic => StrategyMode::Periodic,
            StrategyType::Threshold => StrategyMode::Threshold,
            StrategyType::OorRecenter => StrategyMode::OorRecenter,
            StrategyType::IlLimit => StrategyMode::IlLimit,
            StrategyType::RetouchShift => StrategyMode::RetouchShift,
        };

        if let Some(params) = strategy.config.get("parameters") {
            // Common: width and periodic / thresholds.
            if let Some(range_width_pct) = params.get("range_width_pct").and_then(|v| v.as_f64()) {
                decision_config.range_width_pct = Decimal::from_f64_retain(range_width_pct / 100.0)
                    .unwrap_or(decision_config.range_width_pct);
            }

            if let Some(threshold) = params
                .get("rebalance_threshold_pct")
                .and_then(|v| v.as_f64())
            {
                decision_config.threshold_pct = Decimal::from_f64_retain(threshold / 100.0)
                    .unwrap_or(decision_config.threshold_pct);
            }

            if let Some(min_hours) = params
                .get("min_rebalance_interval_hours")
                .and_then(|v| v.as_u64())
            {
                decision_config.periodic_interval_hours = min_hours;
                decision_config.min_rebalance_interval_hours = min_hours;
            }

            // IL-specific knobs (only meaningful for IlLimit strategy mode).
            if let StrategyMode::IlLimit = decision_config.strategy_mode {
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
        }

        executor.set_decision_config(decision_config);

        let executor = Arc::new(RwLock::new(executor));

        if let Some(ref rp) = result_path_buf {
            if optimize_on_start || std::path::Path::new(rp).exists() {
                if let Err(e) = apply_optimize_result_json(rp, executor.as_ref()).await {
                    warn!(error = %e, "Could not apply optimize JSON; using static config");
                }
            }
        }

        if let Some(p) = il_ledger_path.as_deref() {
            executor
                .read()
                .await
                .set_il_ledger_path(Some(PathBuf::from(p)));
        }

        let busy = {
            let mut m = self.state.optimization_busy.write().await;
            let e = m
                .entry(strategy_id.to_string())
                .or_insert_with(|| Arc::new(AtomicBool::new(false)))
                .clone();
            e
        };

        // Store executor
        {
            let mut executors = self.executors.write().await;
            executors.insert(strategy_id.to_string(), executor.clone());
        }

        if optimize_interval_secs > 0 {
            match (&optimize_command, &result_path_buf) {
                (Some(cmd), Some(rp)) => {
                    let argv = merge_optimize_result_json_arg(cmd.clone(), &rp.to_string_lossy());
                    let sid = strategy_id.to_string();
                    let execs = self.executors.clone();
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
            let executors = self.executors.read().await;
            if let Some(executor) = executors.get(strategy_id) {
                let executor_guard = executor.read().await;
                executor_guard.stop();
            }
        }

        // Remove executor
        {
            let mut executors = self.executors.write().await;
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
        let executors = self.executors.read().await;
        executors.get(strategy_id).cloned()
    }

    /// Triggers a manual evaluation for a strategy.
    pub async fn trigger_evaluation(
        &self,
        strategy_id: &str,
    ) -> Result<StrategyOperationResult, ApiError> {
        info!(strategy_id = %strategy_id, "Triggering manual evaluation");

        let executors = self.executors.read().await;
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
        let executors = self.executors.read().await;

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
