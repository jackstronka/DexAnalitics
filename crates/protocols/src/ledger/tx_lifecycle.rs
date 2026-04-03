//! Shared helpers for `data/ledger/orca_position_lifecycle.jsonl` (schema_version 2).
//!
//! Rows from the rebalance executor use `source: "orca_bot"` and `event: bot_*`.
//! CLI position open/close uses `source: "cli"` and `event: position_open` / `position_close`.
//! CLI Orca swap uses `event: cli_swap`.
//!
//! **Rebalance + swap total cost:** set the same optional `CLMM_REBALANCE_SESSION_ID` in the shell
//! for the whole sequence (swap CLI → bot / `orca-position-open`, etc.); each row may carry
//! `rebalance_session_id` so you can sum `tx_fee_lamports` or `fee_payer_net_lamports_delta` per session.

use crate::rpc::RpcProvider;
use anyhow::{Context, Result};
use serde::Serialize;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Signature;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;
use tracing::warn;

const DEFAULT_REL_PATH: &str = "data/ledger/orca_position_lifecycle.jsonl";

/// Optional correlation id for **one** rebalance workflow (swap + close/open txs).
/// When set in the environment, append helpers include it on each JSONL row so totals can be summed.
#[must_use]
pub fn rebalance_session_id_from_env() -> Option<String> {
    std::env::var("CLMM_REBALANCE_SESSION_ID")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Optional IL / rebalance JSONL path (`orca-bot-run --il-ledger-path` / **`CLMM_IL_LEDGER_PATH`**).
/// Unlike [`ledger_path`], there is **no** default file — unset means IL rows are not persisted to disk unless the flag is passed.
#[must_use]
pub fn il_ledger_path_from_env() -> Option<PathBuf> {
    std::env::var("CLMM_IL_LEDGER_PATH")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
}

/// Default / env-resolved ledger path (same as CLI lifecycle ledger).
#[must_use]
pub fn ledger_path() -> PathBuf {
    if let Ok(p) = std::env::var("CLMM_POSITION_LIFECYCLE_LEDGER_PATH") {
        let t = p.trim();
        if !t.is_empty() {
            return PathBuf::from(t);
        }
    }
    if let Ok(p) = std::env::var("CLMM_POSITION_OPEN_LEDGER_PATH") {
        let t = p.trim();
        if !t.is_empty() {
            return PathBuf::from(t);
        }
    }
    PathBuf::from(DEFAULT_REL_PATH)
}

fn message_static_pubkeys(tx_root: &serde_json::Value) -> Option<Vec<String>> {
    let keys = tx_root
        .get("transaction")?
        .get("message")?
        .get("accountKeys")?
        .as_array()?;
    let mut out = Vec::with_capacity(keys.len());
    for k in keys {
        if let Some(s) = k.as_str() {
            out.push(s.to_string());
        } else {
            let pk = k.get("pubkey").and_then(|x| x.as_str()).or_else(|| {
                k.get("pubKey").and_then(|x| x.as_str())
            })?;
            out.push(pk.to_string());
        }
    }
    Some(out)
}

fn fee_payer_balance_delta(
    tx_root: &serde_json::Value,
    fee_payer: &Pubkey,
) -> (Option<u64>, Option<u64>, Option<i64>) {
    let Some(keys) = message_static_pubkeys(tx_root) else {
        return (None, None, None);
    };
    let Some(idx) = keys.iter().position(|k| k == &fee_payer.to_string()) else {
        return (None, None, None);
    };
    let pre = tx_root
        .get("meta")
        .and_then(|m| m.get("preBalances"))
        .and_then(|a| a.get(idx))
        .and_then(|x| x.as_u64());
    let post = tx_root
        .get("meta")
        .and_then(|m| m.get("postBalances"))
        .and_then(|a| a.get(idx))
        .and_then(|x| x.as_u64());
    match (pre, post) {
        (Some(pre), Some(post)) => {
            let delta = i64::try_from(post).unwrap_or(i64::MAX)
                - i64::try_from(pre).unwrap_or(i64::MAX);
            (Some(pre), Some(post), Some(delta))
        }
        _ => (None, None, None),
    }
}

async fn fetch_tx_json_with_retry(
    provider: &RpcProvider,
    signature: &Signature,
) -> Result<serde_json::Value> {
    let mut last_err = None;
    for attempt in 0u32..10 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(80 * u64::from(attempt))).await;
        }
        match provider.get_transaction_json_parsed(signature).await {
            Ok(enc) => return serde_json::to_value(&enc).context("tx to json"),
            Err(e) => {
                last_err = Some(e);
            }
        }
    }
    Err(last_err
        .map(|e| anyhow::anyhow!(e))
        .unwrap_or_else(|| anyhow::anyhow!("get_transaction failed")))
}

