//! Position "stream" performance: stitch close->open lineage and aggregate costs/fees across PDAs.
//!
//! Motivation: strategies may rotate positions (new PDA per rebalance). Per-PDA monitor baselines
//! are not sufficient for long-lived performance analytics; we persist the lineage and ledger rows.

use crate::models::PositionStreamPerformanceResponse;
use crate::services::price_fetch::fetch_mint_prices_usd;
use crate::services::wallet_gl_posting;
use crate::state::AppState;
use anyhow::Context;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::Value;
use sqlx::Row;
use std::collections::{BTreeSet, HashSet, VecDeque};
use std::str::FromStr;
use std::time::{Duration, Instant};

/// Wrapped SOL mint — tx fees are in SOL (lamports).
const WSOL_MINT: &str = "So11111111111111111111111111111111111111112";

fn parse_ts_utc(v: &Value) -> Option<DateTime<Utc>> {
    let s = v.as_str()?.trim();
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn decimal_from_value(v: &Value) -> Option<Decimal> {
    if let Some(s) = v.as_str() {
        return Decimal::from_str(s.trim()).ok();
    }
    if let Some(n) = v.as_u64() {
        return Some(Decimal::from(n));
    }
    if let Some(n) = v.as_i64() {
        return Some(Decimal::from(n));
    }
    if let Some(f) = v.as_f64() {
        return Decimal::from_f64_retain(f);
    }
    None
}

async fn ingest_il_edges_best_effort(state: &AppState) -> anyhow::Result<()> {
    let Some(db) = state.db.as_ref() else {
        return Ok(());
    };
    let Some(path) = clmm_lp_protocols::ledger::tx_lifecycle::il_ledger_path_from_env() else {
        return Ok(());
    };
    if !path.exists() {
        return Ok(());
    }

    let txt = std::fs::read_to_string(&path).context("read il ledger file")?;
    for line in txt.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(t) else {
            continue;
        };
        let sid = v
            .get("rebalance_session_id")
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let old = v
            .get("old_position")
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let newp = v
            .get("position")
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let (Some(sid), Some(old), Some(newp)) = (sid, old, newp) else {
            continue;
        };
        let ts = v.get("timestamp").and_then(parse_ts_utc);

        // Idempotent edge insert via PK (session, old, new).
        sqlx::query(
            r#"
            INSERT INTO position_stream_edges (rebalance_session_id, ts_utc, old_position, new_position, source)
            VALUES ($1, $2, $3, $4, 'il_ledger')
            ON CONFLICT (rebalance_session_id, old_position, new_position) DO UPDATE
            SET ts_utc = COALESCE(EXCLUDED.ts_utc, position_stream_edges.ts_utc)
            "#,
        )
        .bind(sid)
        .bind(ts)
        .bind(old)
        .bind(newp)
        .execute(db.pool())
        .await?;
    }
    Ok(())
}

