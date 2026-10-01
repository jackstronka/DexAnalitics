//! Portfel łańcucha: backfill `chain_session_id`, registry status, history, lineage reconcile.

use crate::error::ApiError;
use crate::models::{
    WalletChainCollectedFeesSummary, WalletChainLineageReconcile,
    WalletChainPortfolioHistoryResponse, WalletChainPortfolioHistoryRow,
    WalletChainPortfolioLedgerEvent, WalletChainPortfolioLedgerLeg,
    WalletChainSessionIdBackfillReport, WalletChainSessionMeta, WalletSessionBalanceUsdLeg,
    WalletSessionMetrics, WalletSessionOpenStartSnapshot,
};
use crate::services::position_stream_performance::compute_position_stream_performance;
use crate::services::position_stream_pnl::compute_position_stream_pnl;
use crate::state::AppState;
use chrono::{DateTime, Utc};
use clmm_lp_data::repositories::Database;
use clmm_lp_data::wallet_session::{
    self, ensure_chain_session_id_on_lifecycle_row, is_lifecycle_open_event,
};
use rust_decimal::Decimal;
use serde_json::Value;
use sqlx::Row;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::str::FromStr;
use std::time::Duration;
use tokio::time::timeout;
use uuid::Uuid;

const WSOL_MINT: &str = "So11111111111111111111111111111111111111112";

