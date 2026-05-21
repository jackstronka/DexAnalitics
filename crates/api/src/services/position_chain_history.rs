//! Persisted read-model for position rotation chain (Postgres `position_chain_history_*`).
//!
//! Writer reuses `compute_position_stream_lineage_opts` (with **`await_valuation_snapshot_persist`**) then stores rows for fast `GET …/chain-history`.

use crate::error::{ApiError, ApiResult};
use crate::models::{
    LineageChainCostSummary, MaterializeChainHistoryResponse, PositionStreamLineageNode,
    PositionStreamLineageResponse, PositionStreamPnLResponse,
};
use crate::services::position_stream_performance::compute_position_stream_performance;
use crate::services::position_stream_lineage::{
    ComputePositionStreamLineageOpts,
    apply_open_start_usd_from_lifecycle_snapshots_for_chain_history,
    compute_position_stream_lineage_opts, enrich_chain_history_nodes_open_quote_baseline_lift,
    node_metrics, prefer_lifecycle_lineage_if_extends_db_prefix,
    refresh_chain_history_node_fees_from_ledger, refresh_lineage_totals_from_nodes,
    apply_tx_fees_usd_from_lamports_on_nodes, resolve_lineage_chain_for_stream_pnl,
    rollup_lineage_chain_costs, sol_usd_for_tx_fees,
};
use futures::future::join_all;
use std::collections::HashMap;
use crate::services::position_stream_pnl::compute_position_stream_pnl_settlement_v1;
use crate::state::{ApiConfig, AppState};
use axum::http::{HeaderMap, header::AUTHORIZATION};
use chrono::{DateTime, Utc};
use clmm_lp_data::repositories::Database;
use rust_decimal::Decimal;
use serde_json::Value as JsonValue;
use solana_sdk::pubkey::Pubkey;
use sqlx::{PgPool, Row};
use std::str::FromStr;
use tracing::{info, warn};

const METRICS_LIVE: &str = "live";
const METRICS_SETTLEMENT_V1: &str = "settlement_v1";

#[must_use]
pub fn metrics_mode_label(is_settlement_v1: bool) -> &'static str {
    if is_settlement_v1 {
        METRICS_SETTLEMENT_V1
    } else {
        METRICS_LIVE
    }
}

fn parse_ts(s: &Option<String>) -> Option<DateTime<Utc>> {
    let t = s.as_ref()?.trim();
    if t.is_empty() {
        return None;
    }
    DateTime::parse_from_rfc3339(t)
        .map(|d| d.with_timezone(&Utc))
        .ok()
}

fn require_db(state: &AppState) -> ApiResult<&Database> {
    state
        .db
        .as_ref()
        .ok_or_else(|| {
            ApiError::service_unavailable(
                "Postgres is not connected (chain-history requires DATABASE_URL and a running PostgreSQL instance; check API logs for connect/migrate errors, then restart the API process with .env loaded).",
            )
        })
}

fn chain_history_decimal_column_string_positive(v: Option<Decimal>) -> Option<String> {
    v.filter(|d| *d > Decimal::ZERO)
        .map(|d| d.round_dp(12).normalize().to_string())
}

#[derive(Debug, Clone, Default)]
struct ChainHistoryLedgerAux {
    pool_address: Option<String>,
    tick_lower_open: Option<i32>,
    tick_upper_open: Option<i32>,
    range_label_at_open: Option<String>,
    /// Open lifecycle row: `details.event_price_a_usd`.
    event_price_a_usd: Option<Decimal>,
    /// Close lifecycle row: `details.event_price_a_usd` (same field as open; stored in SQL `event_price_b_usd`).
    event_price_close_a_usd: Option<Decimal>,
}

fn ch_json_f64_positive_price(v: Option<&JsonValue>) -> Option<f64> {
    v.and_then(|x| {
        x.as_f64()
            .or_else(|| x.as_i64().map(|i| i as f64))
            .or_else(|| x.as_str().and_then(|s| s.parse().ok()))
    })
    .filter(|p| p.is_finite() && *p > 0.0)
}

fn ch_decimal_event_price_a_from_details(details: &JsonValue) -> Option<Decimal> {
    let obj = details.as_object()?;
    ch_json_f64_positive_price(obj.get("event_price_a_usd"))
        .and_then(Decimal::from_f64_retain)
        .filter(|d| *d > Decimal::ZERO)
}

fn ch_ticks_open_from_details(details: &JsonValue) -> Option<(i32, i32)> {
    let obj = details.as_object()?;
    for (lk, uk) in [
        ("tick_lower", "tick_upper"),
        ("new_tick_lower", "new_tick_upper"),
    ] {
        let Some(lo_v) = obj.get(lk) else {
            continue;
        };
        let Some(hi_v) = obj.get(uk) else {
            continue;
        };
        let lo = lo_v
            .as_i64()
            .or_else(|| lo_v.as_f64().map(|f| f as i64))?;
        let hi = hi_v
            .as_i64()
            .or_else(|| hi_v.as_f64().map(|f| f as i64))?;
        let lo_i = i32::try_from(lo).ok()?;
        let hi_i = i32::try_from(hi).ok()?;
        if lo_i < hi_i {
            return Some((lo_i, hi_i));
        }
    }
    None
}