/// Fetches parsed tx JSON and returns network fee + fee payer balance delta (best-effort).
pub async fn enrich_tx_costs(
    provider: &RpcProvider,
    signature: &Signature,
    fee_payer: &Pubkey,
) -> (u64, Option<u64>, Option<u64>, Option<u64>, Option<i64>) {
    match fetch_tx_json_with_retry(provider, signature).await {
        Ok(v) => {
            let fee = v
                .pointer("/meta/fee")
                .and_then(|x| x.as_u64())
                .unwrap_or(0);
            let slot = v.get("slot").and_then(|x| x.as_u64());
            let (pre, post, delta) = fee_payer_balance_delta(&v, fee_payer);
            (fee, slot, pre, post, delta)
        }
        Err(e) => {
            warn!(err = %e, "tx lifecycle ledger: could not fetch transaction");
            (0, None, None, None, None)
        }
    }
}

/// Append one JSON line to the lifecycle ledger file.
pub fn append_jsonl_line<T: Serialize>(rec: &T) -> Result<()> {
    let path = ledger_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create_dir_all {:?}", parent))?;
    }
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("open ledger {:?}", path))?;
    let line = serde_json::to_string(rec).context("serialize ledger row")?;
    writeln!(f, "{line}")?;
    Ok(())
}

fn rebalance_event_name(operation: &str) -> &'static str {
    match operation {
        "collect_fees" => "bot_collect_fees",
        "decrease_liquidity" => "bot_decrease_liquidity",
        "close_position" => "bot_close_position",
        "open_full_range_position" => "bot_open_position_full_range",
        "open_position" => "bot_open_position",
        "swap_exact_in" => "bot_swap_exact_in",
        _ => "bot_orca_tx",
    }
}

/// Best-effort append after a successful Orca op from the execution-layer rebalance executor.
///
/// `position` is the position PDA when known before the tx; `created_position` fills in open flows.
///
/// When `rebalance_session_id_override` is set, it is written as `rebalance_session_id` on the row
/// (API / UI correlation for swap + open). Otherwise [`rebalance_session_id_from_env`] is used.
pub async fn try_append_rebalance_executor_tx_cost(
    provider: &RpcProvider,
    fee_payer: &Pubkey,
    signature: &Signature,
    operation: &str,
    pool: Option<Pubkey>,
    position: Option<Pubkey>,
    created_position: Option<Pubkey>,
    rebalance_session_id_override: Option<String>,
) {
    if let Err(e) = append_rebalance_inner(
        provider,
        fee_payer,
        signature,
        operation,
        pool,
        position,
        created_position,
        rebalance_session_id_override,
    )
    .await
    {
        warn!(error = %e, "rebalance tx lifecycle ledger: append failed");
    }
}