/// Max time for optional lineage reconcile on `GET /wallets/chain-portfolio` (must stay below API request timeout).
pub fn chain_lineage_reconcile_timeout_secs() -> u64 {
    std::env::var("CLMM_CHAIN_PORTFOLIO_RECONCILE_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .filter(|&n| (3..=25).contains(&n))
        .unwrap_or(8)
}

/// Assign one `chain_session_id` to all PSLR rows in the BFS component around `anchor_position`.
pub async fn backfill_chain_session_ids_for_position(
    state: &AppState,
    anchor_position: &str,
    chain_session_id: Option<String>,
) -> Result<WalletChainSessionIdBackfillReport, ApiError> {
    let anchor = anchor_position.trim();
    if anchor.is_empty() {
        return Err(ApiError::bad_request("anchor_position is required"));
    }
    let Some(db) = state.db.as_ref() else {
        return Err(ApiError::internal("database not connected"));
    };

    let perf = compute_position_stream_performance(state, anchor, false).await?;
    let positions = perf.positions;
    let sessions = perf.sessions;
    if positions.is_empty() {
        return Err(ApiError::not_found(format!(
            "no stream component for position {anchor}"
        )));
    }

    let chain_session_id = chain_session_id
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let head = positions
        .last()
        .cloned()
        .unwrap_or_else(|| anchor.to_string());
    sqlx::query(
        r#"
        INSERT INTO chain_session_registry (chain_session_id, anchor_position, head_position, status)
        VALUES ($1, $2, $3, 'active')
        ON CONFLICT (chain_session_id) DO UPDATE SET
            anchor_position = COALESCE(chain_session_registry.anchor_position, EXCLUDED.anchor_position),
            head_position = EXCLUDED.head_position
        "#,
    )
    .bind(&chain_session_id)
    .bind(anchor)
    .bind(&head)
    .execute(db.pool())
    .await
    .map_err(|e| ApiError::internal(format!("chain_session_registry upsert: {e}")))?;

    let updated = if !sessions.is_empty() {
        sqlx::query(
            r#"
            UPDATE position_stream_ledger_rows
            SET chain_session_id = $1,
                raw_json = jsonb_set(
                    COALESCE(raw_json, '{}'::jsonb),
                    '{chain_session_id}',
                    to_jsonb($1::text),
                    true
                )
            WHERE (
                position_pubkey = ANY($2)
                OR rebalance_session_id = ANY($3)
            )
            AND (chain_session_id IS NULL OR TRIM(chain_session_id) = '' OR chain_session_id = $1)
            "#,
        )
        .bind(&chain_session_id)
        .bind(&positions)
        .bind(&sessions)
        .execute(db.pool())
        .await
        .map_err(|e| ApiError::internal(format!("pslr chain_session_id update: {e}")))?
        .rows_affected()
    } else {
        sqlx::query(
            r#"
            UPDATE position_stream_ledger_rows
            SET chain_session_id = $1,
                raw_json = jsonb_set(
                    COALESCE(raw_json, '{}'::jsonb),
                    '{chain_session_id}',
                    to_jsonb($1::text),
                    true
                )
            WHERE position_pubkey = ANY($2)
              AND (chain_session_id IS NULL OR TRIM(chain_session_id) = '' OR chain_session_id = $1)
            "#,
        )
        .bind(&chain_session_id)
        .bind(&positions)
        .execute(db.pool())
        .await
        .map_err(|e| ApiError::internal(format!("pslr chain_session_id update: {e}")))?
        .rows_affected()
    };

    Ok(WalletChainSessionIdBackfillReport {
        chain_session_id,
        anchor_position: anchor.to_string(),
        chain_pda_count: positions.len().min(u32::MAX as usize) as u32,
        rebalance_sessions_linked: sessions.len().min(u32::MAX as usize) as u32,
        pslr_rows_updated: updated.min(u32::MAX as u64) as u32,
    })
}

/// Resolve `chain_session_id` for a position from registry or PSLR (best-effort).
pub async fn resolve_chain_session_id_for_position(
    state: &AppState,
    position: &str,
) -> Option<String> {
    let pos = position.trim();
    if pos.is_empty() {
        return None;
    }
    if let Some(db) = state.db.as_ref() {
        if let Ok(row) = sqlx::query(
            r#"
            SELECT chain_session_id
            FROM chain_session_registry
            WHERE anchor_position = $1 OR head_position = $1
            ORDER BY created_at DESC
            LIMIT 1
            "#,
        )
        .bind(pos)
        .fetch_optional(db.pool())
        .await
            && let Some(r) = row
        {
            let cid: String = r.try_get("chain_session_id").unwrap_or_default();
            let t = cid.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
        if let Ok(row) = sqlx::query(
            r#"
            SELECT chain_session_id
            FROM position_stream_ledger_rows
            WHERE position_pubkey = $1
              AND chain_session_id IS NOT NULL
              AND TRIM(chain_session_id) <> ''
            ORDER BY ts_utc DESC NULLS LAST
            LIMIT 1
            "#,
        )
        .bind(pos)
        .fetch_optional(db.pool())
        .await
            && let Some(r) = row
        {
            let cid: String = r.try_get("chain_session_id").unwrap_or_default();
            let t = cid.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    None
}

fn parse_usd_decimal(s: Option<&str>) -> Option<Decimal> {
    let t = s?.trim();
    if t.is_empty() {
        return None;
    }
    Decimal::from_str(t).ok()
}

fn fmt_usd_decimal(d: Decimal) -> String {
    format!("{:.8}", d)
}

/// Registry row for one `chain_session_id` (best-effort).
pub async fn fetch_chain_session_meta(
    state: &AppState,
    chain_session_id: &str,
    anchor_hint: Option<&str>,
) -> Result<Option<WalletChainSessionMeta>, ApiError> {
    let cid = chain_session_id.trim();
    if cid.is_empty() {
        return Ok(None);
    }
    let Some(db) = state.db.as_ref() else {
        return Ok(None);
    };
    let row = sqlx::query(
        r#"
        SELECT status, closed_at, anchor_position, head_position
        FROM chain_session_registry
        WHERE chain_session_id = $1
        "#,
    )
    .bind(cid)
    .fetch_optional(db.pool())
    .await
    .map_err(|e| ApiError::internal(format!("chain_session_registry read: {e}")))?;
    let meta = if let Some(r) = row {
        let status: String = r.try_get("status").unwrap_or_else(|_| "active".to_string());
        let closed_at: Option<DateTime<Utc>> = r.try_get("closed_at").ok().flatten();
        let anchor_position: Option<String> = r.try_get("anchor_position").ok().flatten();
        let head_position: Option<String> = r.try_get("head_position").ok().flatten();
        WalletChainSessionMeta {
            status: status.trim().to_string(),
            closed_at: closed_at.map(|t| t.to_rfc3339()),
            anchor_position,
            head_position,
            chain_pda_count: None,
        }
    } else {
        WalletChainSessionMeta {
            status: "active".to_string(),
            closed_at: None,
            anchor_position: anchor_hint
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            head_position: None,
            chain_pda_count: None,
        }
    };
    Ok(Some(meta))
}

/// After operator manual close: freeze chain cycle when the closed PDA is the chain head.
pub async fn mark_chain_session_closed_after_manual_close(
    state: &AppState,
    closed_position: &str,
) -> Result<(), ApiError> {
    let pos = closed_position.trim();
    if pos.is_empty() {
        return Ok(());
    }
    let Some(chain_session_id) = resolve_chain_session_id_for_position(state, pos).await else {
        return Ok(());
    };
    let Some(db) = state.db.as_ref() else {
        return Ok(());
    };
    let registry = sqlx::query(
        r#"
        SELECT status, head_position, anchor_position
        FROM chain_session_registry
        WHERE chain_session_id = $1
        "#,
    )
    .bind(&chain_session_id)
    .fetch_optional(db.pool())
    .await
    .map_err(|e| ApiError::internal(format!("chain_session_registry read: {e}")))?;
    let Some(reg) = registry else {
        return Ok(());
    };
    let status: String = reg
        .try_get("status")
        .unwrap_or_else(|_| "active".to_string());
    if status.trim().eq_ignore_ascii_case("closed") {
        return Ok(());
    }
    let head: Option<String> = reg.try_get("head_position").ok().flatten();
    let anchor: Option<String> = reg.try_get("anchor_position").ok().flatten();
    let probe = anchor.as_deref().or(head.as_deref()).unwrap_or(pos);
    let is_head = match compute_position_stream_performance(state, probe, false).await {
        Ok(perf) => perf.positions.last().is_some_and(|last| last.trim() == pos),
        Err(_) => head.as_deref().map(str::trim) == Some(pos),
    };
    if !is_head {
        return Ok(());
    }
    sqlx::query(
        r#"
        UPDATE chain_session_registry
        SET status = 'closed',
            closed_at = COALESCE(closed_at, NOW()),
            head_position = $2
        WHERE chain_session_id = $1
          AND status <> 'closed'
        "#,
    )
    .bind(&chain_session_id)
    .bind(pos)
    .execute(db.pool())
    .await
    .map_err(|e| ApiError::internal(format!("chain_session_registry close: {e}")))?;
    Ok(())
}

fn summarize_history_row(event: &str, raw: &Value, tx_fee_lamports: Option<i64>) -> Option<String> {
    let details = raw.get("details").and_then(|d| d.as_object());
    let mut parts: Vec<String> = Vec::new();
    if let Some(d) = details {
        if let Some(a) = d.get("open_amount_a_raw").and_then(|x| x.as_u64()) {
            parts.push(format!("open_a={a}"));
        }
        if let Some(b) = d.get("open_amount_b_raw").and_then(|x| x.as_u64()) {
            parts.push(format!("open_b={b}"));
        }
        if let Some(a) = d.get("close_amount_a_raw").and_then(|x| x.as_u64()) {
            parts.push(format!("close_a={a}"));
        }
        if let Some(b) = d.get("close_amount_b_raw").and_then(|x| x.as_u64()) {
            parts.push(format!("close_b={b}"));
        }
    }
    if let Some(lamports) = tx_fee_lamports.filter(|n| *n > 0) {
        parts.push(format!("tx_fee_lamports={lamports}"));
    }
    if parts.is_empty() {
        if event.is_empty() {
            None
        } else {
            Some(event.to_string())
        }
    } else {
        Some(format!("{event}: {}", parts.join(", ")))
    }
}

/// Ordered lifecycle timeline for one chain (PSLR rows with matching `chain_session_id`).
pub async fn fetch_chain_portfolio_history(
    state: &AppState,
    chain_session_id: &str,
    limit: usize,
) -> Result<WalletChainPortfolioHistoryResponse, ApiError> {
    let cid = chain_session_id.trim();
    if cid.is_empty() {
        return Err(ApiError::bad_request("chain_session_id is required"));
    }
    let limit = limit.clamp(1, 2000);
    let Some(db) = state.db.as_ref() else {
        return Err(ApiError::internal("database not connected"));
    };
    let meta = fetch_chain_session_meta(state, cid, None).await?;
    let rows = sqlx::query(
        r#"
        SELECT ts_utc, event, signature, position_pubkey, rebalance_session_id,
               tx_fee_lamports, raw_json
        FROM position_stream_ledger_rows
        WHERE chain_session_id = $1
        ORDER BY ts_utc ASC NULLS LAST, signature ASC NULLS LAST
        LIMIT $2
        "#,
    )
    .bind(cid)
    .bind(limit as i64)
    .fetch_all(db.pool())
    .await
    .map_err(|e| ApiError::internal(format!("chain history read: {e}")))?;

    let history_rows: Vec<WalletChainPortfolioHistoryRow> = rows
        .iter()
        .map(|r| {
            let ts: Option<DateTime<Utc>> = r.try_get("ts_utc").ok().flatten();
            let event: String = r.try_get("event").unwrap_or_default();
            let signature: Option<String> = r.try_get("signature").ok().flatten();
            let position_pubkey: Option<String> = r.try_get("position_pubkey").ok().flatten();
            let rebalance_session_id: Option<String> =
                r.try_get("rebalance_session_id").ok().flatten();
            let tx_fee_lamports: Option<i64> = r.try_get("tx_fee_lamports").ok().flatten();
            let raw: Value = r.get("raw_json");
            WalletChainPortfolioHistoryRow {
                ts_utc: ts.map(|t| t.to_rfc3339()),
                event: event.trim().to_string(),
                signature,
                position_pubkey,
                rebalance_session_id,
                tx_fee_lamports,
                summary: summarize_history_row(event.trim(), &raw, tx_fee_lamports),
            }
        })
        .collect();
    let row_count = history_rows.len().min(u32::MAX as usize) as u32;
    Ok(WalletChainPortfolioHistoryResponse {
        chain_session_id: cid.to_string(),
        status: meta
            .as_ref()
            .map(|m| m.status.clone())
            .unwrap_or_else(|| "active".to_string()),
        closed_at: meta.and_then(|m| m.closed_at),
        rows: history_rows,
        row_count,
    })
}

fn mint_decimals_ledger(mint: &str) -> u8 {
    let m = mint.trim();
    if m == WSOL_MINT || m == "So11111111111111111111111111111111111111111" {
        9
    } else {
        6
    }
}

fn fmt_ledger_usd(v: f64) -> String {
    format!("{:.8}", v)
}

fn price_by_mint_from_open_start(
    open_start: &WalletSessionOpenStartSnapshot,
) -> BTreeMap<String, f64> {
    open_start
        .price_by_mint_usd
        .iter()
        .filter_map(|(mint, px)| {
            px.trim()
                .parse::<f64>()
                .ok()
                .filter(|p| p.is_finite() && *p > 0.0)
                .map(|p| (mint.trim().to_string(), p))
        })
        .collect()
}

/// Cycle-start USD marks, overridden by newer event-time spots accumulated in order.
fn merged_ledger_prices(row: &Value, spot_cache: &BTreeMap<String, f64>) -> BTreeMap<String, f64> {
    let mut out = spot_cache.clone();
    for (mint, px) in wallet_session::lifecycle_price_by_mint(row) {
        out.insert(mint, px);
    }
    out
}

fn apply_prices_to_ledger_events(
    events: &mut [WalletChainPortfolioLedgerEvent],
    prices: &BTreeMap<String, f64>,
) {
    for evt in events.iter_mut() {
        for leg in evt.legs.iter_mut() {
            if leg.value_usd.is_some() {
                continue;
            }
            let Ok(amount) = leg.amount_raw.trim().parse::<i128>() else {
                continue;
            };
            leg.value_usd = leg_value_usd_opt(&leg.mint, amount, prices);
        }
        evt.total_usd = sum_leg_usd(&evt.legs);
    }
}

async fn seed_wsol_spot_if_missing(running: &mut BTreeMap<String, f64>) {
    if sol_price_from_map(running).is_some() {
        return;
    }
    let (sol, _) = crate::services::price_fetch::fetch_sol_usd_best_effort().await;
    if sol.is_finite() && sol > 0.0 {
        running.insert(WSOL_MINT.to_string(), sol);
    }
}

async fn enrich_ledger_events_usd_from_feed(
    events: &mut [WalletChainPortfolioLedgerEvent],
    running: &mut BTreeMap<String, f64>,
) {
    seed_wsol_spot_if_missing(running).await;

    let mut need = BTreeSet::new();
    for evt in events.iter() {
        for leg in &evt.legs {
            if leg.value_usd.is_none() {
                need.insert(leg.mint.trim().to_string());
            }
        }
    }
    need.insert(WSOL_MINT.to_string());

    apply_prices_to_ledger_events(events, running);

    if need.iter().all(|m| price_for_mint(m, running).is_some()) {
        return;
    }

    let fetched = match timeout(
        Duration::from_secs(5),
        crate::services::price_fetch::fetch_mint_prices_usd(&need),
    )
    .await
    {
        Ok((px, _)) => px,
        Err(_) => BTreeMap::new(),
    };
    for (mint, px) in fetched {
        if px.is_finite() && px > 0.0 {
            running.insert(mint, px);
        }
    }
    seed_wsol_spot_if_missing(running).await;
    apply_prices_to_ledger_events(events, running);
}

/// Per-mint CHAIN wallet legs with USD from cycle spot map (non-zero balances only).
pub fn chain_balance_usd_legs_from_balances(
    balances: &[crate::models::WalletSessionBalanceRow],
    spot_prices: &BTreeMap<String, f64>,
) -> Vec<crate::models::WalletSessionBalanceUsdLeg> {
    let mut legs: Vec<crate::models::WalletSessionBalanceUsdLeg> = balances
        .iter()
        .filter_map(|b| {
            let Ok(amount) = b.amount_raw.trim().parse::<i128>() else {
                return None;
            };
            if amount == 0 {
                return None;
            }
            let price = price_for_mint(&b.mint, spot_prices);
            Some(crate::models::WalletSessionBalanceUsdLeg {
                mint: b.mint.clone(),
                amount_raw: b.amount_raw.clone(),
                price_usd: price.map(fmt_ledger_usd),
                value_usd: leg_value_usd_opt(&b.mint, amount, spot_prices),
            })
        })
        .collect();
    legs.sort_by(|a, b| {
        let va = a
            .value_usd
            .as_deref()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);
        let vb = b
            .value_usd
            .as_deref()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);
        vb.partial_cmp(&va).unwrap_or(std::cmp::Ordering::Equal)
    });
    legs
}

/// Keep only balances for strategy pool/swap mints (+ WSOL); returns excluded row count.
pub fn filter_strategy_wallet_balances(
    balances: &[crate::models::WalletSessionBalanceRow],
    strategy_mints: &BTreeSet<String>,
) -> (Vec<crate::models::WalletSessionBalanceRow>, u32) {
    if strategy_mints.is_empty() {
        return (balances.to_vec(), 0);
    }
    let filtered: Vec<_> = balances
        .iter()
        .filter(|b| strategy_mints.contains(b.mint.trim()))
        .cloned()
        .collect();
    let excluded = balances.len().saturating_sub(filtered.len()) as u32;
    (filtered, excluded)
}

pub fn filter_usd_legs_to_strategy_mints(
    legs: &[crate::models::WalletSessionBalanceUsdLeg],
    strategy_mints: &BTreeSet<String>,
) -> Vec<crate::models::WalletSessionBalanceUsdLeg> {
    if strategy_mints.is_empty() {
        return legs.to_vec();
    }
    legs.iter()
        .filter(|l| strategy_mints.contains(l.mint.trim()))
        .cloned()
        .collect()
}

/// USD map for CHAIN wallet display: open-start prices when metrics trusted, else ledger spot.
pub fn chain_wallet_display_prices(
    metrics: Option<&WalletSessionMetrics>,
    spot_fallback: &BTreeMap<String, f64>,
) -> BTreeMap<String, f64> {
    if let Some(m) = metrics.filter(|m| m.metrics_trusted) {
        let from_open = price_by_mint_from_open_start(&m.open_start);
        if !from_open.is_empty() {
            return from_open;
        }
    }
    spot_fallback.clone()
}

fn parse_u64_json_value(v: &Value) -> Option<u64> {
    v.as_u64()
        .or_else(|| v.as_i64().filter(|&n| n >= 0).map(|n| n as u64))
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

fn push_lp_collected_fee_deltas(
    out: &mut BTreeMap<String, i128>,
    raw: &Value,
    lp_a: Option<i64>,
    lp_b: Option<i64>,
) {
    let details = raw.get("details").and_then(|d| d.as_object());
    let (mint_a, mint_b, _) = wallet_session::pool_mints_from_lifecycle_row(raw, details);
    let lp_a = lp_a
        .or_else(|| {
            raw.get("lp_collected_token_a_raw")
                .and_then(parse_u64_json_value)
                .map(|n| n as i64)
        })
        .or_else(|| {
            details
                .and_then(|d| d.get("lp_collected_token_a_raw"))
                .and_then(parse_u64_json_value)
                .map(|n| n as i64)
        });
    let lp_b = lp_b
        .or_else(|| {
            raw.get("lp_collected_token_b_raw")
                .and_then(parse_u64_json_value)
                .map(|n| n as i64)
        })
        .or_else(|| {
            details
                .and_then(|d| d.get("lp_collected_token_b_raw"))
                .and_then(parse_u64_json_value)
                .map(|n| n as i64)
        });
    if let (Some(ma), Some(a)) = (mint_a.as_ref(), lp_a.filter(|&x| x > 0)) {
        *out.entry(ma.clone()).or_insert(0) += a as i128;
    }
    if let (Some(mb), Some(b)) = (mint_b.as_ref(), lp_b.filter(|&x| x > 0)) {
        *out.entry(mb.clone()).or_insert(0) += b as i128;
    }
}

/// Σ LP fees for one chain cycle: explicit collect rows + `lp_collected_*` on close (no principal).
pub fn aggregate_chain_collected_fees(
    agg: &[(Value, Option<i64>, Option<i64>)],
    spot_prices: &BTreeMap<String, f64>,
) -> WalletChainCollectedFeesSummary {
    let mut by_mint: BTreeMap<String, i128> = BTreeMap::new();
    let mut collect_events = 0u32;
    for (raw, lp_a, lp_b) in agg {
        let event = raw
            .get("event")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if event == "bot_collect_fees" {
            collect_events += 1;
            if let Some((_, _, _, postings)) =
                wallet_session::chain_mint_deltas_from_lifecycle_json(raw, *lp_a, *lp_b)
            {
                for (mint, delta) in postings {
                    if delta > 0 {
                        *by_mint.entry(mint).or_insert(0) += delta;
                    }
                }
            }
        } else if matches!(event, "bot_close_position" | "position_close") {
            push_lp_collected_fee_deltas(&mut by_mint, raw, *lp_a, *lp_b);
        }
    }
    let mut legs: Vec<WalletSessionBalanceUsdLeg> = by_mint
        .into_iter()
        .filter(|(_, amount)| *amount != 0)
        .map(|(mint, amount)| WalletSessionBalanceUsdLeg {
            mint: mint.clone(),
            amount_raw: amount.to_string(),
            price_usd: price_for_mint(&mint, spot_prices).map(fmt_ledger_usd),
            value_usd: leg_value_usd_opt(&mint, amount, spot_prices),
        })
        .collect();
    legs.sort_by(|a, b| {
        let va = a
            .value_usd
            .as_deref()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);
        let vb = b
            .value_usd
            .as_deref()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);
        vb.partial_cmp(&va).unwrap_or(std::cmp::Ordering::Equal)
    });
    let total_usd = {
        let mut total = 0.0f64;
        let mut any = false;
        for leg in &legs {
            if let Some(v) = leg.value_usd.as_deref().and_then(|s| s.parse::<f64>().ok()) {
                total += v;
                any = true;
            }
        }
        any.then(|| fmt_ledger_usd(total))
    };
    WalletChainCollectedFeesSummary {
        collect_events,
        legs,
        total_usd,
    }
}

/// CHAIN GL wallet total USD using cycle spot prices (start + accumulated event marks + feed).
pub fn portfolio_balance_usd_from_balances(
    balances: &[crate::models::WalletSessionBalanceRow],
    spot_prices: &BTreeMap<String, f64>,
) -> Option<String> {
    let mut total = 0.0f64;
    let mut any = false;
    for b in balances {
        let Ok(amount) = b.amount_raw.trim().parse::<i128>() else {
            continue;
        };
        if amount == 0 {
            continue;
        }
        let Some(v) = leg_value_usd_opt(&b.mint, amount, spot_prices)
            .and_then(|s| s.trim().parse::<f64>().ok())
        else {
            continue;
        };
        total += v;
        any = true;
    }
    any.then(|| fmt_ledger_usd(total))
}

fn leg_value_usd_opt(
    mint: &str,
    amount_raw: i128,
    prices: &BTreeMap<String, f64>,
) -> Option<String> {
    let price = price_for_mint(mint, prices)?;
    let dec = mint_decimals_ledger(mint);
    let ui = (amount_raw.unsigned_abs() as f64) / 10f64.powi(i32::from(dec));
    Some(fmt_ledger_usd(ui * price))
}

fn price_for_mint(mint: &str, prices: &BTreeMap<String, f64>) -> Option<f64> {
    let m = mint.trim();
    prices
        .get(m)
        .copied()
        .filter(|p| p.is_finite() && *p > 0.0)
        .or_else(|| {
            if m == WSOL_MINT || m == "So11111111111111111111111111111111111111111" {
                sol_price_from_map(prices)
            } else {
                None
            }
        })
}

fn sum_leg_usd(legs: &[WalletChainPortfolioLedgerLeg]) -> Option<String> {
    let mut total = 0.0f64;
    let mut any = false;
    for leg in legs {
        if let Some(v) = leg
            .value_usd
            .as_deref()
            .and_then(|s| s.trim().parse::<f64>().ok())
        {
            total += v;
            any = true;
        }
    }
    any.then(|| fmt_ledger_usd(total))
}

fn ledger_event_kind(event: &str) -> &'static str {
    if is_lifecycle_open_event(event) {
        "open_to_pool"
    } else if matches!(event, "bot_close_position" | "position_close") {
        "close_from_pool"
    } else if event == "bot_collect_fees" {
        "collect_fees"
    } else if matches!(
        event,
        "cli_swap" | "bot_swap_exact_in" | "bot_swap" | "bot_orca_tx"
    ) {
        "swap"
    } else {
        "other"
    }
}

