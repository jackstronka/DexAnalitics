//! Aggregate `bot_collect_fees` rows from `orca_position_lifecycle.jsonl` (same path as CLI).

use crate::models::FeesCollectedFromLedger;
use clmm_lp_protocols::ledger::tx_lifecycle::ledger_read_path;
use rust_decimal::Decimal;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::str::FromStr;

fn decimal_from_value(v: &serde_json::Value) -> Option<Decimal> {
    if let Some(s) = v.as_str() {
        return Decimal::from_str(s.trim()).ok();
    }
    if let Some(n) = v.as_u64() {
        return Some(Decimal::from(n));
    }
    if let Some(n) = v.as_i64() {
        return Some(Decimal::from(n));
    }
    if let Some(f) = v.as_f64() {
        return Decimal::from_str(&format!("{f:.18}")).ok();
    }
    None
}

/// Sum fee credits across all `bot_collect_fees` events in the lifecycle ledger file.
#[must_use]
pub fn aggregate_bot_collect_fees_totals() -> FeesCollectedFromLedger {
    let path = ledger_read_path();
    let file = match File::open(&path) {
        Ok(f) => f,
        Err(_) => {
            return FeesCollectedFromLedger {
                file_missing: true,
                collect_events: 0,
                sum_token_a_ui: None,
                sum_token_b_ui: None,
            };
        }
    };

    let reader = BufReader::new(file);
    let mut collect_events = 0u32;
    let mut sum_a = Decimal::ZERO;
    let mut sum_b = Decimal::ZERO;
    let mut any_a = false;
    let mut any_b = false;

    for line in reader.lines().map_while(Result::ok) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let event = v.get("event").and_then(|x| x.as_str()).unwrap_or("");
        if event != "bot_collect_fees" {
            continue;
        }
        collect_events = collect_events.saturating_add(1);
        if let Some(d) = v
            .get("fee_payer_token_a_delta_ui")
            .and_then(decimal_from_value)
        {
            sum_a += d;
            any_a = true;
        }
        if let Some(d) = v
            .get("fee_payer_token_b_delta_ui")
            .and_then(decimal_from_value)
        {
            sum_b += d;
            any_b = true;
        }
    }

    FeesCollectedFromLedger {
        file_missing: false,
        collect_events,
        sum_token_a_ui: any_a.then_some(sum_a),
        sum_token_b_ui: any_b.then_some(sum_b),
    }
}
