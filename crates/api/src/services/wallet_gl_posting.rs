//! GL posting for `SESSION:{session_id}` accounts (analytics sub-ledger).
//!
//! Policy: session accounts are **never closed or liquidated** after position close (including
//! manual close) — balances accumulate for operator analytics.
//!
//! **Primary source (PR-A):** `position_stream_ledger_rows` / lifecycle JSONL on ingest.
//! **Secondary:** wallet journal `deltas[]` when present.

use crate::models::{
    WalletLedgerEvent, WalletLedgerStatus, WalletSessionBalanceRow, WalletSessionBalanceUsdLeg,
    WalletSessionGlBackfillReport, WalletSessionGlReconcileGap, WalletSessionGlReconcileResponse,
    WalletSessionMetrics, WalletSessionOpenStartSnapshot, WalletChainGlBackfillReport,
    WalletGlBalancesResponse, WalletGlOpeningImportReport, WalletGlRpcReconcileGap,
    WalletGlRpcReconcileResponse, WalletBalanceConfidence, WalletEffectiveBalancesResponse,
    WalletTokenBalance,
};
use clmm_lp_data::repositories::Database;
use clmm_lp_data::wallet_session::{
    self, apply_session_mint_postings, apply_session_postings_from_lifecycle_row,
    format_raw_i128, parse_raw_i128, session_lifecycle_posting_already_applied,
    SessionBalanceMint, SessionLifecyclePostingOutcome,
};
use serde_json::Value;
use sqlx::Row;
use std::collections::BTreeMap;

pub use clmm_lp_data::wallet_session::{
    lifecycle_posting_event_id, session_account_code, session_mint_deltas_from_lifecycle_json,
    wallet_account_code, wallet_journal_posting_event_id, wallet_opening_import_event_id,
    TX_FEE_ACCOUNT_CODE,
};

pub type LifecyclePostingOutcome = SessionLifecyclePostingOutcome;

/// Phase D1: resolved scope read (SESSION / CHAIN) with explicit quality flags.
#[derive(Debug, Clone)]
pub struct GlScopeReadResolved {
    pub balances: Vec<WalletSessionBalanceRow>,
    pub source: String,
    pub quality: String,
    pub gl_matches_pslr: bool,
    pub needs_reconcile: bool,
}

#[must_use]
pub fn gl_read_quality_from_source(source: &str) -> &'static str {
    match source {
        "gl_session_shadow" | "gl_chain_shadow" => "exact",
        s if s.contains("_pslr_fallback") => "pslr_fallback",
        s if s.contains("_pslr_corrected") => "pslr_corrected",
        s if s.ends_with("_empty") => "empty",
        s if s.ends_with("_disabled") => "disabled",
        s if s.ends_with("_no_db") => "no_db",
        _ => "unknown",
    }
}

#[must_use]
pub fn gl_needs_reconcile_from_read(
    source: &str,
    gl_matches_pslr: bool,
    metrics_trusted: Option<bool>,
) -> bool {
    if matches!(
        gl_read_quality_from_source(source),
        "disabled" | "no_db"
    ) {
        return false;
    }
    if metrics_trusted == Some(false) {
        return true;
    }
    if !gl_matches_pslr {
        return true;
    }
    matches!(
        gl_read_quality_from_source(source),
        "pslr_fallback" | "pslr_corrected" | "empty"
    )
}

/// Whether to apply SESSION postings when journal rows are persisted.
pub fn session_posting_enabled() -> bool {
    match std::env::var("CLMM_WALLET_GL_SESSION_POSTING") {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        ),
        Err(_) => true,
    }
}

/// Posting from lifecycle rows on `ingest_lifecycle_rows` (primary path for close/collect/swap).
pub fn lifecycle_posting_enabled() -> bool {
    match std::env::var("CLMM_WALLET_GL_SESSION_LIFECYCLE_POSTING") {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        ),
        Err(_) => true,
    }
}

pub fn session_read_enabled() -> bool {
    match std::env::var("CLMM_WALLET_GL_SESSION_READ") {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        ),
        Err(_) => true,
    }
}

/// Resolve session id for GL posting (cost_session_id / rebalance_session_id alias).
pub fn session_id_from_ledger_event(ev: &WalletLedgerEvent) -> Option<String> {
    if ev.status != WalletLedgerStatus::Confirmed {
        return None;
    }
    if ev.dry_run {
        return None;
    }
    let sid = ev.cost_session_id.as_deref()?.trim();
    if sid.is_empty() {
        return None;
    }
    Some(sid.to_string())
}

/// Build SESSION postings from a confirmed journal row (mint → signed raw delta).
pub fn session_postings_from_event(ev: &WalletLedgerEvent) -> Option<Vec<(String, i128)>> {
    let _session_id = session_id_from_ledger_event(ev)?;
    if ev.deltas.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    for d in &ev.deltas {
        let mint = d.mint.trim();
        if mint.is_empty() {
            continue;
        }
        let Some(delta) = parse_raw_i128(&d.raw_delta_i128) else {
            tracing::warn!(
                event_id = %ev.event_id,
                mint = %mint,
                raw = %d.raw_delta_i128,
                "wallet_gl_posting: skip unparseable delta"
            );
            continue;
        };
        if delta == 0 {
            continue;
        }
        out.push((mint.to_string(), delta));
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

async fn apply_postings_to_session_best_effort(
    db: &Database,
    session_id: &str,
    owner: Option<&str>,
    event_id: &str,
    kind: &str,
    postings: &[(String, i128)],
) {
    if postings.is_empty() {
        return;
    }
    if let Err(e) = apply_session_mint_postings(
        db, session_id, owner, event_id, kind, postings,
    )
    .await
    {
        tracing::warn!(
            error = %e,
            session_id = %session_id,
            event_id = %event_id,
            "wallet_gl_posting: apply_session_mint_postings failed"
        );
    }
}

/// Apply SESSION postings from one lifecycle JSONL row (ingest hook).
pub async fn apply_session_postings_from_lifecycle_json(
    db: &Database,
    v: &Value,
    lp_collected_a_raw: Option<i64>,
    lp_collected_b_raw: Option<i64>,
) {
    if !lifecycle_posting_enabled() {
        return;
    }
    match apply_session_postings_from_lifecycle_row(db, v, lp_collected_a_raw, lp_collected_b_raw).await
    {
        Ok(_) => {}
        Err(e) => tracing::warn!(error = %e, "wallet_gl_posting: lifecycle row posting failed"),
    }
    apply_tx_fee_posting_from_lifecycle_json(db, v).await;
}

pub fn tx_fee_posting_enabled() -> bool {
    match std::env::var("CLMM_WALLET_GL_TX_FEE_POSTING") {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        ),
        Err(_) => true,
    }
}

/// Post accumulated network fee to system account `TX_FEE` (once per lifecycle signature).
pub async fn apply_tx_fee_posting_from_lifecycle_json(db: &Database, v: &Value) {
    if !tx_fee_posting_enabled() {
        return;
    }
    match wallet_session::apply_tx_fee_posting_from_lifecycle_row(db, v).await {
        Ok(_) => {}
        Err(e) => tracing::warn!(error = %e, "wallet_gl_posting: tx_fee lifecycle posting failed"),
    }
}

/// Replay `position_stream_ledger_rows` into SESSION GL (idempotent).
pub async fn backfill_session_postings_from_pslr(
    db: &Database,
    session_id: Option<&str>,
    max_sessions: usize,
) -> Result<WalletSessionGlBackfillReport, sqlx::Error> {
    let max_sessions = max_sessions.clamp(1, 500);
    let mut report = WalletSessionGlBackfillReport {
        sessions_processed: 0,
        rows_scanned: 0,
        postings_applied: 0,
        rows_skipped_already: 0,
        rows_skipped_no_deltas: 0,
    };

    let session_ids: Vec<String> = if let Some(sid) = session_id.map(str::trim).filter(|s| !s.is_empty())
    {
        vec![sid.to_string()]
    } else {
        let rows = sqlx::query(
            r#"
            SELECT DISTINCT rebalance_session_id AS sid
            FROM position_stream_ledger_rows
            WHERE rebalance_session_id IS NOT NULL AND TRIM(rebalance_session_id) <> ''
            ORDER BY sid
            LIMIT $1
            "#,
        )
        .bind(max_sessions as i64)
        .fetch_all(db.pool())
        .await?;
        rows.iter()
            .filter_map(|r| {
                let s: String = r.get("sid");
                let t = s.trim();
                if t.is_empty() {
                    None
                } else {
                    Some(t.to_string())
                }
            })
            .collect()
    };

    for sid in session_ids {
        report.sessions_processed += 1;
        let rows = sqlx::query(
            r#"
            SELECT raw_json, lp_collected_token_a_raw, lp_collected_token_b_raw
            FROM position_stream_ledger_rows
            WHERE rebalance_session_id = $1
            ORDER BY ts_utc ASC NULLS LAST
            "#,
        )
        .bind(&sid)
        .fetch_all(db.pool())
        .await?;

        for r in &rows {
            report.rows_scanned += 1;
            let raw: Value = r.get("raw_json");
            let lp_a: Option<i64> = r.try_get("lp_collected_token_a_raw").ok().flatten();
            let lp_b: Option<i64> = r.try_get("lp_collected_token_b_raw").ok().flatten();
            match apply_session_postings_from_lifecycle_row(db, &raw, lp_a, lp_b).await {
                Ok(LifecyclePostingOutcome::Applied) => report.postings_applied += 1,
                Ok(LifecyclePostingOutcome::SkippedAlready) => report.rows_skipped_already += 1,
                Ok(LifecyclePostingOutcome::SkippedNoDeltas) => report.rows_skipped_no_deltas += 1,
                Err(e) => return Err(e),
            }
            apply_tx_fee_posting_from_lifecycle_json(db, &raw).await;
        }
    }

    Ok(report)
}

fn last_close_returned_from_lifecycle_json(
    v: &Value,
    lp_a: Option<i64>,
    lp_b: Option<i64>,
) -> Option<Vec<(String, i128)>> {
    let event = v.get("event").and_then(|x| x.as_str()).unwrap_or("");
    if !matches!(event, "bot_close_position" | "position_close") {
        return None;
    }
    session_mint_deltas_from_lifecycle_json(v, lp_a, lp_b).map(|(_, _, _, p)| p)
}

pub fn session_reconcile_enabled() -> bool {
    match std::env::var("CLMM_WALLET_GL_SESSION_RECONCILE") {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        ),
        Err(_) => true,
    }
}

