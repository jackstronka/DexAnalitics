//! Stream lineage: ordered chain of rotated position PDAs + per-node aggregates.
//!
//! We already persist edges old->new in `position_stream_edges` (best-effort from IL ledger).
//! This service builds a best-effort *linear* chain (root → ... → current) and enriches each node
//! with valuation snapshots + ledger aggregates so the UI can show "history of positions".

use crate::error::ApiError;
use crate::models::{LineageChainCostSummary, PositionStreamLineageNode, PositionStreamLineageResponse};
use clmm_lp_data::repositories::Database;
use crate::services::position_stream_performance::compute_position_stream_performance;
use crate::services::position_stream_pnl::compute_position_stream_pnl;
use crate::services::price_fetch::fetch_mint_prices_usd;
use crate::services::position_valuation::{
    compute_position_usd_valuation, fetch_prices_for_positions, monitored_position_from_chain,
};
use crate::state::AppState;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use rust_decimal::MathematicalOps;
use rust_decimal::prelude::ToPrimitive;
use spl_token::solana_program::program_pack::Pack;
use spl_token::state::Mint;
use sqlx::Row;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::str::FromStr;
use std::fs;
use std::io::{BufRead, BufReader};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::SystemTime;
use tokio::time::{sleep, timeout, Duration};
use futures::future::join_all;

const WSOL_MINT: &str = "So11111111111111111111111111111111111111112";
const WHETH_MINT: &str = "7vfCXTUXx5WJV5JADk17DUJ4ksgau7utNKj4b963voxs";

/// In-process cache: Whirlpool pool address → (token_mint_a, token_mint_b). Pool mints are immutable;
/// caching avoids N duplicate `get_account` RPCs when lineage builds many PDAs in parallel (public RPC
/// often times out under burst — previously produced empty LP-fee legs + misleading `collect_events`).
static POOL_TOKEN_MINTS_CACHE: OnceLock<RwLock<HashMap<String, (String, String)>>> = OnceLock::new();

async fn pool_token_mints_cached(state: &AppState, pool: &str) -> Option<(String, String)> {
    let pool = pool.trim();
    if pool.is_empty() {
        return None;
    }
    let cache = POOL_TOKEN_MINTS_CACHE.get_or_init(|| RwLock::new(HashMap::new()));
    if let Ok(g) = cache.read() {
        if let Some(p) = g.get(pool) {
            return Some(p.clone());
        }
    }
    let reader = clmm_lp_protocols::prelude::WhirlpoolReader::new(state.provider.clone());
    for attempt in 0u32..3 {
        if attempt > 0 {
            sleep(Duration::from_millis(150 * u64::from(attempt))).await;
        }
        let Ok(Ok(ps)) = timeout(Duration::from_secs(4), reader.get_pool_state(pool)).await else {
            continue;
        };
        let pair = (ps.token_mint_a.to_string(), ps.token_mint_b.to_string());
        if let Ok(mut g) = cache.write() {
            g.insert(pool.to_string(), pair.clone());
        }
        return Some(pair);
    }
    None
}

fn token_short_label(mint: &str) -> String {
    match mint.trim() {
        "So11111111111111111111111111111111111111112" => "SOL".to_string(),
        "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v" => "USDC".to_string(),
        "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB" => "USDT".to_string(),
        WHETH_MINT => "whETH".to_string(),
        s => {
            if s.len() > 10 {
                format!("{}…{}", &s[..4], &s[s.len().saturating_sub(4)..])
            } else {
                s.to_string()
            }
        }
    }
}

async fn pool_leg_mints_best_effort(state: &AppState, pool: &str) -> (Option<String>, Option<String>) {
    let Some((a, b)) = pool_token_mints_cached(state, pool).await else {
        return (None, None);
    };
    (Some(a), Some(b))
}

#[derive(Debug, Clone)]
struct LifecycleRow {
    ts_utc: Option<DateTime<Utc>>,
    event: Option<String>,
    pool_address: Option<String>,
    position_pubkey: Option<String>,
    fee_payer_pubkey: Option<String>,
    /// Same id on swap/close/open rows during a bot rebalance (ties parent close → child open without relying on swap rows).
    rebalance_session_id: Option<String>,
    tx_fee_lamports: Option<u64>,
    fee_payer_token_deltas: Option<serde_json::Value>,
    /// Pool-order leg deltas on `bot_collect_fees` (when executor wrote them); complements `fee_payer_token_deltas`
    /// for WSOL/native flows where the mint map may omit the SOL leg.
    fee_payer_token_a_delta_ui: Option<Decimal>,
    fee_payer_token_b_delta_ui: Option<Decimal>,
    /// **Pool token A/B** raw amounts harvested: `fee_owed_{a,b}` on the Whirlpool position read immediately before collect (bot).
    lp_collected_token_a_raw: Option<u64>,
    lp_collected_token_b_raw: Option<u64>,
    details: Option<serde_json::Value>,
}

fn parse_ts(v: &serde_json::Value) -> Option<DateTime<Utc>> {
    let s = v.as_str()?.trim();
    DateTime::parse_from_rfc3339(s).ok().map(|dt| dt.with_timezone(&Utc))
}

/// Bot rows use `bot_open_*` / `bot_close_*`; CLI `orca-position-open/close` uses `position_open` / `position_close`.
#[inline]
fn is_lifecycle_open_event(ev: Option<&str>) -> bool {
    matches!(
        ev,
        Some("bot_open_position") | Some("bot_open_position_full_range") | Some("position_open")
    )
}

#[inline]
fn is_lifecycle_close_event(ev: Option<&str>) -> bool {
    matches!(ev, Some("bot_close_position") | Some("position_close"))
}

struct LifecycleRowsCache {
    mtime: Option<SystemTime>,
    rows: Arc<Vec<LifecycleRow>>,
}

static LIFECYCLE_ROWS_CACHE: OnceLock<RwLock<LifecycleRowsCache>> = OnceLock::new();

