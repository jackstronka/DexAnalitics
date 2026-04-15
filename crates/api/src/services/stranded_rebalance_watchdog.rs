//! Detect rebalance sessions with `bot_close_position` but no `bot_open_position`, and optionally
//! enqueue recovery rows into `pending-open-recovery.json` when IL ledger has `rebalance_incomplete`.

use crate::models::{StrandedRebalanceItem, StrandedRebalancesResponse};
use anyhow::Context;
use clmm_lp_execution::lifecycle::RebalanceReason;
use clmm_lp_protocols::ledger::tx_lifecycle::{il_ledger_path_from_env, ledger_read_path};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const MAX_LEDGER_ROWS: usize = 2000;

/// Background reconcile interval. `0` = disabled (default).
#[must_use]
pub fn reconcile_interval_secs_from_env() -> u64 {
    std::env::var("CLMM_STRANDED_RECONCILE_INTERVAL_SECS")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .filter(|&n| n > 0 && n <= 86_400)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
struct LocalPendingOpenStore {
    #[serde(default)]
    items: Vec<LocalPendingOpenItem>,
    #[serde(default)]
    dismissed_session_ids: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct LocalPendingOpenItem {
    pool: String,
    intended_tick_lower: i32,
    intended_tick_upper: i32,
    closed_position_nft: String,
    reason: RebalanceReason,
    optimization_run_id: Option<String>,
    attempts: u32,
    last_error: Option<String>,
    created_at: String,
}

#[derive(Debug, Clone)]
struct IncompleteHints {
    intended_tick_lower: Option<i32>,
    intended_tick_upper: Option<i32>,
    reason: Option<String>,
}

fn parse_iso_ts(v: Option<&serde_json::Value>) -> Option<String> {
    v.and_then(serde_json::Value::as_str)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn parse_i32(v: Option<&serde_json::Value>) -> Option<i32> {
    v.and_then(|x| x.as_i64())
        .and_then(|x| i32::try_from(x).ok())
}

fn parse_reason_str(raw: Option<&str>) -> Option<RebalanceReason> {
    match raw.map(str::trim) {
        Some("RangeExit") => Some(RebalanceReason::RangeExit),
        Some("RetouchShift") => Some(RebalanceReason::RetouchShift),
        Some("ILThreshold") => Some(RebalanceReason::ILThreshold),
        Some("Periodic") => Some(RebalanceReason::Periodic),
        Some("Manual") => Some(RebalanceReason::Manual),
        Some("Optimization") => Some(RebalanceReason::Optimization),
        _ => None,
    }
}

fn token_short_label(mint: &str) -> String {
    let m = mint.trim();
    match m {
        "So11111111111111111111111111111111111111112" => "SOL".to_string(),
        "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v" => "USDC".to_string(),
        "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB" => "USDT".to_string(),
        _ => m.chars().take(4).collect(),
    }
}

fn is_manual_close_event(row: &serde_json::Value) -> bool {
    let Some(details) = row.get("details").and_then(serde_json::Value::as_object) else {
        return false;
    };
    let close_kind_manual = details
        .get("close_kind")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|v| v.eq_ignore_ascii_case("manual"));
    let close_source_api = details
        .get("close_source")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|v| v.eq_ignore_ascii_case("api"));
    close_kind_manual || close_source_api
}

fn pending_open_path_string() -> String {
    std::env::var("CLMM_PENDING_OPEN_RECOVERY_PATH")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "data/pending-open-recovery.json".to_string())
}

fn dismissed_sessions_path_string() -> String {
    std::env::var("CLMM_STRANDED_DISMISSED_PATH")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "data/stranded-dismissed-sessions.json".to_string())
}

fn load_dismissed_session_ids(path: &Path) -> anyhow::Result<std::collections::HashSet<String>> {
    if !path.exists() {
        return Ok(std::collections::HashSet::new());
    }
    let txt = std::fs::read_to_string(path).context("read dismissed sessions file")?;
    let trimmed = txt.trim();
    if trimmed.is_empty() {
        return Ok(std::collections::HashSet::new());
    }
    let mut out = std::collections::HashSet::new();
    let v: serde_json::Value =
        serde_json::from_str(trimmed).context("parse dismissed sessions JSON")?;
    if let Some(arr) = v.get("session_ids").and_then(serde_json::Value::as_array) {
        for sid in arr {
            if let Some(s) = sid.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                out.insert(s.to_string());
            }
        }
    }
    Ok(out)
}