pub async fn reconcile_session_gl(
    db: &Database,
    session_id: &str,
    owner: Option<&str>,
) -> Result<WalletSessionGlReconcileResponse, sqlx::Error> {
    let gl = read_session_balances(db, session_id, owner).await?;
    let pslr = compute_session_balances_from_pslr(db, session_id).await?;

    let close_row = sqlx::query(
        r#"
        SELECT raw_json, lp_collected_token_a_raw, lp_collected_token_b_raw
        FROM position_stream_ledger_rows
        WHERE rebalance_session_id = $1
          AND event IN ('bot_close_position', 'position_close')
        ORDER BY ts_utc DESC NULLS LAST
        LIMIT 1
        "#,
    )
    .bind(session_id)
    .fetch_optional(db.pool())
    .await?;

    let mut last_close_returned: Vec<WalletSessionBalanceRow> = Vec::new();
    if let Some(r) = close_row {
        let raw: Value = r.get("raw_json");
        let lp_a: Option<i64> = r.try_get("lp_collected_token_a_raw").ok().flatten();
        let lp_b: Option<i64> = r.try_get("lp_collected_token_b_raw").ok().flatten();
        if let Some(posts) = last_close_returned_from_lifecycle_json(&raw, lp_a, lp_b) {
            last_close_returned = posts
                .into_iter()
                .map(|(mint, amount)| WalletSessionBalanceRow {
                    mint,
                    amount_raw: format_raw_i128(amount),
                    decimals: None,
                })
                .collect();
        }
    }

    let mut gl_map: BTreeMap<String, String> = BTreeMap::new();
    for b in &gl {
        gl_map.insert(b.mint.clone(), b.amount_raw.clone());
    }
    let mut pslr_map: BTreeMap<String, String> = BTreeMap::new();
    for b in &pslr {
        pslr_map.insert(b.mint.clone(), b.amount_raw.clone());
    }
    let mut close_map: BTreeMap<String, String> = BTreeMap::new();
    for b in &last_close_returned {
        close_map.insert(b.mint.clone(), b.amount_raw.clone());
    }

    let mut all_mints: BTreeMap<String, ()> = BTreeMap::new();
    for m in gl_map.keys() {
        all_mints.insert(m.clone(), ());
    }
    for m in pslr_map.keys() {
        all_mints.insert(m.clone(), ());
    }

    let mut gaps = Vec::new();
    let gl_mint: Vec<SessionBalanceMint> = gl
        .iter()
        .map(|b| SessionBalanceMint {
            mint: b.mint.clone(),
            amount_raw: b.amount_raw.clone(),
        })
        .collect();
    let pslr_mint: Vec<SessionBalanceMint> = pslr
        .iter()
        .map(|b| SessionBalanceMint {
            mint: b.mint.clone(),
            amount_raw: b.amount_raw.clone(),
        })
        .collect();
    let gl_matches_pslr = wallet_session::gl_pslr_match(&gl_mint, &pslr_mint);
    for mint in all_mints.keys() {
        let g = gl_map.get(mint);
        let p = pslr_map.get(mint);
        let c = close_map.get(mint);
        let mismatch = match (g, p) {
            (Some(gv), Some(pv)) => gv != pv,
            (Some(_), None) | (None, Some(_)) => true,
            (None, None) => false,
        };
        if mismatch || c.is_some() {
            gaps.push(WalletSessionGlReconcileGap {
                mint: mint.clone(),
                gl_amount_raw: g.cloned(),
                pslr_amount_raw: p.cloned(),
                last_close_returned_raw: c.cloned(),
            });
        }
    }

    let note = if last_close_returned.is_empty() {
        "Compare gl vs pslr (should match after backfill). last_close_returned is informational only (§6.1); SESSION sum includes collect/swap/open in session."
            .to_string()
    } else {
        "gl vs pslr should match when posting is complete. last_close_returned is from the latest close row only, not full SESSION inventory."
            .to_string()
    };

    Ok(WalletSessionGlReconcileResponse {
        session_id: session_id.to_string(),
        gl_balances: gl,
        pslr_balances: pslr,
        last_close_returned,
        gaps,
        gl_matches_pslr,
        note,
    })
}

/// Principal open/close is posted from lifecycle (`open_amount_*` / `close_amount_*` on-chain).
/// Journal rows use request caps and a different `event_id` — skip to avoid double SESSION GL.
pub fn journal_principal_deferred_to_lifecycle(kind: &str) -> bool {
    lifecycle_posting_enabled()
        && matches!(kind.trim(), "open_position" | "close_position")
}

/// Apply SESSION balance updates from a confirmed wallet journal event (best-effort).
pub async fn apply_session_postings_from_journal(db: &Database, ev: &WalletLedgerEvent) {
    if !session_posting_enabled() {
        return;
    }
    if journal_principal_deferred_to_lifecycle(&ev.kind) {
        return;
    }
    let Some(session_id) = session_id_from_ledger_event(ev) else {
        return;
    };
    let Some(postings) = session_postings_from_event(ev) else {
        return;
    };
    if let Some(sig) = ev.signature.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let lifecycle_id = lifecycle_posting_event_id(sig);
        if session_lifecycle_posting_already_applied(db, &lifecycle_id)
            .await
            .unwrap_or(false)
        {
            return;
        }
    }
    let owner = ev.owner.as_deref();
    apply_postings_to_session_best_effort(
        db, &session_id, owner, &ev.event_id, &ev.kind, &postings,
    )
    .await;
}

/// Aggregate SESSION balances from `position_stream_ledger_rows` (fallback read, no GL write).
pub async fn compute_session_balances_from_pslr(
    db: &Database,
    session_id: &str,
) -> Result<Vec<WalletSessionBalanceRow>, sqlx::Error> {
    let rows = wallet_session::compute_session_balances_from_pslr(db, session_id).await?;
    Ok(rows
        .into_iter()
        .map(|b| WalletSessionBalanceRow {
            mint: b.mint,
            amount_raw: b.amount_raw,
            decimals: None,
        })
        .collect())
}