async fn ingest_lifecycle_rows_best_effort(state: &AppState) -> anyhow::Result<()> {
    let Some(db) = state.db.as_ref() else {
        return Ok(());
    };
    let schema_caps = load_ledger_row_schema_caps(db).await?;
    let path = clmm_lp_protocols::ledger::tx_lifecycle::ledger_read_path();
    if !path.exists() {
        return Ok(());
    }
    let txt = std::fs::read_to_string(&path).context("read lifecycle ledger file")?;

    for line in txt.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(t) else {
            continue;
        };
        let signature = v
            .get("signature")
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string);
        let ts = v.get("ts_utc").and_then(parse_ts_utc);
        let source = v
            .get("source")
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string);
        let event = v
            .get("event")
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string);
        let sid = v
            .get("rebalance_session_id")
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string);
        let position = v
            .get("position_pda")
            .and_then(|x| x.as_str())
            .or_else(|| v.get("position_pubkey").and_then(|x| x.as_str()))
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string);
        // Lifecycle rows historically used `pool_address`; keep `pool_pubkey` as preferred key
        // but accept either so ingest remains backward-compatible.
        let pool = v
            .get("pool_pubkey")
            .and_then(|x| x.as_str())
            .or_else(|| v.get("pool_address").and_then(|x| x.as_str()))
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string);
        let tx_fee_lamports = v.get("tx_fee_lamports").and_then(|x| x.as_i64());
        let fee_a = v
            .get("fee_payer_token_a_delta_ui")
            .and_then(decimal_from_value);
        let fee_b = v
            .get("fee_payer_token_b_delta_ui")
            .and_then(decimal_from_value);
        let token_deltas = v.get("fee_payer_token_deltas").cloned();
        let lp_a = v.get("lp_collected_token_a_raw").and_then(|x| {
            x.as_i64()
                .or_else(|| x.as_u64().map(|n| n as i64))
                .or_else(|| x.as_str().and_then(|s| s.parse().ok()))
        });
        let lp_b = v.get("lp_collected_token_b_raw").and_then(|x| {
            x.as_i64()
                .or_else(|| x.as_u64().map(|n| n as i64))
                .or_else(|| x.as_str().and_then(|s| s.parse().ok()))
        });

        // Signature is the idempotency key when present. When absent, we still store best-effort rows.
        if schema_caps.has_fee_payer_token_deltas && schema_caps.has_lp_collected_raw {
            sqlx::query(
                r#"
                INSERT INTO position_stream_ledger_rows (
                  signature, ts_utc, source, event, rebalance_session_id, position_pubkey, pool_pubkey,
                  tx_fee_lamports, fee_payer_token_a_delta_ui, fee_payer_token_b_delta_ui, fee_payer_token_deltas,
                  lp_collected_token_a_raw, lp_collected_token_b_raw, raw_json
                )
                VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)
                ON CONFLICT (signature) DO UPDATE SET
                  ts_utc = COALESCE(EXCLUDED.ts_utc, position_stream_ledger_rows.ts_utc),
                  source = COALESCE(EXCLUDED.source, position_stream_ledger_rows.source),
                  event = COALESCE(EXCLUDED.event, position_stream_ledger_rows.event),
                  rebalance_session_id = COALESCE(EXCLUDED.rebalance_session_id, position_stream_ledger_rows.rebalance_session_id),
                  position_pubkey = COALESCE(EXCLUDED.position_pubkey, position_stream_ledger_rows.position_pubkey),
                  pool_pubkey = COALESCE(EXCLUDED.pool_pubkey, position_stream_ledger_rows.pool_pubkey),
                  tx_fee_lamports = COALESCE(EXCLUDED.tx_fee_lamports, position_stream_ledger_rows.tx_fee_lamports),
                  fee_payer_token_a_delta_ui = COALESCE(EXCLUDED.fee_payer_token_a_delta_ui, position_stream_ledger_rows.fee_payer_token_a_delta_ui),
                  fee_payer_token_b_delta_ui = COALESCE(EXCLUDED.fee_payer_token_b_delta_ui, position_stream_ledger_rows.fee_payer_token_b_delta_ui),
                  fee_payer_token_deltas = COALESCE(EXCLUDED.fee_payer_token_deltas, position_stream_ledger_rows.fee_payer_token_deltas),
                  lp_collected_token_a_raw = COALESCE(EXCLUDED.lp_collected_token_a_raw, position_stream_ledger_rows.lp_collected_token_a_raw),
                  lp_collected_token_b_raw = COALESCE(EXCLUDED.lp_collected_token_b_raw, position_stream_ledger_rows.lp_collected_token_b_raw),
                  raw_json = EXCLUDED.raw_json
                "#,
            )
            .bind(signature)
            .bind(ts)
            .bind(source)
            .bind(event)
            .bind(sid)
            .bind(position)
            .bind(pool)
            .bind(tx_fee_lamports)
            .bind(fee_a)
            .bind(fee_b)
            .bind(token_deltas)
            .bind(lp_a)
            .bind(lp_b)
            .bind(&v)
            .execute(db.pool())
            .await?;
            wallet_gl_posting::apply_session_postings_from_lifecycle_json(db, &v, lp_a, lp_b)
                .await;
        } else if schema_caps.has_fee_payer_token_deltas {
            sqlx::query(
                r#"
                INSERT INTO position_stream_ledger_rows (
                  signature, ts_utc, source, event, rebalance_session_id, position_pubkey, pool_pubkey,
                  tx_fee_lamports, fee_payer_token_a_delta_ui, fee_payer_token_b_delta_ui, fee_payer_token_deltas, raw_json
                )
                VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)
                ON CONFLICT (signature) DO UPDATE SET
                  ts_utc = COALESCE(EXCLUDED.ts_utc, position_stream_ledger_rows.ts_utc),
                  source = COALESCE(EXCLUDED.source, position_stream_ledger_rows.source),
                  event = COALESCE(EXCLUDED.event, position_stream_ledger_rows.event),
                  rebalance_session_id = COALESCE(EXCLUDED.rebalance_session_id, position_stream_ledger_rows.rebalance_session_id),
                  position_pubkey = COALESCE(EXCLUDED.position_pubkey, position_stream_ledger_rows.position_pubkey),
                  pool_pubkey = COALESCE(EXCLUDED.pool_pubkey, position_stream_ledger_rows.pool_pubkey),
                  tx_fee_lamports = COALESCE(EXCLUDED.tx_fee_lamports, position_stream_ledger_rows.tx_fee_lamports),
                  fee_payer_token_a_delta_ui = COALESCE(EXCLUDED.fee_payer_token_a_delta_ui, position_stream_ledger_rows.fee_payer_token_a_delta_ui),
                  fee_payer_token_b_delta_ui = COALESCE(EXCLUDED.fee_payer_token_b_delta_ui, position_stream_ledger_rows.fee_payer_token_b_delta_ui),
                  fee_payer_token_deltas = COALESCE(EXCLUDED.fee_payer_token_deltas, position_stream_ledger_rows.fee_payer_token_deltas),
                  raw_json = EXCLUDED.raw_json
                "#,
            )
            .bind(signature)
            .bind(ts)
            .bind(source)
            .bind(event)
            .bind(sid)
            .bind(position)
            .bind(pool)
            .bind(tx_fee_lamports)
            .bind(fee_a)
            .bind(fee_b)
            .bind(token_deltas)
            .bind(&v)
            .execute(db.pool())
            .await?;
            wallet_gl_posting::apply_session_postings_from_lifecycle_json(db, &v, None, None)
                .await;
        } else {
            sqlx::query(
                r#"
                INSERT INTO position_stream_ledger_rows (
                  signature, ts_utc, source, event, rebalance_session_id, position_pubkey, pool_pubkey,
                  tx_fee_lamports, fee_payer_token_a_delta_ui, fee_payer_token_b_delta_ui, raw_json
                )
                VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
                ON CONFLICT (signature) DO UPDATE SET
                  ts_utc = COALESCE(EXCLUDED.ts_utc, position_stream_ledger_rows.ts_utc),
                  source = COALESCE(EXCLUDED.source, position_stream_ledger_rows.source),
                  event = COALESCE(EXCLUDED.event, position_stream_ledger_rows.event),
                  rebalance_session_id = COALESCE(EXCLUDED.rebalance_session_id, position_stream_ledger_rows.rebalance_session_id),
                  position_pubkey = COALESCE(EXCLUDED.position_pubkey, position_stream_ledger_rows.position_pubkey),
                  pool_pubkey = COALESCE(EXCLUDED.pool_pubkey, position_stream_ledger_rows.pool_pubkey),
                  tx_fee_lamports = COALESCE(EXCLUDED.tx_fee_lamports, position_stream_ledger_rows.tx_fee_lamports),
                  fee_payer_token_a_delta_ui = COALESCE(EXCLUDED.fee_payer_token_a_delta_ui, position_stream_ledger_rows.fee_payer_token_a_delta_ui),
                  fee_payer_token_b_delta_ui = COALESCE(EXCLUDED.fee_payer_token_b_delta_ui, position_stream_ledger_rows.fee_payer_token_b_delta_ui),
                  raw_json = EXCLUDED.raw_json
                "#,
            )
            .bind(signature)
            .bind(ts)
            .bind(source)
            .bind(event)
            .bind(sid)
            .bind(position)
            .bind(pool)
            .bind(tx_fee_lamports)
            .bind(fee_a)
            .bind(fee_b)
            .bind(&v)
            .execute(db.pool())
            .await?;
            wallet_gl_posting::apply_session_postings_from_lifecycle_json(db, &v, None, None)
                .await;
        }
    }
    Ok(())
}