fn save_dismissed_session_ids(
    path: &Path,
    session_ids: &std::collections::HashSet<String>,
) -> anyhow::Result<()> {
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
    {
        std::fs::create_dir_all(dir).ok();
    }
    let mut ids: Vec<String> = session_ids
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    ids.sort();
    let payload = serde_json::json!({ "session_ids": ids });
    std::fs::write(
        path,
        serde_json::to_string_pretty(&payload).context("serialize dismissed sessions JSON")?,
    )
    .context("write dismissed sessions file")
}

fn load_pending_open_store(path: &Path) -> anyhow::Result<LocalPendingOpenStore> {
    if !path.exists() {
        return Ok(LocalPendingOpenStore::default());
    }
    let txt = std::fs::read_to_string(path).context("read pending-open file")?;
    let trimmed = txt.trim();
    if trimmed.is_empty() {
        return Ok(LocalPendingOpenStore::default());
    }
    serde_json::from_str::<LocalPendingOpenStore>(trimmed).context("parse pending-open JSON")
}

fn save_pending_open_store(path: &Path, store: &LocalPendingOpenStore) -> anyhow::Result<()> {
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
    {
        std::fs::create_dir_all(dir).ok();
    }
    std::fs::write(
        path,
        serde_json::to_string_pretty(store).context("serialize pending-open JSON")?,
    )
    .context("write pending-open file")
}

fn read_jsonl_tail(
    path: &Path,
    limit: usize,
    offset: usize,
    filter: Option<&str>,
) -> std::io::Result<(bool, usize, Vec<serde_json::Value>)> {
    if !path.exists() {
        return Ok((true, 0, Vec::new()));
    }

    let content = std::fs::read_to_string(path)?;
    let mut parsed: Vec<serde_json::Value> = Vec::new();

    for line in content.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(t) else {
            continue;
        };
        if let Some(f) = filter
            && !v.to_string().contains(f)
        {
            continue;
        }
        parsed.push(v);
    }

    let total = parsed.len();
    if total == 0 {
        return Ok((false, 0, Vec::new()));
    }
    let end_exclusive = total.saturating_sub(offset).min(total);
    let start_inclusive = end_exclusive.saturating_sub(limit);
    let out = parsed[start_inclusive..end_exclusive].to_vec();

    Ok((false, total, out))
}

