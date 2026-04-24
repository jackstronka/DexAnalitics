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
use std::io::{BufRead, BufReader};
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
///
/// **Appends** (bot, CLI) always use this path. For **reads** (API aggregates, lineage), prefer
/// [`ledger_read_path`] so an offline-enriched copy can be used without moving the canonical file.
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

fn env_truthy(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .map(|s| {
            matches!(
                s.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

/// Sibling path produced by `enrich-lifecycle-ledger` (same directory, `<stem>.enriched.jsonl`).
#[must_use]
pub fn enriched_ledger_path_candidate() -> PathBuf {
    let base = ledger_path();
    let s = base.to_string_lossy();
    if s.ends_with(".jsonl") {
        PathBuf::from(format!("{}.enriched.jsonl", s.trim_end_matches(".jsonl")))
    } else {
        let mut p = base.clone();
        p.set_extension("enriched.jsonl");
        p
    }
}

/// Path for **reading** lifecycle JSONL rows (dashboard, lineage, aggregates).
///
/// Resolution order:
/// 1. **`CLMM_POSITION_LIFECYCLE_LEDGER_READ_PATH`** — explicit path when set and non-empty.
/// 2. If **`CLMM_POSITION_LIFECYCLE_USE_ENRICHED`** is truthy (`1` / `true` / `yes` / `on`) **and**
///    [`enriched_ledger_path_candidate`] exists as a regular file — use it (RPC-recomputed
///    `fee_payer_token_deltas` from `enrich-lifecycle-ledger`).
/// 3. Otherwise [`ledger_path`] (same file the bot appends to).
#[must_use]
pub fn ledger_read_path() -> PathBuf {
    if let Ok(p) = std::env::var("CLMM_POSITION_LIFECYCLE_LEDGER_READ_PATH") {
        let t = p.trim();
        if !t.is_empty() {
            return PathBuf::from(t);
        }
    }
    if env_truthy("CLMM_POSITION_LIFECYCLE_USE_ENRICHED") {
        let cand = enriched_ledger_path_candidate();
        if cand.is_file() {
            return cand;
        }
    }
    ledger_path()
}

/// Best-effort check if lifecycle ledger already contains a bot open row for a session id.
///
/// Used by execution guardrails to avoid duplicate opens in one `rebalance_session_id`.
#[must_use]
pub fn session_has_bot_open_position(rebalance_session_id: &str) -> bool {
    let sid = rebalance_session_id.trim();
    if sid.is_empty() {
        return false;
    }
    let p = ledger_read_path();
    let Ok(f) = std::fs::File::open(&p) else {
        return false;
    };
    let r = BufReader::new(f);
    for line in r.lines().map_while(Result::ok) {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(t) else {
            continue;
        };
        let ev = v.get("event").and_then(|x| x.as_str()).unwrap_or("").trim();
        if ev != "bot_open_position" && ev != "bot_open_position_full_range" {
            continue;
        }
        let row_sid = v
            .get("rebalance_session_id")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .trim();
        if row_sid == sid {
            return true;
        }
    }
    false
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
            let pk = k
                .get("pubkey")
                .and_then(|x| x.as_str())
                .or_else(|| k.get("pubKey").and_then(|x| x.as_str()))?;
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
            let delta =
                i64::try_from(post).unwrap_or(i64::MAX) - i64::try_from(pre).unwrap_or(i64::MAX);
            (Some(pre), Some(post), Some(delta))
        }
        _ => (None, None, None),
    }
}

/// Compute per-mint UI deltas for SPL token balances owned by `fee_payer`.
///
/// Uses tx `meta.preTokenBalances`/`postTokenBalances` and **sums across all token accounts** per mint
/// (important for WSOL/temp accounts).
#[must_use]
pub fn fee_payer_token_deltas_by_mint(
    tx_root: &serde_json::Value,
    fee_payer: &Pubkey,
) -> serde_json::Value {
    use rust_decimal::Decimal;
    use std::collections::BTreeMap;
    use std::collections::BTreeSet;
    use std::str::FromStr;

    fn ui_amount_from_token_balance_entry(e: &serde_json::Value) -> Option<Decimal> {
        let ui = e
            .get("uiTokenAmount")
            .or_else(|| e.get("ui_token_amount"))?;
        if let Some(s) = ui.get("uiAmountString").and_then(|x| x.as_str()) {
            return Decimal::from_str(s.trim()).ok();
        }
        if let Some(f) = ui.get("uiAmount").and_then(|x| x.as_f64()) {
            return Decimal::from_str(&format!("{f:.18}")).ok();
        }
        None
    }

    let owner_s = fee_payer.to_string();
    let mut pre: BTreeMap<String, Decimal> = BTreeMap::new();
    let mut post: BTreeMap<String, Decimal> = BTreeMap::new();

    if let Some(arr) = tx_root
        .get("meta")
        .and_then(|m| m.get("preTokenBalances"))
        .and_then(|x| x.as_array())
    {
        for e in arr {
            let owner = e.get("owner").and_then(|x| x.as_str()).unwrap_or("");
            if owner != owner_s {
                continue;
            }
            let mint = e.get("mint").and_then(|x| x.as_str()).unwrap_or("");
            if mint.is_empty() {
                continue;
            }
            if let Some(v) = ui_amount_from_token_balance_entry(e) {
                // Multiple token accounts for the same mint may exist (ATA + temp WSOL, etc.).
                // Sum UI amounts across all accounts owned by the fee payer.
                *pre.entry(mint.to_string()).or_insert(Decimal::ZERO) += v;
            }
        }
    }
    if let Some(arr) = tx_root
        .get("meta")
        .and_then(|m| m.get("postTokenBalances"))
        .and_then(|x| x.as_array())
    {
        for e in arr {
            let owner = e.get("owner").and_then(|x| x.as_str()).unwrap_or("");
            if owner != owner_s {
                continue;
            }
            let mint = e.get("mint").and_then(|x| x.as_str()).unwrap_or("");
            if mint.is_empty() {
                continue;
            }
            if let Some(v) = ui_amount_from_token_balance_entry(e) {
                *post.entry(mint.to_string()).or_insert(Decimal::ZERO) += v;
            }
        }
    }

    let mut out = serde_json::Map::new();
    let mut all: BTreeSet<String> = BTreeSet::new();
    all.extend(pre.keys().cloned());
    all.extend(post.keys().cloned());
    for mint in all {
        let a = pre.get(&mint).cloned().unwrap_or(Decimal::ZERO);
        let b = post.get(&mint).cloned().unwrap_or(Decimal::ZERO);
        let d = b - a;
        if !d.is_zero() {
            out.insert(mint, serde_json::Value::String(d.to_string()));
        }
    }
    serde_json::Value::Object(out)
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
) -> (
    u64,
    Option<u64>,
    Option<u64>,
    Option<u64>,
    Option<i64>,
    Option<serde_json::Value>,
) {
    match fetch_tx_json_with_retry(provider, signature).await {
        Ok(v) => {
            let fee = v.pointer("/meta/fee").and_then(|x| x.as_u64()).unwrap_or(0);
            let slot = v.get("slot").and_then(|x| x.as_u64());
            let (pre, post, delta) = fee_payer_balance_delta(&v, fee_payer);
            (fee, slot, pre, post, delta, Some(v))
        }
        Err(e) => {
            warn!(err = %e, "tx lifecycle ledger: could not fetch transaction");
            (0, None, None, None, None, None)
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
///
/// `lp_collected_token_{a,b}_raw`: for `collect_fees` only — `fee_owed_a` / `fee_owed_b` from the
/// Whirlpool **position** account immediately before the harvest tx (pool token A/B order). Records
/// both LP legs even when RPC `pre/postTokenBalances` omit WSOL. For other operations, pass `None`.
///
/// For **`open_position`** / **`open_full_range_position`**, callers may merge `details` fields such
/// as **`open_origin: "operator_api"`** (dashboard/API open). Lineage treats that as an operator
/// mint with **no** stitched prior rotation history (same rule as CLI `position_open`).
#[allow(clippy::too_many_arguments)]
pub async fn try_append_rebalance_executor_tx_cost(
    provider: &RpcProvider,
    fee_payer: &Pubkey,
    signature: &Signature,
    operation: &str,
    pool: Option<Pubkey>,
    position: Option<Pubkey>,
    created_position: Option<Pubkey>,
    rebalance_session_id_override: Option<String>,
    // Extra structured fields for operators (e.g. swap mints + amount_in for swap_exact_in).
    details: Option<serde_json::Value>,
    lp_collected_token_a_raw: Option<u64>,
    lp_collected_token_b_raw: Option<u64>,
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
        details,
        lp_collected_token_a_raw,
        lp_collected_token_b_raw,
    )
    .await
    {
        warn!(error = %e, "rebalance tx lifecycle ledger: append failed");
    }
}

#[allow(clippy::too_many_arguments)]
async fn append_rebalance_inner(
    provider: &RpcProvider,
    fee_payer: &Pubkey,
    signature: &Signature,
    operation: &str,
    pool: Option<Pubkey>,
    position: Option<Pubkey>,
    created_position: Option<Pubkey>,
    rebalance_session_id_override: Option<String>,
    details: Option<serde_json::Value>,
    lp_collected_token_a_raw: Option<u64>,
    lp_collected_token_b_raw: Option<u64>,
) -> Result<()> {
    let (tx_fee, slot, pre, post, delta, tx_json) =
        enrich_tx_costs(provider, signature, fee_payer).await;
    let fee_payer_token_deltas = tx_json
        .as_ref()
        .map(|v| fee_payer_token_deltas_by_mint(v, fee_payer))
        .filter(|v| !v.as_object().is_some_and(|m| m.is_empty()));
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
        #[serde(skip_serializing_if = "Option::is_none")]
        details: Option<serde_json::Value>,
        tx_fee_lamports: u64,
        fee_payer_pubkey: String,
        fee_payer_pre_lamports: Option<u64>,
        fee_payer_post_lamports: Option<u64>,
        fee_payer_net_lamports_delta: Option<i64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        fee_payer_token_deltas: Option<serde_json::Value>,
        /// Pool leg A/B raw amounts harvested (from position `fee_owed_*` before tx), when `operation == collect_fees`.
        #[serde(skip_serializing_if = "Option::is_none")]
        lp_collected_token_a_raw: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        lp_collected_token_b_raw: Option<u64>,
        accounting_note: &'static str,
        slot: Option<u64>,
        rpc_url: String,
    }

    let (lp_a, lp_b) = if operation == "collect_fees" {
        (lp_collected_token_a_raw, lp_collected_token_b_raw)
    } else {
        (None, None)
    };

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
        details,
        tx_fee_lamports: tx_fee,
        fee_payer_pubkey: fee_payer.to_string(),
        fee_payer_pre_lamports: pre,
        fee_payer_post_lamports: post,
        fee_payer_net_lamports_delta: delta,
        fee_payer_token_deltas,
        lp_collected_token_a_raw: lp_a,
        lp_collected_token_b_raw: lp_b,
        accounting_note: "tx_fee_lamports=network fee only; fee_payer_net_lamports_delta includes fee+rent+token/SOL legs affecting fee payer. fee_payer_token_deltas is a mint->Δ(ui) map derived from meta pre/postTokenBalances (owner=fee payer). For collect_fees, lp_collected_token_{a,b}_raw = position fee_owed_{a,b} read immediately before harvest (both pool legs). Sum rows with same rebalance_session_id for swap+rebalance total.",
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
    if let Err(e) = append_cli_swap_inner(
        provider,
        fee_payer,
        signature,
        pool,
        rebalance_session_id_override,
    )
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
    let (tx_fee, slot, pre, post, delta, tx_json) =
        enrich_tx_costs(provider, signature, fee_payer).await;
    let fee_payer_token_deltas = tx_json
        .as_ref()
        .map(|v| fee_payer_token_deltas_by_mint(v, fee_payer))
        .filter(|v| !v.as_object().is_some_and(|m| m.is_empty()));
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
        #[serde(skip_serializing_if = "Option::is_none")]
        fee_payer_token_deltas: Option<serde_json::Value>,
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
        fee_payer_token_deltas,
        accounting_note: "Whirlpool swap only; fee_payer_token_deltas is a mint->Δ(ui) map for the fee payer. Sum with bot_* and position_* rows sharing rebalance_session_id for full rebalance+open cost.",
        slot,
        rpc_url,
    };

    append_jsonl_line(&rec)
}

/// Best-effort append of a **non-tx** diagnostic row (e.g. swap-mix planning / failure).
///
/// This is used to debug cases where the bot closes a position but cannot open a new one
/// due to wallet mix / deposit quote constraints. These rows are intentionally lightweight
/// (no `getTransaction` calls).
pub async fn try_append_bot_diagnostic_row(
    provider: &RpcProvider,
    event: &'static str,
    operation: &'static str,
    pool: Option<Pubkey>,
    position: Option<Pubkey>,
    rebalance_session_id_override: Option<String>,
    details: serde_json::Value,
) {
    #[derive(Serialize)]
    struct Row {
        schema_version: u32,
        ts_utc: String,
        event: &'static str,
        source: &'static str,
        operation: &'static str,
        #[serde(skip_serializing_if = "Option::is_none")]
        pool_address: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        position_pubkey: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        rebalance_session_id: Option<String>,
        rpc_url: String,
        /// Structured free-form fields; stable keys are preferred but not enforced.
        details: serde_json::Value,
        accounting_note: &'static str,
    }

    let rpc_url = provider.current_endpoint().await;
    let rec = Row {
        schema_version: 2,
        ts_utc: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        event,
        source: "orca_bot",
        operation,
        pool_address: pool.map(|p| p.to_string()),
        position_pubkey: position.map(|p| p.to_string()),
        rebalance_session_id: rebalance_session_id_override.or_else(rebalance_session_id_from_env),
        rpc_url,
        details,
        accounting_note: "Diagnostic row (no tx); helps debug swap-mix / rebalance incomplete sequences.",
    };

    if let Err(e) = append_jsonl_line(&rec) {
        warn!(error = %e, event, operation, "bot diagnostic ledger: append failed");
    }
}
