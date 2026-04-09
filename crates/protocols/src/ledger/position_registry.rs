//! Append-only **open / close** registry for LP positions (operator + collector contract).
//!
//! Default file: `data/positions/registry.jsonl`  
//! Override: `CLMM_POSITION_REGISTRY_PATH`
//!
//! Rows are append-only. To obtain **currently open** positions, replay the file: for each
//! `position_pubkey`, the latest `registry_open` vs `registry_close` determines state. After a
//! `registry_close`, collectors **must not** treat that position as active for position-scoped jobs.

use crate::ledger::tx_lifecycle::rebalance_session_id_from_env;
use crate::rpc::RpcProvider;
use anyhow::{Context, Result};
use serde::Serialize;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Signature;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use tracing::warn;

const DEFAULT_REL_PATH: &str = "data/positions/registry.jsonl";

#[must_use]
pub fn registry_path() -> PathBuf {
    if let Ok(p) = std::env::var("CLMM_POSITION_REGISTRY_PATH") {
        let t = p.trim();
        if !t.is_empty() {
            return PathBuf::from(t);
        }
    }
    PathBuf::from(DEFAULT_REL_PATH)
}

fn append_jsonl<T: Serialize>(rec: &T) -> Result<()> {
    let path = registry_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create_dir_all {:?}", parent))?;
    }
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("open registry {:?}", path))?;
    let line = serde_json::to_string(rec).context("serialize registry row")?;
    writeln!(f, "{line}")?;
    Ok(())
}

#[derive(Serialize)]
struct RegistryRow<'a> {
    schema_version: u32,
    ts_utc: String,
    /// `registry_open` | `registry_close`
    event: &'a str,
    /// `cli` | `orca_bot`
    source: &'a str,
    /// Extra classification for closes (e.g. `manual`, `strategy`, `rotation`).
    /// Omitted for opens and legacy rows.
    #[serde(skip_serializing_if = "Option::is_none")]
    close_kind: Option<&'a str>,
    position_pubkey: String,
    pool_address: String,
    owner_pubkey: String,
    signature: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    rebalance_session_id: Option<String>,
    /// Optional structured snapshot of the position configuration at open (ticks, strategy params, UX intent, etc.).
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<serde_json::Value>,
    rpc_url: String,
    accounting_note: &'static str,
}

fn row(
    event: &'static str,
    source: &'static str,
    position: &Pubkey,
    pool: &Pubkey,
    owner: &Pubkey,
    signature: &Signature,
    note: &'static str,
    rebalance_session_id: Option<String>,
    details: Option<serde_json::Value>,
    close_kind: Option<&'static str>,
) -> RegistryRow<'static> {
    RegistryRow {
        schema_version: 1,
        ts_utc: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        event,
        source,
        close_kind,
        position_pubkey: position.to_string(),
        pool_address: pool.to_string(),
        owner_pubkey: owner.to_string(),
        signature: signature.to_string(),
        rebalance_session_id: rebalance_session_id.or_else(rebalance_session_id_from_env),
        details,
        rpc_url: String::new(),
        accounting_note: note,
    }
}

/// Best-effort: record that a position is **open** (after successful on-chain open).
pub async fn try_append_registry_open(
    provider: &RpcProvider,
    source: &'static str,
    position: &Pubkey,
    pool: &Pubkey,
    owner: &Pubkey,
    signature: &Signature,
    rebalance_session_id_override: Option<String>,
    details: Option<serde_json::Value>,
) {
    if let Err(e) = append_open_inner(
        provider,
        source,
        position,
        pool,
        owner,
        signature,
        rebalance_session_id_override,
        details,
    )
    .await
    {
        warn!(error = %e, "position registry: append registry_open failed");
    }
}

async fn append_open_inner(
    provider: &RpcProvider,
    source: &'static str,
    position: &Pubkey,
    pool: &Pubkey,
    owner: &Pubkey,
    signature: &Signature,
    rebalance_session_id_override: Option<String>,
    details: Option<serde_json::Value>,
) -> Result<()> {
    let mut r = row(
        "registry_open",
        source,
        position,
        pool,
        owner,
        signature,
        "Append-only; collectors may attach per-position jobs until registry_close for this position_pubkey.",
        rebalance_session_id_override,
        details,
        None,
    );
    r.rpc_url = provider.current_endpoint().await;
    append_jsonl(&r).context("registry_open jsonl")
}

/// Best-effort: record that a position is **closed** (collectors should drop position-scoped work).
pub async fn try_append_registry_close(
    provider: &RpcProvider,
    source: &'static str,
    position: &Pubkey,
    pool: &Pubkey,
    owner: &Pubkey,
    signature: &Signature,
    rebalance_session_id_override: Option<String>,
    close_kind: Option<&'static str>,
) {
    if let Err(e) = append_close_inner(
        provider,
        source,
        position,
        pool,
        owner,
        signature,
        rebalance_session_id_override,
        close_kind,
    )
    .await
    {
        warn!(error = %e, "position registry: append registry_close failed");
    }
}

async fn append_close_inner(
    provider: &RpcProvider,
    source: &'static str,
    position: &Pubkey,
    pool: &Pubkey,
    owner: &Pubkey,
    signature: &Signature,
    rebalance_session_id_override: Option<String>,
    close_kind: Option<&'static str>,
) -> Result<()> {
    let mut r = row(
        "registry_close",
        source,
        position,
        pool,
        owner,
        signature,
        "Append-only; last registry_close vs registry_open per position_pubkey defines whether position is active.",
        rebalance_session_id_override,
        None,
        close_kind,
    );
    r.rpc_url = provider.current_endpoint().await;
    append_jsonl(&r).context("registry_close jsonl")
}