struct LedgerRowSchemaCaps {
    has_fee_payer_token_deltas: bool,
    has_lp_collected_raw: bool,
}

async fn load_ledger_row_schema_caps(
    db: &clmm_lp_data::repositories::Database,
) -> anyhow::Result<LedgerRowSchemaCaps> {
    let rows = sqlx::query(
        r#"
        SELECT column_name
        FROM information_schema.columns
        WHERE table_name = 'position_stream_ledger_rows'
          AND table_schema = ANY(current_schemas(false))
        "#,
    )
    .fetch_all(db.pool())
    .await?;
    let mut cols: HashSet<String> = HashSet::new();
    for r in rows {
        let name: String = r.try_get("column_name").unwrap_or_default();
        if !name.trim().is_empty() {
            cols.insert(name);
        }
    }
    Ok(LedgerRowSchemaCaps {
        has_fee_payer_token_deltas: cols.contains("fee_payer_token_deltas"),
        has_lp_collected_raw: cols.contains("lp_collected_token_a_raw")
            && cols.contains("lp_collected_token_b_raw"),
    })
}

async fn maybe_ingest_ledgers(state: &AppState, skip: bool) {
    if skip || state.db.is_none() {
        return;
    }
    // Ingest can scan entire JSONL and issue one INSERT per line — never block hot read paths
    // like `stream-lineage` (UI 120s timeouts). Lineage skips ingest; other callers still ingest.
    let min_interval = Duration::from_secs(10);
    {
        let g = state.ledger_ingest_last_at.read().await;
        if let Some(t) = *g
            && t.elapsed() < min_interval
        {
            return;
        }
    }
    {
        let mut g = state.ledger_ingest_last_at.write().await;
        *g = Some(Instant::now());
    }

    if let Err(e) = ingest_il_edges_best_effort(state).await {
        tracing::warn!(error = %e, "stream ingest: IL edges failed");
    }
    if let Err(e) = ingest_lifecycle_rows_best_effort(state).await {
        tracing::warn!(error = %e, "stream ingest: lifecycle rows failed");
    }
}

