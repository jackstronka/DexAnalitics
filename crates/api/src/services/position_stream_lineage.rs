//! Stream lineage: ordered chain of rotated position PDAs + per-node aggregates.
//!
//! We already persist edges old->new in `position_stream_edges` (best-effort from IL ledger).
//! This service builds a best-effort *linear* chain (root → ... → current) and enriches each node
//! with valuation snapshots + ledger aggregates so the UI can show "history of positions".

use crate::error::ApiError;
use crate::models::{
    LineageChainCostSummary, LineageCollectZeroDiagnostics, PositionStreamLineageNode,
    PositionStreamLineageResponse,
};
use crate::services::position_stream_performance::compute_position_stream_performance;
use crate::services::position_valuation::{
    compute_position_usd_valuation, fetch_prices_for_positions, monitored_position_from_chain,
};
use crate::services::price_fetch::fetch_mint_prices_usd;
use crate::state::AppState;
use chrono::{DateTime, Utc};
use clmm_lp_data::repositories::Database;
use futures::future::join_all;
use rust_decimal::Decimal;
use rust_decimal::MathematicalOps;
use rust_decimal::prelude::ToPrimitive;
use spl_token::solana_program::program_pack::Pack;
use spl_token::state::Mint;
use sqlx::Row;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader};
use std::str::FromStr;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::SystemTime;
use tokio::time::{Duration, sleep, timeout};

const WSOL_MINT: &str = "So11111111111111111111111111111111111111112";
const WHETH_MINT: &str = "7vfCXTUXx5WJV5JADk17DUJ4ksgau7utNKj4b963voxs";

/// In-process cache: Whirlpool pool address → (token_mint_a, token_mint_b). Pool mints are immutable;
/// caching avoids N duplicate `get_account` RPCs when lineage builds many PDAs in parallel (public RPC
/// often times out under burst — previously produced empty LP-fee legs + misleading `collect_events`).
static POOL_TOKEN_MINTS_CACHE: OnceLock<RwLock<HashMap<String, (String, String)>>> =
    OnceLock::new();
const MINT_PRICE_CACHE_TTL_SECS: i64 = 15 * 60;

#[derive(Debug, Clone)]
struct CachedMintPrice {
    usd: f64,
    updated_at: DateTime<Utc>,
}

static MINT_PRICE_CACHE: OnceLock<RwLock<HashMap<String, CachedMintPrice>>> = OnceLock::new();

fn merge_live_with_cached_prices(
    requested_mints: &BTreeSet<String>,
    live: &BTreeMap<String, f64>,
    cache: &HashMap<String, CachedMintPrice>,
    now: DateTime<Utc>,
) -> (BTreeMap<String, f64>, bool) {
    let mut merged: BTreeMap<String, f64> = BTreeMap::new();
    let mut used_cache = false;
    for mint in requested_mints {
        if let Some(v) = live
            .get(mint)
            .copied()
            .filter(|v| v.is_finite() && *v > 0.0)
        {
            merged.insert(mint.clone(), v);
            continue;
        }
        if let Some(c) = cache.get(mint) {
            let age = now.signed_duration_since(c.updated_at).num_seconds();
            if age <= MINT_PRICE_CACHE_TTL_SECS && c.usd.is_finite() && c.usd > 0.0 {
                merged.insert(mint.clone(), c.usd);
                used_cache = true;
            }
        }
    }
    (merged, used_cache)
}

async fn fetch_mint_prices_usd_stable(mints: &BTreeSet<String>) -> (BTreeMap<String, f64>, String) {
    if mints.is_empty() {
        return (BTreeMap::new(), "none".to_string());
    }
    let now = Utc::now();
    let (live_prices, live_source) =
        match timeout(Duration::from_secs(2), fetch_mint_prices_usd(mints)).await {
            Ok((px, src)) => (px, src),
            Err(_) => (BTreeMap::new(), "timeout".to_string()),
        };

    let cache = MINT_PRICE_CACHE.get_or_init(|| RwLock::new(HashMap::new()));
    let mut source = live_source.clone();

    if let Ok(mut g) = cache.write() {
        for (mint, usd) in &live_prices {
            if usd.is_finite() && *usd > 0.0 {
                g.insert(
                    mint.clone(),
                    CachedMintPrice {
                        usd: *usd,
                        updated_at: now,
                    },
                );
            }
        }
        let (merged, used_cache) = merge_live_with_cached_prices(mints, &live_prices, &g, now);
        if used_cache {
            source = if live_prices.is_empty() {
                "cache_last_good".to_string()
            } else {
                format!("{live_source}+cache")
            };
        }
        return (merged, source);
    }

    (live_prices, source)
}

fn json_f64_for_event_price(v: Option<&serde_json::Value>) -> Option<f64> {
    v.and_then(|x| {
        x.as_f64()
            .or_else(|| x.as_i64().map(|i| i as f64))
            .or_else(|| x.as_str().and_then(|s| s.parse().ok()))
    })
    .filter(|p| p.is_finite() && *p > 0.0)
}

fn json_u64_for_event_slot(v: Option<&serde_json::Value>) -> Option<u64> {
    v.and_then(|x| {
        x.as_u64()
            .or_else(|| x.as_i64().and_then(|i| (i >= 0).then_some(i as u64)))
            .or_else(|| x.as_str().and_then(|s| s.parse().ok()))
    })
}

fn json_decimal_from_value(v: &serde_json::Value) -> Option<Decimal> {
    v.as_f64()
        .and_then(Decimal::from_f64_retain)
        .or_else(|| v.as_i64().and_then(|i| Decimal::try_from(i).ok()))
        .or_else(|| {
            v.as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .and_then(|s| s.parse().ok())
        })
}

/// Open NAV from the first `position_stream_valuation_snapshots` row for this PDA (same ordering as
/// [`node_metrics`]: prefer `raw_json.kind = baseline_open`, then earliest `ts_utc`).
///
/// Uses **amount_a_ui × price_a_usd + amount_b_ui × price_b_usd** from typed columns, falling back to
/// the same keys inside `raw_json`. Missing positive prices for a non‑zero leg are filled via
/// [`fetch_mint_prices_usd_stable`]. If recomputed NAV is still zero, returns persisted `value_usd` when
/// **> 0** (legacy rows that only stored the total).
async fn open_nav_usd_from_valuation_snapshot_row(
    _state: &AppState,
    row: &sqlx::postgres::PgRow,
) -> Option<Decimal> {
    let raw_json: serde_json::Value = row
        .try_get::<serde_json::Value, _>("raw_json")
        .unwrap_or_else(|_| serde_json::json!({}));
    let value_usd: Decimal = row.try_get("value_usd").ok().unwrap_or(Decimal::ZERO);

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
    let mut ma = row
        .try_get::<Option<String>, _>("token_mint_a")
        .ok()
        .flatten()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let mut mb = row
        .try_get::<Option<String>, _>("token_mint_b")
        .ok()
        .flatten()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    if let Some(o) = raw_json.as_object() {
        if aa.is_none() {
            aa = o.get("amount_a_ui").and_then(json_decimal_from_value);
        }
        if bb.is_none() {
            bb = o.get("amount_b_ui").and_then(json_decimal_from_value);
        }
        if pa.is_none() {
            pa = o.get("price_a_usd").and_then(json_decimal_from_value);
        }
        if pb.is_none() {
            pb = o.get("price_b_usd").and_then(json_decimal_from_value);
        }
        if ma.is_none() {
            ma = o
                .get("token_mint_a")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
        }
        if mb.is_none() {
            mb = o
                .get("token_mint_b")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
        }
    }

    let aa = aa.unwrap_or(Decimal::ZERO);
    let bb = bb.unwrap_or(Decimal::ZERO);
    if aa.is_zero() && bb.is_zero() {
        return (value_usd > Decimal::ZERO).then_some(value_usd);
    }

    let mut mints: BTreeSet<String> = BTreeSet::new();
    if let Some(ref m) = ma {
        mints.insert(m.clone());
    }
    if let Some(ref m) = mb {
        mints.insert(m.clone());
    }
    let (stable_px, _) = fetch_mint_prices_usd_stable(&mints).await;

    let pa_d = pa
        .or_else(|| {
            ma.as_ref()
                .and_then(|m| stable_px.get(m.as_str()).copied())
                .and_then(Decimal::from_f64_retain)
        })
        .unwrap_or(Decimal::ZERO);
    let pb_d = pb
        .or_else(|| {
            mb.as_ref()
                .and_then(|m| stable_px.get(m.as_str()).copied())
                .and_then(Decimal::from_f64_retain)
        })
        .unwrap_or(Decimal::ZERO);

    if !aa.is_zero() && pa_d <= Decimal::ZERO {
        return (value_usd > Decimal::ZERO).then_some(value_usd);
    }
    if !bb.is_zero() && pb_d <= Decimal::ZERO {
        return (value_usd > Decimal::ZERO).then_some(value_usd);
    }

    let nav = aa * pa_d + bb * pb_d;
    if nav > Decimal::ZERO {
        return Some(nav);
    }
    (value_usd > Decimal::ZERO).then_some(value_usd)
}

/// Ledger `details` fields written on successful bot open/close (`event_*` keys — see `doc/DATA_CATALOG.md`).
fn event_spot_from_ledger_details(
    details: Option<&serde_json::Value>,
) -> Option<(f64, f64, String, Option<u64>)> {
    let d = details?.as_object()?;
    let pa = json_f64_for_event_price(d.get("event_price_a_usd"))?;
    let pb = json_f64_for_event_price(d.get("event_price_b_usd"))?;
    let src = d
        .get("event_price_source")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    let slot = json_u64_for_event_slot(d.get("event_slot"));
    Some((pa, pb, src, slot))
}

/// Open NAV from lifecycle **open** row: leg UI amounts (`open_amount_raw` / caps / payer deltas) ×
/// **`event_price_a_usd` / `event_price_b_usd`** on the same row when present (bot persist path — see
/// `DATA_CATALOG.md`). If only one leg is priced in `details`, the other leg uses
/// [`fetch_mint_prices_usd_stable`] so the UI can show `event_price_a_usd` alone (e.g. SOL/USDC quote)
/// while still materializing **start NAV**.
/// Picks the **latest** qualifying open row by `ts_utc` (same spirit as open-quote merge).
async fn open_start_usd_from_event_spot_open_row(
    state: &AppState,
    rows: &[LifecycleRow],
    node: &PositionStreamLineageNode,
) -> Option<Decimal> {
    let mint_a = node
        .token_mint_a
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    let mint_b = node
        .token_mint_b
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    let pos = node.position_address.trim();
    if pos.is_empty() {
        return None;
    }
    let ma = solana_sdk::pubkey::Pubkey::from_str(mint_a).ok()?;
    let mb = solana_sdk::pubkey::Pubkey::from_str(mint_b).ok()?;
    let dec_a = fetch_mint_decimals_best_effort(state.provider.as_ref(), &ma).await?;
    let dec_b = fetch_mint_decimals_best_effort(state.provider.as_ref(), &mb).await?;

    let mut mints = BTreeSet::new();
    mints.insert(mint_a.to_string());
    mints.insert(mint_b.to_string());
    let (stable_px, _) = fetch_mint_prices_usd_stable(&mints).await;

    let mut best: Option<(DateTime<Utc>, Decimal)> = None;
    for r in rows {
        let Some(p) = r
            .position_pubkey
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        if p != pos {
            continue;
        }
        if !is_lifecycle_open_event(r.event.as_deref()) {
            continue;
        }
        let details_obj = r.details.as_ref().and_then(|v| v.as_object());
        let deltas_obj = r
            .fee_payer_token_deltas
            .as_ref()
            .and_then(|v| v.as_object());
        let (amount_a_ui, amount_b_ui, _) = baseline_open_amounts_ui_from_details_or_deltas(
            details_obj,
            deltas_obj,
            mint_a,
            mint_b,
            Some(dec_a),
            Some(dec_b),
        );
        if amount_a_ui.is_zero() && amount_b_ui.is_zero() {
            continue;
        }

        let (pa_e, pb_e) =
            if let Some((pa, pb, _, _)) = event_spot_from_ledger_details(r.details.as_ref()) {
                (Some(pa), Some(pb))
            } else {
                let Some(d) = details_obj else {
                    continue;
                };
                (
                    json_f64_for_event_price(d.get("event_price_a_usd")),
                    json_f64_for_event_price(d.get("event_price_b_usd")),
                )
            };

        let pa_d = pa_e
            .and_then(Decimal::from_f64_retain)
            .or_else(|| {
                stable_px
                    .get(mint_a)
                    .copied()
                    .and_then(Decimal::from_f64_retain)
            })
            .unwrap_or(Decimal::ZERO);
        let pb_d = pb_e
            .and_then(Decimal::from_f64_retain)
            .or_else(|| {
                stable_px
                    .get(mint_b)
                    .copied()
                    .and_then(Decimal::from_f64_retain)
            })
            .unwrap_or(Decimal::ZERO);

        if !amount_a_ui.is_zero() && pa_d.is_zero() {
            continue;
        }
        if !amount_b_ui.is_zero() && pb_d.is_zero() {
            continue;
        }

        let usd = amount_a_ui * pa_d + amount_b_ui * pb_d;
        if usd <= Decimal::ZERO {
            continue;
        }
        let ts = r.ts_utc.unwrap_or(DateTime::<Utc>::MIN_UTC);
        let replace = match best {
            None => true,
            Some((et, _)) => ts >= et,
        };
        if replace {
            best = Some((ts, usd));
        }
    }
    best.map(|(_, u)| u)
}

/// Before persisting `position_chain_history_nodes`, set `baseline_value_usd` (and derived PnL) for
/// **open NAV** so `start_value_usd` matches the economic “value at open” column in the UI.
///
/// **Order:** (1) recompute from **`position_stream_valuation_snapshots`** first row for the PDA
/// (amounts × snapshot prices, same source as stream lineage), (2) else lifecycle open row
/// (`event_price_*` + deposit amounts, [`open_start_usd_from_event_spot_open_row`]).
pub async fn apply_open_start_usd_from_lifecycle_snapshots_for_chain_history(
    state: &AppState,
    nodes: &mut [PositionStreamLineageNode],
) -> Result<(), ApiError> {
    let rows = lifecycle_rows_cached_best_effort().await;
    for node in nodes.iter_mut() {
        let pos = node.position_address.trim();
        if pos.is_empty() {
            continue;
        }

        let mut chosen: Option<(Decimal, &'static str)> = None;

        if let Some(db) = state.db.as_ref() {
            let q = sqlx::query(
                r#"
                SELECT value_usd, amount_a_ui, amount_b_ui, price_a_usd, price_b_usd, token_mint_a, token_mint_b, raw_json
                FROM position_stream_valuation_snapshots
                WHERE position_pubkey = $1
                ORDER BY
                  CASE WHEN COALESCE(raw_json->>'kind', '') = 'baseline_open' THEN 0 ELSE 1 END,
                  ts_utc ASC
                LIMIT 1
                "#,
            )
            .bind(pos);
            match q.fetch_optional(db.pool()).await {
                Ok(Some(row)) => {
                    if let Some(usd) = open_nav_usd_from_valuation_snapshot_row(state, &row).await
                        && usd > Decimal::ZERO
                    {
                        chosen = Some((usd, "baseline_open_snapshot_amounts_prices"));
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        position = %pos,
                        "chain-history materialize: baseline snapshot row lookup failed"
                    );
                }
            }
        }

        if chosen.is_none() {
            if let Some(usd) =
                open_start_usd_from_event_spot_open_row(state, rows.as_ref(), node).await
                && usd > Decimal::ZERO
            {
                chosen = Some((usd, "open_event_spot_amounts"));
            }
        }

        let Some((usd, quality)) = chosen else {
            continue;
        };
        node.baseline_value_usd = usd;
        node.baseline_valuation_quality = Some(quality.to_string());
        node.net_pnl_usd = node.current_value_usd + node.realized_cashflow_usd
            - node.baseline_value_usd
            - node.tx_fees_usd;
        if !node.baseline_value_usd.is_zero() {
            node.net_pnl_pct = node.net_pnl_usd / node.baseline_value_usd;
        } else {
            node.net_pnl_pct = Decimal::ZERO;
        }
    }
    Ok(())
}

async fn persist_event_valuation_snapshots_for_positions(
    state: &AppState,
    rows: &[LifecycleRow],
    positions: &[String],
) -> Result<(), ApiError> {
    let Some(db) = state.db.as_ref() else {
        return Ok(());
    };
    if positions.is_empty() {
        return Ok(());
    }

    let mut pos_set: HashSet<&str> = HashSet::new();
    for p in positions {
        let t = p.trim();
        if !t.is_empty() {
            pos_set.insert(t);
        }
    }
    if pos_set.is_empty() {
        return Ok(());
    }

    let mut open_by_pos: HashMap<String, &LifecycleRow> = HashMap::new();
    let mut close_by_pos: HashMap<String, &LifecycleRow> = HashMap::new();
    let mut pools: HashSet<String> = HashSet::new();

    for r in rows {
        let Some(pos) = r
            .position_pubkey
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        if !pos_set.contains(pos) {
            continue;
        }
        if let Some(pool) = r
            .pool_address
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            pools.insert(pool.to_string());
        }
        if is_lifecycle_open_event(r.event.as_deref()) {
            open_by_pos.entry(pos.to_string()).or_insert(r);
        }
        if is_lifecycle_close_event(r.event.as_deref()) {
            close_by_pos.insert(pos.to_string(), r);
        }
    }

    let mut pool_mints: HashMap<String, (String, String)> = HashMap::new();
    let mut requested_mints: BTreeSet<String> = BTreeSet::new();
    for pool in pools {
        let (ma, mb) = pool_leg_mints_best_effort(state, &pool).await;
        if let (Some(ma), Some(mb)) = (ma, mb) {
            requested_mints.insert(ma.clone());
            requested_mints.insert(mb.clone());
            pool_mints.insert(pool, (ma, mb));
        }
    }
    let (prices, price_source) = fetch_mint_prices_usd_stable(&requested_mints).await;

    #[allow(clippy::too_many_arguments)]
    async fn insert_snapshot(
        db: &Database,
        pos: &str,
        ts: DateTime<Utc>,
        pool: &str,
        mint_a: &str,
        mint_b: &str,
        amount_a_ui: Decimal,
        amount_b_ui: Decimal,
        value_usd: Decimal,
        pa_d: Decimal,
        pb_d: Decimal,
        price_source: &str,
        kind: &str,
        quality: &str,
        deltas: Option<&serde_json::Value>,
        // NOTE: legacy `open_caps` heuristics were removed; keep field for DB schema compatibility.
        baseline_amounts_source: Option<&'static str>,
        price_time_kind: &'static str,
        event_slot: Option<u64>,
    ) -> Result<(), ApiError> {
        let mut raw = serde_json::json!({
            "source": price_source,
            "kind": kind,
            "valuation_quality": quality,
            "position": pos,
            "pool": pool,
            "ts_utc": ts.to_rfc3339(),
            "token_mint_a": mint_a,
            "token_mint_b": mint_b,
            "amount_a_ui": amount_a_ui,
            "amount_b_ui": amount_b_ui,
            "price_a_usd": pa_d,
            "price_b_usd": pb_d,
            "price_time_kind": price_time_kind,
            "fee_payer_token_deltas": deltas.cloned(),
        });
        if let Some(s) = baseline_amounts_source {
            raw["baseline_amounts_source"] = serde_json::json!(s);
        }
        if let Some(s) = event_slot {
            raw["event_slot"] = serde_json::json!(s);
        }
        sqlx::query(
            r#"
            INSERT INTO position_stream_valuation_snapshots
              (position_pubkey, ts_utc, pool_pubkey, value_usd, amount_a_ui, amount_b_ui, fees_usd, token_mint_a, token_mint_b, price_a_usd, price_b_usd, price_source, raw_json)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
            ON CONFLICT (position_pubkey, ts_utc) DO UPDATE SET
              pool_pubkey = EXCLUDED.pool_pubkey,
              value_usd = EXCLUDED.value_usd,
              amount_a_ui = EXCLUDED.amount_a_ui,
              amount_b_ui = EXCLUDED.amount_b_ui,
              fees_usd = EXCLUDED.fees_usd,
              token_mint_a = EXCLUDED.token_mint_a,
              token_mint_b = EXCLUDED.token_mint_b,
              price_a_usd = EXCLUDED.price_a_usd,
              price_b_usd = EXCLUDED.price_b_usd,
              price_source = EXCLUDED.price_source,
              raw_json = EXCLUDED.raw_json
            WHERE EXCLUDED.raw_json->>'kind' = 'end_close'
               OR (
                    EXCLUDED.raw_json->>'kind' = 'baseline_open'
                    AND (
                        CASE COALESCE(EXCLUDED.raw_json->>'baseline_amounts_source', '')
                            WHEN 'open_amount_raw' THEN 30
                            WHEN 'open_quote_caps' THEN 20
                            WHEN 'open_amount_caps' THEN 10
                            WHEN 'open_caps' THEN 5
                            ELSE 0
                        END
                        >
                        CASE COALESCE(position_stream_valuation_snapshots.raw_json->>'baseline_amounts_source', '')
                            WHEN 'open_amount_raw' THEN 30
                            WHEN 'open_quote_caps' THEN 20
                            WHEN 'open_amount_caps' THEN 10
                            WHEN 'open_caps' THEN 5
                            ELSE 0
                        END
                    )
                )
            "#,
        )
        .bind(pos)
        .bind(ts)
        .bind(pool)
        .bind(value_usd)
        .bind(amount_a_ui)
        .bind(amount_b_ui)
        .bind(Decimal::ZERO)
        .bind(mint_a)
        .bind(mint_b)
        .bind(pa_d)
        .bind(pb_d)
        .bind(price_source)
        .bind(raw)
        .execute(db.pool())
        .await
        .map_err(|e| ApiError::internal(format!("event snapshot insert failed: {e}")))?;
        Ok(())
    }

    for pos in positions {
        let p = pos.trim();
        if p.is_empty() {
            continue;
        }
        let open = open_by_pos.get(p).copied();
        let close = close_by_pos.get(p).copied();
        let pool = open
            .and_then(|r| r.pool_address.as_deref())
            .or_else(|| close.and_then(|r| r.pool_address.as_deref()))
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let Some(pool) = pool else { continue };
        let Some((mint_a, mint_b)) = pool_mints.get(pool).cloned() else {
            continue;
        };

        if let Some(r) = open
            && let Some(ts) = r.ts_utc
        {
            let (pa_eff, pb_eff, mint_feed_suffix, time_kind, ev_slot) =
                match event_spot_from_ledger_details(r.details.as_ref()) {
                    Some((ea, eb, src, sl)) => (ea, eb, src, "at_tx_event", sl),
                    None => {
                        let pa0 = prices.get(&mint_a).copied().unwrap_or(0.0);
                        let pb0 = prices.get(&mint_b).copied().unwrap_or(0.0);
                        (pa0, pb0, price_source.clone(), "at_persist_fallback", None)
                    }
                };
            let pa_d = Decimal::from_f64_retain(pa_eff).unwrap_or(Decimal::ZERO);
            let pb_d = Decimal::from_f64_retain(pb_eff).unwrap_or(Decimal::ZERO);

            let details_obj = r.details.as_ref().and_then(|v| v.as_object());
            let deltas_obj = r
                .fee_payer_token_deltas
                .as_ref()
                .and_then(|v| v.as_object());

            let mut mint_a_decimals: Option<u8> = None;
            let mut mint_b_decimals: Option<u8> = None;
            if details_obj.is_some_and(|d| {
                (d.get("open_amount_a_raw")
                    .and_then(parse_u64_from_json)
                    .is_some()
                    && d.get("open_amount_b_raw")
                        .and_then(parse_u64_from_json)
                        .is_some())
                    || (d
                        .get("open_quote_token_max_a")
                        .and_then(parse_u64_from_json)
                        .is_some()
                        && d.get("open_quote_token_max_b")
                            .and_then(parse_u64_from_json)
                            .is_some())
                    || (d
                        .get("amount_a_cap")
                        .and_then(parse_u64_from_json)
                        .is_some()
                        && d.get("amount_b_cap")
                            .and_then(parse_u64_from_json)
                            .is_some())
            }) {
                let a_pk = solana_sdk::pubkey::Pubkey::from_str(mint_a.trim()).ok();
                let b_pk = solana_sdk::pubkey::Pubkey::from_str(mint_b.trim()).ok();
                if let (Some(a_pk), Some(b_pk)) = (a_pk, b_pk) {
                    mint_a_decimals =
                        fetch_mint_decimals_best_effort(state.provider.as_ref(), &a_pk).await;
                    mint_b_decimals =
                        fetch_mint_decimals_best_effort(state.provider.as_ref(), &b_pk).await;
                }
            }

            let (amount_a_ui, amount_b_ui, baseline_amounts_source) =
                baseline_open_amounts_ui_from_details_or_deltas(
                    details_obj,
                    deltas_obj,
                    &mint_a,
                    &mint_b,
                    mint_a_decimals,
                    mint_b_decimals,
                );

            let value_usd = amount_a_ui * pa_d + amount_b_ui * pb_d;

            if !amount_a_ui.is_zero() || !amount_b_ui.is_zero() {
                let quality = if pa_eff > 0.0 && pb_eff > 0.0 {
                    "exact"
                } else {
                    "missing_price"
                };
                let col_price_src = format!("event_open_{mint_feed_suffix}");
                insert_snapshot(
                    db,
                    p,
                    ts,
                    pool,
                    &mint_a,
                    &mint_b,
                    amount_a_ui,
                    amount_b_ui,
                    value_usd,
                    pa_d,
                    pb_d,
                    &col_price_src,
                    "baseline_open",
                    quality,
                    r.fee_payer_token_deltas.as_ref(),
                    baseline_amounts_source,
                    time_kind,
                    ev_slot,
                )
                .await?;
            }
        }

        if let Some(r) = close
            && let Some(ts) = r.ts_utc
        {
            let (pa_eff, pb_eff, mint_feed_suffix, time_kind, ev_slot) =
                match event_spot_from_ledger_details(r.details.as_ref()) {
                    Some((ea, eb, src, sl)) => (ea, eb, src, "at_tx_event", sl),
                    None => {
                        let pa0 = prices.get(&mint_a).copied().unwrap_or(0.0);
                        let pb0 = prices.get(&mint_b).copied().unwrap_or(0.0);
                        (pa0, pb0, price_source.clone(), "at_persist_fallback", None)
                    }
                };
            let pa_d = Decimal::from_f64_retain(pa_eff).unwrap_or(Decimal::ZERO);
            let pb_d = Decimal::from_f64_retain(pb_eff).unwrap_or(Decimal::ZERO);

            let mut amount_a_ui = Decimal::ZERO;
            let mut amount_b_ui = Decimal::ZERO;
            if let Some(details) = r.details.as_ref().and_then(|v| v.as_object())
                && let (Some(raw_a), Some(raw_b)) = (
                    details
                        .get("close_amount_a_raw")
                        .and_then(parse_u64_from_json),
                    details
                        .get("close_amount_b_raw")
                        .and_then(parse_u64_from_json),
                )
            {
                let a_pk = solana_sdk::pubkey::Pubkey::from_str(mint_a.trim()).ok();
                let b_pk = solana_sdk::pubkey::Pubkey::from_str(mint_b.trim()).ok();
                if let (Some(a_pk), Some(b_pk)) = (a_pk, b_pk) {
                    let dec_a =
                        fetch_mint_decimals_best_effort(state.provider.as_ref(), &a_pk).await;
                    let dec_b =
                        fetch_mint_decimals_best_effort(state.provider.as_ref(), &b_pk).await;
                    if let (Some(dec_a), Some(dec_b)) = (dec_a, dec_b) {
                        amount_a_ui = decimal_ui_from_raw_u64(raw_a, dec_a);
                        amount_b_ui = decimal_ui_from_raw_u64(raw_b, dec_b);
                    }
                }
            }
            if amount_a_ui.is_zero()
                && amount_b_ui.is_zero()
                && let Some(obj) = r
                    .fee_payer_token_deltas
                    .as_ref()
                    .and_then(|v| v.as_object())
            {
                let da = obj
                    .get(&mint_a)
                    .and_then(dec_from_any)
                    .unwrap_or(Decimal::ZERO);
                let dbb = obj
                    .get(&mint_b)
                    .and_then(dec_from_any)
                    .unwrap_or(Decimal::ZERO);
                amount_a_ui = da.max(Decimal::ZERO);
                amount_b_ui = dbb.max(Decimal::ZERO);
            }
            if !amount_a_ui.is_zero() || !amount_b_ui.is_zero() {
                let value_usd = amount_a_ui * pa_d + amount_b_ui * pb_d;
                let quality = if pa_eff > 0.0 && pb_eff > 0.0 {
                    "exact"
                } else {
                    "missing_price"
                };
                let col_price_src = format!("event_close_{mint_feed_suffix}");
                insert_snapshot(
                    db,
                    p,
                    ts,
                    pool,
                    &mint_a,
                    &mint_b,
                    amount_a_ui,
                    amount_b_ui,
                    value_usd,
                    pa_d,
                    pb_d,
                    &col_price_src,
                    "end_close",
                    quality,
                    r.fee_payer_token_deltas.as_ref(),
                    None,
                    time_kind,
                    ev_slot,
                )
                .await?;
            }
        }
    }

    Ok(())
}

