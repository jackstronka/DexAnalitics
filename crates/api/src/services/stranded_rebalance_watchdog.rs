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

fn pending_open_path_string() -> String {
    std::env::var("CLMM_PENDING_OPEN_RECOVERY_PATH")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "data/pending-open-recovery.json".to_string())
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
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir).ok();
        }
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
        if let Some(f) = filter {
            if !v.to_string().contains(f) {
                continue;
            }
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
        match event {
            "bot_close_position" => {
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
    out.sort_by(|a, b| b.close_ts_utc.cmp(&a.close_ts_utc));
    out
}

/// Read-only scan (no writes). Used by `GET /bot-activity/stranded-rebalances`.
pub fn get_stranded_rebalances_snapshot() -> anyhow::Result<StrandedRebalancesResponse> {
    let lifecycle_path = ledger_read_path();
    let il_path = il_ledger_path_from_env();
    let pending_open_path = pending_open_path_string();
    let pending_path_buf = PathBuf::from(&pending_open_path);

    let (_lifecycle_missing, _total, lifecycle_rows) =
        read_jsonl_tail(lifecycle_path.as_path(), MAX_LEDGER_ROWS, 0, None)?;
    let il_rows = if let Some(ref p) = il_path {
        read_jsonl_tail(p.as_path(), MAX_LEDGER_ROWS, 0, None)?.2
    } else {
        Vec::new()
    };
    let pending_store = load_pending_open_store(&pending_path_buf)?;
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

    let (_lifecycle_missing, _total, lifecycle_rows) =
        read_jsonl_tail(lifecycle_path.as_path(), MAX_LEDGER_ROWS, 0, None)?;
    let (_il_missing, _il_total, il_rows) =
        read_jsonl_tail(il_path, MAX_LEDGER_ROWS, 0, None)?;
    let mut pending_store = load_pending_open_store(&pending_path_buf)?;

    let items_before = build_stranded_rebalances(&lifecycle_rows, &il_rows, &pending_store);
    let mut auto_enqueued = 0usize;
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
    use super::LocalPendingOpenStore;
    use super::build_stranded_rebalances;

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
}
