//! Backtest endpoints (spawn CLI backtest runs).

use crate::error::{ApiError, ApiResult};
use crate::models::{
    BacktestFromClosedPositionRequest, BacktestFromOpenPositionRequest, BacktestFullJobResponse,
    BacktestFullJobStatusResponse, BacktestFullMetricRow, BacktestFullRequest,
    BacktestFullWindowResult, BacktestJobResponse, BacktestJobStatusResponse,
    BacktestStrategyCatalogEntry, BacktestStrategyCatalogResponse,
};
use crate::services::price_fetch::fetch_mint_prices_usd;
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use clmm_lp_data::repositories::Database;
use clmm_lp_domain::math::price_tick::tick_to_price;
use clmm_lp_protocols::ledger::position_registry::registry_path;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use serde_json::Value;
use sqlx::Row;
use std::collections::{BTreeSet, HashMap};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::Stdio;
use std::str::FromStr;
use std::sync::{LazyLock, Mutex};
use tokio::process::Command;
use tokio::sync::RwLock;
use uuid::Uuid;

static JOBS: LazyLock<RwLock<HashMap<String, BacktestJobResponse>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));
static FULL_JOBS: LazyLock<RwLock<HashMap<String, BacktestFullJobResponse>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Cache: `(exe path | mtime secs)` -> whether `backtest-optimize --help` lists `--include-strategy-families`.
/// Key includes modification time so a rebuilt `clmm-lp-cli` is re-probed without restarting API.
static CLI_SUPPORTS_INCLUDE_STRATEGY_FAMILIES: LazyLock<Mutex<HashMap<String, bool>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn clmm_lp_cli_filename() -> &'static str {
    if cfg!(windows) {
        "clmm-lp-cli.exe"
    } else {
        "clmm-lp-cli"
    }
}

/// `target/{debug|release}/clmm-lp-cli` next to `clmm-lp-api` (typical `cargo run -p clmm-lp-api`).
fn try_clmm_lp_cli_next_to_api_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let p = dir.join(clmm_lp_cli_filename());
    p.is_file().then_some(p)
}

/// Resolve CLI for `POST /backtests/from-closed-position` when `clmm-lp-cli` is not on `PATH`.
///
/// Prefers **`target/release`** over **`target/debug`** when both exist under `CLMM_REPO_ROOT`, so
/// long-lived `cargo run` API + freshly built release CLI (common for snapshot jobs) still works.
fn resolve_clmm_lp_cli_path(repo_root: Option<&str>) -> Result<PathBuf, String> {
    if let Ok(p) = std::env::var("CLMM_LP_CLI_PATH") {
        let pb = PathBuf::from(p.trim());
        if pb.as_os_str().is_empty() {
            return Err("CLMM_LP_CLI_PATH is set but empty".to_string());
        }
        if pb.is_file() {
            return Ok(pb);
        }
        return Err(format!(
            "CLMM_LP_CLI_PATH points to a missing file: {}",
            pb.display()
        ));
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(root) = repo_root.map(str::trim).filter(|s| !s.is_empty()) {
        let root = std::path::Path::new(root);
        for profile in ["release", "debug"] {
            candidates.push(
                root.join("target")
                    .join(profile)
                    .join(clmm_lp_cli_filename()),
            );
        }
        // Same layout as tools/Start-ClmmApi-8081.ps1: `cargo run --target-dir target-dev-api`
        if let Ok(td) = std::env::var("CLMM_API_TARGET_DIR") {
            let rel = td.trim();
            if !rel.is_empty() {
                for profile in ["release", "debug"] {
                    candidates.push(root.join(rel).join(profile).join(clmm_lp_cli_filename()));
                }
            }
        }
    }
    if let Ok(td) = std::env::var("CARGO_TARGET_DIR") {
        let td = std::path::Path::new(td.trim());
        for profile in ["release", "debug"] {
            candidates.push(td.join(profile).join(clmm_lp_cli_filename()));
        }
    }
    if let Some(p) = try_clmm_lp_cli_next_to_api_exe() {
        candidates.push(p);
    }
    for p in candidates {
        if p.is_file() {
            return Ok(p);
        }
    }
    Err(
        "clmm-lp-cli not found (not on PATH). Fix: (1) `cargo build -p clmm-lp-cli` so the binary \
         lives next to clmm-lp-api in target/debug or target/release, or (2) set CLMM_LP_CLI_PATH \
         to the full path of clmm-lp-cli (.exe on Windows), or (3) set CLMM_REPO_ROOT to the repo \
         root so target/release|debug is searched (release preferred when both exist), or (4) add \
         the CLI install location to PATH."
            .to_string(),
    )
}

fn probe_backtest_optimize_help_has_include_strategy_families(cli_path: &std::path::Path) -> bool {
    std::process::Command::new(cli_path)
        .args(["backtest-optimize", "--help"])
        .output()
        .map(|o| {
            let combined = format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            );
            combined.contains("include-strategy-families")
        })
        .unwrap_or(false)
}

