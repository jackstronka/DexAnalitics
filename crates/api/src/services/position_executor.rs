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

/// Load the signing wallet from file-path vars first, then env key material.
///
/// Resolution order:
/// 1) `KEYPAIR_PATH`
/// 2) `SOLANA_KEYPAIR_PATH`
/// 3) `WALLET_KEYPAIR_PATH`
/// 4) `SOLANA_KEYPAIR` (JSON array or base58 keypair)
/// 5) `WALLET_KEYPAIR_BASE58`
pub fn load_wallet_from_env() -> Result<Option<Arc<Wallet>>, ApiError> {
    // Track which env var we picked for better diagnostics.
    let file_source = env::var("KEYPAIR_PATH")
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
        });

    if let Some((keypair_env, keypair_path)) = file_source {
        let keypair_path = expand_home_path(&keypair_path);
        let w = Wallet::from_file(&keypair_path, "api-keypair").map_err(|e| {
            ApiError::internal(format!("Failed to load wallet from {keypair_env}: {e}"))
        })?;
        return Ok(Some(Arc::new(w)));
    }

    for env_var in ["SOLANA_KEYPAIR", "WALLET_KEYPAIR_BASE58"] {
        let has_value = env::var(env_var)
            .ok()
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false);
        if !has_value {
            continue;
        }
        let w = Wallet::from_env(env_var, "api-keypair").map_err(|e| {
            ApiError::internal(format!("Failed to load wallet from {env_var}: {e}"))
        })?;
        return Ok(Some(Arc::new(w)));
    }

    Ok(None)
}

fn expand_home_path(input: &str) -> String {
    let p = input.trim();
    if p.starts_with("~/") || p.starts_with("~\\") {
        let home = env::var("USERPROFILE")
            .or_else(|_| env::var("HOME"))
            .unwrap_or_default();
        if !home.is_empty() {
            return format!("{}\\{}", home.trim_end_matches(['\\', '/']), &p[2..]);
        }
    }
    p.to_string()
}

fn resolve_wallet_file_from_state(state: &AppState, wallet_id: &str) -> Option<String> {
    let wid = wallet_id.trim();
    if wid.is_empty() {
        return None;
    }
    let mut dirs: Vec<String> = Vec::new();
    if let Some(p) = state
        .config
        .wallets_dir_primary
        .as_ref()
        .or(state.config.wallets_dir.as_ref())
    {
        if !p.trim().is_empty() {
            dirs.push(p.trim().to_string());
        }
    } else if let Ok(v) = env::var("CLMM_WALLETS_DIR_PRIMARY") {
        if !v.trim().is_empty() {
            dirs.push(v.trim().to_string());
        }
    } else if let Ok(v) = env::var("CLMM_WALLETS_DIR")
        && !v.trim().is_empty()
    {
        dirs.push(v.trim().to_string());
    }
    if let Some(s) = state.config.wallets_dir_secondary.as_ref() {
        if !s.trim().is_empty() {
            dirs.push(s.trim().to_string());
        }
    } else if let Ok(v) = env::var("CLMM_WALLETS_DIR_SECONDARY")
        && !v.trim().is_empty()
    {
        dirs.push(v.trim().to_string());
    }
    for d in dirs {
        let path = std::path::PathBuf::from(d).join(format!("{wid}.json"));
        if path.exists() {
            return Some(path.to_string_lossy().to_string());
        }
    }
    None
}

fn load_wallet_from_active_signer_or_env(
    state: &AppState,
) -> Result<Option<Arc<Wallet>>, ApiError> {
    if let Ok(guard) = state.active_signer_wallet_id.try_read()
        && let Some(wallet_id) = guard.as_ref()
        && let Some(path) = resolve_wallet_file_from_state(state, wallet_id)
    {
        let w = Wallet::from_file(&path, "api-active-wallet").map_err(|e| {
            ApiError::internal(format!("Failed to load active signer `{wallet_id}`: {e}"))
        })?;
        return Ok(Some(Arc::new(w)));
    }
    load_wallet_from_env()
}

