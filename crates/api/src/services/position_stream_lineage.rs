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
use spl_token::solana_program::program_pack::Pack;
use spl_token::state::Mint;
use sqlx::Row;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::str::FromStr;
use std::fs;

const WSOL_MINT: &str = "So11111111111111111111111111111111111111112";

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

fn parse_lifecycle_rows_best_effort() -> Vec<LifecycleRow> {
    let path = clmm_lp_protocols::ledger::tx_lifecycle::ledger_read_path();
    let txt = fs::read_to_string(&path).unwrap_or_default();
    let mut out = Vec::new();
    for line in txt.lines() {
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
            details,
        });
    }
    out.sort_by(|a, b| a.ts_utc.cmp(&b.ts_utc));
    out
}

fn chain_from_lifecycle_best_effort(entry: &str, max_hops: usize) -> Vec<String> {
    let rows = parse_lifecycle_rows_best_effort();

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
        let window_start = open_ts - chrono::Duration::minutes(10);
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
        let window_end = close_ts + chrono::Duration::minutes(10);
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

#[allow(clippy::too_many_lines)]
async fn lp_fees_collected_usd_from_lifecycle_rows(
    state: &AppState,
    rows: &[LifecycleRow],
    position_pubkey: &str,
) -> (u32, Decimal) {
    let mut pool_mints: HashMap<String, (String, String)> = HashMap::new();
    let mut events: u32 = 0;
    let mut by_mint_ui: BTreeMap<String, Decimal> = BTreeMap::new();

    for r in rows {
        if r.position_pubkey.as_deref() != Some(position_pubkey) {
            continue;
        }
        if r.event.as_deref() != Some("bot_collect_fees") {
            continue;
        }
        events += 1;
        let Some(pool) = r.pool_address.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
            continue;
        };
        let (ma, mb) = match pool_mints.get(pool) {
            Some(p) => (p.0.clone(), p.1.clone()),
            None => {
                let Ok(ps) = clmm_lp_protocols::prelude::WhirlpoolReader::new(state.provider.clone())
                    .get_pool_state(pool)
                    .await
                else {
                    continue;
                };
                let pair = (ps.token_mint_a.to_string(), ps.token_mint_b.to_string());
                pool_mints.insert(pool.to_string(), pair.clone());
                pair
            }
        };
        let Some(obj) = r.fee_payer_token_deltas.as_ref().and_then(|v| v.as_object()) else {
            continue;
        };
        for m in [ma, mb] {
            if let Some(dv) = obj.get(&m) {
                if let Some(d) = dec_from_any(dv) {
                    if d > Decimal::ZERO {
                        *by_mint_ui.entry(m).or_insert(Decimal::ZERO) += d;
                    }
                }
            }
        }
    }

    if by_mint_ui.is_empty() {
        return (events, Decimal::ZERO);
    }

    let mints: BTreeSet<String> = by_mint_ui.keys().cloned().collect();
    let (px, _) = fetch_mint_prices_usd(&mints).await;
    let mut usd = Decimal::ZERO;
    for (m, amt) in by_mint_ui {
        let p = px.get(&m).copied().unwrap_or(0.0);
        if p > 0.0 && p.is_finite() {
            let pd = Decimal::from_f64_retain(p).unwrap_or(Decimal::ZERO);
            usd += amt * pd;
        }
    }
    (events, usd)
}