fn sol_price_from_map(prices: &BTreeMap<String, f64>) -> Option<f64> {
    prices.get(WSOL_MINT).copied().or_else(|| {
        prices
            .get("So11111111111111111111111111111111111111111")
            .copied()
    })
}

fn tx_fee_ledger_event(
    ts_utc: Option<String>,
    signature: Option<String>,
    position_pubkey: Option<String>,
    tx_fee_lamports: i64,
    prices: &BTreeMap<String, f64>,
) -> WalletChainPortfolioLedgerEvent {
    let amount_raw = tx_fee_lamports.to_string();
    let value_usd = sol_price_from_map(prices).map(|p| {
        let ui = (tx_fee_lamports as f64) / 1e9;
        fmt_ledger_usd(ui * p)
    });
    let leg = WalletChainPortfolioLedgerLeg {
        mint: WSOL_MINT.to_string(),
        amount_raw,
        direction: "out".to_string(),
        value_usd: value_usd.clone(),
    };
    WalletChainPortfolioLedgerEvent {
        ts_utc,
        kind: "tx_fee".to_string(),
        event: "tx_fee".to_string(),
        signature,
        position_pubkey,
        legs: vec![leg],
        total_usd: value_usd,
    }
}

#[allow(clippy::too_many_arguments)]
fn ledger_event_from_lifecycle_row(
    ts_utc: Option<String>,
    event: &str,
    signature: Option<String>,
    position_pubkey: Option<String>,
    raw: &Value,
    lp_a: Option<i64>,
    lp_b: Option<i64>,
    prices: &BTreeMap<String, f64>,
) -> Option<WalletChainPortfolioLedgerEvent> {
    let (_, _, ev, postings) =
        wallet_session::chain_mint_deltas_from_lifecycle_json(raw, lp_a, lp_b)?;
    let legs: Vec<WalletChainPortfolioLedgerLeg> = postings
        .into_iter()
        .filter(|(_, delta)| *delta != 0)
        .map(|(mint, delta)| {
            let direction = if delta > 0 { "in" } else { "out" };
            WalletChainPortfolioLedgerLeg {
                mint: mint.clone(),
                amount_raw: delta.unsigned_abs().to_string(),
                direction: direction.to_string(),
                value_usd: leg_value_usd_opt(&mint, delta, prices),
            }
        })
        .collect();
    if legs.is_empty() {
        return None;
    }
    Some(WalletChainPortfolioLedgerEvent {
        ts_utc,
        kind: ledger_event_kind(&ev).to_string(),
        event: if event.trim().is_empty() {
            ev
        } else {
            event.trim().to_string()
        },
        signature,
        position_pubkey,
        total_usd: sum_leg_usd(&legs),
        legs,
    })
}

