//! Materialize rolling time-window slices of Orca `snapshots.jsonl` for faster backtests.
//!
//! Output layout: `data/backtest-snapshot-cache/orca/<POOL>/window_h24.jsonl`, `window_d7.jsonl`, …
//! Use `backtest --price-path-source snapshots --prepared-snapshot-window h24` to read a slice
//! (still intersected with `--hours` / date window).

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::commands::snapshot_price_path::resolve_snapshot_jsonl_path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrcaPoolMeta {
    pub pool_address: String,
    pub token_mint_a: String,
    pub token_mint_b: String,
    pub token_mint_a_decimals: u8,
    pub token_mint_b_decimals: u8,
    pub tick_spacing: Option<u16>,
    pub protocol_fee_rate_bps: Option<u16>,
    pub fee_rate_raw: Option<u16>,
    pub generated_at_utc: String,
    pub snapshots_suffix: Option<String>,
}

/// Default Orca pools (STARTUP.md curated): SOL/USDC, whETH/SOL.
const DEFAULT_ORCA_POOLS: &[&str] = &[
    "Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE",
    "HktfL7iwGKT5QHjywQkcDnZXScoh811k7akrMZJkCcEF",
];

/// Path to a prepared window file (`label` e.g. `h24`, `d7`).
pub fn orca_prepared_jsonl_path(pool_address: &str, label: &str) -> PathBuf {
    Path::new("data")
        .join("backtest-snapshot-cache")
        .join("orca")
        .join(pool_address.trim())
        .join(format!("window_{}.jsonl", label.trim()))
}

/// Same as [`orca_prepared_jsonl_path`] but for a snapshot variant (suffix).
///
/// `suffix=None` => legacy path under `data/backtest-snapshot-cache/orca/...`.
/// `suffix=Some("5m")` => `data/backtest-snapshot-cache/orca_5m/...`.
pub fn orca_prepared_jsonl_path_with_suffix(
    pool_address: &str,
    label: &str,
    suffix: Option<&str>,
) -> PathBuf {
    let suffix_clean = suffix
        .map(|s| s.trim().trim_start_matches('_'))
        .filter(|s| !s.is_empty());

    let orca_dir = match suffix_clean {
        Some(s) => format!("orca_{}", s),
        None => "orca".to_string(),
    };

    Path::new("data")
        .join("backtest-snapshot-cache")
        .join(orca_dir)
        .join(pool_address.trim())
        .join(format!("window_{}.jsonl", label.trim()))
}

fn source_jsonl_path(pool: &str, suffix: Option<&str>) -> PathBuf {
    let suffix_clean = suffix
        .map(|s| s.trim().trim_start_matches('_'))
        .filter(|s| !s.is_empty());

    let file = match suffix_clean {
        Some(s) => format!("snapshots_{}.jsonl", s),
        None => "snapshots.jsonl".to_string(),
    };

    Path::new("data")
        .join("pool-snapshots")
        .join("orca")
        .join(pool.trim())
        .join(file)
}

fn source_jsonl_resolved(pool: &str, suffix: Option<&str>) -> PathBuf {
    resolve_snapshot_jsonl_path(&source_jsonl_path(pool, suffix))
}

fn ts_from_jsonl_line(line: &str) -> Option<i64> {
    let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    let s = v.get("ts_utc")?.as_str()?;
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.timestamp())
}

#[derive(Default, Serialize)]
struct WindowMeta {
    lines: usize,
    min_ts_unix: Option<i64>,
    max_ts_unix: Option<i64>,
}

#[derive(Serialize)]
struct PoolManifest {
    windows: BTreeMap<String, WindowMeta>,
}

#[derive(Serialize)]
struct RootManifest {
    generated_at_utc: String,
    now_unix: i64,
    pools: BTreeMap<String, PoolManifest>,
}

fn parse_u64_list(s: &str) -> Result<Vec<u64>> {
    let mut out = Vec::new();
    for part in s.split(',') {
        let p = part.trim();
        if p.is_empty() {
            continue;
        }
        out.push(
            p.parse::<u64>()
                .with_context(|| format!("invalid unsigned integer in list: {:?}", p))?,
        );
    }
    if out.is_empty() {
        bail!("window list is empty");
    }
    Ok(out)
}