/// Human-readable diagnostics for why API position ops may not find a signer wallet.
pub fn wallet_config_diagnostic() -> String {
    let mut parts: Vec<String> = Vec::new();
    for var in ["KEYPAIR_PATH", "SOLANA_KEYPAIR_PATH", "WALLET_KEYPAIR_PATH"] {
        match env::var(var).ok().map(|v| v.trim().to_string()) {
            Some(v) if !v.is_empty() => {
                let expanded = expand_home_path(&v);
                let exists = std::path::Path::new(&expanded).exists();
                parts.push(format!("{var}=set(path_exists={exists})"));
            }
            _ => parts.push(format!("{var}=unset")),
        }
    }
    for var in ["SOLANA_KEYPAIR", "WALLET_KEYPAIR_BASE58"] {
        let set = env::var(var)
            .ok()
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false);
        parts.push(format!("{var}={}", if set { "set" } else { "unset" }));
    }
    format!(
        "Wallet env diagnostics: {}. Configure at least one valid signer source.",
        parts.join(", ")
    )
}

async fn executor_can_sign_on_chain(exec: &Arc<RwLock<StrategyExecutor>>) -> bool {
    let g = exec.read().await;
    !g.is_dry_run() && g.wallet_pubkey().is_some()
}

/// Resolve a `StrategyExecutor` that can **submit real on-chain txs** for manual position ops.
///
/// Never returns a strategy executor that is in `dry_run` mode (those no-op `execute_*` and would
/// still surface as HTTP success). Prefer [`API_POSITION_OPS_EXECUTOR_ID`], then any live
/// strategy runner with a wallet, then create the lazy ops executor from env keypair.
pub async fn resolve_executor_for_position_ops(
    state: &AppState,
) -> Option<Arc<RwLock<StrategyExecutor>>> {
    if state.dry_run {
        return None;
    }

    let snapshot: Vec<(String, Arc<RwLock<StrategyExecutor>>)> = {
        let map = state.executors.read().await;
        map.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    };

    if let Some(e) = snapshot
        .iter()
        .find(|(k, _)| k.as_str() == API_POSITION_OPS_EXECUTOR_ID)
        .map(|(_, v)| v)
        && executor_can_sign_on_chain(e).await
    {
        return Some(e.clone());
    }
    for (sid, e) in snapshot
        .iter()
        .filter(|(k, _)| k.as_str() != API_POSITION_OPS_EXECUTOR_ID)
    {
        if executor_can_sign_on_chain(e).await {
            tracing::debug!(
                strategy_id = %sid,
                "resolve_executor_for_position_ops: using running strategy executor (not dry-run, wallet set)"
            );
            return Some(e.clone());
        }
    }

    let wallet = match load_wallet_from_active_signer_or_env(state) {
        Ok(Some(w)) => w,
        Ok(None) => return None,
        Err(e) => {
            tracing::warn!(error = %e, "Could not load wallet for lazy position executor");
            return None;
        }
    };

    let snapshot2: Vec<(String, Arc<RwLock<StrategyExecutor>>)> = {
        let map = state.executors.read().await;
        map.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    };

    if let Some(e) = snapshot2
        .iter()
        .find(|(k, _)| k.as_str() == API_POSITION_OPS_EXECUTOR_ID)
        .map(|(_, v)| v)
        && executor_can_sign_on_chain(e).await
    {
        return Some(e.clone());
    }
    for (sid, e) in snapshot2
        .iter()
        .filter(|(k, _)| k.as_str() != API_POSITION_OPS_EXECUTOR_ID)
    {
        if executor_can_sign_on_chain(e).await {
            tracing::debug!(
                strategy_id = %sid,
                "resolve_executor_for_position_ops: using strategy executor after wallet load (not dry-run, wallet set)"
            );
            return Some(e.clone());
        }
    }

    let mut executors = state.executors.write().await;
    if let Some(e) = executors.get(API_POSITION_OPS_EXECUTOR_ID).cloned() {
        // Another task may have registered the ops executor while we loaded the wallet from env.
        return Some(e);
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
        "registered StrategyExecutor for position ops (path/env keypair source)"
    );
    Some(executor)
}