fn ch_details_from_ledger_raw(raw: &JsonValue) -> Option<&JsonValue> {
    raw.get("details").filter(|d| !d.is_null())
}

/// Best-effort: pool from valuation snapshots / ledger; ticks + event spots from earliest open / latest close rows.
async fn fetch_chain_history_ledger_aux_best_effort(pool: &PgPool, position_pubkey: &str) -> ChainHistoryLedgerAux {
    let mut out = ChainHistoryLedgerAux::default();
    let pos = position_pubkey.trim();
    if pos.is_empty() {
        return out;
    }

    if let Ok(r) = sqlx::query_scalar::<_, Option<String>>(
        r#"SELECT pool_pubkey FROM position_stream_valuation_snapshots
           WHERE position_pubkey = $1 AND pool_pubkey IS NOT NULL AND TRIM(pool_pubkey) <> ''
           ORDER BY ts_utc DESC NULLS LAST
           LIMIT 1"#,
    )
    .bind(pos)
    .fetch_optional(pool)
    .await
        && let Some(Some(s)) = r {
            let t = s.trim();
            if !t.is_empty() {
                out.pool_address = Some(t.to_string());
            }
        }

    if out.pool_address.is_none()
        && let Ok(r) = sqlx::query_scalar::<_, Option<String>>(
            r#"SELECT pool_pubkey FROM position_stream_ledger_rows
               WHERE position_pubkey = $1 AND pool_pubkey IS NOT NULL AND TRIM(pool_pubkey) <> ''
               ORDER BY ts_utc DESC NULLS LAST
               LIMIT 1"#,
        )
        .bind(pos)
        .fetch_optional(pool)
        .await
            && let Some(Some(s)) = r {
                let t = s.trim();
                if !t.is_empty() {
                    out.pool_address = Some(t.to_string());
                }
            }

    if let Ok(Some(raw)) = sqlx::query_scalar::<_, JsonValue>(
        r#"SELECT raw_json FROM position_stream_ledger_rows
           WHERE position_pubkey = $1
             AND event IN ('bot_open_position','bot_open_position_full_range','position_open')
           ORDER BY ts_utc ASC NULLS LAST
           LIMIT 1"#,
    )
    .bind(pos)
    .fetch_optional(pool)
    .await
    {
        let tick_src = ch_details_from_ledger_raw(&raw).unwrap_or(&raw);
        if let Some((lo, hi)) = ch_ticks_open_from_details(tick_src) {
            out.tick_lower_open = Some(lo);
            out.tick_upper_open = Some(hi);
            out.range_label_at_open = Some(format!("{lo}→{hi} ticks"));
        }
        if let Some(d) = ch_details_from_ledger_raw(&raw)
            && let Some(px) = ch_decimal_event_price_a_from_details(d) {
                out.event_price_a_usd = Some(px);
            }
    }

    if let Ok(Some(raw)) = sqlx::query_scalar::<_, JsonValue>(
        r#"SELECT raw_json FROM position_stream_ledger_rows
           WHERE position_pubkey = $1
             AND event IN ('bot_close_position','position_close')
           ORDER BY ts_utc DESC NULLS LAST
           LIMIT 1"#,
    )
    .bind(pos)
    .fetch_optional(pool)
    .await
        && let Some(d) = ch_details_from_ledger_raw(&raw)
            && let Some(px) = ch_decimal_event_price_a_from_details(d) {
                out.event_price_close_a_usd = Some(px);
            }

    out
}

/// Meta `chain_json` can be **too short** (e.g. materialize right after reopen stored only `[newest]`).
/// Live `resolve_lineage_chain_for_stream_pnl` returns the full rotation chain. Prefer resolved when
/// it is a strict **prefix extension** of meta, or when meta is a **single tail** of resolved.
fn merge_meta_chain_with_resolved_for_read(meta_chain: Vec<String>, resolved: Vec<String>) -> Vec<String> {
    let m = prefer_lifecycle_lineage_if_extends_db_prefix(meta_chain.clone(), resolved.clone());
    if m.len() < resolved.len()
        && meta_chain.len() == 1
        && resolved.last().map(|s| s.as_str()) == meta_chain.first().map(|s| s.as_str())
    {
        return resolved;
    }
    m
}

/// `raw_snapshot` JSON can disagree with typed `NUMERIC` columns (serde/JSONB edge cases or older writers).
/// Prefer persisted `start_value_usd` / `current_value_usd` when the deserialized node still has zero marks.
fn overlay_chain_history_node_from_persisted_columns(
    node: &mut PositionStreamLineageNode,
    start_value_usd: Option<Decimal>,
    end_value_usd: Option<Decimal>,
    current_value_usd_col: Option<Decimal>,
) {
    if let Some(s) = start_value_usd.filter(|x| *x > Decimal::ZERO)
        && node.baseline_value_usd.is_zero() {
            node.baseline_value_usd = s;
            node.baseline_valuation_quality = Some("exact".to_string());
        }
    if let Some(c) = current_value_usd_col.filter(|x| *x > Decimal::ZERO)
        && node.current_value_usd.is_zero() {
            node.current_value_usd = c;
            node.current_valuation_quality = Some("exact".to_string());
        }
    if node.closed_ts_utc.is_some()
        && let Some(e) = end_value_usd.filter(|x| *x > Decimal::ZERO)
            && node.current_value_usd.is_zero() {
                node.current_value_usd = e;
                node.current_valuation_quality = Some("exact".to_string());
            }
    node.net_pnl_usd = node.current_value_usd + node.realized_cashflow_usd
        - node.baseline_value_usd
        - node.tx_fees_usd;
    if !node.baseline_value_usd.is_zero() {
        node.net_pnl_pct = node.net_pnl_usd / node.baseline_value_usd;
    } else {
        node.net_pnl_pct = Decimal::ZERO;
    }
}