async fn append_rebalance_inner(
    provider: &RpcProvider,
    fee_payer: &Pubkey,
    signature: &Signature,
    operation: &str,
    pool: Option<Pubkey>,
    position: Option<Pubkey>,
    created_position: Option<Pubkey>,
    rebalance_session_id_override: Option<String>,
) -> Result<()> {
    let (tx_fee, slot, pre, post, delta) = enrich_tx_costs(provider, signature, fee_payer).await;
    let rpc_url = provider.current_endpoint().await;
    let effective_position = position.or(created_position);

    #[derive(Serialize)]
    struct Row<'a> {
        schema_version: u32,
        ts_utc: String,
        event: &'a str,
        source: &'a str,
        operation: &'a str,
        signature: String,
        pool_address: Option<String>,
        position_pubkey: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        rebalance_session_id: Option<String>,
        tx_fee_lamports: u64,
        fee_payer_pubkey: String,
        fee_payer_pre_lamports: Option<u64>,
        fee_payer_post_lamports: Option<u64>,
        fee_payer_net_lamports_delta: Option<i64>,
        accounting_note: &'static str,
        slot: Option<u64>,
        rpc_url: String,
    }

    let rec = Row {
        schema_version: 2,
        ts_utc: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        event: rebalance_event_name(operation),
        source: "orca_bot",
        operation,
        signature: signature.to_string(),
        pool_address: pool.map(|p| p.to_string()),
        position_pubkey: effective_position.map(|p| p.to_string()),
        rebalance_session_id: rebalance_session_id_override.or_else(rebalance_session_id_from_env),
        tx_fee_lamports: tx_fee,
        fee_payer_pubkey: fee_payer.to_string(),
        fee_payer_pre_lamports: pre,
        fee_payer_post_lamports: post,
        fee_payer_net_lamports_delta: delta,
        accounting_note: "tx_fee_lamports=network fee only; fee_payer_net_lamports_delta includes fee+rent+token/SOL legs affecting fee payer. source=orca_bot joins IL/fee ledgers via pool_address/position_pubkey. Sum rows with same rebalance_session_id for swap+rebalance total.",
        slot,
        rpc_url,
    };

    append_jsonl_line(&rec)
}

/// Best-effort append after a successful **`orca-swap`** CLI tx (Whirlpool swap).
pub async fn try_append_cli_swap_tx_cost(
    provider: &RpcProvider,
    fee_payer: &Pubkey,
    signature: &Signature,
    pool: &Pubkey,
    rebalance_session_id_override: Option<String>,
) {
    if let Err(e) =
        append_cli_swap_inner(provider, fee_payer, signature, pool, rebalance_session_id_override)
            .await
    {
        warn!(error = %e, "cli swap tx lifecycle ledger: append failed");
    }
}

async fn append_cli_swap_inner(
    provider: &RpcProvider,
    fee_payer: &Pubkey,
    signature: &Signature,
    pool: &Pubkey,
    rebalance_session_id_override: Option<String>,
) -> Result<()> {
    let (tx_fee, slot, pre, post, delta) = enrich_tx_costs(provider, signature, fee_payer).await;
    let rpc_url = provider.current_endpoint().await;

    #[derive(Serialize)]
    struct Row {
        schema_version: u32,
        ts_utc: String,
        event: &'static str,
        source: &'static str,
        operation: &'static str,
        signature: String,
        pool_address: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        position_pubkey: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        rebalance_session_id: Option<String>,
        tx_fee_lamports: u64,
        fee_payer_pubkey: String,
        fee_payer_pre_lamports: Option<u64>,
        fee_payer_post_lamports: Option<u64>,
        fee_payer_net_lamports_delta: Option<i64>,
        accounting_note: &'static str,
        slot: Option<u64>,
        rpc_url: String,
    }

    let rec = Row {
        schema_version: 2,
        ts_utc: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        event: "cli_swap",
        source: "cli",
        operation: "orca_whirlpool_swap",
        signature: signature.to_string(),
        pool_address: pool.to_string(),
        position_pubkey: None,
        rebalance_session_id: rebalance_session_id_override.or_else(rebalance_session_id_from_env),
        tx_fee_lamports: tx_fee,
        fee_payer_pubkey: fee_payer.to_string(),
        fee_payer_pre_lamports: pre,
        fee_payer_post_lamports: post,
        fee_payer_net_lamports_delta: delta,
        accounting_note: "Whirlpool swap only; sum with bot_* and position_* rows sharing rebalance_session_id for full rebalance+open cost.",
        slot,
        rpc_url,
    };

    append_jsonl_line(&rec)
}