/// One-shot: slice each pool's Orca JSONL into `data/backtest-snapshot-cache/orca/<pool>/window_*.jsonl`.
pub async fn run_snapshot_backtest_prep(
    pools: Option<Vec<String>>,
    windows_hours: &str,
    windows_days: &str,
    snapshots_suffix: Option<&str>,
) -> Result<()> {
    let pool_list: Vec<String> = match pools {
        Some(v) if !v.is_empty() => v.into_iter().map(|s| s.trim().to_string()).collect(),
        _ => DEFAULT_ORCA_POOLS.iter().map(|s| (*s).to_string()).collect(),
    };

    let hours = parse_u64_list(windows_hours)?;
    let days = parse_u64_list(windows_days)?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| anyhow::anyhow!("system time: {e}"))?
        .as_secs() as i64;

    let mut root = RootManifest {
        generated_at_utc: chrono::Utc::now().to_rfc3339(),
        now_unix: now,
        pools: BTreeMap::new(),
    };

    for pool in &pool_list {
        let src = source_jsonl_resolved(pool, snapshots_suffix);
        if !src.exists() {
            eprintln!(
                "skip {} — source snapshot missing: {}",
                pool,
                src.display()
            );
            continue;
        }

        let txt = std::fs::read_to_string(&src)
            .with_context(|| format!("read {}", src.display()))?;
        let lines: Vec<&str> = txt.lines().map(str::trim).filter(|l| !l.is_empty()).collect();

        let suffix_clean = snapshots_suffix
            .map(|s| s.trim().trim_start_matches('_'))
            .filter(|s| !s.is_empty());
        let orca_dir = match suffix_clean {
            Some(s) => format!("orca_{}", s),
            None => "orca".to_string(),
        };

        let out_dir = Path::new("data")
            .join("backtest-snapshot-cache")
            .join(orca_dir)
            .join(pool.trim());
        std::fs::create_dir_all(&out_dir)
            .with_context(|| format!("mkdir {}", out_dir.display()))?;

        // Create a side-car meta cache so that later backtests can run with RPC disabled.
        // This meta is intentionally small: decimals + tick spacing + basic fee params.
        let rpc = std::sync::Arc::new(clmm_lp_protocols::rpc::RpcProvider::mainnet());
        let reader = clmm_lp_protocols::orca::pool_reader::WhirlpoolReader::new(rpc.clone());
        let state = reader.get_pool_state(pool.trim()).await?;

        use crate::engine::token_meta::fetch_mint_decimals;
        let token_mint_a = state.token_mint_a.to_string();
        let token_mint_b = state.token_mint_b.to_string();

        let token_mint_a_decimals = fetch_mint_decimals(&rpc, &token_mint_a)
            .await
            .unwrap_or(9);
        let token_mint_b_decimals = fetch_mint_decimals(&rpc, &token_mint_b)
            .await
            .unwrap_or(9);

        let meta = OrcaPoolMeta {
            pool_address: pool.trim().to_string(),
            token_mint_a,
            token_mint_b,
            token_mint_a_decimals,
            token_mint_b_decimals,
            tick_spacing: Some(state.tick_spacing),
            protocol_fee_rate_bps: Some(state.protocol_fee_rate_bps),
            fee_rate_raw: Some(state.fee_rate_bps),
            generated_at_utc: chrono::Utc::now().to_rfc3339(),
            snapshots_suffix: snapshots_suffix.map(|s| s.trim().to_string()),
        };
        let meta_path = out_dir.join("pool_meta.json");
        std::fs::write(&meta_path, serde_json::to_string_pretty(&meta)?)?;

        let mut pool_entry = PoolManifest {
            windows: BTreeMap::new(),
        };

        for h in &hours {
            let label = format!("h{}", h);
            let start = now.saturating_sub((*h as i64).saturating_mul(3600));
            let meta = write_window(&lines, &out_dir, &label, start, now)?;
            pool_entry.windows.insert(label, meta);
        }
        for d in &days {
            let label = format!("d{}", d);
            let start = now.saturating_sub((*d as i64).saturating_mul(86400));
            let meta = write_window(&lines, &out_dir, &label, start, now)?;
            pool_entry.windows.insert(label, meta);
        }

        root.pools.insert(pool.clone(), pool_entry);
        println!(
            "snapshot-backtest-prep: {} → {} (windows written under {})",
            pool,
            src.display(),
            out_dir.display()
        );
    }

    let manifest_file = match snapshots_suffix
        .map(|s| s.trim().trim_start_matches('_'))
        .filter(|s| !s.is_empty())
    {
        Some(s) => format!("manifest_{}.json", s),
        None => "manifest.json".to_string(),
    };

    let manifest_path = Path::new("data")
        .join("backtest-snapshot-cache")
        .join(manifest_file);
    if let Some(parent) = manifest_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(&root)?;
    std::fs::write(&manifest_path, json)?;
    println!("Wrote {}", manifest_path.display());

    Ok(())
}

fn write_window(
    lines: &[&str],
    out_dir: &Path,
    label: &str,
    start_ts: i64,
    end_ts: i64,
) -> Result<WindowMeta> {
    let path = out_dir.join(format!("window_{}.jsonl", label));
    let mut file = std::fs::File::create(&path).with_context(|| format!("create {}", path.display()))?;
    let mut count = 0usize;
    let mut min_ts: Option<i64> = None;
    let mut max_ts: Option<i64> = None;

    for line in lines {
        let Some(ts) = ts_from_jsonl_line(line) else {
            continue;
        };
        if ts < start_ts || ts >= end_ts {
            continue;
        }
        writeln!(file, "{line}")?;
        count += 1;
        min_ts = Some(match min_ts {
            None => ts,
            Some(m) => m.min(ts),
        });
        max_ts = Some(match max_ts {
            None => ts,
            Some(m) => m.max(ts),
        });
    }

    Ok(WindowMeta {
        lines: count,
        min_ts_unix: min_ts,
        max_ts_unix: max_ts,
    })
}