/// Pre-open chain wallet snapshot at first open (synthetic first ledger row).
pub fn ledger_start_event_from_open_start(
    open_start: &WalletSessionOpenStartSnapshot,
) -> Option<WalletChainPortfolioLedgerEvent> {
    let prices = price_by_mint_from_open_start(open_start);
    let legs: Vec<WalletChainPortfolioLedgerLeg> = open_start
        .pre_open_balances
        .iter()
        .filter_map(|b| {
            let amount = b.amount_raw.trim().parse::<i128>().ok()?;
            if amount == 0 {
                return None;
            }
            Some(WalletChainPortfolioLedgerLeg {
                mint: b.mint.clone(),
                amount_raw: amount.unsigned_abs().to_string(),
                direction: "in".to_string(),
                value_usd: leg_value_usd_opt(&b.mint, amount, &prices),
            })
        })
        .collect();
    if legs.is_empty() {
        return None;
    }
    Some(WalletChainPortfolioLedgerEvent {
        ts_utc: open_start.ts_utc.clone(),
        kind: "portfolio_start".to_string(),
        event: "chain_portfolio_start".to_string(),
        signature: if open_start.signature.trim().is_empty() {
            None
        } else {
            Some(open_start.signature.clone())
        },
        position_pubkey: open_start.position_pubkey.clone(),
        total_usd: open_start
            .pre_open_value_usd
            .clone()
            .or_else(|| open_start.value_usd.clone())
            .or_else(|| sum_leg_usd(&legs)),
        legs,
    })
}

