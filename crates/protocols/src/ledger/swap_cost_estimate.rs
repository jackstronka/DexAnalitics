//! Heuristic swap **network fee** (`meta.fee`) estimates from local lifecycle JSONL — no paid data feeds.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use super::tx_lifecycle::ledger_path;

/// Conservative default when the ledger has no swap rows (typical base fee + small priority band).
pub const DEFAULT_ESTIMATED_SWAP_NETWORK_FEE_LAMPORTS: u64 = 10_000;

/// Median `tx_fee_lamports` for rows that correspond to Orca `swap_exact_in`, optionally filtered by pool.
///
/// Returns `(median, sample_count)`.
#[must_use]
pub fn median_historical_swap_network_fee_lamports(
    pool_address: Option<&str>,
) -> (Option<u64>, usize) {
    median_from_path(&ledger_path(), pool_address)
}

fn median_from_path(path: &Path, pool_address: Option<&str>) -> (Option<u64>, usize) {
    let Ok(file) = File::open(path) else {
        return (None, 0);
    };
    let reader = BufReader::new(file);
    let mut fees = Vec::new();
    for line in reader.lines().filter_map(std::result::Result::ok) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let op = v.get("operation").and_then(|x| x.as_str());
        let evt = v.get("event").and_then(|x| x.as_str());
        let is_swap = matches!(op, Some("swap_exact_in"))
            || evt == Some("bot_swap_exact_in")
            || (evt == Some("bot_orca_tx") && op == Some("swap_exact_in"));
        if !is_swap {
            continue;
        }
        if let Some(pool) = pool_address {
            let pa = v.get("pool_address").and_then(|x| x.as_str());
            if pa != Some(pool) {
                continue;
            }
        }
        if let Some(fee) = v.get("tx_fee_lamports").and_then(|x| x.as_u64()) {
            fees.push(fee);
        }
    }
    let n = fees.len();
    if n == 0 {
        return (None, 0);
    }
    fees.sort_unstable();
    (Some(fees[n / 2]), n)
}
