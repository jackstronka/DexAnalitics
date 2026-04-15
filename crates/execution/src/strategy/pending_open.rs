//! Persisted queue for resuming `open` after `rebalance_incomplete`.

use crate::lifecycle::RebalanceReason;
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Root JSON document at `CLMM_PENDING_OPEN_RECOVERY_PATH` (default `data/pending-open-recovery.json`).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PendingOpenStore {
    /// Pending recoveries (usually 0–1 per bot process).
    #[serde(default)]
    pub items: Vec<PendingOpenItem>,
    /// Session ids explicitly dismissed from stranded-rebalance UI/API.
    /// Kept here so execution-side save() does not drop operator dismiss choices.
    #[serde(default)]
    pub dismissed_session_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingOpenItem {
    pub pool: String,
    pub intended_tick_lower: i32,
    pub intended_tick_upper: i32,
    pub closed_position_nft: String,
    #[serde(default)]
    pub rebalance_session_id: Option<String>,
    pub reason: RebalanceReason,
    pub optimization_run_id: Option<String>,
    pub attempts: u32,
    pub last_error: Option<String>,
    pub created_at: String,
}

pub fn load(path: &Path) -> anyhow::Result<PendingOpenStore> {
    if !path.exists() {
        return Ok(PendingOpenStore::default());
    }
    let txt = std::fs::read_to_string(path).context("read pending_open_recovery")?;
    serde_json::from_str(&txt).context("parse pending_open_recovery JSON")
}

pub fn save(path: &Path, store: &PendingOpenStore) -> anyhow::Result<()> {
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
    {
        std::fs::create_dir_all(dir).ok();
    }
    std::fs::write(
        path,
        serde_json::to_string_pretty(store).context("serialize pending_open_recovery")?,
    )
    .context("write pending_open_recovery")
}

/// Insert or replace by `(pool, closed_position_nft)`.
pub fn upsert(store: &mut PendingOpenStore, item: PendingOpenItem) {
    let key = (&item.pool, &item.closed_position_nft);
    if let Some(i) = store
        .items
        .iter()
        .position(|x| (&x.pool, &x.closed_position_nft) == key)
    {
        store.items[i] = item;
    } else {
        store.items.push(item);
    }
}

pub fn max_recovery_attempts() -> u32 {
    std::env::var("CLMM_PENDING_OPEN_MAX_ATTEMPTS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n| (1..=10_000).contains(&n))
        .unwrap_or(100)
}

#[cfg(test)]
mod tests {
    use super::PendingOpenStore;

    #[test]
    fn pending_open_store_parses_and_keeps_dismissed_sessions() {
        let raw = serde_json::json!({
            "items": [],
            "dismissed_session_ids": ["sid-1", "sid-2"]
        });
        let parsed: PendingOpenStore =
            serde_json::from_value(raw).expect("must parse pending-open store");
        assert_eq!(parsed.dismissed_session_ids.len(), 2);
        assert_eq!(parsed.dismissed_session_ids[0], "sid-1");
        assert_eq!(parsed.dismissed_session_ids[1], "sid-2");
    }
}
