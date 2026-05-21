//! Shared manual close execution (single + bulk).

use crate::error::ApiError;
use crate::models::{
    MessageResponse, WalletLedgerStatus, CLOSE_ALL_DEFAULT_SLIPPAGE_BPS, CLOSE_ALL_MAX_SLIPPAGE_BPS,
};
use crate::services::position_chain_history::spawn_chain_history_materialize_background;
use crate::services::position_executor::build_ephemeral_position_executor;
use crate::services::position_valuation::monitored_position_from_chain;
use crate::services::strategy_service::remove_position_address_from_all_strategies;
use crate::services::wallet_ledger;
use crate::services::PositionService;
use crate::state::{AppState, PositionUpdate};
use clmm_lp_execution::prelude::Wallet;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Signature;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tracing::warn;
use uuid::Uuid;

/// Context for wallet-ledger rows during close.
#[derive(Debug, Clone)]
pub struct ManualCloseLedgerContext {
    pub correlation_id: String,
    pub ledger_owner: String,
    pub close_kind: &'static str,
    pub batch_id: Option<String>,
    /// Bulk fast path: one close tx (Orca bundles fee collect); default false for single close.
    pub skip_pre_collect: bool,
    /// Close min-out slippage (bps) for bulk send-first / retry.
    pub slippage_bps: u16,
    /// One automatic 6018 retry with raised slippage (bulk send-first).
    pub slippage_6018_retry_done: bool,
}

/// Resolve bulk close slippage from request (default 200 bps, cap 2000).
#[must_use]
pub fn resolve_bulk_close_slippage_bps(opt: Option<u16>) -> u16 {
    opt.unwrap_or(CLOSE_ALL_DEFAULT_SLIPPAGE_BPS)
        .min(CLOSE_ALL_MAX_SLIPPAGE_BPS)
}

/// Raised slippage for a single 6018 retry after failed confirm/submit.
#[must_use]
pub fn bump_close_slippage_bps_for_6018_retry(base: u16) -> u16 {
    const MIN_RETRY_BPS: u16 = 500;
    base.saturating_mul(2)
        .max(MIN_RETRY_BPS)
        .min(CLOSE_ALL_MAX_SLIPPAGE_BPS)
}

/// True when error text indicates Whirlpool close min-out failure (6018).
#[must_use]
pub fn is_close_slippage_6018(err: &str) -> bool {
    let s = err.to_lowercase();
    s.contains("6018")
        || s.contains("0x1782")
        || s.contains("tokenminsubceeded")
        || s.contains("token_min_subceeded")
}

impl ManualCloseLedgerContext {
    pub fn new_single() -> Self {
        Self {
            correlation_id: Uuid::new_v4().to_string(),
            ledger_owner: String::new(),
            close_kind: "manual",
            batch_id: None,
            skip_pre_collect: false,
            slippage_bps: resolve_bulk_close_slippage_bps(None),
            slippage_6018_retry_done: false,
        }
    }

    pub fn for_bulk(
        batch_id: &str,
        ledger_owner: &str,
        skip_pre_collect: bool,
        slippage_bps: u16,
    ) -> Self {
        Self {
            correlation_id: Uuid::new_v4().to_string(),
            ledger_owner: ledger_owner.to_string(),
            close_kind: "manual_bulk",
            batch_id: Some(batch_id.to_string()),
            skip_pre_collect,
            slippage_bps,
            slippage_6018_retry_done: false,
        }
    }
}

/// Outcome of send-first submit (before background confirm).
#[derive(Debug, Clone)]
pub enum ManualCloseSubmitOutcome {
    Submitted(ManualCloseInFlight),
    AlreadyClosed,
}

/// State carried between send-first submit and background finalize.
#[derive(Debug, Clone)]
pub struct ManualCloseInFlight {
    pub ledger_ctx: ManualCloseLedgerContext,
    pub pos_pda: String,
    pub pool_str: String,
    pub pubkey: Pubkey,
    pub cost_session_id: Option<String>,
    pub signature: String,
    pub collect_fee_owed_a_raw: Option<u64>,
    pub collect_fee_owed_b_raw: Option<u64>,
}

async fn complete_manual_close_success(
    state: &AppState,
    pubkey: &Pubkey,
    pos_pda: &str,
    already_closed: bool,
) {
    if let Err(e) = remove_position_address_from_all_strategies(state, pos_pda).await {
        warn!(
            position = %pos_pda,
            error = %e,
            "close_position: strategy unlink failed after manual close (continuing)"
        );
    }
    state.monitor.remove_position(pubkey).await;
    state
        .broadcast_position_update(PositionUpdate {
            update_type: "closed".to_string(),
            position_address: pos_pda.to_string(),
            timestamp: chrono::Utc::now(),
            data: serde_json::json!({}),
        })
        .await;
    if !already_closed {
        spawn_chain_history_materialize_background(state, pos_pda.to_string(), "close_position");
    }
}