async fn stream_component_positions_and_sessions(
    state: &AppState,
    start_position: &str,
    max_nodes: usize,
) -> anyhow::Result<(Vec<String>, Vec<String>)> {
    let Some(db) = state.db.as_ref() else {
        return Ok((vec![start_position.to_string()], Vec::new()));
    };

    // One round-trip: load all edges, BFS in memory. The old per-node SQL BFS could issue
    // thousands of queries for a large connected component and exceed HTTP timeouts.
    let edge_rows = sqlx::query(
        r#"SELECT rebalance_session_id, old_position, new_position FROM position_stream_edges"#,
    )
    .fetch_all(db.pool())
    .await?;

    let mut edges_mem: Vec<(String, String, String)> = Vec::with_capacity(edge_rows.len());
    for r in edge_rows {
        let sid: String = r.try_get("rebalance_session_id").unwrap_or_default();
        let oldp: String = r.try_get("old_position").unwrap_or_default();
        let newp: String = r.try_get("new_position").unwrap_or_default();
        if oldp.trim().is_empty() || newp.trim().is_empty() {
            continue;
        }
        edges_mem.push((sid, oldp, newp));
    }

    use std::collections::HashMap;
    let mut adj: HashMap<String, Vec<String>> = HashMap::new();
    for (_sid, oldp, newp) in &edges_mem {
        adj.entry(oldp.clone()).or_default().push(newp.clone());
        adj.entry(newp.clone()).or_default().push(oldp.clone());
    }

    let mut seen: HashSet<String> = HashSet::new();
    let mut q: VecDeque<String> = VecDeque::new();
    seen.insert(start_position.to_string());
    q.push_back(start_position.to_string());

    while let Some(cur) = q.pop_front() {
        if seen.len() >= max_nodes {
            break;
        }
        let Some(neighbors) = adj.get(&cur) else {
            continue;
        };
        for nxt in neighbors {
            if seen.len() >= max_nodes {
                break;
            }
            if seen.insert(nxt.clone()) {
                q.push_back(nxt.clone());
            }
        }
    }

    let mut sessions: BTreeSet<String> = BTreeSet::new();
    for (sid, oldp, newp) in &edges_mem {
        if sid.trim().is_empty() {
            continue;
        }
        if seen.contains(oldp) && seen.contains(newp) {
            sessions.insert(sid.clone());
        }
    }

    let mut positions: Vec<String> = seen.into_iter().collect();
    positions.sort();
    let sessions: Vec<String> = sessions.into_iter().collect();
    Ok((positions, sessions))
}