fn fmt_usd_opt(v: Option<f64>) -> Option<String> {
    v.filter(|x| x.is_finite())
        .map(|x| format!("{x:.8}"))
}

fn to_balance_rows(mints: &[wallet_session::SessionBalanceMint]) -> Vec<WalletSessionBalanceRow> {
    mints
        .iter()
        .map(|b| WalletSessionBalanceRow {
            mint: b.mint.clone(),
            amount_raw: b.amount_raw.clone(),
            decimals: None,
        })
        .collect()
}

fn fmt_price_usd(v: f64) -> String {
    format!("{v:.8}")
}

fn to_balance_usd_legs(
    legs: &[wallet_session::SessionBalanceUsdLeg],
) -> Vec<WalletSessionBalanceUsdLeg> {
    legs.iter()
        .map(|leg| WalletSessionBalanceUsdLeg {
            mint: leg.mint.clone(),
            amount_raw: leg.amount_raw.clone(),
            price_usd: leg.price_usd.map(fmt_price_usd),
            value_usd: fmt_usd_opt(leg.value_usd),
        })
        .collect()
}

fn to_open_start_snapshot(
    s: &wallet_session::SessionOpenStartSnapshot,
) -> WalletSessionOpenStartSnapshot {
    let price_by_mint_usd = s
        .price_by_mint
        .iter()
        .map(|(mint, px)| (mint.clone(), fmt_price_usd(*px)))
        .collect();
    WalletSessionOpenStartSnapshot {
        ts_utc: s.ts_utc.clone(),
        signature: s.signature.clone(),
        position_pubkey: s.position_pubkey.clone(),
        event: s.event.clone(),
        deployed_balances: to_balance_rows(&s.deployed_balances),
        value_usd: fmt_usd_opt(s.value_usd),
        value_usd_source: s.value_usd_source.clone(),
        pre_open_balances: to_balance_rows(&s.pre_open_balances),
        pre_open_value_usd: fmt_usd_opt(s.pre_open_value_usd),
        mint_resolution: s.mint_resolution.clone(),
        price_by_mint_usd,
    }
}

/// Cycle-start metrics: first open in session + current session USD (at open event prices).
pub async fn resolve_session_metrics(
    db: &Database,
    session_id: &str,
    owner: Option<&str>,
    current_balances: &[WalletSessionBalanceRow],
) -> Result<Option<WalletSessionMetrics>, sqlx::Error> {
    let gl: Vec<wallet_session::SessionBalanceMint> = current_balances
        .iter()
        .map(|b| wallet_session::SessionBalanceMint {
            mint: b.mint.clone(),
            amount_raw: b.amount_raw.clone(),
        })
        .collect();
    let current =
        wallet_session::session_balances_for_metrics(db, session_id, &gl, owner).await?;
    let resolved =
        wallet_session::compute_session_metrics_from_pslr(db, session_id, &current).await?;
    let Some(open) = resolved.open_start else {
        return Ok(None);
    };
    let current_legs = wallet_session::session_balance_usd_legs(&current, &open.price_by_mint);
    Ok(Some(WalletSessionMetrics {
        open_start: to_open_start_snapshot(&open),
        current_value_usd: fmt_usd_opt(resolved.current_value_usd),
        delta_vs_pre_open_usd: fmt_usd_opt(resolved.delta_vs_pre_open_usd),
        current_balance_usd_legs: to_balance_usd_legs(&current_legs),
        metrics_trusted: resolved.metrics_trusted,
    }))
}

/// Read SESSION balances: GL when it matches PSLR; empty GL → PSLR; GL mismatch → PSLR (corrected).
pub async fn read_session_balances_resolved(
    db: &Database,
    session_id: &str,
    owner: Option<&str>,
) -> Result<GlScopeReadResolved, sqlx::Error> {
    let gl_rows = read_session_balances(db, session_id, owner).await?;
    let gl_mint: Vec<wallet_session::SessionBalanceMint> = gl_rows
        .iter()
        .map(|b| wallet_session::SessionBalanceMint {
            mint: b.mint.clone(),
            amount_raw: b.amount_raw.clone(),
        })
        .collect();
    let pslr_mint =
        wallet_session::compute_session_balances_from_pslr(db, session_id).await?;
    let pslr: Vec<WalletSessionBalanceRow> = pslr_mint
        .iter()
        .map(|b| WalletSessionBalanceRow {
            mint: b.mint.clone(),
            amount_raw: b.amount_raw.clone(),
            decimals: None,
        })
        .collect();

    let gl_matches_pslr = wallet_session::gl_pslr_match(&gl_mint, &pslr_mint);
    let (balances, source) = if gl_rows.is_empty() {
        let source = if pslr.is_empty() {
            "gl_session_shadow_empty".to_string()
        } else {
            "gl_session_shadow_pslr_fallback".to_string()
        };
        (pslr, source)
    } else if gl_matches_pslr {
        (gl_rows, "gl_session_shadow".to_string())
    } else {
        (pslr, "gl_session_shadow_pslr_corrected".to_string())
    };

    let quality = gl_read_quality_from_source(&source).to_string();
    Ok(GlScopeReadResolved {
        needs_reconcile: gl_needs_reconcile_from_read(&source, gl_matches_pslr, None),
        quality,
        gl_matches_pslr,
        source,
        balances,
    })
}

/// Read current SESSION balances for analytics (shadow read model).
pub async fn read_session_balances(
    db: &Database,
    session_id: &str,
    owner: Option<&str>,
) -> Result<Vec<WalletSessionBalanceRow>, sqlx::Error> {
    let rows = wallet_session::read_session_balances(db, session_id, owner).await?;
    Ok(rows
        .into_iter()
        .map(|b| WalletSessionBalanceRow {
            mint: b.mint,
            amount_raw: b.amount_raw,
            decimals: None,
        })
        .collect())
}

pub fn chain_posting_enabled() -> bool {
    match std::env::var("CLMM_WALLET_GL_CHAIN_POSTING") {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        ),
        Err(_) => true,
    }
}

pub fn chain_read_enabled() -> bool {
    match std::env::var("CLMM_WALLET_GL_CHAIN_READ") {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        ),
        Err(_) => true,
    }
}

pub async fn apply_chain_postings_from_lifecycle_json(
    db: &Database,
    v: &Value,
    lp_collected_a_raw: Option<i64>,
    lp_collected_b_raw: Option<i64>,
) {
    if !chain_posting_enabled() {
        return;
    }
    match wallet_session::apply_chain_postings_from_lifecycle_row(
        db,
        v,
        lp_collected_a_raw,
        lp_collected_b_raw,
    )
    .await
    {
        Ok(_) => {}
        Err(e) => tracing::warn!(error = %e, "wallet_gl_posting: chain lifecycle row posting failed"),
    }
}

pub async fn backfill_chain_postings_from_pslr(
    db: &Database,
    chain_session_id: Option<&str>,
    max_chains: usize,
) -> Result<WalletChainGlBackfillReport, sqlx::Error> {
    let max_chains = max_chains.clamp(1, 500);
    let mut report = WalletChainGlBackfillReport {
        chains_processed: 0,
        rows_scanned: 0,
        postings_applied: 0,
        rows_skipped_already: 0,
        rows_skipped_no_deltas: 0,
    };

    let chain_ids: Vec<String> =
        if let Some(cid) = chain_session_id.map(str::trim).filter(|s| !s.is_empty()) {
            vec![cid.to_string()]
        } else {
            let rows = sqlx::query(
                r#"
                SELECT DISTINCT chain_session_id AS cid
                FROM position_stream_ledger_rows
                WHERE chain_session_id IS NOT NULL AND TRIM(chain_session_id) <> ''
                ORDER BY cid
                LIMIT $1
                "#,
            )
            .bind(max_chains as i64)
            .fetch_all(db.pool())
            .await?;
            rows.iter()
                .filter_map(|r| {
                    let s: String = r.get("cid");
                    let t = s.trim();
                    if t.is_empty() {
                        None
                    } else {
                        Some(t.to_string())
                    }
                })
                .collect()
        };

    for cid in chain_ids {
        report.chains_processed += 1;
        let rows = sqlx::query(
            r#"
            SELECT raw_json, lp_collected_token_a_raw, lp_collected_token_b_raw
            FROM position_stream_ledger_rows
            WHERE chain_session_id = $1
            ORDER BY ts_utc ASC NULLS LAST
            "#,
        )
        .bind(&cid)
        .fetch_all(db.pool())
        .await?;

        for r in &rows {
            report.rows_scanned += 1;
            let raw: Value = r.get("raw_json");
            let lp_a: Option<i64> = r.try_get("lp_collected_token_a_raw").ok().flatten();
            let lp_b: Option<i64> = r.try_get("lp_collected_token_b_raw").ok().flatten();
            match wallet_session::apply_chain_postings_from_lifecycle_row(db, &raw, lp_a, lp_b).await
            {
                Ok(wallet_session::ChainLifecyclePostingOutcome::Applied) => {
                    report.postings_applied += 1;
                }
                Ok(wallet_session::ChainLifecyclePostingOutcome::SkippedAlready) => {
                    report.rows_skipped_already += 1;
                }
                Ok(wallet_session::ChainLifecyclePostingOutcome::SkippedNoDeltas) => {
                    report.rows_skipped_no_deltas += 1;
                }
                Err(e) => return Err(e),
            }
        }
    }

    Ok(report)
}