fn overlay_chain_history_node_from_persisted_display_columns(
    node: &mut PositionStreamLineageNode,
    pool_address: Option<String>,
    tick_lower_open: Option<i32>,
    tick_upper_open: Option<i32>,
    event_price_a_usd: Option<Decimal>,
    // Close-event token A spot (materialized into SQL column `event_price_b_usd`).
    event_price_b_usd: Option<Decimal>,
) {
    if let Some(p) = pool_address {
        let t = p.trim();
        if !t.is_empty() {
            node.chain_history_pool_address = Some(t.to_string());
        }
    }
    if let (Some(lo), Some(hi)) = (tick_lower_open, tick_upper_open)
        && lo < hi {
            node.chain_history_tick_lower_open = Some(lo);
            node.chain_history_tick_upper_open = Some(hi);
        }
    if let Some(s) = chain_history_decimal_column_string_positive(event_price_a_usd) {
        node.chain_history_event_spot_token_a_usd_open = Some(s);
    }
    if let Some(s) = chain_history_decimal_column_string_positive(event_price_b_usd) {
        node.chain_history_event_spot_token_a_usd_close = Some(s);
    }
}

const X_CHAIN_HISTORY_REFRESH: &str = "X-Chain-History-Refresh";

fn bearer_token_from_authorization(value: &str) -> Option<&str> {
    let r = value.trim();
    const PREFIX: &str = "Bearer ";
    if r.len() > PREFIX.len() && r[..PREFIX.len()].eq_ignore_ascii_case(PREFIX) {
        Some(r[PREFIX.len()..].trim())
    } else {
        None
    }
}

/// When [`ApiConfig::chain_history_refresh_secret`] is set (non-empty), requires matching token in
/// `Authorization: Bearer …` or `X-Chain-History-Refresh: …`.
pub fn require_chain_history_refresh_auth(
    config: &ApiConfig,
    headers: &HeaderMap,
) -> ApiResult<()> {
    let Some(ref secret) = config.chain_history_refresh_secret else {
        return Ok(());
    };
    let expected = secret.trim();
    if expected.is_empty() {
        return Ok(());
    }

    let from_bearer = headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|raw| bearer_token_from_authorization(raw));

    let from_header = headers
        .get(X_CHAIN_HISTORY_REFRESH)
        .and_then(|v| v.to_str().ok())
        .map(str::trim);

    let token = from_bearer.or(from_header);
    match token {
        Some(t) if t == expected => Ok(()),
        Some(_) => Err(ApiError::unauthorized(
            "invalid chain-history refresh token",
        )),
        None => Err(ApiError::unauthorized(
            "chain-history refresh requires Authorization: Bearer <CLMM_CHAIN_HISTORY_REFRESH_SECRET> or X-Chain-History-Refresh: <secret>",
        )),
    }
}

/// When unset, background chain-history materialization is **on** (if DB is configured).  
/// `CLMM_CHAIN_HISTORY_TRIGGERS` (preferred) or legacy `CLMM_CHAIN_HISTORY_CLOSE_TRIGGER`:  
/// set to `0` / `false` / `off` / `no` to disable all position-handler triggers.
fn chain_history_background_triggers_enabled() -> bool {
    if let Ok(s) = std::env::var("CLMM_CHAIN_HISTORY_TRIGGERS") {
        return env_triggers_truthy(&s);
    }
    if let Ok(s) = std::env::var("CLMM_CHAIN_HISTORY_CLOSE_TRIGGER") {
        return env_triggers_truthy(&s);
    }
    true
}

fn env_triggers_truthy(raw: &str) -> bool {
    let t = raw.trim().to_ascii_lowercase();
    !matches!(t.as_str(), "" | "0" | "false" | "off" | "no")
}

/// After a successful `live` pass, also materialize `settlement_v1` when `CLMM_CHAIN_HISTORY_TRIGGERS_SETTLEMENT_V1`
/// is `1` / `true` / `yes` / `on` (expensive; default off).
fn chain_history_background_settlement_v1_pass_enabled() -> bool {
    std::env::var("CLMM_CHAIN_HISTORY_TRIGGERS_SETTLEMENT_V1")
        .ok()
        .map(|s| {
            let t = s.trim().to_ascii_lowercase();
            matches!(t.as_str(), "1" | "true" | "yes" | "on")
        })
        .unwrap_or(false)
}