fn cli_supports_include_strategy_families(cli_path: &std::path::Path) -> bool {
    let cache_key = match std::fs::metadata(cli_path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
    {
        Some(d) => format!("{}|{}", cli_path.to_string_lossy(), d.as_secs()),
        None => {
            return probe_backtest_optimize_help_has_include_strategy_families(cli_path);
        }
    };
    let mut guard = CLI_SUPPORTS_INCLUDE_STRATEGY_FAMILIES.lock().unwrap();
    if let Some(v) = guard.get(&cache_key) {
        return *v;
    }
    let supported = probe_backtest_optimize_help_has_include_strategy_families(cli_path);
    guard.insert(cache_key, supported);
    supported
}

fn is_full_strategy_catalog(include_ids: &std::collections::HashSet<String>) -> bool {
    let catalog = strategy_catalog();
    if catalog.len() != include_ids.len() {
        return false;
    }
    catalog
        .iter()
        .all(|e| include_ids.contains(&e.id.to_ascii_lowercase()))
}

type RegistryLookup = (
    Option<String>,
    Option<String>,
    String,
    Option<String>,
    Option<Value>,
);

fn registry_lookup(position_pubkey: &str) -> Option<RegistryLookup> {
    let p = registry_path();
    let f = std::fs::File::open(p).ok()?;
    let r = BufReader::new(f);

    let mut opened_ts: Option<String> = None;
    let mut closed_ts: Option<String> = None;
    let mut pool: Option<String> = None;
    let mut sid: Option<String> = None;
    let mut details: Option<Value> = None;

    for line in r.lines().map_while(Result::ok) {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(t) else {
            continue;
        };
        let ev = v.get("event").and_then(|x| x.as_str()).unwrap_or("").trim();
        if ev != "registry_open" && ev != "registry_close" {
            continue;
        }
        let pos = v
            .get("position_pubkey")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .trim();
        if pos != position_pubkey {
            continue;
        }
        pool = v
            .get("pool_address")
            .and_then(|x| x.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or(pool);
        sid = v
            .get("rebalance_session_id")
            .and_then(|x| x.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or(sid);
        details = v.get("details").cloned().or(details);
        let ts = v
            .get("ts_utc")
            .and_then(|x| x.as_str())
            .map(|s| s.trim().to_string());
        match ev {
            "registry_open" => opened_ts = ts.or(opened_ts),
            "registry_close" => closed_ts = ts.or(closed_ts),
            _ => {}
        }
    }

    let pool = pool?;
    Some((opened_ts, closed_ts, pool, sid, details))
}

fn iso_date(ts_utc: &str) -> Option<String> {
    let t = ts_utc.trim();
    if t.len() >= 10 {
        return Some(t[..10].to_string());
    }
    None
}

/// `clmm-lp-cli backtest` parses `YYYY-MM-DD` as 00:00 UTC; Orca snapshot rows are kept when `start_ts <= ts < end_ts`.
/// The registry only gives the **calendar day** of close — that entire day must fall inside the window, so the CLI `--end-date`
/// must be the **following** calendar day (exclusive upper bound).
fn next_calendar_day_utc(yyyy_mm_dd: &str) -> Option<String> {
    use chrono::{Duration, NaiveDate};
    let d = NaiveDate::parse_from_str(yyyy_mm_dd.trim(), "%Y-%m-%d").ok()?;
    d.checked_add_signed(Duration::days(1))
        .map(|d| d.format("%Y-%m-%d").to_string())
}

fn json_to_decimal(v: &Value) -> Option<Decimal> {
    match v {
        Value::String(s) => Decimal::from_str(s.trim()).ok(),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Some(Decimal::from(i))
            } else if let Some(u) = n.as_u64() {
                Some(Decimal::from(u))
            } else {
                n.as_f64().and_then(Decimal::from_f64_retain)
            }
        }
        _ => None,
    }
}

#[derive(Clone)]
struct CuratedBacktestPool {
    id: &'static str,
    label: &'static str,
    protocol: &'static str,
    pool_address: &'static str,
    symbol_a: &'static str,
    mint_a: &'static str,
    symbol_b: &'static str,
    mint_b: &'static str,
}

fn curated_backtest_pools() -> Vec<CuratedBacktestPool> {
    vec![
        CuratedBacktestPool {
            id: "ORCA_SOL_USDC",
            label: "Orca SOL/USDC 0.04%",
            protocol: "orca",
            pool_address: "Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE",
            symbol_a: "SOL",
            mint_a: "So11111111111111111111111111111111111111112",
            symbol_b: "USDC",
            mint_b: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
        },
        CuratedBacktestPool {
            id: "ORCA_WHETH_SOL",
            label: "Orca whETH/SOL 0.05%",
            protocol: "orca",
            pool_address: "HktfL7iwGKT5QHjywQkcDnZXScoh811k7akrMZJkCcEF",
            symbol_a: "SOL",
            mint_a: "So11111111111111111111111111111111111111112",
            symbol_b: "WHETH",
            mint_b: "7vfCXTUXx5WJV5JADk17DUJ4ksgau7utNKj4b963voxs",
        },
        CuratedBacktestPool {
            id: "ORCA_CBBTC_USDC",
            label: "Orca cbBTC/USDC 0.04%",
            protocol: "orca",
            pool_address: "HxA6SKW5qA4o12fjVgTpXdq2YnZ5Zv1s7SB4FFomsyLM",
            symbol_a: "CBBTC",
            mint_a: "cbbtcf3aa214zXHbiAZQwf4122FBYbraNdFqgw4iMij",
            symbol_b: "USDC",
            mint_b: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
        },
        CuratedBacktestPool {
            id: "ORCA_CBBTC_WBTC",
            label: "Orca cbBTC/WBTC 0.01%",
            protocol: "orca",
            pool_address: "4v8ufj8Hj7UvFgtofQJAtzUud5xomwZfEqfCTHZ4wM72",
            symbol_a: "CBBTC",
            mint_a: "cbbtcf3aa214zXHbiAZQwf4122FBYbraNdFqgw4iMij",
            symbol_b: "WBTC",
            mint_b: "3NZ9JMVBmGAqocybic2c7LQCJScmgsAZ6vQqTDzcqmJh",
        },
        CuratedBacktestPool {
            id: "RAYDIUM_SOL_USDT",
            label: "Raydium SOL/USDT 0.01%",
            protocol: "raydium",
            pool_address: "3nMFwZXwY1s1M5s8vYAHqd4wGs4iSxXE4LRoUMMYqEgF",
            symbol_a: "SOL",
            mint_a: "So11111111111111111111111111111111111111112",
            symbol_b: "USDT",
            mint_b: "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB",
        },
        CuratedBacktestPool {
            id: "METEORA_SOL_USDC_S1",
            label: "Meteora SOL/USDC Step1",
            protocol: "meteora",
            pool_address: "HTvjzsfX3yU6BUodCjZ5vZkUrAxMDTrBs3CJaq43ashR",
            symbol_a: "SOL",
            mint_a: "So11111111111111111111111111111111111111112",
            symbol_b: "USDC",
            mint_b: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
        },
        CuratedBacktestPool {
            id: "METEORA_SOL_USDC_S4",
            label: "Meteora SOL/USDC Step4",
            protocol: "meteora",
            pool_address: "5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6",
            symbol_a: "SOL",
            mint_a: "So11111111111111111111111111111111111111112",
            symbol_b: "USDC",
            mint_b: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
        },
        CuratedBacktestPool {
            id: "METEORA_SOL_USDC_S10",
            label: "Meteora SOL/USDC Step10",
            protocol: "meteora",
            pool_address: "BGm1tav58oGcsQJehL9WXBFXF7D27vZsKefj4xJKD5Y",
            symbol_a: "SOL",
            mint_a: "So11111111111111111111111111111111111111112",
            symbol_b: "USDC",
            mint_b: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
        },
    ]
}

fn strategy_catalog() -> Vec<BacktestStrategyCatalogEntry> {
    vec![
        BacktestStrategyCatalogEntry {
            id: "static".to_string(),
            label: "Static".to_string(),
            parameters: vec!["width_pct".to_string()],
        },
        BacktestStrategyCatalogEntry {
            id: "oor_recenter".to_string(),
            label: "Out-of-range recenter".to_string(),
            parameters: vec!["width_pct".to_string()],
        },
        BacktestStrategyCatalogEntry {
            id: "threshold".to_string(),
            label: "Threshold".to_string(),
            parameters: vec!["threshold_pct".to_string(), "width_pct".to_string()],
        },
        BacktestStrategyCatalogEntry {
            id: "periodic".to_string(),
            label: "Periodic".to_string(),
            parameters: vec!["period_hours".to_string(), "width_pct".to_string()],
        },
        BacktestStrategyCatalogEntry {
            id: "il_limit".to_string(),
            label: "IL limit".to_string(),
            parameters: vec![
                "max_il_pct".to_string(),
                "close_il_pct".to_string(),
                "grace_steps".to_string(),
                "width_pct".to_string(),
            ],
        },
        BacktestStrategyCatalogEntry {
            id: "retouch_shift".to_string(),
            label: "Retouch shift".to_string(),
            parameters: vec!["width_pct".to_string()],
        },
        BacktestStrategyCatalogEntry {
            id: "bollinger".to_string(),
            label: "Bollinger".to_string(),
            parameters: vec!["window".to_string(), "k".to_string(), "rebalance_steps".to_string()],
        },
        BacktestStrategyCatalogEntry {
            id: "last_candle".to_string(),
            label: "Last candle".to_string(),
            parameters: vec![
                "candle_steps/candle_seconds".to_string(),
                "rebalance_steps/rebalance_seconds".to_string(),
            ],
        },
    ]
}

fn parse_optimize_table(stdout: &str) -> Vec<BacktestFullMetricRow> {
    let mut rows = Vec::new();
    for line in stdout.lines() {
        let t = line.trim();
        if !t.starts_with('|') || t.contains("Rank") || t.contains("----") {
            continue;
        }
        let cols: Vec<String> = t
            .trim_matches('|')
            .split('|')
            .map(|s| s.trim().to_string())
            .collect();
        if cols.len() < 18 {
            continue;
        }
        let rank = cols[0].parse::<u32>().ok();
        let strategy = cols[5].clone();
        let lower_usd = cols[1].replace('+', "").parse::<f64>().ok();
        let upper_usd = cols[2].replace('+', "").parse::<f64>().ok();
        let score = cols[6].replace('+', "").parse::<f64>().ok();
        let fees = cols[7].replace('+', "").parse::<f64>().ok();
        let rebals = cols[8].parse::<u32>().ok();
        let pnl = cols[13].replace('+', "").parse::<f64>().ok();
        let vs_hodl = cols[14].replace('+', "").parse::<f64>().ok();
        let tir_pct = cols[16].trim_end_matches('%').parse::<f64>().ok();
        let il_like_pct = cols[17].trim_end_matches('%').parse::<f64>().ok();
        if let (
            Some(rank),
            Some(lower_usd),
            Some(upper_usd),
            Some(score),
            Some(fees),
            Some(rebalances),
            Some(pnl),
            Some(vs_hodl),
            Some(tir_pct),
        ) = (
            rank, lower_usd, upper_usd, score, fees, rebals, pnl, vs_hodl, tir_pct,
        )
        {
            let mid = (lower_usd + upper_usd) / 2.0;
            let width_pct = if mid > 0.0 {
                ((upper_usd - lower_usd) / mid) * 100.0
            } else {
                0.0
            };
            rows.push(BacktestFullMetricRow {
                rank,
                strategy,
                lower_usd,
                upper_usd,
                width_pct,
                score,
                fees,
                rebalances,
                pnl,
                vs_hodl,
                tir_pct,
                il_like_pct,
            });
        }
    }
    rows
}

/// USD spent to open: negative fee-payer deltas on pool legs × prices (same convention as lifecycle summaries).
fn capital_usd_from_mint_deltas(
    mint_deltas: &HashMap<String, Decimal>,
    mint_a: &str,
    mint_b: &str,
    pa: f64,
    pb: f64,
) -> f64 {
    let da = mint_deltas.get(mint_a).cloned().unwrap_or(Decimal::ZERO);
    let db = mint_deltas.get(mint_b).cloned().unwrap_or(Decimal::ZERO);
    let spend_a = (-da).max(Decimal::ZERO);
    let spend_b = (-db).max(Decimal::ZERO);
    let usd = spend_a * Decimal::from_f64_retain(pa).unwrap_or(Decimal::ZERO)
        + spend_b * Decimal::from_f64_retain(pb).unwrap_or(Decimal::ZERO);
    usd.to_f64().unwrap_or(0.0)
}

fn merge_fee_payer_deltas_from_object(
    obj: &serde_json::Map<String, Value>,
    into: &mut HashMap<String, Decimal>,
) {
    for (mint, dv) in obj {
        if let Some(d) = json_to_decimal(dv) {
            *into.entry(mint.clone()).or_insert(Decimal::ZERO) += d;
        }
    }
}

/// Sum `fee_payer_token_deltas` for all ledger lines sharing `rebalance_session_id`.
fn capital_from_ledger_session(
    txt: &str,
    sid: &str,
    mint_a: &str,
    mint_b: &str,
    pa: f64,
    pb: f64,
) -> f64 {
    let mut mint_deltas: HashMap<String, Decimal> = HashMap::new();
    for line in txt.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(t) else {
            continue;
        };
        let line_sid = v
            .get("rebalance_session_id")
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        if line_sid != Some(sid) {
            continue;
        }
        let Some(obj) = v.get("fee_payer_token_deltas").and_then(|x| x.as_object()) else {
            continue;
        };
        merge_fee_payer_deltas_from_object(obj, &mut mint_deltas);
    }
    capital_usd_from_mint_deltas(&mint_deltas, mint_a, mint_b, pa, pb)
}

