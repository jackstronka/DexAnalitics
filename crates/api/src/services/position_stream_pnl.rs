//! Stream-level Net PnL / IL across rotated position PDAs.
//!
//! Uses DB-backed valuation snapshots + lifecycle ledger token deltas when available.
//!
//! When a rotation **lineage chain** is provided (oldest → newest PDA), IL/HODL is anchored to the
//! **first** member’s open baseline and the **last** member’s latest/end mark — matching “history from
//! start position to final position”. Without it, fallback uses global MIN/MAX snapshot timestamps across
//! the stream component (legacy behavior).

use crate::error::ApiError;
use crate::models::{PositionStreamPnLResponse, StreamPnLInterpretation};
use crate::services::position_stream_lineage::resolve_lineage_chain_for_stream_pnl;
use crate::services::position_stream_performance::compute_position_stream_performance;
use crate::services::position_valuation::{
    compute_position_usd_valuation, fetch_prices_for_positions, monitored_position_from_chain,
};
use crate::services::price_fetch::fetch_mint_prices_usd;
use crate::state::AppState;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::Value;
use sqlx::Row;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::str::FromStr;
use tokio::time::{Duration, timeout};

const WSOL_MINT: &str = "So11111111111111111111111111111111111111112";

/// Earliest snapshot across stream members — must include mint columns or HODL/IL falls back incorrectly.
const BASELINE_SNAPSHOT_SQL: &str = r#"
        SELECT ts_utc, value_usd, amount_a_ui, amount_b_ui, pool_pubkey, token_mint_a, token_mint_b
        FROM position_stream_valuation_snapshots
        WHERE position_pubkey = ANY($1)
        ORDER BY ts_utc ASC
        LIMIT 1
        "#;

/// Latest snapshot across stream members.
const CURRENT_SNAPSHOT_SQL: &str = r#"
        SELECT ts_utc, value_usd, amount_a_ui, amount_b_ui, pool_pubkey, token_mint_a, token_mint_b
        FROM position_stream_valuation_snapshots
        WHERE position_pubkey = ANY($1)
        ORDER BY ts_utc DESC
        LIMIT 1
        "#;

/// First PDA in lineage: prefer explicit open snapshot (same ordering as lineage `node_metrics`).
const BASELINE_SNAPSHOT_FIRST_PDA_SQL: &str = r#"
        SELECT ts_utc, value_usd, amount_a_ui, amount_b_ui, pool_pubkey, token_mint_a, token_mint_b
        FROM position_stream_valuation_snapshots
        WHERE position_pubkey = $1
        ORDER BY
          CASE WHEN COALESCE(raw_json->>'kind', '') = 'baseline_open' THEN 0 ELSE 1 END,
          ts_utc ASC
        LIMIT 1
        "#;

/// Last PDA in lineage: prefer close/end snapshot (same ordering as lineage `node_metrics`).
const CURRENT_SNAPSHOT_LAST_PDA_SQL: &str = r#"
        SELECT ts_utc, value_usd, amount_a_ui, amount_b_ui, pool_pubkey, token_mint_a, token_mint_b
        FROM position_stream_valuation_snapshots
        WHERE position_pubkey = $1
        ORDER BY
          CASE WHEN COALESCE(raw_json->>'kind', '') = 'end_close' THEN 0 ELSE 1 END,
          ts_utc DESC
        LIMIT 1
        "#;

fn nonempty_trimmed_mint(opt: Option<String>) -> Option<String> {
    opt.and_then(|s| {
        let t = s.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        }
    })
}

/// Pool leg mints for pricing the baseline basket: prefer baseline snapshot, fall back to latest snapshot.
fn pool_mints_for_hodl(
    baseline_mints: (Option<String>, Option<String>),
    current_mints: (Option<String>, Option<String>),
) -> Vec<String> {
    let mut pool_mints = Vec::new();
    if let Some(a) =
        nonempty_trimmed_mint(baseline_mints.0).or_else(|| nonempty_trimmed_mint(current_mints.0))
    {
        pool_mints.push(a);
    }
    if let Some(b) =
        nonempty_trimmed_mint(baseline_mints.1).or_else(|| nonempty_trimmed_mint(current_mints.1))
    {
        pool_mints.push(b);
    }
    pool_mints
}