/// Best-effort background materialize: `live` always; optional `settlement_v1` second pass (env).  
/// `trigger` is a static label for logs (e.g. `close_position`).
pub fn spawn_chain_history_materialize_background(
    state: &AppState,
    anchor: impl Into<String>,
    trigger: &'static str,
) {
    let anchor = anchor.into();
    if !chain_history_background_triggers_enabled() {
        return;
    }
    if state.db.is_none() {
        return;
    }
    let a = anchor.trim().to_string();
    if a.is_empty() {
        return;
    }
    let st = state.clone();
    tokio::spawn(async move {
        match materialize_chain_history_for_anchor(&st, &a, false).await {
            Ok(r) => {
                info!(
                    anchor = %r.chain_anchor_pubkey,
                    nodes = r.nodes_written,
                    metrics_mode = %r.metrics_mode,
                    ok = r.ok,
                    trigger,
                    "chain-history: background materialize (live)"
                );
            }
            Err(e) => {
                warn!(
                    anchor = %a,
                    trigger,
                    error = %e,
                    "chain-history: background materialize (live) failed (ignored)"
                );
                return;
            }
        }
        if chain_history_background_settlement_v1_pass_enabled() {
            match materialize_chain_history_for_anchor(&st, &a, true).await {
                Ok(r) => {
                    info!(
                        anchor = %r.chain_anchor_pubkey,
                        nodes = r.nodes_written,
                        metrics_mode = %r.metrics_mode,
                        ok = r.ok,
                        trigger,
                        "chain-history: background materialize (settlement_v1)"
                    );
                }
                Err(e) => {
                    warn!(
                        anchor = %a,
                        trigger,
                        error = %e,
                        "chain-history: background materialize (settlement_v1) failed (ignored)"
                    );
                }
            }
        }
    });
}

/// Recompute lineage + stream totals, then replace materialized rows for `(anchor, metrics_mode)`.
pub async fn materialize_chain_history_for_anchor(
    state: &AppState,
    anchor: &str,
    is_settlement_v1: bool,
) -> ApiResult<MaterializeChainHistoryResponse> {
    Pubkey::from_str(anchor.trim())
        .map_err(|_| ApiError::bad_request("Invalid position address"))?;
    let mode = metrics_mode_label(is_settlement_v1);
    let db = require_db(state)?;
    let pool = db.pool();

    let mut resp = compute_position_stream_lineage_opts(
        state,
        anchor.trim(),
        ComputePositionStreamLineageOpts {
            await_valuation_snapshot_persist: true,
        },
    )
    .await?;
    if is_settlement_v1 {
        resp.totals = Some(compute_position_stream_pnl_settlement_v1(state, anchor.trim()).await?);
        let mut note = resp.note.unwrap_or_default();
        if !note.is_empty() {
            note.push(' ');
        }
        note.push_str(
            "Settlement v1 mode: totals are computed from persisted DB snapshots only (no live self-seed).",
        );
        resp.note = Some(note);
    }

    apply_open_start_usd_from_lifecycle_snapshots_for_chain_history(state, &mut resp.nodes).await?;

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::internal(format!("chain-history tx: {e}")))?;

    sqlx::query(
        "DELETE FROM position_chain_history_nodes WHERE chain_anchor_pubkey = $1 AND metrics_mode = $2",
    )
    .bind(anchor.trim())
    .bind(mode)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(format!("chain-history delete nodes: {e}")))?;

    sqlx::query(
        "DELETE FROM position_chain_history_meta WHERE chain_anchor_pubkey = $1 AND metrics_mode = $2",
    )
    .bind(anchor.trim())
    .bind(mode)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(format!("chain-history delete meta: {e}")))?;

    let chain_json =
        serde_json::to_value(&resp.chain).map_err(|e| ApiError::internal(e.to_string()))?;
    let totals_json = resp
        .totals
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let cost_json = resp
        .chain_cost_summary
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .map_err(|e| ApiError::internal(e.to_string()))?;

    sqlx::query(
        r#"INSERT INTO position_chain_history_meta (
            chain_anchor_pubkey, metrics_mode, entry_position_address,
            chain_json, totals_json, chain_cost_summary_json, note
        ) VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
    )
    .bind(anchor.trim())
    .bind(mode)
    .bind(resp.position_address.as_str())
    .bind(chain_json)
    .bind(totals_json)
    .bind(cost_json)
    .bind(resp.note.as_deref())
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(format!("chain-history insert meta: {e}")))?;

    let mut written: u32 = 0;
    for (i, node) in resp.nodes.iter().enumerate() {
        let seq: i16 =
            i16::try_from(i + 1).map_err(|_| ApiError::internal("chain seq overflow"))?;
        let pred = if i == 0 {
            None
        } else {
            resp.chain.get(i - 1).cloned()
        };
        let aux = fetch_chain_history_ledger_aux_best_effort(pool, node.position_address.as_str()).await;
        let mut node_snapshot = node.clone();
        if let Some(ref p) = aux.pool_address {
            node_snapshot.chain_history_pool_address = Some(p.clone());
        }
        if let (Some(lo), Some(hi)) = (aux.tick_lower_open, aux.tick_upper_open) {
            node_snapshot.chain_history_tick_lower_open = Some(lo);
            node_snapshot.chain_history_tick_upper_open = Some(hi);
        }
        if let Some(d) = aux.event_price_a_usd {
            node_snapshot.chain_history_event_spot_token_a_usd_open =
                chain_history_decimal_column_string_positive(Some(d));
        }
        if let Some(d) = aux.event_price_close_a_usd {
            node_snapshot.chain_history_event_spot_token_a_usd_close =
                chain_history_decimal_column_string_positive(Some(d));
        }
        let raw_snapshot = serde_json::to_value(&node_snapshot).map_err(|e| ApiError::internal(e.to_string()))?;
        let principal_delta = node.current_value_usd - node.baseline_value_usd;
        let end_val = if node.closed_ts_utc.is_some() {
            Some(node.current_value_usd)
        } else {
            None
        };

        info!(
            chain_anchor = %anchor.trim(),
            metrics_mode = %mode,
            chain_seq = %seq,
            position_pubkey = %node.position_address,
            baseline_value_usd = %node.baseline_value_usd,
            baseline_valuation_quality = ?node.baseline_valuation_quality,
            current_value_usd = %node.current_value_usd,
            "chain-history materialize: node fields bound before INSERT (start_value_usd <- baseline_value_usd)"
        );

        sqlx::query(
            r#"INSERT INTO position_chain_history_nodes (
                chain_anchor_pubkey, chain_seq, position_pubkey, predecessor_position_pubkey,
                pool_address, opened_ts_utc, closed_ts_utc,
                range_label_at_open, tick_lower_open, tick_upper_open,
                close_price_label, event_price_a_usd, event_price_b_usd,
                start_value_usd, end_value_usd, current_value_usd, principal_delta_usd,
                tx_fee_lamports, tx_fees_usd, collect_events, fees_collected_usd,
                fees_token_a_ui, fees_token_b_ui, fees_token_a_raw, fees_token_b_raw,
                token_mint_a, token_mint_b, realized_cashflow_usd, net_pnl_usd, net_pnl_pct,
                source_version, metrics_mode, raw_snapshot
            ) VALUES (
                $1, $2, $3, $4,
                $5, $6, $7,
                $8, $9, $10,
                NULL, $11, $12,
                $13, $14, $15, $16,
                $17, $18, $19, $20,
                $21, $22, $23, $24,
                $25, $26, $27, $28, $29,
                1, $30, $31
            )"#,
        )
        .bind(anchor.trim())
        .bind(seq)
        .bind(node.position_address.as_str())
        .bind(pred.as_deref())
        .bind(aux.pool_address.as_deref())
        .bind(parse_ts(&node.opened_ts_utc))
        .bind(parse_ts(&node.closed_ts_utc))
        .bind(aux.range_label_at_open.as_deref())
        .bind(aux.tick_lower_open)
        .bind(aux.tick_upper_open)
        .bind(aux.event_price_a_usd)
        .bind(aux.event_price_close_a_usd)
        .bind(node.baseline_value_usd)
        .bind(end_val)
        .bind(node.current_value_usd)
        .bind(principal_delta)
        .bind(i64::try_from(node.tx_fee_lamports).unwrap_or(i64::MAX))
        .bind(node.tx_fees_usd)
        .bind(i32::try_from(node.collect_events).unwrap_or(i32::MAX))
        .bind(node.fees_collected_usd)
        .bind(node.fees_collected_token_a_ui)
        .bind(node.fees_collected_token_b_ui)
        .bind(
            node.fees_collected_token_a_raw
                .map(|v| i64::try_from(v).unwrap_or(i64::MAX)),
        )
        .bind(
            node.fees_collected_token_b_raw
                .map(|v| i64::try_from(v).unwrap_or(i64::MAX)),
        )
        .bind(node.token_mint_a.as_deref())
        .bind(node.token_mint_b.as_deref())
        .bind(node.realized_cashflow_usd)
        .bind(node.net_pnl_usd)
        .bind(node.net_pnl_pct)
        .bind(mode)
        .bind(raw_snapshot)
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(format!("chain-history insert node: {e}")))?;
        written = written.saturating_add(1);
    }

    tx.commit()
        .await
        .map_err(|e| ApiError::internal(format!("chain-history commit: {e}")))?;

    info!(
        anchor = %anchor,
        mode = %mode,
        nodes = written,
        "materialized position_chain_history"
    );

    Ok(MaterializeChainHistoryResponse {
        ok: true,
        chain_anchor_pubkey: anchor.trim().to_string(),
        metrics_mode: mode.to_string(),
        nodes_written: written,
    })
}