fn build_stranded_rebalances(
    lifecycle_rows: &[serde_json::Value],
    il_rows: &[serde_json::Value],
    pending_store: &LocalPendingOpenStore,
) -> Vec<StrandedRebalanceItem> {
    let dismissed_sessions: std::collections::HashSet<String> = pending_store
        .dismissed_session_ids
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let mut by_sid: std::collections::HashMap<String, StrandedRebalanceItem> =
        std::collections::HashMap::new();
    let mut il_hints: std::collections::HashMap<String, IncompleteHints> =
        std::collections::HashMap::new();

    for row in il_rows {
        let Some(sid) = row
            .get("rebalance_session_id")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        if dismissed_sessions.contains(sid) {
            continue;
        }
        let Some(event) = row.get("event").and_then(serde_json::Value::as_str) else {
            continue;
        };
        if event != "rebalance_incomplete" {
            continue;
        }
        il_hints.insert(
            sid.to_string(),
            IncompleteHints {
                intended_tick_lower: parse_i32(row.get("intended_tick_lower")),
                intended_tick_upper: parse_i32(row.get("intended_tick_upper")),
                reason: row
                    .get("reason")
                    .and_then(serde_json::Value::as_str)
                    .map(|s| s.to_string()),
            },
        );
    }

    let mut pending_by_old: std::collections::HashSet<String> = std::collections::HashSet::new();
    for it in &pending_store.items {
        pending_by_old.insert(it.closed_position_nft.clone());
    }

    for row in lifecycle_rows {
        if row.get("source").and_then(serde_json::Value::as_str) != Some("orca_bot") {
            continue;
        }
        let Some(sid) = row
            .get("rebalance_session_id")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        if dismissed_sessions.contains(sid) {
            continue;
        }
        let Some(event) = row.get("event").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let entry = by_sid
            .entry(sid.to_string())
            .or_insert_with(|| StrandedRebalanceItem {
                rebalance_session_id: sid.to_string(),
                close_seen: false,
                open_seen: false,
                close_ts_utc: None,
                open_ts_utc: None,
                old_position: None,
                new_position: None,
                pool_address: None,
                token_mint_a: None,
                token_mint_b: None,
                token_a_label: None,
                token_b_label: None,
                rebalance_incomplete_logged: il_hints.contains_key(sid),
                in_pending_open_queue: false,
                intended_tick_lower: None,
                intended_tick_upper: None,
                reason: None,
                can_auto_enqueue: false,
                note: None,
            });
        entry.rebalance_incomplete_logged =
            entry.rebalance_incomplete_logged || il_hints.contains_key(sid);
        if entry.pool_address.is_none() {
            entry.pool_address = row
                .get("pool_address")
                .and_then(serde_json::Value::as_str)
                .map(|s| s.to_string());
        }
        if let Some(details) = row.get("details").and_then(serde_json::Value::as_object) {
            if entry.token_mint_a.is_none() {
                entry.token_mint_a = details
                    .get("token_mint_a")
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string());
            }
            if entry.token_mint_b.is_none() {
                entry.token_mint_b = details
                    .get("token_mint_b")
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string());
            }
            if entry.token_a_label.is_none()
                && let Some(m) = entry.token_mint_a.as_deref()
            {
                entry.token_a_label = Some(token_short_label(m));
            }
            if entry.token_b_label.is_none()
                && let Some(m) = entry.token_mint_b.as_deref()
            {
                entry.token_b_label = Some(token_short_label(m));
            }
        }
        match event {
            "bot_close_position" => {
                if is_manual_close_event(row) {
                    // Manual close can be logged through the same tx lifecycle path.
                    // Exclude it from the "Closed by bot, waiting for reopen" section.
                    continue;
                }
                entry.close_seen = true;
                if entry.close_ts_utc.is_none() {
                    entry.close_ts_utc = parse_iso_ts(row.get("ts_utc"));
                }
                if entry.old_position.is_none() {
                    entry.old_position = row
                        .get("position_pubkey")
                        .and_then(serde_json::Value::as_str)
                        .map(|s| s.to_string());
                }
            }
            "bot_open_position" | "bot_open_position_full_range" => {
                entry.open_seen = true;
                if entry.open_ts_utc.is_none() {
                    entry.open_ts_utc = parse_iso_ts(row.get("ts_utc"));
                }
                if entry.new_position.is_none() {
                    entry.new_position = row
                        .get("position_pubkey")
                        .and_then(serde_json::Value::as_str)
                        .map(|s| s.to_string());
                }
            }
            _ => {}
        }
    }

    let mut out: Vec<StrandedRebalanceItem> = Vec::new();
    for (_, mut it) in by_sid {
        if !it.close_seen || it.open_seen {
            continue;
        }
        if let Some(h) = il_hints.get(&it.rebalance_session_id) {
            it.intended_tick_lower = h.intended_tick_lower;
            it.intended_tick_upper = h.intended_tick_upper;
            it.reason = h.reason.clone();
            if it.reason.is_none() {
                it.reason = Some("Manual".to_string());
            }
        }
        if let Some(old) = &it.old_position {
            it.in_pending_open_queue = pending_by_old.contains(old);
        }
        it.can_auto_enqueue = it.intended_tick_lower.is_some()
            && it.intended_tick_upper.is_some()
            && it.old_position.is_some()
            && it.pool_address.is_some()
            && !it.in_pending_open_queue;
        it.note = if it.in_pending_open_queue {
            Some("Already queued for pending-open recovery.".to_string())
        } else if !it.rebalance_incomplete_logged {
            Some("Missing IL rebalance_incomplete row; watchdog can report but cannot infer intended ticks.".to_string())
        } else if !it.can_auto_enqueue {
            Some("Missing required fields for auto-enqueue.".to_string())
        } else {
            Some("Eligible for watchdog auto-enqueue to pending-open recovery.".to_string())
        };
        out.push(it);
    }

    // Keep UI/API operationally coherent with pending-open queue:
    // if queue contains a recoverable item without a visible lifecycle-close row,
    // expose it as a synthetic stranded row so operator can remove it from UI.
    for p in &pending_store.items {
        let old = p.closed_position_nft.trim();
        if old.is_empty() {
            continue;
        }
        let synthetic_sid = synthetic_pending_session_id(old, p.intended_tick_lower, p.intended_tick_upper);
        let group_sid = synthetic_pending_group_id(p.pool.trim(), p.intended_tick_lower, p.intended_tick_upper);
        if dismissed_sessions.contains(&synthetic_sid) || dismissed_sessions.contains(&group_sid) {
            continue;
        }
        let already_visible = out.iter().any(|it| {
            it.old_position
                .as_deref()
                .is_some_and(|x| x.trim() == old)
                || (it.pool_address.as_deref().is_some_and(|x| x == p.pool)
                    && it.intended_tick_lower == Some(p.intended_tick_lower)
                    && it.intended_tick_upper == Some(p.intended_tick_upper))
        });
        if already_visible {
            continue;
        }
        out.push(StrandedRebalanceItem {
            rebalance_session_id: synthetic_sid,
            close_seen: true,
            open_seen: false,
            close_ts_utc: Some(p.created_at.clone()),
            open_ts_utc: None,
            old_position: Some(p.closed_position_nft.clone()),
            new_position: None,
            pool_address: Some(p.pool.clone()),
            token_mint_a: None,
            token_mint_b: None,
            token_a_label: None,
            token_b_label: None,
            rebalance_incomplete_logged: false,
            in_pending_open_queue: true,
            intended_tick_lower: Some(p.intended_tick_lower),
            intended_tick_upper: Some(p.intended_tick_upper),
            reason: Some(format!("{:?}", p.reason)),
            can_auto_enqueue: false,
            note: Some(
                "Pending-open item exists without visible stranded lifecycle row; shown for operator cleanup."
                    .to_string(),
            ),
        });
    }
    out.sort_by(|a, b| b.close_ts_utc.cmp(&a.close_ts_utc));
    out
}

