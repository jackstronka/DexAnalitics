//! Build per-step **pool fee USD** from local `data/swaps/` (P1 / P1.1).
//! Used by `backtest` and `backtest-optimize` when Dune swaps are not used or empty.

use crate::backtest_engine::StepData;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// `data/` at runtime (cwd) or workspace `data/` during unit tests (stable paths, no `set_current_dir`).
fn repo_data_dir() -> PathBuf {
    #[cfg(test)]
    {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data")
    }
    #[cfg(not(test))]
    {
        Path::new("data").to_path_buf()
    }
}

/// Raw tx counts per step (for timing proxy).
pub fn raw_swap_counts_by_step(
    protocol_dir: &str,
    pool: &str,
    step_data: &[StepData],
    resolution_seconds: u64,
) -> Option<BTreeMap<usize, u64>> {
    if step_data.is_empty() {
        return None;
    }
    let path = Path::new("data")
        .join("swaps")
        .join(protocol_dir)
        .join(pool)
        .join("swaps.jsonl");
    let txt = std::fs::read_to_string(&path).ok()?;
    if txt.trim().is_empty() {
        return None;
    }
    let step_seconds = resolution_seconds.max(1) as i64;
    let start_ts = step_data[0].start_timestamp as i64;
    let mut counts: BTreeMap<usize, u64> = BTreeMap::new();
    for line in txt.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if v.get("err")
            .and_then(|x| x.as_str())
            .map(|s| !s.trim().is_empty() && s != "None")
            .unwrap_or(false)
        {
            continue;
        }
        let Some(ts) = v.get("block_time").and_then(|x| x.as_i64()) else {
            continue;
        };
        let delta = ts - start_ts;
        if delta < 0 {
            continue;
        }
        let idx = (delta / step_seconds) as usize;
        *counts.entry(idx).or_insert(0) += 1;
    }
    if counts.is_empty() {
        None
    } else {
        Some(counts)
    }
}

/// Distribute total candle-implied pool fees across steps by tx-count weights.
pub fn distribute_pool_fees_by_tx_counts(
    counts: &BTreeMap<usize, u64>,
    step_data: &[StepData],
    fee_rate: Decimal,
) -> Option<BTreeMap<usize, Decimal>> {
    let total_count: u64 = counts.values().copied().sum();
    if total_count == 0 {
        return None;
    }
    let total_pool_fees: Decimal = step_data.iter().map(|p| p.step_volume_usd * fee_rate).sum();
    if total_pool_fees <= Decimal::ZERO {
        return None;
    }
    let mut map: BTreeMap<usize, Decimal> = BTreeMap::new();
    let denom = Decimal::from(total_count);
    for (idx, c) in counts {
        if *c == 0 {
            continue;
        }
        let w = Decimal::from(*c) / denom;
        let v = total_pool_fees * w;
        if v > Decimal::ZERO {
            map.insert(*idx, v);
        }
    }
    if map.is_empty() { None } else { Some(map) }
}

/// Decoded vault-delta fees (pool-level USD) per step from `decoded_swaps.jsonl`.
pub fn decoded_swap_fees_usd_by_step(
    protocol_dir: &str,
    pool: &str,
    step_data: &[StepData],
    resolution_seconds: u64,
    token_a_decimals: u8,
    token_b_decimals: u8,
    fee_rate: Decimal,
    require_decode_ok: bool,
) -> Option<BTreeMap<usize, Decimal>> {
    if step_data.is_empty() {
        return None;
    }
    let path = repo_data_dir()
        .join("swaps")
        .join(protocol_dir)
        .join(pool)
        .join("decoded_swaps.jsonl");
    let txt = std::fs::read_to_string(&path).ok()?;
    if txt.trim().is_empty() {
        return None;
    }
    let step_seconds = resolution_seconds.max(1) as i64;
    let start_ts = step_data[0].start_timestamp as i64;
    let pow10 = |d: u32| -> Decimal {
        let mut v = Decimal::ONE;
        for _ in 0..d {
            v *= Decimal::from(10u32);
        }
        v
    };
    let mut map: BTreeMap<usize, Decimal> = BTreeMap::new();
    for line in txt.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if require_decode_ok {
            let st = v.get("decode_status").and_then(|x| x.as_str());
            if !matches!(
                st,
                Some("ok") | Some("ok_traded_event") | Some("ok_swap_event")
            ) {
                continue;
            }
        }
        if !v.get("success").and_then(|x| x.as_bool()).unwrap_or(false) {
            continue;
        }
        let Some(ts) = v.get("block_time").and_then(|x| x.as_i64()) else {
            continue;
        };
        let delta = ts - start_ts;
        if delta < 0 {
            continue;
        }
        let idx = (delta / step_seconds) as usize;
        let Some(step) = step_data.get(idx) else {
            continue;
        };
        let amount_in_raw = v
            .get("amount_in_raw")
            .and_then(|x| x.as_u64())
            .map(Decimal::from)
            .or_else(|| {
                v.get("amount_in_raw")
                    .and_then(|x| x.as_str())
                    .and_then(|s| s.parse::<u128>().ok())
                    .and_then(Decimal::from_u128)
            });
        let Some(amount_in_raw) = amount_in_raw else {
            continue;
        };
        let direction = v.get("direction").and_then(|x| x.as_str()).unwrap_or("");
        let input_is_a = direction == "a_to_b";
        let (decimals_in, price_in_usd) = if input_is_a {
            (token_a_decimals as u32, step.price_usd.value)
        } else {
            (token_b_decimals as u32, step.quote_usd)
        };
        let amount_in_h = amount_in_raw / pow10(decimals_in);
        let fee_usd = amount_in_h * price_in_usd * fee_rate;
        if fee_usd > Decimal::ZERO {
            *map.entry(idx).or_insert(Decimal::ZERO) += fee_usd;
        }
    }
    if map.is_empty() { None } else { Some(map) }
}