async fn position_snapshot(state: &AppState, pubkey: &Pubkey) -> Result<clmm_lp_execution::monitor::MonitoredPosition, ApiError> {
    let positions = state.monitor.get_positions().await;
    if let Some(p) = positions.iter().find(|p| p.address == *pubkey) {
        return Ok(p.clone());
    }
    monitored_position_from_chain(state.provider.clone(), pubkey).await
}

/// Execute one manual close with an explicit API wallet (does not touch global active signer).
pub async fn execute_manual_close_with_wallet(
    state: &AppState,
    address: &str,
    cost_session_id: Option<String>,
    wallet: Arc<Wallet>,
    mut ledger_ctx: ManualCloseLedgerContext,
) -> Result<MessageResponse, ApiError> {
    let pubkey = Pubkey::from_str(address.trim())
        .map_err(|_| ApiError::bad_request("Invalid position address"))?;

    if state.dry_run {
        let position_snapshot = position_snapshot(state, &pubkey).await?;
        state
            .broadcast_position_update(PositionUpdate {
                update_type: "close_simulated".to_string(),
                position_address: address.to_string(),
                timestamp: chrono::Utc::now(),
                data: serde_json::json!({
                    "liquidity": position_snapshot.on_chain.liquidity.to_string(),
                    "dry_run": true
                }),
            })
            .await;
        return Ok(MessageResponse::new(format!(
            "[DRY-RUN] Would close position {} with liquidity {}",
            address, position_snapshot.on_chain.liquidity
        )));
    }

    let position_snapshot = position_snapshot(state, &pubkey).await?;
    let pool_str = position_snapshot.pool.to_string();
    let pos_pda = address.trim().to_string();
    if ledger_ctx.ledger_owner.is_empty() {
        ledger_ctx.ledger_owner = wallet.pubkey().to_string();
    }

    let pending = wallet_ledger::new_ledger_event(
        &ledger_ctx.correlation_id,
        WalletLedgerStatus::Pending,
        "close_position",
        Some(ledger_ctx.ledger_owner.clone()),
        None,
        Some(pool_str.clone()),
        Some(pos_pda.clone()),
        cost_session_id.clone(),
        false,
        None,
        vec![],
        None,
        "api:positions",
    );
    wallet_ledger::append_wallet_ledger_event(state, pending).await;

    let executor = build_ephemeral_position_executor(state, wallet);
    let mut svc = PositionService::new(state.clone());
    svc.set_dry_run(false);
    svc.set_executor(executor);

    let op = match svc
        .close_position(&pos_pda, cost_session_id.clone(), ledger_ctx.skip_pre_collect)
        .await
    {
        Ok(o) => o,
        Err(e) => {
            let fail = wallet_ledger::new_ledger_event(
                &ledger_ctx.correlation_id,
                WalletLedgerStatus::Failed,
                "close_position",
                Some(ledger_ctx.ledger_owner.clone()),
                None,
                Some(pool_str.clone()),
                Some(pos_pda.clone()),
                cost_session_id.clone(),
                false,
                None,
                vec![],
                Some(e.to_string()),
                "api:positions",
            );
            wallet_ledger::append_wallet_ledger_event(state, fail).await;
            return Err(e);
        }
    };

    if op.success {
        let sig = op.signature.clone();
        let conf = wallet_ledger::new_ledger_event(
            &ledger_ctx.correlation_id,
            WalletLedgerStatus::Confirmed,
            "close_position",
            Some(ledger_ctx.ledger_owner.clone()),
            sig,
            Some(pool_str.clone()),
            Some(pos_pda.clone()),
            cost_session_id.clone(),
            false,
            None,
            vec![],
            None,
            "api:positions",
        );
        wallet_ledger::append_wallet_ledger_event(state, conf).await;

        let already = op
            .data
            .as_ref()
            .and_then(|d| d.get("already_closed_on_chain"))
            .and_then(|v| v.as_bool())
            .filter(|b| *b)
            .unwrap_or(false);
        complete_manual_close_success(state, &pubkey, &pos_pda, already).await;

        let note = op
            .data
            .as_ref()
            .and_then(|d| d.get("already_closed_on_chain"))
            .and_then(|v| v.as_bool())
            .filter(|b| *b)
            .map(|_| " (already closed on-chain)".to_string())
            .unwrap_or_default();
        Ok(MessageResponse::new(format!(
            "Position closed: {pos_pda}{note}"
        )))
    } else {
        let fail = wallet_ledger::new_ledger_event(
            &ledger_ctx.correlation_id,
            WalletLedgerStatus::Failed,
            "close_position",
            Some(ledger_ctx.ledger_owner.clone()),
            None,
            Some(pool_str.clone()),
            Some(pos_pda.clone()),
            cost_session_id.clone(),
            false,
            None,
            vec![],
            op.error
                .clone()
                .or_else(|| Some("Position closing failed".to_string())),
            "api:positions",
        );
        wallet_ledger::append_wallet_ledger_event(state, fail).await;
        Err(ApiError::ServiceUnavailable(
            op.error
                .unwrap_or_else(|| "Position closing failed".to_string()),
        ))
    }
}