/// Chronological CHAIN wallet journal from PSLR lifecycle rows.
/// Returns `(events, spot_prices, collected_fees, strategy_mints)`.
pub async fn build_chain_portfolio_ledger(
    db: &Database,
    chain_session_id: &str,
) -> Result<
    (
        Vec<WalletChainPortfolioLedgerEvent>,
        BTreeMap<String, f64>,
        WalletChainCollectedFeesSummary,
        BTreeSet<String>,
    ),
    sqlx::Error,
> {
    let cid = chain_session_id.trim();
    if cid.is_empty() {
        return Ok((
            vec![],
            BTreeMap::new(),
            WalletChainCollectedFeesSummary::default(),
            BTreeSet::new(),
        ));
    }
    let rows = sqlx::query(
        r#"
        SELECT ts_utc, event, signature, position_pubkey, tx_fee_lamports,
               raw_json, lp_collected_token_a_raw, lp_collected_token_b_raw
        FROM position_stream_ledger_rows
        WHERE chain_session_id = $1
        ORDER BY ts_utc ASC NULLS LAST, signature ASC NULLS LAST
        LIMIT 2000
        "#,
    )
    .bind(cid)
    .fetch_all(db.pool())
    .await?;

    let agg: Vec<(Value, Option<i64>, Option<i64>)> = rows
        .iter()
        .map(|r| {
            let raw: Value = ensure_chain_session_id_on_lifecycle_row(r.get("raw_json"), cid);
            let lp_a: Option<i64> = r.try_get("lp_collected_token_a_raw").ok().flatten();
            let lp_b: Option<i64> = r.try_get("lp_collected_token_b_raw").ok().flatten();
            (raw, lp_a, lp_b)
        })
        .collect();

    let start_prices =
        wallet_session::compute_chain_open_start_from_lifecycle_rows(agg.iter().cloned(), cid)
            .map(|snap| snap.price_by_mint)
            .unwrap_or_default();
    let boot = wallet_session::bootstrap_chain_spot_prices_from_rows(agg.iter().cloned(), cid);
    let mut running = start_prices.clone();
    for (mint, px) in boot {
        running.entry(mint).or_insert(px);
    }
    seed_wsol_spot_if_missing(&mut running).await;

    let mut events = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        let ts: Option<DateTime<Utc>> = r.try_get("ts_utc").ok().flatten();
        let ts_utc = ts.map(|t| t.to_rfc3339());
        let event: String = r.try_get("event").unwrap_or_default();
        let signature: Option<String> = r.try_get("signature").ok().flatten();
        let position_pubkey: Option<String> = r.try_get("position_pubkey").ok().flatten();
        let tx_fee_lamports: Option<i64> = r.try_get("tx_fee_lamports").ok().flatten();
        let (raw, lp_a, lp_b) = &agg[i];
        for (mint, px) in wallet_session::lifecycle_price_by_mint(raw) {
            running.insert(mint, px);
        }
        let prices = merged_ledger_prices(raw, &running);

        if let Some(evt) = ledger_event_from_lifecycle_row(
            ts_utc.clone(),
            event.trim(),
            signature.clone(),
            position_pubkey.clone(),
            raw,
            *lp_a,
            *lp_b,
            &prices,
        ) {
            events.push(evt);
        }
        if let Some(lamports) = tx_fee_lamports.filter(|n| *n > 0) {
            events.push(tx_fee_ledger_event(
                ts_utc,
                signature,
                position_pubkey,
                lamports,
                &prices,
            ));
        }
    }

    enrich_ledger_events_usd_from_feed(&mut events, &mut running).await;
    let collected_fees = aggregate_chain_collected_fees(&agg, &running);
    let strategy_mints = wallet_session::chain_strategy_wallet_mints_from_agg(agg.clone(), cid);
    Ok((events, running, collected_fees, strategy_mints))
}

