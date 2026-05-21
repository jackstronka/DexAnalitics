//! Bulk close-all job store and background worker.

use crate::error::ApiError;
use crate::models::{
    CloseAllBatchItem, CloseAllBatchStatusResponse, CloseAllBatchSummary, CloseAllItemStatus,
    CloseAllPositionsPreviewResponse, CloseAllPositionsRequest, CloseAllPositionsStartResponse,
    CloseAllSkippedPreview, CloseAllWalletGroup, CLOSE_ALL_MAX_SLIPPAGE_BPS,
};
use crate::position_registry_seed::{registry_open_position_pubkeys, registry_position_open_map};
use crate::services::position_close_ops::{
    bump_close_slippage_bps_for_6018_retry, execute_manual_close_with_wallet,
    finalize_manual_close_send_first, is_close_slippage_6018,
    poll_close_signature_until_terminal, resolve_bulk_close_slippage_bps,
    submit_manual_close_send_first, wait_close_signature_processed, CloseSignaturePoll,
    ManualCloseLedgerContext, ManualCloseSubmitOutcome,
};
use crate::services::position_close_signer::{
    resolve_close_signer_for_position, CloseSignerSkipReason,
};
use crate::services::position_on_chain_cache::{
    fetch_supplement_positions_parallel, running_strategy_position_pubkeys,
};
use crate::services::position_wallet_store::load_api_wallet_by_id;
use crate::services::strategy_service::disable_automation_for_positions;
use crate::state::AppState;
use chrono::Utc;
use solana_sdk::pubkey::Pubkey;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::LazyLock;
use tokio::sync::RwLock;
use tracing::{info, warn};
use uuid::Uuid;

#[derive(Debug, Clone)]
struct CloseAllJob {
    batch_id: String,
    status: String,
    started_ts_utc: chrono::DateTime<Utc>,
    finished_ts_utc: Option<chrono::DateTime<Utc>>,
    items: Vec<CloseAllBatchItem>,
}

static CLOSE_ALL_JOBS: LazyLock<RwLock<HashMap<String, CloseAllJob>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

fn summarize(items: &[CloseAllBatchItem]) -> CloseAllBatchSummary {
    let mut summary = CloseAllBatchSummary {
        total: items.len() as u32,
        ..Default::default()
    };
    for item in items {
        match item.status {
            CloseAllItemStatus::Confirmed | CloseAllItemStatus::AlreadyClosed => {
                summary.closed += 1;
            }
            CloseAllItemStatus::Failed => summary.failed += 1,
            CloseAllItemStatus::SkippedUnmanagedSigner => summary.skipped += 1,
            CloseAllItemStatus::Queued
            | CloseAllItemStatus::PendingOnChain
            | CloseAllItemStatus::Submitted => {
                summary.pending += 1;
            }
        }
    }
    summary
}

/// `confirm_sync` (default) or `send_first` for bulk close-all worker.
pub fn parse_close_all_send_mode(send_mode: &str) -> Result<bool, ApiError> {
    match send_mode.trim() {
        "confirm_sync" => Ok(false),
        "send_first" => Ok(true),
        other => Err(ApiError::bad_request(format!(
            "Unknown close-all send_mode '{other}'; use confirm_sync or send_first"
        ))),
    }
}