/// Earliest open row for this position PDA (when session id on registry is missing or does not match JSONL).
fn capital_from_ledger_first_open(
    txt: &str,
    position: &str,
    mint_a: &str,
    mint_b: &str,
    pa: f64,
    pb: f64,
) -> f64 {
    #[derive(Clone)]
    struct Row {
        ts: String,
        deltas: HashMap<String, Decimal>,
    }
    let mut rows: Vec<Row> = Vec::new();
    for line in txt.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(t) else {
            continue;
        };
        let pos = v
            .get("position_pubkey")
            .or_else(|| v.get("position_pda"))
            .and_then(|x| x.as_str())
            .map(str::trim)
            .unwrap_or("");
        if pos != position {
            continue;
        }
        let ev = v
            .get("event")
            .and_then(|x| x.as_str())
            .map(str::trim)
            .unwrap_or("");
        let is_open = matches!(
            ev,
            "bot_open_position" | "bot_open_position_full_range" | "position_open"
        );
        if !is_open {
            continue;
        }
        let Some(obj) = v.get("fee_payer_token_deltas").and_then(|x| x.as_object()) else {
            continue;
        };
        let mut deltas: HashMap<String, Decimal> = HashMap::new();
        merge_fee_payer_deltas_from_object(obj, &mut deltas);
        if deltas.is_empty() {
            continue;
        }
        let ts = v
            .get("ts_utc")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        rows.push(Row { ts, deltas });
    }
    rows.sort_by(|a, b| a.ts.cmp(&b.ts));
    for r in rows {
        let u = capital_usd_from_mint_deltas(&r.deltas, mint_a, mint_b, pa, pb);
        if u > 0.0 {
            return u;
        }
    }
    0.0
}