async fn lp_fees_collected_usd_from_ledger_db(
    state: &AppState,
    db: &Database,
    position_pubkey: &str,
) -> Result<(u32, Decimal), ApiError> {
    let rows = sqlx::query(
        r#"
        SELECT fee_payer_token_deltas, pool_pubkey
        FROM position_stream_ledger_rows
        WHERE position_pubkey = $1 AND event = 'bot_collect_fees'
        "#,
    )
    .bind(position_pubkey)
    .fetch_all(db.pool())
    .await
    .map_err(|e| ApiError::internal(format!("stream lineage: collect fee rows: {e}")))?;

    let mut pool_mints: HashMap<String, (String, String)> = HashMap::new();
    let mut by_mint_ui: BTreeMap<String, Decimal> = BTreeMap::new();
    let events: u32 = rows.len() as u32;

    for r in rows {
        let v: Option<serde_json::Value> = r.try_get("fee_payer_token_deltas").ok().flatten();
        let pool: Option<String> = r.try_get("pool_pubkey").ok();
        let Some(obj) = v.as_ref().and_then(|x| x.as_object()) else {
            continue;
        };
        let Some(pool) = pool.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
            continue;
        };
        let (ma, mb) = match pool_mints.get(pool) {
            Some(p) => (p.0.clone(), p.1.clone()),
            None => {
                let Ok(ps) = clmm_lp_protocols::prelude::WhirlpoolReader::new(state.provider.clone())
                    .get_pool_state(pool)
                    .await
                else {
                    continue;
                };
                let pair = (ps.token_mint_a.to_string(), ps.token_mint_b.to_string());
                pool_mints.insert(pool.to_string(), pair.clone());
                pair
            }
        };
        for m in [ma, mb] {
            if let Some(dv) = obj.get(&m) {
                if let Some(d) = dec_from_any(dv) {
                    if d > Decimal::ZERO {
                        *by_mint_ui.entry(m).or_insert(Decimal::ZERO) += d;
                    }
                }
            }
        }
    }

    if by_mint_ui.is_empty() {
        return Ok((events, Decimal::ZERO));
    }

    let mints: BTreeSet<String> = by_mint_ui.keys().cloned().collect();
    let (px, _) = fetch_mint_prices_usd(&mints).await;
    let mut usd = Decimal::ZERO;
    for (m, amt) in by_mint_ui {
        let p = px.get(&m).copied().unwrap_or(0.0);
        if p > 0.0 && p.is_finite() {
            let pd = Decimal::from_f64_retain(p).unwrap_or(Decimal::ZERO);
            usd += amt * pd;
        }
    }
    Ok((events, usd))
}