/// Best-effort lineage reconcile; returns `None` on timeout or error (does not fail the hot path).
pub async fn compute_chain_lineage_reconcile_best_effort(
    state: &AppState,
    chain_session_id: &str,
    anchor_position: &str,
    metrics: Option<&WalletSessionMetrics>,
    lp_nav_usd: Option<&str>,
) -> Option<WalletChainLineageReconcile> {
    let timeout_secs = chain_lineage_reconcile_timeout_secs();
    let deadline = Duration::from_secs(timeout_secs);
    match timeout(
        deadline,
        compute_chain_lineage_reconcile(
            state,
            chain_session_id,
            anchor_position,
            metrics,
            lp_nav_usd,
        ),
    )
    .await
    {
        Ok(Ok(Some(reconcile))) => Some(reconcile),
        Ok(Ok(None)) => None,
        Ok(Err(e)) => {
            tracing::warn!(
                error = %e,
                chain_session_id = %chain_session_id.trim(),
                anchor_position = %anchor_position.trim(),
                "chain lineage reconcile failed (omitting reconcile block)"
            );
            None
        }
        Err(_) => {
            tracing::warn!(
                timeout_secs,
                chain_session_id = %chain_session_id.trim(),
                anchor_position = %anchor_position.trim(),
                "chain lineage reconcile timed out (omitting reconcile block)"
            );
            None
        }
    }
}