fn parse_lifecycle_rows_from_reader<R: BufRead>(reader: R) -> Vec<LifecycleRow> {
    let mut out = Vec::new();
    for line in reader.lines().filter_map(Result::ok) {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(t) else {
            continue;
        };
        let ts_utc = v.get("ts_utc").and_then(parse_ts);
        let event = v
            .get("event")
            .and_then(|x| x.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let pool_address = v
            .get("pool_address")
            .or_else(|| v.get("pool_pubkey"))
            .and_then(|x| x.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let rebalance_session_id = v
            .get("rebalance_session_id")
            .and_then(|x| x.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let position_pubkey = v
            .get("position_pubkey")
            .or_else(|| v.get("position_pda"))
            .and_then(|x| x.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let fee_payer_pubkey = v
            .get("fee_payer_pubkey")
            .and_then(|x| x.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let tx_fee_lamports = v.get("tx_fee_lamports").and_then(|x| x.as_u64());
        let fee_payer_token_deltas = v.get("fee_payer_token_deltas").cloned();
        let fee_payer_token_a_delta_ui = v
            .get("fee_payer_token_a_delta_ui")
            .and_then(dec_from_any);
        let fee_payer_token_b_delta_ui = v
            .get("fee_payer_token_b_delta_ui")
            .and_then(dec_from_any);
        let lp_collected_token_a_raw = v
            .get("lp_collected_token_a_raw")
            .and_then(|x| x.as_u64().or_else(|| x.as_str().and_then(|s| s.parse().ok())));
        let lp_collected_token_b_raw = v
            .get("lp_collected_token_b_raw")
            .and_then(|x| x.as_u64().or_else(|| x.as_str().and_then(|s| s.parse().ok())));
        let details = v.get("details").cloned();
        out.push(LifecycleRow {
            ts_utc,
            event,
            pool_address,
            position_pubkey,
            fee_payer_pubkey,
            rebalance_session_id,
            tx_fee_lamports,
            fee_payer_token_deltas,
            fee_payer_token_a_delta_ui,
            fee_payer_token_b_delta_ui,
            lp_collected_token_a_raw,
            lp_collected_token_b_raw,
            details,
        });
    }
    out.sort_by(|a, b| a.ts_utc.cmp(&b.ts_utc));
    out
}

async fn lifecycle_rows_cached_best_effort() -> Arc<Vec<LifecycleRow>> {
    let path = clmm_lp_protocols::ledger::tx_lifecycle::ledger_read_path();
    let mtime = fs::metadata(&path).ok().and_then(|m| m.modified().ok());

    let lock = LIFECYCLE_ROWS_CACHE.get_or_init(|| {
        RwLock::new(LifecycleRowsCache {
            mtime: None,
            rows: Arc::new(Vec::new()),
        })
    });

    // Fast path: cache hit.
    if let Ok(g) = lock.read() {
        if g.mtime == mtime && !g.rows.is_empty() {
            return g.rows.clone();
        }
    }

    // Slow path: rebuild off-thread to avoid blocking the async runtime.
    let rebuilt: Vec<LifecycleRow> = tokio::task::spawn_blocking(move || {
        let file = fs::File::open(&path).ok();
        match file {
            Some(f) => {
                let reader = BufReader::new(f);
                parse_lifecycle_rows_from_reader(reader)
            }
            None => Vec::new(),
        }
    })
    .await
    .unwrap_or_default();

    let rows = Arc::new(rebuilt);
    if let Ok(mut g) = lock.write() {
        g.mtime = mtime;
        g.rows = rows.clone();
    }
    rows
}

fn chain_from_lifecycle_best_effort_rows(
    rows: &[LifecycleRow],
    entry: &str,
    max_hops: usize,
) -> Vec<String> {

    // Helper: find the latest OPEN row for a position (best-effort).
    fn find_open_row<'a>(
        rows: &'a [LifecycleRow],
        position: &str,
    ) -> Option<(usize, &'a LifecycleRow)> {
        let mut out: Option<(usize, &LifecycleRow)> = None;
        for (i, r) in rows.iter().enumerate() {
            if r.position_pubkey.as_deref() != Some(position) {
                continue;
            }
            if !is_lifecycle_open_event(r.event.as_deref()) {
                continue;
            }
            out = Some((i, r));
        }
        out
    }

    // Helper: find the latest CLOSE row for a position starting at index.
    fn find_close_row_from<'a>(
        rows: &'a [LifecycleRow],
        position: &str,
        start_idx: usize,
    ) -> Option<(usize, &'a LifecycleRow)> {
        for (i, r) in rows.iter().enumerate().skip(start_idx) {
            if r.position_pubkey.as_deref() == Some(position) && is_lifecycle_close_event(r.event.as_deref())
            {
                return Some((i, r));
            }
        }
        None
    }

    // Helper: parent lookup for a given OPEN (pool+payer, within 10 minutes backwards).
    fn find_parent_from_open<'a>(
        rows: &'a [LifecycleRow],
        open_ts: DateTime<Utc>,
        pool: &str,
        payer: &str,
        open_rebalance_session_id: Option<&str>,
    ) -> Option<&'a str> {
        let window_start = open_ts - chrono::Duration::minutes(60);
        let mut best: Option<&LifecycleRow> = None;
        for r in rows.iter() {
            let Some(ts) = r.ts_utc else { continue };
            if ts < window_start || ts > open_ts {
                continue;
            }
            if r.pool_address.as_deref() != Some(pool) {
                continue;
            }
            if r.fee_payer_pubkey.as_deref() != Some(payer) {
                continue;
            }
            if !is_lifecycle_close_event(r.event.as_deref()) {
                continue;
            }
            // Guardrail: don't link a manual "start" open to a random prior close.
            // Only accept a parent when we observe *rotation-like* activity between close→open
            // (typically swap-mix / swap txs the bot does to reopen into the new range).
            let parent_pda = r.position_pubkey.as_deref().unwrap_or("");
            if parent_pda.is_empty() {
                continue;
            }
            let mut has_rotation_signal = false;
            // Strong explicit signal: close row declares it was a rotation (bot close before reopen).
            // This is emitted by the executor and is reliable even when the bot does *no swaps* between close→open.
            if let Some(d) = r.details.as_ref().and_then(|x| x.as_object()) {
                if d.get("close_kind")
                    .and_then(|x| x.as_str())
                    .is_some_and(|s| s.trim() == "rotation")
                {
                    has_rotation_signal = true;
                }
            }
            // Strongest: same rebalance_session_id on close and open (bot stamps both when CLMM_REBALANCE_SESSION_ID is set).
            if let (Some(osid), Some(csid)) = (open_rebalance_session_id, r.rebalance_session_id.as_deref())
            {
                if !osid.is_empty() && osid == csid {
                    has_rotation_signal = true;
                }
            }
            if !has_rotation_signal {
                for rr in rows.iter() {
                    let Some(t2) = rr.ts_utc else { continue };
                    if t2 < ts || t2 > open_ts {
                        continue;
                    }
                    if rr.pool_address.as_deref() != Some(pool) {
                        continue;
                    }
                    if rr.fee_payer_pubkey.as_deref() != Some(payer) {
                        continue;
                    }
                    // swaps / mix diagnostics are strong indicators this was a close→open rotation flow
                    let ev = rr.event.as_deref().unwrap_or("");
                    if ev.starts_with("bot_swap_") || ev.starts_with("bot_swap") {
                        // tie to the closed position when possible
                        if rr.position_pubkey.as_deref() == Some(parent_pda) {
                            has_rotation_signal = true;
                            break;
                        }
                        // or accept pool+payer swaps even if position key isn't repeated on that row
                        if rr.position_pubkey.is_none() {
                            has_rotation_signal = true;
                            break;
                        }
                    }
                    if ev == "bot_reopen_preflight_failed" {
                        if rr.position_pubkey.as_deref() == Some(parent_pda) {
                            has_rotation_signal = true;
                            break;
                        }
                    }
                    // Typical rebalance sequence before close (same position PDA).
                    if matches!(ev, "bot_collect_fees" | "bot_decrease_liquidity")
                        && rr.position_pubkey.as_deref() == Some(parent_pda)
                    {
                        has_rotation_signal = true;
                        break;
                    }
                }
            }
            if !has_rotation_signal {
                continue;
            }
            if best.is_none() || ts > best.and_then(|b| b.ts_utc).unwrap_or(ts) {
                best = Some(r);
            }
        }
        best.and_then(|r| r.position_pubkey.as_deref())
    }

    // Anchor: prefer OPEN row (lets us go backwards); otherwise fall back to CLOSE.
    let mut anchor: Option<(usize, &LifecycleRow)> = find_open_row(&rows, entry);
    if anchor.is_none() {
        // newest close row for entry
        let mut last: Option<(usize, &LifecycleRow)> = None;
        for (i, r) in rows.iter().enumerate() {
            if r.position_pubkey.as_deref() == Some(entry) && is_lifecycle_close_event(r.event.as_deref())
            {
                last = Some((i, r));
            }
        }
        anchor = last;
    }
    let Some((anchor_idx, _anchor_row)) = anchor else {
        return vec![entry.to_string()];
    };

    let mut seen: HashSet<String> = HashSet::new();
    seen.insert(entry.to_string());

    // Build backward chain (oldest ... entry) by following OPEN -> parent CLOSE.
    let mut backward: Vec<String> = Vec::new();
    let mut cur_pos = entry.to_string();
    for _ in 0..max_hops {
        let Some((_oi, o)) = find_open_row(&rows, &cur_pos) else { break };
        let (Some(open_ts), Some(pool), Some(payer)) = (
            o.ts_utc,
            o.pool_address.as_deref(),
            o.fee_payer_pubkey.as_deref(),
        ) else {
            break;
        };
        let Some(parent) = find_parent_from_open(
            &rows,
            open_ts,
            pool,
            payer,
            o.rebalance_session_id.as_deref(),
        ) else {
            break;
        };
        if !seen.insert(parent.to_string()) {
            break;
        }
        backward.push(parent.to_string());
        cur_pos = parent.to_string();
    }
    backward.reverse();

    // Start chain with backward + entry.
    let mut chain: Vec<String> = backward;
    if !chain.iter().any(|p| p == entry) {
        chain.push(entry.to_string());
    }

    // Forward chain (entry -> ... newest) by following CLOSE -> next OPEN.
    let mut cur_idx = anchor_idx;
    for _ in 0..max_hops {
        let cur = chain.last().expect("non-empty").clone();
        let Some((close_i, c)) = find_close_row_from(&rows, &cur, cur_idx) else { break };
        let (Some(close_ts), Some(pool), Some(payer)) = (
            c.ts_utc,
            c.pool_address.as_deref(),
            c.fee_payer_pubkey.as_deref(),
        ) else {
            break;
        };
        let window_end = close_ts + chrono::Duration::minutes(60);
        let mut next_open: Option<(usize, &LifecycleRow)> = None;
        for (i, r) in rows.iter().enumerate().skip(close_i) {
            let Some(ts) = r.ts_utc else { continue };
            if ts < close_ts {
                continue;
            }
            if ts > window_end {
                break;
            }
            if r.pool_address.as_deref() != Some(pool) {
                continue;
            }
            if r.fee_payer_pubkey.as_deref() != Some(payer) {
                continue;
            }
            if !is_lifecycle_open_event(r.event.as_deref()) {
                continue;
            }
            next_open = Some((i, r));
            break;
        }
        let Some((open_i, o)) = next_open else { break };
        let Some(next_pda) = o.position_pubkey.as_deref() else { break };
        if !seen.insert(next_pda.to_string()) {
            break;
        }
        chain.push(next_pda.to_string());
        cur_idx = open_i;
    }

    // Keep entry reachable even if weird ordering happened.
    if !chain.iter().any(|p| p == entry) {
        vec![entry.to_string()]
    } else {
        chain
    }
}

fn dec_from_any(v: &serde_json::Value) -> Option<Decimal> {
    use serde_json::Value;
    match v {
        Value::String(s) => Decimal::from_str(s.trim()).ok(),
        Value::Number(n) => n.as_f64().and_then(Decimal::from_f64_retain),
        _ => None,
    }
}

#[derive(Debug, Clone)]
struct RegistryRow {
    ts_utc: Option<DateTime<Utc>>,
    event: Option<String>,
    pool_address: Option<String>,
    position_pubkey: Option<String>,
    owner_pubkey: Option<String>,
    rebalance_session_id: Option<String>,
}

fn parse_registry_rows_from_reader<R: BufRead>(reader: R) -> Vec<RegistryRow> {
    let mut out = Vec::new();
    for line in reader.lines().filter_map(Result::ok) {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(t) else {
            continue;
        };
        let ts_utc = v.get("ts_utc").and_then(parse_ts);
        let event = v
            .get("event")
            .and_then(|x| x.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let pool_address = v
            .get("pool_address")
            .and_then(|x| x.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let position_pubkey = v
            .get("position_pubkey")
            .and_then(|x| x.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let owner_pubkey = v
            .get("owner_pubkey")
            .and_then(|x| x.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let rebalance_session_id = v
            .get("rebalance_session_id")
            .and_then(|x| x.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        out.push(RegistryRow {
            ts_utc,
            event,
            pool_address,
            position_pubkey,
            owner_pubkey,
            rebalance_session_id,
        });
    }
    out.sort_by(|a, b| a.ts_utc.cmp(&b.ts_utc));
    out
}

fn registry_rows_best_effort() -> Vec<RegistryRow> {
    let path = clmm_lp_protocols::ledger::position_registry::registry_path();
    let Ok(file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    parse_registry_rows_from_reader(BufReader::new(file))
}

/// Immediate parent PDA: `registry_close` for the position closed just before this PDA's `registry_open`
/// (same pool + owner; prefers matching `rebalance_session_id`; 60m window).
pub fn infer_parent_position_from_registry_best_effort(entry: &str) -> Option<String> {
    let rows = registry_rows_best_effort();
    infer_parent_position_from_registry_rows(&rows, entry)
}

fn infer_parent_position_from_registry_rows(rows: &[RegistryRow], entry: &str) -> Option<String> {
    let entry = entry.trim();
    if entry.is_empty() {
        return None;
    }
    let mut open_row: Option<&RegistryRow> = None;
    for r in rows.iter() {
        if r.position_pubkey.as_deref() != Some(entry) {
            continue;
        }
        if r.event.as_deref() != Some("registry_open") {
            continue;
        }
        open_row = Some(r);
    }
    let o = open_row?;
    let open_ts = o.ts_utc?;
    let pool = o.pool_address.as_deref()?.trim();
    let owner = o.owner_pubkey.as_deref()?.trim();
    if pool.is_empty() || owner.is_empty() {
        return None;
    }

    let window_start = open_ts - chrono::Duration::minutes(60);
    let mut best: Option<(&RegistryRow, DateTime<Utc>)> = None;
    for r in rows.iter() {
        if r.event.as_deref() != Some("registry_close") {
            continue;
        }
        let Some(ts) = r.ts_utc else { continue };
        if ts < window_start || ts > open_ts {
            continue;
        }
        if r.pool_address.as_deref().map(str::trim) != Some(pool) {
            continue;
        }
        if r.owner_pubkey.as_deref().map(str::trim) != Some(owner) {
            continue;
        }
        if let (Some(osid), Some(csid)) =
            (o.rebalance_session_id.as_deref(), r.rebalance_session_id.as_deref())
        {
            if !osid.is_empty() && osid == csid {
                best = Some((r, ts));
                break;
            }
        }
        if best.as_ref().is_none_or(|(_, bts)| ts > *bts) {
            best = Some((r, ts));
        }
    }
    let (c, _) = best?;
    let parent = c.position_pubkey.as_deref()?.trim();
    if parent.is_empty() || parent == entry {
        return None;
    }
    Some(parent.to_string())
}

/// Parent PDA after a rotation: prefer append-only registry, then lifecycle JSONL.
pub async fn infer_rotation_parent_best_effort(entry: &str) -> Option<String> {
    if let Some(p) = infer_parent_position_from_registry_best_effort(entry) {
        return Some(p);
    }
    infer_parent_position_from_lifecycle_best_effort(entry).await
}

fn chain_from_registry_best_effort_rows(rows: &[RegistryRow], entry: &str, max_hops: usize) -> Vec<String> {
    fn find_open_row<'a>(rows: &'a [RegistryRow], position: &str) -> Option<&'a RegistryRow> {
        let mut out: Option<&RegistryRow> = None;
        for r in rows.iter() {
            if r.position_pubkey.as_deref() != Some(position) {
                continue;
            }
            if r.event.as_deref() != Some("registry_open") {
                continue;
            }
            out = Some(r);
        }
        out
    }
    fn find_close_row<'a>(rows: &'a [RegistryRow], position: &str) -> Option<&'a RegistryRow> {
        let mut out: Option<&RegistryRow> = None;
        for r in rows.iter() {
            if r.position_pubkey.as_deref() != Some(position) {
                continue;
            }
            if r.event.as_deref() != Some("registry_close") {
                continue;
            }
            out = Some(r);
        }
        out
    }

    let entry = entry.trim();
    if entry.is_empty() {
        return Vec::new();
    }

    let anchor_open = find_open_row(rows, entry);
    let anchor_close = find_close_row(rows, entry);
    let anchor_ts = anchor_open
        .and_then(|r| r.ts_utc)
        .or_else(|| anchor_close.and_then(|r| r.ts_utc));
    let pool = anchor_open
        .and_then(|r| r.pool_address.as_deref())
        .or_else(|| anchor_close.and_then(|r| r.pool_address.as_deref()));
    let owner = anchor_open
        .and_then(|r| r.owner_pubkey.as_deref())
        .or_else(|| anchor_close.and_then(|r| r.owner_pubkey.as_deref()));
    if anchor_ts.is_none() || pool.is_none() || owner.is_none() {
        return vec![entry.to_string()];
    }
    let pool = pool.unwrap();
    let owner = owner.unwrap();

    let mut seen: HashSet<String> = HashSet::new();
    seen.insert(entry.to_string());

    let mut backward: Vec<String> = Vec::new();
    let mut cur = entry.to_string();
    for _ in 0..max_hops {
        let Some(o) = find_open_row(rows, &cur) else { break };
        let Some(open_ts) = o.ts_utc else { break };
        let window_start = open_ts - chrono::Duration::minutes(60);
        let mut best: Option<(&RegistryRow, DateTime<Utc>)> = None;
        for r in rows.iter() {
            if r.event.as_deref() != Some("registry_close") {
                continue;
            }
            let Some(ts) = r.ts_utc else { continue };
            if ts < window_start || ts > open_ts {
                continue;
            }
            if r.pool_address.as_deref() != Some(pool) {
                continue;
            }
            if r.owner_pubkey.as_deref() != Some(owner) {
                continue;
            }
            if let (Some(osid), Some(csid)) =
                (o.rebalance_session_id.as_deref(), r.rebalance_session_id.as_deref())
            {
                if !osid.is_empty() && osid == csid {
                    best = Some((r, ts));
                    break;
                }
            }
            if best.as_ref().is_none_or(|(_, bts)| ts > *bts) {
                best = Some((r, ts));
            }
        }
        let Some((c, _)) = best else { break };
        let Some(parent) = c.position_pubkey.as_deref() else { break };
        if !seen.insert(parent.to_string()) {
            break;
        }
        backward.push(parent.to_string());
        cur = parent.to_string();
    }
    backward.reverse();

    let mut chain = backward;
    if !chain.iter().any(|p| p == entry) {
        chain.push(entry.to_string());
    }

    let mut cur = entry.to_string();
    for _ in 0..max_hops {
        let Some(c) = find_close_row(rows, &cur) else { break };
        let Some(close_ts) = c.ts_utc else { break };
        let window_end = close_ts + chrono::Duration::minutes(60);
        let mut next: Option<(&RegistryRow, DateTime<Utc>)> = None;
        for r in rows.iter() {
            if r.event.as_deref() != Some("registry_open") {
                continue;
            }
            let Some(ts) = r.ts_utc else { continue };
            if ts < close_ts || ts > window_end {
                continue;
            }
            if r.pool_address.as_deref() != Some(pool) {
                continue;
            }
            if r.owner_pubkey.as_deref() != Some(owner) {
                continue;
            }
            if let (Some(csid), Some(osid)) =
                (c.rebalance_session_id.as_deref(), r.rebalance_session_id.as_deref())
            {
                if !csid.is_empty() && csid == osid {
                    next = Some((r, ts));
                    break;
                }
            }
            if next.as_ref().is_none_or(|(_, nts)| ts < *nts) {
                next = Some((r, ts));
            }
        }
        let Some((o, _)) = next else { break };
        let Some(next_pda) = o.position_pubkey.as_deref() else { break };
        if !seen.insert(next_pda.to_string()) {
            break;
        }
        chain.push(next_pda.to_string());
        cur = next_pda.to_string();
    }

    if !chain.iter().any(|p| p == entry) {
        vec![entry.to_string()]
    } else {
        chain
    }
}

#[allow(clippy::too_many_lines)]
async fn lp_fees_collected_usd_from_lifecycle_rows(
    state: &AppState,
    rows: &[LifecycleRow],
    position_pubkey: &str,
) -> (u32, Decimal, BTreeMap<String, Decimal>) {
    let mut pool_mints: HashMap<String, (String, String)> = HashMap::new();
    let mut mint_decimals: HashMap<String, u8> = HashMap::new();
    let mut events: u32 = 0;
    let mut by_mint_ui: BTreeMap<String, Decimal> = BTreeMap::new();

    for r in rows {
        if r.position_pubkey.as_deref() != Some(position_pubkey) {
            continue;
        }
        if r.event.as_deref() != Some("bot_collect_fees") {
            continue;
        }
        let Some(pool) = r.pool_address.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
            continue;
        };
        let (ma, mb) = match pool_mints.get(pool) {
            Some(p) => (p.0.clone(), p.1.clone()),
            None => {
                let Some(pair) = pool_token_mints_cached(state, pool).await else {
                    continue;
                };
                pool_mints.insert(pool.to_string(), pair.clone());
                pair
            }
        };
        events += 1;
        let obj = r
            .fee_payer_token_deltas
            .as_ref()
            .and_then(|v| v.as_object());

        let map_a = obj
            .and_then(|o| o.get(&ma))
            .and_then(dec_from_any)
            .filter(|d| *d > Decimal::ZERO)
            .unwrap_or(Decimal::ZERO);
        let map_b = obj
            .and_then(|o| o.get(&mb))
            .and_then(dec_from_any)
            .filter(|d| *d > Decimal::ZERO)
            .unwrap_or(Decimal::ZERO);

        let col_a = r
            .fee_payer_token_a_delta_ui
            .filter(|d| *d > Decimal::ZERO)
            .unwrap_or(Decimal::ZERO);
        let col_b = r
            .fee_payer_token_b_delta_ui
            .filter(|d| *d > Decimal::ZERO)
            .unwrap_or(Decimal::ZERO);

        let mut merged_a = map_a.max(col_a);
        let mut merged_b = map_b.max(col_b);

        // Authoritative both legs: position `fee_owed_a/b` read by bot immediately before harvest.
        if let Some(raw) = r.lp_collected_token_a_raw {
            if let Ok(pk) = solana_sdk::pubkey::Pubkey::from_str(ma.as_str()) {
                let dec = if let Some(d) = mint_decimals.get(&ma).copied() {
                    d
                } else if let Some(d) =
                    fetch_mint_decimals_best_effort(state.provider.as_ref(), &pk).await
                {
                    mint_decimals.insert(ma.clone(), d);
                    d
                } else {
                    9u8
                };
                let ui = decimal_ui_from_raw_u64(raw, dec);
                merged_a = merged_a.max(ui);
            }
        }
        if let Some(raw) = r.lp_collected_token_b_raw {
            if let Ok(pk) = solana_sdk::pubkey::Pubkey::from_str(mb.as_str()) {
                let dec = if let Some(d) = mint_decimals.get(&mb).copied() {
                    d
                } else if let Some(d) =
                    fetch_mint_decimals_best_effort(state.provider.as_ref(), &pk).await
                {
                    mint_decimals.insert(mb.clone(), d);
                    d
                } else {
                    9u8
                };
                let ui = decimal_ui_from_raw_u64(raw, dec);
                merged_b = merged_b.max(ui);
            }
        }

        if merged_a > Decimal::ZERO {
            *by_mint_ui.entry(ma).or_insert(Decimal::ZERO) += merged_a;
        }
        if merged_b > Decimal::ZERO {
            *by_mint_ui.entry(mb).or_insert(Decimal::ZERO) += merged_b;
        }
    }

    if by_mint_ui.is_empty() {
        return (events, Decimal::ZERO, BTreeMap::new());
    }

    let mints: BTreeSet<String> = by_mint_ui.keys().cloned().collect();
    let (px, _) = match timeout(Duration::from_secs(5), fetch_mint_prices_usd(&mints)).await {
        Ok(r) => r,
        Err(_) => return (events, Decimal::ZERO, by_mint_ui),
    };
    let mut usd = Decimal::ZERO;
    for (m, amt) in &by_mint_ui {
        let p = px.get(m).copied().unwrap_or(0.0);
        if p > 0.0 && p.is_finite() {
            let pd = Decimal::from_f64_retain(p).unwrap_or(Decimal::ZERO);
            usd += *amt * pd;
        }
    }
    (events, usd, by_mint_ui)
}

async fn lp_fees_collected_usd_from_ledger_db(
    state: &AppState,
    db: &Database,
    position_pubkey: &str,
) -> Result<(u32, Decimal, BTreeMap<String, Decimal>), ApiError> {
    let rows = sqlx::query(
        r#"
        SELECT fee_payer_token_deltas, pool_pubkey,
               fee_payer_token_a_delta_ui, fee_payer_token_b_delta_ui,
               lp_collected_token_a_raw, lp_collected_token_b_raw
        FROM position_stream_ledger_rows
        WHERE position_pubkey = $1 AND event = 'bot_collect_fees'
        "#,
    )
    .bind(position_pubkey)
    .fetch_all(db.pool())
    .await
    .map_err(|e| ApiError::internal(format!("stream lineage: collect fee rows: {e}")))?;

    let mut pool_mints: HashMap<String, (String, String)> = HashMap::new();
    let mut mint_decimals: HashMap<String, u8> = HashMap::new();
    let mut by_mint_ui: BTreeMap<String, Decimal> = BTreeMap::new();
    let mut events: u32 = 0;

    for r in rows {
        let v: Option<serde_json::Value> = r.try_get("fee_payer_token_deltas").ok().flatten();
        let pool: Option<String> = r.try_get("pool_pubkey").ok();
        let lp_raw_a: Option<i64> = r.try_get("lp_collected_token_a_raw").ok().flatten();
        let lp_raw_b: Option<i64> = r.try_get("lp_collected_token_b_raw").ok().flatten();
        let col_a: Option<Decimal> = r
            .try_get::<Option<Decimal>, _>("fee_payer_token_a_delta_ui")
            .ok()
            .flatten()
            .filter(|d| *d > Decimal::ZERO);
        let col_b: Option<Decimal> = r
            .try_get::<Option<Decimal>, _>("fee_payer_token_b_delta_ui")
            .ok()
            .flatten()
            .filter(|d| *d > Decimal::ZERO);
        let Some(pool) = pool.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
            continue;
        };
        let (ma, mb) = match pool_mints.get(pool) {
            Some(p) => (p.0.clone(), p.1.clone()),
            None => {
                let Some(pair) = pool_token_mints_cached(state, pool).await else {
                    continue;
                };
                pool_mints.insert(pool.to_string(), pair.clone());
                pair
            }
        };
        events += 1;
        let obj = v.as_ref().and_then(|x| x.as_object());
        let map_a = obj
            .and_then(|o| o.get(&ma))
            .and_then(dec_from_any)
            .filter(|d| *d > Decimal::ZERO)
            .unwrap_or(Decimal::ZERO);
        let map_b = obj
            .and_then(|o| o.get(&mb))
            .and_then(dec_from_any)
            .filter(|d| *d > Decimal::ZERO)
            .unwrap_or(Decimal::ZERO);

        let col_a = col_a.unwrap_or(Decimal::ZERO);
        let col_b = col_b.unwrap_or(Decimal::ZERO);
        let mut merged_a = map_a.max(col_a);
        let mut merged_b = map_b.max(col_b);

        if let Some(raw) = lp_raw_a.filter(|x| *x > 0).map(|x| x as u64) {
            if let Ok(pk) = solana_sdk::pubkey::Pubkey::from_str(ma.as_str()) {
                let dec = if let Some(d) = mint_decimals.get(&ma).copied() {
                    d
                } else if let Some(d) =
                    fetch_mint_decimals_best_effort(state.provider.as_ref(), &pk).await
                {
                    mint_decimals.insert(ma.clone(), d);
                    d
                } else {
                    9u8
                };
                merged_a = merged_a.max(decimal_ui_from_raw_u64(raw, dec));
            }
        }
        if let Some(raw) = lp_raw_b.filter(|x| *x > 0).map(|x| x as u64) {
            if let Ok(pk) = solana_sdk::pubkey::Pubkey::from_str(mb.as_str()) {
                let dec = if let Some(d) = mint_decimals.get(&mb).copied() {
                    d
                } else if let Some(d) =
                    fetch_mint_decimals_best_effort(state.provider.as_ref(), &pk).await
                {
                    mint_decimals.insert(mb.clone(), d);
                    d
                } else {
                    9u8
                };
                merged_b = merged_b.max(decimal_ui_from_raw_u64(raw, dec));
            }
        }

        if merged_a > Decimal::ZERO {
            *by_mint_ui.entry(ma).or_insert(Decimal::ZERO) += merged_a;
        }
        if merged_b > Decimal::ZERO {
            *by_mint_ui.entry(mb).or_insert(Decimal::ZERO) += merged_b;
        }
    }

    if by_mint_ui.is_empty() {
        return Ok((events, Decimal::ZERO, BTreeMap::new()));
    }

    let mints: BTreeSet<String> = by_mint_ui.keys().cloned().collect();
    let (px, _) = match timeout(Duration::from_secs(5), fetch_mint_prices_usd(&mints)).await {
        Ok(r) => r,
        Err(_) => return Ok((events, Decimal::ZERO, by_mint_ui)),
    };
    let mut usd = Decimal::ZERO;
    for (m, amt) in &by_mint_ui {
        let p = px.get(m).copied().unwrap_or(0.0);
        if p > 0.0 && p.is_finite() {
            let pd = Decimal::from_f64_retain(p).unwrap_or(Decimal::ZERO);
            usd += *amt * pd;
        }
    }
    Ok((events, usd, by_mint_ui))
}

fn rollup_lineage_chain_costs(nodes: &[PositionStreamLineageNode]) -> Option<LineageChainCostSummary> {
    if nodes.is_empty() {
        return None;
    }
    let mut any_a = false;
    let mut any_b = false;
    let mut a_ui = Decimal::ZERO;
    let mut b_ui = Decimal::ZERO;
    let mut a_raw: u64 = 0;
    let mut b_raw: u64 = 0;
    for n in nodes {
        if let Some(v) = n.fees_collected_token_a_ui {
            any_a = true;
            a_ui += v;
        }
        if let Some(v) = n.fees_collected_token_b_ui {
            any_b = true;
            b_ui += v;
        }
        if let Some(v) = n.fees_collected_token_a_raw {
            a_raw = a_raw.saturating_add(v);
        }
        if let Some(v) = n.fees_collected_token_b_raw {
            b_raw = b_raw.saturating_add(v);
        }
    }
    Some(LineageChainCostSummary {
        tx_fee_lamports_total: nodes.iter().map(|n| n.tx_fee_lamports).sum(),
        tx_fees_usd_total: nodes.iter().map(|n| n.tx_fees_usd).sum(),
        fees_collected_usd_total: nodes.iter().map(|n| n.fees_collected_usd).sum(),
        fees_collected_token_a_ui_total: any_a.then_some(a_ui),
        fees_collected_token_b_ui_total: any_b.then_some(b_ui),
        fees_collected_token_a_raw_total: any_a.then_some(a_raw),
        fees_collected_token_b_raw_total: any_b.then_some(b_raw),
        collect_events_total: nodes.iter().map(|n| n.collect_events).sum(),
    })
}

fn build_linear_chain(
    positions: &[String],
    edges: &[(Option<DateTime<Utc>>, String, String, String)],
    entry: &str,
) -> Vec<String> {
    let pos_set: HashSet<&str> = positions.iter().map(|s| s.as_str()).collect();

    // Adjacency old -> list of (ts, new).
    let mut out: HashMap<&str, Vec<(Option<DateTime<Utc>>, &str)>> = HashMap::new();
    let mut indeg: HashMap<&str, usize> = HashMap::new();
    for p in pos_set.iter() {
        indeg.insert(*p, 0);
    }
    for (ts, old, newp, _sid) in edges {
        if !pos_set.contains(old.as_str()) || !pos_set.contains(newp.as_str()) {
            continue;
        }
        out.entry(old.as_str())
            .or_default()
            .push((*ts, newp.as_str()));
        *indeg.entry(newp.as_str()).or_insert(0) += 1;
    }

    for v in out.values_mut() {
        v.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)));
    }

    // Pick a root: in-degree 0 is ideal. If none (cycle/noise), fall back to entry.
    let mut roots: Vec<&str> = indeg
        .iter()
        .filter_map(|(p, d)| if *d == 0 { Some(*p) } else { None })
        .collect();
    roots.sort();
    let root = if roots.is_empty() {
        entry
    } else if roots.len() == 1 {
        roots[0]
    } else {
        // Prefer the root with earliest outgoing edge timestamp (if any).
        let mut best = roots[0];
        let mut best_ts = None;
        for r in &roots {
            let ts = out
                .get(r)
                .and_then(|v| v.first())
                .and_then(|x| x.0);
            if best_ts.is_none() || (ts.is_some() && ts < best_ts) {
                best = *r;
                best_ts = ts;
            }
        }
        best
    };

    // Walk forward following the earliest edge each time; stop on missing / loop.
    let mut chain: Vec<String> = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut cur = root;
    while pos_set.contains(cur) && seen.insert(cur) {
        chain.push(cur.to_string());
        let Some(nexts) = out.get(cur) else { break };
        let Some((_, next)) = nexts.iter().find(|(_, n)| !seen.contains(*n)) else {
            break;
        };
        cur = *next;
    }

    // Ensure entry is included: if traversal missed it, fall back to trivial chain.
    if !chain.iter().any(|p| p == entry) {
        vec![entry.to_string()]
    } else {
        chain
    }
}