fn synthetic_pending_session_id(closed_position_nft: &str, lower: i32, upper: i32) -> String {
    format!("pending:{closed_position_nft}:{lower}:{upper}")
}

fn synthetic_pending_group_id(pool: &str, lower: i32, upper: i32) -> String {
    format!("pending-group:{pool}:{lower}:{upper}")
}

fn prune_pending_open_items_for_dismiss(
    pending_store: &mut LocalPendingOpenStore,
    dismissed_item: Option<&StrandedRebalanceItem>,
) {
    let Some(item) = dismissed_item else {
        return;
    };
    let pool = item.pool_address.as_deref().map(str::trim).unwrap_or_default();
    let old = item.old_position.as_deref().map(str::trim).unwrap_or_default();
    let lower = item.intended_tick_lower;
    let upper = item.intended_tick_upper;

    pending_store.items.retain(|x| {
        let same_old = !old.is_empty() && x.closed_position_nft.trim() == old;
        let same_pool_and_range = !pool.is_empty()
            && x.pool.trim() == pool
            && lower.is_some()
            && upper.is_some()
            && Some(x.intended_tick_lower) == lower
            && Some(x.intended_tick_upper) == upper;
        !(same_old || same_pool_and_range)
    });
}

pub fn dismiss_stranded_rebalance_session_for_api(
    session_id: &str,
) -> anyhow::Result<StrandedRebalancesResponse> {
    let sid = session_id.trim();
    if sid.is_empty() {
        return Err(anyhow::anyhow!("session_id is empty"));
    }

    let lifecycle_path = ledger_read_path();
    let il_path = il_ledger_path_from_env();
    let pending_open_path = pending_open_path_string();
    let pending_path_buf = PathBuf::from(&pending_open_path);
    let dismissed_path = PathBuf::from(dismissed_sessions_path_string());

    let (_lifecycle_missing, _total, lifecycle_rows) =
        read_jsonl_tail(lifecycle_path.as_path(), MAX_LEDGER_ROWS, 0, None)?;
    let il_rows = if let Some(ref p) = il_path {
        read_jsonl_tail(p.as_path(), MAX_LEDGER_ROWS, 0, None)?.2
    } else {
        Vec::new()
    };
    let mut pending_store = load_pending_open_store(&pending_path_buf)?;
    let items_before = build_stranded_rebalances(&lifecycle_rows, &il_rows, &pending_store);

    let dismissed_item = items_before.iter().find(|it| it.rebalance_session_id == sid);

    if !pending_store
        .dismissed_session_ids
        .iter()
        .any(|x| x.trim() == sid)
    {
        pending_store.dismissed_session_ids.push(sid.to_string());
    }
    let mut dismissed_set = load_dismissed_session_ids(&dismissed_path)?;
    dismissed_set.insert(sid.to_string());
    if let Some(item) = dismissed_item
        && let (Some(pool), Some(lower), Some(upper)) = (
            item.pool_address.as_deref(),
            item.intended_tick_lower,
            item.intended_tick_upper,
        )
    {
        let group_sid = synthetic_pending_group_id(pool.trim(), lower, upper);
        if !pending_store
            .dismissed_session_ids
            .iter()
            .any(|x| x.trim() == group_sid)
        {
            pending_store.dismissed_session_ids.push(group_sid.clone());
        }
        dismissed_set.insert(group_sid);
    }

    prune_pending_open_items_for_dismiss(&mut pending_store, dismissed_item);

    save_pending_open_store(&pending_path_buf, &pending_store)?;
    save_dismissed_session_ids(&dismissed_path, &dismissed_set)?;
    let items = build_stranded_rebalances(&lifecycle_rows, &il_rows, &pending_store);
    Ok(StrandedRebalancesResponse {
        lifecycle_path: lifecycle_path.display().to_string(),
        il_ledger_path: il_path.map(|p| p.display().to_string()),
        pending_open_path,
        rows_scanned: lifecycle_rows.len(),
        items,
        auto_enqueued: 0,
    })
}