/// Read-only diff: chain wallet metrics vs stream-lineage net PnL (methods differ by design).
pub async fn compute_chain_lineage_reconcile(
    state: &AppState,
    chain_session_id: &str,
    anchor_position: &str,
    metrics: Option<&WalletSessionMetrics>,
    lp_nav_usd: Option<&str>,
) -> Result<Option<WalletChainLineageReconcile>, ApiError> {
    let anchor = anchor_position.trim();
    if anchor.is_empty() {
        return Ok(None);
    }
    let lineage = compute_position_stream_pnl(state, anchor).await?;
    let chain_start = metrics.and_then(|m| parse_usd_decimal(m.open_start.value_usd.as_deref()));
    let chain_wallet = metrics.and_then(|m| parse_usd_decimal(m.current_value_usd.as_deref()));
    let lp_nav = parse_usd_decimal(lp_nav_usd);
    let chain_combined = match (chain_wallet, lp_nav) {
        (Some(w), Some(nav)) => Some(w + nav),
        (Some(w), None) => Some(w),
        (None, Some(nav)) => Some(nav),
        (None, None) => None,
    };
    let chain_vs_start = chain_start
        .zip(chain_combined)
        .map(|(start, combined)| combined - start);
    let diff = chain_vs_start
        .zip(Some(lineage.net_pnl_usd))
        .map(|(c, l)| c - l);
    let note = "Chain model: GL wallet (+ optional head NAV) minus cycle start at open prices. \
                Lineage: baseline→current NAV + lifecycle cashflow − tx fees. \
                Expect small gaps from price basis, phantom SESSION debits, and uncollected LP."
        .to_string();
    Ok(Some(WalletChainLineageReconcile {
        chain_session_id: chain_session_id.trim().to_string(),
        chain_start_usd: chain_start.map(fmt_usd_decimal),
        chain_wallet_usd: chain_wallet.map(fmt_usd_decimal),
        chain_lp_nav_usd: lp_nav.map(fmt_usd_decimal),
        chain_combined_usd: chain_combined.map(fmt_usd_decimal),
        chain_vs_start_usd: chain_vs_start.map(fmt_usd_decimal),
        lineage_net_pnl_usd: Some(fmt_usd_decimal(lineage.net_pnl_usd)),
        lineage_baseline_usd: Some(fmt_usd_decimal(lineage.baseline_value_usd)),
        lineage_current_usd: Some(fmt_usd_decimal(lineage.current_value_usd)),
        diff_chain_vs_lineage_usd: diff.map(fmt_usd_decimal),
        note,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn summarize_history_row_includes_open_amounts() {
        let raw = json!({
            "event": "bot_open_position",
            "details": { "open_amount_a_raw": 1000, "open_amount_b_raw": 2000 }
        });
        let s = summarize_history_row("bot_open_position", &raw, Some(5000)).expect("summary");
        assert!(s.contains("open_a=1000"));
        assert!(s.contains("tx_fee_lamports=5000"));
    }

    #[test]
    fn chain_lineage_reconcile_timeout_default_below_api_cap() {
        let secs = chain_lineage_reconcile_timeout_secs();
        assert!((3..=25).contains(&secs));
    }

    #[test]
    fn ledger_event_kind_maps_open_close_collect() {
        assert_eq!(ledger_event_kind("bot_open_position"), "open_to_pool");
        assert_eq!(ledger_event_kind("bot_close_position"), "close_from_pool");
        assert_eq!(ledger_event_kind("bot_collect_fees"), "collect_fees");
        assert_eq!(ledger_event_kind("bot_swap"), "swap");
    }

    #[test]
    fn ledger_start_from_open_start_builds_legs() {
        let snap = WalletSessionOpenStartSnapshot {
            ts_utc: Some("2026-01-01T00:00:00Z".to_string()),
            signature: "sig".to_string(),
            position_pubkey: Some("pos".to_string()),
            event: "bot_open_position".to_string(),
            deployed_balances: vec![],
            value_usd: None,
            value_usd_source: "test".to_string(),
            pre_open_balances: vec![crate::models::WalletSessionBalanceRow {
                mint: WSOL_MINT.to_string(),
                amount_raw: "1000000000".to_string(),
                decimals: None,
            }],
            pre_open_value_usd: Some("150.00000000".to_string()),
            mint_resolution: "details".to_string(),
            price_by_mint_usd: BTreeMap::from([(WSOL_MINT.to_string(), "150".to_string())]),
        };
        let evt = ledger_start_event_from_open_start(&snap).expect("start row");
        assert_eq!(evt.kind, "portfolio_start");
        assert_eq!(evt.legs.len(), 1);
        assert_eq!(evt.legs[0].direction, "in");
    }

    #[test]
    fn merged_ledger_prices_falls_back_to_cycle_start() {
        let start = BTreeMap::from([(WSOL_MINT.to_string(), 150.0)]);
        let raw = json!({ "event": "bot_swap", "details": {} });
        let merged = merged_ledger_prices(&raw, &start);
        assert_eq!(merged.get(WSOL_MINT).copied(), Some(150.0));
    }

    #[test]
    fn merged_ledger_prices_accumulates_row_spot() {
        let cache = BTreeMap::from([(WSOL_MINT.to_string(), 100.0)]);
        let raw = json!({
            "event": "bot_close_position",
            "details": {
                "token_mint_a": WSOL_MINT,
                "token_mint_b": "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
                "event_price_a_usd": 155.5,
                "event_price_b_usd": 1.0
            }
        });
        let merged = merged_ledger_prices(&raw, &cache);
        assert_eq!(merged.get(WSOL_MINT).copied(), Some(155.5));
    }

    #[test]
    fn tx_fee_ledger_uses_start_sol_price_when_row_has_no_event_price() {
        let start = BTreeMap::from([(WSOL_MINT.to_string(), 200.0)]);
        let raw = json!({ "event": "bot_collect_fees", "details": {} });
        let prices = merged_ledger_prices(&raw, &start);
        let evt = tx_fee_ledger_event(None, None, None, 5_000_000, &prices);
        assert_eq!(evt.legs[0].value_usd.as_deref(), Some("1.00000000"));
    }

    #[test]
    fn filter_strategy_wallet_balances_drops_phantom_mints() {
        let strategy: BTreeSet<String> = BTreeSet::from([
            WSOL_MINT.to_string(),
            "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string(),
        ]);
        let balances = vec![
            crate::models::WalletSessionBalanceRow {
                mint: WSOL_MINT.to_string(),
                amount_raw: "1000".to_string(),
                decimals: None,
            },
            crate::models::WalletSessionBalanceRow {
                mint: "phantom-meme".to_string(),
                amount_raw: "1000000000".to_string(),
                decimals: None,
            },
        ];
        let (filtered, excluded) = filter_strategy_wallet_balances(&balances, &strategy);
        assert_eq!(filtered.len(), 1);
        assert_eq!(excluded, 1);
    }

    #[test]
    fn aggregate_chain_collected_fees_sums_collect_rows() {
        let raw = json!({
            "event": "bot_collect_fees",
            "chain_session_id": "test-chain",
            "signature": "sig-collect-1",
            "details": {
                "token_mint_a": WSOL_MINT,
                "token_mint_b": "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
            }
        });
        let agg = vec![(raw, Some(1_000_000i64), Some(2_000_000i64))];
        let spot = BTreeMap::from([
            (WSOL_MINT.to_string(), 100.0),
            (
                "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string(),
                1.0,
            ),
        ]);
        let summary = aggregate_chain_collected_fees(&agg, &spot);
        assert_eq!(summary.collect_events, 1);
        assert_eq!(summary.legs.len(), 2);
        assert_eq!(summary.total_usd.as_deref(), Some("2.10000000"));
    }

    #[test]
    fn chain_balance_usd_legs_from_balances_lists_mints() {
        let balances = vec![
            crate::models::WalletSessionBalanceRow {
                mint: WSOL_MINT.to_string(),
                amount_raw: "1000000000".to_string(),
                decimals: None,
            },
            crate::models::WalletSessionBalanceRow {
                mint: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string(),
                amount_raw: "0".to_string(),
                decimals: None,
            },
        ];
        let spot = BTreeMap::from([(WSOL_MINT.to_string(), 150.0)]);
        let legs = chain_balance_usd_legs_from_balances(&balances, &spot);
        assert_eq!(legs.len(), 1);
        assert_eq!(legs[0].value_usd.as_deref(), Some("150.00000000"));
    }

    #[test]
    fn portfolio_balance_usd_from_balances_sums_spot() {
        let balances = vec![crate::models::WalletSessionBalanceRow {
            mint: WSOL_MINT.to_string(),
            amount_raw: "1000000000".to_string(),
            decimals: None,
        }];
        let spot = BTreeMap::from([(WSOL_MINT.to_string(), 150.0)]);
        let usd = portfolio_balance_usd_from_balances(&balances, &spot).expect("total");
        assert_eq!(usd, "150.00000000");
    }
}