fn close_all_inter_tx_secs() -> u64 {
    std::env::var("CLMM_SEND_FIRST_INTER_TX_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(4)
}

fn job_to_response(job: &CloseAllJob) -> CloseAllBatchStatusResponse {
    CloseAllBatchStatusResponse {
        batch_id: job.batch_id.clone(),
        status: job.status.clone(),
        started_ts_utc: job.started_ts_utc.to_rfc3339(),
        finished_ts_utc: job.finished_ts_utc.map(|t| t.to_rfc3339()),
        summary: summarize(&job.items),
        items: job.items.clone(),
    }
}

/// Collect open monitored position addresses (same scope as `GET /positions`).
pub async fn collect_monitored_position_addresses(state: &AppState) -> Vec<String> {
    let mut positions = state.monitor.get_positions().await;
    let reg_state = registry_position_open_map();
    if !reg_state.is_empty() {
        positions.retain(|p| reg_state.get(&p.address).copied().unwrap_or(true));
    }
    let mut monitored: HashSet<Pubkey> = positions.iter().map(|p| p.address).collect();

    let registry_candidates: HashSet<Pubkey> =
        registry_open_position_pubkeys().into_iter().collect();
    let strategy_candidates: HashSet<Pubkey> =
        running_strategy_position_pubkeys(state).await.into_iter().collect();
    let mut supplemental: Vec<Pubkey> = registry_candidates
        .iter()
        .chain(strategy_candidates.iter())
        .copied()
        .collect();
    supplemental.sort_unstable_by_key(|p| p.to_string());
    supplemental.dedup();

    let concurrency = std::env::var("CLMM_LIST_POSITIONS_FETCH_CONCURRENCY")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(6);

    let (supplement, _) = fetch_supplement_positions_parallel(
        state,
        &monitored,
        supplemental,
        &registry_candidates,
        &strategy_candidates,
        &reg_state,
        concurrency,
    )
    .await;

    for p in supplement {
        monitored.insert(p.address);
        positions.push(p);
    }

    positions
        .into_iter()
        .map(|p| p.address.to_string())
        .collect()
}

fn apply_scope_and_excludes(
    req: &CloseAllPositionsRequest,
    monitored: Vec<String>,
) -> Result<Vec<String>, ApiError> {
    let exclude: HashSet<String> = req
        .exclude_addresses
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let scope = req.scope.trim().to_ascii_lowercase();
    let mut addresses = if scope == "explicit" {
        if req.addresses.is_empty() {
            return Err(ApiError::bad_request(
                "scope=explicit requires non-empty addresses",
            ));
        }
        req.addresses
            .iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
    } else if scope == "monitored" {
        monitored
    } else {
        return Err(ApiError::bad_request(
            "scope must be `monitored` or `explicit`",
        ));
    };

    addresses.retain(|a| !exclude.contains(a));
    addresses.sort_unstable();
    addresses.dedup();
    Ok(addresses)
}

async fn resolve_close_all_addresses(
    state: &AppState,
    req: &CloseAllPositionsRequest,
) -> Result<Vec<String>, ApiError> {
    let scope = req.scope.trim().to_ascii_lowercase();
    if scope == "explicit" {
        // Selected addresses only — do not run full monitored supplement RPC scan.
        return apply_scope_and_excludes(req, Vec::new());
    }
    if scope == "monitored" {
        let monitored = collect_monitored_position_addresses(state).await;
        return apply_scope_and_excludes(req, monitored);
    }
    Err(ApiError::bad_request(
        "scope must be `monitored` or `explicit`",
    ))
}

async fn plan_close_all(
    state: &AppState,
    addresses: &[String],
) -> (
    Vec<CloseAllBatchItem>,
    Vec<CloseAllWalletGroup>,
    Vec<CloseAllSkippedPreview>,
) {
    let mut items = Vec::with_capacity(addresses.len());
    let mut skipped_preview = Vec::new();
    let mut group_counts: BTreeMap<(String, String), u32> = BTreeMap::new();

    for addr in addresses {
        match resolve_close_signer_for_position(state, addr).await {
            Ok(Ok(res)) => {
                *group_counts
                    .entry((res.close_signer_wallet_id.clone(), res.close_signer_pubkey.clone()))
                    .or_insert(0) += 1;
                items.push(CloseAllBatchItem {
                    address: res.address.clone(),
                    owner_pubkey: Some(res.owner_pubkey.clone()),
                    close_signer_wallet_id: Some(res.close_signer_wallet_id.clone()),
                    status: CloseAllItemStatus::Queued,
                    signature: None,
                    error: None,
                });
            }
            Ok(Err(skip)) => {
                skipped_preview.push(CloseAllSkippedPreview {
                    address: skip.address.clone(),
                    reason: match skip.reason {
                        CloseSignerSkipReason::UnmanagedSigner => "unmanaged_signer".to_string(),
                        CloseSignerSkipReason::InvalidAddress => "invalid_address".to_string(),
                    },
                    owner_pubkey: skip.owner_pubkey.clone(),
                });
                items.push(CloseAllBatchItem {
                    address: skip.address.clone(),
                    owner_pubkey: skip.owner_pubkey.clone(),
                    close_signer_wallet_id: None,
                    status: CloseAllItemStatus::SkippedUnmanagedSigner,
                    signature: None,
                    error: Some("No API-managed wallet for position owner".to_string()),
                });
            }
            Err(e) => {
                skipped_preview.push(CloseAllSkippedPreview {
                    address: addr.clone(),
                    reason: "invalid_address".to_string(),
                    owner_pubkey: None,
                });
                items.push(CloseAllBatchItem {
                    address: addr.clone(),
                    owner_pubkey: None,
                    close_signer_wallet_id: None,
                    status: CloseAllItemStatus::SkippedUnmanagedSigner,
                    signature: None,
                    error: Some(e),
                });
            }
        }
    }

    let groups = group_counts
        .into_iter()
        .map(|((wallet_id, owner_pubkey), count)| CloseAllWalletGroup {
            wallet_id,
            owner_pubkey,
            count,
        })
        .collect();

    (items, groups, skipped_preview)
}

async fn update_job_item(batch_id: &str, address: &str, item: CloseAllBatchItem) {
    let mut jobs = CLOSE_ALL_JOBS.write().await;
    if let Some(job) = jobs.get_mut(batch_id) {
        if let Some(row) = job.items.iter_mut().find(|i| i.address == address) {
            *row = item;
        }
    }
}

async fn finish_job(batch_id: &str, status: &str) {
    let mut jobs = CLOSE_ALL_JOBS.write().await;
    if let Some(job) = jobs.get_mut(batch_id) {
        job.status = status.to_string();
        job.finished_ts_utc = Some(Utc::now());
    }
}

fn validate_close_all_slippage_bps(opt: Option<u16>) -> Result<u16, ApiError> {
    if let Some(v) = opt {
        if v > CLOSE_ALL_MAX_SLIPPAGE_BPS {
            return Err(ApiError::bad_request(format!(
                "options.slippage_bps too high (max {CLOSE_ALL_MAX_SLIPPAGE_BPS})"
            )));
        }
    }
    Ok(resolve_bulk_close_slippage_bps(opt))
}

async fn close_wallet_group(
    state: AppState,
    batch_id: String,
    wallet_id: String,
    addrs: Vec<String>,
    skip_pre_collect: bool,
    send_first: bool,
    slippage_bps: u16,
) {
    let wallet = match load_api_wallet_by_id(&state, &wallet_id) {
        Ok(w) => w,
        Err(e) => {
            for addr in addrs {
                update_job_item(
                    &batch_id,
                    &addr,
                    CloseAllBatchItem {
                        address: addr.clone(),
                        owner_pubkey: None,
                        close_signer_wallet_id: Some(wallet_id.clone()),
                        status: CloseAllItemStatus::Failed,
                        signature: None,
                        error: Some(e.to_string()),
                    },
                )
                .await;
            }
            return;
        }
    };
    let ledger_owner = wallet.pubkey().to_string();
    let inter_tx_secs = close_all_inter_tx_secs();
    let mut finalize_handles = Vec::new();

    for addr in addrs {
        info!(
            batch_id = %batch_id,
            wallet_id = %wallet_id,
            position = %addr,
            skip_pre_collect,
            send_first,
            "close-all: closing position on-chain"
        );
        update_job_item(
            &batch_id,
            &addr,
            CloseAllBatchItem {
                address: addr.clone(),
                owner_pubkey: Some(ledger_owner.clone()),
                close_signer_wallet_id: Some(wallet_id.clone()),
                status: CloseAllItemStatus::PendingOnChain,
                signature: None,
                error: None,
            },
        )
        .await;

        let mut ctx =
            ManualCloseLedgerContext::for_bulk(&batch_id, &ledger_owner, skip_pre_collect, slippage_bps);

        if send_first {
            loop {
                let submit =
                    submit_manual_close_send_first(&state, &addr, None, wallet.clone(), ctx.clone())
                        .await;
                match submit {
                    Ok(ManualCloseSubmitOutcome::AlreadyClosed) => {
                        update_job_item(
                            &batch_id,
                            &addr,
                            CloseAllBatchItem {
                                address: addr.clone(),
                                owner_pubkey: Some(ledger_owner.clone()),
                                close_signer_wallet_id: Some(wallet_id.clone()),
                                status: CloseAllItemStatus::AlreadyClosed,
                                signature: None,
                                error: None,
                            },
                        )
                        .await;
                        break;
                    }
                    Ok(ManualCloseSubmitOutcome::Submitted(flight)) => {
                        let sig = flight.signature.clone();
                        let sig_wait = sig.clone();
                        update_job_item(
                            &batch_id,
                            &addr,
                            CloseAllBatchItem {
                                address: addr.clone(),
                                owner_pubkey: Some(ledger_owner.clone()),
                                close_signer_wallet_id: Some(wallet_id.clone()),
                                status: CloseAllItemStatus::Submitted,
                                signature: Some(sig.clone()),
                                error: None,
                            },
                        )
                        .await;

                        let poll_secs = inter_tx_secs.saturating_add(45);
                        if let CloseSignaturePoll::Failed(ref err) =
                            poll_close_signature_until_terminal(&state, &sig, poll_secs).await
                        {
                            if is_close_slippage_6018(err) && !ctx.slippage_6018_retry_done {
                                let bumped =
                                    bump_close_slippage_bps_for_6018_retry(ctx.slippage_bps);
                                if bumped > ctx.slippage_bps {
                                    ctx.slippage_6018_retry_done = true;
                                    ctx.slippage_bps = bumped;
                                    info!(
                                        batch_id = %batch_id,
                                        position = %addr,
                                        slippage_bps = bumped,
                                        "close-all: on-chain 6018 after submit, retrying with higher slippage"
                                    );
                                    update_job_item(
                                        &batch_id,
                                        &addr,
                                        CloseAllBatchItem {
                                            address: addr.clone(),
                                            owner_pubkey: Some(ledger_owner.clone()),
                                            close_signer_wallet_id: Some(wallet_id.clone()),
                                            status: CloseAllItemStatus::PendingOnChain,
                                            signature: None,
                                            error: None,
                                        },
                                    )
                                    .await;
                                    continue;
                                }
                            }
                        }

                        let state_fin = state.clone();
                        let batch_fin = batch_id.clone();
                        let addr_fin = addr.clone();
                        let wallet_fin = wallet.clone();
                        let owner_fin = ledger_owner.clone();
                        let wallet_id_fin = wallet_id.clone();
                        finalize_handles.push(tokio::spawn(async move {
                            let item = match finalize_manual_close_send_first(
                                &state_fin,
                                wallet_fin,
                                flight,
                            )
                            .await
                            {
                                Ok(confirmed_sig) => CloseAllBatchItem {
                                    address: addr_fin.clone(),
                                    owner_pubkey: Some(owner_fin.clone()),
                                    close_signer_wallet_id: Some(wallet_id_fin.clone()),
                                    status: CloseAllItemStatus::Confirmed,
                                    signature: Some(confirmed_sig),
                                    error: None,
                                },
                                Err(e) => CloseAllBatchItem {
                                    address: addr_fin.clone(),
                                    owner_pubkey: Some(owner_fin),
                                    close_signer_wallet_id: Some(wallet_id_fin),
                                    status: CloseAllItemStatus::Failed,
                                    signature: Some(sig),
                                    error: Some(e.to_string()),
                                },
                            };
                            update_job_item(&batch_fin, &addr_fin, item).await;
                        }));

                        wait_close_signature_processed(&state, &sig_wait, inter_tx_secs).await;
                        break;
                    }
                    Err(e) => {
                        update_job_item(
                            &batch_id,
                            &addr,
                            CloseAllBatchItem {
                                address: addr.clone(),
                                owner_pubkey: Some(ledger_owner.clone()),
                                close_signer_wallet_id: Some(wallet_id.clone()),
                                status: CloseAllItemStatus::Failed,
                                signature: None,
                                error: Some(e.to_string()),
                            },
                        )
                        .await;
                        break;
                    }
                }
            }
        } else {
            let result =
                execute_manual_close_with_wallet(&state, &addr, None, wallet.clone(), ctx).await;

            let item = match result {
                Ok(msg) => {
                    let already = msg.message.contains("already closed");
                    CloseAllBatchItem {
                        address: addr.clone(),
                        owner_pubkey: Some(ledger_owner.clone()),
                        close_signer_wallet_id: Some(wallet_id.clone()),
                        status: if already {
                            CloseAllItemStatus::AlreadyClosed
                        } else {
                            CloseAllItemStatus::Confirmed
                        },
                        signature: None,
                        error: None,
                    }
                }
                Err(e) => CloseAllBatchItem {
                    address: addr.clone(),
                    owner_pubkey: Some(ledger_owner.clone()),
                    close_signer_wallet_id: Some(wallet_id.clone()),
                    status: CloseAllItemStatus::Failed,
                    signature: None,
                    error: Some(e.to_string()),
                },
            };
            update_job_item(&batch_id, &addr, item).await;
        }
    }

    for handle in finalize_handles {
        if let Err(e) = handle.await {
            warn!(
                batch_id = %batch_id,
                error = %e,
                "close-all: send_first finalize task join error"
            );
        }
    }
}

async fn run_close_all_worker(state: AppState, batch_id: String, req: CloseAllPositionsRequest) {
    let skip_pre_collect = req.options.skip_pre_collect;
    let slippage_bps = match validate_close_all_slippage_bps(req.options.slippage_bps) {
        Ok(v) => v,
        Err(e) => {
            warn!(batch_id = %batch_id, error = %e, "close-all: invalid slippage_bps");
            finish_job(&batch_id, "failed").await;
            return;
        }
    };
    let send_first = match parse_close_all_send_mode(&req.options.send_mode) {
        Ok(v) => v,
        Err(e) => {
            warn!(batch_id = %batch_id, error = %e, "close-all: invalid send_mode");
            finish_job(&batch_id, "failed").await;
            return;
        }
    };
    info!(
        batch_id = %batch_id,
        skip_pre_collect,
        send_first,
        slippage_bps,
        "close-all worker started"
    );

    if req.pause_linked_strategies {
        let addresses: Vec<String> = {
            let jobs = CLOSE_ALL_JOBS.read().await;
            jobs.get(&batch_id)
                .map(|j| {
                    j.items
                        .iter()
                        .filter(|i| i.status == CloseAllItemStatus::Queued)
                        .map(|i| i.address.clone())
                        .collect()
                })
                .unwrap_or_default()
        };
        if let Err(e) = disable_automation_for_positions(&state, &addresses).await {
            warn!(batch_id = %batch_id, error = %e, "close-all: pause_linked_strategies partial failure");
        }
    }

    let queued: Vec<(String, String)> = {
        let jobs = CLOSE_ALL_JOBS.read().await;
        jobs.get(&batch_id)
            .map(|j| {
                j.items
                    .iter()
                    .filter(|i| i.status == CloseAllItemStatus::Queued)
                    .filter_map(|i| {
                        Some((
                            i.address.clone(),
                            i.close_signer_wallet_id.clone()?,
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default()
    };

    let mut by_wallet: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (addr, wallet_id) in queued {
        by_wallet.entry(wallet_id).or_default().push(addr);
    }

    let mut handles = Vec::with_capacity(by_wallet.len());
    for (wallet_id, addrs) in by_wallet {
        let state = state.clone();
        let batch_id = batch_id.clone();
        handles.push(tokio::spawn(async move {
            close_wallet_group(
                state,
                batch_id,
                wallet_id,
                addrs,
                skip_pre_collect,
                send_first,
                slippage_bps,
            )
            .await;
        }));
    }
    for handle in handles {
        if let Err(e) = handle.await {
            warn!(batch_id = %batch_id, error = %e, "close-all: wallet group task join error");
        }
    }

    finish_job(&batch_id, "done").await;
    info!(batch_id = %batch_id, "close-all worker finished");
}

/// Resolve signer groups without starting a batch (for confirm UI).
pub async fn preview_close_all(
    state: &AppState,
    req: CloseAllPositionsRequest,
) -> Result<CloseAllPositionsPreviewResponse, ApiError> {
    parse_close_all_send_mode(&req.options.send_mode)?;
    let _ = validate_close_all_slippage_bps(req.options.slippage_bps)?;
    let addresses = resolve_close_all_addresses(state, &req).await?;
    let (items, groups, skipped_preview) = plan_close_all(state, &addresses).await;
    let closable = items
        .iter()
        .filter(|i| i.status == CloseAllItemStatus::Queued)
        .count() as u32;
    Ok(CloseAllPositionsPreviewResponse {
        total: addresses.len() as u32,
        closable,
        groups,
        skipped_preview,
    })
}

/// Start a close-all batch; returns immediately with `batch_id`.
pub async fn start_close_all_batch(
    state: AppState,
    req: CloseAllPositionsRequest,
) -> Result<CloseAllPositionsStartResponse, ApiError> {
    parse_close_all_send_mode(&req.options.send_mode)?;
    let _ = validate_close_all_slippage_bps(req.options.slippage_bps)?;
    let addresses = resolve_close_all_addresses(&state, &req).await?;
    let (items, groups, skipped_preview) = plan_close_all(&state, &addresses).await;
    let batch_id = Uuid::new_v4().to_string();
    let closable = items
        .iter()
        .filter(|i| i.status == CloseAllItemStatus::Queued)
        .count() as u32;

    let job = CloseAllJob {
        batch_id: batch_id.clone(),
        status: if closable == 0 {
            "done".to_string()
        } else {
            "running".to_string()
        },
        started_ts_utc: Utc::now(),
        finished_ts_utc: if closable == 0 {
            Some(Utc::now())
        } else {
            None
        },
        items,
    };
    CLOSE_ALL_JOBS.write().await.insert(batch_id.clone(), job);

    if closable > 0 {
        let worker_state = state.clone();
        let worker_req = req.clone();
        let worker_batch = batch_id.clone();
        tokio::spawn(async move {
            run_close_all_worker(worker_state, worker_batch, worker_req).await;
        });
    }

    Ok(CloseAllPositionsStartResponse {
        batch_id,
        status: "queued".to_string(),
        total: addresses.len() as u32,
        groups,
        skipped_preview,
    })
}

pub async fn get_close_all_batch(batch_id: &str) -> Result<CloseAllBatchStatusResponse, ApiError> {
    let jobs = CLOSE_ALL_JOBS.read().await;
    let job = jobs
        .get(batch_id.trim())
        .ok_or_else(|| ApiError::not_found("Close-all batch not found"))?;
    Ok(job_to_response(job))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_excludes_filters_addresses() {
        let req = CloseAllPositionsRequest {
            scope: "explicit".to_string(),
            addresses: vec!["a".to_string(), "b".to_string()],
            exclude_addresses: vec!["b".to_string()],
            pause_linked_strategies: true,
            options: Default::default(),
        };
        let got = apply_scope_and_excludes(&req, vec![]).expect("ok");
        assert_eq!(got, vec!["a".to_string()]);
    }

    #[test]
    fn explicit_scope_uses_request_addresses_without_monitored_union() {
        let req = CloseAllPositionsRequest {
            scope: "explicit".to_string(),
            addresses: vec!["pda_one".to_string(), "pda_two".to_string()],
            exclude_addresses: vec![],
            pause_linked_strategies: true,
            options: Default::default(),
        };
        let got = apply_scope_and_excludes(&req, vec![]).expect("ok");
        assert_eq!(got, vec!["pda_one".to_string(), "pda_two".to_string()]);
    }

    #[test]
    fn parse_send_mode_accepts_known_values() {
        assert!(!parse_close_all_send_mode("confirm_sync").unwrap());
        assert!(parse_close_all_send_mode("send_first").unwrap());
        assert!(parse_close_all_send_mode("fast").is_err());
    }
}