async fn pool_token_mints_cached(state: &AppState, pool: &str) -> Option<(String, String)> {
    let pool = pool.trim();
    if pool.is_empty() {
        return None;
    }
    let cache = POOL_TOKEN_MINTS_CACHE.get_or_init(|| RwLock::new(HashMap::new()));
    if let Ok(g) = cache.read()
        && let Some(p) = g.get(pool)
    {
        return Some(p.clone());
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

async fn pool_leg_mints_best_effort(
    state: &AppState,
    pool: &str,
) -> (Option<String>, Option<String>) {
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
    /// `cli` / `orca_bot` / etc. from JSONL (`PositionLifecycleRecord`, rebalance executor rows).
    source: Option<String>,
}

fn parse_ts(v: &serde_json::Value) -> Option<DateTime<Utc>> {
    let s = v.as_str()?.trim();
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

#[inline]
fn closed_ts_for_snapshot_kind(
    kind: Option<&str>,
    ts: Option<DateTime<Utc>>,
) -> Option<DateTime<Utc>> {
    let kind = kind.map(str::trim).unwrap_or_default();
    if kind.eq_ignore_ascii_case("end_close") {
        ts
    } else {
        None
    }
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

/// True when this row is an **operator** open (CLI / dashboard): never stitch prior pool rotation
/// history onto this mint in lineage.
#[inline]
fn lifecycle_open_row_is_operator_manual(r: &LifecycleRow) -> bool {
    if !is_lifecycle_open_event(r.event.as_deref()) {
        return false;
    }
    if r.event.as_deref() == Some("position_open") {
        return true;
    }
    if r.source
        .as_deref()
        .is_some_and(|s| s.eq_ignore_ascii_case("cli"))
    {
        return true;
    }
    let Some(d) = r.details.as_ref().and_then(|x| x.as_object()) else {
        return false;
    };
    d.get("open_origin")
        .and_then(|x| x.as_str())
        .is_some_and(|s| s.trim() == "operator_api")
}

/// Latest open row for `entry` is operator-driven (see [`lifecycle_open_row_is_operator_manual`]).
fn lifecycle_entry_open_is_operator_manual(rows: &[LifecycleRow], entry: &str) -> bool {
    lifecycle_latest_open_row(rows, entry).is_some_and(lifecycle_open_row_is_operator_manual)
}

/// Operator closed this position (`position_close`, API `close_kind=manual`, etc.) — not a
/// strategy rotation close to chain **through** when inferring parents or walking forward.
#[inline]
fn lifecycle_close_row_is_operator_manual(r: &LifecycleRow) -> bool {
    if !is_lifecycle_close_event(r.event.as_deref()) {
        return false;
    }
    if r.event.as_deref() == Some("position_close") {
        return true;
    }
    let Some(d) = r.details.as_ref().and_then(|x| x.as_object()) else {
        return false;
    };
    d.get("close_kind")
        .and_then(|x| x.as_str())
        .is_some_and(|s| s.trim().eq_ignore_ascii_case("manual"))
        || d.get("close_source")
            .and_then(|x| x.as_str())
            .is_some_and(|s| s.trim().eq_ignore_ascii_case("api"))
}

fn lifecycle_latest_open_row<'a>(
    rows: &'a [LifecycleRow],
    entry: &str,
) -> Option<&'a LifecycleRow> {
    let entry = entry.trim();
    if entry.is_empty() {
        return None;
    }
    let mut open_row: Option<&LifecycleRow> = None;
    let mut best_ts: Option<DateTime<Utc>> = None;
    for r in rows.iter() {
        if r.position_pubkey.as_deref().map(str::trim) != Some(entry) {
            continue;
        }
        if !is_lifecycle_open_event(r.event.as_deref()) {
            continue;
        }
        let Some(ts) = r.ts_utc else {
            continue;
        };
        if best_ts.is_none() || ts >= best_ts.unwrap() {
            best_ts = Some(ts);
            open_row = Some(r);
        }
    }
    open_row
}

/// True when the **latest** open row for `entry` shares a non-empty `rebalance_session_id` with some
/// **close** row in the same pool + fee payer strictly before that open.
///
/// UI/API opens attach a fresh `cost_session_id` per request; that id does **not** match prior
/// strategy `bot_close_position` rows, so JSONL/registry rotation stitching must not inherit unrelated
/// pool history. True strategy rotations (close → open, same session) still match here.
fn lifecycle_open_has_prior_close_same_session(rows: &[LifecycleRow], entry: &str) -> bool {
    let Some(o) = lifecycle_latest_open_row(rows, entry) else {
        return false;
    };
    let Some(open_ts) = o.ts_utc else {
        return false;
    };
    let Some(sid) = o
        .rebalance_session_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return false;
    };
    let Some(pool) = o.pool_address.as_deref() else {
        return false;
    };
    let Some(payer) = o.fee_payer_pubkey.as_deref() else {
        return false;
    };
    for r in rows.iter() {
        let Some(ts) = r.ts_utc else {
            continue;
        };
        if ts >= open_ts {
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
        let Some(csid) = r
            .rebalance_session_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        if csid == sid {
            return true;
        }
    }
    false
}

/// No registry/JSONL rotation chain for **operator** opens (CLI `position_open`, `source:cli`, or
/// API `details.open_origin=operator_api` on `bot_open_*`).
/// For other bot opens, allow stitching when we can infer a concrete rotation parent from lifecycle
/// evidence (session match, or bot activity tied to the closed PDA in the pre-open window).
fn suppress_jsonl_rotation_stitch(rows: &[LifecycleRow], entry: &str) -> bool {
    if lifecycle_entry_open_is_operator_manual(rows, entry) {
        return true;
    }
    let Some(open_row) = lifecycle_latest_open_row(rows, entry) else {
        return true;
    };
    let event = open_row.event.as_deref();
    if !matches!(
        event,
        Some("bot_open_position") | Some("bot_open_position_full_range")
    ) {
        return true;
    }
    let has_parent = lifecycle_rotation_parent_before_open(rows, open_row).is_some();
    let has_session_anchor = lifecycle_open_has_prior_close_same_session(rows, entry);
    !(has_parent || has_session_anchor)
}

struct LifecycleRowsCache {
    mtime: Option<SystemTime>,
    rows: Arc<Vec<LifecycleRow>>,
}

static LIFECYCLE_ROWS_CACHE: OnceLock<RwLock<LifecycleRowsCache>> = OnceLock::new();

fn parse_lifecycle_rows_from_reader<R: BufRead>(reader: R) -> Vec<LifecycleRow> {
    let mut out = Vec::new();
    for line in reader.lines().map_while(Result::ok) {
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
        let fee_payer_token_a_delta_ui = v.get("fee_payer_token_a_delta_ui").and_then(dec_from_any);
        let fee_payer_token_b_delta_ui = v.get("fee_payer_token_b_delta_ui").and_then(dec_from_any);
        let lp_collected_token_a_raw = v.get("lp_collected_token_a_raw").and_then(|x| {
            x.as_u64()
                .or_else(|| x.as_str().and_then(|s| s.parse().ok()))
        });
        let lp_collected_token_b_raw = v.get("lp_collected_token_b_raw").and_then(|x| {
            x.as_u64()
                .or_else(|| x.as_str().and_then(|s| s.parse().ok()))
        });
        let details = v.get("details").cloned();
        let source = v
            .get("source")
            .and_then(|x| x.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
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
            source,
        });
    }
    out.sort_by_key(|a| a.ts_utc);
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
    if let Ok(g) = lock.read()
        && g.mtime == mtime
        && !g.rows.is_empty()
    {
        return g.rows.clone();
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

#[derive(Debug, Clone)]
struct FeeCheckpointRow {
    position: String,
    pool: String,
    ts_utc: DateTime<Utc>,
    tick_current: i32,
    tick_lower: i32,
    tick_upper: i32,
    liquidity: u128,
}

struct FeeCheckpointRowsCache {
    mtime: Option<SystemTime>,
    rows: Arc<Vec<FeeCheckpointRow>>,
}

static FEE_CHECKPOINT_ROWS_CACHE: OnceLock<RwLock<FeeCheckpointRowsCache>> = OnceLock::new();

fn parse_fee_checkpoint_rows_from_reader<R: BufRead>(reader: R) -> Vec<FeeCheckpointRow> {
    let mut out = Vec::new();
    for line in reader.lines().map_while(Result::ok) {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(t) else {
            continue;
        };
        let Some(ts_utc) = v.get("ts_utc").and_then(parse_ts) else {
            continue;
        };
        let Some(position) = v
            .get("position")
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
        else {
            continue;
        };
        let Some(pool) = v
            .get("pool")
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
        else {
            continue;
        };
        let tick_current = v
            .get("tick_current")
            .and_then(|x| x.as_i64())
            .map(|n| n as i32)
            .unwrap_or(0);
        let tick_lower = v
            .get("tick_lower")
            .and_then(|x| x.as_i64())
            .map(|n| n as i32)
            .unwrap_or(0);
        let tick_upper = v
            .get("tick_upper")
            .and_then(|x| x.as_i64())
            .map(|n| n as i32)
            .unwrap_or(0);
        let liquidity = v
            .get("liquidity")
            .and_then(|x| {
                x.as_u64()
                    .map(|n| n as u128)
                    .or_else(|| x.as_str().and_then(|s| s.parse::<u128>().ok()))
            })
            .unwrap_or(0);
        out.push(FeeCheckpointRow {
            position,
            pool,
            ts_utc,
            tick_current,
            tick_lower,
            tick_upper,
            liquidity,
        });
    }
    out.sort_by_key(|a| a.ts_utc);
    out
}

async fn fee_checkpoint_rows_cached_best_effort() -> Arc<Vec<FeeCheckpointRow>> {
    let path = std::path::PathBuf::from("data/position-fee-checkpoints.jsonl");
    let mtime = fs::metadata(&path).ok().and_then(|m| m.modified().ok());

    let lock = FEE_CHECKPOINT_ROWS_CACHE.get_or_init(|| {
        RwLock::new(FeeCheckpointRowsCache {
            mtime: None,
            rows: Arc::new(Vec::new()),
        })
    });
    if let Ok(g) = lock.read()
        && g.mtime == mtime
        && !g.rows.is_empty()
    {
        return g.rows.clone();
    }
    let rebuilt: Vec<FeeCheckpointRow> = tokio::task::spawn_blocking(move || {
        let file = fs::File::open(&path).ok();
        match file {
            Some(f) => parse_fee_checkpoint_rows_from_reader(BufReader::new(f)),
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

fn attach_collect_zero_diagnostics(
    rows: &[LifecycleRow],
    checkpoint_rows: &[FeeCheckpointRow],
    nodes: &mut [PositionStreamLineageNode],
) {
    for n in nodes.iter_mut() {
        let zero_collect = (n.collect_events > 0)
            && n.fees_collected_token_a_ui.is_some_and(|v| v.is_zero())
            && n.fees_collected_token_b_ui.is_some_and(|v| v.is_zero());
        if !zero_collect {
            continue;
        }
        let pool = rows
            .iter()
            .find(|r| r.position_pubkey.as_deref() == Some(n.position_address.as_str()))
            .and_then(|r| r.pool_address.clone())
            .unwrap_or_default();
        let start = n
            .opened_ts_utc
            .as_deref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc));
        let end = n
            .closed_ts_utc
            .as_deref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc))
            .or_else(|| {
                rows.iter()
                    .filter(|r| r.position_pubkey.as_deref() == Some(n.position_address.as_str()))
                    .filter_map(|r| r.ts_utc)
                    .max()
            });
        let window_contains = |ts: DateTime<Utc>| {
            let ge_start = start.map(|s| ts >= s).unwrap_or(true);
            let le_end = end.map(|e| ts <= e).unwrap_or(true);
            ge_start && le_end
        };
        let swap_events_in_window_est = rows
            .iter()
            .filter(|r| r.pool_address.as_deref() == Some(pool.as_str()))
            .filter(|r| r.event.as_deref().is_some_and(|e| e.contains("swap")))
            .filter_map(|r| r.ts_utc)
            .filter(|ts| window_contains(*ts))
            .count() as u32;

        let samples: Vec<&FeeCheckpointRow> = checkpoint_rows
            .iter()
            .filter(|r| r.position == n.position_address)
            .filter(|r| window_contains(r.ts_utc))
            .collect();
        let in_range_samples = samples.len() as u32;
        let in_range_time_share_pct_est = if samples.is_empty() {
            None
        } else if samples.len() == 1 {
            let s = samples[0];
            Some(
                if s.tick_current >= s.tick_lower && s.tick_current <= s.tick_upper {
                    Decimal::new(100, 0)
                } else {
                    Decimal::ZERO
                },
            )
        } else {
            let mut weighted_secs: i64 = 0;
            let mut in_range_secs: i64 = 0;
            for w in samples.windows(2) {
                let a = w[0];
                let b = w[1];
                let dt = (b.ts_utc - a.ts_utc).num_seconds().max(0);
                if dt == 0 {
                    continue;
                }
                weighted_secs += dt;
                if a.tick_current >= a.tick_lower && a.tick_current <= a.tick_upper {
                    in_range_secs += dt;
                }
            }
            if weighted_secs <= 0 {
                None
            } else {
                Some(
                    (Decimal::from(in_range_secs) * Decimal::new(100, 0))
                        / Decimal::from(weighted_secs),
                )
            }
        };

        let node_avg_liq = if samples.is_empty() {
            None
        } else {
            let sum: u128 = samples.iter().map(|s| s.liquidity).sum();
            Some(sum / (samples.len() as u128))
        };
        let max_pool_avg_liq = if pool.is_empty() {
            None
        } else {
            let mut by_pos: HashMap<&str, (u128, u32)> = HashMap::new();
            for s in checkpoint_rows
                .iter()
                .filter(|r| r.pool == pool)
                .filter(|r| window_contains(r.ts_utc))
            {
                let e = by_pos.entry(s.position.as_str()).or_insert((0, 0));
                e.0 = e.0.saturating_add(s.liquidity);
                e.1 += 1;
            }
            by_pos
                .values()
                .filter(|(_, c)| *c > 0)
                .map(|(sum, c)| sum / (*c as u128))
                .max()
        };
        let position_share_pct_est = match (node_avg_liq, max_pool_avg_liq) {
            (Some(a), Some(m)) if m > 0 => {
                let da = Decimal::from_str(&a.to_string()).unwrap_or(Decimal::ZERO);
                let dm = Decimal::from_str(&m.to_string()).unwrap_or(Decimal::ZERO);
                if dm.is_zero() {
                    None
                } else {
                    let pct = (da * Decimal::new(100, 0)) / dm;
                    Some(pct.min(Decimal::new(100, 0)))
                }
            }
            _ => None,
        };

        n.collect_zero_diagnostics = Some(LineageCollectZeroDiagnostics {
            in_range_time_share_pct_est,
            in_range_samples,
            swap_events_in_window_est,
            position_share_pct_est,
            methodology_note: "Best-effort estimate from local fee checkpoints + lifecycle rows. swap_events counts bot/ledger swaps in pool window (not full market volume). position_share compares sampled liquidity vs max sampled in same pool/window.".to_string(),
        });
    }
}

/// Parent PDA for a **rotation** into `o`: only when lifecycle shows explicit rotation evidence
/// (matching non-empty `rebalance_session_id` on close vs open, or bot activity on the closed parent
/// PDA between that close and this open — never `close_kind=rotation` alone, which would false-link
/// unrelated later `bot_open_*` rows such as API opens with a fresh `cost_session_id`).
fn lifecycle_rotation_parent_before_open<'a>(
    rows: &'a [LifecycleRow],
    o: &'a LifecycleRow,
) -> Option<&'a str> {
    let open_ts = o.ts_utc?;
    let pool = o.pool_address.as_deref()?;
    let payer = o.fee_payer_pubkey.as_deref()?;

    // Strong anchor: if the open row has a non-empty session id, prefer linking by exact session match
    // without a time window. This supports reboot / delayed recovery where open happens hours later.
    if let Some(osid) = o
        .rebalance_session_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let mut best: Option<&LifecycleRow> = None;
        for r in rows.iter() {
            let Some(ts) = r.ts_utc else { continue };
            if ts >= open_ts {
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
            if lifecycle_close_row_is_operator_manual(r) {
                continue;
            }
            let csid = r
                .rebalance_session_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            if csid != Some(osid) {
                continue;
            }
            if best.is_none() || ts > best.and_then(|b| b.ts_utc).unwrap_or(ts) {
                best = Some(r);
            }
        }
        if let Some(p) = best.and_then(|r| r.position_pubkey.as_deref()) {
            return Some(p);
        }
    }

    let open_rebalance_session_id = o.rebalance_session_id.as_deref();
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
        if lifecycle_close_row_is_operator_manual(r) {
            continue;
        }
        let parent_pda = r.position_pubkey.as_deref().unwrap_or("");
        if parent_pda.is_empty() {
            continue;
        }
        let mut has_rotation_signal = false;
        // Do **not** treat `close_kind=rotation` alone as proof the *current* open row continues that
        // close. API/dashboard opens are logged as `bot_open_position` with a fresh
        // `rebalance_session_id` (`cost_session_id`); an unrelated rotation close in the same pool
        // within the lookback window would otherwise become a false "parent" and defeat
        // `suppress_jsonl_rotation_stitch` / lineage isolation.
        if let (Some(osid), Some(csid)) =
            (open_rebalance_session_id, r.rebalance_session_id.as_deref())
            && !osid.is_empty()
            && osid == csid
        {
            has_rotation_signal = true;
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
                let ev = rr.event.as_deref().unwrap_or("");
                if (ev.starts_with("bot_swap_") || ev.starts_with("bot_swap"))
                    && rr.position_pubkey.as_deref() == Some(parent_pda)
                {
                    has_rotation_signal = true;
                    break;
                }
                if ev == "bot_reopen_preflight_failed"
                    && rr.position_pubkey.as_deref() == Some(parent_pda)
                {
                    has_rotation_signal = true;
                    break;
                }
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
            if r.position_pubkey.as_deref() == Some(position)
                && is_lifecycle_close_event(r.event.as_deref())
            {
                return Some((i, r));
            }
        }
        None
    }

    // Anchor: prefer OPEN row (lets us go backwards); otherwise fall back to CLOSE.
    let mut anchor: Option<(usize, &LifecycleRow)> = find_open_row(rows, entry);
    if anchor.is_none() {
        // newest close row for entry
        let mut last: Option<(usize, &LifecycleRow)> = None;
        for (i, r) in rows.iter().enumerate() {
            if r.position_pubkey.as_deref() == Some(entry)
                && is_lifecycle_close_event(r.event.as_deref())
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
        let Some((_oi, o)) = find_open_row(rows, &cur_pos) else {
            break;
        };
        let Some(parent) = lifecycle_rotation_parent_before_open(rows, o) else {
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
        let Some((close_i, c)) = find_close_row_from(rows, &cur, cur_idx) else {
            break;
        };
        if lifecycle_close_row_is_operator_manual(c) {
            break;
        }
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
            let Some(parent) = lifecycle_rotation_parent_before_open(rows, r) else {
                continue;
            };
            if parent != cur {
                continue;
            }
            next_open = Some((i, r));
            break;
        }
        let Some((open_i, o)) = next_open else { break };
        let Some(next_pda) = o.position_pubkey.as_deref() else {
            break;
        };
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

fn parse_u64_from_json(v: &serde_json::Value) -> Option<u64> {
    v.as_u64()
        .or_else(|| v.as_i64().and_then(|x| (x > 0).then_some(x as u64)))
        .or_else(|| v.as_str().and_then(|s| s.parse::<u64>().ok()))
}

fn baseline_open_amounts_ui_from_details_or_deltas(
    details_obj: Option<&serde_json::Map<String, serde_json::Value>>,
    deltas_obj: Option<&serde_json::Map<String, serde_json::Value>>,
    mint_a: &str,
    mint_b: &str,
    mint_a_decimals: Option<u8>,
    mint_b_decimals: Option<u8>,
) -> (Decimal, Decimal, Option<&'static str>) {
    if let Some(details) = details_obj
        && let (Some(raw_a), Some(raw_b)) = (
            details
                .get("open_amount_a_raw")
                .and_then(parse_u64_from_json),
            details
                .get("open_amount_b_raw")
                .and_then(parse_u64_from_json),
        )
        && let (Some(dec_a), Some(dec_b)) = (mint_a_decimals, mint_b_decimals)
    {
        return (
            decimal_ui_from_raw_u64(raw_a, dec_a),
            decimal_ui_from_raw_u64(raw_b, dec_b),
            Some("open_amount_raw"),
        );
    }

    // Planned (pre-open) caps from our deposit quote sizing (strategy/bot path).
    // This is what we *intended* to open with; it should be close to measured amounts and is
    // preferable to ambiguous `fee_payer_token_deltas` while on-chain measurement is pending.
    if let Some(details) = details_obj
        && let (Some(raw_a), Some(raw_b)) = (
            details
                .get("open_quote_token_max_a")
                .and_then(parse_u64_from_json),
            details
                .get("open_quote_token_max_b")
                .and_then(parse_u64_from_json),
        )
        && let (Some(dec_a), Some(dec_b)) = (mint_a_decimals, mint_b_decimals)
    {
        return (
            decimal_ui_from_raw_u64(raw_a, dec_a),
            decimal_ui_from_raw_u64(raw_b, dec_b),
            Some("open_quote_caps"),
        );
    }

    // Planned caps when open was requested with explicit amounts (operator_api / tx-build flows).
    if let Some(details) = details_obj
        && let (Some(raw_a), Some(raw_b)) = (
            details.get("amount_a_cap").and_then(parse_u64_from_json),
            details.get("amount_b_cap").and_then(parse_u64_from_json),
        )
        && let (Some(dec_a), Some(dec_b)) = (mint_a_decimals, mint_b_decimals)
    {
        return (
            decimal_ui_from_raw_u64(raw_a, dec_a),
            decimal_ui_from_raw_u64(raw_b, dec_b),
            Some("open_amount_caps"),
        );
    }

    if let Some(obj) = deltas_obj {
        let da = obj
            .get(mint_a)
            .and_then(dec_from_any)
            .unwrap_or(Decimal::ZERO);
        let dbb = obj
            .get(mint_b)
            .and_then(dec_from_any)
            .unwrap_or(Decimal::ZERO);
        return ((-da).max(Decimal::ZERO), (-dbb).max(Decimal::ZERO), None);
    }

    (Decimal::ZERO, Decimal::ZERO, None)
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
    for line in reader.lines().map_while(Result::ok) {
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
    out.sort_by_key(|a| a.ts_utc);
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
/// (same pool + owner; 60m window) **only** when `rebalance_session_id` is non-empty and matches on both rows.
/// Manual opens without a shared session id are not linked to unrelated prior closes.
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
        if let (Some(osid), Some(csid)) = (
            o.rebalance_session_id.as_deref(),
            r.rebalance_session_id.as_deref(),
        ) && !osid.is_empty()
            && !csid.is_empty()
            && osid == csid
            && best.as_ref().is_none_or(|(_, bts)| ts > *bts)
        {
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

fn chain_from_registry_best_effort_rows(
    rows: &[RegistryRow],
    entry: &str,
    max_hops: usize,
) -> Vec<String> {
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
        let Some(o) = find_open_row(rows, &cur) else {
            break;
        };
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
            if let (Some(osid), Some(csid)) = (
                o.rebalance_session_id.as_deref(),
                r.rebalance_session_id.as_deref(),
            ) && !osid.is_empty()
                && !csid.is_empty()
                && osid == csid
                && best.as_ref().is_none_or(|(_, bts)| ts > *bts)
            {
                best = Some((r, ts));
            }
        }
        let Some((c, _)) = best else { break };
        let Some(parent) = c.position_pubkey.as_deref() else {
            break;
        };
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
        let Some(c) = find_close_row(rows, &cur) else {
            break;
        };
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
            if let (Some(csid), Some(osid)) = (
                c.rebalance_session_id.as_deref(),
                r.rebalance_session_id.as_deref(),
            ) && !csid.is_empty()
                && !osid.is_empty()
                && csid == osid
                && next.as_ref().is_none_or(|(_, nts)| ts < *nts)
            {
                next = Some((r, ts));
            }
        }
        let Some((o, _)) = next else { break };
        let Some(next_pda) = o.position_pubkey.as_deref() else {
            break;
        };
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
        let event = r.event.as_deref().unwrap_or_default();
        let is_collect = event == "bot_collect_fees";
        let is_close = event == "bot_close_position";
        if !(is_collect || is_close) {
            continue;
        }
        let Some(pool) = r
            .pool_address
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
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
        // NOTE: Some legacy close rows may carry `lp_collected_token_*_raw=0/0` from a stale
        // pre-update snapshot (principal then leaked into "fees" via `fee_payer_token_deltas`).
        // Treat 0/0 as non-authoritative so close subtraction can isolate fee legs.
        let has_authoritative_pair = (is_collect || is_close)
            && r.lp_collected_token_a_raw.is_some()
            && r.lp_collected_token_b_raw.is_some()
            && (r.lp_collected_token_a_raw.unwrap_or(0) > 0
                || r.lp_collected_token_b_raw.unwrap_or(0) > 0);
        let mut raw_a_ui: Option<Decimal> = None;
        let mut raw_b_ui: Option<Decimal> = None;

        // Authoritative both legs: position `fee_owed_a/b` read by bot immediately before harvest/close.
        if (is_collect || is_close)
            && let Some(raw) = r.lp_collected_token_a_raw
            && let Ok(pk) = solana_sdk::pubkey::Pubkey::from_str(ma.as_str())
        {
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
            raw_a_ui = Some(ui);
            merged_a = merged_a.max(ui);
        }
        if (is_collect || is_close)
            && let Some(raw) = r.lp_collected_token_b_raw
            && let Ok(pk) = solana_sdk::pubkey::Pubkey::from_str(mb.as_str())
        {
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
            raw_b_ui = Some(ui);
            merged_b = merged_b.max(ui);
        }

        // Prefer authoritative pair from position fee_owed_{a,b} whenever both legs are available.
        if has_authoritative_pair {
            if let Some(v) = raw_a_ui {
                merged_a = v;
            }
            if let Some(v) = raw_b_ui {
                merged_b = v;
            }
        }
        // On close, `fee_payer_token_deltas` contains principal+fees. If we do NOT have authoritative
        // fee_owed legs, we can isolate fee leg only when we also have close principal amounts.
        if is_close
            && !has_authoritative_pair
            && let Some(details) = r.details.as_ref().and_then(serde_json::Value::as_object)
        {
            let close_raw_a = details
                .get("close_amount_a_raw")
                .and_then(dec_from_any)
                .filter(|d| *d > Decimal::ZERO);
            let close_raw_b = details
                .get("close_amount_b_raw")
                .and_then(dec_from_any)
                .filter(|d| *d > Decimal::ZERO);
            // If we cannot subtract principal, do not treat this close row as "fees collected".
            // Otherwise we would overcount principal+fees as fees.
            if close_raw_a.is_none() && close_raw_b.is_none() {
                continue;
            }
            if let Some(raw) = close_raw_a
                && let Ok(pk) = solana_sdk::pubkey::Pubkey::from_str(ma.as_str())
            {
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
                let ui = raw / Decimal::from(10u64.pow(dec as u32));
                merged_a = (merged_a - ui).max(Decimal::ZERO);
            }
            if let Some(raw) = close_raw_b
                && let Ok(pk) = solana_sdk::pubkey::Pubkey::from_str(mb.as_str())
            {
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
                let ui = raw / Decimal::from(10u64.pow(dec as u32));
                merged_b = (merged_b - ui).max(Decimal::ZERO);
            }
        }

        if merged_a > Decimal::ZERO || has_authoritative_pair {
            *by_mint_ui.entry(ma.clone()).or_insert(Decimal::ZERO) += merged_a;
        }
        if merged_b > Decimal::ZERO || has_authoritative_pair {
            *by_mint_ui.entry(mb.clone()).or_insert(Decimal::ZERO) += merged_b;
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

pub(crate) async fn lp_fees_collected_usd_from_ledger_db(
    state: &AppState,
    db: &Database,
    position_pubkey: &str,
) -> Result<(u32, Decimal, BTreeMap<String, Decimal>), ApiError> {
    let rows = sqlx::query(
        r#"
        SELECT fee_payer_token_deltas, pool_pubkey,
               fee_payer_token_a_delta_ui, fee_payer_token_b_delta_ui,
               event,
               NULLIF(raw_json->>'lp_collected_token_a_raw', '')::BIGINT AS lp_collected_token_a_raw,
               NULLIF(raw_json->>'lp_collected_token_b_raw', '')::BIGINT AS lp_collected_token_b_raw,
               COALESCE(
                 NULLIF(raw_json->>'close_amount_a_raw', '')::BIGINT,
                 NULLIF(raw_json->'details'->>'close_amount_a_raw', '')::BIGINT
               ) AS close_amount_a_raw,
               COALESCE(
                 NULLIF(raw_json->>'close_amount_b_raw', '')::BIGINT,
                 NULLIF(raw_json->'details'->>'close_amount_b_raw', '')::BIGINT
               ) AS close_amount_b_raw,
               raw_json
        FROM position_stream_ledger_rows
        WHERE position_pubkey = $1 AND event IN ('bot_collect_fees', 'bot_close_position')
        "#,
    )
    .bind(position_pubkey)
    .fetch_all(db.pool())
    .await
    .map_err(|e| ApiError::internal(format!("stream lineage: collect fee rows: {e}")))?;

    let mut pool_mints: HashMap<String, (String, String)> = HashMap::new();
    let mut mint_decimals: HashMap<String, u8> = HashMap::new();
    let mut by_mint_ui: BTreeMap<String, Decimal> = BTreeMap::new();
    let mut fallback_by_mint_ui: BTreeMap<String, Decimal> = BTreeMap::new();
    let mut usd = Decimal::ZERO;
    let mut events: u32 = 0;

    for r in rows {
        let v: Option<serde_json::Value> = r.try_get("fee_payer_token_deltas").ok().flatten();
        let raw_json: Option<serde_json::Value> = r.try_get("raw_json").ok().flatten();
        let pool: Option<String> = r.try_get("pool_pubkey").ok();
        let lp_raw_a: Option<i64> = r.try_get("lp_collected_token_a_raw").ok().flatten();
        let lp_raw_b: Option<i64> = r.try_get("lp_collected_token_b_raw").ok().flatten();
        let close_raw_a: Option<i64> = r.try_get("close_amount_a_raw").ok().flatten();
        let close_raw_b: Option<i64> = r.try_get("close_amount_b_raw").ok().flatten();
        let event: Option<String> = r.try_get("event").ok();
        let is_collect = event.as_deref() == Some("bot_collect_fees");
        let is_close = event.as_deref() == Some("bot_close_position");
        if !(is_collect || is_close) {
            continue;
        }
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
        let has_authoritative_pair = (is_collect || is_close)
            && lp_raw_a.is_some()
            && lp_raw_b.is_some()
            && (lp_raw_a.unwrap_or(0) > 0 || lp_raw_b.unwrap_or(0) > 0);
        let mut raw_a_ui: Option<Decimal> = None;
        let mut raw_b_ui: Option<Decimal> = None;

        if (is_collect || is_close)
            && let Some(raw) = lp_raw_a.filter(|x| *x > 0).map(|x| x as u64)
            && let Ok(pk) = solana_sdk::pubkey::Pubkey::from_str(ma.as_str())
        {
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
            raw_a_ui = Some(ui);
            merged_a = merged_a.max(ui);
        }
        if (is_collect || is_close)
            && let Some(raw) = lp_raw_b.filter(|x| *x > 0).map(|x| x as u64)
            && let Ok(pk) = solana_sdk::pubkey::Pubkey::from_str(mb.as_str())
        {
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
            raw_b_ui = Some(ui);
            merged_b = merged_b.max(ui);
        }

        // Prefer authoritative pair from position fee_owed_{a,b} whenever both legs are available.
        if has_authoritative_pair {
            if let Some(v) = raw_a_ui {
                merged_a = v;
            }
            if let Some(v) = raw_b_ui {
                merged_b = v;
            }
        }
        if is_close && !has_authoritative_pair {
            if close_raw_a.is_none() && close_raw_b.is_none() {
                // Without principal subtraction inputs, do not treat close token deltas as fees.
                continue;
            }
            if let Some(raw) = close_raw_a.filter(|x| *x > 0).map(|x| x as u64)
                && let Ok(pk) = solana_sdk::pubkey::Pubkey::from_str(ma.as_str())
            {
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
                merged_a = (merged_a - ui).max(Decimal::ZERO);
            }
            if let Some(raw) = close_raw_b.filter(|x| *x > 0).map(|x| x as u64)
                && let Ok(pk) = solana_sdk::pubkey::Pubkey::from_str(mb.as_str())
            {
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
                merged_b = (merged_b - ui).max(Decimal::ZERO);
            }
        }

        if merged_a > Decimal::ZERO || has_authoritative_pair {
            *by_mint_ui.entry(ma.clone()).or_insert(Decimal::ZERO) += merged_a;
        }
        if merged_b > Decimal::ZERO || has_authoritative_pair {
            *by_mint_ui.entry(mb.clone()).or_insert(Decimal::ZERO) += merged_b;
        }

        let event_spot = raw_json
            .as_ref()
            .and_then(|raw| raw.get("details"))
            .and_then(|details| event_spot_from_ledger_details(Some(details)))
            .or_else(|| event_spot_from_ledger_details(raw_json.as_ref()));
        if let Some((pa, pb, _src, _slot)) = event_spot {
            let pa_d = Decimal::from_f64_retain(pa).unwrap_or(Decimal::ZERO);
            let pb_d = Decimal::from_f64_retain(pb).unwrap_or(Decimal::ZERO);
            usd += merged_a * pa_d + merged_b * pb_d;
        } else {
            if merged_a > Decimal::ZERO || has_authoritative_pair {
                *fallback_by_mint_ui.entry(ma).or_insert(Decimal::ZERO) += merged_a;
            }
            if merged_b > Decimal::ZERO || has_authoritative_pair {
                *fallback_by_mint_ui.entry(mb).or_insert(Decimal::ZERO) += merged_b;
            }
        }
    }

    if by_mint_ui.is_empty() {
        return Ok((events, Decimal::ZERO, BTreeMap::new()));
    }

    let mints: BTreeSet<String> = fallback_by_mint_ui.keys().cloned().collect();
    let (px, _) = match timeout(Duration::from_secs(5), fetch_mint_prices_usd(&mints)).await {
        Ok(r) => r,
        Err(_) => return Ok((events, usd, by_mint_ui)),
    };
    for (m, amt) in &fallback_by_mint_ui {
        let p = px.get(m).copied().unwrap_or(0.0);
        if p > 0.0 && p.is_finite() {
            let pd = Decimal::from_f64_retain(p).unwrap_or(Decimal::ZERO);
            usd += *amt * pd;
        }
    }
    Ok((events, usd, by_mint_ui))
}

pub(crate) fn rollup_lineage_chain_costs(
    nodes: &[PositionStreamLineageNode],
) -> Option<LineageChainCostSummary> {
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

/// Build **entry-centric** lineage from persisted `position_stream_edges`.
///
/// The old root-forward walk could miss `entry` when the directed graph forks (e.g. `A→B` and `A→C`):
/// traversal followed one branch, hit `if !chain.contains(entry) { vec![entry] }`, and JSONL fallback
/// then stitched an overly long “history”. We instead walk **backward** from `entry` to oldest
/// ancestor, then **forward** to newest descendant along best-effort timestamps.
fn build_lineage_chain_from_db_edges(
    positions: &[String],
    edges: &[(Option<DateTime<Utc>>, String, String, String)],
    entry: &str,
    max_hops: usize,
) -> Vec<String> {
    let pos_set: HashSet<&str> = positions.iter().map(|s| s.as_str()).collect();
    if !pos_set.contains(entry) {
        return vec![entry.to_string()];
    }

    let mut ancestors: Vec<String> = Vec::new();
    let mut seen_b: HashSet<String> = HashSet::new();
    seen_b.insert(entry.to_string());
    let mut cur = entry.to_string();
    for _ in 0..max_hops {
        let mut preds: Vec<(Option<DateTime<Utc>>, &str)> = Vec::new();
        for (ts, old, newp, _sid) in edges {
            if newp != &cur || !pos_set.contains(old.as_str()) {
                continue;
            }
            preds.push((*ts, old.as_str()));
        }
        if preds.is_empty() {
            break;
        }
        let pred = preds
            .iter()
            .max_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)))
            .map(|(_ts, p)| *p)
            .expect("preds non-empty");
        let pred = pred.to_string();
        if !seen_b.insert(pred.clone()) {
            break;
        }
        cur = pred.clone();
        ancestors.push(pred);
    }
    ancestors.reverse();

    let mut chain = ancestors;
    chain.push(entry.to_string());

    let mut seen_f: HashSet<String> = chain.iter().cloned().collect();
    cur = entry.to_string();
    for _ in 0..max_hops {
        let mut succs: Vec<(Option<DateTime<Utc>>, &str)> = Vec::new();
        for (ts, old, newp, _sid) in edges {
            if old != &cur || !pos_set.contains(newp.as_str()) {
                continue;
            }
            succs.push((*ts, newp.as_str()));
        }
        if succs.is_empty() {
            break;
        }
        let succ = succs
            .iter()
            .max_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)))
            .map(|(_ts, s)| *s)
            .expect("succs non-empty");
        let succ = succ.to_string();
        if !seen_f.insert(succ.clone()) {
            break;
        }
        cur = succ.clone();
        chain.push(succ);
    }

    chain
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

pub(crate) async fn node_metrics(
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
    // Prefer explicit `baseline_open` / `end_close` rows from `persist_event_valuation_snapshots_for_positions`
    // (corrected with `open_caps` when fee-payer deltas understate deposit).
    let baseline = sqlx::query(
        r#"
        SELECT ts_utc, value_usd, token_mint_a, token_mint_b, raw_json
        FROM position_stream_valuation_snapshots
        WHERE position_pubkey = $1
        ORDER BY
          CASE WHEN COALESCE(raw_json->>'kind', '') = 'baseline_open' THEN 0 ELSE 1 END,
          ts_utc ASC
        LIMIT 1
        "#,
    )
    .bind(position_pubkey)
    .fetch_optional(db.pool())
    .await
    .map_err(|e| ApiError::internal(format!("stream lineage: baseline snapshot query: {e}")))?;

    let current = sqlx::query(
        r#"
        SELECT ts_utc, value_usd, raw_json
        FROM position_stream_valuation_snapshots
        WHERE position_pubkey = $1
        ORDER BY
          CASE WHEN COALESCE(raw_json->>'kind', '') = 'end_close' THEN 0 ELSE 1 END,
          ts_utc DESC
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
        if let Some(pk) = pk
            && let Ok(Ok(pos)) = timeout(
                Duration::from_secs(2),
                monitored_position_from_chain(state.provider.clone(), &pk),
            )
            .await
            && let Ok(v) = compute_position_usd_valuation(
                state.provider.clone(),
                &pos,
                &fetch_prices_for_positions(state.provider.clone(), std::slice::from_ref(&pos))
                    .await,
            )
            .await
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

        baseline = sqlx::query(
            r#"
            SELECT ts_utc, value_usd, token_mint_a, token_mint_b, raw_json
            FROM position_stream_valuation_snapshots
            WHERE position_pubkey = $1
            ORDER BY
              CASE WHEN COALESCE(raw_json->>'kind', '') = 'baseline_open' THEN 0 ELSE 1 END,
              ts_utc ASC
            LIMIT 1
            "#,
        )
        .bind(position_pubkey)
        .fetch_optional(db.pool())
        .await
        .map_err(|e| {
            ApiError::internal(format!(
                "stream lineage: baseline snapshot query (after seed): {e}"
            ))
        })?;

        current = sqlx::query(
            r#"
            SELECT ts_utc, value_usd, raw_json
            FROM position_stream_valuation_snapshots
            WHERE position_pubkey = $1
            ORDER BY
              CASE WHEN COALESCE(raw_json->>'kind', '') = 'end_close' THEN 0 ELSE 1 END,
              ts_utc DESC
            LIMIT 1
            "#,
        )
        .bind(position_pubkey)
        .fetch_optional(db.pool())
        .await
        .map_err(|e| {
            ApiError::internal(format!(
                "stream lineage: current snapshot query (after seed): {e}"
            ))
        })?;

        if baseline.is_none() && current.is_none() {
            let rows = lifecycle_rows_cached_best_effort().await;
            return node_metrics_from_lifecycle_best_effort(state, &rows, position_pubkey).await;
        }
    }

    let opened_ts: Option<DateTime<Utc>> = baseline
        .as_ref()
        .and_then(|r| r.try_get::<Option<DateTime<Utc>>, _>("ts_utc").ok())
        .flatten();
    let baseline_valuation_quality: Option<String> = baseline
        .as_ref()
        .and_then(|r| {
            r.try_get::<Option<serde_json::Value>, _>("raw_json")
                .ok()
                .flatten()
        })
        .and_then(|v| {
            v.get("valuation_quality")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string())
        });
    let current_ts: Option<DateTime<Utc>> = current
        .as_ref()
        .and_then(|r| r.try_get::<Option<DateTime<Utc>>, _>("ts_utc").ok())
        .flatten();
    let current_kind = current
        .as_ref()
        .and_then(|r| {
            r.try_get::<Option<serde_json::Value>, _>("raw_json")
                .ok()
                .flatten()
        })
        .and_then(|v| {
            v.get("kind")
                .and_then(|x| x.as_str())
                .map(std::string::ToString::to_string)
        });
    // `closed_ts_utc` represents an actual close marker (`end_close`) only.
    // Fresh open nodes may already have a "current valuation snapshot" timestamp, which is not a close.
    let closed_ts: Option<DateTime<Utc>> =
        closed_ts_for_snapshot_kind(current_kind.as_deref(), current_ts);
    let current_valuation_quality: Option<String> = current
        .as_ref()
        .and_then(|r| {
            r.try_get::<Option<serde_json::Value>, _>("raw_json")
                .ok()
                .flatten()
        })
        .and_then(|v| {
            v.get("valuation_quality")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string())
        });

    let mut baseline_value: Decimal = baseline
        .as_ref()
        .and_then(|r| r.try_get::<Decimal, _>("value_usd").ok())
        .unwrap_or(Decimal::ZERO);
    let mut current_value: Decimal = current
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
    let mut fee_lamports_u = fee_lamports.max(0) as u64;

    let (sol_usd, sol_src) = sol_usd().await;
    let mut tx_fees_usd = if sol_usd > 0.0 {
        Decimal::from_f64_retain((fee_lamports_u as f64 / 1e9) * sol_usd).unwrap_or(Decimal::ZERO)
    } else {
        Decimal::ZERO
    };

    // Realized cashflow for this PDA: sum non-principal fee_payer_token_deltas (pool legs)
    // × current mint USD prices. Open/close principal legs are excluded.
    let mut mint_deltas: BTreeMap<String, Decimal> = BTreeMap::new();
    let rows = sqlx::query(
        r#"
        SELECT fee_payer_token_deltas, event
        FROM position_stream_ledger_rows
        WHERE position_pubkey = $1 AND fee_payer_token_deltas IS NOT NULL
        "#,
    )
    .bind(position_pubkey)
    .fetch_all(db.pool())
    .await
    .map_err(|e| ApiError::internal(format!("stream lineage: token deltas query: {e}")))?;

    for r in rows {
        let event: Option<String> = r.try_get("event").ok();
        if is_lifecycle_open_event(event.as_deref()) || is_lifecycle_close_event(event.as_deref()) {
            continue;
        }
        let v: Option<serde_json::Value> = r.try_get("fee_payer_token_deltas").ok();
        let Some(serde_json::Value::Object(map)) = v else {
            continue;
        };
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

    // When `value_usd` on the snapshot row is still zero but `amount_*_ui` + `price_*_usd` are present
    // (or only in `raw_json`), recompute open NAV so lineage baseline matches materialized `start_value_usd`.
    if baseline_value <= Decimal::ZERO {
        if let Some(ref br) = baseline {
            if let Some(nav) = open_nav_usd_from_valuation_snapshot_row(state, br).await
                && nav > Decimal::ZERO
            {
                baseline_value = nav;
                baseline_note = Some("baseline_nav_from_snapshot_amounts_prices".to_string());
            }
        }
    }

    // DB path guardrail: baseline snapshots derived from open deltas may miss one leg (WSOL),
    // which can massively understate "start value". Correct from open `amount_*_cap` when available.
    if baseline_value.is_zero()
        || (current_value > Decimal::ZERO && baseline_value < current_value * Decimal::new(60, 2))
    {
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
                    let dec_a =
                        fetch_mint_decimals_best_effort(state.provider.as_ref(), &a_pk).await;
                    let dec_b =
                        fetch_mint_decimals_best_effort(state.provider.as_ref(), &b_pk).await;
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
                                let cap_usd_f =
                                    ui_amount(cap_a, dec_a) * pa + ui_amount(cap_b, dec_b) * pb;
                                let cap_usd =
                                    Decimal::from_f64_retain(cap_usd_f).unwrap_or(Decimal::ZERO);
                                let cap_not_too_high = if current_value > Decimal::ZERO {
                                    cap_usd <= current_value * Decimal::new(135, 2)
                                } else {
                                    true
                                };
                                if cap_not_too_high && cap_usd > baseline_value {
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

    let open_usd_solo =
        fetch_ledger_open_quote_usd_by_positions(db, &[position_pubkey.trim().to_string()]).await?;
    if let Some(open_usd) = open_usd_solo.get(position_pubkey.trim()).copied() {
        if open_usd > baseline_value {
            let vs_mark = current_value > Decimal::ZERO
                && baseline_value < current_value * Decimal::new(60, 2);
            let zero_current_open = current_value.is_zero() && open_usd > baseline_value;
            let open_notional_mismatch = baseline_value < open_usd * Decimal::new(85, 2);
            if baseline_value.is_zero() || vs_mark || zero_current_open || open_notional_mismatch {
                baseline_value = open_usd;
                baseline_note = Some(
                    baseline_note
                        .map(|n| format!("{n} baseline_from_ledger_open_quote_usd."))
                        .unwrap_or_else(|| "baseline_from_ledger_open_quote_usd.".to_string()),
                );
            }
        }
    }

    let realized_cashflow_usd = if let (Some(a), Some(b)) = (mint_a.clone(), mint_b.clone()) {
        let mut mints: BTreeSet<String> = BTreeSet::new();
        mints.insert(a.clone());
        mints.insert(b.clone());
        let (px, _src) = match timeout(Duration::from_secs(2), fetch_mint_prices_usd(&mints)).await
        {
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

    let (mut collect_events, mut fees_collected_usd, mut collect_by_mint) =
        lp_fees_collected_usd_from_ledger_db(state, db, position_pubkey).await?;

    // When DB ledger rows are present but sparse (common after schema/drift/partial ingest),
    // bridge tx/collect aggregates from lifecycle JSONL to avoid misleading zeros for closed chains.
    let mut db_ledger_fallback_note = String::new();
    if fee_lamports_u == 0 || (collect_events == 0 && fees_collected_usd.is_zero()) {
        let lifecycle_rows = lifecycle_rows_cached_best_effort().await;
        if fee_lamports_u == 0 {
            let lifecycle_tx_fee_lamports: u64 = lifecycle_rows
                .iter()
                .filter(|r| r.position_pubkey.as_deref() == Some(position_pubkey))
                .filter_map(|r| r.tx_fee_lamports)
                .sum();
            if lifecycle_tx_fee_lamports > 0 {
                fee_lamports_u = lifecycle_tx_fee_lamports;
                tx_fees_usd = if sol_usd > 0.0 {
                    Decimal::from_f64_retain((fee_lamports_u as f64 / 1e9) * sol_usd)
                        .unwrap_or(Decimal::ZERO)
                } else {
                    Decimal::ZERO
                };
                db_ledger_fallback_note.push_str(" tx_fees_from_lifecycle_fallback.");
            }
        }
        if collect_events == 0 && fees_collected_usd.is_zero() {
            let (lc_events, lc_fees_usd, lc_by_mint) =
                lp_fees_collected_usd_from_lifecycle_rows(state, &lifecycle_rows, position_pubkey)
                    .await;
            if lc_events > 0 || !lc_fees_usd.is_zero() {
                collect_events = lc_events;
                fees_collected_usd = lc_fees_usd;
                collect_by_mint = lc_by_mint;
                db_ledger_fallback_note.push_str(" collect_fees_from_lifecycle_fallback.");
            }
        }
    }

    let collected_a_ui = mint_a
        .as_deref()
        .and_then(|m| collect_by_mint.get(m).copied());
    let collected_b_ui = mint_b
        .as_deref()
        .and_then(|m| collect_by_mint.get(m).copied());
    // Show zero only when we have explicit evidence for that leg (e.g. authoritative raw value).
    // Keep "—" when the leg is unknown/missing in source data.
    let mut fees_collected_token_a_ui = ((collect_events > 0) && mint_a.is_some())
        .then_some(collected_a_ui)
        .flatten();
    let mut fees_collected_token_b_ui = ((collect_events > 0) && mint_b.is_some())
        .then_some(collected_b_ui)
        .flatten();
    let bridged_missing_collect_leg = collect_events > 0
        && ((fees_collected_token_a_ui.is_some() && fees_collected_token_b_ui.is_none())
            || (fees_collected_token_a_ui.is_none() && fees_collected_token_b_ui.is_some()));
    if bridged_missing_collect_leg {
        if fees_collected_token_a_ui.is_none() && mint_a.is_some() {
            fees_collected_token_a_ui = Some(Decimal::ZERO);
        }
        if fees_collected_token_b_ui.is_none() && mint_b.is_some() {
            fees_collected_token_b_ui = Some(Decimal::ZERO);
        }
    }

    let fees_collected_token_a_raw =
        if let (Some(m), Some(ui)) = (mint_a.as_deref(), fees_collected_token_a_ui) {
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
    let fees_collected_token_b_raw =
        if let (Some(m), Some(ui)) = (mint_b.as_deref(), fees_collected_token_b_ui) {
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

    // Open PDA: align "current" with live mark (matches `/positions/{pda}`); DB snapshots can lag.
    let mut current_value_usd_live_note = String::new();
    if closed_ts.is_none() {
        if let Ok(pk) = solana_sdk::pubkey::Pubkey::from_str(position_pubkey) {
            if let Ok(Ok(pos)) = timeout(
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
                    && v.value_usd > Decimal::ZERO
                {
                    if (v.value_usd - current_value).abs() > Decimal::new(5, 2) {
                        current_value_usd_live_note.push_str(" current_value_usd_from_live_rpc.");
                    }
                    current_value = v.value_usd;
                }
            }
        }
    }

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

    let collect_zero_note = if collect_events > 0
        && fees_collected_token_a_ui.is_some_and(|v| v.is_zero())
        && fees_collected_token_b_ui.is_some_and(|v| v.is_zero())
    {
        " Collect tx executed, but pre-tx fee_owed_a/b were 0 for this node (no LP fees available at collect time)."
    } else if bridged_missing_collect_leg {
        " Collect tx executed; one LP leg was missing in source mapping and was normalized to 0 for pair completeness."
    } else {
        ""
    };

    Ok(PositionStreamLineageNode {
        position_address: position_pubkey.to_string(),
        token_a_label: mint_a.as_deref().map(token_short_label),
        token_b_label: mint_b.as_deref().map(token_short_label),
        token_mint_a: mint_a.clone(),
        token_mint_b: mint_b.clone(),
        opened_ts_utc: opened_ts.map(|t| t.to_rfc3339()),
        closed_ts_utc: closed_ts.map(|t| t.to_rfc3339()),
        baseline_value_usd: baseline_value,
        baseline_valuation_quality,
        current_value_usd: current_value,
        current_valuation_quality,
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
        note: Some(
            format!(
                "Best-effort per-PDA. tx_fee_lamports = sum of network fees for this PDA; fees_collected_usd = realized fee legs from collect + close rows × USD (mint map + fee_payer_token_*_delta_ui columns when present); tx fees use SOL/USD ({sol_src}). cashflow uses non-principal fee_payer_token_deltas (excluding open/close legs) × current mint USD prices when baseline mints are known.{}{}",
                baseline_note
                    .as_deref()
                    .map(|n| format!(" {n}."))
                    .unwrap_or_default(),
                db_ledger_fallback_note
            ) + collect_zero_note
                + current_value_usd_live_note.as_str(),
        ),
        collect_zero_diagnostics: None,
        chain_history_start_value_usd: None,
        chain_history_end_value_usd: None,
        chain_history_current_value_usd: None,
        chain_history_pool_address: None,
        chain_history_tick_lower_open: None,
        chain_history_tick_upper_open: None,
        chain_history_event_spot_token_a_usd_open: None,
        chain_history_event_spot_token_a_usd_close: None,
    })
}

#[derive(Debug, Clone)]
struct FastSnapshotMetric {
    ts_utc: Option<DateTime<Utc>>,
    value_usd: Decimal,
    token_mint_a: Option<String>,
    token_mint_b: Option<String>,
    kind: Option<String>,
    valuation_quality: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct FastFeeMetric {
    collect_events: u32,
    fees_collected_usd: Decimal,
    by_mint_ui: BTreeMap<String, Decimal>,
    /// On-chain pool token mint A/B (same order as Whirlpool `token_mint_a/b`) when fee rows carried a `pool_pubkey`.
    pool_mint_a: Option<String>,
    pool_mint_b: Option<String>,
}

/// Fill missing `token_mint_a` / `token_mint_b` on lineage nodes from batched fee rollup metadata.
/// Fast-path snapshots often omit one or both mints for mid-chain PDAs; `by_mint_ui` keys are still pool-canonical.
fn fill_missing_lineage_mints_from_fee_metric(
    mint_a: &mut Option<String>,
    mint_b: &mut Option<String>,
    fee: &FastFeeMetric,
) {
    let pool_pair = fee
        .pool_mint_a
        .as_ref()
        .zip(fee.pool_mint_b.as_ref())
        .map(|(a, b)| (a.as_str(), b.as_str()));
    if let Some((pa, pb)) = pool_pair {
        match (mint_a.as_ref(), mint_b.as_ref()) {
            (None, None) => {
                *mint_a = Some(pa.to_string());
                *mint_b = Some(pb.to_string());
            }
            (Some(a), None) => {
                if a == pa {
                    *mint_b = Some(pb.to_string());
                } else if a == pb {
                    *mint_b = Some(pa.to_string());
                } else {
                    *mint_b = Some(pb.to_string());
                }
            }
            (None, Some(b)) => {
                if b == pb {
                    *mint_a = Some(pa.to_string());
                } else if b == pa {
                    *mint_a = Some(pb.to_string());
                } else {
                    *mint_a = Some(pa.to_string());
                }
            }
            _ => {}
        }
        return;
    }
    if fee.by_mint_ui.len() != 2 {
        return;
    }
    let mut ks: Vec<String> = fee.by_mint_ui.keys().cloned().collect();
    ks.sort();
    let pa = ks[0].as_str();
    let pb = ks[1].as_str();
    match (mint_a.as_ref(), mint_b.as_ref()) {
        (None, None) => {
            *mint_a = Some(pa.to_string());
            *mint_b = Some(pb.to_string());
        }
        (Some(a), None) => {
            if a == pa {
                *mint_b = Some(pb.to_string());
            } else if a == pb {
                *mint_b = Some(pa.to_string());
            } else {
                *mint_b = Some(pb.to_string());
            }
        }
        (None, Some(b)) => {
            if b == pb {
                *mint_a = Some(pa.to_string());
            } else if b == pa {
                *mint_a = Some(pb.to_string());
            } else {
                *mint_a = Some(pa.to_string());
            }
        }
        _ => {}
    }
}

fn fees_collected_token_ui_for_fee_metric(
    mint_a: Option<&String>,
    mint_b: Option<&String>,
    fee_metric: &FastFeeMetric,
) -> (Option<Decimal>, Option<Decimal>) {
    let ev = fee_metric.collect_events;
    let collected_a_ui = mint_a
        .map(|s| s.as_str())
        .and_then(|m| fee_metric.by_mint_ui.get(m).copied());
    let collected_b_ui = mint_b
        .map(|s| s.as_str())
        .and_then(|m| fee_metric.by_mint_ui.get(m).copied());
    let mut fees_collected_token_a_ui = (ev > 0 && mint_a.is_some())
        .then_some(collected_a_ui)
        .flatten();
    let mut fees_collected_token_b_ui = (ev > 0 && mint_b.is_some())
        .then_some(collected_b_ui)
        .flatten();
    let bridged_missing_collect_leg = ev > 0
        && ((fees_collected_token_a_ui.is_some() && fees_collected_token_b_ui.is_none())
            || (fees_collected_token_a_ui.is_none() && fees_collected_token_b_ui.is_some()));
    if bridged_missing_collect_leg {
        if fees_collected_token_a_ui.is_none() && mint_a.is_some() {
            fees_collected_token_a_ui = Some(Decimal::ZERO);
        }
        if fees_collected_token_b_ui.is_none() && mint_b.is_some() {
            fees_collected_token_b_ui = Some(Decimal::ZERO);
        }
    }
    (fees_collected_token_a_ui, fees_collected_token_b_ui)
}

#[allow(clippy::too_many_lines)]
async fn lp_fees_collected_usd_from_ledger_db_batch(
    state: &AppState,
    db: &Database,
    positions: &[String],
) -> Result<HashMap<String, FastFeeMetric>, ApiError> {
    if positions.is_empty() {
        return Ok(HashMap::new());
    }

    let rows = sqlx::query(
        r#"
        SELECT position_pubkey, fee_payer_token_deltas, pool_pubkey,
               fee_payer_token_a_delta_ui, fee_payer_token_b_delta_ui,
               event,
               NULLIF(raw_json->>'lp_collected_token_a_raw', '')::BIGINT AS lp_collected_token_a_raw,
               NULLIF(raw_json->>'lp_collected_token_b_raw', '')::BIGINT AS lp_collected_token_b_raw,
               COALESCE(
                 NULLIF(raw_json->>'close_amount_a_raw', '')::BIGINT,
                 NULLIF(raw_json->'details'->>'close_amount_a_raw', '')::BIGINT
               ) AS close_amount_a_raw,
               COALESCE(
                 NULLIF(raw_json->>'close_amount_b_raw', '')::BIGINT,
                 NULLIF(raw_json->'details'->>'close_amount_b_raw', '')::BIGINT
               ) AS close_amount_b_raw,
               raw_json
        FROM position_stream_ledger_rows
        WHERE position_pubkey = ANY($1) AND event IN ('bot_collect_fees', 'bot_close_position')
        "#,
    )
    .bind(positions)
    .fetch_all(db.pool())
    .await
    .map_err(|e| ApiError::internal(format!("stream lineage: fast collect fee rows: {e}")))?;

    let mut pool_mints: HashMap<String, (String, String)> = HashMap::new();
    let mut mint_decimals: HashMap<String, u8> = HashMap::new();
    let mut out: HashMap<String, FastFeeMetric> = HashMap::new();
    let mut fallback_by_pos_mint_ui: HashMap<String, BTreeMap<String, Decimal>> = HashMap::new();

    for r in rows {
        let position: String = match r.try_get("position_pubkey") {
            Ok(p) => p,
            Err(_) => continue,
        };
        let event: Option<String> = r.try_get("event").ok();
        let is_collect = event.as_deref() == Some("bot_collect_fees");
        let is_close = event.as_deref() == Some("bot_close_position");
        if !(is_collect || is_close) {
            continue;
        }

        let pool: Option<String> = r.try_get("pool_pubkey").ok();
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

        {
            let m = out.entry(position.clone()).or_default();
            m.pool_mint_a = Some(ma.clone());
            m.pool_mint_b = Some(mb.clone());
        }

        let v: Option<serde_json::Value> = r.try_get("fee_payer_token_deltas").ok().flatten();
        let raw_json: Option<serde_json::Value> = r.try_get("raw_json").ok().flatten();
        let lp_raw_a: Option<i64> = r.try_get("lp_collected_token_a_raw").ok().flatten();
        let lp_raw_b: Option<i64> = r.try_get("lp_collected_token_b_raw").ok().flatten();
        let close_raw_a: Option<i64> = r.try_get("close_amount_a_raw").ok().flatten();
        let close_raw_b: Option<i64> = r.try_get("close_amount_b_raw").ok().flatten();
        let col_a = r
            .try_get::<Option<Decimal>, _>("fee_payer_token_a_delta_ui")
            .ok()
            .flatten()
            .filter(|d| *d > Decimal::ZERO)
            .unwrap_or(Decimal::ZERO);
        let col_b = r
            .try_get::<Option<Decimal>, _>("fee_payer_token_b_delta_ui")
            .ok()
            .flatten()
            .filter(|d| *d > Decimal::ZERO)
            .unwrap_or(Decimal::ZERO);

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
        let mut merged_a = map_a.max(col_a);
        let mut merged_b = map_b.max(col_b);
        let has_authoritative_pair = lp_raw_a.is_some()
            && lp_raw_b.is_some()
            && (lp_raw_a.unwrap_or(0) > 0 || lp_raw_b.unwrap_or(0) > 0);
        let mut raw_a_ui: Option<Decimal> = None;
        let mut raw_b_ui: Option<Decimal> = None;

        if let Some(raw) = lp_raw_a.filter(|x| *x > 0).map(|x| x as u64)
            && let Ok(pk) = solana_sdk::pubkey::Pubkey::from_str(ma.as_str())
        {
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
            raw_a_ui = Some(ui);
            merged_a = merged_a.max(ui);
        }
        if let Some(raw) = lp_raw_b.filter(|x| *x > 0).map(|x| x as u64)
            && let Ok(pk) = solana_sdk::pubkey::Pubkey::from_str(mb.as_str())
        {
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
            raw_b_ui = Some(ui);
            merged_b = merged_b.max(ui);
        }

        if has_authoritative_pair {
            if let Some(v) = raw_a_ui {
                merged_a = v;
            }
            if let Some(v) = raw_b_ui {
                merged_b = v;
            }
        }
        if is_close && !has_authoritative_pair {
            if close_raw_a.is_none() && close_raw_b.is_none() {
                let metric = out.entry(position.clone()).or_default();
                metric.collect_events = metric.collect_events.saturating_add(1);
                continue;
            }
            if let Some(raw) = close_raw_a.filter(|x| *x > 0).map(|x| x as u64)
                && let Ok(pk) = solana_sdk::pubkey::Pubkey::from_str(ma.as_str())
            {
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
                merged_a = (merged_a - decimal_ui_from_raw_u64(raw, dec)).max(Decimal::ZERO);
            }
            if let Some(raw) = close_raw_b.filter(|x| *x > 0).map(|x| x as u64)
                && let Ok(pk) = solana_sdk::pubkey::Pubkey::from_str(mb.as_str())
            {
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
                merged_b = (merged_b - decimal_ui_from_raw_u64(raw, dec)).max(Decimal::ZERO);
            }
        }

        {
            let metric = out.entry(position.clone()).or_default();
            metric.collect_events = metric.collect_events.saturating_add(1);
            if merged_a > Decimal::ZERO || has_authoritative_pair {
                *metric.by_mint_ui.entry(ma.clone()).or_insert(Decimal::ZERO) += merged_a;
            }
            if merged_b > Decimal::ZERO || has_authoritative_pair {
                *metric.by_mint_ui.entry(mb.clone()).or_insert(Decimal::ZERO) += merged_b;
            }
        }

        let event_spot = raw_json
            .as_ref()
            .and_then(|raw| raw.get("details"))
            .and_then(|details| event_spot_from_ledger_details(Some(details)))
            .or_else(|| event_spot_from_ledger_details(raw_json.as_ref()));
        if let Some((pa, pb, _src, _slot)) = event_spot {
            let pa_d = Decimal::from_f64_retain(pa).unwrap_or(Decimal::ZERO);
            let pb_d = Decimal::from_f64_retain(pb).unwrap_or(Decimal::ZERO);
            if let Some(metric) = out.get_mut(&position) {
                metric.fees_collected_usd += merged_a * pa_d + merged_b * pb_d;
            }
        } else {
            let fallback = fallback_by_pos_mint_ui.entry(position).or_default();
            if merged_a > Decimal::ZERO || has_authoritative_pair {
                *fallback.entry(ma).or_insert(Decimal::ZERO) += merged_a;
            }
            if merged_b > Decimal::ZERO || has_authoritative_pair {
                *fallback.entry(mb).or_insert(Decimal::ZERO) += merged_b;
            }
        }
    }

    let mints: BTreeSet<String> = fallback_by_pos_mint_ui
        .values()
        .flat_map(|m| m.keys().cloned())
        .collect();
    let (px, _) = fetch_mint_prices_usd_stable(&mints).await;
    for (position, by_mint) in fallback_by_pos_mint_ui {
        let Some(metric) = out.get_mut(&position) else {
            continue;
        };
        for (mint, amount) in by_mint {
            let p = px.get(&mint).copied().unwrap_or(0.0);
            if p > 0.0 && p.is_finite() {
                let pd = Decimal::from_f64_retain(p).unwrap_or(Decimal::ZERO);
                metric.fees_collected_usd += amount * pd;
            }
        }
    }

    Ok(out)
}

/// Latest open-quote USD per position from stream ledger (`details.open_quote_estimated_value_usd`, etc.).
async fn fetch_ledger_open_quote_usd_by_positions(
    db: &Database,
    chain: &[String],
) -> Result<HashMap<String, Decimal>, ApiError> {
    if chain.is_empty() {
        return Ok(HashMap::new());
    }
    let open_quote_rows = sqlx::query(
        r#"
        SELECT DISTINCT ON (position_pubkey)
          position_pubkey,
          COALESCE(
            NULLIF(raw_json #>> '{details,open_quote_estimated_value_usd}', '')::DOUBLE PRECISION,
            NULLIF(raw_json #>> '{details,open_target_usd}', '')::DOUBLE PRECISION,
            NULLIF(raw_json #>> '{details,open_prev_end_value_usd}', '')::DOUBLE PRECISION
          ) AS open_usd
        FROM position_stream_ledger_rows
        WHERE position_pubkey = ANY($1)
          AND event IN ('bot_open_position', 'bot_open_position_full_range', 'position_open')
        ORDER BY position_pubkey, ts_utc DESC
        "#,
    )
    .bind(chain)
    .fetch_all(db.pool())
    .await
    .map_err(|e| ApiError::internal(format!("stream lineage: open quote USD batch query: {e}")))?;

    let out: HashMap<String, Decimal> = open_quote_rows
        .into_iter()
        .filter_map(|r| {
            let position: String = r.try_get("position_pubkey").ok()?;
            let open_usd: Option<f64> = r.try_get::<Option<f64>, _>("open_usd").ok().flatten();
            let d = open_usd
                .and_then(Decimal::from_f64_retain)
                .unwrap_or(Decimal::ZERO);
            if d.is_zero() {
                None
            } else {
                Some((position, d))
            }
        })
        .collect();
    Ok(out)
}

/// Parse open USD fields from a lifecycle `bot_open_position` `details` object (same precedence as SQL).
fn open_quote_usd_from_open_details(
    details: &serde_json::Map<String, serde_json::Value>,
) -> Decimal {
    for key in [
        "open_quote_estimated_value_usd",
        "open_target_usd",
        "open_prev_end_value_usd",
    ] {
        let Some(v) = details.get(key) else {
            continue;
        };
        let f = v
            .as_f64()
            .or_else(|| v.as_str().and_then(|s| s.parse::<f64>().ok()));
        let Some(f) = f else { continue };
        let Some(d) = Decimal::from_f64_retain(f) else {
            continue;
        };
        if d > Decimal::ZERO {
            return d;
        }
    }
    Decimal::ZERO
}

/// Merge latest-per-PDA open-quote USD from lifecycle JSONL into `out` (max with any DB value).
fn merge_open_quote_usd_from_lifecycle_rows(
    rows: &[LifecycleRow],
    chain: &[String],
    out: &mut HashMap<String, Decimal>,
) {
    let want: HashSet<&str> = chain.iter().map(|s| s.trim()).collect();
    let mut best: HashMap<String, (Option<DateTime<Utc>>, Decimal)> = HashMap::new();
    for r in rows {
        let Some(pos) = r
            .position_pubkey
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        if !want.contains(pos) {
            continue;
        }
        if !is_lifecycle_open_event(r.event.as_deref()) {
            continue;
        }
        let Some(details) = r.details.as_ref().and_then(|v| v.as_object()) else {
            continue;
        };
        let usd = open_quote_usd_from_open_details(details);
        if usd.is_zero() {
            continue;
        }
        let ts = r.ts_utc;
        let pos_key = pos.to_string();
        let entry = best.entry(pos_key).or_insert((None, Decimal::ZERO));
        let replace = match (entry.0, ts) {
            (None, Some(_)) => true,
            (Some(et), Some(nt)) => nt >= et,
            (Some(_), None) => false,
            (None, None) => true,
        };
        if replace {
            *entry = (ts.or(entry.0), usd);
        }
    }
    for (pos, (_, usd)) in best {
        if usd.is_zero() {
            continue;
        }
        out.entry(pos)
            .and_modify(|e| {
                if usd > *e {
                    *e = usd;
                }
            })
            .or_insert(usd);
    }
}

/// [`node_metrics_fast_for_chain`] only reads `opened_ts_utc` / `closed_ts_utc` from DB valuation snapshots.
/// When snapshots are missing (common for mid-chain PDAs), the UI shows long runs of `—`. Lifecycle JSONL
/// has authoritative open/close timestamps and usually has `token_mint_a` / `token_mint_b` on bot rows.
fn hydrate_lineage_open_close_ts_and_mints_from_lifecycle(
    rows: &[LifecycleRow],
    nodes: &mut [PositionStreamLineageNode],
) {
    if nodes.is_empty() {
        return;
    }
    let want: HashSet<&str> = nodes.iter().map(|n| n.position_address.as_str()).collect();
    let mut first_open: HashMap<String, DateTime<Utc>> = HashMap::new();
    let mut last_close: HashMap<String, DateTime<Utc>> = HashMap::new();
    let mut mints: HashMap<String, (Option<String>, Option<String>)> = HashMap::new();
    for r in rows {
        let Some(pos) = r
            .position_pubkey
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        if !want.contains(pos) {
            continue;
        }
        let pos_key = pos.to_string();
        if let Some(ts) = r.ts_utc {
            if is_lifecycle_open_event(r.event.as_deref()) {
                first_open
                    .entry(pos_key.clone())
                    .and_modify(|e| {
                        if ts < *e {
                            *e = ts;
                        }
                    })
                    .or_insert(ts);
            }
            if is_lifecycle_close_event(r.event.as_deref()) {
                last_close
                    .entry(pos_key.clone())
                    .and_modify(|e| {
                        if ts > *e {
                            *e = ts;
                        }
                    })
                    .or_insert(ts);
            }
        }
        if let Some(d) = r.details.as_ref().and_then(|v| v.as_object()) {
            let ma = d
                .get("token_mint_a")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());
            let mb = d
                .get("token_mint_b")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());
            if ma.is_some() || mb.is_some() {
                let e = mints.entry(pos_key).or_insert((None, None));
                if e.0.is_none() {
                    e.0 = ma;
                }
                if e.1.is_none() {
                    e.1 = mb;
                }
            }
        }
    }
    for n in nodes.iter_mut() {
        let addr = n.position_address.clone();
        if n.opened_ts_utc.is_none() {
            if let Some(ts) = first_open.get(&addr) {
                n.opened_ts_utc = Some(ts.to_rfc3339());
            }
        }
        if n.closed_ts_utc.is_none() {
            if let Some(ts) = last_close.get(&addr) {
                n.closed_ts_utc = Some(ts.to_rfc3339());
            }
        }
        if let Some((ma, mb)) = mints.get(&addr) {
            if n.token_mint_a.is_none() {
                n.token_mint_a = ma.clone();
                n.token_a_label = n.token_mint_a.as_deref().map(token_short_label);
            }
            if n.token_mint_b.is_none() {
                n.token_mint_b = mb.clone();
                n.token_b_label = n.token_mint_b.as_deref().map(token_short_label);
            }
        }
    }
}

/// After rotation continuity fills `current_value_usd` / baselines, re-apply open-quote baseline lift.
///
/// [`node_metrics_fast_for_chain`] runs **before** [`apply_end_value_fallback_from_next_baseline`]. For
/// closed nodes without an `end_close` snapshot, `current_value_usd` is still zero there, so the
/// inline `baseline < 60% * current` guard never fires and one-leg snapshot baselines (e.g. ~$0.84)
/// survive. This pass uses the final `current_value_usd` plus `closed_ts_utc` heuristics.
fn apply_open_quote_baseline_lift_after_lineage_fallbacks(
    nodes: &mut [PositionStreamLineageNode],
    chain: &[String],
    open_usd_by_pos: &HashMap<String, Decimal>,
) {
    for (i, addr) in chain.iter().enumerate() {
        if i >= nodes.len() {
            break;
        }
        let Some(open_usd) = open_usd_by_pos.get(addr.trim()).copied() else {
            continue;
        };
        let n = &mut nodes[i];
        if open_usd <= n.baseline_value_usd {
            continue;
        }
        let has_end = n.current_value_usd > Decimal::ZERO;
        let suspicious_vs_mark =
            has_end && n.baseline_value_usd < n.current_value_usd * Decimal::new(60, 2);
        let suspicious_closed_vs_quote =
            n.closed_ts_utc.is_some() && n.baseline_value_usd < open_usd * Decimal::new(85, 2);
        let suspicious_open_vs_quote =
            n.closed_ts_utc.is_none() && n.baseline_value_usd < open_usd * Decimal::new(85, 2);
        if !(n.baseline_value_usd.is_zero()
            || suspicious_vs_mark
            || suspicious_closed_vs_quote
            || suspicious_open_vs_quote)
        {
            continue;
        }
        n.baseline_value_usd = open_usd;
        n.net_pnl_usd =
            n.current_value_usd + n.realized_cashflow_usd - n.baseline_value_usd - n.tx_fees_usd;
        if !n.baseline_value_usd.is_zero() {
            n.net_pnl_pct = n.net_pnl_usd / n.baseline_value_usd;
        }
        let append = " open_quote_baseline_lift_post_fallbacks.";
        if let Some(ref mut note) = n.note {
            note.push_str(append);
        } else {
            n.note = Some(append.trim().to_string());
        }
    }
}

/// After loading materialized chain-history nodes from `raw_snapshot`, re-apply open-quote baseline lift
/// using **current** `position_stream_ledger_rows` + lifecycle JSONL (same inputs as live stream-lineage).
///
/// Snapshots can be older than the stream ledger table (open rows appended post-materialize); without this,
/// `GET …/chain-history` often shows empty "start" baselines while `GET …/stream-lineage` does not.
pub async fn enrich_chain_history_nodes_open_quote_baseline_lift(
    state: &AppState,
    chain: &[String],
    nodes: &mut [PositionStreamLineageNode],
) -> Result<(), ApiError> {
    if chain.is_empty() || nodes.is_empty() {
        return Ok(());
    }
    let Some(db) = state.db.as_ref() else {
        return Ok(());
    };
    let mut open_usd_map = fetch_ledger_open_quote_usd_by_positions(db, chain).await?;
    let rows = lifecycle_rows_cached_best_effort().await;
    merge_open_quote_usd_from_lifecycle_rows(&rows, chain, &mut open_usd_map);
    apply_open_quote_baseline_lift_after_lineage_fallbacks(nodes, chain, &open_usd_map);
    Ok(())
}

/// Refresh per-PDA **network tx fees** and **LP fees collected** from `position_stream_ledger_rows`
/// (+ lifecycle JSONL fallback when DB rows are sparse), then recompute `net_pnl_*`.
///
/// Used by `GET …/chain-history`: materialized `raw_snapshot` can freeze zeros while the ledger table
/// already has rows (or rows arrive after materialize).
pub(crate) async fn refresh_chain_history_node_fees_from_ledger(
    state: &AppState,
    chain: &[String],
    nodes: &mut [PositionStreamLineageNode],
) -> Result<(), ApiError> {
    let Some(db) = state.db.as_ref() else {
        return Ok(());
    };
    if chain.is_empty() || nodes.is_empty() {
        return Ok(());
    }

    let fee_rows = sqlx::query(
        r#"
        SELECT position_pubkey, COALESCE(SUM(tx_fee_lamports), 0) AS fee_lamports
        FROM position_stream_ledger_rows
        WHERE position_pubkey = ANY($1)
        GROUP BY position_pubkey
        "#,
    )
    .bind(chain)
    .fetch_all(db.pool())
    .await
    .map_err(|e| ApiError::internal(format!("chain-history: tx fee batch: {e}")))?;

    let fee_lamports_by_pos: HashMap<String, u64> = fee_rows
        .into_iter()
        .filter_map(|r| {
            let position: String = r.try_get("position_pubkey").ok()?;
            let fee: i64 = r.try_get("fee_lamports").unwrap_or(0);
            Some((position, fee.max(0) as u64))
        })
        .collect();

    let mut fee_metrics = lp_fees_collected_usd_from_ledger_db_batch(state, db, chain).await?;
    let needs_lifecycle_fee_fallback = chain.iter().any(|p| {
        fee_metrics.get(p).is_none_or(|m| {
            m.collect_events == 0 && m.fees_collected_usd.is_zero() && m.by_mint_ui.is_empty()
        })
    });
    let lifecycle_rows = if needs_lifecycle_fee_fallback {
        Some(lifecycle_rows_cached_best_effort().await)
    } else {
        None
    };
    if let Some(rows) = lifecycle_rows.as_ref() {
        for p in chain {
            let has_fee_metric = fee_metrics.get(p).is_some_and(|m| {
                m.collect_events > 0 || !m.fees_collected_usd.is_zero() || !m.by_mint_ui.is_empty()
            });
            if has_fee_metric {
                continue;
            }
            let (collect_events, fees_collected_usd, by_mint_ui) =
                lp_fees_collected_usd_from_lifecycle_rows(state, rows, p).await;
            if collect_events > 0 || !fees_collected_usd.is_zero() || !by_mint_ui.is_empty() {
                fee_metrics.insert(
                    p.clone(),
                    FastFeeMetric {
                        collect_events,
                        fees_collected_usd,
                        by_mint_ui,
                        pool_mint_a: None,
                        pool_mint_b: None,
                    },
                );
            }
        }
    }

    let (sol_usd, _) = sol_usd().await;
    for node in nodes {
        let p = node.position_address.trim().to_string();
        let fee_lamports = fee_lamports_by_pos.get(&p).copied().unwrap_or(0);
        let tx_fees_usd = if sol_usd > 0.0 {
            Decimal::from_f64_retain((fee_lamports as f64 / 1e9) * sol_usd).unwrap_or(Decimal::ZERO)
        } else {
            Decimal::ZERO
        };
        let fee_metric = fee_metrics.get(&p).cloned().unwrap_or_default();
        let mut mint_a = node.token_mint_a.clone();
        let mut mint_b = node.token_mint_b.clone();
        fill_missing_lineage_mints_from_fee_metric(&mut mint_a, &mut mint_b, &fee_metric);
        node.token_mint_a = mint_a.clone();
        node.token_mint_b = mint_b.clone();
        node.token_a_label = mint_a.as_deref().map(token_short_label);
        node.token_b_label = mint_b.as_deref().map(token_short_label);
        let (fees_collected_token_a_ui, fees_collected_token_b_ui) =
            fees_collected_token_ui_for_fee_metric(mint_a.as_ref(), mint_b.as_ref(), &fee_metric);

        node.tx_fee_lamports = fee_lamports;
        node.tx_fees_usd = tx_fees_usd;
        node.collect_events = fee_metric.collect_events;
        node.fees_collected_usd = fee_metric.fees_collected_usd;
        node.fees_collected_token_a_ui = fees_collected_token_a_ui;
        node.fees_collected_token_b_ui = fees_collected_token_b_ui;

        node.net_pnl_usd = node.current_value_usd + node.realized_cashflow_usd
            - node.baseline_value_usd
            - node.tx_fees_usd;
        if !node.baseline_value_usd.is_zero() {
            node.net_pnl_pct = node.net_pnl_usd / node.baseline_value_usd;
        } else {
            node.net_pnl_pct = Decimal::ZERO;
        }
    }

    Ok(())
}

async fn node_metrics_fast_for_chain(
    state: &AppState,
    chain: &[String],
) -> Result<Vec<PositionStreamLineageNode>, ApiError> {
    let Some(db) = state.db.as_ref() else {
        let rows = lifecycle_rows_cached_best_effort().await;
        let mut out = Vec::with_capacity(chain.len());
        for p in chain {
            out.push(node_metrics_from_lifecycle_best_effort(state, &rows, p).await?);
        }
        return Ok(out);
    };
    if chain.is_empty() {
        return Ok(Vec::new());
    }

    let baseline_rows = sqlx::query(
        r#"
        SELECT DISTINCT ON (position_pubkey)
          position_pubkey, ts_utc, value_usd, token_mint_a, token_mint_b, raw_json
        FROM position_stream_valuation_snapshots
        WHERE position_pubkey = ANY($1)
        ORDER BY
          position_pubkey,
          CASE WHEN COALESCE(raw_json->>'kind', '') = 'baseline_open' THEN 0 ELSE 1 END,
          ts_utc ASC
        "#,
    )
    .bind(chain)
    .fetch_all(db.pool())
    .await
    .map_err(|e| ApiError::internal(format!("stream lineage: fast baseline query: {e}")))?;

    let current_rows = sqlx::query(
        r#"
        SELECT DISTINCT ON (position_pubkey)
          position_pubkey, ts_utc, value_usd, token_mint_a, token_mint_b, raw_json
        FROM position_stream_valuation_snapshots
        WHERE position_pubkey = ANY($1)
        ORDER BY
          position_pubkey,
          CASE WHEN COALESCE(raw_json->>'kind', '') = 'end_close' THEN 0 ELSE 1 END,
          ts_utc DESC
        "#,
    )
    .bind(chain)
    .fetch_all(db.pool())
    .await
    .map_err(|e| ApiError::internal(format!("stream lineage: fast current query: {e}")))?;

    let fee_rows = sqlx::query(
        r#"
        SELECT position_pubkey, COALESCE(SUM(tx_fee_lamports), 0) AS fee_lamports
        FROM position_stream_ledger_rows
        WHERE position_pubkey = ANY($1)
        GROUP BY position_pubkey
        "#,
    )
    .bind(chain)
    .fetch_all(db.pool())
    .await
    .map_err(|e| ApiError::internal(format!("stream lineage: fast tx fee query: {e}")))?;

    // Guardrail: baseline snapshots derived from open deltas may miss one leg (e.g. WSOL/native),
    // which can massively understate "start value". Prefer the USD quote captured at open time
    // (deposit quote estimate / target) when it is meaningfully higher than snapshot baseline.
    let open_usd_by_pos = fetch_ledger_open_quote_usd_by_positions(db, chain).await?;

    let parse_metric = |r: sqlx::postgres::PgRow| -> Option<(String, FastSnapshotMetric)> {
        let position: String = r.try_get("position_pubkey").ok()?;
        let raw_json: Option<serde_json::Value> = r.try_get("raw_json").ok().flatten();
        let kind = raw_json
            .as_ref()
            .and_then(|v| v.get("kind"))
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let valuation_quality = raw_json
            .as_ref()
            .and_then(|v| v.get("valuation_quality"))
            .and_then(|v| v.as_str())
            .map(str::to_string);
        Some((
            position,
            FastSnapshotMetric {
                ts_utc: r
                    .try_get::<Option<DateTime<Utc>>, _>("ts_utc")
                    .ok()
                    .flatten(),
                value_usd: r.try_get("value_usd").unwrap_or(Decimal::ZERO),
                token_mint_a: r
                    .try_get::<Option<String>, _>("token_mint_a")
                    .ok()
                    .flatten(),
                token_mint_b: r
                    .try_get::<Option<String>, _>("token_mint_b")
                    .ok()
                    .flatten(),
                kind,
                valuation_quality,
            },
        ))
    };

    let baselines: HashMap<String, FastSnapshotMetric> =
        baseline_rows.into_iter().filter_map(parse_metric).collect();
    let currents: HashMap<String, FastSnapshotMetric> =
        current_rows.into_iter().filter_map(parse_metric).collect();
    let fee_lamports_by_pos: HashMap<String, u64> = fee_rows
        .into_iter()
        .filter_map(|r| {
            let position: String = r.try_get("position_pubkey").ok()?;
            let fee: i64 = r.try_get("fee_lamports").unwrap_or(0);
            Some((position, fee.max(0) as u64))
        })
        .collect();
    let mut fee_metrics = lp_fees_collected_usd_from_ledger_db_batch(state, db, chain).await?;
    let needs_lifecycle_fee_fallback = chain.iter().any(|p| {
        fee_metrics.get(p).is_none_or(|m| {
            m.collect_events == 0 && m.fees_collected_usd.is_zero() && m.by_mint_ui.is_empty()
        })
    });
    let lifecycle_rows = if needs_lifecycle_fee_fallback {
        Some(lifecycle_rows_cached_best_effort().await)
    } else {
        None
    };
    if let Some(rows) = lifecycle_rows.as_ref() {
        for p in chain {
            let has_fee_metric = fee_metrics.get(p).is_some_and(|m| {
                m.collect_events > 0 || !m.fees_collected_usd.is_zero() || !m.by_mint_ui.is_empty()
            });
            if has_fee_metric {
                continue;
            }
            let (collect_events, fees_collected_usd, by_mint_ui) =
                lp_fees_collected_usd_from_lifecycle_rows(state, rows, p).await;
            if collect_events > 0 || !fees_collected_usd.is_zero() || !by_mint_ui.is_empty() {
                fee_metrics.insert(
                    p.clone(),
                    FastFeeMetric {
                        collect_events,
                        fees_collected_usd,
                        by_mint_ui,
                        pool_mint_a: None,
                        pool_mint_b: None,
                    },
                );
            }
        }
    }

    let (sol_usd, _sol_src) = sol_usd().await;
    let mut out = Vec::with_capacity(chain.len());
    for p in chain {
        let baseline = baselines.get(p);
        let current = currents.get(p);
        let mut baseline_value = baseline.map(|m| m.value_usd).unwrap_or(Decimal::ZERO);
        let mut current_value = current.map(|m| m.value_usd).unwrap_or(Decimal::ZERO);
        if let Some(open_usd) = open_usd_by_pos.get(p).copied() {
            let looks_understated = baseline_value.is_zero()
                || (current_value > Decimal::ZERO
                    && baseline_value < current_value * Decimal::new(60, 2))
                || (current_value.is_zero() && open_usd > baseline_value);
            if looks_understated && open_usd > baseline_value {
                baseline_value = open_usd;
            }
        }
        let fee_lamports = fee_lamports_by_pos.get(p).copied().unwrap_or(0);
        let tx_fees_usd = if sol_usd > 0.0 {
            Decimal::from_f64_retain((fee_lamports as f64 / 1e9) * sol_usd).unwrap_or(Decimal::ZERO)
        } else {
            Decimal::ZERO
        };
        let closed_ts_pre =
            current.and_then(|m| closed_ts_for_snapshot_kind(m.kind.as_deref(), m.ts_utc));
        if closed_ts_pre.is_none() {
            if let Ok(pk) = solana_sdk::pubkey::Pubkey::from_str(p.trim()) {
                if let Ok(Ok(pos)) = timeout(
                    Duration::from_secs(2),
                    monitored_position_from_chain(state.provider.clone(), &pk),
                )
                .await
                {
                    let prices = fetch_prices_for_positions(
                        state.provider.clone(),
                        std::slice::from_ref(&pos),
                    )
                    .await;
                    if let Ok(v) =
                        compute_position_usd_valuation(state.provider.clone(), &pos, &prices).await
                        && v.value_usd > Decimal::ZERO
                    {
                        current_value = v.value_usd;
                    }
                }
            }
        }
        let net_pnl_usd = current_value - baseline_value - tx_fees_usd;
        let net_pnl_pct = if baseline_value.is_zero() {
            Decimal::ZERO
        } else {
            net_pnl_usd / baseline_value
        };
        let mut mint_a = baseline
            .and_then(|m| m.token_mint_a.clone())
            .or_else(|| current.and_then(|m| m.token_mint_a.clone()));
        let mut mint_b = baseline
            .and_then(|m| m.token_mint_b.clone())
            .or_else(|| current.and_then(|m| m.token_mint_b.clone()));
        let fee_metric = fee_metrics.get(p).cloned().unwrap_or_default();
        fill_missing_lineage_mints_from_fee_metric(&mut mint_a, &mut mint_b, &fee_metric);
        let (fees_collected_token_a_ui, fees_collected_token_b_ui) =
            fees_collected_token_ui_for_fee_metric(mint_a.as_ref(), mint_b.as_ref(), &fee_metric);
        let closed_ts = closed_ts_pre;
        out.push(PositionStreamLineageNode {
            position_address: p.clone(),
            token_a_label: mint_a.as_deref().map(token_short_label),
            token_b_label: mint_b.as_deref().map(token_short_label),
            token_mint_a: mint_a,
            token_mint_b: mint_b,
            opened_ts_utc: baseline.and_then(|m| m.ts_utc.map(|t| t.to_rfc3339())),
            closed_ts_utc: closed_ts.map(|t| t.to_rfc3339()),
            baseline_value_usd: baseline_value,
            baseline_valuation_quality: baseline.and_then(|m| m.valuation_quality.clone()),
            current_value_usd: current_value,
            current_valuation_quality: current.and_then(|m| m.valuation_quality.clone()),
            tx_fee_lamports: fee_lamports,
            tx_fees_usd,
            fees_collected_usd: fee_metric.fees_collected_usd,
            fees_collected_token_a_ui,
            fees_collected_token_b_ui,
            fees_collected_token_a_raw: None,
            fees_collected_token_b_raw: None,
            collect_events: fee_metric.collect_events,
            realized_cashflow_usd: Decimal::ZERO,
            net_pnl_usd,
            net_pnl_pct,
            note: Some("fast_long_chain_metrics: per-node values use batched DB snapshots and batched ledger fee rollups; detailed non-fee cashflow is intentionally omitted from this hot path.".to_string()),
            collect_zero_diagnostics: None,
            chain_history_start_value_usd: None,
            chain_history_end_value_usd: None,
            chain_history_current_value_usd: None,
            chain_history_pool_address: None,
            chain_history_tick_lower_open: None,
            chain_history_tick_upper_open: None,
            chain_history_event_spot_token_a_usd_open: None,
            chain_history_event_spot_token_a_usd_close: None,
        });
    }
    Ok(out)
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
    let mut close_amount_a_raw: Option<u64> = None;
    let mut close_amount_b_raw: Option<u64> = None;

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
            Some("bot_open_position")
            | Some("bot_open_position_full_range")
            | Some("position_open") => {
                if opened_ts.is_none() {
                    opened_ts = r.ts_utc;
                }
                if open_leg_deltas.is_none()
                    && let Some(obj) = r
                        .fee_payer_token_deltas
                        .as_ref()
                        .and_then(|v| v.as_object())
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
                if (open_amount_a_cap.is_none() || open_amount_b_cap.is_none())
                    && r.details.is_some()
                    && let Some(d) = r.details.as_ref().and_then(|v| v.as_object())
                {
                    if open_amount_a_cap.is_none() {
                        open_amount_a_cap = d.get("amount_a_cap").and_then(|v| v.as_u64());
                    }
                    if open_amount_b_cap.is_none() {
                        open_amount_b_cap = d.get("amount_b_cap").and_then(|v| v.as_u64());
                    }
                }
            }
            Some("bot_close_position") | Some("position_close") => {
                closed_ts = r.ts_utc.or(closed_ts);
                if close_leg_deltas.is_none()
                    && let Some(obj) = r
                        .fee_payer_token_deltas
                        .as_ref()
                        .and_then(|v| v.as_object())
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
                if (close_amount_a_raw.is_none() || close_amount_b_raw.is_none())
                    && r.details.is_some()
                    && let Some(d) = r.details.as_ref().and_then(|v| v.as_object())
                {
                    if close_amount_a_raw.is_none() {
                        close_amount_a_raw = d.get("close_amount_a_raw").and_then(|v| v.as_u64());
                    }
                    if close_amount_b_raw.is_none() {
                        close_amount_b_raw = d.get("close_amount_b_raw").and_then(|v| v.as_u64());
                    }
                }
            }
            _ => {}
        }

        // Parse token deltas (string decimals) for realized cashflow — exclude open/close principal.
        let is_principal = is_lifecycle_open_event(r.event.as_deref())
            || is_lifecycle_close_event(r.event.as_deref());
        if !is_principal
            && let Some(obj) = r
                .fee_payer_token_deltas
                .as_ref()
                .and_then(|v| v.as_object())
        {
            for (mint, dv) in obj {
                if let Some(d) = dec_from_any(dv) {
                    *mint_deltas.entry(mint.clone()).or_insert(Decimal::ZERO) += d;
                }
            }
        }

        // Pull leg mints from details if present.
        if (mint_a.is_none() || mint_b.is_none())
            && let Some(details) = r.details.as_ref().and_then(|v| v.as_object())
        {
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
        .and_then(|m| collect_by_mint.get(m).copied());
    let collected_b_ui = mint_b
        .as_deref()
        .and_then(|m| collect_by_mint.get(m).copied());
    // Show zero only when we have explicit evidence for that leg (e.g. authoritative raw value).
    // Keep "—" when the leg is unknown/missing in source data.
    let mut fees_collected_token_a_ui = ((collect_events > 0) && mint_a.is_some())
        .then_some(collected_a_ui)
        .flatten();
    let mut fees_collected_token_b_ui = ((collect_events > 0) && mint_b.is_some())
        .then_some(collected_b_ui)
        .flatten();
    let bridged_missing_collect_leg = collect_events > 0
        && ((fees_collected_token_a_ui.is_some() && fees_collected_token_b_ui.is_none())
            || (fees_collected_token_a_ui.is_none() && fees_collected_token_b_ui.is_some()));
    if bridged_missing_collect_leg {
        if fees_collected_token_a_ui.is_none() && mint_a.is_some() {
            fees_collected_token_a_ui = Some(Decimal::ZERO);
        }
        if fees_collected_token_b_ui.is_none() && mint_b.is_some() {
            fees_collected_token_b_ui = Some(Decimal::ZERO);
        }
    }

    let fees_collected_token_a_raw =
        if let (Some(m), Some(ui)) = (mint_a.as_deref(), fees_collected_token_a_ui) {
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
    let fees_collected_token_b_raw =
        if let (Some(m), Some(ui)) = (mint_b.as_deref(), fees_collected_token_b_ui) {
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
            let (px, src) = fetch_mint_prices_usd_stable(&mints).await;
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

            let end_close = if let (Some(raw_a), Some(raw_b)) =
                (close_amount_a_raw, close_amount_b_raw)
            {
                let mint_a_pk = solana_sdk::pubkey::Pubkey::from_str(&a).ok();
                let mint_b_pk = solana_sdk::pubkey::Pubkey::from_str(&b).ok();
                if let (Some(ma), Some(mb)) = (mint_a_pk, mint_b_pk) {
                    let dec_a = fetch_mint_decimals_best_effort(state.provider.as_ref(), &ma).await;
                    let dec_b = fetch_mint_decimals_best_effort(state.provider.as_ref(), &mb).await;
                    if let (Some(da), Some(db)) = (dec_a, dec_b) {
                        let qa = decimal_ui_from_raw_u64(raw_a, da);
                        let qb = decimal_ui_from_raw_u64(raw_b, db);
                        Some(qa * pa_d + qb * pb_d)
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                close_leg_deltas.as_ref().map(|m| {
                    let da = m.get(&a).cloned().unwrap_or(Decimal::ZERO);
                    let dbb = m.get(&b).cloned().unwrap_or(Decimal::ZERO);
                    // Legacy fallback for old rows without deterministic close amounts.
                    da * pa_d + dbb * pb_d
                })
            };

            (baseline.max(Decimal::ZERO), realized_usd, end_close)
        } else {
            (Decimal::ZERO, Decimal::ZERO, None)
        };

    // Current value:
    // - when DB is enabled: try to get on-chain position valuation for open positions
    // - when DB is disabled: rely on close-leg deltas only (no on-chain calls)
    let mut current_value_usd = if !db_disabled && closed_ts.is_none() {
        if let Ok(pk) = solana_sdk::pubkey::Pubkey::from_str(position_pubkey) {
            if let Ok(Ok(pos)) = timeout(
                Duration::from_secs(2),
                monitored_position_from_chain(state.provider.clone(), &pk),
            )
            .await
            {
                let prices =
                    fetch_prices_for_positions(state.provider.clone(), std::slice::from_ref(&pos))
                        .await;
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
    if current_value_usd.is_zero()
        && let Some(end) = end_value_usd_from_close
    {
        current_value_usd = end;
    }

    // DB-disabled mode: active positions have no close leg yet. Still show a meaningful "end value"
    // using a best-effort on-chain valuation with a tight timeout to avoid UI stalls.
    if db_disabled
        && current_value_usd.is_zero()
        && closed_ts.is_none()
        && let Ok(pk) = solana_sdk::pubkey::Pubkey::from_str(position_pubkey)
        && let Ok(Ok(pos)) = timeout(
            Duration::from_secs(2),
            monitored_position_from_chain(state.provider.clone(), &pk),
        )
        .await
    {
        let prices =
            fetch_prices_for_positions(state.provider.clone(), std::slice::from_ref(&pos)).await;
        if let Ok(v) = compute_position_usd_valuation(state.provider.clone(), &pos, &prices).await
            && v.value_usd > Decimal::ZERO
        {
            current_value_usd = v.value_usd;
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
    ) && current_value_usd > Decimal::ZERO
        && baseline_value_usd > Decimal::ZERO
    {
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
                    let (px, _src) = fetch_mint_prices_usd_stable(&mints).await;
                    let pa = px.get(&a).copied().unwrap_or(0.0);
                    let pb = px.get(&b).copied().unwrap_or(0.0);
                    if pa.is_finite() && pb.is_finite() && pa > 0.0 && pb > 0.0 {
                        let cap_value_usd_f = ui_amount(cap_a, da) * pa + ui_amount(cap_b, db) * pb;
                        let cap_value_usd =
                            Decimal::from_f64_retain(cap_value_usd_f).unwrap_or(Decimal::ZERO);
                        let cap_not_too_high = if current_value_usd > Decimal::ZERO {
                            cap_value_usd <= current_value_usd * Decimal::new(135, 2)
                        } else {
                            true
                        };
                        if cap_not_too_high && cap_value_usd > baseline_value_usd {
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

    let net_pnl_usd = current_value_usd + realized_cashflow_usd - baseline_value_usd - tx_fees_usd;
    let net_pnl_pct = if baseline_value_usd.is_zero() {
        Decimal::ZERO
    } else {
        net_pnl_usd / baseline_value_usd
    };
    let price_note_s = price_note.clone().unwrap_or_default().to_ascii_lowercase();
    let baseline_valuation_quality = if baseline_value_usd > Decimal::ZERO {
        Some(if price_note_s.contains("timeout") {
            "fallback".to_string()
        } else {
            "exact".to_string()
        })
    } else {
        Some("missing_inputs".to_string())
    };
    let current_valuation_quality = if current_value_usd > Decimal::ZERO {
        Some(if price_note_s.contains("timeout") {
            "fallback".to_string()
        } else {
            "exact".to_string()
        })
    } else {
        Some("missing_inputs".to_string())
    };

    let collect_zero_note = if collect_events > 0
        && fees_collected_token_a_ui.is_some_and(|v| v.is_zero())
        && fees_collected_token_b_ui.is_some_and(|v| v.is_zero())
    {
        " collect executed with zero fee_owed_a/b on-chain for this node."
    } else if bridged_missing_collect_leg {
        " collect executed; one LP leg missing in source mapping was normalized to 0."
    } else {
        ""
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
        baseline_valuation_quality,
        current_value_usd,
        current_valuation_quality,
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
        note: Some(
            format!(
                "{} tx_fee_lamports = network fees for this PDA; fees_collected_usd = realized fee legs from collect + close rows × USD; tx fees use SOL/USD ({sol_src}). start/end value derived from open/close token deltas × current mint USD prices ({}).",
                if db_disabled {
                    "DB is disabled; per-node metrics from lifecycle JSONL (no on-chain valuation)."
                } else {
                    "Per-node metrics from lifecycle JSONL + on-chain valuation when available."
                },
                price_note.unwrap_or_else(|| "no price source".to_string())
            ) + collect_zero_note,
        ),
        collect_zero_diagnostics: None,
        chain_history_start_value_usd: None,
        chain_history_end_value_usd: None,
        chain_history_current_value_usd: None,
        chain_history_pool_address: None,
        chain_history_tick_lower_open: None,
        chain_history_tick_upper_open: None,
        chain_history_event_spot_token_a_usd_open: None,
        chain_history_event_spot_token_a_usd_close: None,
    })
}

/// When `position_stream_edges` lags JSONL (e.g. reopen PDA not ingested yet), DB-only lineage can stop
/// early while lifecycle already lists the same ordered prefix plus newer PDAs. Prefer the longer chain
/// only when it **extends** `from_db` as an exact prefix (avoid replacing unrelated orderings).
pub(crate) fn prefer_lifecycle_lineage_if_extends_db_prefix(
    from_db: Vec<String>,
    lifecycle: Vec<String>,
) -> Vec<String> {
    if from_db.is_empty() {
        return if lifecycle.is_empty() { from_db } else { lifecycle };
    }
    if lifecycle.len() > from_db.len()
        && from_db
            .iter()
            .enumerate()
            .all(|(i, p)| lifecycle.get(i) == Some(p))
    {
        lifecycle
    } else {
        from_db
    }
}

/// Same ordered PDA chain as [`compute_position_stream_lineage`] (oldest → newest along rotations).
/// Used by stream PnL/IL to anchor baseline snapshot on the **first** chain member and current on the **last**
/// (instead of global MIN/MAX `ts_utc` across unrelated PDAs in the BFS component).
pub(crate) async fn resolve_lineage_chain_for_stream_pnl(
    state: &AppState,
    perf: &crate::models::PositionStreamPerformanceResponse,
    entry: &str,
) -> Vec<String> {
    let entry = entry.trim();
    let Some(db) = state.db.as_ref() else {
        return vec![entry.to_string()];
    };

    let rows = lifecycle_rows_cached_best_effort().await;
    let stitch_suppressed = suppress_jsonl_rotation_stitch(&rows, entry);
    let stream_positions: Vec<String> = if stitch_suppressed {
        vec![entry.to_string()]
    } else {
        perf.positions.clone()
    };

    let edge_rows = sqlx::query(
        r#"
        SELECT ts_utc, old_position, new_position, rebalance_session_id
        FROM position_stream_edges
        WHERE old_position = ANY($1) OR new_position = ANY($1)
        "#,
    )
    .bind(&stream_positions)
    .fetch_all(db.pool())
    .await;

    let Ok(mut edge_rows) = edge_rows else {
        return vec![entry.to_string()];
    };

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

    let pos_set: HashSet<&str> = stream_positions.iter().map(|s| s.as_str()).collect();
    let entry_touches_db_edge = edges.iter().any(|(_, o, n, _)| {
        (o == entry || n == entry) && pos_set.contains(o.as_str()) && pos_set.contains(n.as_str())
    });

    let mut chain = build_lineage_chain_from_db_edges(&stream_positions, &edges, entry, 100);

    if chain.len() <= 1 && !entry_touches_db_edge {
        let rows_fb = lifecycle_rows_cached_best_effort().await;
        if !suppress_jsonl_rotation_stitch(&rows_fb, entry) {
            let reg_rows = registry_rows_best_effort();
            if !reg_rows.is_empty() {
                let rc = chain_from_registry_best_effort_rows(&reg_rows, entry, 50);
                if rc.len() > chain.len() {
                    chain = rc;
                }
            }

            let lc = chain_from_lifecycle_best_effort_rows(&rows_fb, entry, 25);
            if lc.len() > chain.len() {
                chain = lc;
            }
        }
    }

    if !stitch_suppressed {
        let lc = chain_from_lifecycle_best_effort_rows(&rows, entry, 100);
        chain = prefer_lifecycle_lineage_if_extends_db_prefix(chain, lc);
    }

    if chain.is_empty() {
        vec![entry.to_string()]
    } else {
        chain
    }
}

/// Options for [`compute_position_stream_lineage_opts`].
#[derive(Debug, Clone, Copy, Default)]
pub struct ComputePositionStreamLineageOpts {
    /// When true, **await** `persist_event_valuation_snapshots_for_positions` before per-node metrics
    /// (chains ≤ 8 PDAs). Used by **chain-history materialize** so `apply_open_start_usd_from_lifecycle_snapshots_for_chain_history`
    /// reads rows that were previously written only in a background `tokio::spawn` (race → `start_value_usd` = 0).
    pub await_valuation_snapshot_persist: bool,
}

/// Build an ordered stream lineage chain and enrich each node with best-effort metrics.
pub async fn compute_position_stream_lineage(
    state: &AppState,
    position_address: &str,
) -> Result<PositionStreamLineageResponse, ApiError> {
    compute_position_stream_lineage_opts(
        state,
        position_address,
        ComputePositionStreamLineageOpts::default(),
    )
    .await
}

/// Same as [`compute_position_stream_lineage`] with optional snapshot-persist **ordering** for writers.
pub async fn compute_position_stream_lineage_opts(
    state: &AppState,
    position_address: &str,
    opts: ComputePositionStreamLineageOpts,
) -> Result<PositionStreamLineageResponse, ApiError> {
    use crate::services::position_stream_pnl::compute_position_stream_pnl_for_stream_members;

    let entry = position_address.trim();

    // Connectivity + totals: reuse existing stream services.
    let perf = compute_position_stream_performance(state, entry, true).await?;
    let mut totals = None;

    if state.db.is_none() {
        let rows = lifecycle_rows_cached_best_effort().await;
        let stitch_suppressed = suppress_jsonl_rotation_stitch(&rows, entry);
        let chain = if stitch_suppressed {
            vec![entry.to_string()]
        } else {
            chain_from_lifecycle_best_effort_rows(&rows, entry, 25)
        };
        let mut nodes = Vec::new();
        for p in &chain {
            nodes.push(node_metrics_from_lifecycle_best_effort(state, &rows, p).await?);
        }

        apply_session_continuity_from_lifecycle_rows(&rows, &mut nodes);
        // Fill gaps in closed nodes (close row missing leg token deltas) by using next node baseline.
        apply_end_value_fallback_from_next_baseline(&mut nodes);
        apply_baseline_fallback_from_prev_end(&mut nodes);
        let cps = fee_checkpoint_rows_cached_best_effort().await;
        attach_collect_zero_diagnostics(&rows, &cps, &mut nodes);

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
                "DB is disabled; chain reconstructed best-effort from lifecycle JSONL (rotation signals: matching rebalance_session_id, or bot activity tied to the closed PDA in the pre-open window; 60m lookback)."
                    .to_string(),
            ),
            chain_history_materialized_ts_utc: None,
        });
    }

    // When lifecycle says this mint should not inherit prior pool rotation history (manual CLI open,
    // fresh API cost session, or unanchored bot open), the DB path must not use the full undirected
    // BFS component from `compute_position_stream_performance` — that can merge unrelated PDAs.
    let rows = lifecycle_rows_cached_best_effort().await;
    let stitch_suppressed = suppress_jsonl_rotation_stitch(&rows, entry);
    let chain = resolve_lineage_chain_for_stream_pnl(state, &perf, entry).await;

    totals = compute_position_stream_pnl_for_stream_members(
        state,
        entry,
        perf.positions.clone(),
        perf.sessions.clone(),
        Some(chain.as_slice()),
        false,
        false,
    )
    .await
    .ok();

    if state.db.is_some() && chain.len() <= 8 {
        if opts.await_valuation_snapshot_persist {
            persist_event_valuation_snapshots_for_positions(state, &rows, &chain).await?;
        } else {
            let st_bg = state.clone();
            let rows_bg = rows.clone();
            let chain_bg = chain.clone();
            tokio::spawn(async move {
                if let Err(e) =
                    persist_event_valuation_snapshots_for_positions(&st_bg, &rows_bg, &chain_bg)
                        .await
                {
                    tracing::warn!(error = %e, "stream lineage: background snapshot persist failed");
                }
            });
        }
    }

    let mut nodes: Vec<PositionStreamLineageNode> = if chain.len() > 8 {
        node_metrics_fast_for_chain(state, &chain).await?
    } else {
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
        let mut nodes = Vec::with_capacity(node_futs.len());
        for res in join_all(node_futs).await {
            nodes.push(res?);
        }
        nodes
    };

    hydrate_lineage_open_close_ts_and_mints_from_lifecycle(&rows, &mut nodes);

    apply_session_continuity_from_lifecycle_rows(&rows, &mut nodes);
    // Fill gaps in closed nodes (close row missing leg token deltas) by using next node baseline.
    // This is useful in DB mode too when per-PDA current/end valuation is missing.
    apply_end_value_fallback_from_next_baseline(&mut nodes);
    apply_baseline_fallback_from_prev_end(&mut nodes);
    // Long chains use batched fast metrics; short chains use per-`node_metrics`. Both need the same
    // post-fallback open-quote baseline lift (BUG-20260513-03 regressed when gated on `chain.len() > 8`).
    if let Some(db) = state.db.as_ref() {
        let mut open_usd_map = fetch_ledger_open_quote_usd_by_positions(db, &chain).await?;
        merge_open_quote_usd_from_lifecycle_rows(&rows, &chain, &mut open_usd_map);
        apply_open_quote_baseline_lift_after_lineage_fallbacks(&mut nodes, &chain, &open_usd_map);
    }
    let cps = fee_checkpoint_rows_cached_best_effort().await;
    attach_collect_zero_diagnostics(&rows, &cps, &mut nodes);

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
    let mut note = "Lineage chain is best-effort and assumes a mostly linear old→new rotation path (common for strategies). If edges are missing, the chain may be incomplete.".to_string();
    if stitch_suppressed {
        note.push_str(
            " Cross-PDA stream stitching was suppressed for this mint (operator open: CLI `position_open` / `source:cli` / API `open_origin=operator_api`, unanchored bot open, or non-rotation lifecycle); the history table lists this position only.",
        );
    }
    Ok(PositionStreamLineageResponse {
        position_address: entry.to_string(),
        chain,
        nodes,
        totals,
        chain_cost_summary,
        note: Some(note),
        chain_history_materialized_ts_utc: None,
    })
}

fn apply_end_value_fallback_from_next_baseline(nodes: &mut [PositionStreamLineageNode]) {
    for i in 0..nodes.len().saturating_sub(1) {
        let next_baseline = nodes[i + 1].baseline_value_usd;
        let is_closed = nodes[i].closed_ts_utc.is_some();
        let has_end = !nodes[i].current_value_usd.is_zero();
        if is_closed && !has_end && !next_baseline.is_zero() {
            nodes[i].current_value_usd = next_baseline;
            nodes[i].net_pnl_usd = nodes[i].current_value_usd + nodes[i].realized_cashflow_usd
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

fn apply_session_continuity_from_lifecycle_rows(
    rows: &[LifecycleRow],
    nodes: &mut [PositionStreamLineageNode],
) {
    if nodes.len() < 2 {
        return;
    }

    let mut close_by_sid: HashMap<String, (Option<DateTime<Utc>>, String)> = HashMap::new();
    let mut open_by_sid: HashMap<String, (Option<DateTime<Utc>>, String)> = HashMap::new();
    for r in rows {
        let Some(sid) = r
            .rebalance_session_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let Some(pos) = r
            .position_pubkey
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };

        if is_lifecycle_close_event(r.event.as_deref()) {
            let e = close_by_sid
                .entry(sid.to_string())
                .or_insert((r.ts_utc, pos.to_string()));
            if r.ts_utc >= e.0 {
                *e = (r.ts_utc, pos.to_string());
            }
        } else if is_lifecycle_open_event(r.event.as_deref()) {
            let e = open_by_sid
                .entry(sid.to_string())
                .or_insert((r.ts_utc, pos.to_string()));
            if r.ts_utc >= e.0 {
                *e = (r.ts_utc, pos.to_string());
            }
        }
    }

    let mut links: HashSet<(String, String)> = HashSet::new();
    for (sid, (_, old_pos)) in &close_by_sid {
        if let Some((_, new_pos)) = open_by_sid.get(sid) {
            links.insert((old_pos.clone(), new_pos.clone()));
        }
    }
    if links.is_empty() {
        return;
    }

    for i in 1..nodes.len() {
        let old = nodes[i - 1].position_address.clone();
        let newp = nodes[i].position_address.clone();
        if !links.contains(&(old, newp)) {
            continue;
        }
        // Preserve baseline computed directly from the open row (e.g. `open_amount_raw` / caps path).
        // Session continuity should only fill missing baseline, not overwrite explicit open valuation.
        if !nodes[i].baseline_value_usd.is_zero() {
            continue;
        }
        let prev_end = nodes[i - 1].current_value_usd;
        if prev_end > Decimal::ZERO {
            nodes[i].baseline_value_usd = prev_end;
            nodes[i].net_pnl_usd = nodes[i].current_value_usd + nodes[i].realized_cashflow_usd
                - nodes[i].baseline_value_usd
                - nodes[i].tx_fees_usd;
            if !nodes[i].baseline_value_usd.is_zero() {
                nodes[i].net_pnl_pct = nodes[i].net_pnl_usd / nodes[i].baseline_value_usd;
            }
            if let Some(ref mut n) = nodes[i].note {
                n.push_str(" baseline_from_rotation_session.");
            } else {
                nodes[i].note = Some("baseline_from_rotation_session.".to_string());
            }
        }
    }
}

fn apply_baseline_fallback_from_prev_end(nodes: &mut [PositionStreamLineageNode]) {
    for i in 1..nodes.len() {
        let prev_end = nodes[i - 1].current_value_usd;
        let has_baseline = !nodes[i].baseline_value_usd.is_zero();
        let continuity_plausible = if nodes[i].current_value_usd > Decimal::ZERO {
            prev_end <= nodes[i].current_value_usd * Decimal::new(135, 2)
        } else {
            true
        };
        if !has_baseline && !prev_end.is_zero() && continuity_plausible {
            nodes[i].baseline_value_usd = prev_end;
            nodes[i].net_pnl_usd = nodes[i].current_value_usd + nodes[i].realized_cashflow_usd
                - nodes[i].baseline_value_usd
                - nodes[i].tx_fees_usd;
            if !nodes[i].baseline_value_usd.is_zero() {
                nodes[i].net_pnl_pct = nodes[i].net_pnl_usd / nodes[i].baseline_value_usd;
            }
            if let Some(ref mut n) = nodes[i].note {
                n.push_str(
                    " baseline approximated from previous node end value (rotation continuity).",
                );
            } else {
                nodes[i].note = Some(
                    "baseline approximated from previous node end value (rotation continuity)."
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
    let realized_lp_fees_usd: Decimal = nodes.iter().map(|n| n.fees_collected_usd).sum();
    let clean_il_usd = Decimal::ZERO;
    let clean_il_pct = Decimal::ZERO;
    let lp_fees_total_usd = realized_lp_fees_usd;
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
        clean_il_usd,
        clean_il_pct,
        realized_lp_fees_usd,
        uncollected_lp_fees_usd: Decimal::ZERO,
        lp_fees_total_usd,
        lp_vs_hodl_with_fees_usd: lp_fees_total_usd,
        lp_vs_hodl_with_fees_pct: Decimal::ZERO,
        valuation_price_time_kind: "node_fallback_unavailable".to_string(),
        price_basis_note: Some(
            "Fallback totals from lineage nodes do not have baseline token basket, so HODL/IL price basis is unavailable.".to_string(),
        ),
        tx_fees_usd,
        realized_cashflow_usd,
        net_pnl_usd,
        net_pnl_pct,
        interpretation: crate::models::StreamPnLInterpretation {
            economic_net_pnl_caption_pl:
                "Wynik ekonomiczny (fallback z węzłów lineage, bez pełnych snapshotów DB): końcowy NAV + suma cashflow z węzłów − baseline pierwszego węzła − suma tx fees z węzłów."
                    .to_string(),
            il_vs_initial_hodl_caption_pl:
                "Benchmark IL vs HODL: w tym trybie nie liczony (brak ilości tokenów ze snapshotów); pola il_* są zerowe."
                    .to_string(),
        },
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
        let Some(pos) = r
            .position_pubkey
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
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
        let (Some(ma), Some(mb)) = (ma, mb) else {
            continue;
        };
        mints.insert(ma.clone());
        mints.insert(mb.clone());
        mints_by_pos.insert(pos.to_string(), (pool.to_string(), ma, mb));
    }
    let (px, _) = fetch_mint_prices_usd_stable(&mints).await;

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
        if let Some(open) = open_by_pos.get(pos)
            && let (Some(ts), Some(obj)) = (
                open.ts_utc,
                open.fee_payer_token_deltas
                    .as_ref()
                    .and_then(|v| v.as_object()),
            )
        {
            let da = obj
                .get(&mint_a)
                .and_then(dec_from_any)
                .unwrap_or(Decimal::ZERO);
            let dbb = obj
                .get(&mint_b)
                .and_then(dec_from_any)
                .unwrap_or(Decimal::ZERO);
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

        // End snapshot at close ts.
        if let Some(close) = close_by_pos.get(pos)
            && let (Some(ts), Some(obj)) = (
                close.ts_utc,
                close
                    .fee_payer_token_deltas
                    .as_ref()
                    .and_then(|v| v.as_object()),
            )
        {
            let da = obj
                .get(&mint_a)
                .and_then(dec_from_any)
                .unwrap_or(Decimal::ZERO);
            let dbb = obj
                .get(&mint_b)
                .and_then(dec_from_any)
                .unwrap_or(Decimal::ZERO);
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

/// Best-effort inference: parent PDA only when lifecycle shows a **rotation** into this open
/// (same rules as stream-lineage `lifecycle_rotation_parent_before_open`).
pub async fn infer_parent_position_from_lifecycle_best_effort(entry: &str) -> Option<String> {
    let entry = entry.trim();
    if entry.is_empty() {
        return None;
    }
    let rows = lifecycle_rows_cached_best_effort().await;
    if lifecycle_entry_open_is_operator_manual(&rows, entry) {
        return None;
    }
    let o = lifecycle_latest_open_row(&rows, entry)?;
    lifecycle_rotation_parent_before_open(&rows, o).map(std::string::ToString::to_string)
}

#[cfg(test)]
mod tests {
    use super::event_spot_from_ledger_details;
    use super::*;
    use crate::state::{ApiConfig, AppState};
    use clmm_lp_protocols::prelude::RpcConfig;
    use serde::Serialize;

    fn test_state_no_db() -> AppState {
        let rpc_config = RpcConfig {
            primary_url: "http://127.0.0.1:1".to_string(),
            ..Default::default()
        };
        AppState::new(rpc_config, ApiConfig::default(), None)
    }

    #[test]
    fn prefer_lifecycle_lineage_if_extends_db_prefix_extends_tail() {
        let db = vec!["a".to_string(), "b".to_string()];
        let lc = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(
            super::prefer_lifecycle_lineage_if_extends_db_prefix(db, lc),
            vec!["a", "b", "c"]
        );
    }

    #[test]
    fn prefer_lifecycle_lineage_if_extends_db_prefix_noop_when_not_prefix() {
        let db = vec!["b".to_string()];
        let lc = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(
            super::prefer_lifecycle_lineage_if_extends_db_prefix(db, lc),
            vec!["b"]
        );
    }

    #[test]
    fn prefer_lifecycle_lineage_if_extends_db_prefix_empty_db_uses_lifecycle() {
        let db = vec![];
        let lc = vec!["x".to_string()];
        assert_eq!(
            super::prefer_lifecycle_lineage_if_extends_db_prefix(db, lc),
            vec!["x"]
        );
    }

    #[test]
    fn stream_lineage_does_not_call_ingest_enabled_pnl_wrapper() {
        let source = include_str!("position_stream_lineage.rs");
        let wrapper_call = concat!("compute_position_stream_", "pnl(");

        assert!(
            !source.contains(wrapper_call),
            "stream-lineage hot path must use compute_position_stream_pnl_for_stream_members with the already-computed perf instead of the wrapper that recomputes stream performance with ingest enabled"
        );
        assert!(source.contains("compute_position_stream_pnl_for_stream_members"));
    }

    #[test]
    fn stream_lineage_long_chains_use_batched_node_metrics() {
        let source = include_str!("position_stream_lineage.rs");

        assert!(source.contains("node_metrics_fast_for_chain(state, &chain).await?"));
        assert!(
            source.contains("lp_fees_collected_usd_from_ledger_db_batch(state, db, chain).await?")
        );
        assert!(
            source.contains("open_quote_estimated_value_usd"),
            "fast path should keep baseline correction from open quote USD"
        );
        assert!(
            source.contains("apply_open_quote_baseline_lift_after_lineage_fallbacks"),
            "long-chain lineage must re-lift understated baselines after rotation end-value fallbacks"
        );
        assert!(
            source.contains("chain.len() > 8"),
            "long rotation chains must stay on the batched node-metrics path instead of fanning out full node_metrics calls per PDA"
        );
        assert!(source.contains("fees_collected_usd: fee_metric.fees_collected_usd"));
        assert!(source.contains("collect_events: fee_metric.collect_events"));
    }

    #[test]
    fn fill_missing_mints_from_fee_metric_restores_sol_usdc_fee_legs() {
        let wsol = WSOL_MINT.to_string();
        let usdc = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string();
        let mut mint_a: Option<String> = None;
        let mut mint_b: Option<String> = None;
        let mut fee = FastFeeMetric::default();
        fee.collect_events = 1;
        fee.pool_mint_a = Some(wsol.clone());
        fee.pool_mint_b = Some(usdc.clone());
        fee
            .by_mint_ui
            .insert(wsol.clone(), Decimal::new(11434, 9));
        fee
            .by_mint_ui
            .insert(usdc.clone(), Decimal::new(145, 6));
        fill_missing_lineage_mints_from_fee_metric(&mut mint_a, &mut mint_b, &fee);
        assert_eq!(mint_a.as_ref(), Some(&wsol));
        assert_eq!(mint_b.as_ref(), Some(&usdc));
        let (a_ui, b_ui) =
            fees_collected_token_ui_for_fee_metric(mint_a.as_ref(), mint_b.as_ref(), &fee);
        assert!(a_ui.is_some_and(|v| v > Decimal::ZERO));
        assert!(b_ui.is_some_and(|v| v > Decimal::ZERO));
    }

    #[test]
    fn merge_open_quote_usd_from_lifecycle_rows_fills_missing_db_map() {
        let ts = DateTime::parse_from_rfc3339("2026-05-13T12:15:58.385Z")
            .unwrap()
            .with_timezone(&Utc);
        let row = LifecycleRow {
            ts_utc: Some(ts),
            event: Some("bot_open_position".to_string()),
            pool_address: None,
            position_pubkey: Some("2Zjk86Sb5sM5T54CefNtjK6BypP8SUjujE1wokfce4qP".to_string()),
            fee_payer_pubkey: None,
            rebalance_session_id: None,
            tx_fee_lamports: None,
            fee_payer_token_deltas: None,
            fee_payer_token_a_delta_ui: None,
            fee_payer_token_b_delta_ui: None,
            lp_collected_token_a_raw: None,
            lp_collected_token_b_raw: None,
            details: Some(serde_json::json!({
                "open_quote_estimated_value_usd": 9.955919156645884
            })),
            source: None,
        };
        let chain = vec!["2Zjk86Sb5sM5T54CefNtjK6BypP8SUjujE1wokfce4qP".to_string()];
        let mut out = HashMap::new();
        merge_open_quote_usd_from_lifecycle_rows(&[row], &chain, &mut out);
        let v = out
            .get("2Zjk86Sb5sM5T54CefNtjK6BypP8SUjujE1wokfce4qP")
            .copied()
            .unwrap();
        let expected = Decimal::from_f64_retain(9.955919156645884).unwrap();
        assert!((v - expected).abs() < Decimal::new(1, 6));
    }

    #[test]
    fn hydrate_lineage_fills_open_close_ts_and_mints_from_lifecycle() {
        let ts_open = DateTime::parse_from_rfc3339("2026-05-13T12:15:58.385Z")
            .unwrap()
            .with_timezone(&Utc);
        let ts_close = DateTime::parse_from_rfc3339("2026-05-13T13:05:51.721Z")
            .unwrap()
            .with_timezone(&Utc);
        let rows = vec![
            LifecycleRow {
                ts_utc: Some(ts_open),
                event: Some("bot_open_position".to_string()),
                pool_address: Some("Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE".to_string()),
                position_pubkey: Some("2Zjk86Sb5sM5T54CefNtjK6BypP8SUjujE1wokfce4qP".to_string()),
                fee_payer_pubkey: None,
                rebalance_session_id: None,
                tx_fee_lamports: None,
                fee_payer_token_deltas: None,
                fee_payer_token_a_delta_ui: None,
                fee_payer_token_b_delta_ui: None,
                lp_collected_token_a_raw: None,
                lp_collected_token_b_raw: None,
                details: Some(serde_json::json!({
                    "token_mint_a": "So11111111111111111111111111111111111111112",
                    "token_mint_b": "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
                })),
                source: None,
            },
            LifecycleRow {
                ts_utc: Some(ts_close),
                event: Some("bot_close_position".to_string()),
                pool_address: Some("Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE".to_string()),
                position_pubkey: Some("2Zjk86Sb5sM5T54CefNtjK6BypP8SUjujE1wokfce4qP".to_string()),
                fee_payer_pubkey: None,
                rebalance_session_id: None,
                tx_fee_lamports: None,
                fee_payer_token_deltas: None,
                fee_payer_token_a_delta_ui: None,
                fee_payer_token_b_delta_ui: None,
                lp_collected_token_a_raw: None,
                lp_collected_token_b_raw: None,
                details: None,
                source: None,
            },
        ];
        let mut nodes = vec![mk_node(
            "2Zjk86Sb5sM5T54CefNtjK6BypP8SUjujE1wokfce4qP",
            Decimal::ZERO,
            Decimal::ZERO,
        )];
        hydrate_lineage_open_close_ts_and_mints_from_lifecycle(&rows, &mut nodes);
        assert_eq!(
            nodes[0].opened_ts_utc.as_deref(),
            Some(ts_open.to_rfc3339().as_str())
        );
        assert_eq!(
            nodes[0].closed_ts_utc.as_deref(),
            Some(ts_close.to_rfc3339().as_str())
        );
        assert_eq!(
            nodes[0].token_mint_a.as_deref(),
            Some("So11111111111111111111111111111111111111112")
        );
        assert_eq!(
            nodes[0].token_mint_b.as_deref(),
            Some("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v")
        );
        assert_eq!(nodes[0].token_a_label.as_deref(), Some("SOL"));
        assert_eq!(nodes[0].token_b_label.as_deref(), Some("USDC"));
    }

    #[test]
    fn open_quote_baseline_lift_post_fallbacks_repairs_understated_closed_node() {
        let mut n = mk_node(
            "Aaiey3FyWAwAU8YC",
            Decimal::new(842, 3),
            Decimal::new(3301, 3),
        );
        n.closed_ts_utc = Some("2026-05-13T15:50:00Z".to_string());
        let mut nodes = vec![n];
        let chain = vec!["Aaiey3FyWAwAU8YC".to_string()];
        let mut m = HashMap::new();
        m.insert("Aaiey3FyWAwAU8YC".to_string(), Decimal::new(3300, 3));
        apply_open_quote_baseline_lift_after_lineage_fallbacks(&mut nodes, &chain, &m);
        assert_eq!(nodes[0].baseline_value_usd, Decimal::new(3300, 3));
        assert!(
            nodes[0]
                .note
                .as_deref()
                .unwrap_or("")
                .contains("post_fallbacks")
        );
    }

    #[test]
    fn open_quote_baseline_lift_post_fallbacks_repairs_open_node_rotation_baseline() {
        let mut n = mk_node(
            "GJaKtidWc1eu59BJmhmHXDX4YxDjc9UH8omedNNizjzv",
            Decimal::new(3301, 3),
            Decimal::new(8700, 3),
        );
        n.closed_ts_utc = None;
        let mut nodes = vec![n];
        let chain = vec!["GJaKtidWc1eu59BJmhmHXDX4YxDjc9UH8omedNNizjzv".to_string()];
        let mut m = HashMap::new();
        m.insert(
            "GJaKtidWc1eu59BJmhmHXDX4YxDjc9UH8omedNNizjzv".to_string(),
            Decimal::new(8700, 3),
        );
        apply_open_quote_baseline_lift_after_lineage_fallbacks(&mut nodes, &chain, &m);
        assert_eq!(nodes[0].baseline_value_usd, Decimal::new(8700, 3));
    }

    fn mk_node(addr: &str, baseline: Decimal, current: Decimal) -> PositionStreamLineageNode {
        PositionStreamLineageNode {
            position_address: addr.to_string(),
            token_a_label: None,
            token_b_label: None,
            token_mint_a: None,
            token_mint_b: None,
            opened_ts_utc: None,
            closed_ts_utc: None,
            baseline_value_usd: baseline,
            baseline_valuation_quality: None,
            current_value_usd: current,
            current_valuation_quality: None,
            tx_fee_lamports: 0,
            tx_fees_usd: Decimal::ZERO,
            fees_collected_usd: Decimal::ZERO,
            fees_collected_token_a_ui: None,
            fees_collected_token_b_ui: None,
            fees_collected_token_a_raw: None,
            fees_collected_token_b_raw: None,
            collect_events: 0,
            realized_cashflow_usd: Decimal::ZERO,
            net_pnl_usd: Decimal::ZERO,
            net_pnl_pct: Decimal::ZERO,
            note: None,
            collect_zero_diagnostics: None,
            chain_history_start_value_usd: None,
            chain_history_end_value_usd: None,
            chain_history_current_value_usd: None,
            chain_history_pool_address: None,
            chain_history_tick_lower_open: None,
            chain_history_tick_upper_open: None,
            chain_history_event_spot_token_a_usd_open: None,
            chain_history_event_spot_token_a_usd_close: None,
        }
    }

    #[derive(Debug, Serialize)]
    struct LineageShadowNode {
        position_address: String,
        baseline_value_usd: String,
        current_value_usd: String,
        note: Option<String>,
    }

    fn to_shadow(nodes: &[PositionStreamLineageNode]) -> Vec<LineageShadowNode> {
        nodes
            .iter()
            .map(|n| LineageShadowNode {
                position_address: n.position_address.clone(),
                baseline_value_usd: n.baseline_value_usd.round_dp(6).to_string(),
                current_value_usd: n.current_value_usd.round_dp(6).to_string(),
                note: n.note.clone(),
            })
            .collect()
    }

    fn empty_lifecycle_row() -> LifecycleRow {
        LifecycleRow {
            ts_utc: None,
            event: None,
            pool_address: None,
            position_pubkey: None,
            fee_payer_pubkey: None,
            rebalance_session_id: None,
            tx_fee_lamports: None,
            fee_payer_token_deltas: None,
            fee_payer_token_a_delta_ui: None,
            fee_payer_token_b_delta_ui: None,
            lp_collected_token_a_raw: None,
            lp_collected_token_b_raw: None,
            details: None,
            source: None,
        }
    }

    #[tokio::test]
    async fn lp_fees_close_row_uses_close_subtraction_when_authoritative_is_zero_pair() {
        // This reproduces the "principal leaked into fees" shape when lifecycle has:
        // - `bot_close_position` with positive `fee_payer_token_deltas` (principal+fees)
        // - `lp_collected_token_*_raw = Some(0)` (stale snapshot)
        // In that case we must NOT treat the close row as authoritative; we must subtract principal
        // using `details.close_amount_*_raw` to isolate fee legs.
        //
        // Numbers chosen so both mints use the fallback 9 decimals (no on-chain mint fetch in unit test).
        let mut r = empty_lifecycle_row();
        r.event = Some("bot_close_position".to_string());
        r.pool_address = Some("poolP".to_string());
        r.position_pubkey = Some("posX".to_string());
        r.details = Some(serde_json::json!({
            "close_amount_a_raw": 100_000_000u64,   // 0.1 token A principal (9 decimals)
            "close_amount_b_raw": 5_000_000_000u64, // 5 token B principal (9 decimals)
        }));
        // Seed pool mints cache so `pool_token_mints_cached` does not hit RPC.
        {
            let cache = POOL_TOKEN_MINTS_CACHE.get_or_init(|| RwLock::new(HashMap::new()));
            let mut g = cache.write().expect("pool mints cache write");
            g.insert(
                "poolP".to_string(),
                (
                    "So11111111111111111111111111111111111111112".to_string(),
                    "G15Lm1pZXNtRc5gJWXRxRXtzc8JKqbaW9nJcw3Ns5UBD".to_string(),
                ),
            );
        }
        // Net deltas to fee payer (principal+fees):
        // +0.100027916 tokenA and +5.002338 tokenB => fees = 0.000027916 tokenA and 0.002338 tokenB
        r.fee_payer_token_deltas = Some(serde_json::json!({
            "So11111111111111111111111111111111111111112": "0.100027916",
            "G15Lm1pZXNtRc5gJWXRxRXtzc8JKqbaW9nJcw3Ns5UBD": "5.002338"
        }));
        // Legacy stale snapshot (0/0) that must not suppress close-subtraction.
        r.lp_collected_token_a_raw = Some(0);
        r.lp_collected_token_b_raw = Some(0);

        let (events, _usd, by_mint) =
            lp_fees_collected_usd_from_lifecycle_rows(&test_state_no_db(), &[r], "posX").await;
        assert_eq!(events, 1);
        let sol = by_mint
            .get("So11111111111111111111111111111111111111112")
            .copied()
            .unwrap_or(Decimal::ZERO);
        let usdc = by_mint
            .get("G15Lm1pZXNtRc5gJWXRxRXtzc8JKqbaW9nJcw3Ns5UBD")
            .copied()
            .unwrap_or(Decimal::ZERO);
        assert_eq!(sol.round_dp(9).to_string(), "0.000027916");
        assert_eq!(usdc.round_dp(6).to_string(), "0.002338");
    }

    #[test]
    fn db_edges_fork_walks_entry_branch_not_sibling() {
        let t0 = Utc::now();
        let t1 = t0 + chrono::Duration::seconds(1);
        let edges = vec![
            (Some(t0), "A".to_string(), "B".to_string(), String::new()),
            (Some(t1), "A".to_string(), "C".to_string(), String::new()),
        ];
        let positions = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        assert_eq!(
            build_lineage_chain_from_db_edges(&positions, &edges, "C", 10),
            vec!["A".to_string(), "C".to_string()]
        );
        assert_eq!(
            build_lineage_chain_from_db_edges(&positions, &edges, "B", 10),
            vec!["A".to_string(), "B".to_string()]
        );
    }

    #[test]
    fn db_edges_linear_includes_ancestors_and_descendants() {
        let t0 = Utc::now();
        let t1 = t0 + chrono::Duration::seconds(1);
        let edges = vec![
            (Some(t0), "A".to_string(), "B".to_string(), String::new()),
            (Some(t1), "B".to_string(), "C".to_string(), String::new()),
        ];
        let positions = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        assert_eq!(
            build_lineage_chain_from_db_edges(&positions, &edges, "B", 10),
            vec!["A".to_string(), "B".to_string(), "C".to_string()]
        );
        assert_eq!(
            build_lineage_chain_from_db_edges(&positions, &edges, "A", 10),
            vec!["A".to_string(), "B".to_string(), "C".to_string()]
        );
    }

    /// When `positions` is entry-only (lineage DB path after `suppress_jsonl_rotation_stitch`),
    /// rotation edges whose other endpoint is outside the member set must not extend the chain.
    #[test]
    fn db_edges_entry_only_positions_ignore_external_rotation_neighbor() {
        let t0 = Utc::now();
        let edges = vec![(
            Some(t0),
            "OLD".to_string(),
            "ENTRY".to_string(),
            String::new(),
        )];
        let positions = vec!["ENTRY".to_string()];
        assert_eq!(
            build_lineage_chain_from_db_edges(&positions, &edges, "ENTRY", 10),
            vec!["ENTRY".to_string()]
        );
    }

    #[test]
    fn jsonl_stitch_suppressed_when_open_session_not_on_prior_close() {
        let t0 = DateTime::parse_from_rfc3339("2026-04-13T20:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let t1 = t0 + chrono::Duration::minutes(5);
        let rows = vec![
            LifecycleRow {
                ts_utc: Some(t0),
                event: Some("bot_close_position".to_string()),
                pool_address: Some("poolP".to_string()),
                position_pubkey: Some("posA".to_string()),
                fee_payer_pubkey: Some("payerX".to_string()),
                rebalance_session_id: Some("strategy-sid".to_string()),
                ..empty_lifecycle_row()
            },
            LifecycleRow {
                ts_utc: Some(t1),
                event: Some("bot_open_position".to_string()),
                pool_address: Some("poolP".to_string()),
                position_pubkey: Some("posB".to_string()),
                fee_payer_pubkey: Some("payerX".to_string()),
                rebalance_session_id: Some("ui-cost-session-new".to_string()),
                ..empty_lifecycle_row()
            },
        ];
        assert!(
            !lifecycle_open_has_prior_close_same_session(&rows, "posB"),
            "fresh UI session must not anchor to old close"
        );
        assert!(suppress_jsonl_rotation_stitch(&rows, "posB"));
    }

    /// `close_kind=rotation` on an older close must not imply a parent for a later `bot_open_*`
    /// with a different `rebalance_session_id` (typical API open with `cost_session_id`).
    #[test]
    fn rotation_parent_ignores_ambient_close_kind_rotation_without_session_or_bot_tie() {
        let t0 = DateTime::parse_from_rfc3339("2026-04-13T20:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let t1 = t0 + chrono::Duration::minutes(10);
        let rows = vec![
            LifecycleRow {
                ts_utc: Some(t0),
                event: Some("bot_close_position".to_string()),
                pool_address: Some("poolP".to_string()),
                position_pubkey: Some("posA".to_string()),
                fee_payer_pubkey: Some("payerX".to_string()),
                rebalance_session_id: Some("old-rot-sid".to_string()),
                details: Some(serde_json::json!({"close_kind": "rotation"})),
                ..empty_lifecycle_row()
            },
            LifecycleRow {
                ts_utc: Some(t1),
                event: Some("bot_open_position".to_string()),
                pool_address: Some("poolP".to_string()),
                position_pubkey: Some("posB".to_string()),
                fee_payer_pubkey: Some("payerX".to_string()),
                rebalance_session_id: Some("fresh-api-session".to_string()),
                ..empty_lifecycle_row()
            },
        ];
        let open = rows.last().expect("open row");
        assert!(
            lifecycle_rotation_parent_before_open(&rows, open).is_none(),
            "ambient rotation close must not become parent without session match or bot-tied evidence"
        );
        assert!(suppress_jsonl_rotation_stitch(&rows, "posB"));
    }

    #[test]
    fn operator_api_open_marks_bot_open_row_but_suppresses_lineage_stitch() {
        let t0 = DateTime::parse_from_rfc3339("2026-04-13T21:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let rows = vec![LifecycleRow {
            ts_utc: Some(t0),
            event: Some("bot_open_position".to_string()),
            pool_address: Some("poolP".to_string()),
            position_pubkey: Some("posNew".to_string()),
            fee_payer_pubkey: Some("payerX".to_string()),
            rebalance_session_id: Some("cost-session-ui".to_string()),
            details: Some(serde_json::json!({"open_origin": "operator_api"})),
            source: Some("orca_bot".to_string()),
            ..empty_lifecycle_row()
        }];
        assert!(suppress_jsonl_rotation_stitch(&rows, "posNew"));
    }

    #[test]
    fn lifecycle_chain_links_session_matched_close_open_even_after_long_delay() {
        let t0 = DateTime::parse_from_rfc3339("2026-04-15T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let t1 = t0 + chrono::Duration::minutes(210); // > 60m
        let sid = "sid-long-delay".to_string();
        let rows = vec![
            LifecycleRow {
                ts_utc: Some(t0),
                event: Some("bot_close_position".to_string()),
                pool_address: Some("poolP".to_string()),
                position_pubkey: Some("posOld".to_string()),
                fee_payer_pubkey: Some("payerX".to_string()),
                rebalance_session_id: Some(sid.clone()),
                ..empty_lifecycle_row()
            },
            LifecycleRow {
                ts_utc: Some(t1),
                event: Some("bot_open_position".to_string()),
                pool_address: Some("poolP".to_string()),
                position_pubkey: Some("posNew".to_string()),
                fee_payer_pubkey: Some("payerX".to_string()),
                rebalance_session_id: Some(sid),
                ..empty_lifecycle_row()
            },
        ];
        let chain = chain_from_lifecycle_best_effort_rows(&rows, "posNew", 25);
        assert_eq!(chain, vec!["posOld".to_string(), "posNew".to_string()]);
        assert!(lifecycle_open_has_prior_close_same_session(&rows, "posNew"));
        assert_eq!(
            lifecycle_rotation_parent_before_open(&rows, rows.last().unwrap()),
            Some("posOld")
        );
    }

    #[test]
    fn baseline_open_prefers_open_amount_raw_over_fee_payer_deltas() {
        let mint_a = "MINTA";
        let mint_b = "MINTB";

        let details = serde_json::json!({
            "open_amount_a_raw": 1_000_000u64, // 1.0 with 6 decimals
            "open_amount_b_raw": 2_000_000u64, // 2.0 with 6 decimals
        });
        let deltas = serde_json::json!({
            mint_a: "-999.0",
            mint_b: "-999.0",
        });

        let (a_ui, b_ui, src) = baseline_open_amounts_ui_from_details_or_deltas(
            details.as_object(),
            deltas.as_object(),
            mint_a,
            mint_b,
            Some(6),
            Some(6),
        );

        assert_eq!(a_ui, Decimal::from_str("1").unwrap());
        assert_eq!(b_ui, Decimal::from_str("2").unwrap());
        assert_eq!(src, Some("open_amount_raw"));
    }

    #[test]
    fn baseline_open_uses_open_quote_caps_when_present() {
        let mint_a = "MINTA";
        let mint_b = "MINTB";

        let details = serde_json::json!({
            "open_quote_token_max_a": 1_000_000u64, // 1.0 with 6 decimals
            "open_quote_token_max_b": 2_000_000u64, // 2.0 with 6 decimals
        });
        let deltas = serde_json::json!({
            mint_a: "-999.0",
            mint_b: "-999.0",
        });

        let (a_ui, b_ui, src) = baseline_open_amounts_ui_from_details_or_deltas(
            details.as_object(),
            deltas.as_object(),
            mint_a,
            mint_b,
            Some(6),
            Some(6),
        );

        assert_eq!(a_ui, Decimal::from_str("1").unwrap());
        assert_eq!(b_ui, Decimal::from_str("2").unwrap());
        assert_eq!(src, Some("open_quote_caps"));
    }

    #[test]
    fn baseline_open_falls_back_to_deltas_when_decimals_missing() {
        let mint_a = "MINTA";
        let mint_b = "MINTB";

        let details = serde_json::json!({
            "open_amount_a_raw": 1_000_000u64,
            "open_amount_b_raw": 2_000_000u64,
        });
        let deltas = serde_json::json!({
            mint_a: "-0.5",
            mint_b: "-1.25",
        });

        let (a_ui, b_ui, src) = baseline_open_amounts_ui_from_details_or_deltas(
            details.as_object(),
            deltas.as_object(),
            mint_a,
            mint_b,
            None,
            None,
        );

        assert_eq!(a_ui, Decimal::from_str("0.5").unwrap());
        assert_eq!(b_ui, Decimal::from_str("1.25").unwrap());
        assert_eq!(src, None);
    }

    #[test]
    fn operator_manual_close_stops_forward_lifecycle_chain() {
        let t0 = DateTime::parse_from_rfc3339("2026-04-13T22:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let t1 = t0 + chrono::Duration::minutes(1);
        let t2 = t1 + chrono::Duration::minutes(15);
        let rows = vec![
            LifecycleRow {
                ts_utc: Some(t0),
                event: Some("bot_open_position".to_string()),
                pool_address: Some("poolP".to_string()),
                position_pubkey: Some("posA".to_string()),
                fee_payer_pubkey: Some("payerX".to_string()),
                rebalance_session_id: Some("s0".to_string()),
                ..empty_lifecycle_row()
            },
            LifecycleRow {
                ts_utc: Some(t1),
                event: Some("bot_close_position".to_string()),
                pool_address: Some("poolP".to_string()),
                position_pubkey: Some("posA".to_string()),
                fee_payer_pubkey: Some("payerX".to_string()),
                rebalance_session_id: Some("s0".to_string()),
                details: Some(serde_json::json!({"close_kind": "manual", "close_source": "api"})),
                ..empty_lifecycle_row()
            },
            LifecycleRow {
                ts_utc: Some(t2),
                event: Some("bot_open_position".to_string()),
                pool_address: Some("poolP".to_string()),
                position_pubkey: Some("posB".to_string()),
                fee_payer_pubkey: Some("payerX".to_string()),
                rebalance_session_id: Some("s-bot-later".to_string()),
                ..empty_lifecycle_row()
            },
        ];
        assert_eq!(
            chain_from_lifecycle_best_effort_rows(&rows, "posA", 25),
            vec!["posA".to_string()]
        );
    }

    #[test]
    fn jsonl_stitch_allowed_when_session_matches_prior_close() {
        let t0 = DateTime::parse_from_rfc3339("2026-04-13T20:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let t1 = t0 + chrono::Duration::minutes(1);
        let sid = Some("one-rotation".to_string());
        let rows = vec![
            LifecycleRow {
                ts_utc: Some(t0),
                event: Some("bot_close_position".to_string()),
                pool_address: Some("poolP".to_string()),
                position_pubkey: Some("posA".to_string()),
                fee_payer_pubkey: Some("payerX".to_string()),
                rebalance_session_id: sid.clone(),
                ..empty_lifecycle_row()
            },
            LifecycleRow {
                ts_utc: Some(t1),
                event: Some("bot_open_position".to_string()),
                pool_address: Some("poolP".to_string()),
                position_pubkey: Some("posB".to_string()),
                fee_payer_pubkey: Some("payerX".to_string()),
                rebalance_session_id: sid,
                ..empty_lifecycle_row()
            },
        ];
        assert!(lifecycle_open_has_prior_close_same_session(&rows, "posB"));
        assert!(!suppress_jsonl_rotation_stitch(&rows, "posB"));
    }

    #[test]
    fn jsonl_stitch_allowed_when_rotation_parent_exists_without_session_match() {
        let t0 = DateTime::parse_from_rfc3339("2026-04-13T20:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let t_mid = t0 + chrono::Duration::seconds(30);
        let t1 = t0 + chrono::Duration::minutes(1);
        let rows = vec![
            LifecycleRow {
                ts_utc: Some(t0),
                event: Some("bot_close_position".to_string()),
                pool_address: Some("poolP".to_string()),
                position_pubkey: Some("posA".to_string()),
                fee_payer_pubkey: Some("payerX".to_string()),
                details: Some(serde_json::json!({ "close_kind": "rotation" })),
                ..empty_lifecycle_row()
            },
            // Without matching `rebalance_session_id`, parent inference now requires bot-tied evidence
            // on the closed PDA between close and open (not `close_kind` alone).
            LifecycleRow {
                ts_utc: Some(t_mid),
                event: Some("bot_decrease_liquidity".to_string()),
                pool_address: Some("poolP".to_string()),
                position_pubkey: Some("posA".to_string()),
                fee_payer_pubkey: Some("payerX".to_string()),
                ..empty_lifecycle_row()
            },
            LifecycleRow {
                ts_utc: Some(t1),
                event: Some("bot_open_position".to_string()),
                pool_address: Some("poolP".to_string()),
                position_pubkey: Some("posB".to_string()),
                fee_payer_pubkey: Some("payerX".to_string()),
                rebalance_session_id: Some("fresh-open-session".to_string()),
                ..empty_lifecycle_row()
            },
        ];
        assert!(
            !lifecycle_open_has_prior_close_same_session(&rows, "posB"),
            "session may differ, but rotation parent still exists"
        );
        assert!(
            !suppress_jsonl_rotation_stitch(&rows, "posB"),
            "bot open with inferred rotation parent should stay stitchable"
        );
        assert_eq!(
            chain_from_lifecycle_best_effort_rows(&rows, "posB", 25),
            vec!["posA".to_string(), "posB".to_string()]
        );
    }

    #[test]
    fn lifecycle_chain_skips_unrelated_close_without_rotation_signal() {
        let t0 = DateTime::parse_from_rfc3339("2026-04-13T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let t1 = DateTime::parse_from_rfc3339("2026-04-13T10:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let rows = vec![
            LifecycleRow {
                ts_utc: Some(t0),
                event: Some("bot_close_position".to_string()),
                pool_address: Some("poolP".to_string()),
                position_pubkey: Some("posA".to_string()),
                fee_payer_pubkey: Some("payerX".to_string()),
                ..empty_lifecycle_row()
            },
            LifecycleRow {
                ts_utc: Some(t1),
                event: Some("position_open".to_string()),
                pool_address: Some("poolP".to_string()),
                position_pubkey: Some("posB".to_string()),
                fee_payer_pubkey: Some("payerX".to_string()),
                ..empty_lifecycle_row()
            },
        ];
        let chain = chain_from_lifecycle_best_effort_rows(&rows, "posB", 25);
        assert_eq!(chain, vec!["posB".to_string()]);
    }

    #[test]
    fn lifecycle_chain_links_session_matched_close_open() {
        let t0 = DateTime::parse_from_rfc3339("2026-04-13T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let t1 = DateTime::parse_from_rfc3339("2026-04-13T10:05:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let sid = Some("sess-rot-1".to_string());
        let rows = vec![
            LifecycleRow {
                ts_utc: Some(t0),
                event: Some("bot_close_position".to_string()),
                pool_address: Some("poolP".to_string()),
                position_pubkey: Some("posA".to_string()),
                fee_payer_pubkey: Some("payerX".to_string()),
                rebalance_session_id: sid.clone(),
                ..empty_lifecycle_row()
            },
            LifecycleRow {
                ts_utc: Some(t1),
                event: Some("bot_open_position".to_string()),
                pool_address: Some("poolP".to_string()),
                position_pubkey: Some("posB".to_string()),
                fee_payer_pubkey: Some("payerX".to_string()),
                rebalance_session_id: sid,
                ..empty_lifecycle_row()
            },
        ];
        assert_eq!(
            chain_from_lifecycle_best_effort_rows(&rows, "posB", 25),
            vec!["posA".to_string(), "posB".to_string()]
        );
        assert_eq!(
            chain_from_lifecycle_best_effort_rows(&rows, "posA", 25),
            vec!["posA".to_string(), "posB".to_string()]
        );
    }

    #[test]
    fn continuity_from_session_carries_prev_end_to_next_baseline() {
        let ts = chrono::Utc::now();
        let rows = vec![
            LifecycleRow {
                ts_utc: Some(ts),
                event: Some("bot_close_position".to_string()),
                pool_address: Some("pool".to_string()),
                position_pubkey: Some("old".to_string()),
                fee_payer_pubkey: Some("payer".to_string()),
                rebalance_session_id: Some("sid-1".to_string()),
                tx_fee_lamports: None,
                fee_payer_token_deltas: None,
                fee_payer_token_a_delta_ui: None,
                fee_payer_token_b_delta_ui: None,
                lp_collected_token_a_raw: None,
                lp_collected_token_b_raw: None,
                details: None,
                source: None,
            },
            LifecycleRow {
                ts_utc: Some(ts),
                event: Some("bot_open_position".to_string()),
                pool_address: Some("pool".to_string()),
                position_pubkey: Some("new".to_string()),
                fee_payer_pubkey: Some("payer".to_string()),
                rebalance_session_id: Some("sid-1".to_string()),
                tx_fee_lamports: None,
                fee_payer_token_deltas: None,
                fee_payer_token_a_delta_ui: None,
                fee_payer_token_b_delta_ui: None,
                lp_collected_token_a_raw: None,
                lp_collected_token_b_raw: None,
                details: None,
                source: None,
            },
        ];

        let mut nodes = vec![
            mk_node("old", Decimal::new(200, 2), Decimal::new(180, 2)),
            mk_node("new", Decimal::ZERO, Decimal::new(175, 2)),
        ];
        apply_session_continuity_from_lifecycle_rows(&rows, &mut nodes);
        assert_eq!(nodes[1].baseline_value_usd, Decimal::new(180, 2));
    }

    #[test]
    fn continuity_from_session_does_not_override_existing_baseline() {
        let ts = chrono::Utc::now();
        let rows = vec![
            LifecycleRow {
                ts_utc: Some(ts),
                event: Some("bot_close_position".to_string()),
                pool_address: Some("pool".to_string()),
                position_pubkey: Some("old".to_string()),
                fee_payer_pubkey: Some("payer".to_string()),
                rebalance_session_id: Some("sid-1".to_string()),
                tx_fee_lamports: None,
                fee_payer_token_deltas: None,
                fee_payer_token_a_delta_ui: None,
                fee_payer_token_b_delta_ui: None,
                lp_collected_token_a_raw: None,
                lp_collected_token_b_raw: None,
                details: None,
                source: None,
            },
            LifecycleRow {
                ts_utc: Some(ts),
                event: Some("bot_open_position".to_string()),
                pool_address: Some("pool".to_string()),
                position_pubkey: Some("new".to_string()),
                fee_payer_pubkey: Some("payer".to_string()),
                rebalance_session_id: Some("sid-1".to_string()),
                tx_fee_lamports: None,
                fee_payer_token_deltas: None,
                fee_payer_token_a_delta_ui: None,
                fee_payer_token_b_delta_ui: None,
                lp_collected_token_a_raw: None,
                lp_collected_token_b_raw: None,
                details: None,
                source: None,
            },
        ];

        let mut nodes = vec![
            mk_node("old", Decimal::new(200, 2), Decimal::new(180, 2)),
            // Simulates explicit baseline derived from open row (even if tiny/dust).
            mk_node("new", Decimal::new(1, 6), Decimal::new(175, 2)),
        ];
        apply_session_continuity_from_lifecycle_rows(&rows, &mut nodes);
        assert_eq!(nodes[1].baseline_value_usd, Decimal::new(1, 6));
        assert!(
            nodes[1]
                .note
                .as_deref()
                .is_none_or(|n| !n.contains("baseline_from_rotation_session"))
        );
    }

    #[test]
    fn baseline_fallback_guardrail_blocks_implausible_prev_end() {
        let mut nodes = vec![
            mk_node("old", Decimal::new(100, 2), Decimal::new(900, 2)),
            mk_node("new", Decimal::ZERO, Decimal::new(300, 2)),
        ];
        apply_baseline_fallback_from_prev_end(&mut nodes);
        assert!(nodes[1].baseline_value_usd.is_zero());
    }

    #[test]
    fn merge_live_prices_uses_recent_cache_for_missing_mint() {
        let now = Utc::now();
        let requested = BTreeSet::from(["mint-a".to_string(), "mint-b".to_string()]);
        let live = BTreeMap::from([("mint-a".to_string(), 12.5_f64)]);
        let cache = HashMap::from([(
            "mint-b".to_string(),
            CachedMintPrice {
                usd: 8.25,
                updated_at: now,
            },
        )]);
        let (merged, used_cache) = merge_live_with_cached_prices(&requested, &live, &cache, now);
        assert!(used_cache);
        assert_eq!(merged.get("mint-a").copied(), Some(12.5));
        assert_eq!(merged.get("mint-b").copied(), Some(8.25));
    }

    #[test]
    fn lineage_shadow_diff_matches_golden_fixture() {
        let ts = chrono::Utc::now();
        let rows = vec![
            LifecycleRow {
                ts_utc: Some(ts),
                event: Some("bot_close_position".to_string()),
                pool_address: Some("pool".to_string()),
                position_pubkey: Some("old".to_string()),
                fee_payer_pubkey: Some("payer".to_string()),
                rebalance_session_id: Some("sid-1".to_string()),
                tx_fee_lamports: None,
                fee_payer_token_deltas: None,
                fee_payer_token_a_delta_ui: None,
                fee_payer_token_b_delta_ui: None,
                lp_collected_token_a_raw: None,
                lp_collected_token_b_raw: None,
                details: None,
                source: None,
            },
            LifecycleRow {
                ts_utc: Some(ts),
                event: Some("bot_open_position".to_string()),
                pool_address: Some("pool".to_string()),
                position_pubkey: Some("new".to_string()),
                fee_payer_pubkey: Some("payer".to_string()),
                rebalance_session_id: Some("sid-1".to_string()),
                tx_fee_lamports: None,
                fee_payer_token_deltas: None,
                fee_payer_token_a_delta_ui: None,
                fee_payer_token_b_delta_ui: None,
                lp_collected_token_a_raw: None,
                lp_collected_token_b_raw: None,
                details: None,
                source: None,
            },
        ];
        let mut nodes = vec![
            mk_node("old", Decimal::new(200, 2), Decimal::new(180, 2)),
            mk_node("new", Decimal::ZERO, Decimal::new(175, 2)),
            mk_node("next", Decimal::ZERO, Decimal::new(160, 2)),
        ];
        nodes[0].closed_ts_utc = Some("2026-04-10T00:00:00Z".to_string());
        apply_session_continuity_from_lifecycle_rows(&rows, &mut nodes);
        apply_baseline_fallback_from_prev_end(&mut nodes);
        apply_end_value_fallback_from_next_baseline(&mut nodes);

        let got = serde_json::to_string_pretty(&to_shadow(&nodes)).expect("serialize shadow");
        let expected = include_str!("../../tests/fixtures/lineage_shadow_expected.json").trim();
        assert_eq!(got.trim(), expected);
    }

    #[test]
    fn event_spot_from_ledger_details_parses_prices_source_and_slot() {
        let d = serde_json::json!({
            "event_price_a_usd": "100.5",
            "event_price_b_usd": 1.0,
            "event_price_source": "gecko+pool_tick_wsol",
            "event_slot": "12345"
        });
        let got = event_spot_from_ledger_details(Some(&d)).expect("parse");
        assert!((got.0 - 100.5).abs() < 1e-9);
        assert!((got.1 - 1.0).abs() < 1e-9);
        assert_eq!(got.2, "gecko+pool_tick_wsol");
        assert_eq!(got.3, Some(12345));
    }

    #[test]
    fn event_spot_from_ledger_details_requires_both_prices() {
        let d = serde_json::json!({ "event_price_a_usd": 1.0 });
        assert!(event_spot_from_ledger_details(Some(&d)).is_none());
    }

    #[test]
    fn closed_ts_for_snapshot_kind_only_marks_end_close() {
        let now = Utc::now();
        assert_eq!(
            closed_ts_for_snapshot_kind(Some("end_close"), Some(now)),
            Some(now)
        );
        assert_eq!(
            closed_ts_for_snapshot_kind(Some("baseline_open"), Some(now)),
            None
        );
        assert_eq!(
            closed_ts_for_snapshot_kind(Some("current_mark"), Some(now)),
            None
        );
        assert_eq!(closed_ts_for_snapshot_kind(None, Some(now)), None);
    }

    // NOTE: legacy open-caps heuristics tests removed intentionally.
}
