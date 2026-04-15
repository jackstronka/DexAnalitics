//! Stream-level Net PnL / IL across rotated position PDAs.
//!
//! Uses DB-backed valuation snapshots + lifecycle ledger token deltas when available.

use crate::error::ApiError;
use crate::models::PositionStreamPnLResponse;
use crate::services::position_stream_performance::compute_position_stream_performance;
use crate::services::price_fetch::fetch_mint_prices_usd;
use crate::services::position_valuation::{
    compute_position_usd_valuation, fetch_prices_for_positions, monitored_position_from_chain,
};
use crate::state::AppState;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::Value;
use sqlx::Row;
use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;
use tokio::time::{timeout, Duration};

const WSOL_MINT: &str = "So11111111111111111111111111111111111111112";

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

async fn sol_usd() -> (f64, String) {
    let mut mints: BTreeSet<String> = BTreeSet::new();
    mints.insert(WSOL_MINT.to_string());
    let (px, src) = fetch_mint_prices_usd(&mints).await;
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
        note: Some("DB is disabled (DATABASE_URL missing/failed); stream PnL/IL unavailable.".to_string()),
    }
}

/// Stream PnL using an explicit member list (e.g. entry-only when lineage suppresses cross-PDA stitching).
pub(crate) async fn compute_position_stream_pnl_for_stream_members(
    state: &AppState,
    position_address: &str,
    positions: Vec<String>,
    sessions: Vec<String>,
) -> Result<PositionStreamPnLResponse, ApiError> {
    let Some(db) = state.db.as_ref() else {
        return Ok(stream_pnl_db_disabled_response(position_address));
    };

    // Baseline = earliest valuation snapshot across the stream; current = latest.
    let mut baseline_row = sqlx::query(
        r#"
        SELECT ts_utc, value_usd, amount_a_ui, amount_b_ui, pool_pubkey
        FROM position_stream_valuation_snapshots
        WHERE position_pubkey = ANY($1)
        ORDER BY ts_utc ASC
        LIMIT 1
        "#,
    )
    .bind(&positions)
    .fetch_optional(db.pool())
    .await
    .map_err(|e| ApiError::internal(format!("stream pnl: baseline query: {e}")))?;

    let current_row = sqlx::query(
        r#"
        SELECT ts_utc, value_usd, amount_a_ui, amount_b_ui, pool_pubkey
        FROM position_stream_valuation_snapshots
        WHERE position_pubkey = ANY($1)
        ORDER BY ts_utc DESC
        LIMIT 1
        "#,
    )
    .bind(&positions)
    .fetch_optional(db.pool())
    .await
    .map_err(|e| ApiError::internal(format!("stream pnl: current query: {e}")))?;

    if baseline_row.is_none() {
        // Best-effort self-seed: compute a valuation snapshot for the entry PDA now.
        // This avoids the UI showing zeros unless the user manually visited `GET /positions/:address` first.
        let pk = solana_sdk::pubkey::Pubkey::from_str(position_address)
            .map_err(|_| ApiError::bad_request("Invalid position address"))?;
        // Closed positions often no longer exist on-chain; don't block lineage on a slow/failed RPC.
        if let Ok(Ok(pos)) = timeout(
            Duration::from_secs(2),
            monitored_position_from_chain(state.provider.clone(), &pk),
        )
        .await
        {
            let prices = fetch_prices_for_positions(state.provider.clone(), std::slice::from_ref(&pos)).await;
            if let Ok(v) = compute_position_usd_valuation(state.provider.clone(), &pos, &prices).await {
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
        baseline_row = sqlx::query(
            r#"
            SELECT ts_utc, value_usd, amount_a_ui, amount_b_ui, pool_pubkey
            FROM position_stream_valuation_snapshots
            WHERE position_pubkey = ANY($1)
            ORDER BY ts_utc ASC
            LIMIT 1
            "#,
        )
        .bind(&positions)
        .fetch_optional(db.pool())
        .await
        .map_err(|e| ApiError::internal(format!("stream pnl: baseline query (after seed): {e}")))?;
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
            note: Some("No valuation snapshots yet (even after best-effort self-seed). Check DB migrations and RPC health.".to_string()),
        });
    };

    let baseline_ts: Option<DateTime<Utc>> = b.try_get("ts_utc").ok();
    let baseline_value: Decimal = b.try_get("value_usd").unwrap_or(Decimal::ZERO);
    let baseline_a: Decimal = b.try_get("amount_a_ui").unwrap_or(Decimal::ZERO);
    let baseline_b: Decimal = b.try_get("amount_b_ui").unwrap_or(Decimal::ZERO);
    let mint_a: Option<String> = b.try_get("token_mint_a").ok();
    let mint_b: Option<String> = b.try_get("token_mint_b").ok();

    let (current_ts, current_value) = if let Some(c) = current_row {
        let ts: Option<DateTime<Utc>> = c.try_get("ts_utc").ok();
        let v: Decimal = c.try_get("value_usd").unwrap_or(Decimal::ZERO);
        (ts, v)
    } else {
        (None, Decimal::ZERO)
    };

    // Convert tx fees to USD using SOL/USD now (best-effort).
    let (sol_usd, sol_src) = sol_usd().await;
    let tx_fee_lamports: i64 = if !sessions.is_empty() {
        sqlx::query(
            r#"SELECT COALESCE(SUM(tx_fee_lamports), 0) AS fee_lamports
               FROM position_stream_ledger_rows
               WHERE rebalance_session_id = ANY($1)"#,
        )
        .bind(&sessions)
        .fetch_one(db.pool())
        .await
        .map_err(|e| ApiError::internal(format!("stream pnl: tx fee sum: {e}")))?
        .try_get("fee_lamports")
        .unwrap_or(0)
    } else {
        sqlx::query(
            r#"SELECT COALESCE(SUM(tx_fee_lamports), 0) AS fee_lamports
               FROM position_stream_ledger_rows
               WHERE position_pubkey = ANY($1)"#,
        )
        .bind(&positions)
        .fetch_one(db.pool())
        .await
        .map_err(|e| ApiError::internal(format!("stream pnl: tx fee sum: {e}")))?
        .try_get("fee_lamports")
        .unwrap_or(0)
    };
    let tx_fee_lamports_u = tx_fee_lamports.max(0) as u64;
    let tx_fees_usd = if sol_usd > 0.0 {
        Decimal::from_f64_retain((tx_fee_lamports_u as f64 / 1e9) * sol_usd).unwrap_or(Decimal::ZERO)
    } else {
        Decimal::ZERO
    };

    // Realized cashflow from lifecycle rows: sum fee_payer_token_deltas for the stream.
    // We don't yet have stable token symbols here; we treat it as USD using current mint prices.
    let rows = if !sessions.is_empty() {
        sqlx::query(
            r#"SELECT fee_payer_token_deltas
               FROM position_stream_ledger_rows
               WHERE rebalance_session_id = ANY($1) AND fee_payer_token_deltas IS NOT NULL"#,
        )
        .bind(&sessions)
        .fetch_all(db.pool())
        .await
        .map_err(|e| ApiError::internal(format!("stream pnl: token deltas rows: {e}")))?
    } else {
        sqlx::query(
            r#"SELECT fee_payer_token_deltas
               FROM position_stream_ledger_rows
               WHERE position_pubkey = ANY($1) AND fee_payer_token_deltas IS NOT NULL"#,
        )
        .bind(&positions)
        .fetch_all(db.pool())
        .await
        .map_err(|e| ApiError::internal(format!("stream pnl: token deltas rows: {e}")))?
    };

    let mut mint_deltas: BTreeMap<String, Decimal> = BTreeMap::new();
    for r in rows {
        let v: Option<Value> = r.try_get("fee_payer_token_deltas").ok();
        let Some(Value::Object(map)) = v else { continue };
        for (mint, dv) in map {
            if let Some(d) = decimal_from_json(&dv) {
                *mint_deltas.entry(mint).or_insert(Decimal::ZERO) += d;
            }
        }
    }

    // Use mints persisted on the baseline valuation snapshot for stable HODL/IL and cashflow conversion.
    let mut pool_mints: Vec<String> = Vec::new();
    if let Some(a) = mint_a.clone().filter(|s| !s.trim().is_empty()) {
        pool_mints.push(a);
    }
    if let Some(b) = mint_b.clone().filter(|s| !s.trim().is_empty()) {
        pool_mints.push(b);
    }
    let mint_set: BTreeSet<String> = pool_mints.iter().cloned().collect();
    let (px, price_src) = fetch_mint_prices_usd(&mint_set).await;
    let pa = pool_mints
        .first()
        .and_then(|m| px.get(m))
        .copied()
        .unwrap_or(0.0);
    let pb = pool_mints.get(1).and_then(|m| px.get(m)).copied().unwrap_or(0.0);
    let pa_d = Decimal::from_f64_retain(pa).unwrap_or(Decimal::ZERO);
    let pb_d = Decimal::from_f64_retain(pb).unwrap_or(Decimal::ZERO);

    let realized_cashflow_usd = if pool_mints.len() == 2 {
        let da = mint_deltas.get(&pool_mints[0]).cloned().unwrap_or(Decimal::ZERO);
        let dbb = mint_deltas.get(&pool_mints[1]).cloned().unwrap_or(Decimal::ZERO);
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
        note: Some(format!(
            "Best-effort. tx fees in USD use SOL/USD ({sol_src}). realized_cashflow uses lifecycle fee_payer_token_deltas × mint USD prices ({price_src})."
        )),
    })
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
    compute_position_stream_pnl_for_stream_members(
        state,
        position_address,
        perf.positions,
        perf.sessions,
    )
    .await
}

