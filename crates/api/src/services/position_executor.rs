//! Resolve `StrategyExecutor` + wallet for **position** RPCs (open with swap, close, collect, …).

use crate::error::ApiError;
use crate::state::AppState;
use clmm_lp_domain::prelude::PositionTruthMode;
use clmm_lp_execution::prelude::{ExecutorConfig, StrategyExecutor, Wallet};
use rust_decimal::Decimal;
use std::env;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Reserved map key for an executor created on demand when a strategy is not running.
pub const API_POSITION_OPS_EXECUTOR_ID: &str = "__api_position_ops__";

/// Load the signing wallet from `KEYPAIR_PATH`, `SOLANA_KEYPAIR_PATH`, or `WALLET_KEYPAIR_PATH` (first non-empty wins).
pub fn load_wallet_from_env() -> Result<Option<Arc<Wallet>>, ApiError> {
    // Track which env var we picked for better diagnostics.
    let (keypair_env, keypair_path) = match env::var("KEYPAIR_PATH")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .map(|p| ("KEYPAIR_PATH", p))
        .or_else(|| {
            env::var("SOLANA_KEYPAIR_PATH")
                .ok()
                .filter(|v| !v.trim().is_empty())
                .map(|p| ("SOLANA_KEYPAIR_PATH", p))
        })
        .or_else(|| {
            // Backward/compat alias used in .env/.env.example
            env::var("WALLET_KEYPAIR_PATH")
                .ok()
                .filter(|v| !v.trim().is_empty())
                .map(|p| ("WALLET_KEYPAIR_PATH", p))
        }) {
        Some(v) => v,
        None => return Ok(None),
    };

    // Expand ~ on Windows (common in .env files).
    let keypair_path = {
        let p = keypair_path.trim();
        if p.starts_with("~/") || p.starts_with("~\\") {
            let home = env::var("USERPROFILE")
                .or_else(|_| env::var("HOME"))
                .unwrap_or_default();
            if !home.is_empty() {
                format!("{}\\{}", home.trim_end_matches(['\\', '/']), &p[2..])
            } else {
                p.to_string()
            }
        } else {
            p.to_string()
        }
    };
    let w = Wallet::from_file(&keypair_path, "api-keypair").map_err(|e| {
        ApiError::internal(format!("Failed to load wallet from {keypair_env}: {e}"))
    })?;
    Ok(Some(Arc::new(w)))
}

/// Prefer a **strategy** executor; otherwise a lazy executor backed by `KEYPAIR_PATH` (see [`API_POSITION_OPS_EXECUTOR_ID`]).
pub async fn resolve_executor_for_position_ops(
    state: &AppState,
) -> Option<Arc<RwLock<StrategyExecutor>>> {
    if state.dry_run {
        return None;
    }
    {
        let map = state.executors.read().await;
        if let Some(e) = map
            .iter()
            .filter(|(k, _)| k.as_str() != API_POSITION_OPS_EXECUTOR_ID)
            .map(|(_, v)| v.clone())
            .next()
        {
            return Some(e);
        }
        if let Some(e) = map.get(API_POSITION_OPS_EXECUTOR_ID) {
            return Some(e.clone());
        }
    }

    let wallet = match load_wallet_from_env() {
        Ok(Some(w)) => w,
        Ok(None) => return None,
        Err(e) => {
            tracing::warn!(error = %e, "Could not load wallet for lazy position executor");
            return None;
        }
    };

    let mut executors = state.executors.write().await;
    if let Some(e) = executors
        .iter()
        .filter(|(k, _)| k.as_str() != API_POSITION_OPS_EXECUTOR_ID)
        .map(|(_, v)| v.clone())
        .next()
    {
        return Some(e);
    }
    if let Some(e) = executors.get(API_POSITION_OPS_EXECUTOR_ID) {
        return Some(e.clone());
    }

    let executor_config = ExecutorConfig {
        eval_interval_secs: 3600,
        auto_execute: false,
        require_confirmation: true,
        max_slippage_pct: Decimal::new(5, 3),
        dry_run: false,
        fee_mode: PositionTruthMode::Heuristic,
    };
    let executor = StrategyExecutor::new(
        state.provider.clone(),
        state.monitor.clone(),
        state.tx_manager.clone(),
        executor_config,
    );
    executor.set_position_fee_ledger_path(Some(std::path::PathBuf::from(
        "data/position-fee-checkpoints.jsonl",
    )));
    executor.set_wallet(wallet);
    let executor = Arc::new(RwLock::new(executor));
    executors.insert(API_POSITION_OPS_EXECUTOR_ID.to_string(), executor.clone());
    tracing::info!(
        executor_id = API_POSITION_OPS_EXECUTOR_ID,
        "registered StrategyExecutor for position ops (KEYPAIR_PATH / SOLANA_KEYPAIR_PATH)"
    );
    Some(executor)
}
