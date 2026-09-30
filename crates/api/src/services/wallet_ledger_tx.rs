//! Wallet GL Phase B — journal helpers for unsigned-tx build/submit flow.

use crate::models::{SubmitSignedTxRequest, WalletLedgerStatus};
use crate::services::wallet_ledger;
use crate::state::AppState;

pub const KIND_SWAP_BEFORE_OPEN: &str = "swap_before_open";
pub const KIND_OPEN_POSITION: &str = "open_position";
pub const KIND_CLOSE_POSITION: &str = "close_position";
pub const KIND_COLLECT_FEES: &str = "collect_fees";
pub const KIND_DECREASE_LIQUIDITY: &str = "decrease_liquidity";
pub const KIND_INCREASE_LIQUIDITY: &str = "increase_liquidity";
pub const KIND_REBALANCE_POSITION: &str = "rebalance_position";
pub const KIND_TRANSFER_SOL: &str = "transfer_sol";
pub const KIND_CONVERT_SOL: &str = "convert_sol";

/// All `kind` values emitted by API handlers (Phase B coverage registry).
pub const KNOWN_LEDGER_KINDS: &[&str] = &[
    KIND_SWAP_BEFORE_OPEN,
    KIND_OPEN_POSITION,
    KIND_CLOSE_POSITION,
    KIND_COLLECT_FEES,
    KIND_DECREASE_LIQUIDITY,
    KIND_INCREASE_LIQUIDITY,
    KIND_REBALANCE_POSITION,
    KIND_TRANSFER_SOL,
    KIND_CONVERT_SOL,
];

/// Returns true when `kind` is a registered Phase B journal kind.
#[must_use]
pub fn is_known_ledger_kind(kind: &str) -> bool {
    let k = kind.trim();
    !k.is_empty() && KNOWN_LEDGER_KINDS.contains(&k)
}

/// Audit metadata for `POST /tx/submit-signed` (optional; journal only when complete).
#[derive(Debug, Clone)]
pub struct TxSubmitLedgerAudit {
    pub correlation_id: String,
    pub kind: String,
    pub wallet_pubkey: String,
    pub pool_address: Option<String>,
    pub position_address: Option<String>,
    pub cost_session_id: Option<String>,
}

impl TxSubmitLedgerAudit {
    /// Build audit context from submit body when all required fields are present.
    #[must_use]
    pub fn from_submit_request(req: &SubmitSignedTxRequest) -> Option<Self> {
        let correlation_id = req
            .correlation_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())?
            .to_string();
        let kind = req
            .ledger_kind
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .filter(|k| is_known_ledger_kind(k))
            .map(str::to_string)?;
        let wallet_pubkey = req
            .wallet_pubkey
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())?
            .to_string();
        Some(Self {
            correlation_id,
            kind,
            wallet_pubkey,
            pool_address: trim_opt(req.pool_address.as_deref()),
            position_address: trim_opt(req.position_address.as_deref()),
            cost_session_id: trim_opt(req.cost_session_id.as_deref()),
        })
    }
}

fn trim_opt(v: Option<&str>) -> Option<String> {
    v.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub async fn append_tx_submit_pending(state: &AppState, audit: &TxSubmitLedgerAudit) {
    if state.dry_run {
        return;
    }
    let ev = wallet_ledger::new_ledger_event(
        &audit.correlation_id,
        WalletLedgerStatus::Pending,
        &audit.kind,
        Some(audit.wallet_pubkey.clone()),
        None,
        audit.pool_address.clone(),
        audit.position_address.clone(),
        audit.cost_session_id.clone(),
        false,
        None,
        vec![],
        None,
        "api:tx/submit-signed",
    );
    wallet_ledger::append_wallet_ledger_event(state, ev).await;
}

pub async fn append_tx_submit_outcome(
    state: &AppState,
    audit: &TxSubmitLedgerAudit,
    status: WalletLedgerStatus,
    signature: Option<String>,
    error: Option<String>,
) {
    if state.dry_run {
        return;
    }
    let ev = wallet_ledger::new_ledger_event(
        &audit.correlation_id,
        status,
        &audit.kind,
        Some(audit.wallet_pubkey.clone()),
        signature,
        audit.pool_address.clone(),
        audit.position_address.clone(),
        audit.cost_session_id.clone(),
        false,
        None,
        vec![],
        error,
        "api:tx/submit-signed",
    );
    let mut ev = ev;
    if matches!(status, WalletLedgerStatus::Confirmed) && ev.deltas.is_empty() {
        ev.decode_status = Some(crate::services::wallet_ledger::decode_status::PENDING_DECODE.to_string());
    }
    wallet_ledger::append_wallet_ledger_event(state, ev).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_ledger_kinds_includes_tx_flow_kinds() {
        assert!(is_known_ledger_kind(KIND_INCREASE_LIQUIDITY));
        assert!(is_known_ledger_kind(KIND_OPEN_POSITION));
        assert!(!is_known_ledger_kind("unknown_kind"));
        assert!(!is_known_ledger_kind(""));
    }

    #[test]
    fn tx_submit_audit_requires_correlation_kind_wallet() {
        let full = SubmitSignedTxRequest {
            signed_tx_base64: "x".to_string(),
            chain_history_anchors: None,
            correlation_id: Some("cid-1".to_string()),
            ledger_kind: Some(KIND_INCREASE_LIQUIDITY.to_string()),
            wallet_pubkey: Some("Wallet111111111111111111111111111111111111111".to_string()),
            pool_address: Some(" Pool111111111111111111111111111111111111111 ".to_string()),
            position_address: None,
            cost_session_id: None,
        };
        let audit = TxSubmitLedgerAudit::from_submit_request(&full).expect("audit");
        assert_eq!(audit.correlation_id, "cid-1");
        assert_eq!(audit.kind, KIND_INCREASE_LIQUIDITY);
        assert_eq!(
            audit.pool_address.as_deref(),
            Some("Pool111111111111111111111111111111111111111")
        );

        let missing = SubmitSignedTxRequest {
            signed_tx_base64: "x".to_string(),
            chain_history_anchors: None,
            correlation_id: None,
            ledger_kind: Some(KIND_OPEN_POSITION.to_string()),
            wallet_pubkey: Some("w".to_string()),
            pool_address: None,
            position_address: None,
            cost_session_id: None,
        };
        assert!(TxSubmitLedgerAudit::from_submit_request(&missing).is_none());
    }
}