fn decimal_from_json(v: &Value) -> Option<Decimal> {
    if let Some(s) = v.as_str() {
        return Decimal::from_str(s.trim()).ok();
    }
    if let Some(f) = v.as_f64() {
        return Decimal::from_f64_retain(f);
    }
    if let Some(i) = v.as_i64() {
        return Some(Decimal::from(i));
    }
    if let Some(u) = v.as_u64() {
        return Some(Decimal::from(u));
    }
    None
}

fn chain_session_ids_from_edges(
    chain: &[String],
    edges: &[(String, String, String)],
) -> Vec<String> {
    if chain.len() < 2 {
        return Vec::new();
    }
    let mut adjacent_pairs: HashSet<(&str, &str)> = HashSet::new();
    for w in chain.windows(2) {
        if let [a, b] = w {
            adjacent_pairs.insert((a.as_str(), b.as_str()));
        }
    }
    let mut out: BTreeSet<String> = BTreeSet::new();
    for (sid, oldp, newp) in edges {
        let sid_t = sid.trim();
        if sid_t.is_empty() {
            continue;
        }
        if adjacent_pairs.contains(&(oldp.as_str(), newp.as_str())) {
            out.insert(sid_t.to_string());
        }
    }
    out.into_iter().collect()
}

async fn chain_sessions_from_db(
    db: &clmm_lp_data::repositories::Database,
    chain: &[String],
) -> Result<Vec<String>, ApiError> {
    if chain.len() < 2 {
        return Ok(Vec::new());
    }
    let rows = sqlx::query(
        r#"
        SELECT rebalance_session_id, old_position, new_position
        FROM position_stream_edges
        WHERE old_position = ANY($1) AND new_position = ANY($1)
        "#,
    )
    .bind(chain)
    .fetch_all(db.pool())
    .await
    .map_err(|e| ApiError::internal(format!("stream pnl: chain sessions query: {e}")))?;
    let mut edges: Vec<(String, String, String)> = Vec::with_capacity(rows.len());
    for r in rows {
        let sid: String = r.try_get("rebalance_session_id").unwrap_or_default();
        let oldp: String = r.try_get("old_position").unwrap_or_default();
        let newp: String = r.try_get("new_position").unwrap_or_default();
        if oldp.trim().is_empty() || newp.trim().is_empty() {
            continue;
        }
        edges.push((sid, oldp, newp));
    }
    Ok(chain_session_ids_from_edges(chain, &edges))
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

fn stream_pnl_db_disabled_response(position_address: &str) -> PositionStreamPnLResponse {
    PositionStreamPnLResponse {
        position_address: position_address.to_string(),
        baseline_ts_utc: None,
        current_ts_utc: None,
        baseline_value_usd: Decimal::ZERO,
        current_value_usd: Decimal::ZERO,
        hodl_value_usd: Decimal::ZERO,
        il_usd: Decimal::ZERO,
        il_pct: Decimal::ZERO,
        tx_fees_usd: Decimal::ZERO,
        realized_cashflow_usd: Decimal::ZERO,
        net_pnl_usd: Decimal::ZERO,
        net_pnl_pct: Decimal::ZERO,
        interpretation: StreamPnLInterpretation::default(),
        note: Some(
            "DB is disabled (DATABASE_URL missing/failed); stream PnL/IL unavailable.".to_string(),
        ),
    }
}

fn stream_pnl_interpretation_pl(
    use_lineage_anchor: bool,
    hodl_basket_ok: bool,
) -> StreamPnLInterpretation {
    let mut il = String::from(
        "Benchmark IL vs HODL: wartość LP na końcu łańcucha minus hipotetyczna wartość trzymania tokenów z depozytu na początku łańcucha, przy bieżących cenach mintów. Inna definicja niż net PnL (bez pełnej księgowości między rotacjami).",
    );
    if use_lineage_anchor {
        il.push_str(
            " Start i koniec odczytu odpowiadają pierwszemu i ostatniemu PDA w historii rotacji.",
        );
    }
    if !hodl_basket_ok {
        il.push_str(" Przy braku mintów/ilości w snapshotach benchmark HODL może być zdegradowany — patrz `note`.");
    }
    StreamPnLInterpretation {
        economic_net_pnl_caption_pl: "Wynik ekonomiczny łańcucha (net PnL): końcowy NAV + cashflow z ledgera (collect/close/open, best-effort USD) − NAV startowy − opłaty sieci SOL (USD). To bilans „strategii”, nie ta sama liczba co IL poniżej.".to_string(),
        il_vs_initial_hodl_caption_pl: il,
    }
}

/// Stream PnL using an explicit member list (e.g. entry-only when lineage suppresses cross-PDA stitching).
///
/// `lineage_chain`: ordered PDAs **old → new** along rotations. When present, baseline snapshot comes from
/// the **first** PDA (start position) and current/end from the **last** PDA (final position).
pub(crate) async fn compute_position_stream_pnl_for_stream_members(
    state: &AppState,
    position_address: &str,
    positions: Vec<String>,
    sessions: Vec<String>,
    lineage_chain: Option<&[String]>,
    allow_self_seed: bool,
    settlement_strict: bool,
) -> Result<PositionStreamPnLResponse, ApiError> {
    let Some(db) = state.db.as_ref() else {
        return Ok(stream_pnl_db_disabled_response(position_address));
    };

    let anchor_chain = lineage_chain.filter(|c| !c.is_empty());
    let start_pubkey = anchor_chain.and_then(|c| c.first().map(|s| s.as_str()));
    let end_pubkey = anchor_chain.and_then(|c| c.last().map(|s| s.as_str()));
    let use_lineage_anchor = anchor_chain.is_some();
    let chain_vec: Vec<String> = anchor_chain
        .map(|c| c.to_vec())
        .unwrap_or_else(|| positions.clone());
    let chain_sessions = chain_sessions_from_db(db, &chain_vec).await?;
    let scoped_sessions = if !chain_sessions.is_empty() {
        chain_sessions
    } else {
        Vec::new()
    };
    let use_chain_session_scope = !scoped_sessions.is_empty();

    let mut baseline_row = if let Some(pk) = start_pubkey {
        sqlx::query(BASELINE_SNAPSHOT_FIRST_PDA_SQL)
            .bind(pk)
            .fetch_optional(db.pool())
            .await
            .map_err(|e| {
                ApiError::internal(format!("stream pnl: baseline query (chain start): {e}"))
            })?
    } else {
        sqlx::query(BASELINE_SNAPSHOT_SQL)
            .bind(&positions)
            .fetch_optional(db.pool())
            .await
            .map_err(|e| ApiError::internal(format!("stream pnl: baseline query: {e}")))?
    };

    let mut current_row = if let Some(pk) = end_pubkey {
        sqlx::query(CURRENT_SNAPSHOT_LAST_PDA_SQL)
            .bind(pk)
            .fetch_optional(db.pool())
            .await
            .map_err(|e| {
                ApiError::internal(format!("stream pnl: current query (chain end): {e}"))
            })?
    } else {
        sqlx::query(CURRENT_SNAPSHOT_SQL)
            .bind(&positions)
            .fetch_optional(db.pool())
            .await
            .map_err(|e| ApiError::internal(format!("stream pnl: current query: {e}")))?
    };

    let seed_pk_for_baseline = start_pubkey.unwrap_or_else(|| position_address.trim());

    if baseline_row.is_none() && allow_self_seed && !settlement_strict {
        // Best-effort self-seed: valuation snapshot for chain **start** PDA (or entry) when missing.
        if let Ok(pk) = solana_sdk::pubkey::Pubkey::from_str(seed_pk_for_baseline)
            && let Ok(Ok(pos)) = timeout(
                Duration::from_secs(2),
                monitored_position_from_chain(state.provider.clone(), &pk),
            )
            .await
        {
            let prices =
                fetch_prices_for_positions(state.provider.clone(), std::slice::from_ref(&pos))
                    .await;
            if let Ok(v) =
                compute_position_usd_valuation(state.provider.clone(), &pos, &prices).await
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
                    "source": "stream_pnl_self_seed"
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
        baseline_row = if let Some(pk) = start_pubkey {
            sqlx::query(BASELINE_SNAPSHOT_FIRST_PDA_SQL)
                .bind(pk)
                .fetch_optional(db.pool())
                .await
                .map_err(|e| {
                    ApiError::internal(format!("stream pnl: baseline query (after seed): {e}"))
                })?
        } else {
            sqlx::query(BASELINE_SNAPSHOT_SQL)
                .bind(&positions)
                .fetch_optional(db.pool())
                .await
                .map_err(|e| {
                    ApiError::internal(format!("stream pnl: baseline query (after seed): {e}"))
                })?
        };
    }

    let seed_pk_for_current = end_pubkey.unwrap_or_else(|| position_address.trim());

    if current_row.is_none() && allow_self_seed && !settlement_strict {
        if let Ok(pk) = solana_sdk::pubkey::Pubkey::from_str(seed_pk_for_current)
            && let Ok(Ok(pos)) = timeout(
                Duration::from_secs(2),
                monitored_position_from_chain(state.provider.clone(), &pk),
            )
            .await
        {
            let prices =
                fetch_prices_for_positions(state.provider.clone(), std::slice::from_ref(&pos))
                    .await;
            if let Ok(v) =
                compute_position_usd_valuation(state.provider.clone(), &pos, &prices).await
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
                    "source": "stream_pnl_self_seed_current"
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
        current_row = if let Some(pk) = end_pubkey {
            sqlx::query(CURRENT_SNAPSHOT_LAST_PDA_SQL)
                .bind(pk)
                .fetch_optional(db.pool())
                .await
                .map_err(|e| {
                    ApiError::internal(format!("stream pnl: current query (after seed): {e}"))
                })?
        } else {
            sqlx::query(CURRENT_SNAPSHOT_SQL)
                .bind(&positions)
                .fetch_optional(db.pool())
                .await
                .map_err(|e| {
                    ApiError::internal(format!("stream pnl: current query (after seed): {e}"))
                })?
        };
    }

    let Some(b) = baseline_row else {
        return Ok(PositionStreamPnLResponse {
            position_address: position_address.to_string(),
            baseline_ts_utc: None,
            current_ts_utc: None,
            baseline_value_usd: Decimal::ZERO,
            current_value_usd: Decimal::ZERO,
            hodl_value_usd: Decimal::ZERO,
            il_usd: Decimal::ZERO,
            il_pct: Decimal::ZERO,
            tx_fees_usd: Decimal::ZERO,
            realized_cashflow_usd: Decimal::ZERO,
            net_pnl_usd: Decimal::ZERO,
            net_pnl_pct: Decimal::ZERO,
            interpretation: stream_pnl_interpretation_pl(use_lineage_anchor, false),
            note: Some(if settlement_strict {
                "Settlement v1 requires persisted valuation snapshots (self-seed disabled). Baseline snapshot unavailable.".to_string()
            } else {
                "No valuation snapshots yet (even after best-effort self-seed). Check DB migrations and RPC health.".to_string()
            }),
        });
    };

    let baseline_ts: Option<DateTime<Utc>> = b.try_get("ts_utc").ok();
    let baseline_value: Decimal = b.try_get("value_usd").unwrap_or(Decimal::ZERO);
    let baseline_a: Decimal = b.try_get("amount_a_ui").unwrap_or(Decimal::ZERO);
    let baseline_b: Decimal = b.try_get("amount_b_ui").unwrap_or(Decimal::ZERO);
    let baseline_ma = b
        .try_get::<Option<String>, _>("token_mint_a")
        .ok()
        .flatten();
    let baseline_mb = b
        .try_get::<Option<String>, _>("token_mint_b")
        .ok()
        .flatten();

    let (current_ts, current_value, current_ma, current_mb) = if let Some(c) = current_row {
        (
            c.try_get::<DateTime<Utc>, _>("ts_utc").ok(),
            c.try_get::<Decimal, _>("value_usd")
                .unwrap_or(Decimal::ZERO),
            c.try_get::<Option<String>, _>("token_mint_a")
                .ok()
                .flatten(),
            c.try_get::<Option<String>, _>("token_mint_b")
                .ok()
                .flatten(),
        )
    } else {
        (None, Decimal::ZERO, None, None)
    };

    // Convert tx fees to USD using SOL/USD now (best-effort).
    let (sol_usd, sol_src) = sol_usd().await;
    let tx_fee_lamports: i64 = if use_chain_session_scope {
        sqlx::query(
            r#"SELECT COALESCE(SUM(tx_fee_lamports), 0) AS fee_lamports
               FROM position_stream_ledger_rows
               WHERE rebalance_session_id = ANY($1)"#,
        )
        .bind(&scoped_sessions)
        .fetch_one(db.pool())
        .await
        .map_err(|e| ApiError::internal(format!("stream pnl: tx fee sum: {e}")))?
        .try_get("fee_lamports")
        .unwrap_or(0)
    } else if !chain_vec.is_empty() {
        sqlx::query(
            r#"SELECT COALESCE(SUM(tx_fee_lamports), 0) AS fee_lamports
               FROM position_stream_ledger_rows
               WHERE position_pubkey = ANY($1)"#,
        )
        .bind(&chain_vec)
        .fetch_one(db.pool())
        .await
        .map_err(|e| ApiError::internal(format!("stream pnl: tx fee sum: {e}")))?
        .try_get("fee_lamports")
        .unwrap_or(0)
    } else if !sessions.is_empty() {
        sqlx::query(
            r#"SELECT COALESCE(SUM(tx_fee_lamports), 0) AS fee_lamports
               FROM position_stream_ledger_rows
               WHERE rebalance_session_id = ANY($1)"#,
        )
        .bind(&sessions)
        .fetch_one(db.pool())
        .await
        .map_err(|e| ApiError::internal(format!("stream pnl: tx fee sum (legacy sessions): {e}")))?
        .try_get("fee_lamports")
        .unwrap_or(0)
    } else {
        0
    };
    let tx_fee_lamports_u = tx_fee_lamports.max(0) as u64;
    let tx_fees_usd = if sol_usd > 0.0 {
        Decimal::from_f64_retain((tx_fee_lamports_u as f64 / 1e9) * sol_usd)
            .unwrap_or(Decimal::ZERO)
    } else {
        Decimal::ZERO
    };

    // Realized cashflow from lifecycle rows: sum fee_payer_token_deltas for the stream.
    // We don't yet have stable token symbols here; we treat it as USD using current mint prices.
    let rows = if use_chain_session_scope {
        sqlx::query(
            r#"SELECT fee_payer_token_deltas
               FROM position_stream_ledger_rows
               WHERE rebalance_session_id = ANY($1) AND fee_payer_token_deltas IS NOT NULL"#,
        )
        .bind(&scoped_sessions)
        .fetch_all(db.pool())
        .await
        .map_err(|e| ApiError::internal(format!("stream pnl: token deltas rows: {e}")))?
    } else if !chain_vec.is_empty() {
        sqlx::query(
            r#"SELECT fee_payer_token_deltas
               FROM position_stream_ledger_rows
               WHERE position_pubkey = ANY($1) AND fee_payer_token_deltas IS NOT NULL"#,
        )
        .bind(&chain_vec)
        .fetch_all(db.pool())
        .await
        .map_err(|e| ApiError::internal(format!("stream pnl: token deltas rows: {e}")))?
    } else if !sessions.is_empty() {
        sqlx::query(
            r#"SELECT fee_payer_token_deltas
               FROM position_stream_ledger_rows
               WHERE rebalance_session_id = ANY($1) AND fee_payer_token_deltas IS NOT NULL"#,
        )
        .bind(&sessions)
        .fetch_all(db.pool())
        .await
        .map_err(|e| {
            ApiError::internal(format!(
                "stream pnl: token deltas rows (legacy sessions): {e}"
            ))
        })?
    } else {
        Vec::new()
    };

    let mut mint_deltas: BTreeMap<String, Decimal> = BTreeMap::new();
    for r in rows {
        let v: Option<Value> = r.try_get("fee_payer_token_deltas").ok();
        let Some(Value::Object(map)) = v else {
            continue;
        };
        for (mint, dv) in map {
            if let Some(d) = decimal_from_json(&dv) {
                *mint_deltas.entry(mint).or_insert(Decimal::ZERO) += d;
            }
        }
    }

    // Use mints from baseline snapshot (fallback: latest snapshot) for HODL/IL and cashflow conversion.
    let pool_mints = pool_mints_for_hodl(
        (baseline_ma.clone(), baseline_mb.clone()),
        (current_ma, current_mb),
    );
    let mint_set: BTreeSet<String> = pool_mints.iter().cloned().collect();
    let (px, price_src) =
        match timeout(Duration::from_secs(2), fetch_mint_prices_usd(&mint_set)).await {
            Ok(r) => r,
            Err(_) => (BTreeMap::new(), "timeout".to_string()),
        };
    let pa = pool_mints
        .first()
        .and_then(|m| px.get(m))
        .copied()
        .unwrap_or(0.0);
    let pb = pool_mints
        .get(1)
        .and_then(|m| px.get(m))
        .copied()
        .unwrap_or(0.0);
    let pa_d = Decimal::from_f64_retain(pa).unwrap_or(Decimal::ZERO);
    let pb_d = Decimal::from_f64_retain(pb).unwrap_or(Decimal::ZERO);

    let realized_cashflow_usd = if pool_mints.len() == 2 {
        let da = mint_deltas
            .get(&pool_mints[0])
            .cloned()
            .unwrap_or(Decimal::ZERO);
        let dbb = mint_deltas
            .get(&pool_mints[1])
            .cloned()
            .unwrap_or(Decimal::ZERO);
        da * pa_d + dbb * pb_d
    } else {
        Decimal::ZERO
    };

    // HODL baseline value for IL: value of baseline basket at current prices (pool legs).
    let hodl_value_usd = if pool_mints.len() == 2 && (!pa_d.is_zero() || !pb_d.is_zero()) {
        baseline_a * pa_d + baseline_b * pb_d
    } else {
        baseline_value
    };

    let il_usd = current_value - hodl_value_usd;
    let il_pct = if hodl_value_usd.is_zero() {
        Decimal::ZERO
    } else {
        il_usd / hodl_value_usd
    };

    let net_pnl_usd = current_value + realized_cashflow_usd - baseline_value - tx_fees_usd;
    let net_pnl_pct = if baseline_value.is_zero() {
        Decimal::ZERO
    } else {
        net_pnl_usd / baseline_value
    };

    let hodl_basket_ok = pool_mints.len() == 2;

    Ok(PositionStreamPnLResponse {
        position_address: position_address.to_string(),
        baseline_ts_utc: baseline_ts.map(|t| t.to_rfc3339()),
        current_ts_utc: current_ts.map(|t| t.to_rfc3339()),
        baseline_value_usd: baseline_value,
        current_value_usd: current_value,
        hodl_value_usd,
        il_usd,
        il_pct,
        tx_fees_usd,
        realized_cashflow_usd,
        net_pnl_usd,
        net_pnl_pct,
        interpretation: stream_pnl_interpretation_pl(use_lineage_anchor, hodl_basket_ok),
        note: Some(format!(
            "Best-effort.{anchor} IL/HODL: baseline basket (open amounts at chain start) × current mint USD prices when pool mints are known ({price_src}). tx fees in USD use SOL/USD ({sol_src}). realized_cashflow uses lifecycle fee_payer_token_deltas × mint USD prices ({price_src}). cost/cashflow scope={scope}.",
            anchor = if use_lineage_anchor {
                " LP mark vs HODL uses first→last position in rotation lineage;"
            } else {
                ""
            },
            scope = if use_chain_session_scope {
                "chain sessions only"
            } else {
                "chain positions fallback"
            }
        )),
    })
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::{chain_session_ids_from_edges, pool_mints_for_hodl};

    #[test]
    fn pool_mints_prefers_baseline_over_current() {
        let v = pool_mints_for_hodl(
            (Some("A".into()), Some("B".into())),
            (Some("C".into()), Some("D".into())),
        );
        assert_eq!(v, vec!["A", "B"]);
    }

    #[test]
    fn pool_mints_falls_back_to_current_when_baseline_missing() {
        let v = pool_mints_for_hodl((None, None), (Some("M1".into()), Some("M2".into())));
        assert_eq!(v, vec!["M1", "M2"]);
    }

    #[test]
    fn pool_mints_mixed_fallback_per_leg() {
        let v = pool_mints_for_hodl((Some("  A  ".into()), None), (None, Some("B2".into())));
        assert_eq!(v, vec!["A", "B2"]);
    }

    #[test]
    fn chain_sessions_ignore_fork_edges_outside_ordered_chain() {
        let chain = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        let edges = vec![
            ("s1".to_string(), "A".to_string(), "B".to_string()),
            ("s2".to_string(), "B".to_string(), "C".to_string()),
            ("sx".to_string(), "A".to_string(), "X".to_string()),
        ];
        let out = chain_session_ids_from_edges(&chain, &edges);
        assert_eq!(out, vec!["s1".to_string(), "s2".to_string()]);
    }

    #[test]
    fn chain_sessions_empty_for_single_node_chain() {
        let chain = vec!["A".to_string()];
        let edges = vec![("s1".to_string(), "A".to_string(), "B".to_string())];
        let out = chain_session_ids_from_edges(&chain, &edges);
        assert!(out.is_empty());
    }
}

pub async fn compute_position_stream_pnl(
    state: &AppState,
    position_address: &str,
) -> Result<PositionStreamPnLResponse, ApiError> {
    if state.db.is_none() {
        return Ok(stream_pnl_db_disabled_response(position_address));
    }

    // Reuse stream connectivity from the existing endpoint implementation.
    let perf = compute_position_stream_performance(state, position_address, false).await?;
    let lineage_chain =
        resolve_lineage_chain_for_stream_pnl(state, &perf, position_address.trim()).await;
    compute_position_stream_pnl_for_stream_members(
        state,
        position_address,
        perf.positions,
        perf.sessions,
        Some(lineage_chain.as_slice()),
        true,
        false,
    )
    .await
}

pub async fn compute_position_stream_pnl_settlement_v1(
    state: &AppState,
    position_address: &str,
) -> Result<PositionStreamPnLResponse, ApiError> {
    if state.db.is_none() {
        return Ok(stream_pnl_db_disabled_response(position_address));
    }
    let perf = compute_position_stream_performance(state, position_address, false).await?;
    let lineage_chain =
        resolve_lineage_chain_for_stream_pnl(state, &perf, position_address.trim()).await;
    compute_position_stream_pnl_for_stream_members(
        state,
        position_address,
        perf.positions,
        perf.sessions,
        Some(lineage_chain.as_slice()),
        false,
        true,
    )
    .await
}