/// Send-first bulk close: submit tx, return signature + in-flight context for finalize.
pub async fn submit_manual_close_send_first(
    state: &AppState,
    address: &str,
    cost_session_id: Option<String>,
    wallet: Arc<Wallet>,
    mut ledger_ctx: ManualCloseLedgerContext,
) -> Result<ManualCloseSubmitOutcome, ApiError> {
    let pubkey = Pubkey::from_str(address.trim())
        .map_err(|_| ApiError::bad_request("Invalid position address"))?;

    if state.dry_run {
        let position_snapshot = position_snapshot(state, &pubkey).await?;
        state
            .broadcast_position_update(PositionUpdate {
                update_type: "close_simulated".to_string(),
                position_address: address.to_string(),
                timestamp: chrono::Utc::now(),
                data: serde_json::json!({
                    "liquidity": position_snapshot.on_chain.liquidity.to_string(),
                    "dry_run": true,
                    "send_mode": "send_first",
                }),
            })
            .await;
        return Err(ApiError::bad_request(
            "[DRY-RUN] send_first close not submitted",
        ));
    }

    let position_snapshot = position_snapshot(state, &pubkey).await?;
    let pool_str = position_snapshot.pool.to_string();
    let pos_pda = address.trim().to_string();
    if ledger_ctx.ledger_owner.is_empty() {
        ledger_ctx.ledger_owner = wallet.pubkey().to_string();
    }

    let pending = wallet_ledger::new_ledger_event(
        &ledger_ctx.correlation_id,
        WalletLedgerStatus::Pending,
        "close_position",
        Some(ledger_ctx.ledger_owner.clone()),
        None,
        Some(pool_str.clone()),
        Some(pos_pda.clone()),
        cost_session_id.clone(),
        false,
        None,
        vec![],
        None,
        "api:positions",
    );
    wallet_ledger::append_wallet_ledger_event(state, pending).await;

    let executor = build_ephemeral_position_executor(state, wallet);
    let mut svc = PositionService::new(state.clone());
    svc.set_dry_run(false);
    svc.set_executor(executor);

    let op = match svc
        .close_position_submit_only(
            &pos_pda,
            ledger_ctx.skip_pre_collect,
            Some(ledger_ctx.slippage_bps),
        )
        .await
    {
        Ok(o) => o,
        Err(e) => {
            let fail = wallet_ledger::new_ledger_event(
                &ledger_ctx.correlation_id,
                WalletLedgerStatus::Failed,
                "close_position",
                Some(ledger_ctx.ledger_owner.clone()),
                None,
                Some(pool_str.clone()),
                Some(pos_pda.clone()),
                cost_session_id.clone(),
                false,
                None,
                vec![],
                Some(e.to_string()),
                "api:positions",
            );
            wallet_ledger::append_wallet_ledger_event(state, fail).await;
            return Err(e);
        }
    };

    if !op.success {
        let fail = wallet_ledger::new_ledger_event(
            &ledger_ctx.correlation_id,
            WalletLedgerStatus::Failed,
            "close_position",
            Some(ledger_ctx.ledger_owner.clone()),
            None,
            Some(pool_str.clone()),
            Some(pos_pda.clone()),
            cost_session_id.clone(),
            false,
            None,
            vec![],
            op.error
                .clone()
                .or_else(|| Some("Position close submit failed".to_string())),
            "api:positions",
        );
        wallet_ledger::append_wallet_ledger_event(state, fail).await;
        return Err(ApiError::ServiceUnavailable(
            op.error
                .unwrap_or_else(|| "Position close submit failed".to_string()),
        ));
    }

    if op
        .data
        .as_ref()
        .and_then(|d| d.get("already_closed_on_chain"))
        .and_then(|v| v.as_bool())
        == Some(true)
    {
        let conf = wallet_ledger::new_ledger_event(
            &ledger_ctx.correlation_id,
            WalletLedgerStatus::Confirmed,
            "close_position",
            Some(ledger_ctx.ledger_owner.clone()),
            op.signature.clone(),
            Some(pool_str.clone()),
            Some(pos_pda.clone()),
            cost_session_id.clone(),
            false,
            None,
            vec![],
            None,
            "api:positions",
        );
        wallet_ledger::append_wallet_ledger_event(state, conf).await;
        complete_manual_close_success(state, &pubkey, &pos_pda, true).await;
        return Ok(ManualCloseSubmitOutcome::AlreadyClosed);
    }

    let signature = op
        .signature
        .clone()
        .ok_or_else(|| ApiError::ServiceUnavailable("Close submit missing signature".into()))?;

    let submitted_ledger = wallet_ledger::new_ledger_event(
        &ledger_ctx.correlation_id,
        WalletLedgerStatus::Pending,
        "close_position",
        Some(ledger_ctx.ledger_owner.clone()),
        Some(signature.clone()),
        Some(pool_str.clone()),
        Some(pos_pda.clone()),
        cost_session_id.clone(),
        false,
        None,
        vec![],
        None,
        "api:positions",
    );
    wallet_ledger::append_wallet_ledger_event(state, submitted_ledger).await;

    let collect_fee_owed_a_raw = op
        .data
        .as_ref()
        .and_then(|d| d.get("collect_fee_owed_a_raw"))
        .and_then(|v| v.as_u64());
    let collect_fee_owed_b_raw = op
        .data
        .as_ref()
        .and_then(|d| d.get("collect_fee_owed_b_raw"))
        .and_then(|v| v.as_u64());

    Ok(ManualCloseSubmitOutcome::Submitted(ManualCloseInFlight {
        ledger_ctx,
        pos_pda,
        pool_str,
        pubkey,
        cost_session_id,
        signature,
        collect_fee_owed_a_raw,
        collect_fee_owed_b_raw,
    }))
}