async fn fetch_chain_pslr_agg_rows(
    db: &Database,
    chain_session_id: &str,
) -> Result<Vec<(Value, Option<i64>, Option<i64>)>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT raw_json, lp_collected_token_a_raw, lp_collected_token_b_raw
        FROM position_stream_ledger_rows
        WHERE chain_session_id = $1
        ORDER BY ts_utc ASC NULLS LAST, signature ASC NULLS LAST
        "#,
    )
    .bind(chain_session_id)
    .fetch_all(db.pool())
    .await?;
    Ok(rows
        .iter()
        .map(|r| {
            let raw: Value = r.get("raw_json");
            let lp_a: Option<i64> = r.try_get("lp_collected_token_a_raw").ok().flatten();
            let lp_b: Option<i64> = r.try_get("lp_collected_token_b_raw").ok().flatten();
            (raw, lp_a, lp_b)
        })
        .collect())
}

fn chain_pslr_mints_from_agg(
    agg: &[(Value, Option<i64>, Option<i64>)],
    chain_session_id: &str,
) -> Vec<SessionBalanceMint> {
    wallet_session::aggregate_chain_sums_from_lifecycle_rows(agg.iter().cloned(), chain_session_id)
        .into_iter()
        .map(|(mint, amount_raw)| SessionBalanceMint {
            mint,
            amount_raw: format_raw_i128(amount_raw),
        })
        .collect()
}

fn chain_metrics_from_agg(
    agg: Vec<(Value, Option<i64>, Option<i64>)>,
    chain_session_id: &str,
    current_balances: &[WalletSessionBalanceRow],
) -> Option<WalletSessionMetrics> {
    let current: Vec<SessionBalanceMint> = current_balances
        .iter()
        .map(|b| SessionBalanceMint {
            mint: b.mint.clone(),
            amount_raw: b.amount_raw.clone(),
        })
        .collect();
    let trusted = wallet_session::chain_principal_mints_trusted(&agg, chain_session_id);
    let open_start =
        wallet_session::compute_chain_open_start_from_lifecycle_rows(agg, chain_session_id);
    match open_start {
        Some(ref snap) => {
            let resolved = wallet_session::resolve_session_metrics_from_open_start(
                snap,
                &current,
                trusted,
            );
            let current_legs =
                wallet_session::session_balance_usd_legs(&current, &snap.price_by_mint);
            Some(WalletSessionMetrics {
                open_start: to_open_start_snapshot(snap),
                current_value_usd: fmt_usd_opt(resolved.current_value_usd),
                delta_vs_pre_open_usd: fmt_usd_opt(resolved.delta_vs_pre_open_usd),
                current_balance_usd_legs: to_balance_usd_legs(&current_legs),
                metrics_trusted: resolved.metrics_trusted,
            })
        }
        None => None,
    }
}

/// Single PSLR fetch + parallel GL read for `GET /wallets/chain-portfolio` hot path.
pub async fn read_chain_portfolio_resolved(
    db: &Database,
    chain_session_id: &str,
    owner: Option<&str>,
) -> Result<(GlScopeReadResolved, Option<WalletSessionMetrics>), sqlx::Error> {
    let (gl_rows, agg) = tokio::join!(
        read_chain_balances(db, chain_session_id, owner),
        fetch_chain_pslr_agg_rows(db, chain_session_id),
    );
    let gl_rows = gl_rows?;
    let agg = agg?;
    let pslr_mint = chain_pslr_mints_from_agg(&agg, chain_session_id);
    let pslr: Vec<WalletSessionBalanceRow> = pslr_mint
        .iter()
        .map(|b| WalletSessionBalanceRow {
            mint: b.mint.clone(),
            amount_raw: b.amount_raw.clone(),
            decimals: None,
        })
        .collect();

    let gl_mint: Vec<SessionBalanceMint> = gl_rows
        .iter()
        .map(|b| SessionBalanceMint {
            mint: b.mint.clone(),
            amount_raw: b.amount_raw.clone(),
        })
        .collect();
    let gl_matches_pslr = wallet_session::gl_pslr_match(&gl_mint, &pslr_mint);
    let (balances, source) = if gl_rows.is_empty() {
        let source = if pslr.is_empty() {
            "gl_chain_shadow_empty".to_string()
        } else {
            "gl_chain_shadow_pslr_fallback".to_string()
        };
        (pslr, source)
    } else if gl_matches_pslr {
        (gl_rows, "gl_chain_shadow".to_string())
    } else {
        (pslr, "gl_chain_shadow_pslr_corrected".to_string())
    };
    let quality = gl_read_quality_from_source(&source).to_string();
    let resolved = GlScopeReadResolved {
        needs_reconcile: gl_needs_reconcile_from_read(&source, gl_matches_pslr, None),
        quality,
        gl_matches_pslr,
        source,
        balances: balances.clone(),
    };
    let metrics = chain_metrics_from_agg(agg, chain_session_id, &balances);
    Ok((resolved, metrics))
}

pub async fn read_chain_balances_resolved(
    db: &Database,
    chain_session_id: &str,
    owner: Option<&str>,
) -> Result<GlScopeReadResolved, sqlx::Error> {
    let gl_rows = read_chain_balances(db, chain_session_id, owner).await?;
    let gl_mint: Vec<wallet_session::SessionBalanceMint> = gl_rows
        .iter()
        .map(|b| wallet_session::SessionBalanceMint {
            mint: b.mint.clone(),
            amount_raw: b.amount_raw.clone(),
        })
        .collect();
    let pslr_mint =
        wallet_session::compute_chain_balances_from_pslr(db, chain_session_id).await?;
    let pslr: Vec<WalletSessionBalanceRow> = pslr_mint
        .iter()
        .map(|b| WalletSessionBalanceRow {
            mint: b.mint.clone(),
            amount_raw: b.amount_raw.clone(),
            decimals: None,
        })
        .collect();

    let gl_matches_pslr = wallet_session::gl_pslr_match(&gl_mint, &pslr_mint);
    let (balances, source) = if gl_rows.is_empty() {
        let source = if pslr.is_empty() {
            "gl_chain_shadow_empty".to_string()
        } else {
            "gl_chain_shadow_pslr_fallback".to_string()
        };
        (pslr, source)
    } else if gl_matches_pslr {
        (gl_rows, "gl_chain_shadow".to_string())
    } else {
        (pslr, "gl_chain_shadow_pslr_corrected".to_string())
    };

    let quality = gl_read_quality_from_source(&source).to_string();
    Ok(GlScopeReadResolved {
        needs_reconcile: gl_needs_reconcile_from_read(&source, gl_matches_pslr, None),
        quality,
        gl_matches_pslr,
        source,
        balances,
    })
}

pub async fn read_chain_balances(
    db: &Database,
    chain_session_id: &str,
    owner: Option<&str>,
) -> Result<Vec<WalletSessionBalanceRow>, sqlx::Error> {
    let rows = wallet_session::read_chain_balances(db, chain_session_id, owner).await?;
    Ok(rows
        .into_iter()
        .map(|b| WalletSessionBalanceRow {
            mint: b.mint,
            amount_raw: b.amount_raw,
            decimals: None,
        })
        .collect())
}