fn rollup_lineage_chain_costs(nodes: &[PositionStreamLineageNode]) -> Option<LineageChainCostSummary> {
    if nodes.is_empty() {
        return None;
    }
    Some(LineageChainCostSummary {
        tx_fee_lamports_total: nodes.iter().map(|n| n.tx_fee_lamports).sum(),
        tx_fees_usd_total: nodes.iter().map(|n| n.tx_fees_usd).sum(),
        fees_collected_usd_total: nodes.iter().map(|n| n.fees_collected_usd).sum(),
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
    let (px, src) = fetch_mint_prices_usd(&mints).await;
    (px.get(WSOL_MINT).copied().unwrap_or(0.0), src)
}

fn ui_amount(raw: u64, decimals: u8) -> f64 {
    if decimals == 0 {
        return raw as f64;
    }
    let denom = 10f64.powi(i32::from(decimals));
    (raw as f64) / denom
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
) -> Result<PositionStreamLineageNode, ApiError> {
    let Some(db) = state.db.as_ref() else {
        return node_metrics_from_lifecycle_best_effort(state, position_pubkey).await;
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

    let opened_ts: Option<DateTime<Utc>> = baseline
        .as_ref()
        .and_then(|r| r.try_get::<Option<DateTime<Utc>>, _>("ts_utc").ok())
        .flatten();
    let closed_ts: Option<DateTime<Utc>> = current
        .as_ref()
        .and_then(|r| r.try_get::<Option<DateTime<Utc>>, _>("ts_utc").ok())
        .flatten();

    let baseline_value: Decimal = baseline
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

    let mint_a: Option<String> = baseline
        .as_ref()
        .and_then(|r| r.try_get::<Option<String>, _>("token_mint_a").ok())
        .flatten()
        .filter(|s| !s.trim().is_empty());
    let mint_b: Option<String> = baseline
        .as_ref()
        .and_then(|r| r.try_get::<Option<String>, _>("token_mint_b").ok())
        .flatten()
        .filter(|s| !s.trim().is_empty());

    let realized_cashflow_usd = if let (Some(a), Some(b)) = (mint_a.clone(), mint_b.clone()) {
        let mut mints: BTreeSet<String> = BTreeSet::new();
        mints.insert(a.clone());
        mints.insert(b.clone());
        let (px, _src) = fetch_mint_prices_usd(&mints).await;
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

    let (collect_events, fees_collected_usd) =
        lp_fees_collected_usd_from_ledger_db(state, db, position_pubkey).await?;

    let net_pnl_usd = current_value + realized_cashflow_usd - baseline_value - tx_fees_usd;
    let net_pnl_pct = if baseline_value.is_zero() {
        Decimal::ZERO
    } else {
        net_pnl_usd / baseline_value
    };

    Ok(PositionStreamLineageNode {
        position_address: position_pubkey.to_string(),
        opened_ts_utc: opened_ts.map(|t| t.to_rfc3339()),
        closed_ts_utc: closed_ts.map(|t| t.to_rfc3339()),
        baseline_value_usd: baseline_value,
        current_value_usd: current_value,
        tx_fee_lamports: fee_lamports_u,
        tx_fees_usd,
        fees_collected_usd,
        collect_events,
        realized_cashflow_usd,
        net_pnl_usd,
        net_pnl_pct,
        note: Some(format!(
            "Best-effort per-PDA. tx_fee_lamports = sum of network fees for this PDA; fees_collected_usd = bot_collect_fees legs × USD; tx fees use SOL/USD ({sol_src}). cashflow uses fee_payer_token_deltas × current mint USD prices when baseline mints are known."
        )),
    })
}

async fn node_metrics_from_lifecycle_best_effort(
    state: &AppState,
    position_pubkey: &str,
) -> Result<PositionStreamLineageNode, ApiError> {
    let rows = parse_lifecycle_rows_best_effort();
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

    for r in &rows {
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

    // If we didn't find mints from details, resolve from pool via free RPC (best-effort).
    if (mint_a.is_none() || mint_b.is_none()) && pool_address.is_some() {
        let pool = pool_address.clone().unwrap_or_default();
        if !pool.trim().is_empty() {
            if let Ok(ps) = clmm_lp_protocols::prelude::WhirlpoolReader::new(state.provider.clone())
                .get_pool_state(&pool)
                .await
            {
                mint_a.get_or_insert(ps.token_mint_a.to_string());
                mint_b.get_or_insert(ps.token_mint_b.to_string());
            }
        }
    }

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
            let (px, src) = fetch_mint_prices_usd(&mints).await;
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

    // Current value: best-effort from chain (only works for open positions).
    let mut current_value_usd = if let Ok(pk) = solana_sdk::pubkey::Pubkey::from_str(position_pubkey) {
        if let Ok(pos) = monitored_position_from_chain(state.provider.clone(), &pk).await {
            let prices = fetch_prices_for_positions(state.provider.clone(), std::slice::from_ref(&pos)).await;
            compute_position_usd_valuation(state.provider.clone(), &pos, &prices)
                .await
                .map(|v| v.value_usd)
                .unwrap_or(Decimal::ZERO)
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
                        let (px, _src) = fetch_mint_prices_usd(&mints).await;
                        let pa = px.get(&a).copied().unwrap_or(0.0);
                        let pb = px.get(&b).copied().unwrap_or(0.0);
                        if pa.is_finite() && pb.is_finite() && pa > 0.0 && pb > 0.0 {
                            let cap_value_usd_f =
                                ui_amount(cap_a, da) * pa + ui_amount(cap_b, db) * pb;
                            let cap_value_usd =
                                Decimal::from_f64_retain(cap_value_usd_f).unwrap_or(Decimal::ZERO);
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

    let (collect_events, fees_collected_usd) =
        lp_fees_collected_usd_from_lifecycle_rows(state, &rows, position_pubkey).await;

    let net_pnl_usd = current_value_usd + realized_cashflow_usd - baseline_value_usd - tx_fees_usd;
    let net_pnl_pct = if baseline_value_usd.is_zero() {
        Decimal::ZERO
    } else {
        net_pnl_usd / baseline_value_usd
    };

    Ok(PositionStreamLineageNode {
        position_address: position_pubkey.to_string(),
        opened_ts_utc: opened_ts.map(|t| t.to_rfc3339()),
        closed_ts_utc: closed_ts.map(|t| t.to_rfc3339()),
        baseline_value_usd,
        current_value_usd,
        tx_fee_lamports: tx_fee_lamports_sum,
        tx_fees_usd,
        fees_collected_usd,
        collect_events,
        realized_cashflow_usd,
        net_pnl_usd,
        net_pnl_pct,
        note: Some(format!(
            "DB is disabled; per-node metrics from lifecycle JSONL. tx_fee_lamports = network fees for this PDA; fees_collected_usd = bot_collect_fees × USD; tx fees use SOL/USD ({sol_src}). start/end value derived from open/close token deltas × current mint USD prices ({}).",
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
    let perf = compute_position_stream_performance(state, entry).await?;
    let mut totals = compute_position_stream_pnl(state, entry).await.ok();

    let Some(db) = state.db.as_ref() else {
        let chain = chain_from_lifecycle_best_effort(entry, 25);
        let mut nodes = Vec::new();
        for p in &chain {
            nodes.push(node_metrics(state, p).await?);
        }

        // UX helper: when a closed node has no reliable "end value" (close row missing leg token deltas),
        // we can approximate it by the next node's baseline (capital rolled into the next open).
        // This makes the table less confusing in JSONL-only mode.
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

        // In DB-disabled mode, `stream-pnl` often returns a "zeros + note" placeholder.
        // Prefer a consistent totals row derived from lineage nodes.
        let totals_is_placeholder = totals.as_ref().is_some_and(|t| {
            t.baseline_value_usd.is_zero()
                && t.current_value_usd.is_zero()
                && t.tx_fees_usd.is_zero()
                && t.realized_cashflow_usd.is_zero()
                && t.net_pnl_usd.is_zero()
                && t.note
                    .as_deref()
                    .unwrap_or_default()
                    .to_ascii_lowercase()
                    .contains("db is disabled")
        });
        if (totals.is_none() || totals_is_placeholder) && !nodes.is_empty() {
            let baseline_value_usd = nodes
                .first()
                .map(|n| n.baseline_value_usd)
                .unwrap_or(Decimal::ZERO);
            let current_value_usd = nodes
                .last()
                .map(|n| n.current_value_usd)
                .unwrap_or(Decimal::ZERO);
            let tx_fees_usd: Decimal = nodes.iter().map(|n| n.tx_fees_usd).sum();
            let realized_cashflow_usd: Decimal =
                nodes.iter().map(|n| n.realized_cashflow_usd).sum();
            let net_pnl_usd =
                current_value_usd + realized_cashflow_usd - baseline_value_usd - tx_fees_usd;
            let net_pnl_pct = if baseline_value_usd.is_zero() {
                Decimal::ZERO
            } else {
                net_pnl_usd / baseline_value_usd
            };

            totals = Some(crate::models::PositionStreamPnLResponse {
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
                note: Some(
                    "DB is disabled; totals computed best-effort from lineage nodes (baseline=first open outflow, current=last node end value/current mark; IL/HODL unavailable).".to_string(),
                ),
            });
        }
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
        let lc = chain_from_lifecycle_best_effort(entry, 25);
        if lc.len() > chain.len() {
            chain = lc;
        }
    }

    // Per-node metrics.
    let mut nodes: Vec<PositionStreamLineageNode> = Vec::new();
    for p in &chain {
        nodes.push(node_metrics(state, p).await?);
    }

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