async fn capital_from_db_first_snapshot_usd(db: &Database, position: &str) -> Option<f64> {
    let row = sqlx::query(
        r#"
        SELECT value_usd
        FROM position_stream_valuation_snapshots
        WHERE position_pubkey = $1
        ORDER BY ts_utc ASC
        LIMIT 1
        "#,
    )
    .bind(position)
    .fetch_optional(db.pool())
    .await
    .ok()
    .flatten()?;
    let v: Decimal = row.try_get("value_usd").ok()?;
    if v > Decimal::ZERO { v.to_f64() } else { None }
}

/// Spawn a historical backtest for a closed position (best-effort) using CLI `clmm-lp-cli backtest`.
#[utoipa::path(
    post,
    path = "/backtests/from-closed-position",
    tag = "Analytics",
    request_body = BacktestFromClosedPositionRequest,
    responses(
        (status = 200, description = "Backtest job started", body = BacktestJobStatusResponse),
        (status = 400, description = "Invalid request")
    )
)]
pub async fn backtest_from_closed_position(
    State(state): State<AppState>,
    Json(req): Json<BacktestFromClosedPositionRequest>,
) -> ApiResult<Json<BacktestJobStatusResponse>> {
    let position = req.position_address.trim();
    if position.is_empty() {
        return Err(ApiError::bad_request("position_address is required"));
    }

    let (opened_ts, closed_ts, pool_address, open_sid, details) = registry_lookup(position)
        .ok_or_else(|| {
            ApiError::not_found("Position not found in registry (missing open/close rows)")
        })?;

    // Resolve dates (best-effort). CLI `--end-date` is exclusive at 00:00 UTC; see `end_date_for_cli` below when inferring from registry.
    let start_date = req
        .start_date
        .clone()
        .or_else(|| opened_ts.as_deref().and_then(iso_date));
    let end_date = req
        .end_date
        .clone()
        .or_else(|| closed_ts.as_deref().and_then(iso_date));

    // Registry-inferred end is the close **day**; CLI `--end-date` at that same string excludes the whole day (and same as
    // `--start-date` yields an empty window). Pass start of the **next** UTC day as the exclusive upper bound.
    let end_date_for_cli = if req.end_date.is_none() {
        end_date
            .as_deref()
            .and_then(next_calendar_day_utc)
            .or_else(|| end_date.clone())
    } else {
        end_date.clone()
    };

    // Derive range bounds from open ticks stored in registry details (preferred).
    let (mut lower, mut upper) = (req.lower, req.upper);
    if lower.is_none() || upper.is_none() {
        let tick_lower = details
            .as_ref()
            .and_then(|d| d.get("tick_lower"))
            .and_then(|x| x.as_i64())
            .map(|n| n as i32);
        let tick_upper = details
            .as_ref()
            .and_then(|d| d.get("tick_upper"))
            .and_then(|x| x.as_i64())
            .map(|n| n as i32);
        if let (Some(tl), Some(tu)) = (tick_lower, tick_upper) {
            let lp = tick_to_price(tl).ok().and_then(|d| d.to_f64());
            let up = tick_to_price(tu).ok().and_then(|d| d.to_f64());
            if lower.is_none() {
                lower = lp;
            }
            if upper.is_none() {
                upper = up;
            }
        }
    }
    let lower = lower.unwrap_or(0.0);
    let upper = upper.unwrap_or(0.0);
    if lower <= 0.0 || upper <= 0.0 || lower >= upper {
        return Err(ApiError::bad_request(
            "Could not derive lower/upper from registry details; provide lower/upper overrides",
        ));
    }

    // One pool read + prices for capital derivation and CLI args.
    let pool_state = clmm_lp_protocols::prelude::WhirlpoolReader::new(state.provider.clone())
        .get_pool_state(&pool_address)
        .await
        .map_err(|e| ApiError::internal(format!("get_pool_state failed: {e}")))?;
    let mint_a = pool_state.token_mint_a.to_string();
    let mint_b = pool_state.token_mint_b.to_string();
    let mut mints = BTreeSet::new();
    mints.insert(mint_a.clone());
    mints.insert(mint_b.clone());
    let (px, _src) = fetch_mint_prices_usd(&mints).await;
    let pa = px.get(&mint_a).copied().unwrap_or(0.0);
    let pb = px.get(&mint_b).copied().unwrap_or(0.0);

    // Capital: explicit request → ledger session → first open row for PDA → first DB snapshot.
    let capital = if let Some(c) = req.capital {
        c.max(0.0)
    } else {
        let mut cap = 0.0_f64;
        let ledger = clmm_lp_protocols::ledger::tx_lifecycle::ledger_read_path();
        if ledger.exists()
            && let Ok(txt) = std::fs::read_to_string(&ledger)
        {
            if let Some(ref sid) = open_sid {
                let sid = sid.trim();
                if !sid.is_empty() {
                    cap = capital_from_ledger_session(&txt, sid, &mint_a, &mint_b, pa, pb);
                }
            }
            if cap <= 0.0 {
                cap = capital_from_ledger_first_open(&txt, position, &mint_a, &mint_b, pa, pb);
            }
        }
        if cap <= 0.0
            && let Some(ref db) = state.db
            && let Some(c) = capital_from_db_first_snapshot_usd(db, position).await
        {
            cap = c;
        }
        cap
    };
    if capital <= 0.0 {
        return Err(ApiError::bad_request(
            "Could not derive capital (USD). Pass JSON field \"capital\", or ensure lifecycle ledger has fee_payer_token_deltas on an open row for this PDA, registry has rebalance_session_id matching that ledger, and/or DB has position_stream_valuation_snapshots for this PDA.",
        ));
    }

    let id = Uuid::new_v4().to_string();
    let job = BacktestJobResponse {
        id: id.clone(),
        position_address: position.to_string(),
        pool_address: pool_address.clone(),
        status: "running".to_string(),
        started_ts_utc: chrono::Utc::now().to_rfc3339(),
        finished_ts_utc: None,
        exit_code: None,
        stdout: None,
        stderr: None,
        note: Some(
            "Runs `clmm-lp-cli backtest` as a subprocess; resolves CLI via CLMM_LP_CLI_PATH, same target/ as API, CLMM_REPO_ROOT, or PATH. Best-effort; needs local snapshot data."
                .to_string(),
        ),
    };

    {
        let mut w = JOBS.write().await;
        w.insert(id.clone(), job.clone());
    }

    let repo_root = state.config.repo_root.clone();
    let strategy = req.strategy.clone().unwrap_or_else(|| "static".to_string());
    let fee_source = req
        .fee_source
        .clone()
        .unwrap_or_else(|| "snapshots".to_string());
    let price_path_source = req
        .price_path_source
        .clone()
        .unwrap_or_else(|| "snapshots".to_string());
    let snapshot_protocol = req
        .snapshot_protocol
        .clone()
        .unwrap_or_else(|| "orca".to_string());

    let job_id = id.clone();
    tokio::spawn(async move {
        let cli_path = match resolve_clmm_lp_cli_path(repo_root.as_deref()) {
            Ok(p) => p,
            Err(msg) => {
                let mut w = JOBS.write().await;
                let Some(mut cur) = w.get(&job_id).cloned() else {
                    return;
                };
                cur.status = "failed".to_string();
                cur.stderr = Some(msg);
                cur.finished_ts_utc = Some(chrono::Utc::now().to_rfc3339());
                w.insert(job_id, cur);
                return;
            }
        };
        let mut cmd = Command::new(&cli_path);
        cmd.arg("backtest");
        cmd.arg("--symbol-a").arg("A");
        cmd.arg("--mint-a").arg(&mint_a);
        cmd.arg("--symbol-b").arg("B");
        cmd.arg("--mint-b").arg(&mint_b);
        cmd.arg("--lower").arg(format!("{lower}"));
        cmd.arg("--upper").arg(format!("{upper}"));
        cmd.arg("--capital").arg(format!("{capital}"));
        cmd.arg("--strategy").arg(strategy);
        cmd.arg("--fee-source").arg(fee_source);
        cmd.arg("--price-path-source").arg(price_path_source);
        cmd.arg("--snapshot-protocol").arg(snapshot_protocol);
        cmd.arg("--snapshot-pool-address").arg(&pool_address);
        if let Some(sd) = start_date.as_deref() {
            cmd.arg("--start-date").arg(sd);
        }
        if let Some(ed) = end_date_for_cli.as_deref() {
            cmd.arg("--end-date").arg(ed);
        }

        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let out = cmd.output().await;
        let mut w = JOBS.write().await;
        let Some(mut cur) = w.get(&job_id).cloned() else {
            return;
        };
        match out {
            Ok(o) => {
                cur.exit_code = o.status.code();
                cur.status = if o.status.success() {
                    "succeeded".to_string()
                } else {
                    "failed".to_string()
                };
                cur.stdout = Some(String::from_utf8_lossy(&o.stdout).to_string());
                cur.stderr = Some(String::from_utf8_lossy(&o.stderr).to_string());
                cur.finished_ts_utc = Some(chrono::Utc::now().to_rfc3339());
            }
            Err(e) => {
                cur.status = "failed".to_string();
                cur.stderr = Some(format!("failed to spawn backtest subprocess: {e}"));
                cur.finished_ts_utc = Some(chrono::Utc::now().to_rfc3339());
            }
        }
        w.insert(job_id.clone(), cur);
    });

    Ok(Json(BacktestJobStatusResponse {
        id,
        status: job.status,
        note: job.note,
    }))
}