/// Resolve which `chain_anchor_pubkey` row in Postgres should satisfy `GET …/chain-history` for the
/// URL path position. Order: direct meta hit → `position_chain_history_nodes.position_pubkey` →
/// `position_chain_history_meta.entry_position_address` → `chain_json @> [pda]` (same `metrics_mode`).
async fn resolve_chain_history_anchor_for_read(
    pool: &PgPool,
    requested: &str,
    mode: &str,
) -> ApiResult<Option<String>> {
    let direct = sqlx::query_scalar::<_, i32>(
        r#"SELECT 1 FROM position_chain_history_meta
           WHERE chain_anchor_pubkey = $1 AND metrics_mode = $2"#,
    )
    .bind(requested)
    .bind(mode)
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::internal(format!("chain-history meta probe: {e}")))?;
    if direct.is_some() {
        return Ok(Some(requested.to_string()));
    }
    if let Some(root) = sqlx::query_scalar::<_, String>(
        r#"SELECT chain_anchor_pubkey FROM position_chain_history_nodes
           WHERE position_pubkey = $1 AND metrics_mode = $2
           LIMIT 1"#,
    )
    .bind(requested)
    .bind(mode)
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::internal(format!("chain-history anchor resolve (nodes): {e}")))? {
        return Ok(Some(root));
    }
    if let Some(root) = sqlx::query_scalar::<_, String>(
        r#"SELECT chain_anchor_pubkey FROM position_chain_history_meta
           WHERE entry_position_address = $1 AND metrics_mode = $2
           LIMIT 1"#,
    )
    .bind(requested)
    .bind(mode)
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::internal(format!("chain-history anchor resolve (meta.entry): {e}")))? {
        return Ok(Some(root));
    }
    let from_chain = sqlx::query_scalar::<_, String>(
        r#"SELECT m.chain_anchor_pubkey
           FROM position_chain_history_meta m
           WHERE m.metrics_mode = $2
             AND m.chain_json @> jsonb_build_array($1::text)
           LIMIT 1"#,
    )
    .bind(requested)
    .bind(mode)
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::internal(format!("chain-history anchor resolve (meta.chain_json): {e}")))?;
    Ok(from_chain)
}