/// Prefer decoded swap fees; if missing/empty, use raw-swap tx-count timing proxy.
pub fn build_local_pool_fees_usd(
    protocol_dir: &str,
    pool: &str,
    step_data: &[StepData],
    resolution_seconds: u64,
    token_a_decimals: u8,
    token_b_decimals: u8,
    fee_rate: Decimal,
    require_decode_ok: bool,
) -> Option<BTreeMap<usize, Decimal>> {
    let decoded_map = decoded_swap_fees_usd_by_step(
        protocol_dir,
        pool,
        step_data,
        resolution_seconds,
        token_a_decimals,
        token_b_decimals,
        fee_rate,
        require_decode_ok,
    );

    let counts = raw_swap_counts_by_step(protocol_dir, pool, step_data, resolution_seconds);
    let timing_map = if let Some(counts) = counts {
        distribute_pool_fees_by_tx_counts(&counts, step_data, fee_rate)
    } else {
        None
    };

    match (decoded_map, timing_map) {
        (None, None) => None,
        (Some(decoded), None) => Some(decoded),
        (None, Some(timing)) => Some(timing),
        (Some(decoded), Some(mut timing)) => {
            // Prefer decoded values on buckets where we have them,
            // fallback to timing proxy for missing buckets.
            for (idx, v) in decoded {
                timing.insert(idx, v);
            }
            if timing.is_empty() {
                None
            } else {
                Some(timing)
            }
        }
    }
}

#[cfg(test)]
mod regression_tests {
    //! C3: synthetic decoded fixture + step grid -> expect non-empty local fee map from strict decoded rows.

    use super::*;
    use crate::backtest_engine::StepData;
    use clmm_lp_domain::prelude::Price;

    const STEP_BASE_TS: u64 = 1_773_957_000;

    fn synthetic_steps(n: usize) -> Vec<StepData> {
        let vol = Decimal::from(10_000u64);
        (0..n)
            .map(|i| StepData {
                price_usd: Price::new(Decimal::from(130)),
                price_ab: Price::new(Decimal::ONE),
                step_volume_usd: vol,
                quote_usd: Decimal::ONE,
                lp_share: Decimal::new(1, 3),
                pool_liquidity_active: None,
                start_timestamp: STEP_BASE_TS + (i as u64) * 3600,
            })
            .collect()
    }

    fn write_synthetic_decoded_fixture(protocol: &str, pool: &str) {
        let dir = repo_data_dir().join("swaps").join(protocol).join(pool);
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("decoded_swaps.jsonl");

        // Two valid decoded rows in separate buckets + one invalid row that strict mode must ignore.
        let rows = vec![
            serde_json::json!({
                "block_time": STEP_BASE_TS as i64 + 60,
                "success": true,
                "decode_status": "ok_traded_event",
                "amount_in_raw": "1000000000",
                "direction": "a_to_b"
            }),
            serde_json::json!({
                "block_time": STEP_BASE_TS as i64 + 3700,
                "success": true,
                "decode_status": "ok",
                "amount_in_raw": "2500000",
                "direction": "b_to_a"
            }),
            serde_json::json!({
                "block_time": STEP_BASE_TS as i64 + 120,
                "success": true,
                "decode_status": "no_vault_change",
                "amount_in_raw": "1000000000",
                "direction": "a_to_b"
            }),
        ];
        let body = rows
            .into_iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(path, format!("{body}\n")).expect("write synthetic decoded_swaps fixture");
    }

    #[test]
    fn build_local_pool_fees_uses_decoded_swaps_when_strict_ok() {
        let pool = format!(
            "test_pool_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        );
        write_synthetic_decoded_fixture("orca", &pool);

        let steps = synthetic_steps(48);
        let fee_rate = Decimal::new(3, 3);
        let m = build_local_pool_fees_usd("orca", &pool, &steps, 3600, 9, 6, fee_rate, true)
            .expect("expected non-empty local pool fees from fixture decoded_swaps.jsonl");

        let sum: Decimal = m.values().copied().sum();
        assert!(
            sum > Decimal::ZERO,
            "fee map should contain positive USD fees, got {:?}",
            m
        );
    }
}
