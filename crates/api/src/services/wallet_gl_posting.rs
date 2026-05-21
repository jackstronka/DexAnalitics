//! GL posting for `SESSION:{session_id}` accounts (analytics sub-ledger).
//!
//! Policy: session accounts are **never closed or liquidated** after position close (including
//! manual close) — balances accumulate for operator analytics.
//!
//! **Primary source (PR-A):** `position_stream_ledger_rows` / lifecycle JSONL on ingest.
//! **Secondary:** wallet journal `deltas[]` when present.

use crate::models::{
    WalletLedgerEvent, WalletLedgerStatus, WalletSessionBalanceRow, WalletSessionGlBackfillReport,
    WalletSessionGlReconcileGap, WalletSessionGlReconcileResponse, WalletSessionMetrics,
    WalletSessionOpenStartSnapshot,
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
};

pub type LifecyclePostingOutcome = SessionLifecyclePostingOutcome;

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

fn to_open_start_snapshot(
    s: &wallet_session::SessionOpenStartSnapshot,
) -> WalletSessionOpenStartSnapshot {
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
    Ok(Some(WalletSessionMetrics {
        open_start: to_open_start_snapshot(&open),
        current_value_usd: fmt_usd_opt(resolved.current_value_usd),
        delta_vs_pre_open_usd: fmt_usd_opt(resolved.delta_vs_pre_open_usd),
        metrics_trusted: resolved.metrics_trusted,
    }))
}

/// Read SESSION balances: GL when it matches PSLR; empty GL → PSLR; GL mismatch → PSLR (corrected).
pub async fn read_session_balances_resolved(
    db: &Database,
    session_id: &str,
    owner: Option<&str>,
) -> Result<(Vec<WalletSessionBalanceRow>, String), sqlx::Error> {
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

    if gl_rows.is_empty() {
        let source = if pslr.is_empty() {
            "gl_session_shadow_empty".to_string()
        } else {
            "gl_session_shadow_pslr_fallback".to_string()
        };
        return Ok((pslr, source));
    }
    if wallet_session::gl_pslr_match(&gl_mint, &pslr_mint) {
        return Ok((gl_rows, "gl_session_shadow".to_string()));
    }
    Ok((pslr, "gl_session_shadow_pslr_corrected".to_string()))
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
        }
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