/// Poll until a submitted close signature reaches a terminal on-chain state (or timeout).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloseSignaturePoll {
    Pending,
    Confirmed,
    Failed(String),
}

pub async fn poll_close_signature_until_terminal(
    state: &AppState,
    signature: &str,
    max_secs: u64,
) -> CloseSignaturePoll {
    let Ok(sig) = Signature::from_str(signature.trim()) else {
        return CloseSignaturePoll::Pending;
    };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(max_secs);
    while tokio::time::Instant::now() < deadline {
        match state.provider.get_signature_status(&sig).await {
            Ok(Some(status)) => {
                if let Some(err) = status.err {
                    return CloseSignaturePoll::Failed(format!("{err:?}"));
                }
                return CloseSignaturePoll::Confirmed;
            }
            Ok(None) => {}
            Err(_) => {}
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    CloseSignaturePoll::Pending
}

async fn finalize_manual_close_attempt(
    state: &AppState,
    wallet: Arc<Wallet>,
    flight: &ManualCloseInFlight,
) -> Result<(), ApiError> {
    let executor = build_ephemeral_position_executor(state, wallet);
    let mut svc = PositionService::new(state.clone());
    svc.set_dry_run(false);
    svc.set_executor(executor);

    let op = svc
        .finalize_close_submit_only(
            &flight.pos_pda,
            &flight.signature,
            flight.collect_fee_owed_a_raw,
            flight.collect_fee_owed_b_raw,
        )
        .await;

    match op {
        Ok(o) if o.success => {
            let conf = wallet_ledger::new_ledger_event(
                &flight.ledger_ctx.correlation_id,
                WalletLedgerStatus::Confirmed,
                "close_position",
                Some(flight.ledger_ctx.ledger_owner.clone()),
                Some(flight.signature.clone()),
                Some(flight.pool_str.clone()),
                Some(flight.pos_pda.clone()),
                flight.cost_session_id.clone(),
                false,
                None,
                vec![],
                None,
                "api:positions",
            );
            wallet_ledger::append_wallet_ledger_event(state, conf).await;
            complete_manual_close_success(state, &flight.pubkey, &flight.pos_pda, false).await;
            Ok(())
        }
        Ok(o) => {
            let msg = o
                .error
                .clone()
                .unwrap_or_else(|| "Position finalize failed".to_string());
            let fail = wallet_ledger::new_ledger_event(
                &flight.ledger_ctx.correlation_id,
                WalletLedgerStatus::Failed,
                "close_position",
                Some(flight.ledger_ctx.ledger_owner.clone()),
                Some(flight.signature.clone()),
                Some(flight.pool_str.clone()),
                Some(flight.pos_pda.clone()),
                flight.cost_session_id.clone(),
                false,
                None,
                vec![],
                Some(msg.clone()),
                "api:positions",
            );
            wallet_ledger::append_wallet_ledger_event(state, fail).await;
            Err(ApiError::ServiceUnavailable(msg))
        }
        Err(e) => {
            let fail = wallet_ledger::new_ledger_event(
                &flight.ledger_ctx.correlation_id,
                WalletLedgerStatus::Failed,
                "close_position",
                Some(flight.ledger_ctx.ledger_owner.clone()),
                Some(flight.signature.clone()),
                Some(flight.pool_str.clone()),
                Some(flight.pos_pda.clone()),
                flight.cost_session_id.clone(),
                false,
                None,
                vec![],
                Some(e.to_string()),
                "api:positions",
            );
            wallet_ledger::append_wallet_ledger_event(state, fail).await;
            Err(e)
        }
    }
}

/// Background finalize after send-first submit (one automatic 6018 retry with higher slippage).
/// Returns the confirmed transaction signature (may differ after 6018 retry).
pub async fn finalize_manual_close_send_first(
    state: &AppState,
    wallet: Arc<Wallet>,
    mut flight: ManualCloseInFlight,
) -> Result<String, ApiError> {
    loop {
        match finalize_manual_close_attempt(state, wallet.clone(), &flight).await {
            Ok(()) => return Ok(flight.signature),
            Err(e) => {
                let msg = e.to_string();
                if flight.ledger_ctx.slippage_6018_retry_done || !is_close_slippage_6018(&msg) {
                    return Err(e);
                }
                let bumped =
                    bump_close_slippage_bps_for_6018_retry(flight.ledger_ctx.slippage_bps);
                if bumped <= flight.ledger_ctx.slippage_bps {
                    return Err(e);
                }
                flight.ledger_ctx.slippage_6018_retry_done = true;
                flight.ledger_ctx.slippage_bps = bumped;
                tracing::info!(
                    position = %flight.pos_pda,
                    slippage_bps = bumped,
                    "send_first close: 6018 on confirm, retrying submit with higher slippage"
                );
                let addr = flight.pos_pda.clone();
                let cost_session_id = flight.cost_session_id.clone();
                let ctx = flight.ledger_ctx.clone();
                match submit_manual_close_send_first(
                    state,
                    &addr,
                    cost_session_id,
                    wallet.clone(),
                    ctx,
                )
                .await
                {
                    Ok(ManualCloseSubmitOutcome::Submitted(new_flight)) => {
                        flight = new_flight;
                    }
                    Ok(ManualCloseSubmitOutcome::AlreadyClosed) => {
                        complete_manual_close_success(state, &flight.pubkey, &flight.pos_pda, true)
                            .await;
                        return Ok(flight.signature);
                    }
                    Err(e2) => return Err(e2),
                }
            }
        }
    }
}

/// Short wait between sends on the same fee payer (processed, not finalized).
pub async fn wait_close_signature_processed(state: &AppState, signature: &str, max_secs: u64) {
    let Ok(sig) = Signature::from_str(signature.trim()) else {
        return;
    };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(max_secs);
    while tokio::time::Instant::now() < deadline {
        match state.provider.get_signature_status(&sig).await {
            Ok(Some(status)) => {
                if status.err.is_some() {
                    return;
                }
                return;
            }
            Ok(None) => {}
            Err(_) => {}
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_bulk_close_slippage_defaults_to_200() {
        assert_eq!(resolve_bulk_close_slippage_bps(None), 200);
        assert_eq!(resolve_bulk_close_slippage_bps(Some(150)), 150);
        assert_eq!(resolve_bulk_close_slippage_bps(Some(5000)), 2000);
    }

    #[test]
    fn bump_close_slippage_for_6018_retry() {
        assert_eq!(bump_close_slippage_bps_for_6018_retry(200), 500);
        assert_eq!(bump_close_slippage_bps_for_6018_retry(400), 800);
        assert_eq!(bump_close_slippage_bps_for_6018_retry(600), 1200);
        assert_eq!(bump_close_slippage_bps_for_6018_retry(2000), 2000);
    }

    #[test]
    fn is_close_slippage_6018_detects_known_markers() {
        assert!(is_close_slippage_6018(
            "close confirm: Transaction failed: InstructionError(2, Custom(6018))"
        ));
        assert!(is_close_slippage_6018("TokenMinSubceeded"));
        assert!(!is_close_slippage_6018("insufficient funds"));
    }
}

