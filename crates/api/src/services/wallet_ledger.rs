//! Append-only **wallet ledger** (GL-style journal) for application-originated on-chain actions.
//!
//! **Write:** `data/wallet-ledger-events.jsonl` (override: `CLMM_WALLET_LEDGER_PATH`) plus best-effort
//! dual-write to Postgres `wallet_gl_journal_event` when `state.db` is connected.
//! **Read:** Postgres when rows exist, else JSONL tail (see `read_wallet_ledger_events`).

use crate::models::{WalletLedgerDelta, WalletLedgerEvent, WalletLedgerStatus};
use crate::services::wallet_gl_posting;
use crate::state::AppState;
use chrono::{DateTime, Utc};
use clmm_lp_data::repositories::Database;
use clmm_lp_protocols::prelude::RpcProvider;
use solana_sdk::program_pack::Pack;
use solana_sdk::pubkey::Pubkey;
use spl_token::state::Mint;
use sqlx::Row;
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

fn wallet_ledger_status_str(status: WalletLedgerStatus) -> &'static str {
    match status {
        WalletLedgerStatus::Pending => "pending",
        WalletLedgerStatus::Confirmed => "confirmed",
        WalletLedgerStatus::Failed => "failed",
    }
}

fn parse_wallet_ledger_status(s: &str) -> WalletLedgerStatus {
    match s.trim().to_ascii_lowercase().as_str() {
        "confirmed" => WalletLedgerStatus::Confirmed,
        "failed" => WalletLedgerStatus::Failed,
        _ => WalletLedgerStatus::Pending,
    }
}

/// Build a new ledger line (caller fills `deltas`, `error`, etc.).
#[allow(clippy::too_many_arguments)]
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
        ts_utc: Utc::now().to_rfc3339(),
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

async fn persist_wallet_ledger_event_pg(db: &Database, ev: &WalletLedgerEvent) {
    let ts = DateTime::parse_from_rfc3339(&ev.ts_utc)
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    let deltas_json = match serde_json::to_value(&ev.deltas) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "wallet_ledger: deltas_json serialize failed");
            return;
        }
    };
    let status = wallet_ledger_status_str(ev.status);
    if let Err(e) = sqlx::query(
        r#"
        INSERT INTO wallet_gl_journal_event (
            event_id, schema_version, ts_utc, correlation_id, status, kind,
            owner, signature, pool_address, position_pda, cost_session_id,
            dry_run, native_lamports_delta, deltas_json, error, source
        ) VALUES (
            $1, $2, $3, $4, $5, $6,
            $7, $8, $9, $10, $11,
            $12, $13, $14, $15, $16
        )
        ON CONFLICT (event_id) DO NOTHING
        "#,
    )
    .bind(&ev.event_id)
    .bind(ev.schema_version as i32)
    .bind(ts)
    .bind(&ev.correlation_id)
    .bind(status)
    .bind(&ev.kind)
    .bind(ev.owner.as_deref())
    .bind(ev.signature.as_deref())
    .bind(ev.pool_address.as_deref())
    .bind(ev.position_pda.as_deref())
    .bind(ev.cost_session_id.as_deref())
    .bind(ev.dry_run)
    .bind(ev.native_lamports_delta.as_deref())
    .bind(deltas_json)
    .bind(ev.error.as_deref())
    .bind(&ev.source)
    .execute(db.pool())
    .await
    {
        tracing::warn!(error = %e, event_id = %ev.event_id, "wallet_ledger: postgres insert failed");
    }
}

fn wallet_ledger_event_from_pg_row(row: &sqlx::postgres::PgRow) -> Option<WalletLedgerEvent> {
    let ts: DateTime<Utc> = row.try_get("ts_utc").ok()?;
    let deltas_json: serde_json::Value = row.try_get("deltas_json").ok()?;
    let deltas: Vec<WalletLedgerDelta> = serde_json::from_value(deltas_json).ok()?;
    let status_s: String = row.try_get("status").ok()?;
    Some(WalletLedgerEvent {
        schema_version: row.try_get::<i32, _>("schema_version").unwrap_or(1) as u32,
        ts_utc: ts.to_rfc3339(),
        event_id: row.try_get("event_id").ok()?,
        correlation_id: row.try_get("correlation_id").ok()?,
        status: parse_wallet_ledger_status(&status_s),
        kind: row.try_get("kind").ok()?,
        owner: row.try_get("owner").ok(),
        signature: row.try_get("signature").ok(),
        pool_address: row.try_get("pool_address").ok(),
        position_pda: row.try_get("position_pda").ok(),
        cost_session_id: row.try_get("cost_session_id").ok(),
        dry_run: row.try_get("dry_run").unwrap_or(false),
        native_lamports_delta: row.try_get("native_lamports_delta").ok(),
        deltas,
        error: row.try_get("error").ok(),
        source: row.try_get("source").ok()?,
    })
}

async fn read_wallet_ledger_tail_pg(
    db: &Database,
    limit: usize,
    owner_filter: Option<&str>,
    kind_filter: Option<&str>,
    status_filter: Option<&str>,
) -> Result<Vec<WalletLedgerEvent>, sqlx::Error> {
    let cap = limit.clamp(1, 500) as i64;
    let owner_like = owner_filter
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|o| format!("%{o}%"));
    let kind = kind_filter.map(str::trim).filter(|s| !s.is_empty());
    let status = status_filter.map(str::trim).filter(|s| !s.is_empty());

    let rows = sqlx::query(
        r#"
        SELECT
            schema_version, ts_utc, event_id, correlation_id, status, kind,
            owner, signature, pool_address, position_pda, cost_session_id,
            dry_run, native_lamports_delta, deltas_json, error, source
        FROM wallet_gl_journal_event
        WHERE ($1::text IS NULL OR owner ILIKE $1)
          AND ($2::text IS NULL OR kind ILIKE '%' || $2 || '%')
          AND ($3::text IS NULL OR status = $3)
        ORDER BY ts_utc DESC, created_at DESC
        LIMIT $4
        "#,
    )
    .bind(owner_like)
    .bind(kind)
    .bind(status)
    .bind(cap)
    .fetch_all(db.pool())
    .await?;

    Ok(rows
        .iter()
        .filter_map(wallet_ledger_event_from_pg_row)
        .collect())
}