/// Read-only scan (no writes). Used by `GET /bot-activity/stranded-rebalances`.
pub fn get_stranded_rebalances_snapshot() -> anyhow::Result<StrandedRebalancesResponse> {
    let lifecycle_path = ledger_read_path();
    let il_path = il_ledger_path_from_env();
    let pending_open_path = pending_open_path_string();
    let pending_path_buf = PathBuf::from(&pending_open_path);
    let dismissed_path = PathBuf::from(dismissed_sessions_path_string());

    let (_lifecycle_missing, _total, lifecycle_rows) =
        read_jsonl_tail(lifecycle_path.as_path(), MAX_LEDGER_ROWS, 0, None)?;
    let il_rows = if let Some(ref p) = il_path {
        read_jsonl_tail(p.as_path(), MAX_LEDGER_ROWS, 0, None)?.2
    } else {
        Vec::new()
    };
    let pending_store = load_pending_open_store(&pending_path_buf)?;
    let mut pending_store = pending_store;
    let dismissed_set = load_dismissed_session_ids(&dismissed_path)?;
    for sid in dismissed_set {
        if !pending_store
            .dismissed_session_ids
            .iter()
            .any(|x| x.trim() == sid)
        {
            pending_store.dismissed_session_ids.push(sid);
        }
    }
    let items = build_stranded_rebalances(&lifecycle_rows, &il_rows, &pending_store);

    Ok(StrandedRebalancesResponse {
        lifecycle_path: lifecycle_path.display().to_string(),
        il_ledger_path: il_path.map(|p| p.display().to_string()),
        pending_open_path,
        rows_scanned: lifecycle_rows.len(),
        items,
        auto_enqueued: 0,
    })
}

/// Full reconcile: requires IL ledger path (same as manual POST). Returns `Err` if unset.
pub fn reconcile_stranded_rebalances_for_api(
    enqueue_note: &str,
) -> anyhow::Result<StrandedRebalancesResponse> {
    let il_path = il_ledger_path_from_env()
        .ok_or_else(|| anyhow::anyhow!("CLMM_IL_LEDGER_PATH is not set"))?;
    reconcile_stranded_with_il_path(&il_path, enqueue_note)
}

/// Periodic tick: if IL ledger is not configured, no-op (no error) so the background task stays quiet.
pub fn reconcile_stranded_periodic_tick() -> anyhow::Result<StrandedRebalancesResponse> {
    let Some(il_path) = il_ledger_path_from_env() else {
        let lifecycle_path = ledger_read_path();
        return Ok(StrandedRebalancesResponse {
            lifecycle_path: lifecycle_path.display().to_string(),
            il_ledger_path: None,
            pending_open_path: pending_open_path_string(),
            rows_scanned: 0,
            items: vec![],
            auto_enqueued: 0,
        });
    };
    reconcile_stranded_with_il_path(
        &il_path,
        "watchdog periodic auto-enqueue (CLMM_STRANDED_RECONCILE_INTERVAL_SECS)",
    )
}