async fn sol_usd() -> (f64, String) {
    let mut mints: BTreeSet<String> = BTreeSet::new();
    mints.insert(WSOL_MINT.to_string());
    let (px, src) = match timeout(Duration::from_secs(2), fetch_mint_prices_usd(&mints)).await {
        Ok(r) => r,
        Err(_) => (BTreeMap::new(), "timeout".to_string()),
    };
    (px.get(WSOL_MINT).copied().unwrap_or(0.0), src)
}

fn ui_amount(raw: u64, decimals: u8) -> f64 {
    if decimals == 0 {
        return raw as f64;
    }
    let denom = 10f64.powi(i32::from(decimals));
    (raw as f64) / denom
}

fn decimal_ui_to_raw_u64(v: Decimal, decimals: u8) -> Option<u64> {
    if v <= Decimal::ZERO {
        return Some(0);
    }
    let scale = Decimal::from(10u64).checked_powu(u64::from(decimals))?;
    (v * scale).round().to_u64()
}

/// Convert SPL raw amount (smallest units) to decimal UI using mint decimals.
fn decimal_ui_from_raw_u64(raw: u64, decimals: u8) -> Decimal {
    if raw == 0 {
        return Decimal::ZERO;
    }
    let mut d = Decimal::from(raw);
    for _ in 0..decimals.min(18) {
        d /= Decimal::from(10u32);
    }
    d
}