pub async fn resolve_chain_metrics(
    db: &Database,
    chain_session_id: &str,
    owner: Option<&str>,
    current_balances: &[WalletSessionBalanceRow],
) -> Result<Option<WalletSessionMetrics>, sqlx::Error> {
    let _owner = owner;
    let current: Vec<wallet_session::SessionBalanceMint> = current_balances
        .iter()
        .map(|b| wallet_session::SessionBalanceMint {
            mint: b.mint.clone(),
            amount_raw: b.amount_raw.clone(),
        })
        .collect();
    let rows = sqlx::query(
        r#"
        SELECT raw_json, lp_collected_token_a_raw, lp_collected_token_b_raw
        FROM position_stream_ledger_rows
        WHERE chain_session_id = $1
        ORDER BY ts_utc ASC NULLS LAST, signature ASC NULLS LAST
        "#,
    )
    .bind(chain_session_id)
    .fetch_all(db.pool())
    .await?;
    let agg: Vec<(Value, Option<i64>, Option<i64>)> = rows
        .iter()
        .map(|r| {
            let raw: Value = r.get("raw_json");
            let lp_a: Option<i64> = r.try_get("lp_collected_token_a_raw").ok().flatten();
            let lp_b: Option<i64> = r.try_get("lp_collected_token_b_raw").ok().flatten();
            (raw, lp_a, lp_b)
        })
        .collect();
    let trusted = wallet_session::chain_principal_mints_trusted(&agg, chain_session_id);
    let open_start =
        wallet_session::compute_chain_open_start_from_lifecycle_rows(agg, chain_session_id);
    Ok(match open_start {
        Some(ref snap) => {
            let resolved = wallet_session::resolve_session_metrics_from_open_start(
                snap,
                &current,
                trusted,
            );
            let current_legs =
                wallet_session::session_balance_usd_legs(&current, &snap.price_by_mint);
            Some(WalletSessionMetrics {
                open_start: to_open_start_snapshot(snap),
                current_value_usd: fmt_usd_opt(resolved.current_value_usd),
                delta_vs_pre_open_usd: fmt_usd_opt(resolved.delta_vs_pre_open_usd),
                current_balance_usd_legs: to_balance_usd_legs(&current_legs),
                metrics_trusted: resolved.metrics_trusted,
            })
        }
        None => None,
    })
}

/// Compare CHAIN GL vs PSLR aggregate (Phase D1).
pub async fn reconcile_chain_gl(
    db: &Database,
    chain_session_id: &str,
    owner: Option<&str>,
) -> Result<WalletSessionGlReconcileResponse, sqlx::Error> {
    let gl = read_chain_balances(db, chain_session_id, owner).await?;
    let pslr_mint =
        wallet_session::compute_chain_balances_from_pslr(db, chain_session_id).await?;
    let pslr: Vec<WalletSessionBalanceRow> = pslr_mint
        .iter()
        .map(|b| WalletSessionBalanceRow {
            mint: b.mint.clone(),
            amount_raw: b.amount_raw.clone(),
            decimals: None,
        })
        .collect();

    let mut gl_map: BTreeMap<String, String> = BTreeMap::new();
    for b in &gl {
        gl_map.insert(b.mint.clone(), b.amount_raw.clone());
    }
    let mut pslr_map: BTreeMap<String, String> = BTreeMap::new();
    for b in &pslr {
        pslr_map.insert(b.mint.clone(), b.amount_raw.clone());
    }

    let mut all_mints: BTreeMap<String, ()> = BTreeMap::new();
    for m in gl_map.keys() {
        all_mints.insert(m.clone(), ());
    }
    for m in pslr_map.keys() {
        all_mints.insert(m.clone(), ());
    }

    let gl_mint: Vec<SessionBalanceMint> = gl
        .iter()
        .map(|b| SessionBalanceMint {
            mint: b.mint.clone(),
            amount_raw: b.amount_raw.clone(),
        })
        .collect();
    let gl_matches_pslr = wallet_session::gl_pslr_match(&gl_mint, &pslr_mint);
    let mut gaps = Vec::new();
    for mint in all_mints.keys() {
        let g = gl_map.get(mint);
        let p = pslr_map.get(mint);
        let mismatch = match (g, p) {
            (Some(gv), Some(pv)) => gv != pv,
            (Some(_), None) | (None, Some(_)) => true,
            (None, None) => false,
        };
        if mismatch {
            gaps.push(WalletSessionGlReconcileGap {
                mint: mint.clone(),
                gl_amount_raw: g.cloned(),
                pslr_amount_raw: p.cloned(),
                last_close_returned_raw: None,
            });
        }
    }

    Ok(WalletSessionGlReconcileResponse {
        session_id: chain_session_id.to_string(),
        gl_balances: gl,
        pslr_balances: pslr,
        last_close_returned: vec![],
        gaps,
        gl_matches_pslr,
        note: "Compare CHAIN GL vs PSLR aggregate for chain_session_id. Run chain-portfolio backfill when gaps remain."
            .to_string(),
    })
}

pub fn chain_reconcile_enabled() -> bool {
    match std::env::var("CLMM_WALLET_GL_CHAIN_RECONCILE") {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        ),
        Err(_) => true,
    }
}

pub fn wallet_posting_enabled() -> bool {
    match std::env::var("CLMM_WALLET_GL_WALLET_POSTING") {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        ),
        Err(_) => true,
    }
}

pub fn wallet_read_enabled() -> bool {
    match std::env::var("CLMM_WALLET_GL_WALLET_READ") {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        ),
        Err(_) => true,
    }
}

/// Build WALLET postings from confirmed journal rows (`transfer_sol`, `convert_sol`).
#[must_use]
pub fn wallet_postings_from_ledger_event(ev: &WalletLedgerEvent) -> Option<Vec<(String, i128)>> {
    if ev.status != WalletLedgerStatus::Confirmed || ev.dry_run {
        return None;
    }
    if !matches!(ev.kind.as_str(), "transfer_sol" | "convert_sol") {
        return None;
    }
    let mut sums: BTreeMap<String, i128> = BTreeMap::new();
    for d in &ev.deltas {
        let mint = d.mint.trim();
        if mint.is_empty() {
            continue;
        }
        let Some(delta) = parse_raw_i128(&d.raw_delta_i128) else {
            continue;
        };
        if delta == 0 {
            continue;
        }
        *sums.entry(mint.to_string()).or_insert(0) = sums.get(mint).copied().unwrap_or(0) + delta;
    }
    if let Some(n) = ev.native_lamports_delta.as_deref().and_then(parse_raw_i128)
        && n != 0 {
            let e = sums.entry(wallet_session::WSOL_MINT.to_string()).or_insert(0);
            *e += n;
        }
    if sums.is_empty() {
        None
    } else {
        Some(sums.into_iter().collect())
    }
}

/// Apply WALLET GL from a confirmed wallet journal event (best-effort).
pub async fn apply_wallet_postings_from_journal(db: &Database, ev: &WalletLedgerEvent) {
    if !wallet_posting_enabled() {
        return;
    }
    let Some(owner) = ev
        .owner
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return;
    };
    let Some(postings) = wallet_postings_from_ledger_event(ev) else {
        return;
    };
    let event_id = wallet_journal_posting_event_id(&ev.event_id);
    if session_lifecycle_posting_already_applied(db, &event_id)
        .await
        .unwrap_or(false)
    {
        return;
    }
    if let Err(e) = wallet_session::apply_wallet_mint_postings(
        db,
        owner,
        &event_id,
        &ev.kind,
        &postings,
    )
    .await
    {
        tracing::warn!(
            error = %e,
            owner = %owner,
            event_id = %ev.event_id,
            "wallet_gl_posting: apply_wallet_mint_postings failed"
        );
    }
}