/// Spawn a historical backtest for an open position using current open config.
#[utoipa::path(
    post,
    path = "/backtests/from-open-position",
    tag = "Analytics",
    request_body = BacktestFromOpenPositionRequest,
    responses(
        (status = 200, description = "Backtest job started", body = BacktestJobStatusResponse),
        (status = 400, description = "Invalid request")
    )
)]
pub async fn backtest_from_open_position(
    State(state): State<AppState>,
    Json(req): Json<BacktestFromOpenPositionRequest>,
) -> ApiResult<Json<BacktestJobStatusResponse>> {
    let position = req.position_address.trim();
    if position.is_empty() {
        return Err(ApiError::bad_request("position_address is required"));
    }

    let (opened_ts, _closed_ts, pool_address, open_sid, details) = registry_lookup(position)
        .ok_or_else(|| ApiError::not_found("Position not found in registry (missing open row)"))?;

    let start_date = req
        .start_date
        .clone()
        .or_else(|| opened_ts.as_deref().and_then(iso_date));
    let end_date_for_cli = req.end_date.clone().or_else(|| {
        let d = chrono::Utc::now().date_naive();
        d.succ_opt().map(|x| x.format("%Y-%m-%d").to_string())
    });

    let (mut lower, mut upper) = (req.lower, req.upper);
    if lower.is_none() || upper.is_none() {
        let tick_lower = details
            .as_ref()
            .and_then(|d| d.get("tick_lower"))
            .and_then(|x| x.as_i64())
            .map(|n| n as i32);
        let tick_upper = details
            .as_ref()
            .and_then(|d| d.get("tick_upper"))
            .and_then(|x| x.as_i64())
            .map(|n| n as i32);
        if let (Some(tl), Some(tu)) = (tick_lower, tick_upper) {
            let lp = tick_to_price(tl).ok().and_then(|d| d.to_f64());
            let up = tick_to_price(tu).ok().and_then(|d| d.to_f64());
            if lower.is_none() {
                lower = lp;
            }
            if upper.is_none() {
                upper = up;
            }
        }
    }
    let lower = lower.unwrap_or(0.0);
    let upper = upper.unwrap_or(0.0);
    if lower <= 0.0 || upper <= 0.0 || lower >= upper {
        return Err(ApiError::bad_request(
            "Could not derive lower/upper from registry details; provide lower/upper overrides",
        ));
    }

    let pool_state = clmm_lp_protocols::prelude::WhirlpoolReader::new(state.provider.clone())
        .get_pool_state(&pool_address)
        .await
        .map_err(|e| ApiError::internal(format!("get_pool_state failed: {e}")))?;
    let mint_a = pool_state.token_mint_a.to_string();
    let mint_b = pool_state.token_mint_b.to_string();
    let mut mints = BTreeSet::new();
    mints.insert(mint_a.clone());
    mints.insert(mint_b.clone());
    let (px, _src) = fetch_mint_prices_usd(&mints).await;
    let pa = px.get(&mint_a).copied().unwrap_or(0.0);
    let pb = px.get(&mint_b).copied().unwrap_or(0.0);

    let capital = if let Some(c) = req.capital {
        c.max(0.0)
    } else {
        let mut cap = 0.0_f64;
        let ledger = clmm_lp_protocols::ledger::tx_lifecycle::ledger_read_path();
        if ledger.exists()
            && let Ok(txt) = std::fs::read_to_string(&ledger)
        {
            if let Some(ref sid) = open_sid {
                let sid = sid.trim();
                if !sid.is_empty() {
                    cap = capital_from_ledger_session(&txt, sid, &mint_a, &mint_b, pa, pb);
                }
            }
            if cap <= 0.0 {
                cap = capital_from_ledger_first_open(&txt, position, &mint_a, &mint_b, pa, pb);
            }
        }
        if cap <= 0.0
            && let Some(ref db) = state.db
            && let Some(c) = capital_from_db_first_snapshot_usd(db, position).await
        {
            cap = c;
        }
        cap
    };
    if capital <= 0.0 {
        return Err(ApiError::bad_request(
            "Could not derive capital (USD). Pass JSON field \"capital\", or ensure lifecycle ledger has fee_payer_token_deltas on an open row for this PDA and/or DB has position_stream_valuation_snapshots for this PDA.",
        ));
    }

    let id = Uuid::new_v4().to_string();
    let job = BacktestJobResponse {
        id: id.clone(),
        position_address: position.to_string(),
        pool_address: pool_address.clone(),
        status: "running".to_string(),
        started_ts_utc: chrono::Utc::now().to_rfc3339(),
        finished_ts_utc: None,
        exit_code: None,
        stdout: None,
        stderr: None,
        note: Some(
            "Runs `clmm-lp-cli backtest` from an active position config; best-effort and requires local snapshot data."
                .to_string(),
        ),
    };
    {
        let mut w = JOBS.write().await;
        w.insert(id.clone(), job.clone());
    }

    let repo_root = state.config.repo_root.clone();
    let strategy = req.strategy.clone().unwrap_or_else(|| "static".to_string());
    let fee_source = req
        .fee_source
        .clone()
        .unwrap_or_else(|| "snapshots".to_string());
    let price_path_source = req
        .price_path_source
        .clone()
        .unwrap_or_else(|| "snapshots".to_string());
    let snapshot_protocol = req
        .snapshot_protocol
        .clone()
        .unwrap_or_else(|| "orca".to_string());

    let job_id = id.clone();
    tokio::spawn(async move {
        let cli_path = match resolve_clmm_lp_cli_path(repo_root.as_deref()) {
            Ok(p) => p,
            Err(msg) => {
                let mut w = JOBS.write().await;
                let Some(mut cur) = w.get(&job_id).cloned() else {
                    return;
                };
                cur.status = "failed".to_string();
                cur.stderr = Some(msg);
                cur.finished_ts_utc = Some(chrono::Utc::now().to_rfc3339());
                w.insert(job_id, cur);
                return;
            }
        };
        let mut cmd = Command::new(&cli_path);
        cmd.arg("backtest");
        cmd.arg("--symbol-a").arg("A");
        cmd.arg("--mint-a").arg(&mint_a);
        cmd.arg("--symbol-b").arg("B");
        cmd.arg("--mint-b").arg(&mint_b);
        cmd.arg("--lower").arg(format!("{lower}"));
        cmd.arg("--upper").arg(format!("{upper}"));
        cmd.arg("--capital").arg(format!("{capital}"));
        cmd.arg("--strategy").arg(strategy);
        cmd.arg("--fee-source").arg(fee_source);
        cmd.arg("--price-path-source").arg(price_path_source);
        cmd.arg("--snapshot-protocol").arg(snapshot_protocol);
        cmd.arg("--snapshot-pool-address").arg(&pool_address);
        if let Some(sd) = start_date.as_deref() {
            cmd.arg("--start-date").arg(sd);
        }
        if let Some(ed) = end_date_for_cli.as_deref() {
            cmd.arg("--end-date").arg(ed);
        }
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let out = cmd.output().await;
        let mut w = JOBS.write().await;
        let Some(mut cur) = w.get(&job_id).cloned() else {
            return;
        };
        match out {
            Ok(o) => {
                cur.exit_code = o.status.code();
                cur.status = if o.status.success() {
                    "succeeded".to_string()
                } else {
                    "failed".to_string()
                };
                cur.stdout = Some(String::from_utf8_lossy(&o.stdout).to_string());
                cur.stderr = Some(String::from_utf8_lossy(&o.stderr).to_string());
                cur.finished_ts_utc = Some(chrono::Utc::now().to_rfc3339());
            }
            Err(e) => {
                cur.status = "failed".to_string();
                cur.stderr = Some(format!("failed to spawn backtest subprocess: {e}"));
                cur.finished_ts_utc = Some(chrono::Utc::now().to_rfc3339());
            }
        }
        w.insert(job_id.clone(), cur);
    });

    Ok(Json(BacktestJobStatusResponse {
        id,
        status: job.status,
        note: job.note,
    }))
}

