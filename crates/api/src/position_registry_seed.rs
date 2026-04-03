//! Seed the in-memory monitor from the append-only position registry.
//!
//! The API dashboard endpoints (`GET /positions`, `/analytics/portfolio`) operate on the
//! in-memory `PositionMonitor`. After API restarts, the monitor starts empty.
//! We replay `data/positions/registry.jsonl` to re-add currently open positions.

use clmm_lp_protocols::ledger::position_registry::registry_path;
use solana_sdk::pubkey::Pubkey;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use tracing::{info, warn};

#[derive(Debug, Clone)]
struct RegistryEvent {
    event: String,
    position_pubkey: String,
}

/// Pubkeys still marked open in `registry.jsonl` (last event per key wins).
#[must_use]
pub fn registry_open_position_pubkeys() -> Vec<Pubkey> {
    replay_registry_open_positions()
}

fn replay_registry_open_positions() -> Vec<Pubkey> {
    let path = registry_path();
    let Ok(file) = File::open(&path) else {
        info!(path = %path.display(), "position registry: file missing; monitor seed skipped");
        return Vec::new();
    };
    let reader = BufReader::new(file);

    let mut last: HashMap<String, RegistryEvent> = HashMap::new();
    for line in reader.lines().filter_map(Result::ok) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let event = v
            .get("event")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let position_pubkey = v
            .get("position_pubkey")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        if position_pubkey.is_empty() {
            continue;
        }
        if event != "registry_open" && event != "registry_close" {
            continue;
        }
        last.insert(
            position_pubkey.clone(),
            RegistryEvent {
                event,
                position_pubkey,
            },
        );
    }

    let mut out = Vec::new();
    for e in last.values() {
        if e.event == "registry_open" {
            match Pubkey::try_from(e.position_pubkey.as_str()) {
                Ok(pk) => out.push(pk),
                Err(_) => {
                    warn!(position = %e.position_pubkey, "position registry: invalid pubkey");
                }
            }
        }
    }
    out
}

/// Best-effort: replay registry and re-add open positions into the monitor.
pub async fn seed_monitor_from_registry(
    monitor: std::sync::Arc<clmm_lp_execution::prelude::PositionMonitor>,
) {
    let open = replay_registry_open_positions();
    if open.is_empty() {
        return;
    }

    let mut ok = 0usize;
    for pk in open {
        if let Err(e) = monitor.add_position(&pk.to_string()).await {
            warn!(position = %pk, error = %e, "monitor seed: add_position failed");
        } else {
            ok += 1;
        }
    }
    info!(count = ok, "monitor seeded from position registry");
}
