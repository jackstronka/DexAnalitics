//! Append-only **wallet ledger** (GL-style journal) for application-originated on-chain actions.
//!
//! Events are written to `data/wallet-ledger-events.jsonl` (override: `CLMM_WALLET_LEDGER_PATH`).
//! Reads are best-effort tail scans; the UI does not consume this for balances yet.

use crate::models::{WalletLedgerDelta, WalletLedgerEvent, WalletLedgerStatus};
use crate::state::AppState;
use clmm_lp_protocols::prelude::RpcProvider;
use solana_sdk::program_pack::Pack;
use solana_sdk::pubkey::Pubkey;
use spl_token::state::Mint;
use std::path::PathBuf;
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

const DEFAULT_LEDGER_REL_PATH: &str = "data/wallet-ledger-events.jsonl";
pub const WSOL_MINT: &str = "So11111111111111111111111111111111111111112";

pub fn wallet_ledger_events_path() -> PathBuf {
    std::env::var("CLMM_WALLET_LEDGER_PATH")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_LEDGER_REL_PATH))
}

pub async fn fetch_mint_decimals_best_effort(provider: &RpcProvider, mint: &Pubkey) -> u8 {
    match provider.get_account(mint).await {
        Ok(acc) => Mint::unpack(&acc.data).map(|m| m.decimals).unwrap_or(9),
        Err(_) => 9,
    }
}

/// Build a new ledger line (caller fills `deltas`, `error`, etc.).
pub fn new_ledger_event(
    correlation_id: &str,
    status: WalletLedgerStatus,
    kind: &str,
    owner: Option<String>,
    signature: Option<String>,
    pool_address: Option<String>,
    position_pda: Option<String>,
    cost_session_id: Option<String>,
    dry_run: bool,
    native_lamports_delta: Option<i64>,
    deltas: Vec<WalletLedgerDelta>,
    error: Option<String>,
    source: &str,
) -> WalletLedgerEvent {
    WalletLedgerEvent {
        schema_version: 1,
        ts_utc: chrono::Utc::now().to_rfc3339(),
        event_id: Uuid::new_v4().to_string(),
        correlation_id: correlation_id.to_string(),
        status,
        kind: kind.to_string(),
        owner,
        signature,
        pool_address,
        position_pda,
        cost_session_id,
        dry_run,
        native_lamports_delta: native_lamports_delta.map(|n| n.to_string()),
        deltas,
        error,
        source: source.to_string(),
    }
}

pub async fn append_wallet_ledger_event(state: &AppState, ev: WalletLedgerEvent) {
    let path = wallet_ledger_events_path();
    let _guard = state.wallet_ledger_append_lock.lock().await;
    if let Some(dir) = path.parent() {
        if let Err(e) = fs::create_dir_all(dir).await {
            tracing::warn!(error = %e, dir = %dir.display(), "wallet_ledger: create_dir_all failed");
            return;
        }
    }
    let line = match serde_json::to_string(&ev) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "wallet_ledger: serialize failed");
            return;
        }
    };
    let mut f = match OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .await
    {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(error = %e, path = %path.display(), "wallet_ledger: open failed");
            return;
        }
    };
    if let Err(e) = f.write_all(line.as_bytes()).await {
        tracing::warn!(error = %e, path = %path.display(), "wallet_ledger: write failed");
        return;
    }
    if let Err(e) = f.write_all(b"\n").await {
        tracing::warn!(error = %e, path = %path.display(), "wallet_ledger: newline failed");
    }
}

/// Read the last `limit` events (newest first), optionally filtered by `owner` substring match.
pub async fn read_wallet_ledger_tail(
    limit: usize,
    owner_filter: Option<&str>,
) -> (PathBuf, Vec<WalletLedgerEvent>) {
    let path = wallet_ledger_events_path();
    let cap = limit.min(500).max(1);
    let body = match fs::read_to_string(&path).await {
        Ok(s) => s,
        Err(_) => return (path, Vec::new()),
    };
    let owner_f = owner_filter.map(|s| s.trim()).filter(|s| !s.is_empty());
    let mut out: Vec<WalletLedgerEvent> = Vec::new();
    for line in body.lines().rev() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(ev) = serde_json::from_str::<WalletLedgerEvent>(line) else {
            continue;
        };
        if let Some(o) = owner_f {
            let matches = ev
                .owner
                .as_deref()
                .is_some_and(|ow| ow == o || ow.contains(o) || o.contains(ow));
            if !matches {
                continue;
            }
        }
        out.push(ev);
        if out.len() >= cap {
            break;
        }
    }
    (path, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wallet_ledger_event_roundtrips_json() {
        let ev = new_ledger_event(
            "corr-1",
            WalletLedgerStatus::Pending,
            "swap_before_open",
            Some("8abc".to_string()),
            None,
            Some("pool1".to_string()),
            None,
            Some("sess".to_string()),
            false,
            None,
            vec![WalletLedgerDelta {
                mint: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string(),
                decimals: 6,
                raw_delta_i128: "-1000".to_string(),
            }],
            None,
            "api:positions",
        );
        let s = serde_json::to_string(&ev).unwrap();
        let back: WalletLedgerEvent = serde_json::from_str(&s).unwrap();
        assert_eq!(back.correlation_id, "corr-1");
        assert_eq!(back.status, WalletLedgerStatus::Pending);
        assert_eq!(back.deltas.len(), 1);
    }
}