/// Read materialized chain; returns `None` if no meta row exists.
pub async fn load_chain_history_from_db(
    state: &AppState,
    anchor: &str,
    is_settlement_v1: bool,
) -> ApiResult<Option<PositionStreamLineageResponse>> {
    Pubkey::from_str(anchor.trim())
        .map_err(|_| ApiError::bad_request("Invalid position address"))?;
    let mode = metrics_mode_label(is_settlement_v1);
    let db = require_db(state)?;
    let pool = db.pool();

    let requested = anchor.trim().to_string();
    let effective_anchor =
        if let Some(a) = resolve_chain_history_anchor_for_read(pool, &requested, mode).await? {
            a
        } else {
            // Persisted `chain_json` / `nodes` can lag behind live lineage (new rotation tail not
            // written yet). Walk the resolved stream chain and reuse the first PDA that already has
            // materialized rows (same `metrics_mode`).
            let perf = compute_position_stream_performance(state, requested.as_str(), true)
                .await
                .ok();
            let mut found: Option<String> = None;
            if let Some(p) = perf {
                let resolved =
                    resolve_lineage_chain_for_stream_pnl(state, &p, requested.as_str()).await;
                for cand in resolved {
                    let c = cand.trim();
                    if c.is_empty() {
                        continue;
                    }
                    if let Some(a) = resolve_chain_history_anchor_for_read(pool, c, mode).await? {
                        found = Some(a);
                        break;
                    }
                }
            }
            match found {
                Some(a) => a,
                None => return Ok(None),
            }
        };
    let remapped = effective_anchor != requested;

    let meta = sqlx::query(
        r#"SELECT entry_position_address, chain_json, totals_json, chain_cost_summary_json, note,
                  materialized_ts_utc
           FROM position_chain_history_meta
           WHERE chain_anchor_pubkey = $1 AND metrics_mode = $2"#,
    )
    .bind(&effective_anchor)
    .bind(mode)
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::internal(format!("chain-history meta select: {e}")))?;

    let Some(meta_row) = meta else {
        return Ok(None);
    };

    let entry: String = meta_row.try_get("entry_position_address").map_err(|e| {
        ApiError::internal(format!(
            "chain-history meta decode entry_position_address: {e}"
        ))
    })?;
    let chain_v = meta_row
        .try_get::<serde_json::Value, _>("chain_json")
        .map_err(|e| ApiError::internal(format!("chain-history meta decode chain_json: {e}")))?;
    let meta_chain: Vec<String> = serde_json::from_value(chain_v)
        .map_err(|e| ApiError::internal(format!("chain-history meta chain_json parse: {e}")))?;

    let totals_json: Option<serde_json::Value> = meta_row
        .try_get::<Option<serde_json::Value>, _>("totals_json")
        .map_err(|e| ApiError::internal(format!("chain-history meta totals_json: {e}")))?;
    let totals: Option<PositionStreamPnLResponse> = totals_json
        .filter(|v| !v.is_null())
        .map(serde_json::from_value)
        .transpose()
        .map_err(|e| ApiError::internal(format!("chain-history meta totals_json parse: {e}")))?;

    let cost_json: Option<serde_json::Value> = meta_row
        .try_get::<Option<serde_json::Value>, _>("chain_cost_summary_json")
        .map_err(|e| {
            ApiError::internal(format!("chain-history meta chain_cost_summary_json: {e}"))
        })?;
    let chain_cost_summary_from_meta: Option<LineageChainCostSummary> = cost_json
        .filter(|v| !v.is_null())
        .map(serde_json::from_value)
        .transpose()
        .map_err(|e| {
            ApiError::internal(format!(
                "chain-history meta chain_cost_summary_json parse: {e}"
            ))
        })?;

    let note: Option<String> = meta_row
        .try_get::<Option<String>, _>("note")
        .map_err(|e| ApiError::internal(format!("chain-history meta note: {e}")))?;

    let materialized_ts_utc: DateTime<Utc> = meta_row
        .try_get("materialized_ts_utc")
        .map_err(|e| ApiError::internal(format!("chain-history meta materialized_ts_utc: {e}")))?;
    let materialized_ts_rfc3339 =
        Some(materialized_ts_utc.to_rfc3339_opts(chrono::SecondsFormat::Millis, true));

    let node_rows = sqlx::query(
        r#"SELECT start_value_usd, end_value_usd, current_value_usd,
                  pool_address, tick_lower_open, tick_upper_open,
                  event_price_a_usd, event_price_b_usd,
                  raw_snapshot
           FROM position_chain_history_nodes
           WHERE chain_anchor_pubkey = $1 AND metrics_mode = $2
           ORDER BY chain_seq ASC"#,
    )
    .bind(&effective_anchor)
    .bind(mode)
    .fetch_all(pool)
    .await
    .map_err(|e| ApiError::internal(format!("chain-history nodes select: {e}")))?;

    let mut node_by_pos: HashMap<String, PositionStreamLineageNode> =
        HashMap::with_capacity(node_rows.len());
    for r in node_rows {
        let start_v: Option<Decimal> = r
            .try_get("start_value_usd")
            .map_err(|e| ApiError::internal(format!("chain-history node start_value_usd: {e}")))?;
        let end_v: Option<Decimal> = r
            .try_get("end_value_usd")
            .map_err(|e| ApiError::internal(format!("chain-history node end_value_usd: {e}")))?;
        let cur_v: Option<Decimal> = r.try_get("current_value_usd").map_err(|e| {
            ApiError::internal(format!("chain-history node current_value_usd: {e}"))
        })?;
        let pool_address: Option<String> = r
            .try_get::<Option<String>, _>("pool_address")
            .map_err(|e| ApiError::internal(format!("chain-history node pool_address: {e}")))?;
        let tick_lower_open: Option<i32> = r
            .try_get::<Option<i32>, _>("tick_lower_open")
            .map_err(|e| ApiError::internal(format!("chain-history node tick_lower_open: {e}")))?;
        let tick_upper_open: Option<i32> = r
            .try_get::<Option<i32>, _>("tick_upper_open")
            .map_err(|e| ApiError::internal(format!("chain-history node tick_upper_open: {e}")))?;
        let event_price_a_usd: Option<Decimal> = r
            .try_get::<Option<Decimal>, _>("event_price_a_usd")
            .map_err(|e| ApiError::internal(format!("chain-history node event_price_a_usd: {e}")))?;
        let event_price_b_usd: Option<Decimal> = r
            .try_get::<Option<Decimal>, _>("event_price_b_usd")
            .map_err(|e| ApiError::internal(format!("chain-history node event_price_b_usd: {e}")))?;
        let snap = r
            .try_get::<serde_json::Value, _>("raw_snapshot")
            .map_err(|e| ApiError::internal(format!("chain-history node raw_snapshot: {e}")))?;
        let mut node: PositionStreamLineageNode = serde_json::from_value(snap).map_err(|e| {
            ApiError::internal(format!("chain-history node raw_snapshot decode: {e}"))
        })?;
        node.chain_history_start_value_usd = chain_history_decimal_column_string_positive(start_v);
        node.chain_history_end_value_usd = chain_history_decimal_column_string_positive(end_v);
        node.chain_history_current_value_usd = chain_history_decimal_column_string_positive(cur_v);
        overlay_chain_history_node_from_persisted_columns(&mut node, start_v, end_v, cur_v);
        overlay_chain_history_node_from_persisted_display_columns(
            &mut node,
            pool_address,
            tick_lower_open,
            tick_upper_open,
            event_price_a_usd,
            event_price_b_usd,
        );
        node_by_pos.insert(node.position_address.clone(), node);
    }

    let mut chain = meta_chain.clone();
    if let Ok(perf) = compute_position_stream_performance(state, effective_anchor.as_str(), true).await {
        let resolved = resolve_lineage_chain_for_stream_pnl(
            state,
            &perf,
            effective_anchor.as_str(),
        )
        .await;
        chain = merge_meta_chain_with_resolved_for_read(meta_chain.clone(), resolved);
    }

    let missing: Vec<String> = chain
        .iter().filter(|&p| !node_by_pos.contains_key(p.as_str())).cloned()
        .collect();
    if !missing.is_empty() {
        let futs: Vec<_> = missing
            .into_iter()
            .map(|p| {
                let st = state.clone();
                async move {
                    let n = node_metrics(&st, &p, true).await?;
                    Ok::<_, ApiError>((p, n))
                }
            })
            .collect();
        for joined in join_all(futs).await {
            let (p, n) = joined?;
            node_by_pos.insert(p, n);
        }
    }

    let mut nodes: Vec<PositionStreamLineageNode> = Vec::with_capacity(chain.len());
    for p in &chain {
        let n = node_by_pos.remove(p.as_str()).ok_or_else(|| {
            ApiError::internal(format!(
                "chain-history read: missing node for position {p} after merge (anchor={})",
                effective_anchor
            ))
        })?;
        nodes.push(n);
    }

    enrich_chain_history_nodes_open_quote_baseline_lift(state, &chain, &mut nodes).await?;

    // Do **not** overwrite `chain_history_start_value_usd` whenever `baseline_value_usd` is positive:
    // after enrich, baseline can still reflect open-quote (~9.66x) while SQL `start_value_usd` already
    // holds the snapshot writer mark (~9.67x). Only backfill the JSON mirror when the SQL column was empty.
    for node in &mut nodes {
        if node.chain_history_start_value_usd.is_none() && node.baseline_value_usd > Decimal::ZERO {
            node.chain_history_start_value_usd =
                chain_history_decimal_column_string_positive(Some(node.baseline_value_usd));
        }
        if node.chain_history_end_value_usd.is_none()
            && node.closed_ts_utc.is_some()
            && node.current_value_usd > Decimal::ZERO
        {
            node.chain_history_end_value_usd =
                chain_history_decimal_column_string_positive(Some(node.current_value_usd));
        }
    }

    refresh_chain_history_node_fees_from_ledger(state, &chain, &mut nodes).await?;

    let mut totals = totals;
    refresh_lineage_totals_from_nodes(&entry, &mut totals, &mut nodes);

    let mut chain_cost_summary =
        rollup_lineage_chain_costs(&nodes).or_else(|| chain_cost_summary_from_meta.clone());
    let needs_tx_usd = nodes
        .iter()
        .any(|n| n.tx_fee_lamports > 0 && n.tx_fees_usd.is_zero())
        || chain_cost_summary.as_ref().is_some_and(|cs| {
            cs.tx_fee_lamports_total > 0 && cs.tx_fees_usd_total.is_zero()
        });
    if needs_tx_usd {
        let (sol_px, _) = sol_usd_for_tx_fees(&nodes).await;
        if sol_px > 0.0 {
            apply_tx_fees_usd_from_lamports_on_nodes(&mut nodes, sol_px);
            chain_cost_summary =
                rollup_lineage_chain_costs(&nodes).or_else(|| chain_cost_summary_from_meta);
            refresh_lineage_totals_from_nodes(&entry, &mut totals, &mut nodes);
        }
    }

    let mut note_out = note.unwrap_or_default();
    if !note_out.is_empty() {
        note_out.push(' ');
    }
    note_out.push_str("source=postgres_chain_history (materialized).");
    if remapped {
        note_out.push(' ');
        note_out.push_str(&format!(
            "(read: requested position {requested} maps to materialized chain_anchor_pubkey {effective_anchor}.)"
        ));
    }

    Ok(Some(PositionStreamLineageResponse {
        position_address: entry,
        chain,
        nodes,
        totals,
        chain_cost_summary,
        note: Some(note_out),
        chain_history_materialized_ts_utc: materialized_ts_rfc3339,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderName, HeaderValue};

    #[test]
    fn metrics_mode_labels_distinct() {
        assert_eq!(metrics_mode_label(false), METRICS_LIVE);
        assert_eq!(metrics_mode_label(true), METRICS_SETTLEMENT_V1);
    }

    #[test]
    fn parse_ts_accepts_zulu() {
        let s = Some("2024-01-02T15:04:05Z".to_string());
        assert!(parse_ts(&s).is_some());
    }

    #[test]
    fn merge_meta_chain_with_resolved_tail_extends_single_meta_row() {
        let meta = vec!["NEW".to_string()];
        let resolved = vec!["OLD".to_string(), "NEW".to_string()];
        assert_eq!(
            merge_meta_chain_with_resolved_for_read(meta, resolved),
            vec!["OLD", "NEW"]
        );
    }

    #[test]
    fn merge_meta_chain_prefix_unchanged_when_resolved_not_longer() {
        let meta = vec!["A".to_string(), "B".to_string()];
        let resolved = vec!["A".to_string(), "B".to_string()];
        assert_eq!(
            merge_meta_chain_with_resolved_for_read(meta, resolved),
            vec!["A", "B"]
        );
    }

    #[test]
    fn require_auth_accepts_bearer_case_insensitive_prefix() {
        let cfg = ApiConfig {
            chain_history_refresh_secret: Some("mysecret".to_string()),
            ..Default::default()
        };
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str("bearer mysecret").expect("header value"),
        );
        assert!(require_chain_history_refresh_auth(&cfg, &headers).is_ok());
    }

    #[test]
    fn require_auth_accepts_x_chain_history_refresh_header() {
        let cfg = ApiConfig {
            chain_history_refresh_secret: Some("tok".to_string()),
            ..Default::default()
        };
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-chain-history-refresh"),
            HeaderValue::from_static("tok"),
        );
        assert!(require_chain_history_refresh_auth(&cfg, &headers).is_ok());
    }

    #[test]
    fn require_auth_rejects_wrong_token() {
        let cfg = ApiConfig {
            chain_history_refresh_secret: Some("a".to_string()),
            ..Default::default()
        };
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str("Bearer b").expect("header value"),
        );
        assert!(require_chain_history_refresh_auth(&cfg, &headers).is_err());
    }

    #[test]
    fn ch_ticks_from_details_accepts_tick_lower_upper() {
        let d = serde_json::json!({"tick_lower": -443636, "tick_upper": -443200});
        assert_eq!(
            super::ch_ticks_open_from_details(&d),
            Some((-443_636, -443_200))
        );
    }

    #[test]
    fn ch_ticks_from_details_accepts_new_tick_keys() {
        let d = serde_json::json!({"new_tick_lower": 100, "new_tick_upper": 200});
        assert_eq!(super::ch_ticks_open_from_details(&d), Some((100, 200)));
    }

    #[test]
    fn ch_event_price_a_from_details_parses_string() {
        let d = serde_json::json!({"event_price_a_usd": "123.45"});
        let dec = super::ch_decimal_event_price_a_from_details(&d).expect("decimal");
        assert!(dec > rust_decimal::Decimal::ZERO);
    }

    #[test]
    fn require_auth_skipped_when_secret_unset() {
        let cfg = ApiConfig::default();
        let headers = HeaderMap::new();
        assert!(require_chain_history_refresh_auth(&cfg, &headers).is_ok());
    }
}