pub async fn append_wallet_ledger_event(state: &AppState, ev: WalletLedgerEvent) {
    let path = wallet_ledger_events_path();
    let _guard = state.wallet_ledger_append_lock.lock().await;
    if let Some(dir) = path.parent()
        && let Err(e) = fs::create_dir_all(dir).await
    {
        tracing::warn!(error = %e, dir = %dir.display(), "wallet_ledger: create_dir_all failed");
        return;
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
        return;
    }

    if let Some(db) = state.db.as_ref() {
        persist_wallet_ledger_event_pg(db, &ev).await;
        wallet_gl_posting::apply_session_postings_from_journal(db, &ev).await;
    }
}

/// Whether a ledger row matches optional tail-read filters (used by API + tests).
pub fn wallet_ledger_event_matches_filters(
    ev: &WalletLedgerEvent,
    owner_filter: Option<&str>,
    kind_filter: Option<&str>,
    status_filter: Option<&str>,
) -> bool {
    if let Some(o) = owner_filter.map(str::trim).filter(|s| !s.is_empty()) {
        let matches = ev
            .owner
            .as_deref()
            .is_some_and(|ow| ow == o || ow.contains(o) || o.contains(ow));
        if !matches {
            return false;
        }
    }
    if let Some(k) = kind_filter.map(str::trim).filter(|s| !s.is_empty()) {
        if ev.kind != k && !ev.kind.contains(k) {
            return false;
        }
    }
    if let Some(s) = status_filter.map(str::trim).filter(|s| !s.is_empty()) {
        let got = wallet_ledger_status_str(ev.status);
        if got != s && !got.contains(s) {
            return false;
        }
    }
    true
}

/// Read the last `limit` events from JSONL (newest first), optionally filtered.
pub async fn read_wallet_ledger_tail(
    limit: usize,
    owner_filter: Option<&str>,
    kind_filter: Option<&str>,
    status_filter: Option<&str>,
) -> (PathBuf, Vec<WalletLedgerEvent>) {
    let path = wallet_ledger_events_path();
    let cap = limit.clamp(1, 500);
    let body = match fs::read_to_string(&path).await {
        Ok(s) => s,
        Err(_) => return (path, Vec::new()),
    };
    let mut out: Vec<WalletLedgerEvent> = Vec::new();
    for line in body.lines().rev() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(ev) = serde_json::from_str::<WalletLedgerEvent>(line) else {
            continue;
        };
        if !wallet_ledger_event_matches_filters(&ev, owner_filter, kind_filter, status_filter) {
            continue;
        }
        out.push(ev);
        if out.len() >= cap {
            break;
        }
    }
    (path, out)
}

/// Prefer Postgres when rows exist; otherwise JSONL (legacy file before backfill).
pub async fn read_wallet_ledger_events(
    state: &AppState,
    limit: usize,
    owner_filter: Option<&str>,
    kind_filter: Option<&str>,
    status_filter: Option<&str>,
) -> (PathBuf, &'static str, Vec<WalletLedgerEvent>) {
    let path = wallet_ledger_events_path();
    if let Some(db) = state.db.as_ref() {
        match read_wallet_ledger_tail_pg(db, limit, owner_filter, kind_filter, status_filter).await
        {
            Ok(events) if !events.is_empty() => return (path, "postgres", events),
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(error = %e, "wallet_ledger: postgres read failed, using JSONL");
            }
        }
    }
    let (_, events) =
        read_wallet_ledger_tail(limit, owner_filter, kind_filter, status_filter).await;
    let storage = if state.db.is_some() {
        "jsonl_fallback"
    } else {
        "jsonl"
    };
    (path, storage, events)
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

    #[test]
    fn wallet_ledger_event_matches_filters_owner_kind_status() {
        let ev = new_ledger_event(
            "c",
            WalletLedgerStatus::Confirmed,
            "open_position",
            Some("8abcOwner".to_string()),
            None,
            None,
            None,
            None,
            false,
            None,
            vec![],
            None,
            "api:positions",
        );
        assert!(wallet_ledger_event_matches_filters(
            &ev,
            Some("8abc"),
            None,
            None
        ));
        assert!(wallet_ledger_event_matches_filters(
            &ev,
            None,
            Some("open_position"),
            None
        ));
        assert!(wallet_ledger_event_matches_filters(
            &ev,
            None,
            None,
            Some("confirmed")
        ));
        assert!(!wallet_ledger_event_matches_filters(
            &ev,
            Some("other"),
            None,
            None
        ));
        assert!(!wallet_ledger_event_matches_filters(
            &ev,
            None,
            Some("close_position"),
            None
        ));
        assert!(!wallet_ledger_event_matches_filters(
            &ev,
            None,
            None,
            Some("pending")
        ));
    }

    #[test]
    fn wallet_ledger_status_str_roundtrip() {
        assert_eq!(
            wallet_ledger_status_str(WalletLedgerStatus::Confirmed),
            "confirmed"
        );
        assert_eq!(
            parse_wallet_ledger_status("failed"),
            WalletLedgerStatus::Failed
        );
    }
}