/// Get job status and outputs.
#[utoipa::path(
    get,
    path = "/backtests/{id}",
    tag = "Analytics",
    params(
        ("id" = String, Path, description = "Backtest job id")
    ),
    responses(
        (status = 200, description = "Job record", body = BacktestJobResponse),
        (status = 404, description = "Not found")
    )
)]
pub async fn get_backtest_job(
    State(_state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<BacktestJobResponse>> {
    let r = JOBS.read().await;
    let j = r
        .get(id.trim())
        .cloned()
        .ok_or_else(|| ApiError::not_found("Job not found"))?;
    Ok(Json(j))
}

/// List strategy families/parameter groups for Backtests UI.
#[utoipa::path(
    get,
    path = "/backtests/strategy-catalog",
    tag = "Analytics",
    responses((status = 200, description = "Backtest strategy catalog", body = BacktestStrategyCatalogResponse))
)]
pub async fn get_backtest_strategy_catalog() -> ApiResult<Json<BacktestStrategyCatalogResponse>> {
    Ok(Json(BacktestStrategyCatalogResponse {
        strategies: strategy_catalog(),
    }))
}

/// Run FULL backtest matrix (curated pools × windows) via `backtest-optimize`.
#[utoipa::path(
    post,
    path = "/backtests/full",
    tag = "Analytics",
    request_body = BacktestFullRequest,
    responses((status = 200, description = "Full matrix job started", body = BacktestFullJobStatusResponse))
)]
pub async fn start_backtest_full(
    State(state): State<AppState>,
    Json(req): Json<BacktestFullRequest>,
) -> ApiResult<Json<BacktestFullJobStatusResponse>> {
    let windows = if req.windows_hours.is_empty() {
        vec![24, 48, 72, 96]
    } else {
        req.windows_hours.clone()
    };
    if windows.iter().any(|h| *h == 0) {
        return Err(ApiError::bad_request("windows_hours must be positive"));
    }

    let mut pools = curated_backtest_pools();
    if let Some(ids) = req.pool_ids.as_ref() {
        let want: std::collections::HashSet<String> = ids.iter().map(|s| s.to_ascii_uppercase()).collect();
        pools.retain(|p| want.contains(&p.id.to_ascii_uppercase()));
    }
    if pools.is_empty() {
        return Err(ApiError::bad_request("No pools selected for full backtest run"));
    }

    let id = Uuid::new_v4().to_string();
    let job = BacktestFullJobResponse {
        id: id.clone(),
        status: "running".to_string(),
        started_ts_utc: chrono::Utc::now().to_rfc3339(),
        finished_ts_utc: None,
        stderr: None,
        note: Some("Running backtest-optimize matrix (curated pools x windows).".to_string()),
        results: None,
    };
    {
        let mut w = FULL_JOBS.write().await;
        w.insert(id.clone(), job.clone());
    }

    let repo_root = state.config.repo_root.clone();
    let include_ids = req
        .include_strategy_ids
        .unwrap_or_else(|| strategy_catalog().into_iter().map(|s| s.id).collect::<Vec<_>>());
    let include_ids: std::collections::HashSet<String> =
        include_ids.into_iter().map(|s| s.to_ascii_lowercase()).collect();
    let include_indicators = req.include_indicator_strategies;
    let objective = req.objective.unwrap_or_else(|| "vs-hodl".to_string());
    let lp_share = req.lp_share;
    let capital_usd = req.capital_usd.unwrap_or(7000.0).max(0.0);
    let target_vs_hodl_usd = req.target_vs_hodl_usd;
    let job_id = id.clone();

    tokio::spawn(async move {
        let cli_path = match resolve_clmm_lp_cli_path(repo_root.as_deref()) {
            Ok(p) => p,
            Err(msg) => {
                let mut w = FULL_JOBS.write().await;
                if let Some(mut cur) = w.get(&job_id).cloned() {
                    cur.status = "failed".to_string();
                    cur.stderr = Some(msg);
                    cur.finished_ts_utc = Some(chrono::Utc::now().to_rfc3339());
                    w.insert(job_id.clone(), cur);
                }
                return;
            }
        };

        let supports_include_strategy_families = cli_supports_include_strategy_families(&cli_path);
        let full_strategy_catalog_selected = is_full_strategy_catalog(&include_ids);
        if !supports_include_strategy_families && !full_strategy_catalog_selected {
            let mut w = FULL_JOBS.write().await;
            if let Some(mut cur) = w.get(&job_id).cloned() {
                cur.status = "failed".to_string();
                cur.stderr = Some(format!(
                    "clmm-lp-cli at {} does not support --include-strategy-families (binary too old for this UI filter). Rebuild: `cargo build --release -p clmm-lp-cli` (or `cargo build -p clmm-lp-cli`), restart API, or set CLMM_LP_CLI_PATH to the rebuilt binary.",
                    cli_path.display()
                ));
                cur.finished_ts_utc = Some(chrono::Utc::now().to_rfc3339());
                w.insert(job_id.clone(), cur);
            }
            return;
        }

        let mut all_results: Vec<BacktestFullWindowResult> = Vec::new();
        let mut errors: Vec<String> = Vec::new();

        for pool in pools {
            let snapshots_path = PathBuf::from("data")
                .join("pool-snapshots")
                .join(pool.protocol)
                .join(pool.pool_address)
                .join("snapshots.jsonl");
            if !snapshots_path.exists() {
                errors.push(format!(
                    "{} {}h: missing snapshots file {}",
                    pool.id,
                    0,
                    snapshots_path.display()
                ));
                continue;
            }
            for h in &windows {
                let mut cmd = Command::new(&cli_path);
                cmd.arg("backtest-optimize");
                cmd.arg("--symbol-a").arg(pool.symbol_a);
                cmd.arg("--mint-a").arg(pool.mint_a);
                cmd.arg("--symbol-b").arg(pool.symbol_b);
                cmd.arg("--mint-b").arg(pool.mint_b);
                cmd.arg("--price-path-source").arg("snapshots");
                cmd.arg("--fee-source").arg("snapshots");
                cmd.arg("--snapshot-protocol").arg(pool.protocol);
                cmd.arg("--snapshot-pool-address").arg(pool.pool_address);
                cmd.arg("--hours").arg(h.to_string());
                cmd.arg("--min-range-pct").arg("1");
                cmd.arg("--max-range-pct").arg("15");
                cmd.arg("--range-steps").arg("10");
                cmd.arg("--objective").arg(&objective);
                cmd.arg("--capital").arg(capital_usd.to_string());
                cmd.arg("--full-ranking");
                if supports_include_strategy_families && !include_ids.is_empty() {
                    let mut ids: Vec<String> = include_ids.iter().cloned().collect();
                    ids.sort_unstable();
                    cmd.arg("--include-strategy-families").arg(ids.join(","));
                }
                if include_indicators {
                    cmd.arg("--indicator-strategies");
                }
                if let Some(share) = lp_share {
                    cmd.arg("--lp-share").arg(share.to_string());
                }
                cmd.stdout(Stdio::piped());
                cmd.stderr(Stdio::piped());
                match cmd.output().await {
                    Ok(out) => {
                        if !out.status.success() {
                            errors.push(format!(
                                "{} {}h: exit {:?}: {}",
                                pool.id,
                                h,
                                out.status.code(),
                                String::from_utf8_lossy(&out.stderr)
                            ));
                            continue;
                        }
                        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                        let mut metrics = parse_optimize_table(&stdout);
                        if let Some(target) = target_vs_hodl_usd {
                            metrics.retain(|m| m.vs_hodl >= target);
                        }
                        all_results.push(BacktestFullWindowResult {
                            pool_id: pool.id.to_string(),
                            pool_label: pool.label.to_string(),
                            pool_address: pool.pool_address.to_string(),
                            protocol: pool.protocol.to_string(),
                            window_hours: *h,
                            metrics,
                            note: None,
                        });
                    }
                    Err(e) => {
                        errors.push(format!("{} {}h: spawn error: {}", pool.id, h, e));
                    }
                }
            }
        }

        let mut w = FULL_JOBS.write().await;
        if let Some(mut cur) = w.get(&job_id).cloned() {
            cur.status = if errors.is_empty() { "succeeded" } else { "partial" }.to_string();
            cur.results = Some(all_results);
            cur.stderr = if errors.is_empty() {
                None
            } else {
                Some(errors.join("\n"))
            };
            cur.finished_ts_utc = Some(chrono::Utc::now().to_rfc3339());
            w.insert(job_id.clone(), cur);
        }
    });

    Ok(Json(BacktestFullJobStatusResponse {
        id,
        status: job.status,
        note: job.note,
    }))
}

/// Get FULL backtest matrix job details/results.
#[utoipa::path(
    get,
    path = "/backtests/full/{id}",
    tag = "Analytics",
    params(("id" = String, Path, description = "Full backtest matrix job id")),
    responses((status = 200, description = "Full backtest job", body = BacktestFullJobResponse))
)]
pub async fn get_backtest_full_job(Path(id): Path<String>) -> ApiResult<Json<BacktestFullJobResponse>> {
    let r = FULL_JOBS.read().await;
    let j = r
        .get(id.trim())
        .cloned()
        .ok_or_else(|| ApiError::not_found("Full backtest job not found"))?;
    Ok(Json(j))
}
