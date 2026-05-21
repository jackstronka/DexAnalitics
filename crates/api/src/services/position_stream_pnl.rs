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
use crate::services::position_stream_lineage::{
    is_lifecycle_close_event, is_lifecycle_open_event, lp_fees_collected_usd_from_ledger_db,
    resolve_lineage_chain_for_stream_pnl,
};
use crate::services::position_stream_performance::compute_position_stream_performance;
use crate::services::position_valuation::{
    compute_position_usd_valuation, fetch_prices_for_positions, monitored_position_from_chain,
    PositionUsdValuation,
};
use crate::services::price_fetch::fetch_mint_prices_usd;
use crate::state::AppState;
use chrono::{DateTime, Utc};
use clmm_lp_domain::metrics::impermanent_loss::calculate_il_concentrated;
use clmm_lp_execution::monitor::MonitoredPosition;
use clmm_lp_protocols::prelude::tick_to_price;
use rust_decimal::Decimal;
use serde_json::Value;
use sqlx::Row;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::str::FromStr;
use tokio::time::{Duration, timeout};

const WSOL_MINT: &str = "So11111111111111111111111111111111111111112";

fn snapshot_kind_from_row(row: &sqlx::postgres::PgRow) -> Option<String> {
    row.try_get::<Option<Value>, _>("raw_json")
        .ok()
        .flatten()
        .and_then(|v| {
            v.get("kind")
                .and_then(|k| k.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        })
}

fn current_snapshot_is_stale_open_baseline(
    current: &sqlx::postgres::PgRow,
    baseline: &sqlx::postgres::PgRow,
) -> bool {
    current_snapshot_is_stale_open_baseline_values(
        snapshot_kind_from_row(current).as_deref(),
        current.try_get("ts_utc").ok(),
        baseline.try_get("ts_utc").ok(),
    )
}

fn current_snapshot_is_stale_open_baseline_values(
    current_kind: Option<&str>,
    current_ts: Option<DateTime<Utc>>,
    baseline_ts: Option<DateTime<Utc>>,
) -> bool {
    if current_kind == Some("end_close") {
        return false;
    }
    if current_kind == Some("baseline_open") {
        return true;
    }
    current_ts.is_some() && current_ts == baseline_ts
}

fn is_lifecycle_principal_event(ev: Option<&str>) -> bool {
    is_lifecycle_open_event(ev) || is_lifecycle_close_event(ev)
}

async fn seed_live_current_snapshot(
    state: &AppState,
    db: &clmm_lp_data::repositories::Database,
    seed_pk: &str,
) -> Result<(), ApiError> {
    let pk = solana_sdk::pubkey::Pubkey::from_str(seed_pk.trim())
        .map_err(|e| ApiError::internal(format!("stream pnl: invalid seed pubkey: {e}")))?;
    let Ok(Ok(pos)) = timeout(
        Duration::from_secs(2),
        monitored_position_from_chain(state.provider.clone(), &pk),
    )
    .await
    else {
        return Ok(());
    };
    let prices = fetch_prices_for_positions(state.provider.clone(), std::slice::from_ref(&pos)).await;
    let Ok(v) = compute_position_usd_valuation(state.provider.clone(), &pos, &prices).await else {
        return Ok(());
    };
    let raw = serde_json::json!({
        "position": pos.address.to_string(),
        "pool": pos.pool.to_string(),
        "kind": "live_current",
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
    Ok(())
}

async fn refetch_current_snapshot_row(
    db: &clmm_lp_data::repositories::Database,
    end_pubkey: Option<&str>,
    positions: &[String],
) -> Result<Option<sqlx::postgres::PgRow>, ApiError> {
    if let Some(pk) = end_pubkey {
        sqlx::query(CURRENT_SNAPSHOT_LAST_PDA_SQL)
            .bind(pk)
            .fetch_optional(db.pool())
            .await
            .map_err(|e| ApiError::internal(format!("stream pnl: current query (after seed): {e}")))
    } else {
        sqlx::query(CURRENT_SNAPSHOT_SQL)
            .bind(positions)
            .fetch_optional(db.pool())
            .await
            .map_err(|e| ApiError::internal(format!("stream pnl: current query (after seed): {e}")))
    }
}

fn accumulate_cashflow_mint_deltas(
    mint_deltas: &mut BTreeMap<String, Decimal>,
    rows: &[sqlx::postgres::PgRow],
) {
    for r in rows {
        let ev: Option<String> = r.try_get("event").ok();
        let v: Option<Value> = r.try_get("fee_payer_token_deltas").ok();
        apply_cashflow_fee_payer_deltas(mint_deltas, ev.as_deref(), v.as_ref());
    }
}

fn apply_cashflow_fee_payer_deltas(
    mint_deltas: &mut BTreeMap<String, Decimal>,
    event: Option<&str>,
    fee_payer_token_deltas: Option<&Value>,
) {
    if is_lifecycle_principal_event(event) {
        return;
    }
    let Some(Value::Object(map)) = fee_payer_token_deltas else {
        return;
    };
    for (mint, dv) in map {
        if let Some(d) = decimal_from_json(dv) {
            *mint_deltas.entry(mint.clone()).or_insert(Decimal::ZERO) += d;
        }
    }
}

/// Earliest snapshot across stream members — must include mint columns or HODL/IL falls back incorrectly.
const BASELINE_SNAPSHOT_SQL: &str = r#"
        SELECT ts_utc, value_usd, amount_a_ui, amount_b_ui, pool_pubkey, token_mint_a, token_mint_b,
               price_a_usd, price_b_usd, price_source, raw_json
        FROM position_stream_valuation_snapshots
        WHERE position_pubkey = ANY($1)
        ORDER BY ts_utc ASC
        LIMIT 1
        "#;

/// Latest snapshot across stream members.
const CURRENT_SNAPSHOT_SQL: &str = r#"
        SELECT ts_utc, value_usd, amount_a_ui, amount_b_ui, pool_pubkey, token_mint_a, token_mint_b,
               price_a_usd, price_b_usd, price_source, raw_json
        FROM position_stream_valuation_snapshots
        WHERE position_pubkey = ANY($1)
        ORDER BY ts_utc DESC
        LIMIT 1
        "#;

/// First PDA in lineage: prefer explicit open snapshot (same ordering as lineage `node_metrics`).
const BASELINE_SNAPSHOT_FIRST_PDA_SQL: &str = r#"
        SELECT ts_utc, value_usd, amount_a_ui, amount_b_ui, pool_pubkey, token_mint_a, token_mint_b,
               price_a_usd, price_b_usd, price_source, raw_json
        FROM position_stream_valuation_snapshots
        WHERE position_pubkey = $1
        ORDER BY
          CASE WHEN COALESCE(raw_json->>'kind', '') = 'baseline_open' THEN 0 ELSE 1 END,
          ts_utc ASC
        LIMIT 1
        "#;

/// Last PDA in lineage: prefer close/end snapshot (same ordering as lineage `node_metrics`).
const CURRENT_SNAPSHOT_LAST_PDA_SQL: &str = r#"
        SELECT ts_utc, value_usd, amount_a_ui, amount_b_ui, pool_pubkey, token_mint_a, token_mint_b,
               price_a_usd, price_b_usd, price_source, raw_json
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

fn ratio_or_zero(num: Decimal, den: Decimal) -> Decimal {
    if den.is_zero() {
        Decimal::ZERO
    } else {
        num / den
    }
}

fn raw_json_str(raw: Option<&Value>, key: &str) -> Option<String> {
    raw.and_then(|v| v.get(key))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
}

fn is_end_close_snapshot(raw: Option<&Value>) -> bool {
    raw_json_str(raw, "kind").as_deref() == Some("end_close")
}

fn snapshot_price_time_kind(raw: Option<&Value>) -> Option<String> {
    raw_json_str(raw, "price_time_kind")
}

fn positive_price_pair(pa: Decimal, pb: Decimal) -> Option<(Decimal, Decimal)> {
    (pa > Decimal::ZERO && pb > Decimal::ZERO).then_some((pa, pb))
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct StreamIlComponents {
    clean_il_usd: Decimal,
    clean_il_pct: Decimal,
    lp_fees_total_usd: Decimal,
    lp_vs_hodl_with_fees_usd: Decimal,
    lp_vs_hodl_with_fees_pct: Decimal,
}

fn compute_stream_il_components(
    current_value_usd: Decimal,
    hodl_value_usd: Decimal,
    realized_lp_fees_usd: Decimal,
    uncollected_lp_fees_usd: Decimal,
) -> StreamIlComponents {
    let clean_il_usd = current_value_usd - hodl_value_usd;
    let clean_il_pct = ratio_or_zero(clean_il_usd, hodl_value_usd);
    let lp_fees_total_usd = realized_lp_fees_usd + uncollected_lp_fees_usd;
    let lp_vs_hodl_with_fees_usd = clean_il_usd + lp_fees_total_usd;
    let lp_vs_hodl_with_fees_pct = ratio_or_zero(lp_vs_hodl_with_fees_usd, hodl_value_usd);
    StreamIlComponents {
        clean_il_usd,
        clean_il_pct,
        lp_fees_total_usd,
        lp_vs_hodl_with_fees_usd,
        lp_vs_hodl_with_fees_pct,
    }
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
        clean_il_usd: Decimal::ZERO,
        clean_il_pct: Decimal::ZERO,
        realized_lp_fees_usd: Decimal::ZERO,
        uncollected_lp_fees_usd: Decimal::ZERO,
        lp_fees_total_usd: Decimal::ZERO,
        lp_vs_hodl_with_fees_usd: Decimal::ZERO,
        lp_vs_hodl_with_fees_pct: Decimal::ZERO,
        valuation_price_time_kind: "unavailable".to_string(),
        price_basis_note: None,
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
        "Benchmark IL vs HODL: wartość LP na końcu łańcucha minus hipotetyczna wartość trzymania tokenów z depozytu na początku łańcucha, przy cenach USD z końcowego eventu close albo z live/fallback feedu dla aktywnej pozycji. Inna definicja niż net PnL (bez pełnej księgowości między rotacjami).",
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

    let needs_live_current = allow_self_seed
        && !settlement_strict
        && match (&current_row, &baseline_row) {
            (None, Some(_)) => true,
            (Some(c), Some(b)) => current_snapshot_is_stale_open_baseline(c, b),
            _ => false,
        };

    if needs_live_current {
        seed_live_current_snapshot(state, db, seed_pk_for_current).await?;
        current_row = refetch_current_snapshot_row(db, end_pubkey, &positions).await?;
    } else if current_row.is_none() && allow_self_seed && !settlement_strict {
        seed_live_current_snapshot(state, db, seed_pk_for_current).await?;
        current_row = refetch_current_snapshot_row(db, end_pubkey, &positions).await?;
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
            clean_il_usd: Decimal::ZERO,
            clean_il_pct: Decimal::ZERO,
            realized_lp_fees_usd: Decimal::ZERO,
            uncollected_lp_fees_usd: Decimal::ZERO,
            lp_fees_total_usd: Decimal::ZERO,
            lp_vs_hodl_with_fees_usd: Decimal::ZERO,
            lp_vs_hodl_with_fees_pct: Decimal::ZERO,
            valuation_price_time_kind: "unavailable".to_string(),
            price_basis_note: None,
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

    let (current_ts, current_value, current_ma, current_mb, current_pa, current_pb, current_raw) =
        if let Some(c) = current_row {
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
                c.try_get::<Decimal, _>("price_a_usd")
                    .unwrap_or(Decimal::ZERO),
                c.try_get::<Decimal, _>("price_b_usd")
                    .unwrap_or(Decimal::ZERO),
                c.try_get::<Option<Value>, _>("raw_json").ok().flatten(),
            )
        } else {
            (
                None,
                Decimal::ZERO,
                None,
                None,
                Decimal::ZERO,
                Decimal::ZERO,
                None,
            )
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
    // Exclude open/close principal legs (deposit/withdrawal), same as per-node lineage metrics.
    let rows = if use_chain_session_scope {
        sqlx::query(
            r#"SELECT event, fee_payer_token_deltas
               FROM position_stream_ledger_rows
               WHERE rebalance_session_id = ANY($1) AND fee_payer_token_deltas IS NOT NULL"#,
        )
        .bind(&scoped_sessions)
        .fetch_all(db.pool())
        .await
        .map_err(|e| ApiError::internal(format!("stream pnl: token deltas rows: {e}")))?
    } else if !chain_vec.is_empty() {
        sqlx::query(
            r#"SELECT event, fee_payer_token_deltas
               FROM position_stream_ledger_rows
               WHERE position_pubkey = ANY($1) AND fee_payer_token_deltas IS NOT NULL"#,
        )
        .bind(&chain_vec)
        .fetch_all(db.pool())
        .await
        .map_err(|e| ApiError::internal(format!("stream pnl: token deltas rows: {e}")))?
    } else if !sessions.is_empty() {
        sqlx::query(
            r#"SELECT event, fee_payer_token_deltas
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
    accumulate_cashflow_mint_deltas(&mut mint_deltas, &rows);

    // Use mints from baseline snapshot (fallback: latest snapshot) for HODL/IL and cashflow conversion.
    let pool_mints = pool_mints_for_hodl(
        (baseline_ma.clone(), baseline_mb.clone()),
        (current_ma, current_mb),
    );
    let current_kind = snapshot_price_time_kind(current_raw.as_ref());
    let current_is_end_close = is_end_close_snapshot(current_raw.as_ref());
    let event_price_pair = current_kind
        .as_deref()
        .is_some_and(|k| k == "at_tx_event")
        .then(|| positive_price_pair(current_pa, current_pb))
        .flatten();
    let (pa_d, pb_d, price_src, valuation_price_time_kind, price_basis_note) =
        if current_is_end_close {
            if let Some((pa, pb)) = event_price_pair {
                (
                    pa,
                    pb,
                    raw_json_str(current_raw.as_ref(), "source")
                        .unwrap_or_else(|| "event_snapshot".to_string()),
                    "at_tx_event".to_string(),
                    Some(
                        "HODL/IL uses USD prices captured on the final close snapshot.".to_string(),
                    ),
                )
            } else {
                let mint_set: BTreeSet<String> = pool_mints.iter().cloned().collect();
                let (px, src) =
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
                (
                    Decimal::from_f64_retain(pa).unwrap_or(Decimal::ZERO),
                    Decimal::from_f64_retain(pb).unwrap_or(Decimal::ZERO),
                    src,
                    "free_price_fallback".to_string(),
                    Some("Final close snapshot lacked usable event-time prices; HODL/IL uses current free-price fallback.".to_string()),
                )
            }
        } else {
            let mint_set: BTreeSet<String> = pool_mints.iter().cloned().collect();
            let (px, src) =
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
            (
                Decimal::from_f64_retain(pa).unwrap_or(Decimal::ZERO),
                Decimal::from_f64_retain(pb).unwrap_or(Decimal::ZERO),
                src,
                "live_price".to_string(),
                Some(
                    "Open stream: HODL/IL uses current free USD prices for the final live mark."
                        .to_string(),
                ),
            )
        };

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

    let mut realized_lp_fees_usd = Decimal::ZERO;
    let mut fee_positions: BTreeSet<String> = BTreeSet::new();
    for p in &chain_vec {
        if !p.trim().is_empty() {
            fee_positions.insert(p.trim().to_string());
        }
    }
    if fee_positions.is_empty() {
        fee_positions.insert(position_address.trim().to_string());
    }
    for p in &fee_positions {
        let (_events, usd, _by_mint) = lp_fees_collected_usd_from_ledger_db(state, db, p).await?;
        realized_lp_fees_usd += usd;
    }

    let uncollected_lp_fees_usd = if !current_is_end_close && !settlement_strict {
        let seed = end_pubkey.unwrap_or_else(|| position_address.trim());
        if let Ok(pk) = solana_sdk::pubkey::Pubkey::from_str(seed)
            && let Ok(Ok(pos)) = timeout(
                Duration::from_secs(2),
                monitored_position_from_chain(state.provider.clone(), &pk),
            )
            .await
        {
            let prices =
                fetch_prices_for_positions(state.provider.clone(), std::slice::from_ref(&pos))
                    .await;
            match compute_position_usd_valuation(state.provider.clone(), &pos, &prices).await {
                Ok(v) => v.fees_usd.max(Decimal::ZERO),
                Err(_) => Decimal::ZERO,
            }
        } else {
            Decimal::ZERO
        }
    } else {
        Decimal::ZERO
    };

    let il_components = compute_stream_il_components(
        current_value,
        hodl_value_usd,
        realized_lp_fees_usd,
        uncollected_lp_fees_usd,
    );
    let clean_il_usd = il_components.clean_il_usd;
    let clean_il_pct = il_components.clean_il_pct;
    let il_usd = clean_il_usd;
    let il_pct = clean_il_pct;
    let lp_fees_total_usd = il_components.lp_fees_total_usd;
    let lp_vs_hodl_with_fees_usd = il_components.lp_vs_hodl_with_fees_usd;
    let lp_vs_hodl_with_fees_pct = il_components.lp_vs_hodl_with_fees_pct;

    let net_pnl_usd = current_value + realized_cashflow_usd - baseline_value - tx_fees_usd;
    let net_pnl_pct = ratio_or_zero(net_pnl_usd, baseline_value);

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
        clean_il_usd,
        clean_il_pct,
        realized_lp_fees_usd,
        uncollected_lp_fees_usd,
        lp_fees_total_usd,
        lp_vs_hodl_with_fees_usd,
        lp_vs_hodl_with_fees_pct,
        valuation_price_time_kind: valuation_price_time_kind.clone(),
        price_basis_note: price_basis_note.clone(),
        tx_fees_usd,
        realized_cashflow_usd,
        net_pnl_usd,
        net_pnl_pct,
        interpretation: stream_pnl_interpretation_pl(use_lineage_anchor, hodl_basket_ok),
        note: Some(format!(
            "Best-effort.{anchor} IL/HODL: baseline basket (open amounts at chain start) × valuation USD prices ({price_src}, price_time_kind={valuation_price_time_kind}). clean_il excludes LP fees; lp_vs_hodl_with_fees adds realized LP fees + active uncollected fees. tx fees in USD use SOL/USD ({sol_src}). realized_cashflow uses lifecycle fee_payer_token_deltas × valuation mint USD prices ({price_src}) and is broader than LP fees. cost/cashflow scope={scope}.",
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

/// Performance-card PnL for `GET /positions/{address}` (single PDA, not full rotation chain).
#[derive(Debug, Clone, Copy)]
pub struct SinglePositionDetailPnL {
    pub net_pnl_usd: Decimal,
    pub net_pnl_pct: Decimal,
    pub il_pct: Decimal,
}

fn pool_price_b_per_a(price_a_usd: f64, price_b_usd: f64) -> Option<Decimal> {
    if price_a_usd.is_finite() && price_a_usd > 0.0 && price_b_usd.is_finite() && price_b_usd > 0.0 {
        Decimal::from_f64_retain(price_b_usd / price_a_usd)
    } else {
        None
    }
}

async fn baseline_usd_from_snapshot_row(row: &sqlx::postgres::PgRow) -> Option<Decimal> {
    let value_usd: Decimal = row.try_get("value_usd").ok().unwrap_or(Decimal::ZERO);
    if value_usd > Decimal::ZERO {
        return Some(value_usd);
    }

    let raw_json: Value = row
        .try_get::<Value, _>("raw_json")
        .unwrap_or_else(|_| Value::Object(Default::default()));
    let mut aa = row
        .try_get::<Option<Decimal>, _>("amount_a_ui")
        .ok()
        .flatten();
    let mut bb = row
        .try_get::<Option<Decimal>, _>("amount_b_ui")
        .ok()
        .flatten();
    let mut pa = row
        .try_get::<Option<Decimal>, _>("price_a_usd")
        .ok()
        .flatten();
    let mut pb = row
        .try_get::<Option<Decimal>, _>("price_b_usd")
        .ok()
        .flatten();
    if let Some(o) = raw_json.as_object() {
        if aa.is_none() {
            aa = o.get("amount_a_ui").and_then(decimal_from_json);
        }
        if bb.is_none() {
            bb = o.get("amount_b_ui").and_then(decimal_from_json);
        }
        if pa.is_none() {
            pa = o.get("price_a_usd").and_then(decimal_from_json);
        }
        if pb.is_none() {
            pb = o.get("price_b_usd").and_then(decimal_from_json);
        }
    }
    let aa = aa.unwrap_or(Decimal::ZERO);
    let bb = bb.unwrap_or(Decimal::ZERO);
    if aa.is_zero() && bb.is_zero() {
        return None;
    }

    let mut mints: BTreeSet<String> = BTreeSet::new();
    for key in ["token_mint_a", "token_mint_b"] {
        if let Some(m) = row.try_get::<Option<String>, _>(key).ok().flatten() {
            let t = m.trim();
            if !t.is_empty() {
                mints.insert(t.to_string());
            }
        }
    }
    if let Some(o) = raw_json.as_object() {
        for key in ["token_mint_a", "token_mint_b"] {
            if let Some(m) = o.get(key).and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty())
            {
                mints.insert(m.to_string());
            }
        }
    }
    if !mints.is_empty() {
        let (px, _) = match timeout(Duration::from_secs(3), fetch_mint_prices_usd(&mints)).await {
            Ok(v) => v,
            Err(_) => (BTreeMap::new(), "timeout".to_string()),
        };
        if pa.is_none_or(|p| p <= Decimal::ZERO) {
            if let Some(m) = row.try_get::<Option<String>, _>("token_mint_a").ok().flatten() {
                if let Some(f) = px.get(m.trim()) {
                    pa = Decimal::from_f64_retain(*f);
                }
            }
        }
        if pb.is_none_or(|p| p <= Decimal::ZERO) {
            if let Some(m) = row.try_get::<Option<String>, _>("token_mint_b").ok().flatten() {
                if let Some(f) = px.get(m.trim()) {
                    pb = Decimal::from_f64_retain(*f);
                }
            }
        }
    }
    let pa = pa.unwrap_or(Decimal::ZERO);
    let pb = pb.unwrap_or(Decimal::ZERO);
    let nav = aa * pa + bb * pb;
    (nav > Decimal::ZERO).then_some(nav)
}

fn baseline_entry_pool_price_from_snapshot_row(row: &sqlx::postgres::PgRow) -> Option<Decimal> {
    let pa = row
        .try_get::<Option<Decimal>, _>("price_a_usd")
        .ok()
        .flatten()
        .filter(|p| *p > Decimal::ZERO);
    let pb = row
        .try_get::<Option<Decimal>, _>("price_b_usd")
        .ok()
        .flatten()
        .filter(|p| *p > Decimal::ZERO);
    if let (Some(a), Some(b)) = (pa, pb) {
        let r = ratio_or_zero(b, a);
        return (r > Decimal::ZERO).then_some(r);
    }
    let raw_json: Value = row.try_get("raw_json").unwrap_or(Value::Null);
    let pa = raw_json
        .get("price_a_usd")
        .and_then(decimal_from_json)
        .filter(|p| *p > Decimal::ZERO);
    let pb = raw_json
        .get("price_b_usd")
        .and_then(decimal_from_json)
        .filter(|p| *p > Decimal::ZERO);
    match (pa, pb) {
        (Some(a), Some(b)) => {
            let r = ratio_or_zero(b, a);
            (r > Decimal::ZERO).then_some(r)
        }
        _ => None,
    }
}

/// Baseline USD for one PDA: valuation snapshot → ledger open quote → monitor entry.
pub async fn baseline_value_usd_for_single_position(
    state: &AppState,
    position_pubkey: &str,
    monitor_entry_value_usd: Decimal,
) -> Option<Decimal> {
    if monitor_entry_value_usd > Decimal::ZERO {
        return Some(monitor_entry_value_usd);
    }
    let pk = position_pubkey.trim();
    if pk.is_empty() {
        return None;
    }
    if let Some(db) = state.db.as_ref() {
        if let Ok(Some(row)) = sqlx::query(BASELINE_SNAPSHOT_FIRST_PDA_SQL)
            .bind(pk)
            .fetch_optional(db.pool())
            .await
            && let Some(usd) = baseline_usd_from_snapshot_row(&row).await
        {
            return Some(usd);
        }
        if let Ok(Some(row)) = sqlx::query(
            r#"
            SELECT COALESCE(
              NULLIF(raw_json #>> '{details,open_quote_estimated_value_usd}', '')::DOUBLE PRECISION,
              NULLIF(raw_json #>> '{details,open_target_usd}', '')::DOUBLE PRECISION,
              NULLIF(raw_json #>> '{details,open_prev_end_value_usd}', '')::DOUBLE PRECISION
            ) AS open_usd
            FROM position_stream_ledger_rows
            WHERE position_pubkey = $1
              AND event IN ('bot_open_position', 'bot_open_position_full_range', 'position_open')
            ORDER BY ts_utc DESC
            LIMIT 1
            "#,
        )
        .bind(pk)
        .fetch_optional(db.pool())
        .await
        {
            let open_usd: Option<f64> = row.try_get("open_usd").ok().flatten();
            if let Some(f) = open_usd.and_then(Decimal::from_f64_retain).filter(|d| *d > Decimal::ZERO)
            {
                return Some(f);
            }
        }
    }
    None
}

/// Net PnL + IL for the position detail performance card (uses live valuation + DB open baseline).
pub async fn compute_single_position_detail_pnl(
    state: &AppState,
    position: &MonitoredPosition,
    current_value_usd: Decimal,
    valuation: &PositionUsdValuation,
) -> Option<SinglePositionDetailPnL> {
    if current_value_usd <= Decimal::ZERO {
        return None;
    }
    let baseline = baseline_value_usd_for_single_position(
        state,
        &position.address.to_string(),
        position.pnl.entry_value_usd,
    )
    .await?;
    if baseline <= Decimal::ZERO {
        return None;
    }
    let net_pnl_usd = current_value_usd - baseline;
    let net_pnl_pct = ratio_or_zero(net_pnl_usd, baseline);

    let current_price = pool_price_b_per_a(valuation.price_a_usd, valuation.price_b_usd)?;
    let entry_price = if let Some(ep) = position.pnl.entry_price.filter(|p| *p > Decimal::ZERO) {
        Some(ep)
    } else if let Some(db) = state.db.as_ref() {
        sqlx::query(BASELINE_SNAPSHOT_FIRST_PDA_SQL)
            .bind(position.address.to_string())
            .fetch_optional(db.pool())
            .await
            .ok()
            .flatten()
            .and_then(|row| baseline_entry_pool_price_from_snapshot_row(&row))
    } else {
        None
    };
    let il_pct = entry_price
        .and_then(|ep| {
            let lower = tick_to_price(position.on_chain.tick_lower);
            let upper = tick_to_price(position.on_chain.tick_upper);
            calculate_il_concentrated(ep, current_price, lower, upper).ok()
        })
        .unwrap_or(Decimal::ZERO);

    Some(SinglePositionDetailPnL {
        net_pnl_usd,
        net_pnl_pct,
        il_pct,
    })
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::{
        apply_cashflow_fee_payer_deltas, chain_session_ids_from_edges, compute_stream_il_components,
        current_snapshot_is_stale_open_baseline_values, is_end_close_snapshot, ratio_or_zero,
        is_lifecycle_principal_event, pool_mints_for_hodl, positive_price_pair,
        snapshot_price_time_kind,
    };
    use chrono::{TimeZone, Utc};
    use rust_decimal::Decimal;
    use std::collections::BTreeMap;
    use std::str::FromStr;

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

    #[test]
    fn end_close_event_price_helpers_accept_only_complete_positive_prices() {
        let raw = serde_json::json!({
            "kind": "end_close",
            "price_time_kind": "at_tx_event"
        });
        assert!(is_end_close_snapshot(Some(&raw)));
        assert_eq!(
            snapshot_price_time_kind(Some(&raw)).as_deref(),
            Some("at_tx_event")
        );
        assert_eq!(
            positive_price_pair(Decimal::from(95), Decimal::ONE),
            Some((Decimal::from(95), Decimal::ONE))
        );
        assert!(positive_price_pair(Decimal::ZERO, Decimal::ONE).is_none());
    }

    #[test]
    fn single_position_net_pnl_from_baseline_matches_stream_ratio_scale() {
        let baseline = Decimal::from_str("9.973").unwrap();
        let current = Decimal::from_str("9.938").unwrap();
        let net = current - baseline;
        let pct = ratio_or_zero(net, baseline);
        assert!(net < Decimal::ZERO);
        assert!(pct < Decimal::ZERO);
        assert!(pct > Decimal::new(-1, 2));
    }

    #[test]
    fn stream_il_components_keep_clean_il_separate_from_lp_fees() {
        let c = compute_stream_il_components(
            Decimal::from(90),
            Decimal::from(100),
            Decimal::from(7),
            Decimal::from(2),
        );
        assert_eq!(c.clean_il_usd, Decimal::from(-10));
        assert_eq!(c.clean_il_pct, Decimal::new(-1, 1));
        assert_eq!(c.lp_fees_total_usd, Decimal::from(9));
        assert_eq!(c.lp_vs_hodl_with_fees_usd, Decimal::from(-1));
        assert_eq!(c.lp_vs_hodl_with_fees_pct, Decimal::new(-1, 2));
    }

    #[test]
    fn cashflow_skips_open_and_close_principal_events() {
        assert!(is_lifecycle_principal_event(Some("bot_open_position")));
        assert!(is_lifecycle_principal_event(Some("position_open")));
        assert!(is_lifecycle_principal_event(Some("bot_close_position")));

        let deltas = serde_json::json!({
            "MINTA": "-5.0",
            "MINTB": "-4.859"
        });
        let mut mint_deltas = BTreeMap::new();
        apply_cashflow_fee_payer_deltas(
            &mut mint_deltas,
            Some("bot_open_position"),
            Some(&deltas),
        );
        assert!(mint_deltas.is_empty());

        apply_cashflow_fee_payer_deltas(
            &mut mint_deltas,
            Some("bot_rebalance_swap"),
            Some(&deltas),
        );
        assert_eq!(mint_deltas.get("MINTA"), Some(&Decimal::from_str("-5").unwrap()));
        assert_eq!(
            mint_deltas.get("MINTB"),
            Some(&Decimal::from_str("-4.859").unwrap())
        );
    }

    #[test]
    fn current_snapshot_stale_when_baseline_open_or_same_ts() {
        let ts = Utc.with_ymd_and_hms(2026, 5, 20, 12, 0, 0).unwrap();
        assert!(current_snapshot_is_stale_open_baseline_values(
            Some("baseline_open"),
            Some(ts),
            Some(ts),
        ));
        assert!(current_snapshot_is_stale_open_baseline_values(
            Some("live_current"),
            Some(ts),
            Some(ts),
        ));
        assert!(!current_snapshot_is_stale_open_baseline_values(
            Some("live_current"),
            Some(ts + chrono::Duration::minutes(5)),
            Some(ts),
        ));
        assert!(!current_snapshot_is_stale_open_baseline_values(
            Some("end_close"),
            Some(ts),
            Some(ts),
        ));
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