fn reconcile_stranded_with_il_path(
    il_path: &Path,
    enqueue_note: &str,
) -> anyhow::Result<StrandedRebalancesResponse> {
    let lifecycle_path = ledger_read_path();
    let pending_open_path = pending_open_path_string();
    let pending_path_buf = PathBuf::from(&pending_open_path);
    let dismissed_path = PathBuf::from(dismissed_sessions_path_string());

    let (_lifecycle_missing, _total, lifecycle_rows) =
        read_jsonl_tail(lifecycle_path.as_path(), MAX_LEDGER_ROWS, 0, None)?;
    let (_il_missing, _il_total, il_rows) =
        read_jsonl_tail(il_path, MAX_LEDGER_ROWS, 0, None)?;
    let mut pending_store = load_pending_open_store(&pending_path_buf)?;
    let dismissed_set = load_dismissed_session_ids(&dismissed_path)?;
    for sid in dismissed_set {
        if !pending_store
            .dismissed_session_ids
            .iter()
            .any(|x| x.trim() == sid)
        {
            pending_store.dismissed_session_ids.push(sid);
        }
    }

    let items_before = build_stranded_rebalances(&lifecycle_rows, &il_rows, &pending_store);
    let mut auto_enqueued = 0usize;
    let dismissed_sids: std::collections::HashSet<String> = pending_store
        .dismissed_session_ids
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    for it in &items_before {
        if !it.can_auto_enqueue {
            continue;
        }
        let Some(pool) = it.pool_address.clone() else {
            continue;
        };
        let Some(old_position) = it.old_position.clone() else {
            continue;
        };
        let Some(intended_tick_lower) = it.intended_tick_lower else {
            continue;
        };
        let Some(intended_tick_upper) = it.intended_tick_upper else {
            continue;
        };
        if dismissed_sids.contains(&it.rebalance_session_id) {
            continue;
        }
        let pending_sid =
            synthetic_pending_session_id(&old_position, intended_tick_lower, intended_tick_upper);
        if dismissed_sids.contains(&pending_sid) {
            continue;
        }
        let group_sid = synthetic_pending_group_id(&pool, intended_tick_lower, intended_tick_upper);
        if dismissed_sids.contains(&group_sid) {
            continue;
        }
        if pending_store
            .items
            .iter()
            .any(|x| x.pool == pool && x.closed_position_nft == old_position)
        {
            continue;
        }
        let reason = parse_reason_str(it.reason.as_deref()).unwrap_or(RebalanceReason::Manual);
        pending_store.items.push(LocalPendingOpenItem {
            pool,
            intended_tick_lower,
            intended_tick_upper,
            closed_position_nft: old_position,
            reason,
            optimization_run_id: None,
            attempts: 0,
            last_error: Some(enqueue_note.to_string()),
            created_at: chrono::Utc::now().to_rfc3339(),
        });
        auto_enqueued += 1;
    }
    if auto_enqueued > 0 {
        save_pending_open_store(&pending_path_buf, &pending_store)?;
    }

    let items = build_stranded_rebalances(&lifecycle_rows, &il_rows, &pending_store);
    Ok(StrandedRebalancesResponse {
        lifecycle_path: lifecycle_path.display().to_string(),
        il_ledger_path: Some(il_path.display().to_string()),
        pending_open_path,
        rows_scanned: lifecycle_rows.len(),
        items,
        auto_enqueued,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        LocalPendingOpenItem, LocalPendingOpenStore, StrandedRebalanceItem, build_stranded_rebalances,
        prune_pending_open_items_for_dismiss, synthetic_pending_group_id,
        synthetic_pending_session_id,
    };
    use clmm_lp_execution::lifecycle::RebalanceReason;

    #[test]
    fn stranded_session_close_without_open_is_auto_enqueueable_with_il_hints() {
        let sid = "sess-watchdog-1";
        let lifecycle = vec![
            serde_json::json!({
                "source": "orca_bot",
                "rebalance_session_id": sid,
                "event": "bot_close_position",
                "ts_utc": "2026-01-01T00:00:00Z",
                "position_pubkey": "oldNft111",
                "pool_address": "poolPub222",
            }),
        ];
        let il = vec![serde_json::json!({
            "event": "rebalance_incomplete",
            "rebalance_session_id": sid,
            "intended_tick_lower": -100,
            "intended_tick_upper": 100,
            "reason": "Manual",
        })];
        let pending = LocalPendingOpenStore::default();
        let items = build_stranded_rebalances(&lifecycle, &il, &pending);
        assert_eq!(items.len(), 1);
        let it = &items[0];
        assert_eq!(it.rebalance_session_id, sid);
        assert!(it.close_seen);
        assert!(!it.open_seen);
        assert!(it.can_auto_enqueue);
        assert_eq!(it.old_position.as_deref(), Some("oldNft111"));
        assert_eq!(it.pool_address.as_deref(), Some("poolPub222"));
    }

    #[test]
    fn open_in_same_session_is_not_stranded() {
        let sid = "sess-watchdog-2";
        let lifecycle = vec![
            serde_json::json!({
                "source": "orca_bot",
                "rebalance_session_id": sid,
                "event": "bot_close_position",
                "position_pubkey": "oldNft",
                "pool_address": "pool1",
            }),
            serde_json::json!({
                "source": "orca_bot",
                "rebalance_session_id": sid,
                "event": "bot_open_position",
                "position_pubkey": "newNft",
                "pool_address": "pool1",
            }),
        ];
        let il = vec![];
        let pending = LocalPendingOpenStore::default();
        let items = build_stranded_rebalances(&lifecycle, &il, &pending);
        assert!(items.is_empty());
    }

    #[test]
    fn manual_close_event_is_excluded_from_stranded_list() {
        let sid = "sess-watchdog-manual";
        let lifecycle = vec![serde_json::json!({
            "source": "orca_bot",
            "rebalance_session_id": sid,
            "event": "bot_close_position",
            "position_pubkey": "oldNft",
            "pool_address": "pool1",
            "details": {
                "close_kind": "manual",
                "close_source": "api"
            }
        })];
        let il = vec![serde_json::json!({
            "event": "rebalance_incomplete",
            "rebalance_session_id": sid,
            "intended_tick_lower": -10,
            "intended_tick_upper": 10
        })];
        let pending = LocalPendingOpenStore::default();
        let items = build_stranded_rebalances(&lifecycle, &il, &pending);
        assert!(items.is_empty());
    }

    #[test]
    fn dismissed_session_is_excluded_from_stranded_list() {
        let sid = "sess-watchdog-dismissed";
        let lifecycle = vec![serde_json::json!({
            "source": "orca_bot",
            "rebalance_session_id": sid,
            "event": "bot_close_position",
            "position_pubkey": "oldNft",
            "pool_address": "pool1"
        })];
        let il = vec![serde_json::json!({
            "event": "rebalance_incomplete",
            "rebalance_session_id": sid,
            "intended_tick_lower": -10,
            "intended_tick_upper": 10
        })];
        let pending = LocalPendingOpenStore {
            items: vec![],
            dismissed_session_ids: vec![sid.to_string()],
        };
        let items = build_stranded_rebalances(&lifecycle, &il, &pending);
        assert!(items.is_empty());
    }

    #[test]
    fn dismiss_prunes_pending_by_old_position_and_pool_range() {
        let mut pending = LocalPendingOpenStore {
            items: vec![
                LocalPendingOpenItem {
                    pool: "pool1".to_string(),
                    intended_tick_lower: -10,
                    intended_tick_upper: 10,
                    closed_position_nft: "oldA".to_string(),
                    reason: RebalanceReason::Manual,
                    optimization_run_id: None,
                    attempts: 0,
                    last_error: None,
                    created_at: "2026-04-14T00:00:00Z".to_string(),
                },
                // Different old_position, but same pool + intended range: should be removed too.
                LocalPendingOpenItem {
                    pool: "pool1".to_string(),
                    intended_tick_lower: -10,
                    intended_tick_upper: 10,
                    closed_position_nft: "otherOld".to_string(),
                    reason: RebalanceReason::Manual,
                    optimization_run_id: None,
                    attempts: 0,
                    last_error: None,
                    created_at: "2026-04-14T00:01:00Z".to_string(),
                },
                // Different range: should stay.
                LocalPendingOpenItem {
                    pool: "pool1".to_string(),
                    intended_tick_lower: -20,
                    intended_tick_upper: 20,
                    closed_position_nft: "otherRange".to_string(),
                    reason: RebalanceReason::Manual,
                    optimization_run_id: None,
                    attempts: 0,
                    last_error: None,
                    created_at: "2026-04-14T00:02:00Z".to_string(),
                },
            ],
            dismissed_session_ids: vec![],
        };
        let dismissed = StrandedRebalanceItem {
            rebalance_session_id: "sid-1".to_string(),
            close_seen: true,
            open_seen: false,
            close_ts_utc: None,
            open_ts_utc: None,
            old_position: Some("oldA".to_string()),
            new_position: None,
            pool_address: Some("pool1".to_string()),
            token_mint_a: None,
            token_mint_b: None,
            token_a_label: None,
            token_b_label: None,
            rebalance_incomplete_logged: true,
            in_pending_open_queue: true,
            intended_tick_lower: Some(-10),
            intended_tick_upper: Some(10),
            reason: Some("Manual".to_string()),
            can_auto_enqueue: false,
            note: None,
        };
        prune_pending_open_items_for_dismiss(&mut pending, Some(&dismissed));
        assert_eq!(pending.items.len(), 1);
        assert_eq!(pending.items[0].closed_position_nft, "otherRange");
    }

    #[test]
    fn pending_only_item_is_visible_in_stranded_output() {
        let lifecycle = vec![];
        let il = vec![];
        let pending = LocalPendingOpenStore {
            items: vec![LocalPendingOpenItem {
                pool: "pool-x".to_string(),
                intended_tick_lower: -100,
                intended_tick_upper: 100,
                closed_position_nft: "old-nft-x".to_string(),
                reason: RebalanceReason::Periodic,
                optimization_run_id: None,
                attempts: 2,
                last_error: Some("simulated".to_string()),
                created_at: "2026-04-14T14:00:00Z".to_string(),
            }],
            dismissed_session_ids: vec![],
        };
        let items = build_stranded_rebalances(&lifecycle, &il, &pending);
        assert_eq!(items.len(), 1);
        let it = &items[0];
        assert!(it.in_pending_open_queue);
        assert_eq!(it.old_position.as_deref(), Some("old-nft-x"));
        assert_eq!(it.pool_address.as_deref(), Some("pool-x"));
        assert_eq!(it.intended_tick_lower, Some(-100));
        assert_eq!(it.intended_tick_upper, Some(100));
        assert!(it.rebalance_session_id.starts_with("pending:"));
    }

    #[test]
    fn dismissed_synthetic_pending_row_is_hidden() {
        let sid = synthetic_pending_session_id("old-nft-hidden", -200, 200);
        let lifecycle = vec![];
        let il = vec![];
        let pending = LocalPendingOpenStore {
            items: vec![LocalPendingOpenItem {
                pool: "pool-hidden".to_string(),
                intended_tick_lower: -200,
                intended_tick_upper: 200,
                closed_position_nft: "old-nft-hidden".to_string(),
                reason: RebalanceReason::Periodic,
                optimization_run_id: None,
                attempts: 0,
                last_error: None,
                created_at: "2026-04-14T14:10:00Z".to_string(),
            }],
            dismissed_session_ids: vec![sid],
        };
        let items = build_stranded_rebalances(&lifecycle, &il, &pending);
        assert!(items.is_empty());
    }

    #[test]
    fn dismissed_pending_group_hides_all_matching_pending_rows() {
        let gid = synthetic_pending_group_id("pool-g", -300, 300);
        let lifecycle = vec![];
        let il = vec![];
        let pending = LocalPendingOpenStore {
            items: vec![
                LocalPendingOpenItem {
                    pool: "pool-g".to_string(),
                    intended_tick_lower: -300,
                    intended_tick_upper: 300,
                    closed_position_nft: "old-1".to_string(),
                    reason: RebalanceReason::Periodic,
                    optimization_run_id: None,
                    attempts: 0,
                    last_error: None,
                    created_at: "2026-04-14T14:20:00Z".to_string(),
                },
                LocalPendingOpenItem {
                    pool: "pool-g".to_string(),
                    intended_tick_lower: -300,
                    intended_tick_upper: 300,
                    closed_position_nft: "old-2".to_string(),
                    reason: RebalanceReason::RangeExit,
                    optimization_run_id: None,
                    attempts: 0,
                    last_error: None,
                    created_at: "2026-04-14T14:21:00Z".to_string(),
                },
            ],
            dismissed_session_ids: vec![gid],
        };
        let items = build_stranded_rebalances(&lifecycle, &il, &pending);
        assert!(items.is_empty());
    }
}