async fn fetch_mint_decimals_best_effort(
    provider: &clmm_lp_protocols::rpc::RpcProvider,
    mint: &solana_sdk::pubkey::Pubkey,
) -> Option<u8> {
    let account = provider.get_account(mint).await.ok()?;
    let mint_state = Mint::unpack(&account.data).ok()?;
    Some(mint_state.decimals)
}

async fn node_metrics(
    state: &AppState,
    position_pubkey: &str,
    // When true (stream-lineage): no DB snapshots → lifecycle JSONL, not on-chain self-seed RPC.
    skip_snapshot_self_seed: bool,
) -> Result<PositionStreamLineageNode, ApiError> {
    let Some(db) = state.db.as_ref() else {
        let rows = lifecycle_rows_cached_best_effort().await;
        return node_metrics_from_lifecycle_best_effort(state, &rows, position_pubkey).await;
    };

    // Baseline/current valuation per PDA from persisted snapshots.
    let baseline = sqlx::query(
        r#"
        SELECT ts_utc, value_usd, token_mint_a, token_mint_b
        FROM position_stream_valuation_snapshots
        WHERE position_pubkey = $1
        ORDER BY ts_utc ASC
        LIMIT 1
        "#,
    )
    .bind(position_pubkey)
    .fetch_optional(db.pool())
    .await
    .map_err(|e| ApiError::internal(format!("stream lineage: baseline snapshot query: {e}")))?;

    let current = sqlx::query(
        r#"
        SELECT ts_utc, value_usd
        FROM position_stream_valuation_snapshots
        WHERE position_pubkey = $1
        ORDER BY ts_utc DESC
        LIMIT 1
        "#,
    )
    .bind(position_pubkey)
    .fetch_optional(db.pool())
    .await
    .map_err(|e| ApiError::internal(format!("stream lineage: current snapshot query: {e}")))?;

    // If DB is enabled but we have no snapshots yet:
    // - try a quick self-seed (active positions only; closed may not exist on-chain),
    // - otherwise fall back to lifecycle JSONL.
    let mut baseline = baseline;
    let mut current = current;
    if baseline.is_none() && current.is_none() && skip_snapshot_self_seed {
        let rows = lifecycle_rows_cached_best_effort().await;
        return node_metrics_from_lifecycle_best_effort(state, &rows, position_pubkey).await;
    }
    if baseline.is_none() && current.is_none() {
        let pk = solana_sdk::pubkey::Pubkey::from_str(position_pubkey).ok();
        if let Some(pk) = pk {
            if let Ok(Ok(pos)) = timeout(
                Duration::from_secs(2),
                monitored_position_from_chain(state.provider.clone(), &pk),
            )
            .await
            {
                let prices =
                    fetch_prices_for_positions(state.provider.clone(), std::slice::from_ref(&pos)).await;
                if let Ok(v) = compute_position_usd_valuation(state.provider.clone(), &pos, &prices).await
                {
                    let raw = serde_json::json!({
                        "position": pos.address.to_string(),
                        "pool": pos.pool.to_string(),
                        "value_usd": v.value_usd,
                        "fees_usd": v.fees_usd,
                        "amount_a_ui": v.amount_a_ui,
                        "amount_b_ui": v.amount_b_ui,
                        "token_mint_a": v.token_mint_a.to_string(),
                        "token_mint_b": v.token_mint_b.to_string(),
                        "price_a_usd": v.price_a_usd,
                        "price_b_usd": v.price_b_usd,
                        "source": "stream_lineage_self_seed"
                    });
                    let _ = sqlx::query(
                        r#"
                        INSERT INTO position_stream_valuation_snapshots
                          (position_pubkey, ts_utc, pool_pubkey, value_usd, amount_a_ui, amount_b_ui, fees_usd, token_mint_a, token_mint_b, price_a_usd, price_b_usd, price_source, raw_json)
                        VALUES ($1, NOW(), $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
                        ON CONFLICT (position_pubkey, ts_utc) DO NOTHING
                        "#,
                    )
                    .bind(pos.address.to_string())
                    .bind(pos.pool.to_string())
                    .bind(v.value_usd)
                    .bind(v.amount_a_ui)
                    .bind(v.amount_b_ui)
                    .bind(v.fees_usd)
                    .bind(v.token_mint_a.to_string())
                    .bind(v.token_mint_b.to_string())
                    .bind(Decimal::from_f64_retain(v.price_a_usd).unwrap_or(Decimal::ZERO))
                    .bind(Decimal::from_f64_retain(v.price_b_usd).unwrap_or(Decimal::ZERO))
                    .bind("free_prices")
                    .bind(raw)
                    .execute(db.pool())
                    .await;
                }
            }
        }

        baseline = sqlx::query(
            r#"
            SELECT ts_utc, value_usd, token_mint_a, token_mint_b
            FROM position_stream_valuation_snapshots
            WHERE position_pubkey = $1
            ORDER BY ts_utc ASC
            LIMIT 1
            "#,
        )
        .bind(position_pubkey)
        .fetch_optional(db.pool())
        .await
        .map_err(|e| ApiError::internal(format!("stream lineage: baseline snapshot query (after seed): {e}")))?;

        current = sqlx::query(
            r#"
            SELECT ts_utc, value_usd
            FROM position_stream_valuation_snapshots
            WHERE position_pubkey = $1
            ORDER BY ts_utc DESC
            LIMIT 1
            "#,
        )
        .bind(position_pubkey)
        .fetch_optional(db.pool())
        .await
        .map_err(|e| ApiError::internal(format!("stream lineage: current snapshot query (after seed): {e}")))?;

        if baseline.is_none() && current.is_none() {
            let rows = lifecycle_rows_cached_best_effort().await;
            return node_metrics_from_lifecycle_best_effort(state, &rows, position_pubkey).await;
        }
    }

    let opened_ts: Option<DateTime<Utc>> = baseline
        .as_ref()
        .and_then(|r| r.try_get::<Option<DateTime<Utc>>, _>("ts_utc").ok())
        .flatten();
    let closed_ts: Option<DateTime<Utc>> = current
        .as_ref()
        .and_then(|r| r.try_get::<Option<DateTime<Utc>>, _>("ts_utc").ok())
        .flatten();

    let mut baseline_value: Decimal = baseline
        .as_ref()
        .and_then(|r| r.try_get::<Decimal, _>("value_usd").ok())
        .unwrap_or(Decimal::ZERO);
    let current_value: Decimal = current
        .as_ref()
        .and_then(|r| r.try_get::<Decimal, _>("value_usd").ok())
        .unwrap_or(Decimal::ZERO);

    // Network fees for this PDA.
    let fee_row = sqlx::query(
        r#"
        SELECT COALESCE(SUM(tx_fee_lamports), 0) AS fee_lamports
        FROM position_stream_ledger_rows
        WHERE position_pubkey = $1
        "#,
    )
    .bind(position_pubkey)
    .fetch_one(db.pool())
    .await
    .map_err(|e| ApiError::internal(format!("stream lineage: tx fee sum: {e}")))?;
    let fee_lamports: i64 = fee_row.try_get("fee_lamports").unwrap_or(0);
    let fee_lamports_u = fee_lamports.max(0) as u64;

    let (sol_usd, sol_src) = sol_usd().await;
    let tx_fees_usd = if sol_usd > 0.0 {
        Decimal::from_f64_retain((fee_lamports_u as f64 / 1e9) * sol_usd).unwrap_or(Decimal::ZERO)
    } else {
        Decimal::ZERO
    };

    // Realized cashflow for this PDA: sum fee_payer_token_deltas (pool legs) × current mint USD prices.
    let mut mint_deltas: BTreeMap<String, Decimal> = BTreeMap::new();
    let rows = sqlx::query(
        r#"
        SELECT fee_payer_token_deltas
        FROM position_stream_ledger_rows
        WHERE position_pubkey = $1 AND fee_payer_token_deltas IS NOT NULL
        "#,
    )
    .bind(position_pubkey)
    .fetch_all(db.pool())
    .await
    .map_err(|e| ApiError::internal(format!("stream lineage: token deltas query: {e}")))?;

    for r in rows {
        let v: Option<serde_json::Value> = r.try_get("fee_payer_token_deltas").ok();
        let Some(serde_json::Value::Object(map)) = v else { continue };
        for (mint, dv) in map {
            if let Some(d) = dec_from_any(&dv) {
                *mint_deltas.entry(mint).or_insert(Decimal::ZERO) += d;
            }
        }
    }

    let mut mint_a: Option<String> = baseline
        .as_ref()
        .and_then(|r| r.try_get::<Option<String>, _>("token_mint_a").ok())
        .flatten()
        .filter(|s| !s.trim().is_empty());
    let mut mint_b: Option<String> = baseline
        .as_ref()
        .and_then(|r| r.try_get::<Option<String>, _>("token_mint_b").ok())
        .flatten()
        .filter(|s| !s.trim().is_empty());
    let mut baseline_note: Option<String> = None;

    // DB path guardrail: baseline snapshots derived from open deltas may miss one leg (WSOL),
    // which can massively understate "start value". Correct from open `amount_*_cap` when available.
    if baseline_value.is_zero() || (current_value > Decimal::ZERO && baseline_value < current_value * Decimal::new(60, 2)) {
        let open_row = sqlx::query(
            r#"
            SELECT raw_json
            FROM position_stream_ledger_rows
            WHERE position_pubkey = $1
              AND event IN ('bot_open_position', 'bot_open_position_full_range', 'position_open')
            ORDER BY ts_utc ASC NULLS LAST
            LIMIT 1
            "#,
        )
        .bind(position_pubkey)
        .fetch_optional(db.pool())
        .await
        .map_err(|e| ApiError::internal(format!("stream lineage: open row query: {e}")))?;

        let parse_u64_any = |v: &serde_json::Value| {
            v.as_u64()
                .or_else(|| v.as_i64().and_then(|x| (x > 0).then_some(x as u64)))
                .or_else(|| v.as_str().and_then(|s| s.parse::<u64>().ok()))
        };

        if let Some(r) = open_row {
            let raw: Option<serde_json::Value> = r.try_get("raw_json").ok();
            let details = raw
                .as_ref()
                .and_then(|v| v.get("details"))
                .and_then(|v| v.as_object());

            if mint_a.is_none() {
                mint_a = details
                    .and_then(|d| d.get("token_mint_a"))
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string);
            }
            if mint_b.is_none() {
                mint_b = details
                    .and_then(|d| d.get("token_mint_b"))
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string);
            }

            let cap_a = details
                .and_then(|d| d.get("amount_a_cap"))
                .and_then(parse_u64_any);
            let cap_b = details
                .and_then(|d| d.get("amount_b_cap"))
                .and_then(parse_u64_any);

            if let (Some(a), Some(b), Some(cap_a), Some(cap_b)) =
                (mint_a.clone(), mint_b.clone(), cap_a, cap_b)
            {
                let a_pk = solana_sdk::pubkey::Pubkey::from_str(&a).ok();
                let b_pk = solana_sdk::pubkey::Pubkey::from_str(&b).ok();
                if let (Some(a_pk), Some(b_pk)) = (a_pk, b_pk) {
                    let dec_a = fetch_mint_decimals_best_effort(state.provider.as_ref(), &a_pk).await;
                    let dec_b = fetch_mint_decimals_best_effort(state.provider.as_ref(), &b_pk).await;
                    if let (Some(dec_a), Some(dec_b)) = (dec_a, dec_b) {
                        let mut mints: BTreeSet<String> = BTreeSet::new();
                        mints.insert(a.clone());
                        mints.insert(b.clone());
                        if let Ok((px, _)) =
                            timeout(Duration::from_secs(2), fetch_mint_prices_usd(&mints)).await
                        {
                            let pa = px.get(&a).copied().unwrap_or(0.0);
                            let pb = px.get(&b).copied().unwrap_or(0.0);
                            if pa > 0.0 && pb > 0.0 && pa.is_finite() && pb.is_finite() {
                                let cap_usd_f = ui_amount(cap_a, dec_a) * pa + ui_amount(cap_b, dec_b) * pb;
                                let cap_usd = Decimal::from_f64_retain(cap_usd_f).unwrap_or(Decimal::ZERO);
                                if cap_usd > baseline_value {
                                    baseline_value = cap_usd;
                                    baseline_note = Some("baseline_from_open_caps_db".to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let realized_cashflow_usd = if let (Some(a), Some(b)) = (mint_a.clone(), mint_b.clone()) {
        let mut mints: BTreeSet<String> = BTreeSet::new();
        mints.insert(a.clone());
        mints.insert(b.clone());
        let (px, _src) = match timeout(Duration::from_secs(2), fetch_mint_prices_usd(&mints)).await {
            Ok(r) => r,
            Err(_) => (BTreeMap::new(), "timeout".to_string()),
        };
        let pa = px.get(&a).copied().unwrap_or(0.0);
        let pb = px.get(&b).copied().unwrap_or(0.0);
        let pa_d = Decimal::from_f64_retain(pa).unwrap_or(Decimal::ZERO);
        let pb_d = Decimal::from_f64_retain(pb).unwrap_or(Decimal::ZERO);
        let da = mint_deltas.get(&a).cloned().unwrap_or(Decimal::ZERO);
        let dbb = mint_deltas.get(&b).cloned().unwrap_or(Decimal::ZERO);
        da * pa_d + dbb * pb_d
    } else {
        Decimal::ZERO
    };

    let (collect_events, fees_collected_usd, collect_by_mint) =
        lp_fees_collected_usd_from_ledger_db(state, db, position_pubkey).await?;

    let collected_a_ui = mint_a
        .as_deref()
        .and_then(|m| collect_by_mint.get(m).copied())
        .unwrap_or(Decimal::ZERO);
    let collected_b_ui = mint_b
        .as_deref()
        .and_then(|m| collect_by_mint.get(m).copied())
        .unwrap_or(Decimal::ZERO);
    // Show both pool legs when collect happened, even if one leg is exactly zero.
    // Without this, UI displays "—" for SOL/WSOL on one-sided fee accruals.
    let fees_collected_token_a_ui = ((collect_events > 0) && mint_a.is_some()).then_some(collected_a_ui);
    let fees_collected_token_b_ui = ((collect_events > 0) && mint_b.is_some()).then_some(collected_b_ui);

    let fees_collected_token_a_raw = if let (Some(m), Some(ui)) = (mint_a.as_deref(), fees_collected_token_a_ui) {
        let pk = solana_sdk::pubkey::Pubkey::from_str(m).ok();
        let dec = if let Some(ref p) = pk {
            fetch_mint_decimals_best_effort(state.provider.as_ref(), p).await
        } else {
            None
        };
        dec.and_then(|d| decimal_ui_to_raw_u64(ui, d))
    } else {
        None
    };
    let fees_collected_token_b_raw = if let (Some(m), Some(ui)) = (mint_b.as_deref(), fees_collected_token_b_ui) {
        let pk = solana_sdk::pubkey::Pubkey::from_str(m).ok();
        let dec = if let Some(ref p) = pk {
            fetch_mint_decimals_best_effort(state.provider.as_ref(), p).await
        } else {
            None
        };
        dec.and_then(|d| decimal_ui_to_raw_u64(ui, d))
    } else {
        None
    };

    let net_pnl_usd = current_value + realized_cashflow_usd - baseline_value - tx_fees_usd;
    let net_pnl_pct = if baseline_value.is_zero() {
        Decimal::ZERO
    } else {
        net_pnl_usd / baseline_value
    };

    // Defensive fallback: if DB row data exists but is incomplete/zeros (e.g. snapshots present but
    // ledger rows not ingested), use lifecycle JSONL to give the UI meaningful numbers.
    let db_looks_empty = opened_ts.is_none()
        && closed_ts.is_none()
        && baseline_value.is_zero()
        && current_value.is_zero()
        && fee_lamports_u == 0
        && collect_events == 0
        && fees_collected_usd.is_zero();
    if db_looks_empty {
        let rows = lifecycle_rows_cached_best_effort().await;
        return node_metrics_from_lifecycle_best_effort(state, &rows, position_pubkey).await;
    }

    Ok(PositionStreamLineageNode {
        position_address: position_pubkey.to_string(),
        token_a_label: mint_a.as_deref().map(token_short_label),
        token_b_label: mint_b.as_deref().map(token_short_label),
        token_mint_a: mint_a.clone(),
        token_mint_b: mint_b.clone(),
        opened_ts_utc: opened_ts.map(|t| t.to_rfc3339()),
        closed_ts_utc: closed_ts.map(|t| t.to_rfc3339()),
        baseline_value_usd: baseline_value,
        current_value_usd: current_value,
        tx_fee_lamports: fee_lamports_u,
        tx_fees_usd,
        fees_collected_usd,
        fees_collected_token_a_ui,
        fees_collected_token_b_ui,
        fees_collected_token_a_raw,
        fees_collected_token_b_raw,
        collect_events,
        realized_cashflow_usd,
        net_pnl_usd,
        net_pnl_pct,
        note: Some(format!(
            "Best-effort per-PDA. tx_fee_lamports = sum of network fees for this PDA; fees_collected_usd = bot_collect_fees legs × USD (mint map + fee_payer_token_*_delta_ui columns when present); tx fees use SOL/USD ({sol_src}). cashflow uses fee_payer_token_deltas × current mint USD prices when baseline mints are known.{}",
            baseline_note
                .as_deref()
                .map(|n| format!(" {n}."))
                .unwrap_or_default()
        )),
    })
}

async fn node_metrics_from_lifecycle_best_effort(
    state: &AppState,
    rows: &[LifecycleRow],
    position_pubkey: &str,
) -> Result<PositionStreamLineageNode, ApiError> {
    let mut tx_fee_lamports_sum: u64 = 0;
    let mut opened_ts: Option<DateTime<Utc>> = None;
    let mut closed_ts: Option<DateTime<Utc>> = None;

    // Try to infer pool + leg mints from any row details.
    let mut pool_address: Option<String> = None;
    let mut mint_a: Option<String> = None;
    let mut mint_b: Option<String> = None;
    // Realized cashflow should NOT include principal movements (open/close legs).
    // We therefore aggregate only non-open/non-close token deltas (e.g. collect fees, swaps).
    let mut mint_deltas: BTreeMap<String, Decimal> = BTreeMap::new();
    let mut open_leg_deltas: Option<BTreeMap<String, Decimal>> = None;
    let mut close_leg_deltas: Option<BTreeMap<String, Decimal>> = None;
    let mut open_amount_a_cap: Option<u64> = None;
    let mut open_amount_b_cap: Option<u64> = None;

    for r in rows {
        if r.position_pubkey.as_deref() != Some(position_pubkey) {
            continue;
        }
        if pool_address.is_none() {
            pool_address = r.pool_address.clone();
        }
        if let Some(f) = r.tx_fee_lamports {
            tx_fee_lamports_sum = tx_fee_lamports_sum.saturating_add(f);
        }
        match r.event.as_deref() {
            Some("bot_open_position") | Some("bot_open_position_full_range") | Some("position_open") => {
                if opened_ts.is_none() {
                    opened_ts = r.ts_utc;
                }
                if open_leg_deltas.is_none() {
                    if let Some(obj) = r.fee_payer_token_deltas.as_ref().and_then(|v| v.as_object())
                    {
                        let mut m: BTreeMap<String, Decimal> = BTreeMap::new();
                        for (mint, dv) in obj {
                            if let Some(d) = dec_from_any(dv) {
                                m.insert(mint.clone(), d);
                            }
                        }
                        if !m.is_empty() {
                            open_leg_deltas = Some(m);
                        }
                    }
                }
                if (open_amount_a_cap.is_none() || open_amount_b_cap.is_none())
                    && r.details.is_some()
                {
                    if let Some(d) = r.details.as_ref().and_then(|v| v.as_object()) {
                        if open_amount_a_cap.is_none() {
                            open_amount_a_cap = d.get("amount_a_cap").and_then(|v| v.as_u64());
                        }
                        if open_amount_b_cap.is_none() {
                            open_amount_b_cap = d.get("amount_b_cap").and_then(|v| v.as_u64());
                        }
                    }
                }
            }
            Some("bot_close_position") | Some("position_close") => {
                closed_ts = r.ts_utc.or(closed_ts);
                if close_leg_deltas.is_none() {
                    if let Some(obj) = r.fee_payer_token_deltas.as_ref().and_then(|v| v.as_object())
                    {
                        let mut m: BTreeMap<String, Decimal> = BTreeMap::new();
                        for (mint, dv) in obj {
                            if let Some(d) = dec_from_any(dv) {
                                m.insert(mint.clone(), d);
                            }
                        }
                        if !m.is_empty() {
                            close_leg_deltas = Some(m);
                        }
                    }
                }
            }
            _ => {}
        }

        // Parse token deltas (string decimals) for realized cashflow — exclude open/close principal.
        let is_principal = is_lifecycle_open_event(r.event.as_deref())
            || is_lifecycle_close_event(r.event.as_deref());
        if !is_principal {
            if let Some(obj) = r.fee_payer_token_deltas.as_ref().and_then(|v| v.as_object()) {
                for (mint, dv) in obj {
                    if let Some(d) = dec_from_any(dv) {
                        *mint_deltas.entry(mint.clone()).or_insert(Decimal::ZERO) += d;
                    }
                }
            }
        }

        // Pull leg mints from details if present.
        if mint_a.is_none() || mint_b.is_none() {
            if let Some(details) = r.details.as_ref().and_then(|v| v.as_object()) {
                let da = details
                    .get("token_mint_a")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string());
                let db = details
                    .get("token_mint_b")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string());
                if mint_a.is_none() {
                    mint_a = da;
                }
                if mint_b.is_none() {
                    mint_b = db;
                }
            }
        }
    }

    // Resolve missing mints from pool (best-effort) so UI can show token pair.
    if (mint_a.is_none() || mint_b.is_none()) && pool_address.is_some() {
        let pool = pool_address.clone().unwrap_or_default();
        if !pool.trim().is_empty() {
            let (ma, mb) = pool_leg_mints_best_effort(state, &pool).await;
            if mint_a.is_none() {
                mint_a = ma;
            }
            if mint_b.is_none() {
                mint_b = mb;
            }
        }
    }

    let (collect_events, fees_collected_usd, collect_by_mint) =
        lp_fees_collected_usd_from_lifecycle_rows(state, rows, position_pubkey).await;

    let token_a_label = mint_a.as_deref().map(token_short_label);
    let token_b_label = mint_b.as_deref().map(token_short_label);
    let collected_a_ui = mint_a
        .as_deref()
        .and_then(|m| collect_by_mint.get(m).copied())
        .unwrap_or(Decimal::ZERO);
    let collected_b_ui = mint_b
        .as_deref()
        .and_then(|m| collect_by_mint.get(m).copied())
        .unwrap_or(Decimal::ZERO);
    // Show both pool legs when collect happened, even if one leg is exactly zero.
    // Without this, UI displays "—" for SOL/WSOL on one-sided fee accruals.
    let fees_collected_token_a_ui = ((collect_events > 0) && mint_a.is_some()).then_some(collected_a_ui);
    let fees_collected_token_b_ui = ((collect_events > 0) && mint_b.is_some()).then_some(collected_b_ui);

    let fees_collected_token_a_raw = if let (Some(m), Some(ui)) = (mint_a.as_deref(), fees_collected_token_a_ui) {
        let pk = solana_sdk::pubkey::Pubkey::from_str(m).ok();
        let dec = if let Some(ref p) = pk {
            fetch_mint_decimals_best_effort(state.provider.as_ref(), p).await
        } else {
            None
        };
        dec.and_then(|d| decimal_ui_to_raw_u64(ui, d))
    } else {
        None
    };
    let fees_collected_token_b_raw = if let (Some(m), Some(ui)) = (mint_b.as_deref(), fees_collected_token_b_ui) {
        let pk = solana_sdk::pubkey::Pubkey::from_str(m).ok();
        let dec = if let Some(ref p) = pk {
            fetch_mint_decimals_best_effort(state.provider.as_ref(), p).await
        } else {
            None
        };
        dec.and_then(|d| decimal_ui_to_raw_u64(ui, d))
    } else {
        None
    };

    let db_disabled = state.db.is_none();

    // Convert tx fees to USD (SOL/USD).
    let (sol_usd, sol_src) = sol_usd().await;
    let tx_fees_usd = if sol_usd > 0.0 {
        Decimal::from_f64_retain((tx_fee_lamports_sum as f64 / 1e9) * sol_usd)
            .unwrap_or(Decimal::ZERO)
    } else {
        Decimal::ZERO
    };

    // Convert realized cashflow to USD using current mint prices for the pool leg mints when known.
    // Also derive a "start value" and (when closed) an "end value" using **open/close** token deltas.
    let mut price_note: Option<String> = None;
    let (mut baseline_value_usd, realized_cashflow_usd, end_value_usd_from_close) =
        if let (Some(a), Some(b)) = (mint_a.clone(), mint_b.clone()) {
            let mut mints: BTreeSet<String> = BTreeSet::new();
            mints.insert(a.clone());
            mints.insert(b.clone());
            let (px, src) = match timeout(Duration::from_secs(2), fetch_mint_prices_usd(&mints)).await {
                Ok(r) => r,
                Err(_) => (BTreeMap::new(), "timeout".to_string()),
            };
            price_note = Some(src);
            let pa = px.get(&a).copied().unwrap_or(0.0);
            let pb = px.get(&b).copied().unwrap_or(0.0);
            let pa_d = Decimal::from_f64_retain(pa).unwrap_or(Decimal::ZERO);
            let pb_d = Decimal::from_f64_retain(pb).unwrap_or(Decimal::ZERO);

            let da_all = mint_deltas.get(&a).cloned().unwrap_or(Decimal::ZERO);
            let db_all = mint_deltas.get(&b).cloned().unwrap_or(Decimal::ZERO);
            let realized_usd = da_all * pa_d + db_all * pb_d;

            let baseline = open_leg_deltas
                .as_ref()
                .map(|m| {
                    let da = m.get(&a).cloned().unwrap_or(Decimal::ZERO);
                    let dbb = m.get(&b).cloned().unwrap_or(Decimal::ZERO);
                    // On open, payer deltas are typically negative (spent/deposited). Baseline is the
                    // absolute USD outflow into the position legs.
                    (da * pa_d + dbb * pb_d) * Decimal::NEGATIVE_ONE
                })
                .unwrap_or(Decimal::ZERO);

            let end_close = close_leg_deltas.as_ref().map(|m| {
                let da = m.get(&a).cloned().unwrap_or(Decimal::ZERO);
                let dbb = m.get(&b).cloned().unwrap_or(Decimal::ZERO);
                // On close, payer deltas are typically positive (returned to wallet).
                da * pa_d + dbb * pb_d
            });

            (baseline.max(Decimal::ZERO), realized_usd, end_close)
        } else {
            (Decimal::ZERO, Decimal::ZERO, None)
        };

    // Current value:
    // - when DB is enabled: try to get on-chain position valuation for open positions
    // - when DB is disabled: rely on close-leg deltas only (no on-chain calls)
    let mut current_value_usd = if !db_disabled {
        if let Ok(pk) = solana_sdk::pubkey::Pubkey::from_str(position_pubkey) {
            if let Ok(pos) = monitored_position_from_chain(state.provider.clone(), &pk).await {
                let prices =
                    fetch_prices_for_positions(state.provider.clone(), std::slice::from_ref(&pos)).await;
                compute_position_usd_valuation(state.provider.clone(), &pos, &prices)
                    .await
                    .map(|v| v.value_usd)
                    .unwrap_or(Decimal::ZERO)
            } else {
                Decimal::ZERO
            }
        } else {
            Decimal::ZERO
        }
    } else {
        Decimal::ZERO
    };

    // For closed positions we can't fetch on-chain value. Use the close leg as "end value" when available.
    if current_value_usd.is_zero() {
        if let Some(end) = end_value_usd_from_close {
            current_value_usd = end;
        }
    }

    // DB-disabled mode: active positions have no close leg yet. Still show a meaningful "end value"
    // using a best-effort on-chain valuation with a tight timeout to avoid UI stalls.
    if db_disabled && current_value_usd.is_zero() && closed_ts.is_none() {
        if let Ok(pk) = solana_sdk::pubkey::Pubkey::from_str(position_pubkey) {
            if let Ok(Ok(pos)) = timeout(
                Duration::from_secs(2),
                monitored_position_from_chain(state.provider.clone(), &pk),
            )
            .await
            {
                let prices =
                    fetch_prices_for_positions(state.provider.clone(), std::slice::from_ref(&pos)).await;
                if let Ok(v) =
                    compute_position_usd_valuation(state.provider.clone(), &pos, &prices).await
                {
                    if v.value_usd > Decimal::ZERO {
                        current_value_usd = v.value_usd;
                    }
                }
            }
        }
    }

    // If baseline looks unrealistically low vs current value, try to use open caps from ledger details.
    // This happens when `fee_payer_token_deltas` for open is missing one leg (common on WSOL/ATA flows).
    if let (Some(pool), Some(a), Some(b), Some(cap_a), Some(cap_b)) = (
        pool_address.clone(),
        mint_a.clone(),
        mint_b.clone(),
        open_amount_a_cap,
        open_amount_b_cap,
    ) {
        if current_value_usd > Decimal::ZERO && baseline_value_usd > Decimal::ZERO {
            // heuristic: baseline less than 60% of current is suspicious for short windows
            let suspicious = baseline_value_usd < current_value_usd * Decimal::new(60, 2);
            if suspicious {
                let mint_a_pk = solana_sdk::pubkey::Pubkey::from_str(&a).ok();
                let mint_b_pk = solana_sdk::pubkey::Pubkey::from_str(&b).ok();
                if let (Some(ma), Some(mb)) = (mint_a_pk, mint_b_pk) {
                    let dec_a = fetch_mint_decimals_best_effort(state.provider.as_ref(), &ma).await;
                    let dec_b = fetch_mint_decimals_best_effort(state.provider.as_ref(), &mb).await;
                    if let (Some(da), Some(db)) = (dec_a, dec_b) {
                        let mut mints: BTreeSet<String> = BTreeSet::new();
                        mints.insert(a.clone());
                        mints.insert(b.clone());
                        if let Ok((px, _src)) =
                            timeout(Duration::from_secs(2), fetch_mint_prices_usd(&mints)).await
                        {
                            let pa = px.get(&a).copied().unwrap_or(0.0);
                            let pb = px.get(&b).copied().unwrap_or(0.0);
                            if pa.is_finite() && pb.is_finite() && pa > 0.0 && pb > 0.0 {
                                let cap_value_usd_f =
                                    ui_amount(cap_a, da) * pa + ui_amount(cap_b, db) * pb;
                                let cap_value_usd = Decimal::from_f64_retain(cap_value_usd_f)
                                    .unwrap_or(Decimal::ZERO);
                                if cap_value_usd > baseline_value_usd {
                                    baseline_value_usd = cap_value_usd;
                                    if let Some(ref mut n) = price_note {
                                        n.push_str("; baseline_from_caps");
                                    }
                                    // keep pool used to avoid unused warning in tuple
                                    let _ = pool;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let net_pnl_usd = current_value_usd + realized_cashflow_usd - baseline_value_usd - tx_fees_usd;
    let net_pnl_pct = if baseline_value_usd.is_zero() {
        Decimal::ZERO
    } else {
        net_pnl_usd / baseline_value_usd
    };

    Ok(PositionStreamLineageNode {
        position_address: position_pubkey.to_string(),
        token_a_label,
        token_b_label,
        token_mint_a: mint_a.clone(),
        token_mint_b: mint_b.clone(),
        opened_ts_utc: opened_ts.map(|t| t.to_rfc3339()),
        closed_ts_utc: closed_ts.map(|t| t.to_rfc3339()),
        baseline_value_usd,
        current_value_usd,
        tx_fee_lamports: tx_fee_lamports_sum,
        tx_fees_usd,
        fees_collected_usd,
        fees_collected_token_a_ui,
        fees_collected_token_b_ui,
        fees_collected_token_a_raw,
        fees_collected_token_b_raw,
        collect_events,
        realized_cashflow_usd,
        net_pnl_usd,
        net_pnl_pct,
        note: Some(format!(
            "{} tx_fee_lamports = network fees for this PDA; fees_collected_usd = bot_collect_fees × USD; tx fees use SOL/USD ({sol_src}). start/end value derived from open/close token deltas × current mint USD prices ({}).",
            if db_disabled {
                "DB is disabled; per-node metrics from lifecycle JSONL (no on-chain valuation)."
            } else {
                "Per-node metrics from lifecycle JSONL + on-chain valuation when available."
            },
            price_note.unwrap_or_else(|| "no price source".to_string())
        )),
    })
}

/// Build an ordered stream lineage chain and enrich each node with best-effort metrics.
pub async fn compute_position_stream_lineage(
    state: &AppState,
    position_address: &str,
) -> Result<PositionStreamLineageResponse, ApiError> {
    let entry = position_address.trim();

    // Connectivity + totals: reuse existing stream services.
    let perf = compute_position_stream_performance(state, entry, true).await?;
    let mut totals = compute_position_stream_pnl(state, entry).await.ok();

    let Some(db) = state.db.as_ref() else {
        let rows = lifecycle_rows_cached_best_effort().await;
        let chain = chain_from_lifecycle_best_effort_rows(&rows, entry, 25);
        let mut nodes = Vec::new();
        for p in &chain {
            nodes.push(node_metrics_from_lifecycle_best_effort(state, &rows, p).await?);
        }

        // Fill gaps in closed nodes (close row missing leg token deltas) by using next node baseline.
        apply_end_value_fallback_from_next_baseline(&mut nodes);

        totals = maybe_compute_totals_from_nodes(
            entry,
            &totals,
            &nodes,
            Some("DB is disabled; totals computed best-effort from lineage nodes (baseline=first open outflow, current=last node end value/current mark; IL/HODL unavailable)."),
        )
        .or(totals);
        let chain_cost_summary = rollup_lineage_chain_costs(&nodes);
        return Ok(PositionStreamLineageResponse {
            position_address: entry.to_string(),
            chain,
            nodes,
            totals,
            chain_cost_summary,
            note: Some(
                "DB is disabled; chain reconstructed best-effort from lifecycle JSONL (close→open in same pool+fee payer, 10min window)."
                    .to_string(),
            ),
        });
    };

    // Load edges among the connected component to build a linear chain.
    let mut edge_rows = sqlx::query(
        r#"
        SELECT ts_utc, old_position, new_position, rebalance_session_id
        FROM position_stream_edges
        WHERE old_position = ANY($1) OR new_position = ANY($1)
        "#,
    )
    .bind(&perf.positions)
    .fetch_all(db.pool())
    .await
    .map_err(|e| ApiError::internal(format!("stream lineage: edges query: {e}")))?;

    let mut edges: Vec<(Option<DateTime<Utc>>, String, String, String)> = Vec::new();
    for r in edge_rows.drain(..) {
        let ts: Option<DateTime<Utc>> = r.try_get("ts_utc").ok();
        let oldp: String = r.try_get("old_position").unwrap_or_default();
        let newp: String = r.try_get("new_position").unwrap_or_default();
        let sid: String = r.try_get("rebalance_session_id").unwrap_or_default();
        if oldp.trim().is_empty() || newp.trim().is_empty() {
            continue;
        }
        edges.push((ts, oldp, newp, sid));
    }

    let mut chain = build_linear_chain(&perf.positions, &edges, entry);

    // IL ledger → `position_stream_edges` is optional; without ingested edges the DB graph is a
    // single PDA even when `orca_position_lifecycle.jsonl` has the full close→open chain.
    if chain.len() <= 1 {
        // First fallback: registry.jsonl (cheaper and usually complete for open/close sequences).
        let reg_rows = registry_rows_best_effort();
        if !reg_rows.is_empty() {
            let rc = chain_from_registry_best_effort_rows(&reg_rows, entry, 50);
            if rc.len() > chain.len() {
                chain = rc;
            }
        }

        // Second fallback: lifecycle JSONL (richer, but may omit some PDAs depending on collector config).
        let rows = lifecycle_rows_cached_best_effort().await;
        let lc = chain_from_lifecycle_best_effort_rows(&rows, entry, 25);
        if lc.len() > chain.len() {
            chain = lc;
        }
    }

    // Per-node metrics (parallel; each node skips expensive snapshot self-seed — lineage uses lifecycle).
    let st = state.clone();
    let node_futs: Vec<_> = chain
        .iter()
        .map(|p| {
            let st = st.clone();
            let p = p.clone();
            async move { node_metrics(&st, &p, true).await }
        })
        .collect();
    let mut nodes: Vec<PositionStreamLineageNode> = Vec::with_capacity(node_futs.len());
    for res in join_all(node_futs).await {
        nodes.push(res?);
    }

    // Fill gaps in closed nodes (close row missing leg token deltas) by using next node baseline.
    // This is useful in DB mode too when per-PDA current/end valuation is missing.
    apply_end_value_fallback_from_next_baseline(&mut nodes);

    // If stream PnL has no valuation snapshots yet (common soon after enabling DB),
    // provide a consistent totals row derived from lineage nodes.
    totals = maybe_compute_totals_from_nodes(
        entry,
        &totals,
        &nodes,
        Some("No valuation snapshots yet; totals computed best-effort from lineage nodes (IL/HODL unavailable)."),
    )
    .or(totals);

    let chain_cost_summary = rollup_lineage_chain_costs(&nodes);
    Ok(PositionStreamLineageResponse {
        position_address: entry.to_string(),
        chain,
        nodes,
        totals,
        chain_cost_summary,
        note: Some(
            "Lineage chain is best-effort and assumes a mostly linear old→new rotation path (common for strategies). If edges are missing, the chain may be incomplete."
                .to_string(),
        ),
    })
}

fn apply_end_value_fallback_from_next_baseline(nodes: &mut [PositionStreamLineageNode]) {
    for i in 0..nodes.len().saturating_sub(1) {
        let next_baseline = nodes[i + 1].baseline_value_usd;
        let is_closed = nodes[i].closed_ts_utc.is_some();
        let has_end = !nodes[i].current_value_usd.is_zero();
        if is_closed && !has_end && !next_baseline.is_zero() {
            nodes[i].current_value_usd = next_baseline;
            nodes[i].net_pnl_usd = nodes[i].current_value_usd
                + nodes[i].realized_cashflow_usd
                - nodes[i].baseline_value_usd
                - nodes[i].tx_fees_usd;
            if !nodes[i].baseline_value_usd.is_zero() {
                nodes[i].net_pnl_pct = nodes[i].net_pnl_usd / nodes[i].baseline_value_usd;
            }
            if let Some(ref mut n) = nodes[i].note {
                n.push_str(" end value approximated from next node baseline (rotation close→open); close row lacked leg token deltas.");
            } else {
                nodes[i].note = Some(
                    "end value approximated from next node baseline (rotation close→open); close row lacked leg token deltas."
                        .to_string(),
                );
            }
        }
    }
}

fn maybe_compute_totals_from_nodes(
    entry: &str,
    existing: &Option<crate::models::PositionStreamPnLResponse>,
    nodes: &[PositionStreamLineageNode],
    note: Option<&str>,
) -> Option<crate::models::PositionStreamPnLResponse> {
    if nodes.is_empty() {
        return None;
    }
    let totals_is_placeholder = existing.as_ref().is_some_and(|t| {
        t.baseline_value_usd.is_zero()
            && t.current_value_usd.is_zero()
            && t.tx_fees_usd.is_zero()
            && t.realized_cashflow_usd.is_zero()
            && t.net_pnl_usd.is_zero()
            && t.note
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase()
                .contains("no valuation snapshots")
    });
    if existing.is_some() && !totals_is_placeholder {
        return None;
    }

    let baseline_value_usd = nodes
        .first()
        .map(|n| n.baseline_value_usd)
        .unwrap_or(Decimal::ZERO);
    let current_value_usd = nodes
        .last()
        .map(|n| n.current_value_usd)
        .unwrap_or(Decimal::ZERO);
    let tx_fees_usd: Decimal = nodes.iter().map(|n| n.tx_fees_usd).sum();
    let realized_cashflow_usd: Decimal = nodes.iter().map(|n| n.realized_cashflow_usd).sum();
    let net_pnl_usd = current_value_usd + realized_cashflow_usd - baseline_value_usd - tx_fees_usd;
    let net_pnl_pct = if baseline_value_usd.is_zero() {
        Decimal::ZERO
    } else {
        net_pnl_usd / baseline_value_usd
    };

    Some(crate::models::PositionStreamPnLResponse {
        position_address: entry.to_string(),
        baseline_ts_utc: nodes.first().and_then(|n| n.opened_ts_utc.clone()),
        current_ts_utc: nodes.last().and_then(|n| n.closed_ts_utc.clone()),
        baseline_value_usd,
        current_value_usd,
        hodl_value_usd: Decimal::ZERO,
        il_usd: Decimal::ZERO,
        il_pct: Decimal::ZERO,
        tx_fees_usd,
        realized_cashflow_usd,
        net_pnl_usd,
        net_pnl_pct,
        note: note.map(|s| s.to_string()),
    })
}

/// Backfill synthetic DB valuation snapshots for positions present in lifecycle JSONL.
///
/// For each PDA, we try to create:
/// - baseline snapshot at open timestamp using open leg deltas (fee payer token deltas)
/// - "end/current" snapshot at close timestamp using close leg deltas
///
/// USD valuation is computed using **current** free mint prices. Rows are tagged via `price_source`
/// so downstream consumers can treat them as approximate historical values.
pub async fn backfill_valuation_snapshots_from_lifecycle_current_prices(
    state: &AppState,
    req: &crate::models::BackfillValuationSnapshotsRequest,
) -> Result<crate::models::BackfillValuationSnapshotsResponse, ApiError> {
    let Some(db) = state.db.as_ref() else {
        return Err(ApiError::ServiceUnavailable(
            "DB is not enabled; cannot backfill valuation snapshots".to_string(),
        ));
    };

    let rows = lifecycle_rows_cached_best_effort().await;
    let mut open_by_pos: BTreeMap<String, LifecycleRow> = BTreeMap::new();
    let mut close_by_pos: BTreeMap<String, LifecycleRow> = BTreeMap::new();

    for r in rows.iter() {
        let Some(pos) = r.position_pubkey.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
            continue;
        };
        // First open wins (rows are sorted by ts_utc asc).
        if is_lifecycle_open_event(r.event.as_deref()) && !open_by_pos.contains_key(pos) {
            open_by_pos.insert(pos.to_string(), r.clone());
        }
        // Last close wins.
        if is_lifecycle_close_event(r.event.as_deref()) {
            close_by_pos.insert(pos.to_string(), r.clone());
        }
    }

    // Stable processing order: by open time (then PDA).
    let mut positions: Vec<String> = open_by_pos.keys().cloned().collect();
    positions.sort_by(|a, b| {
        let ta = open_by_pos.get(a).and_then(|r| r.ts_utc);
        let tb = open_by_pos.get(b).and_then(|r| r.ts_utc);
        ta.cmp(&tb).then_with(|| a.cmp(b))
    });
    if let Some(limit) = req.limit_positions {
        let lim = limit as usize;
        if positions.len() > lim {
            positions.truncate(lim);
        }
    }

    // Resolve pool leg mints (best-effort), then fetch mint prices in one batch.
    let mut mints_by_pos: HashMap<String, (String, String, String)> = HashMap::new(); // pos -> (pool, mint_a, mint_b)
    let mut mints: BTreeSet<String> = BTreeSet::new();
    for pos in &positions {
        let open = open_by_pos.get(pos);
        let close = close_by_pos.get(pos);
        let pool = open
            .and_then(|r| r.pool_address.clone())
            .or_else(|| close.and_then(|r| r.pool_address.clone()));
        let Some(pool) = pool.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
            continue;
        };
        let (ma, mb) = pool_leg_mints_best_effort(state, pool).await;
        let (Some(ma), Some(mb)) = (ma, mb) else { continue };
        mints.insert(ma.clone());
        mints.insert(mb.clone());
        mints_by_pos.insert(pos.to_string(), (pool.to_string(), ma, mb));
    }
    let (px, _) = fetch_mint_prices_usd(&mints).await;

    let price_source = "lifecycle_current_prices".to_string();
    let mut considered: u32 = 0;
    let mut with_open: u32 = 0;
    let mut with_close: u32 = 0;
    let mut inserted: u32 = 0;

    for pos in &positions {
        considered += 1;
        let Some((pool, mint_a, mint_b)) = mints_by_pos.get(pos).cloned() else {
            continue;
        };
        let pa = px.get(&mint_a).copied().unwrap_or(0.0);
        let pb = px.get(&mint_b).copied().unwrap_or(0.0);
        let pa_d = Decimal::from_f64_retain(pa).unwrap_or(Decimal::ZERO);
        let pb_d = Decimal::from_f64_retain(pb).unwrap_or(Decimal::ZERO);
        if pa <= 0.0 || pb <= 0.0 || !pa.is_finite() || !pb.is_finite() {
            continue;
        }

        // Baseline snapshot at open ts.
        if let Some(open) = open_by_pos.get(pos) {
            if let (Some(ts), Some(obj)) = (
                open.ts_utc,
                open.fee_payer_token_deltas.as_ref().and_then(|v| v.as_object()),
            ) {
                let da = obj.get(&mint_a).and_then(dec_from_any).unwrap_or(Decimal::ZERO);
                let dbb = obj.get(&mint_b).and_then(dec_from_any).unwrap_or(Decimal::ZERO);
                // For opens, deltas are typically negative (spent). Convert to positive basket.
                let amount_a_ui = (-da).max(Decimal::ZERO);
                let amount_b_ui = (-dbb).max(Decimal::ZERO);
                if !amount_a_ui.is_zero() || !amount_b_ui.is_zero() {
                    with_open += 1;
                    let value_usd = amount_a_ui * pa_d + amount_b_ui * pb_d;
                    let raw = serde_json::json!({
                        "source": price_source,
                        "kind": "baseline_open",
                        "position": pos,
                        "pool": pool,
                        "ts_utc": ts.to_rfc3339(),
                        "token_mint_a": mint_a,
                        "token_mint_b": mint_b,
                        "fee_payer_token_deltas": open.fee_payer_token_deltas,
                        "amount_a_ui": amount_a_ui,
                        "amount_b_ui": amount_b_ui,
                        "price_a_usd": pa,
                        "price_b_usd": pb,
                    });
                    if !req.dry_run {
                        let res = sqlx::query(
                            r#"
                            INSERT INTO position_stream_valuation_snapshots
                              (position_pubkey, ts_utc, pool_pubkey, value_usd, amount_a_ui, amount_b_ui, fees_usd, token_mint_a, token_mint_b, price_a_usd, price_b_usd, price_source, raw_json)
                            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
                            ON CONFLICT (position_pubkey, ts_utc) DO NOTHING
                            "#,
                        )
                        .bind(pos)
                        .bind(ts)
                        .bind(&pool)
                        .bind(value_usd)
                        .bind(amount_a_ui)
                        .bind(amount_b_ui)
                        .bind(Decimal::ZERO)
                        .bind(&mint_a)
                        .bind(&mint_b)
                        .bind(pa_d)
                        .bind(pb_d)
                        .bind(&price_source)
                        .bind(raw)
                        .execute(db.pool())
                        .await
                        .map_err(|e| ApiError::internal(format!("backfill snapshots: insert baseline: {e}")))?;
                        inserted += res.rows_affected() as u32;
                    }
                }
            }
        }

        // End snapshot at close ts.
        if let Some(close) = close_by_pos.get(pos) {
            if let (Some(ts), Some(obj)) = (
                close.ts_utc,
                close.fee_payer_token_deltas.as_ref().and_then(|v| v.as_object()),
            ) {
                let da = obj.get(&mint_a).and_then(dec_from_any).unwrap_or(Decimal::ZERO);
                let dbb = obj.get(&mint_b).and_then(dec_from_any).unwrap_or(Decimal::ZERO);
                // For closes, deltas are typically positive (received). Keep positive.
                let amount_a_ui = da.max(Decimal::ZERO);
                let amount_b_ui = dbb.max(Decimal::ZERO);
                if !amount_a_ui.is_zero() || !amount_b_ui.is_zero() {
                    with_close += 1;
                    let value_usd = amount_a_ui * pa_d + amount_b_ui * pb_d;
                    let raw = serde_json::json!({
                        "source": price_source,
                        "kind": "end_close",
                        "position": pos,
                        "pool": pool,
                        "ts_utc": ts.to_rfc3339(),
                        "token_mint_a": mint_a,
                        "token_mint_b": mint_b,
                        "fee_payer_token_deltas": close.fee_payer_token_deltas,
                        "amount_a_ui": amount_a_ui,
                        "amount_b_ui": amount_b_ui,
                        "price_a_usd": pa,
                        "price_b_usd": pb,
                    });
                    if !req.dry_run {
                        let res = sqlx::query(
                            r#"
                            INSERT INTO position_stream_valuation_snapshots
                              (position_pubkey, ts_utc, pool_pubkey, value_usd, amount_a_ui, amount_b_ui, fees_usd, token_mint_a, token_mint_b, price_a_usd, price_b_usd, price_source, raw_json)
                            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
                            ON CONFLICT (position_pubkey, ts_utc) DO NOTHING
                            "#,
                        )
                        .bind(pos)
                        .bind(ts)
                        .bind(&pool)
                        .bind(value_usd)
                        .bind(amount_a_ui)
                        .bind(amount_b_ui)
                        .bind(Decimal::ZERO)
                        .bind(&mint_a)
                        .bind(&mint_b)
                        .bind(pa_d)
                        .bind(pb_d)
                        .bind(&price_source)
                        .bind(raw)
                        .execute(db.pool())
                        .await
                        .map_err(|e| ApiError::internal(format!("backfill snapshots: insert end: {e}")))?;
                        inserted += res.rows_affected() as u32;
                    }
                }
            }
        }
    }

    let note = if req.dry_run {
        Some("dry_run=true: computed snapshots but did not write to DB".to_string())
    } else {
        Some("Backfilled valuation snapshots from lifecycle open/close deltas using current free USD prices; historical accuracy is approximate by design.".to_string())
    };

    Ok(crate::models::BackfillValuationSnapshotsResponse {
        ok: true,
        positions_considered: considered,
        positions_with_open: with_open,
        positions_with_close: with_close,
        rows_inserted: inserted,
        price_source,
        note,
    })
}

/// Best-effort inference: for a given position PDA (entry), try to find its immediate parent PDA
/// from lifecycle JSONL by matching the closest preceding `bot_close_position`/`position_close`
/// in the same pool and by the same fee payer.
pub async fn infer_parent_position_from_lifecycle_best_effort(entry: &str) -> Option<String> {
    let entry = entry.trim();
    if entry.is_empty() {
        return None;
    }
    let rows = lifecycle_rows_cached_best_effort().await;
    // Find the earliest open row for entry.
    let mut open_row: Option<&LifecycleRow> = None;
    for r in rows.iter() {
        if r.position_pubkey.as_deref() != Some(entry) {
            continue;
        }
        if !is_lifecycle_open_event(r.event.as_deref()) {
            continue;
        }
        if r.ts_utc.is_some() && r.pool_address.is_some() && r.fee_payer_pubkey.is_some() {
            open_row = Some(r);
            break;
        }
    }
    let o = open_row?;
    let open_ts = o.ts_utc?;
    let pool = o.pool_address.as_deref()?.trim();
    let payer = o.fee_payer_pubkey.as_deref()?.trim();
    if pool.is_empty() || payer.is_empty() {
        return None;
    }

    // Search for the latest close row before open_ts within 10 minutes window.
    let window_start = open_ts - chrono::Duration::minutes(10);
    let mut best: Option<(DateTime<Utc>, String)> = None;
    for r in rows.iter() {
        let Some(ts) = r.ts_utc else { continue };
        if ts < window_start || ts > open_ts {
            continue;
        }
        if r.pool_address.as_deref() != Some(pool) {
            continue;
        }
        if r.fee_payer_pubkey.as_deref() != Some(payer) {
            continue;
        }
        if !is_lifecycle_close_event(r.event.as_deref()) {
            continue;
        }
        let Some(parent) = r.position_pubkey.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
            continue;
        };
        if best.as_ref().is_none_or(|(bts, _)| ts > *bts) {
            best = Some((ts, parent.to_string()));
        }
    }
    best.map(|(_, p)| p)
}

