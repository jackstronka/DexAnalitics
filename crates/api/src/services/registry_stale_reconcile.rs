//! Close stale `registry_open` rows when on-chain position accounts are gone (404).
//! Backfill missing lifecycle close rows when on-chain close tx exists but ledger append failed.

use crate::error::ApiError;
use crate::position_registry_seed::{registry_open_position_pubkeys, registry_position_open_map};
use crate::services::position_on_chain_cache::api_error_is_account_absent;
use crate::services::position_valuation::monitored_position_from_chain;
use crate::services::strategy_service::remove_position_address_from_all_strategies;
use crate::state::AppState;
use clmm_lp_protocols::ledger::position_registry::{registry_path, try_append_registry_close};
use clmm_lp_protocols::ledger::tx_lifecycle::{
    lifecycle_has_bot_close_for_position, lifecycle_has_row_with_signature,
};
use clmm_lp_protocols::prelude::RpcProvider;
use futures::{stream, StreamExt};
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Signature;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::str::FromStr;
use std::sync::Arc;
use tracing::{info, warn};

const STALE_RECONCILE_SIG: &str =
    "1111111111111111111111111111111111111111111111111111111111111111";

impl From<StaleReconcileReport> for crate::models::StaleReconcileReportResponse {
    fn from(r: StaleReconcileReport) -> Self {
        Self {
            checked: r.checked,
            registry_closed: r.registry_closed,
            strategy_links_removed: r.strategy_links_removed,
            still_on_chain: r.still_on_chain,
            rpc_errors: r.rpc_errors,
            lifecycle_backfilled: r.lifecycle_backfilled,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct StaleReconcileReport {
    pub checked: u32,
    pub registry_closed: Vec<String>,
    pub strategy_links_removed: u32,
    pub rpc_errors: u32,
    pub still_on_chain: u32,
    pub lifecycle_backfilled: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct RegistryOpenSnapshot {
    pub pool: Pubkey,
    pub owner: Pubkey,
    pub rebalance_session_id: Option<String>,
    pub open_signature: Option<String>,
}

/// Last `registry_open` row for a position (pool + owner), if any.
#[must_use]
pub fn registry_last_open_snapshot(position: &Pubkey) -> Option<RegistryOpenSnapshot> {
    let path = registry_path();
    let file = File::open(&path).ok()?;
    let reader = BufReader::new(file);
    let pos_s = position.to_string();
    let mut pool = None;
    let mut owner = None;
    let mut rebalance_session_id = None;
    let mut open_signature = None;

    for line in reader.lines().map_while(Result::ok) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if v.get("position_pubkey").and_then(|x| x.as_str()) != Some(pos_s.as_str()) {
            continue;
        }
        if v.get("event").and_then(|x| x.as_str()) != Some("registry_open") {
            continue;
        }
        let p = v
            .get("pool_address")
            .and_then(|x| x.as_str())
            .and_then(|s| Pubkey::from_str(s.trim()).ok());
        let o = v
            .get("owner_pubkey")
            .and_then(|x| x.as_str())
            .and_then(|s| Pubkey::from_str(s.trim()).ok());
        if let (Some(pool_pk), Some(owner_pk)) = (p, o) {
            pool = Some(pool_pk);
            owner = Some(owner_pk);
            rebalance_session_id = v
                .get("rebalance_session_id")
                .and_then(|x| x.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            open_signature = v
                .get("signature")
                .and_then(|x| x.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
        }
    }

    Some(RegistryOpenSnapshot {
        pool: pool?,
        owner: owner?,
        rebalance_session_id,
        open_signature,
    })
}

/// Closed positions in registry whose lifecycle is missing `bot_close_position`.
#[must_use]
pub fn registry_closed_missing_lifecycle_close() -> Vec<Pubkey> {
    let path = registry_path();
    let Ok(file) = File::open(&path) else {
        return Vec::new();
    };
    let reader = BufReader::new(file);
    let mut last_event: HashMap<String, String> = HashMap::new();
    for line in reader.lines().map_while(Result::ok) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let event = v.get("event").and_then(|x| x.as_str()).unwrap_or("");
        let position_pubkey = v
            .get("position_pubkey")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        if position_pubkey.is_empty() {
            continue;
        }
        if event == "registry_open" || event == "registry_close" {
            last_event.insert(position_pubkey.to_string(), event.to_string());
        }
    }
    let mut out = Vec::new();
    for (pos_s, ev) in last_event {
        if ev != "registry_close" {
            continue;
        }
        if lifecycle_has_bot_close_for_position(&pos_s) {
            continue;
        }
        if let Ok(pk) = Pubkey::from_str(&pos_s) {
            out.push(pk);
        }
    }
    out
}

pub fn registry_auto_close_stale_enabled() -> bool {
    match std::env::var("CLMM_REGISTRY_AUTO_CLOSE_STALE") {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        ),
        Err(_) => true,
    }
}

fn stale_reconcile_signature() -> Signature {
    Signature::from_str(STALE_RECONCILE_SIG).unwrap_or_default()
}

/// Best-effort: locate the on-chain close signature for a PDA (excluding known open sig).
pub async fn find_on_chain_close_signature(
    provider: &RpcProvider,
    position: &Pubkey,
    exclude_signatures: &[String],
) -> Option<Signature> {
    use solana_client::rpc_client::GetConfirmedSignaturesForAddress2Config;

    let cfg = GetConfirmedSignaturesForAddress2Config {
        before: None,
        until: None,
        limit: Some(32),
        commitment: None,
    };
    let sigs = provider
        .get_signatures_for_address_with_config(position, cfg)
        .await
        .ok()?;
    let exclude: std::collections::HashSet<&str> =
        exclude_signatures.iter().map(String::as_str).collect();
    for entry in sigs {
        if entry.err.is_some() {
            continue;
        }
        let sig_s = entry.signature.trim();
        if sig_s.is_empty() || exclude.contains(sig_s) {
            continue;
        }
        if lifecycle_has_row_with_signature(sig_s) {
            continue;
        }
        return Signature::from_str(sig_s).ok();
    }
    None
}

/// Append lifecycle + registry close from an on-chain close tx when bot ledger row is missing.
pub async fn try_backfill_missing_lifecycle_close(
    provider: &Arc<RpcProvider>,
    position: &Pubkey,
) -> bool {
    let pos_s = position.to_string();
    if lifecycle_has_bot_close_for_position(&pos_s) {
        return false;
    }
    let Some(snap) = registry_last_open_snapshot(position) else {
        warn!(
            position = %position,
            "lifecycle backfill: no registry_open snapshot; skip"
        );
        return false;
    };
    let mut exclude = Vec::new();
    if let Some(open_sig) = snap.open_signature.clone() {
        exclude.push(open_sig);
    }
    let Some(close_sig) = find_on_chain_close_signature(provider, position, &exclude).await else {
        warn!(
            position = %position,
            "lifecycle backfill: no close tx found on-chain"
        );
        return false;
    };
    let close_sig_s = close_sig.to_string();
    if lifecycle_has_row_with_signature(&close_sig_s) {
        return false;
    }

    let details = serde_json::json!({
        "close_kind": "rotation",
        "backfill_source": "on_chain_after_crash",
        "note": "Recovered bot_close_position from on-chain tx; original append likely lost during API restart mid-rebalance."
    });

    clmm_lp_protocols::ledger::tx_lifecycle::try_append_rebalance_executor_tx_cost(
        provider.as_ref(),
        &snap.owner,
        &close_sig,
        "close_position",
        Some(snap.pool),
        Some(*position),
        None,
        snap.rebalance_session_id.clone(),
        None,
        Some(details),
        None,
        None,
    )
    .await;

    try_append_registry_close(
        provider.as_ref(),
        "orca_bot",
        position,
        &snap.pool,
        &snap.owner,
        &close_sig,
        snap.rebalance_session_id.clone(),
        Some("strategy"),
    )
    .await;

    info!(
        position = %position,
        signature = %close_sig,
        "Backfilled missing bot_close_position lifecycle row from on-chain tx"
    );
    true
}

/// Repair registry-closed positions that never received lifecycle `bot_close_position`.
pub async fn repair_orphan_lifecycle_closes(provider: &Arc<RpcProvider>) -> Vec<String> {
    let orphans = registry_closed_missing_lifecycle_close();
    let mut repaired = Vec::new();
    for pk in orphans {
        if try_backfill_missing_lifecycle_close(provider, &pk).await {
            repaired.push(pk.to_string());
        }
    }
    repaired
}

/// Append `registry_close` when registry still marks the PDA open but RPC says account is gone.
pub async fn try_reconcile_stale_registry_open(
    provider: &Arc<RpcProvider>,
    position: &Pubkey,
) -> bool {
    if !registry_auto_close_stale_enabled() {
        return false;
    }
    let reg = registry_position_open_map();
    if reg.get(position) != Some(&true) {
        return false;
    }
    let Some(snap) = registry_last_open_snapshot(position) else {
        warn!(
            position = %position,
            "stale reconcile: registry_open without open row snapshot; skip registry_close"
        );
        return false;
    };

    if try_backfill_missing_lifecycle_close(provider, position).await {
        return true;
    }

    let sig = stale_reconcile_signature();
    try_append_registry_close(
        provider.as_ref(),
        "cli",
        position,
        &snap.pool,
        &snap.owner,
        &sig,
        snap.rebalance_session_id.clone(),
        Some("stale_reconcile"),
    )
    .await;
    info!(
        position = %position,
        "Appended registry_close for stale registry_open (on-chain account absent)"
    );
    true
}

/// Best-effort: remove PDA from all strategy lists when on-chain account is absent.
pub async fn try_prune_strategy_links_for_absent_position(
    state: &AppState,
    position: &str,
) {
    if let Err(e) = remove_position_address_from_all_strategies(state, position).await {
        warn!(
            position = %position,
            error = %e,
            "stale reconcile: remove_position_address_from_all_strategies failed"
        );
    }
}

/// On 404 during supplement fetch: close stale registry row + prune strategy links.
pub async fn on_position_account_absent(state: &AppState, pk: &Pubkey, from_registry: bool) {
    if from_registry {
        let _ = try_reconcile_stale_registry_open(&state.provider, pk).await;
    }
    try_prune_strategy_links_for_absent_position(state, &pk.to_string()).await;
}

/// Scan all `registry_open` PDAs; close registry + prune strategy links when account is missing.
pub async fn reconcile_all_stale_registry_opens(state: &AppState) -> StaleReconcileReport {
    let open = registry_open_position_pubkeys();
    let concurrency = std::env::var("CLMM_REGISTRY_RECONCILE_CONCURRENCY")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .filter(|n| *n > 0)
        .unwrap_or(6);

    let mut report = StaleReconcileReport {
        checked: open.len() as u32,
        ..Default::default()
    };

    let provider = state.provider.clone();

    let outcomes = stream::iter(open)
        .map(|pk| {
            let provider = provider.clone();
            async move {
                match monitored_position_from_chain(provider.clone(), &pk).await {
                    Ok(_) => (pk, None::<ApiError>),
                    Err(e) => (pk, Some(e)),
                }
            }
        })
        .buffer_unordered(concurrency)
        .collect::<Vec<_>>()
        .await;

    for (pk, err) in outcomes {
        let Some(e) = err else {
            report.still_on_chain += 1;
            continue;
        };
        if api_error_is_account_absent(&e) {
            if try_reconcile_stale_registry_open(&state.provider, &pk).await {
                report.registry_closed.push(pk.to_string());
            }
            try_prune_strategy_links_for_absent_position(state, &pk.to_string()).await;
            report.strategy_links_removed += 1;
        } else {
            report.rpc_errors += 1;
        }
    }

    report.lifecycle_backfilled = repair_orphan_lifecycle_closes(&state.provider).await;
    report
}

/// Remove `position_addresses` entries that are not on-chain (404) for one strategy.
pub async fn prune_stale_addresses_in_strategy(
    state: &AppState,
    strategy_id: &str,
) -> Result<StaleReconcileReport, ApiError> {
    let mut report = StaleReconcileReport::default();
    let addresses: Vec<String> = {
        let strategies = state.strategies.read().await;
        let Some(s) = strategies.get(strategy_id) else {
            return Err(ApiError::not_found(format!("Strategy not found: {strategy_id}")));
        };
        s.config
            .get("parameters")
            .and_then(|p| p.get("position_addresses"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    };
    report.checked = addresses.len() as u32;

    for addr in addresses {
        let Ok(pk) = Pubkey::from_str(&addr) else {
            let _ = remove_position_address_from_all_strategies(state, &addr).await;
            report.strategy_links_removed += 1;
            continue;
        };
        match monitored_position_from_chain(state.provider.clone(), &pk).await {
            Ok(_) => report.still_on_chain += 1,
            Err(e) if api_error_is_account_absent(&e) => {
                if registry_position_open_map().get(&pk) == Some(&true) {
                    let _ = try_reconcile_stale_registry_open(&state.provider, &pk).await;
                }
                try_prune_strategy_links_for_absent_position(state, &addr).await;
                report.strategy_links_removed += 1;
            }
            Err(_) => report.rpc_errors += 1,
        }
    }

    report.lifecycle_backfilled = repair_orphan_lifecycle_closes(&state.provider).await;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clmm_lp_protocols::rpc::RpcConfig;

    #[tokio::test]
    async fn backfill_9vhky_orphan_close_when_lifecycle_missing() {
        let repo_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        std::env::set_current_dir(&repo_root).expect("repo root");

        let pos = "9vhKYHAinJ8bpofhy8zwNMdgPDjoqSSXN43W9Rv2pKUH";
        if lifecycle_has_bot_close_for_position(pos) {
            return;
        }
        let provider = Arc::new(RpcProvider::new(RpcConfig::default()));
        let pk = Pubkey::from_str(pos).expect("valid pubkey");
        assert!(
            try_backfill_missing_lifecycle_close(&provider, &pk).await,
            "expected on-chain close backfill for {pos}"
        );
        assert!(lifecycle_has_bot_close_for_position(pos));
        assert!(lifecycle_has_row_with_signature(
            "4JXtYq29KfTfYsbLKFxuDsQoyDNwWHCBVk3Zowb36VkpbnPdFg7EWGfqPkQ4fHwCwhjLD7afYuQzzdZdkiyYBnG8"
        ));
    }
}