pub async fn read_wallet_gl_balances(
    db: &Database,
    owner: &str,
) -> Result<WalletGlBalancesResponse, sqlx::Error> {
    let owner = owner.trim().to_string();
    if !wallet_read_enabled() {
        return Ok(WalletGlBalancesResponse {
            owner,
            source: "gl_wallet_shadow_disabled".to_string(),
            quality: "disabled".to_string(),
            needs_reconcile: false,
            opening_import_applied: false,
            balances: vec![],
        });
    }
    let opening_import_applied =
        wallet_session::wallet_opening_import_already_applied(db, &owner).await?;
    let rows = wallet_session::read_wallet_balances(db, &owner).await?;
    let balances: Vec<WalletSessionBalanceRow> = rows
        .iter()
        .map(|b| WalletSessionBalanceRow {
            mint: b.mint.clone(),
            amount_raw: b.amount_raw.clone(),
            decimals: None,
        })
        .collect();
    let source = if balances.is_empty() {
        "gl_wallet_shadow_empty".to_string()
    } else {
        "gl_wallet_shadow".to_string()
    };
    let quality = gl_read_quality_from_source(&source).to_string();
    let needs_reconcile = !opening_import_applied || balances.is_empty();
    Ok(WalletGlBalancesResponse {
        owner,
        source,
        quality,
        needs_reconcile,
        opening_import_applied,
        balances,
    })
}

pub fn wallet_rpc_compare_enabled() -> bool {
    match std::env::var("CLMM_WALLET_GL_WALLET_RPC_COMPARE") {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        ),
        Err(_) => true,
    }
}

fn wallet_rpc_compare_warn_threshold_raw(mint: &str) -> i128 {
    let default = match std::env::var("CLMM_WALLET_GL_WALLET_RPC_COMPARE_WARN_RAW") {
        Ok(v) => v.trim().parse::<i128>().unwrap_or(1_000_000),
        Err(_) => 1_000_000,
    };
    if mint == wallet_session::USDC_MINT {
        default / 100
    } else {
        default
    }
}

fn raw_map_from_postings(postings: &[(String, i128)]) -> BTreeMap<String, i128> {
    let mut out: BTreeMap<String, i128> = BTreeMap::new();
    for (mint, raw) in postings {
        let m = mint.trim();
        if m.is_empty() || *raw == 0 {
            continue;
        }
        *out.entry(m.to_string()).or_insert(0) += *raw;
    }
    out
}

fn raw_map_from_gl_rows(rows: &[WalletSessionBalanceRow]) -> BTreeMap<String, i128> {
    let mut out: BTreeMap<String, i128> = BTreeMap::new();
    for row in rows {
        let Some(raw) = parse_raw_i128(&row.amount_raw) else {
            continue;
        };
        if raw == 0 {
            continue;
        }
        *out.entry(row.mint.clone()).or_insert(0) += raw;
    }
    out
}

fn balance_rows_from_raw_map(map: &BTreeMap<String, i128>) -> Vec<WalletSessionBalanceRow> {
    map.iter()
        .map(|(mint, raw)| WalletSessionBalanceRow {
            mint: mint.clone(),
            amount_raw: raw.to_string(),
            decimals: None,
        })
        .collect()
}

/// Compare WALLET GL vs effective-balances RPC snapshot (Phase D3).
#[must_use]
pub fn reconcile_wallet_gl_vs_rpc(
    owner: &str,
    gl: &WalletGlBalancesResponse,
    rpc_postings: &[(String, i128)],
    rpc_confidence: &str,
    rpc_is_stale: bool,
    rpc_as_of_utc: Option<&str>,
) -> WalletGlRpcReconcileResponse {
    let gl_map = raw_map_from_gl_rows(&gl.balances);
    let rpc_map = raw_map_from_postings(rpc_postings);

    let mut all_mints: BTreeMap<String, ()> = BTreeMap::new();
    for m in gl_map.keys() {
        all_mints.insert(m.clone(), ());
    }
    for m in rpc_map.keys() {
        all_mints.insert(m.clone(), ());
    }

    let mut gaps = Vec::new();
    for mint in all_mints.keys() {
        let g = gl_map.get(mint).copied();
        let r = rpc_map.get(mint).copied();
        let mismatch = match (g, r) {
            (Some(gv), Some(rv)) => gv != rv,
            (Some(_), None) | (None, Some(_)) => true,
            (None, None) => false,
        };
        if mismatch {
            let delta = match (g, r) {
                (Some(gv), Some(rv)) => Some(format_raw_i128(gv - rv)),
                (Some(gv), None) => Some(format_raw_i128(gv)),
                (None, Some(rv)) => Some(format_raw_i128(-rv)),
                (None, None) => None,
            };
            gaps.push(WalletGlRpcReconcileGap {
                mint: mint.clone(),
                gl_amount_raw: g.map(format_raw_i128),
                rpc_amount_raw: r.map(format_raw_i128),
                delta_raw: delta,
            });
        }
    }

    let gl_matches_rpc = gaps.is_empty();
    for gap in &gaps {
        let Some(delta) = gap.delta_raw.as_deref().and_then(parse_raw_i128) else {
            continue;
        };
        if delta.abs() <= wallet_rpc_compare_warn_threshold_raw(&gap.mint) {
            continue;
        }
        if matches!(gap.mint.as_str(), m if m == wallet_session::WSOL_MINT || m == wallet_session::USDC_MINT)
        {
            tracing::warn!(
                owner = %owner,
                mint = %gap.mint,
                gl = ?gap.gl_amount_raw,
                rpc = ?gap.rpc_amount_raw,
                delta_raw = ?gap.delta_raw,
                "wallet_gl_rpc_compare: curated mint gap exceeds threshold"
            );
        }
    }

    let note = if gl_matches_rpc {
        "WALLET GL matches effective-balances RPC snapshot (native+WSOL merged as WSOL mint)."
    } else if !gl.opening_import_applied {
        "Gaps expected before opening import or when journal postings lag RPC (external transfers not in GL)."
    } else {
        "WALLET GL differs from RPC — check journal coverage, opening import baseline, and pending convert ops."
    };

    WalletGlRpcReconcileResponse {
        owner: owner.to_string(),
        gl_source: gl.source.clone(),
        gl_quality: gl.quality.clone(),
        gl_opening_import_applied: gl.opening_import_applied,
        rpc_confidence: rpc_confidence.to_string(),
        rpc_is_stale,
        rpc_as_of_utc: rpc_as_of_utc.map(str::to_string),
        gl_balances: gl.balances.clone(),
        rpc_balances: balance_rows_from_raw_map(&rpc_map),
        gaps,
        gl_matches_rpc,
        note: note.to_string(),
    }
}

/// Whether `GET /wallets/effective-balances` may overlay amounts from `WALLET:{owner}` GL (D4).
pub fn wallet_effective_read_enabled() -> bool {
    match std::env::var("CLMM_WALLET_GL_EFFECTIVE_READ") {
        Ok(v) => matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => false,
    }
}

fn mint_decimals_for_gl(mint: &str) -> u8 {
    if mint == wallet_session::USDC_MINT {
        6
    } else {
        9
    }
}

fn raw_i128_to_ui_amount(raw: i128, decimals: u8) -> String {
    if raw == 0 {
        return "0".to_string();
    }
    let neg = raw < 0;
    let abs = if neg { -raw } else { raw };
    let scale = 10i128.pow(u32::from(decimals));
    let whole = abs / scale;
    let frac = abs % scale;
    let frac_str = format!("{:0>width$}", frac, width = usize::from(decimals));
    let trimmed = frac_str.trim_end_matches('0');
    let ui = if trimmed.is_empty() {
        whole.to_string()
    } else {
        format!("{whole}.{trimmed}")
    };
    if neg {
        format!("-{ui}")
    } else {
        ui
    }
}

/// True when WALLET GL is safe to use as effective-balance source (flag must already be on).
#[must_use]
pub fn wallet_gl_trustworthy_for_effective_read(
    gl: &WalletGlBalancesResponse,
    rpc: &WalletEffectiveBalancesResponse,
) -> bool {
    wallet_read_enabled()
        && gl.opening_import_applied
        && !gl.needs_reconcile
        && gl.quality == "exact"
        && !gl.balances.is_empty()
        && rpc.pending_ops_count == 0
        && !rpc.is_stale
}

/// Overlay RPC effective balances with WALLET GL when D4 flag on and GL trustworthy.
#[must_use]
pub fn apply_wallet_gl_effective_read_overlay(
    rpc: WalletEffectiveBalancesResponse,
    gl: Option<&WalletGlBalancesResponse>,
) -> WalletEffectiveBalancesResponse {
    apply_wallet_gl_effective_read_overlay_with_enabled(
        rpc,
        gl,
        wallet_effective_read_enabled(),
    )
}