/// Compute best-effort stream-level aggregates for a position PDA.
///
/// When `skip_ledger_ingest` is true (e.g. `stream-lineage`), skip JSONL→DB ingest so the handler
/// stays fast; edges/ledger already in Postgres are still used.
pub async fn compute_position_stream_performance(
    state: &AppState,
    position_address: &str,
    skip_ledger_ingest: bool,
) -> Result<PositionStreamPerformanceResponse, crate::error::ApiError> {
    maybe_ingest_ledgers(state, skip_ledger_ingest).await;

    let max_nodes = 2000;
    let (positions, sessions) =
        stream_component_positions_and_sessions(state, position_address, max_nodes)
            .await
            .map_err(|e| crate::error::ApiError::internal(format!("stream component: {e}")))?;

    let Some(db) = state.db.as_ref() else {
        return Ok(PositionStreamPerformanceResponse {
            position_address: position_address.to_string(),
            positions,
            sessions,
            total_tx_fee_lamports: 0,
            total_tx_fee_usd: Decimal::ZERO,
            collect_events: 0,
            collected_token_a_ui: None,
            collected_token_b_ui: None,
            note: Some("DB is disabled (DATABASE_URL not set or connect/migrate failed) — showing only the current PDA.".to_string()),
        });
    };

    // Total tx fee in lamports: prefer session join; if there are no sessions yet, fall back to position filter.
    let (fee_lamports, collect_events, sum_a, sum_b) = if !sessions.is_empty() {
        let fee_row = sqlx::query(
            r#"
            SELECT
              COALESCE(SUM(tx_fee_lamports), 0) AS fee_lamports,
              COALESCE(SUM(CASE WHEN event = 'bot_collect_fees' THEN 1 ELSE 0 END), 0) AS collect_events,
              SUM(CASE WHEN event = 'bot_collect_fees' THEN fee_payer_token_a_delta_ui ELSE NULL END) AS sum_a,
              SUM(CASE WHEN event = 'bot_collect_fees' THEN fee_payer_token_b_delta_ui ELSE NULL END) AS sum_b
            FROM position_stream_ledger_rows
            WHERE rebalance_session_id = ANY($1)
            "#,
        )
        .bind(&sessions)
        .fetch_one(db.pool())
        .await
        .map_err(|e| crate::error::ApiError::internal(format!("stream aggregate: {e}")))?;
        let fee_lamports: i64 = fee_row.try_get("fee_lamports").unwrap_or(0);
        let collect_events: i64 = fee_row.try_get("collect_events").unwrap_or(0);
        let sum_a: Option<Decimal> = fee_row.try_get("sum_a").ok();
        let sum_b: Option<Decimal> = fee_row.try_get("sum_b").ok();
        (
            fee_lamports.max(0) as u64,
            collect_events.max(0) as u32,
            sum_a,
            sum_b,
        )
    } else {
        let fee_row = sqlx::query(
            r#"
            SELECT
              COALESCE(SUM(tx_fee_lamports), 0) AS fee_lamports,
              COALESCE(SUM(CASE WHEN event = 'bot_collect_fees' THEN 1 ELSE 0 END), 0) AS collect_events,
              SUM(CASE WHEN event = 'bot_collect_fees' THEN fee_payer_token_a_delta_ui ELSE NULL END) AS sum_a,
              SUM(CASE WHEN event = 'bot_collect_fees' THEN fee_payer_token_b_delta_ui ELSE NULL END) AS sum_b
            FROM position_stream_ledger_rows
            WHERE position_pubkey = ANY($1)
            "#,
        )
        .bind(&positions)
        .fetch_one(db.pool())
        .await
        .map_err(|e| crate::error::ApiError::internal(format!("stream aggregate: {e}")))?;
        let fee_lamports: i64 = fee_row.try_get("fee_lamports").unwrap_or(0);
        let collect_events: i64 = fee_row.try_get("collect_events").unwrap_or(0);
        let sum_a: Option<Decimal> = fee_row.try_get("sum_a").ok();
        let sum_b: Option<Decimal> = fee_row.try_get("sum_b").ok();
        (
            fee_lamports.max(0) as u64,
            collect_events.max(0) as u32,
            sum_a,
            sum_b,
        )
    };

    // Convert SOL lamports fee to USD using free mint price fetcher.
    let mut mints: BTreeSet<String> = BTreeSet::new();
    mints.insert(WSOL_MINT.to_string());
    let (px, _src) = fetch_mint_prices_usd(&mints).await;
    let sol_usd = px.get(WSOL_MINT).copied().unwrap_or(0.0);
    let total_tx_fee_usd = if sol_usd > 0.0 {
        let usd = (fee_lamports as f64 / 1e9) * sol_usd;
        Decimal::from_f64_retain(usd).unwrap_or(Decimal::ZERO)
    } else {
        Decimal::ZERO
    };

    Ok(PositionStreamPerformanceResponse {
        position_address: position_address.to_string(),
        positions,
        sessions,
        total_tx_fee_lamports: fee_lamports,
        total_tx_fee_usd,
        collect_events,
        collected_token_a_ui: sum_a,
        collected_token_b_ui: sum_b,
        note: Some(
            "Stream performance is best-effort from local JSONL ledgers + current free price feeds. Full net PnL/IL across rotates requires cashflow baselines (planned)."
                .to_string(),
        ),
    })
}
