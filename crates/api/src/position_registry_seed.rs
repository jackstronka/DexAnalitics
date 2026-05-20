//! Seed the in-memory monitor from the append-only position registry.
//!
//! The API dashboard endpoints (`GET /positions`, `/analytics/portfolio`) operate on the
//! in-memory `PositionMonitor`. After API restarts, the monitor starts empty.
//! We replay `data/positions/registry.jsonl` to re-add currently open positions.

use clmm_lp_protocols::ledger::position_registry::registry_path;
use clmm_lp_protocols::prelude::RpcProvider;
use solana_sdk::pubkey::Pubkey;
use std::sync::Arc;
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

/// Latest open/close state from `registry.jsonl` (last event per key wins).
///
/// `true` = open, `false` = closed.
#[must_use]
pub fn registry_position_open_map() -> HashMap<Pubkey, bool> {
    replay_registry_open_map()
}

fn replay_registry_open_positions() -> Vec<Pubkey> {
    let path = registry_path();
    let Ok(file) = File::open(&path) else {
        info!(path = %path.display(), "position registry: file missing; monitor seed skipped");
        return Vec::new();
    };
    let reader = BufReader::new(file);

    let mut last: HashMap<String, RegistryEvent> = HashMap::new();
    for line in reader.lines().map_while(Result::ok) {
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

fn replay_registry_open_map() -> HashMap<Pubkey, bool> {
    let path = registry_path();
    let Ok(file) = File::open(&path) else {
        return HashMap::new();
    };
    let reader = BufReader::new(file);

    let mut last: HashMap<String, String> = HashMap::new(); // pos_str -> event
    for line in reader.lines().map_while(Result::ok) {
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
        last.insert(position_pubkey, event);
    }

    let mut out: HashMap<Pubkey, bool> = HashMap::new();
    for (pos_s, ev) in last {
        match Pubkey::try_from(pos_s.as_str()) {
            Ok(pk) => {
                out.insert(pk, ev == "registry_open");
            }
            Err(_) => {
                warn!(position = %pos_s, "position registry: invalid pubkey");
            }
        }
    }
    out
}

/// Best-effort: replay registry and re-add **on-chain** open positions into the monitor.
pub async fn seed_monitor_from_registry(
    monitor: std::sync::Arc<clmm_lp_execution::prelude::PositionMonitor>,
    provider: Arc<RpcProvider>,
) {
    use crate::services::position_on_chain_cache::api_error_is_account_absent;
    use crate::services::position_valuation::monitored_position_from_chain;
    use crate::services::registry_stale_reconcile::try_reconcile_stale_registry_open;

    let open = replay_registry_open_positions();
    if open.is_empty() {
        return;
    }

    let mut ok = 0usize;
    let mut stale = 0usize;
    for pk in open {
        match monitored_position_from_chain(provider.clone(), &pk).await {
            Ok(_) => {
                if monitor.add_position(&pk.to_string()).await.is_ok() {
                    ok += 1;
                } else {
                    warn!(position = %pk, "monitor seed: add_position failed after on-chain ok");
                }
            }
            Err(e) if api_error_is_account_absent(&e) => {
                stale += 1;
                let _ = try_reconcile_stale_registry_open(&provider, &pk).await;
            }
            Err(e) => {
                warn!(position = %pk, error = %e, "monitor seed: on-chain check failed");
            }
        }
    }
    info!(
        count = ok,
        stale_reconciled = stale,
        "monitor seeded from position registry (on-chain confirmed)"
    );
}