#[must_use]
pub fn apply_wallet_gl_effective_read_overlay_with_enabled(
    mut rpc: WalletEffectiveBalancesResponse,
    gl: Option<&WalletGlBalancesResponse>,
    enabled: bool,
) -> WalletEffectiveBalancesResponse {
    if !enabled {
        return rpc;
    }
    if rpc.cache_source.as_deref() == Some("warmup") {
        return rpc;
    }
    let Some(gl) = gl else {
        rpc.effective_balance_source = Some("rpc_fallback".to_string());
        return rpc;
    };
    if !wallet_gl_trustworthy_for_effective_read(gl, &rpc) {
        rpc.effective_balance_source = Some("rpc_fallback".to_string());
        return rpc;
    }

    let gl_map = raw_map_from_gl_rows(&gl.balances);
    let wsol_total = gl_map
        .get(wallet_session::WSOL_MINT)
        .copied()
        .unwrap_or(0)
        .max(0) as u64;
    rpc.native_effective_lamports = wsol_total;
    rpc.lamports = wsol_total;
    rpc.sol = format!("{:.9}", (wsol_total as f64) / 1e9);
    rpc.wsol_effective_raw = 0;
    rpc.wsol_onchain_raw = 0;

    let mut tokens = Vec::new();
    for (mint, raw) in &gl_map {
        if mint == wallet_session::WSOL_MINT || *raw == 0 {
            continue;
        }
        tokens.push(WalletTokenBalance {
            mint: mint.clone(),
            ui_amount: raw_i128_to_ui_amount(*raw, mint_decimals_for_gl(mint)),
        });
    }
    rpc.tokens = tokens;
    rpc.token_accounts_total = Some(rpc.tokens.len() as u64);
    rpc.confidence = WalletBalanceConfidence::Verified;
    rpc.effective_balance_source = Some("gl_wallet".to_string());
    tracing::info!(
        owner = %rpc.owner,
        mint_count = rpc.tokens.len(),
        sol_lamports = wsol_total,
        "wallet_gl_effective_read: serving effective-balances from WALLET GL"
    );
    rpc
}

pub async fn import_wallet_opening_balance(
    db: &Database,
    owner: &str,
    postings: &[(String, i128)],
) -> Result<WalletGlOpeningImportReport, sqlx::Error> {
    let owner = owner.trim().to_string();
    match wallet_session::apply_wallet_opening_import(db, &owner, postings).await? {
        SessionLifecyclePostingOutcome::Applied => Ok(WalletGlOpeningImportReport {
            owner: owner.clone(),
            mints_posted: postings.len() as u32,
            status: "applied".to_string(),
            note: format!(
                "Opening snapshot posted to {} (idempotent key {})",
                wallet_account_code(&owner),
                wallet_opening_import_event_id(&owner)
            ),
        }),
        SessionLifecyclePostingOutcome::SkippedAlready => Ok(WalletGlOpeningImportReport {
            owner: owner.clone(),
            mints_posted: 0,
            status: "skipped_already".to_string(),
            note: "Opening import already applied for this owner.".to_string(),
        }),
        SessionLifecyclePostingOutcome::SkippedNoDeltas => Ok(WalletGlOpeningImportReport {
            owner,
            mints_posted: 0,
            status: "skipped_no_deltas".to_string(),
            note: "No non-zero mint amounts to post.".to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::WalletLedgerDelta;
    use clmm_lp_data::wallet_session::{USDC_MINT, WSOL_MINT};

    fn sample_event(deltas: Vec<WalletLedgerDelta>, status: WalletLedgerStatus) -> WalletLedgerEvent {
        WalletLedgerEvent {
            schema_version: 1,
            ts_utc: "2026-01-01T00:00:00Z".to_string(),
            event_id: "ev-1".to_string(),
            correlation_id: "corr-1".to_string(),
            status,
            kind: "close_position".to_string(),
            owner: Some("Owner1111111111111111111111111111111111111111".to_string()),
            signature: None,
            pool_address: None,
            position_pda: Some("Pos111111111111111111111111111111111111111111".to_string()),
            cost_session_id: Some("sess-uuid-1".to_string()),
            dry_run: false,
            native_lamports_delta: None,
            deltas,
            error: None,
            source: "test".to_string(),
            decode_status: None,
        }
    }

    #[test]
    fn gl_read_quality_maps_source_strings() {
        assert_eq!(gl_read_quality_from_source("gl_session_shadow"), "exact");
        assert_eq!(
            gl_read_quality_from_source("gl_chain_shadow_pslr_fallback"),
            "pslr_fallback"
        );
        assert!(gl_needs_reconcile_from_read(
            "gl_session_shadow_pslr_corrected",
            false,
            None
        ));
        assert!(!gl_needs_reconcile_from_read("gl_session_shadow", true, Some(true)));
    }

    #[test]
    fn wallet_postings_from_transfer_journal_native_lamports() {
        let mut ev = sample_event(vec![], WalletLedgerStatus::Confirmed);
        ev.kind = "transfer_sol".to_string();
        ev.native_lamports_delta = Some("-1000000".to_string());
        let posts = wallet_postings_from_ledger_event(&ev).expect("posts");
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].0, WSOL_MINT);
        assert_eq!(posts[0].1, -1_000_000);
    }

    #[test]
    fn reconcile_wallet_gl_rpc_detects_wsol_gap() {
        let gl = WalletGlBalancesResponse {
            owner: "Owner1111111111111111111111111111111111111111".to_string(),
            source: "gl_wallet_shadow".to_string(),
            quality: "exact".to_string(),
            needs_reconcile: false,
            opening_import_applied: true,
            balances: vec![WalletSessionBalanceRow {
                mint: WSOL_MINT.to_string(),
                amount_raw: "1000000".to_string(),
                decimals: None,
            }],
        };
        let rpc = vec![(WSOL_MINT.to_string(), 900_000i128)];
        let resp = reconcile_wallet_gl_vs_rpc(&gl.owner, &gl, &rpc, "verified", false, None);
        assert!(!resp.gl_matches_rpc);
        assert_eq!(resp.gaps.len(), 1);
        assert_eq!(resp.gaps[0].delta_raw.as_deref(), Some("100000"));
    }

    #[test]
    fn reconcile_wallet_gl_rpc_match_when_maps_equal() {
        let gl = WalletGlBalancesResponse {
            owner: "Owner1111111111111111111111111111111111111111".to_string(),
            source: "gl_wallet_shadow".to_string(),
            quality: "exact".to_string(),
            needs_reconcile: false,
            opening_import_applied: true,
            balances: vec![
                WalletSessionBalanceRow {
                    mint: WSOL_MINT.to_string(),
                    amount_raw: "5000000".to_string(),
                    decimals: None,
                },
                WalletSessionBalanceRow {
                    mint: USDC_MINT.to_string(),
                    amount_raw: "2500000".to_string(),
                    decimals: None,
                },
            ],
        };
        let rpc = vec![
            (WSOL_MINT.to_string(), 5_000_000i128),
            (USDC_MINT.to_string(), 2_500_000i128),
        ];
        let resp = reconcile_wallet_gl_vs_rpc(&gl.owner, &gl, &rpc, "verified", false, None);
        assert!(resp.gl_matches_rpc);
        assert!(resp.gaps.is_empty());
    }

    fn sample_gl_trustworthy(owner: &str, wsol_raw: &str) -> WalletGlBalancesResponse {
        WalletGlBalancesResponse {
            owner: owner.to_string(),
            source: "gl_wallet_shadow".to_string(),
            quality: "exact".to_string(),
            needs_reconcile: false,
            opening_import_applied: true,
            balances: vec![WalletSessionBalanceRow {
                mint: WSOL_MINT.to_string(),
                amount_raw: wsol_raw.to_string(),
                decimals: None,
            }],
        }
    }

    fn sample_rpc_effective(owner: &str) -> WalletEffectiveBalancesResponse {
        WalletEffectiveBalancesResponse {
            owner: owner.to_string(),
            as_of_utc: "2026-01-01T00:00:00Z".to_string(),
            is_stale: false,
            stale_age_ms: 0,
            confidence: WalletBalanceConfidence::Verified,
            pending_ops_count: 0,
            native_onchain_lamports: 500_000,
            native_effective_lamports: 500_000,
            wsol_onchain_raw: 0,
            wsol_effective_raw: 0,
            rpc_url: "https://rpc.example".to_string(),
            lamports: 500_000,
            sol: "0.000500000".to_string(),
            tokens: vec![],
            token_accounts_total: Some(0),
            token_legacy_ok: Some(true),
            token_2022_ok: Some(true),
            token_legacy_error: None,
            token_2022_error: None,
            cache_source: Some("memory".to_string()),
            cache_updated_at_utc: None,
            effective_balance_source: None,
        }
    }

    #[test]
    fn wallet_gl_trustworthy_requires_opening_and_no_reconcile() {
        let owner = "Owner1111111111111111111111111111111111111111";
        let rpc = sample_rpc_effective(owner);
        let mut gl = sample_gl_trustworthy(owner, "1000000");
        assert!(wallet_gl_trustworthy_for_effective_read(&gl, &rpc));
        gl.needs_reconcile = true;
        assert!(!wallet_gl_trustworthy_for_effective_read(&gl, &rpc));
    }

    #[test]
    fn effective_read_overlay_applies_gl_amounts_when_enabled() {
        let owner = "Owner1111111111111111111111111111111111111111";
        let gl = sample_gl_trustworthy(owner, "2000000");
        let rpc = sample_rpc_effective(owner);
        let out = apply_wallet_gl_effective_read_overlay_with_enabled(rpc, Some(&gl), true);
        assert_eq!(out.effective_balance_source.as_deref(), Some("gl_wallet"));
        assert_eq!(out.lamports, 2_000_000);
    }

    #[test]
    fn effective_read_overlay_rpc_fallback_when_gl_not_trustworthy() {
        let owner = "Owner1111111111111111111111111111111111111111";
        let mut gl = sample_gl_trustworthy(owner, "2000000");
        gl.opening_import_applied = false;
        let rpc = sample_rpc_effective(owner);
        let out = apply_wallet_gl_effective_read_overlay_with_enabled(rpc.clone(), Some(&gl), true);
        assert_eq!(out.effective_balance_source.as_deref(), Some("rpc_fallback"));
        assert_eq!(out.lamports, rpc.lamports);
    }

    #[test]
    fn session_id_only_on_confirmed_non_dry_run() {
        let mut ev = sample_event(vec![], WalletLedgerStatus::Confirmed);
        assert_eq!(
            session_id_from_ledger_event(&ev).as_deref(),
            Some("sess-uuid-1")
        );
        ev.status = WalletLedgerStatus::Pending;
        assert!(session_id_from_ledger_event(&ev).is_none());
        ev.status = WalletLedgerStatus::Confirmed;
        ev.dry_run = true;
        assert!(session_id_from_ledger_event(&ev).is_none());
    }

    #[test]
    fn postings_sum_deltas_per_mint() {
        let ev = sample_event(
            vec![
                WalletLedgerDelta {
                    mint: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string(),
                    decimals: 6,
                    raw_delta_i128: "1000000".to_string(),
                },
                WalletLedgerDelta {
                    mint: "So11111111111111111111111111111111111111112".to_string(),
                    decimals: 9,
                    raw_delta_i128: "-500000".to_string(),
                },
            ],
            WalletLedgerStatus::Confirmed,
        );
        let posts = session_postings_from_event(&ev).expect("posts");
        assert_eq!(posts.len(), 2);
        assert_eq!(posts[0].1, 1_000_000);
        assert_eq!(posts[1].1, -500_000);
    }

    #[test]
    fn session_account_code_format() {
        assert_eq!(session_account_code("abc"), "SESSION:abc");
    }

    #[test]
    fn lifecycle_close_row_posts_principal_and_lp() {
        let v = serde_json::json!({
            "event": "bot_close_position",
            "signature": "sig-close-1",
            "rebalance_session_id": "sess-abc",
            "fee_payer_pubkey": "Owner1111111111111111111111111111111111111111",
            "lp_collected_token_a_raw": 50_000,
            "lp_collected_token_b_raw": 0,
            "details": {
                "token_mint_a": "So11111111111111111111111111111111111111112",
                "token_mint_b": "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
                "close_amount_a_raw": 1_000_000_000u64,
                "close_amount_b_raw": 2_000_000u64
            }
        });
        let (sid, sig, ev, posts) =
            session_mint_deltas_from_lifecycle_json(&v, Some(50_000), Some(0)).expect("posts");
        assert_eq!(sid, "sess-abc");
        assert_eq!(sig, "sig-close-1");
        assert_eq!(ev, "bot_close_position");
        assert_eq!(posts.len(), 3);
        assert!(posts.iter().any(|(m, d)| m == WSOL_MINT && *d == 1_000_000_000));
        assert!(posts.iter().any(|(m, d)| m == USDC_MINT && *d == 2_000_000));
        assert!(posts.iter().any(|(m, d)| m == WSOL_MINT && *d == 50_000));
    }

    #[test]
    fn lifecycle_collect_uses_lp_columns() {
        let v = serde_json::json!({
            "event": "bot_collect_fees",
            "signature": "sig-collect-1",
            "rebalance_session_id": "sess-xyz",
            "details": {
                "token_mint_a": "So11111111111111111111111111111111111111112",
                "token_mint_b": "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
            }
        });
        let posts =
            session_mint_deltas_from_lifecycle_json(&v, Some(10), Some(20)).expect("posts").3;
        assert_eq!(posts.len(), 2);
        assert_eq!(posts[0].1, 10);
        assert_eq!(posts[1].1, 20);
    }

    #[test]
    fn lifecycle_posting_event_id_prefix() {
        assert_eq!(
            lifecycle_posting_event_id("abc123"),
            "lifecycle:abc123"
        );
    }

    #[test]
    fn journal_open_close_deferred_when_lifecycle_posting_on() {
        assert!(journal_principal_deferred_to_lifecycle("open_position"));
        assert!(journal_principal_deferred_to_lifecycle("close_position"));
        assert!(!journal_principal_deferred_to_lifecycle("swap_before_open"));
        assert!(!journal_principal_deferred_to_lifecycle("collect_fees"));
    }

    #[test]
    fn open_lifecycle_single_post_matches_cap_plus_onchain_double() {
        let v = serde_json::json!({
            "event": "bot_open_position",
            "signature": "sig-open-1",
            "rebalance_session_id": "sess-dup",
            "details": {
                "token_mint_a": WSOL_MINT,
                "token_mint_b": USDC_MINT,
                "open_amount_a_raw": 60_435_307u64,
                "open_amount_b_raw": 4_720_942u64,
                "amount_a_cap": 60_435_308u64,
                "amount_b_cap": 4_865_859u64
            }
        });
        let posts =
            session_mint_deltas_from_lifecycle_json(&v, None, None).expect("posts").3;
        assert_eq!(posts.len(), 2);
        assert_eq!(posts.iter().find(|(m, _)| m == WSOL_MINT).map(|(_, d)| *d), Some(-60_435_307));
        assert_eq!(
            posts.iter().find(|(m, _)| m == USDC_MINT).map(|(_, d)| *d),
            Some(-4_720_942)
        );
        let journal_caps = vec![
            (WSOL_MINT.to_string(), -60_435_308i128),
            (USDC_MINT.to_string(), -4_865_859i128),
        ];
        let mut combined = posts.clone();
        for (m, d) in journal_caps {
            if let Some((_, e)) = combined.iter_mut().find(|(mint, _)| mint == &m) {
                *e = e.saturating_add(d);
            } else {
                combined.push((m, d));
            }
        }
        let pslr: Vec<wallet_session::SessionBalanceMint> = posts
            .into_iter()
            .map(|(mint, amount_raw)| wallet_session::SessionBalanceMint {
                mint,
                amount_raw: wallet_session::format_raw_i128(amount_raw),
            })
            .collect();
        let gl: Vec<wallet_session::SessionBalanceMint> = combined
            .into_iter()
            .map(|(mint, amount_raw)| wallet_session::SessionBalanceMint {
                mint,
                amount_raw: wallet_session::format_raw_i128(amount_raw),
            })
            .collect();
        assert!(!wallet_session::gl_pslr_match(&gl, &pslr));
    }
}
