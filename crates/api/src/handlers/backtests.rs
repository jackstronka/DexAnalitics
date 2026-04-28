//! Backtest endpoints (spawn CLI backtest runs).

use crate::error::{ApiError, ApiResult};
use crate::models::{
    BacktestAutoTuneApplyResponse, BacktestAutoTuneStartRequest, BacktestAutoTuneStatusResponse,
    BacktestAutoTuneWinner, BacktestDataReadinessAggregate, BacktestDataReadinessRequest,
    BacktestDataReadinessResponse, BacktestDataReadinessRow, BacktestDataReadinessThresholds,
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
use chrono::{DateTime, Utc};
use sqlx::Row;
use std::collections::{BTreeSet, HashMap};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::Stdio;
use std::str::FromStr;
use std::sync::{LazyLock, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::process::Command;
use tokio::sync::RwLock;
use tokio::time::{Duration, sleep};
use tracing::{info, warn};
use uuid::Uuid;

#[derive(Clone, Copy, Debug)]
struct SnapshotVariant {
    label: &'static str,
    suffix: Option<&'static str>,
    cadence_minutes: u64,
}

#[derive(Clone, Debug)]
struct ReadinessConfig {
    cache_ttl_secs: i64,
    db_max_age_secs: i64,
    hard_gap_multiplier: u64,
    recommended_coverage_pct: f64,
    recommended_gap_multiplier: u64,
    recommended_fallback_ratio: f64,
}

impl ReadinessConfig {
    fn from_env() -> Self {
        fn env_i64(name: &str, default: i64) -> i64 {
            std::env::var(name)
                .ok()
                .and_then(|v| v.trim().parse::<i64>().ok())
                .unwrap_or(default)
        }
        fn env_u64(name: &str, default: u64) -> u64 {
            std::env::var(name)
                .ok()
                .and_then(|v| v.trim().parse::<u64>().ok())
                .unwrap_or(default)
        }
        fn env_f64(name: &str, default: f64) -> f64 {
            std::env::var(name)
                .ok()
                .and_then(|v| v.trim().parse::<f64>().ok())
                .unwrap_or(default)
        }
        Self {
            cache_ttl_secs: env_i64("BACKTEST_READINESS_CACHE_TTL_SECS", 120).max(0),
            db_max_age_secs: env_i64("BACKTEST_READINESS_DB_MAX_AGE_SECS", 600).max(0),
            hard_gap_multiplier: env_u64("BACKTEST_READINESS_HARD_GAP_MULTIPLIER", 6).max(1),
            recommended_coverage_pct: env_f64("BACKTEST_READINESS_RECOMMENDED_COVERAGE_PCT", 95.0)
                .clamp(0.0, 100.0),
            recommended_gap_multiplier: env_u64("BACKTEST_READINESS_RECOMMENDED_GAP_MULTIPLIER", 2)
                .max(1),
            recommended_fallback_ratio: env_f64("BACKTEST_READINESS_RECOMMENDED_FALLBACK_RATIO", 0.7)
                .clamp(0.0, 1.0),
        }
    }
}

#[derive(Clone)]
struct ReadinessCacheEntry {
    ts_utc: chrono::DateTime<Utc>,
    response: BacktestDataReadinessResponse,
}

fn parse_snapshot_variants(raw: Option<&Vec<String>>) -> Result<Vec<SnapshotVariant>, ApiError> {
    let requested: Vec<String> = raw
        .cloned()
        .unwrap_or_else(|| vec!["10m".to_string()])
        .into_iter()
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    if requested.is_empty() {
        return Err(ApiError::bad_request(
            "snapshot_variants must include at least one of: 10m, 5m",
        ));
    }
    let mut uniq = BTreeSet::new();
    for v in requested {
        uniq.insert(v);
    }
    let mut out = Vec::new();
    for v in uniq {
        match v.as_str() {
            "10m" => out.push(SnapshotVariant {
                label: "10m",
                suffix: None,
                cadence_minutes: 10,
            }),
            "5m" => out.push(SnapshotVariant {
                label: "5m",
                suffix: Some("5m"),
                cadence_minutes: 5,
            }),
            other => {
                return Err(ApiError::bad_request(format!(
                    "unsupported snapshot variant `{other}`; allowed: 10m, 5m"
                )));
            }
        }
    }
    Ok(out)
}

fn hours_grid_to_steps(hours: &[f64], cadence_minutes: u64) -> Vec<u64> {
    let mut out = Vec::new();
    for h in hours {
        if !h.is_finite() || *h <= 0.0 {
            continue;
        }
        let steps = ((*h * 60.0) / cadence_minutes as f64).round();
        if !steps.is_finite() || steps <= 0.0 {
            continue;
        }
        let steps_u64 = steps as u64;
        if steps_u64 > 0 {
            out.push(steps_u64);
        }
    }
    out
}

fn snapshot_file_name_for_variant(variant: SnapshotVariant) -> String {
    match variant.suffix {
        Some(s) => format!("snapshots_{s}.jsonl"),
        None => "snapshots.jsonl".to_string(),
    }
}

async fn load_readiness_rows_from_db(
    db: &Database,
    pool_ids: &[String],
    variant_labels: &[String],
    cfg: &ReadinessConfig,
) -> Result<Option<Vec<BacktestDataReadinessRow>>, sqlx::Error> {
    if pool_ids.is_empty() || variant_labels.is_empty() {
        return Ok(None);
    }
    let rows = sqlx::query(
        r#"
        SELECT
            pool_id,
            pool_label,
            protocol,
            pool_address,
            snapshot_variant,
            cadence_minutes,
            rows,
            oldest_ts_utc,
            latest_ts_utc,
            oldest_continuous_ts_utc,
            max_gap_minutes,
            coverage_pct,
            max_backtest_hours_hard,
            max_backtest_hours_recommended,
            note,
            computed_at
        FROM backtest_data_readiness_rows
        WHERE pool_id = ANY($1)
          AND snapshot_variant = ANY($2)
          AND computed_at >= NOW() - ($3::bigint * INTERVAL '1 second')
        "#,
    )
    .bind(pool_ids)
    .bind(variant_labels)
    .bind(cfg.db_max_age_secs)
    .fetch_all(db.pool())
    .await?;

    let expected = pool_ids.len() * variant_labels.len();
    if rows.len() != expected {
        return Ok(None);
    }
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let oldest_ts_utc = r
            .try_get::<Option<chrono::DateTime<Utc>>, _>("oldest_ts_utc")?
            .map(|d| d.to_rfc3339());
        let latest_ts_utc = r
            .try_get::<Option<chrono::DateTime<Utc>>, _>("latest_ts_utc")?
            .map(|d| d.to_rfc3339());
        let oldest_continuous_ts_utc = r
            .try_get::<Option<chrono::DateTime<Utc>>, _>("oldest_continuous_ts_utc")?
            .map(|d| d.to_rfc3339());
        out.push(BacktestDataReadinessRow {
            pool_id: r.try_get("pool_id")?,
            pool_label: r.try_get("pool_label")?,
            protocol: r.try_get("protocol")?,
            pool_address: r.try_get("pool_address")?,
            snapshot_variant: r.try_get("snapshot_variant")?,
            cadence_minutes: r.try_get::<i64, _>("cadence_minutes")?.max(0) as u64,
            rows: r.try_get::<i64, _>("rows")?.max(0) as u64,
            oldest_ts_utc,
            latest_ts_utc,
            oldest_continuous_ts_utc,
            max_gap_minutes: r.try_get("max_gap_minutes")?,
            coverage_pct: r.try_get("coverage_pct")?,
            max_backtest_hours_hard: r.try_get::<i64, _>("max_backtest_hours_hard")?.max(0) as u64,
            max_backtest_hours_recommended: r
                .try_get::<i64, _>("max_backtest_hours_recommended")?
                .max(0) as u64,
            status: "ok".to_string(),
            status_reason: None,
            latest_age_secs: None,
            note: r.try_get("note")?,
        });
    }
    Ok(Some(out))
}

async fn count_stale_readiness_rows_in_db(
    db: &Database,
    pool_ids: &[String],
    variant_labels: &[String],
    cfg: &ReadinessConfig,
) -> Result<u64, sqlx::Error> {
    if pool_ids.is_empty() || variant_labels.is_empty() {
        return Ok(0);
    }
    let row = sqlx::query(
        r#"
        SELECT COUNT(*) AS n
        FROM backtest_data_readiness_rows
        WHERE pool_id = ANY($1)
          AND snapshot_variant = ANY($2)
          AND computed_at < NOW() - ($3::bigint * INTERVAL '1 second')
        "#,
    )
    .bind(pool_ids)
    .bind(variant_labels)
    .bind(cfg.db_max_age_secs)
    .fetch_one(db.pool())
    .await?;
    Ok(row.try_get::<i64, _>("n")?.max(0) as u64)
}

async fn upsert_readiness_rows_best_effort(
    db: &Database,
    rows: &[BacktestDataReadinessRow],
) -> Result<(), sqlx::Error> {
    for r in rows {
        let oldest_ts_utc = r.oldest_ts_utc.as_deref().and_then(|s| {
            DateTime::parse_from_rfc3339(s)
                .ok()
                .map(|dt| dt.with_timezone(&Utc))
        });
        let latest_ts_utc = r.latest_ts_utc.as_deref().and_then(|s| {
            DateTime::parse_from_rfc3339(s)
                .ok()
                .map(|dt| dt.with_timezone(&Utc))
        });
        let oldest_continuous_ts_utc = r.oldest_continuous_ts_utc.as_deref().and_then(|s| {
            DateTime::parse_from_rfc3339(s)
                .ok()
                .map(|dt| dt.with_timezone(&Utc))
        });
        sqlx::query(
            r#"
            INSERT INTO backtest_data_readiness_rows (
                pool_id, pool_label, protocol, pool_address, snapshot_variant,
                cadence_minutes, rows, oldest_ts_utc, latest_ts_utc, oldest_continuous_ts_utc,
                max_gap_minutes, coverage_pct, max_backtest_hours_hard, max_backtest_hours_recommended,
                note, computed_at
            )
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,NOW())
            ON CONFLICT (pool_id, snapshot_variant) DO UPDATE SET
                pool_label = EXCLUDED.pool_label,
                protocol = EXCLUDED.protocol,
                pool_address = EXCLUDED.pool_address,
                cadence_minutes = EXCLUDED.cadence_minutes,
                rows = EXCLUDED.rows,
                oldest_ts_utc = EXCLUDED.oldest_ts_utc,
                latest_ts_utc = EXCLUDED.latest_ts_utc,
                oldest_continuous_ts_utc = EXCLUDED.oldest_continuous_ts_utc,
                max_gap_minutes = EXCLUDED.max_gap_minutes,
                coverage_pct = EXCLUDED.coverage_pct,
                max_backtest_hours_hard = EXCLUDED.max_backtest_hours_hard,
                max_backtest_hours_recommended = EXCLUDED.max_backtest_hours_recommended,
                note = EXCLUDED.note,
                computed_at = EXCLUDED.computed_at
            "#,
        )
        .bind(&r.pool_id)
        .bind(&r.pool_label)
        .bind(&r.protocol)
        .bind(&r.pool_address)
        .bind(&r.snapshot_variant)
        .bind(r.cadence_minutes as i64)
        .bind(r.rows as i64)
        .bind(oldest_ts_utc)
        .bind(latest_ts_utc)
        .bind(oldest_continuous_ts_utc)
        .bind(r.max_gap_minutes)
        .bind(r.coverage_pct)
        .bind(r.max_backtest_hours_hard as i64)
        .bind(r.max_backtest_hours_recommended as i64)
        .bind(&r.note)
        .execute(db.pool())
        .await?;
    }
    Ok(())
}

fn readiness_cache_key(
    pool_ids: &[String],
    variants: &[String],
    range_start_utc: Option<&str>,
    range_end_utc: Option<&str>,
) -> String {
    let mut p = pool_ids.to_vec();
    p.sort_unstable();
    let mut v = variants.to_vec();
    v.sort_unstable();
    format!(
        "pools={};variants={};start={};end={}",
        p.join(","),
        v.join(","),
        range_start_utc.unwrap_or("-"),
        range_end_utc.unwrap_or("-")
    )
}

fn evaluate_snapshot_readiness(
    path: &std::path::Path,
    cadence_minutes: u64,
    cfg: &ReadinessConfig,
    range_start_utc: Option<DateTime<Utc>>,
    range_end_utc: Option<DateTime<Utc>>,
) -> Result<
    (
        u64,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<f64>,
        Option<f64>,
        u64,
        u64,
        Option<String>,
    ),
    String,
> {
    let file = std::fs::File::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
    let reader = std::io::BufReader::new(file);
    let mut timestamps: Vec<DateTime<Utc>> = Vec::new();
    for line in reader.lines() {
        let Ok(line) = line else { continue };
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(t) else {
            continue;
        };
        let Some(ts) = v.get("ts_utc").and_then(Value::as_str) else {
            continue;
        };
        let Ok(dt) = DateTime::parse_from_rfc3339(ts) else {
            continue;
        };
        timestamps.push(dt.with_timezone(&Utc));
    }
    timestamps.sort_unstable();
    timestamps.dedup();
    if let Some(start) = range_start_utc {
        timestamps.retain(|t| *t >= start);
    }
    if let Some(end) = range_end_utc {
        timestamps.retain(|t| *t <= end);
    }
    if timestamps.is_empty() {
        return Ok((0, None, None, None, None, None, 0, 0, Some("No timestamp rows".to_string())));
    }

    // For explicit [from,to] analysis range, evaluate strict quality on the whole requested
    // interval (not "continuous from latest"). This matches operator expectation:
    // selected window is either OK or incomplete.
    if let (Some(range_start), Some(range_end)) = (range_start_utc, range_end_utc) {
        let requested_minutes = range_end
            .signed_duration_since(range_start)
            .num_minutes()
            .max(0) as u64;
        let requested_hours = requested_minutes / 60;
        let expected_rows =
            ((requested_minutes as f64) / cadence_minutes as f64).round().max(1.0) + 1.0;

        let first = timestamps.first().cloned();
        let last = timestamps.last().cloned();
        let mut max_gap_min = 0.0_f64;
        if let Some(f) = first {
            max_gap_min = max_gap_min.max(
                f.signed_duration_since(range_start)
                    .num_minutes()
                    .max(0) as f64,
            );
        }
        if let Some(l) = last {
            max_gap_min = max_gap_min.max(
                range_end
                    .signed_duration_since(l)
                    .num_minutes()
                    .max(0) as f64,
            );
        }
        for i in 1..timestamps.len() {
            let delta = timestamps[i]
                .signed_duration_since(timestamps[i - 1])
                .num_minutes()
                .max(0) as f64;
            if delta > max_gap_min {
                max_gap_min = delta;
            }
        }

        let coverage = Some(((timestamps.len() as f64 / expected_rows) * 100.0).clamp(0.0, 100.0));
        let hard_ok = max_gap_min <= (cfg.hard_gap_multiplier * cadence_minutes) as f64;
        let recommended_ok = coverage.unwrap_or(0.0) >= cfg.recommended_coverage_pct
            && max_gap_min <= (cfg.recommended_gap_multiplier * cadence_minutes) as f64;

        let hard_hours = if hard_ok { requested_hours } else { 0 };
        let recommended_hours = if recommended_ok { requested_hours } else { 0 };
        let note = if hard_ok && recommended_ok {
            Some("Selected range OK".to_string())
        } else {
            Some("Selected range incomplete".to_string())
        };

        return Ok((
            timestamps.len() as u64,
            first.map(|d| d.to_rfc3339()),
            last.map(|d| d.to_rfc3339()),
            Some(range_start.to_rfc3339()),
            Some(max_gap_min),
            coverage,
            hard_hours,
            recommended_hours,
            note,
        ));
    }

    let oldest = timestamps.first().cloned();
    let latest = timestamps.last().cloned();
    let threshold = (cadence_minutes * cfg.hard_gap_multiplier) as i64;
    let mut oldest_cont_idx = timestamps.len() - 1;
    let mut max_gap_min = 0.0_f64;
    for i in (1..timestamps.len()).rev() {
        let delta = timestamps[i].signed_duration_since(timestamps[i - 1]).num_minutes();
        if delta as f64 > max_gap_min {
            max_gap_min = delta as f64;
        }
        if delta > threshold {
            break;
        }
        oldest_cont_idx = i - 1;
    }

    let oldest_cont = timestamps.get(oldest_cont_idx).cloned();
    let hard_minutes = latest
        .zip(oldest_cont)
        .map(|(l, o)| l.signed_duration_since(o).num_minutes().max(0) as u64)
        .unwrap_or(0);
    let hard_hours = hard_minutes / 60;

    let mut expected_rows = 0.0_f64;
    let mut actual_rows = 0.0_f64;
    for i in oldest_cont_idx + 1..timestamps.len() {
        let delta = timestamps[i]
            .signed_duration_since(timestamps[i - 1])
            .num_minutes()
            .max(1) as f64;
        expected_rows += (delta / cadence_minutes as f64).round().max(1.0);
        actual_rows += 1.0;
    }
    let coverage = if expected_rows > 0.0 {
        Some((actual_rows / expected_rows * 100.0).clamp(0.0, 100.0))
    } else {
        Some(100.0)
    };
    let recommended_hours = if let Some(cov) = coverage {
        if cov >= cfg.recommended_coverage_pct
            && max_gap_min <= (cfg.recommended_gap_multiplier * cadence_minutes) as f64
        {
            hard_hours
        } else {
            ((hard_hours as f64) * cfg.recommended_fallback_ratio)
                .floor()
                .max(0.0) as u64
        }
    } else {
        0
    };

    Ok((
        timestamps.len() as u64,
        oldest.map(|d| d.to_rfc3339()),
        latest.map(|d| d.to_rfc3339()),
        timestamps.get(oldest_cont_idx).map(|d| d.to_rfc3339()),
        Some(max_gap_min),
        coverage,
        hard_hours,
        recommended_hours.min(hard_hours),
        None,
    ))
}

fn classify_readiness_status(
    row: &BacktestDataReadinessRow,
    has_custom_range: bool,
    now: DateTime<Utc>,
) -> (String, Option<String>, Option<i64>) {
    let latest_age_secs = row
        .latest_ts_utc
        .as_deref()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| now.signed_duration_since(dt.with_timezone(&Utc)).num_seconds());

    if row.rows == 0 {
        return (
            "missing".to_string(),
            Some("no_rows".to_string()),
            latest_age_secs,
        );
    }

    if let Some(note) = row.note.as_deref()
        && note.to_ascii_lowercase().contains("missing file")
    {
        return (
            "missing".to_string(),
            Some("missing_file".to_string()),
            latest_age_secs,
        );
    }

    if has_custom_range {
        if row.max_backtest_hours_hard > 0 {
            if row.max_backtest_hours_recommended > 0 {
                return ("ok".to_string(), Some("range_ok".to_string()), latest_age_secs);
            }
            return (
                "degraded".to_string(),
                Some("range_quality_below_recommended".to_string()),
                latest_age_secs,
            );
        }
        return (
            "degraded".to_string(),
            Some("range_incomplete".to_string()),
            latest_age_secs,
        );
    }

    let stale_threshold_secs = if row.cadence_minutes <= 5 { 12 * 60 } else { 20 * 60 };
    if let Some(age) = latest_age_secs && age > stale_threshold_secs {
        return (
            "degraded".to_string(),
            Some("stale_latest_snapshot".to_string()),
            Some(age),
        );
    }

    if row.max_backtest_hours_hard == 0 {
        return (
            "degraded".to_string(),
            Some("hard_window_unavailable".to_string()),
            latest_age_secs,
        );
    }

    if row.max_backtest_hours_recommended == 0 {
        return (
            "recovering".to_string(),
            Some("recommended_window_unavailable".to_string()),
            latest_age_secs,
        );
    }

    ("ok".to_string(), None, latest_age_secs)
}

fn aggregate_readiness_status(
    rows: &[BacktestDataReadinessRow],
) -> (String, u64, u64, u64, u64) {
    let mut ok_count = 0_u64;
    let mut degraded_count = 0_u64;
    let mut recovering_count = 0_u64;
    let mut missing_count = 0_u64;
    for row in rows {
        match row.status.as_str() {
            "ok" => ok_count += 1,
            "degraded" => degraded_count += 1,
            "recovering" => recovering_count += 1,
            _ => missing_count += 1,
        }
    }
    let status = if degraded_count > 0 {
        "degraded"
    } else if recovering_count > 0 {
        "recovering"
    } else if !rows.is_empty() && missing_count == rows.len() as u64 {
        "missing"
    } else {
        "ok"
    };
    (
        status.to_string(),
        ok_count,
        degraded_count,
        recovering_count,
        missing_count,
    )
}

static JOBS: LazyLock<RwLock<HashMap<String, BacktestJobResponse>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));
static FULL_JOBS: LazyLock<RwLock<HashMap<String, BacktestFullJobResponse>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));
static AUTO_TUNE_STOP: LazyLock<Mutex<Option<std::sync::Arc<AtomicBool>>>> =
    LazyLock::new(|| Mutex::new(None));
static AUTO_TUNE_STATUS: LazyLock<RwLock<BacktestAutoTuneStatusResponse>> = LazyLock::new(|| {
    RwLock::new(BacktestAutoTuneStatusResponse {
        running: false,
        interval_minutes: 30,
        started_ts_utc: None,
        last_tick_ts_utc: None,
        next_tick_ts_utc: None,
        latest_job_id: None,
        latest_winner: None,
        note: Some("Auto-tune is stopped".to_string()),
    })
});

/// Cache: `(exe path | mtime secs)` -> `backtest-optimize --help` text.
/// Key includes modification time so a rebuilt `clmm-lp-cli` is re-probed without restarting API.
static CLI_BACKTEST_OPTIMIZE_HELP_TEXT: LazyLock<Mutex<HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static BACKTEST_READINESS_CONFIG: LazyLock<ReadinessConfig> =
    LazyLock::new(ReadinessConfig::from_env);
static BACKTEST_READINESS_CACHE: LazyLock<Mutex<HashMap<String, ReadinessCacheEntry>>> =
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

fn probe_backtest_optimize_help_text(cli_path: &std::path::Path) -> String {
    std::process::Command::new(cli_path)
        .args(["backtest-optimize", "--help"])
        .output()
        .map(|o| {
            format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            )
        })
        .unwrap_or_default()
}

fn cli_supports_include_strategy_families(cli_path: &std::path::Path) -> bool {
    cli_backtest_optimize_help_text(cli_path).contains("include-strategy-families")
}

fn cli_backtest_optimize_help_text(cli_path: &std::path::Path) -> String {
    let cache_key = match std::fs::metadata(cli_path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
    {
        Some(d) => format!("{}|{}", cli_path.to_string_lossy(), d.as_secs()),
        None => {
            return probe_backtest_optimize_help_text(cli_path);
        }
    };
    let mut guard = CLI_BACKTEST_OPTIMIZE_HELP_TEXT.lock().unwrap();
    if let Some(v) = guard.get(&cache_key) {
        return v.clone();
    }
    let help_text = probe_backtest_optimize_help_text(cli_path);
    guard.insert(cache_key, help_text.clone());
    help_text
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
            parameters: vec!["width_pct".to_string(), "retouch_offset_pct".to_string()],
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

/// Compute snapshot data readiness for backtests (coverage/max-gap/max-back-hours).
#[utoipa::path(
    post,
    path = "/backtests/data-readiness",
    tag = "Analytics",
    request_body = BacktestDataReadinessRequest,
    responses((status = 200, description = "Data readiness", body = BacktestDataReadinessResponse))
)]
pub async fn get_backtest_data_readiness(
    State(state): State<AppState>,
    Json(req): Json<BacktestDataReadinessRequest>,
) -> ApiResult<Json<BacktestDataReadinessResponse>> {
    let range_start_utc = req
        .range_start_utc
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            DateTime::parse_from_rfc3339(s)
                .map(|d| d.with_timezone(&Utc))
                .map_err(|e| ApiError::bad_request(format!("invalid range_start_utc: {e}")))
        })
        .transpose()?;
    let range_end_utc = req
        .range_end_utc
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            DateTime::parse_from_rfc3339(s)
                .map(|d| d.with_timezone(&Utc))
                .map_err(|e| ApiError::bad_request(format!("invalid range_end_utc: {e}")))
        })
        .transpose()?;
    if let (Some(s), Some(e)) = (range_start_utc, range_end_utc)
        && s > e
    {
        return Err(ApiError::bad_request(
            "range_start_utc must be <= range_end_utc",
        ));
    }

    let mut pools = curated_backtest_pools();
    if let Some(ids) = req.pool_ids.as_ref() {
        let want: std::collections::HashSet<String> = ids.iter().map(|s| s.to_ascii_uppercase()).collect();
        pools.retain(|p| want.contains(&p.id.to_ascii_uppercase()));
    }
    if pools.is_empty() {
        return Err(ApiError::bad_request(
            "No pools selected for data readiness check",
        ));
    }
    let variants = parse_snapshot_variants(req.snapshot_variants.as_ref())?;
    let variant_labels: Vec<String> = variants.iter().map(|v| v.label.to_string()).collect();
    let selected_pool_ids: Vec<String> = pools.iter().map(|p| p.id.to_string()).collect();
    let cache_key = readiness_cache_key(
        &selected_pool_ids,
        &variant_labels,
        req.range_start_utc.as_deref(),
        req.range_end_utc.as_deref(),
    );
    let cfg = BACKTEST_READINESS_CONFIG.clone();
    let has_custom_range = range_start_utc.is_some() || range_end_utc.is_some();
    if cfg.cache_ttl_secs > 0 {
        let cached = {
            let cache = BACKTEST_READINESS_CACHE
                .lock()
                .expect("backtest readiness cache lock");
            cache.get(&cache_key).cloned()
        };
        if let Some(entry) = cached {
            let age_secs = (Utc::now() - entry.ts_utc).num_seconds();
            if age_secs >= 0 && age_secs <= cfg.cache_ttl_secs {
                return Ok(Json(entry.response));
            }
        }
    }
    let mut used_db_source = false;
    let db_stale_rows = if !has_custom_range && let Some(db) = state.db.as_ref() {
        count_stale_readiness_rows_in_db(db, &selected_pool_ids, &variant_labels, &cfg)
            .await
            .unwrap_or(0)
    } else {
        0
    };
    let mut rows: Vec<BacktestDataReadinessRow> = if !has_custom_range && let Some(db) = state.db.as_ref() {
        match load_readiness_rows_from_db(db, &selected_pool_ids, &variant_labels, &cfg).await {
            Ok(Some(db_rows)) => {
                used_db_source = true;
                db_rows
            }
            Ok(None) => Vec::new(),
            Err(e) => {
                warn!(error = %e, "readiness DB lookup failed; using on-demand fallback");
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };

    if rows.is_empty() {
        for variant in variants {
            for pool in &pools {
                let path = PathBuf::from("data")
                    .join("pool-snapshots")
                    .join(pool.protocol)
                    .join(pool.pool_address)
                    .join(snapshot_file_name_for_variant(variant));
                if !path.exists() {
                    rows.push(BacktestDataReadinessRow {
                        pool_id: pool.id.to_string(),
                        pool_label: pool.label.to_string(),
                        protocol: pool.protocol.to_string(),
                        pool_address: pool.pool_address.to_string(),
                        snapshot_variant: variant.label.to_string(),
                        cadence_minutes: variant.cadence_minutes,
                        rows: 0,
                        oldest_ts_utc: None,
                        latest_ts_utc: None,
                        oldest_continuous_ts_utc: None,
                        max_gap_minutes: None,
                        coverage_pct: None,
                        max_backtest_hours_hard: 0,
                        max_backtest_hours_recommended: 0,
                        status: "missing".to_string(),
                        status_reason: Some("missing_file".to_string()),
                        latest_age_secs: None,
                        note: Some(format!("Missing file: {}", path.display())),
                    });
                    continue;
                }
                let (count, oldest, latest, oldest_cont, max_gap, coverage, hard_h, rec_h, note) =
                    evaluate_snapshot_readiness(
                        &path,
                        variant.cadence_minutes,
                        &cfg,
                        range_start_utc,
                        range_end_utc,
                    )
                        .map_err(ApiError::internal)?;
                rows.push(BacktestDataReadinessRow {
                    pool_id: pool.id.to_string(),
                    pool_label: pool.label.to_string(),
                    protocol: pool.protocol.to_string(),
                    pool_address: pool.pool_address.to_string(),
                    snapshot_variant: variant.label.to_string(),
                    cadence_minutes: variant.cadence_minutes,
                    rows: count,
                    oldest_ts_utc: oldest,
                    latest_ts_utc: latest,
                    oldest_continuous_ts_utc: oldest_cont,
                    max_gap_minutes: max_gap,
                    coverage_pct: coverage,
                    max_backtest_hours_hard: hard_h,
                    max_backtest_hours_recommended: rec_h,
                    status: "ok".to_string(),
                    status_reason: None,
                    latest_age_secs: None,
                    note,
                });
            }
        }
        if !has_custom_range && let Some(db) = state.db.as_ref()
            && let Err(e) = upsert_readiness_rows_best_effort(db, &rows).await
        {
            warn!(error = %e, "readiness DB upsert failed after on-demand fallback");
        }
    }
    let pool_count = rows
        .iter()
        .map(|r| r.pool_id.clone())
        .collect::<std::collections::HashSet<_>>()
        .len() as u64;
    let variant_count = rows
        .iter()
        .map(|r| r.snapshot_variant.clone())
        .collect::<std::collections::HashSet<_>>()
        .len() as u64;
    let now = Utc::now();
    for row in &mut rows {
        let (status, reason, latest_age_secs) = classify_readiness_status(row, has_custom_range, now);
        row.status = status;
        row.status_reason = reason;
        row.latest_age_secs = latest_age_secs;
    }
    let (
        aggregate_status,
        status_ok_count,
        status_degraded_count,
        status_recovering_count,
        status_missing_count,
    ) = aggregate_readiness_status(&rows);
    let aggregate = BacktestDataReadinessAggregate {
        pool_count,
        variant_count,
        max_backtest_hours_hard: rows
            .iter()
            .map(|r| r.max_backtest_hours_hard)
            .min()
            .unwrap_or(0),
        max_backtest_hours_recommended: rows
            .iter()
            .map(|r| r.max_backtest_hours_recommended)
            .min()
            .unwrap_or(0),
        status: aggregate_status,
        status_ok_count,
        status_degraded_count,
        status_recovering_count,
        status_missing_count,
        source: if used_db_source {
            "db".to_string()
        } else {
            "fallback".to_string()
        },
        db_stale_rows,
    };
    let thresholds = BacktestDataReadinessThresholds {
        cache_ttl_secs: cfg.cache_ttl_secs,
        db_max_age_secs: cfg.db_max_age_secs,
        hard_gap_multiplier: cfg.hard_gap_multiplier,
        recommended_coverage_pct: cfg.recommended_coverage_pct,
        recommended_gap_multiplier: cfg.recommended_gap_multiplier,
        recommended_fallback_ratio: cfg.recommended_fallback_ratio,
    };
    let response = BacktestDataReadinessResponse {
        rows,
        aggregate,
        thresholds,
    };
    if cfg.cache_ttl_secs > 0 {
        let mut cache = BACKTEST_READINESS_CACHE
            .lock()
            .expect("backtest readiness cache lock");
        cache.insert(
            cache_key,
            ReadinessCacheEntry {
                ts_utc: Utc::now(),
                response: response.clone(),
            },
        );
    }
    Ok(Json(response))
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
    if windows.contains(&0) {
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
    let snapshot_variants = parse_snapshot_variants(req.snapshot_variants.as_ref())?;

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
    let static_deviation_pct = req.static_deviation_pct;
    let static_manual_lower = req.static_manual_lower;
    let static_manual_upper = req.static_manual_upper;
    let oor_recenter_deviation_pct = req.oor_recenter_deviation_pct;
    if let Some(d) = static_deviation_pct
        && (!d.is_finite() || d <= 0.0 || d >= 100.0)
    {
        return Err(ApiError::bad_request(
            "static_deviation_pct must be in (0,100)",
        ));
    }
    if let Some(d) = oor_recenter_deviation_pct
        && (!d.is_finite() || d <= 0.0 || d >= 100.0)
    {
        return Err(ApiError::bad_request(
            "oor_recenter_deviation_pct must be in (0,100)",
        ));
    }
    if static_deviation_pct.is_some() && oor_recenter_deviation_pct.is_some() {
        return Err(ApiError::bad_request(
            "Use only one of static_deviation_pct or oor_recenter_deviation_pct",
        ));
    }
    let static_manual_set = static_manual_lower.is_some() || static_manual_upper.is_some();
    if static_manual_set {
        let (Some(lower), Some(upper)) = (static_manual_lower, static_manual_upper) else {
            return Err(ApiError::bad_request(
                "Provide both static_manual_lower and static_manual_upper",
            ));
        };
        if !lower.is_finite() || !upper.is_finite() || lower <= 0.0 || upper <= 0.0 || lower >= upper {
            return Err(ApiError::bad_request(
                "static_manual_lower/static_manual_upper must be finite, >0 and lower<upper",
            ));
        }
        if pools.len() != 1 {
            return Err(ApiError::bad_request(
                "Manual static lower/upper range requires selecting exactly one pool",
            ));
        }
        if static_deviation_pct.is_some() {
            return Err(ApiError::bad_request(
                "Use either static_deviation_pct or static_manual_lower/static_manual_upper",
            ));
        }
    }
    if let Some(v) = req.retouch_offset_pct
        && !v.is_finite()
    {
        return Err(ApiError::bad_request(
            "retouch_offset_pct must be a finite number",
        ));
    }
    let threshold_grid_pct = req.threshold_grid_pct.clone();
    let threshold_min_rebalance_interval_hours = req.threshold_min_rebalance_interval_hours;
    let threshold_rebalance_on_range_exit_immediately =
        req.threshold_rebalance_on_range_exit_immediately;
    let periodic_grid_steps = req.periodic_grid_steps.clone();
    let bollinger_rebalance_hours_grid = req.bollinger_rebalance_hours_grid.clone();
    let retouch_offset_pct = req.retouch_offset_pct;
    let bollinger_window_grid = req.bollinger_window_grid.clone();
    let bollinger_k_grid = req.bollinger_k_grid.clone();
    let bollinger_rebalance_steps_grid = req.bollinger_rebalance_steps_grid.clone();
    let last_candle_steps_grid = req.last_candle_steps_grid.clone();
    let last_candle_rebalance_steps_grid = req.last_candle_rebalance_steps_grid.clone();
    let last_candle_seconds_grid = req.last_candle_seconds_grid.clone();
    let last_candle_rebalance_seconds_grid = req.last_candle_rebalance_seconds_grid.clone();
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
        info!(
            "backtests/full: using clmm-lp-cli binary at {}",
            cli_path.display()
        );

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

        let help_text = cli_backtest_optimize_help_text(&cli_path);
        let threshold_oor_flag_accepts_value = help_text
            .contains("--threshold-rebalance-on-range-exit-immediately <");
        let snapshot_jsonl_suffix_requested = snapshot_variants.iter().any(|v| v.suffix.is_some());
        let snapshot_jsonl_suffix_supported = help_text.contains("snapshot-jsonl-suffix");
        let threshold_grid_requested = threshold_grid_pct
            .as_ref()
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        let threshold_min_rebalance_hours_requested = threshold_min_rebalance_interval_hours.is_some();
        let threshold_oor_immediate_requested =
            threshold_rebalance_on_range_exit_immediately.is_some();
        let periodic_grid_requested = periodic_grid_steps
            .as_ref()
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        let static_manual_lower_requested = static_manual_lower.is_some();
        let static_manual_upper_requested = static_manual_upper.is_some();
        let retouch_offset_requested = retouch_offset_pct.is_some();
        let bollinger_window_grid_requested = bollinger_window_grid
            .as_ref()
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        let bollinger_k_grid_requested = bollinger_k_grid.as_ref().map(|v| !v.is_empty()).unwrap_or(false);
        let bollinger_rebalance_steps_grid_requested = bollinger_rebalance_steps_grid
            .as_ref()
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        let last_candle_steps_grid_requested = last_candle_steps_grid
            .as_ref()
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        let last_candle_rebalance_steps_grid_requested = last_candle_rebalance_steps_grid
            .as_ref()
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        let last_candle_seconds_grid_requested = last_candle_seconds_grid
            .as_ref()
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        let last_candle_rebalance_seconds_grid_requested = last_candle_rebalance_seconds_grid
            .as_ref()
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        let requested_grid_overrides = [
            ("--snapshot-jsonl-suffix", snapshot_jsonl_suffix_requested),
            ("--threshold-grid-pct", threshold_grid_requested),
            (
                "--threshold-min-rebalance-interval-hours",
                threshold_min_rebalance_hours_requested,
            ),
            (
                "--threshold-rebalance-on-range-exit-immediately",
                threshold_oor_immediate_requested,
            ),
            ("--periodic-grid-steps", periodic_grid_requested),
            ("--static-manual-lower", static_manual_lower_requested),
            ("--static-manual-upper", static_manual_upper_requested),
            ("--retouch-offset-pct", retouch_offset_requested),
            ("--bollinger-window-grid", bollinger_window_grid_requested),
            ("--bollinger-k-grid", bollinger_k_grid_requested),
            (
                "--bollinger-rebalance-steps-grid",
                bollinger_rebalance_steps_grid_requested,
            ),
            ("--last-candle-steps-grid", last_candle_steps_grid_requested),
            (
                "--last-candle-rebalance-steps-grid",
                last_candle_rebalance_steps_grid_requested,
            ),
            ("--last-candle-seconds-grid", last_candle_seconds_grid_requested),
            (
                "--last-candle-rebalance-seconds-grid",
                last_candle_rebalance_seconds_grid_requested,
            ),
        ];
        let missing_grid_flags: Vec<&str> = requested_grid_overrides
            .iter()
            .filter_map(|(flag, requested)| {
                if *requested && !help_text.contains(flag.trim_start_matches("--")) {
                    Some(*flag)
                } else {
                    None
                }
            })
            .collect();
        if !snapshot_jsonl_suffix_supported && snapshot_jsonl_suffix_requested {
            let mut w = FULL_JOBS.write().await;
            if let Some(mut cur) = w.get(&job_id).cloned() {
                cur.status = "failed".to_string();
                cur.stderr = Some(format!(
                    "clmm-lp-cli at {} does not support --snapshot-jsonl-suffix required for selected snapshot_variants. Rebuild CLI (`cargo build --release -p clmm-lp-cli` or `cargo build -p clmm-lp-cli`) and restart API.",
                    cli_path.display()
                ));
                cur.finished_ts_utc = Some(chrono::Utc::now().to_rfc3339());
                w.insert(job_id.clone(), cur);
            }
            return;
        }
        if !missing_grid_flags.is_empty() {
            warn!(
                "backtests/full: clmm-lp-cli {} missing requested optimize-grid flags: {}",
                cli_path.display(),
                missing_grid_flags.join(", ")
            );
            let mut w = FULL_JOBS.write().await;
            if let Some(mut cur) = w.get(&job_id).cloned() {
                cur.status = "failed".to_string();
                cur.stderr = Some(format!(
                    "clmm-lp-cli at {} does not support requested optimize-grid flags: {}. Rebuild CLI (`cargo build --release -p clmm-lp-cli` or `cargo build -p clmm-lp-cli`) and restart API, or set CLMM_LP_CLI_PATH to the rebuilt binary.",
                    cli_path.display(),
                    missing_grid_flags.join(", ")
                ));
                cur.finished_ts_utc = Some(chrono::Utc::now().to_rfc3339());
                w.insert(job_id.clone(), cur);
            }
            return;
        }

        let mut all_results: Vec<BacktestFullWindowResult> = Vec::new();
        let mut errors: Vec<String> = Vec::new();

        for variant in &snapshot_variants {
            for pool in &pools {
                let snapshot_filename = match variant.suffix {
                    Some(s) => format!("snapshots_{s}.jsonl"),
                    None => "snapshots.jsonl".to_string(),
                };
                let snapshots_path = PathBuf::from("data")
                    .join("pool-snapshots")
                    .join(pool.protocol)
                    .join(pool.pool_address)
                    .join(snapshot_filename);
                if !snapshots_path.exists() {
                    errors.push(format!(
                        "{} {}: missing snapshots file {}",
                        variant.label,
                        pool.id,
                        snapshots_path.display()
                    ));
                    continue;
                }
                let bollinger_rebalance_steps_for_variant = match (
                    bollinger_rebalance_hours_grid.as_ref(),
                    bollinger_rebalance_steps_grid.as_ref(),
                ) {
                    (Some(hours), _) if !hours.is_empty() => {
                        Some(hours_grid_to_steps(hours, variant.cadence_minutes))
                    }
                    (_, Some(steps)) => Some(steps.clone()),
                    _ => None,
                };
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
                    if let Some(suffix) = variant.suffix {
                        cmd.arg("--snapshot-jsonl-suffix").arg(suffix);
                    }
                    cmd.arg("--hours").arg(h.to_string());
                if let Some(dev_pct) = static_deviation_pct.or(oor_recenter_deviation_pct) {
                    let width_pct = 2.0 * dev_pct;
                    cmd.arg("--min-range-pct").arg(width_pct.to_string());
                    cmd.arg("--max-range-pct").arg(width_pct.to_string());
                    cmd.arg("--range-steps").arg("1");
                } else {
                    cmd.arg("--min-range-pct").arg("1");
                    cmd.arg("--max-range-pct").arg("15");
                    cmd.arg("--range-steps").arg("10");
                }
                if let (Some(lower), Some(upper)) = (static_manual_lower, static_manual_upper) {
                    cmd.arg("--static-manual-lower").arg(lower.to_string());
                    cmd.arg("--static-manual-upper").arg(upper.to_string());
                }
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
                if let Some(v) = threshold_grid_pct.as_ref().filter(|v| !v.is_empty()) {
                    cmd.arg("--threshold-grid-pct").arg(
                        v.iter()
                            .map(|x| x.to_string())
                            .collect::<Vec<_>>()
                            .join(","),
                    );
                }
                if let Some(v) = threshold_min_rebalance_interval_hours {
                    cmd.arg("--threshold-min-rebalance-interval-hours")
                        .arg(v.to_string());
                }
                if let Some(v) = threshold_rebalance_on_range_exit_immediately {
                    if threshold_oor_flag_accepts_value {
                        cmd.arg("--threshold-rebalance-on-range-exit-immediately")
                            .arg(v.to_string());
                    } else if v {
                        // Backward compatibility for older CLI binaries where this is a switch-only flag.
                        cmd.arg("--threshold-rebalance-on-range-exit-immediately");
                    } else {
                        // Older CLI cannot express explicit false for this option.
                        // We skip the flag to avoid parse errors and let CLI defaults apply.
                        warn!(
                            "backtests/full: CLI at {} does not accept explicit false for --threshold-rebalance-on-range-exit-immediately; using CLI default",
                            cli_path.display()
                        );
                    }
                }
                if let Some(v) = periodic_grid_steps.as_ref().filter(|v| !v.is_empty()) {
                    cmd.arg("--periodic-grid-steps").arg(
                        v.iter()
                            .map(|x| x.to_string())
                            .collect::<Vec<_>>()
                            .join(","),
                    );
                }
                if let Some(v) = retouch_offset_pct {
                    cmd.arg("--retouch-offset-pct").arg(v.to_string());
                }
                if let Some(v) = bollinger_window_grid.as_ref().filter(|v| !v.is_empty()) {
                    cmd.arg("--bollinger-window-grid").arg(
                        v.iter()
                            .map(|x| x.to_string())
                            .collect::<Vec<_>>()
                            .join(","),
                    );
                }
                if let Some(v) = bollinger_k_grid.as_ref().filter(|v| !v.is_empty()) {
                    cmd.arg("--bollinger-k-grid").arg(
                        v.iter()
                            .map(|x| x.to_string())
                            .collect::<Vec<_>>()
                            .join(","),
                    );
                }
                if let Some(v) = bollinger_rebalance_steps_for_variant
                    .as_ref()
                    .filter(|v| !v.is_empty())
                {
                    cmd.arg("--bollinger-rebalance-steps-grid").arg(
                        v.iter()
                            .map(|x| x.to_string())
                            .collect::<Vec<_>>()
                            .join(","),
                    );
                }
                if let Some(v) = last_candle_steps_grid.as_ref().filter(|v| !v.is_empty()) {
                    cmd.arg("--last-candle-steps-grid").arg(
                        v.iter()
                            .map(|x| x.to_string())
                            .collect::<Vec<_>>()
                            .join(","),
                    );
                }
                if let Some(v) = last_candle_rebalance_steps_grid.as_ref().filter(|v| !v.is_empty()) {
                    cmd.arg("--last-candle-rebalance-steps-grid").arg(
                        v.iter()
                            .map(|x| x.to_string())
                            .collect::<Vec<_>>()
                            .join(","),
                    );
                }
                if let Some(v) = last_candle_seconds_grid.as_ref().filter(|v| !v.is_empty()) {
                    cmd.arg("--last-candle-seconds-grid").arg(
                        v.iter()
                            .map(|x| x.to_string())
                            .collect::<Vec<_>>()
                            .join(","),
                    );
                }
                if let Some(v) = last_candle_rebalance_seconds_grid.as_ref().filter(|v| !v.is_empty()) {
                    cmd.arg("--last-candle-rebalance-seconds-grid").arg(
                        v.iter()
                            .map(|x| x.to_string())
                            .collect::<Vec<_>>()
                            .join(","),
                    );
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
                                "{} {} {}h: exit {:?}: {}",
                                variant.label,
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
                            snapshot_variant: variant.label.to_string(),
                            window_hours: *h,
                            metrics,
                            note: None,
                        });
                    }
                    Err(e) => {
                        errors.push(format!(
                            "{} {} {}h: spawn error: {}",
                            variant.label, pool.id, h, e
                        ));
                    }
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

fn pick_auto_tune_winner(results: &[BacktestFullWindowResult]) -> Option<BacktestAutoTuneWinner> {
    let mut best: Option<BacktestAutoTuneWinner> = None;
    for r in results {
        for m in &r.metrics {
            let cand = BacktestAutoTuneWinner {
                pool_id: r.pool_id.clone(),
                pool_label: r.pool_label.clone(),
                window_hours: r.window_hours,
                strategy: m.strategy.clone(),
                width_pct: m.width_pct,
                score: m.score,
                pnl: m.pnl,
                vs_hodl: m.vs_hodl,
                fees: m.fees,
                rebalances: m.rebalances,
                tir_pct: m.tir_pct,
            };
            let replace = best
                .as_ref()
                .map(|b| cand.score > b.score)
                .unwrap_or(true);
            if replace {
                best = Some(cand);
            }
        }
    }
    best
}

#[utoipa::path(
    post,
    path = "/backtests/auto-tune/start",
    tag = "Analytics",
    request_body = BacktestAutoTuneStartRequest,
    responses((status = 200, description = "Auto-tune status", body = BacktestAutoTuneStatusResponse))
)]
pub async fn start_backtest_auto_tune(
    State(state): State<AppState>,
    Json(req): Json<BacktestAutoTuneStartRequest>,
) -> ApiResult<Json<BacktestAutoTuneStatusResponse>> {
    let interval_minutes = req.interval_minutes.unwrap_or(30).max(1);
    {
        let st = AUTO_TUNE_STATUS.read().await;
        if st.running {
            return Err(ApiError::Conflict(
                "Auto-tune is already running".to_string(),
            ));
        }
    }

    let stop = std::sync::Arc::new(AtomicBool::new(false));
    {
        let mut g = AUTO_TUNE_STOP.lock().expect("auto tune stop lock");
        *g = Some(stop.clone());
    }
    {
        let mut st = AUTO_TUNE_STATUS.write().await;
        st.running = true;
        st.interval_minutes = interval_minutes;
        st.started_ts_utc = Some(chrono::Utc::now().to_rfc3339());
        st.last_tick_ts_utc = None;
        st.next_tick_ts_utc = Some(
            (chrono::Utc::now() + chrono::Duration::minutes(interval_minutes as i64)).to_rfc3339(),
        );
        st.note = Some("Auto-tune loop started".to_string());
    }

    let state_c = state.clone();
    let full_req = req.full_request.clone();
    tokio::spawn(async move {
        while !stop.load(Ordering::Relaxed) {
            {
                let mut st = AUTO_TUNE_STATUS.write().await;
                st.last_tick_ts_utc = Some(chrono::Utc::now().to_rfc3339());
                st.next_tick_ts_utc = Some(
                    (chrono::Utc::now() + chrono::Duration::minutes(interval_minutes as i64))
                        .to_rfc3339(),
                );
                st.note = Some("Running full optimize cycle".to_string());
            }

            let started = start_backtest_full(State(state_c.clone()), Json(full_req.clone())).await;
            let Ok(Json(job_status)) = started else {
                let mut st = AUTO_TUNE_STATUS.write().await;
                st.note = Some("Failed to start full backtest job".to_string());
                sleep(Duration::from_secs(10)).await;
                continue;
            };

            {
                let mut st = AUTO_TUNE_STATUS.write().await;
                st.latest_job_id = Some(job_status.id.clone());
            }

            loop {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                let snapshot = {
                    let r = FULL_JOBS.read().await;
                    r.get(job_status.id.trim()).cloned()
                };
                if let Some(j) = snapshot
                    && j.status != "running"
                {
                    if j.status == "succeeded" || j.status == "partial" {
                        let mut st = AUTO_TUNE_STATUS.write().await;
                        if let Some(results) = j.results.as_ref() {
                            let winner = pick_auto_tune_winner(results);
                            st.latest_winner = winner;
                        }
                        st.note = if j.status == "partial" {
                            Some("Full optimize cycle completed (partial)".to_string())
                        } else {
                            Some("Full optimize cycle completed".to_string())
                        };
                    } else {
                        let mut st = AUTO_TUNE_STATUS.write().await;
                        st.note = Some("Full optimize cycle failed".to_string());
                    }
                    break;
                }
                sleep(Duration::from_secs(5)).await;
            }

            let mut slept = 0u64;
            let target = interval_minutes.saturating_mul(60);
            while slept < target && !stop.load(Ordering::Relaxed) {
                sleep(Duration::from_secs(5)).await;
                slept = slept.saturating_add(5);
            }
        }

        {
            let mut st = AUTO_TUNE_STATUS.write().await;
            st.running = false;
            st.next_tick_ts_utc = None;
            st.note = Some("Auto-tune loop stopped".to_string());
        }
        let mut g = AUTO_TUNE_STOP.lock().expect("auto tune stop lock");
        *g = None;
    });

    Ok(Json(AUTO_TUNE_STATUS.read().await.clone()))
}

#[utoipa::path(
    post,
    path = "/backtests/auto-tune/stop",
    tag = "Analytics",
    responses((status = 200, description = "Auto-tune status", body = BacktestAutoTuneStatusResponse))
)]
pub async fn stop_backtest_auto_tune() -> ApiResult<Json<BacktestAutoTuneStatusResponse>> {
    if let Some(flag) = AUTO_TUNE_STOP
        .lock()
        .expect("auto tune stop lock")
        .as_ref()
        .cloned()
    {
        flag.store(true, Ordering::Relaxed);
    }
    {
        let mut st = AUTO_TUNE_STATUS.write().await;
        st.note = Some("Stopping auto-tune loop...".to_string());
    }
    Ok(Json(AUTO_TUNE_STATUS.read().await.clone()))
}

#[utoipa::path(
    get,
    path = "/backtests/auto-tune/status",
    tag = "Analytics",
    responses((status = 200, description = "Auto-tune status", body = BacktestAutoTuneStatusResponse))
)]
pub async fn get_backtest_auto_tune_status() -> ApiResult<Json<BacktestAutoTuneStatusResponse>> {
    Ok(Json(AUTO_TUNE_STATUS.read().await.clone()))
}

#[utoipa::path(
    post,
    path = "/backtests/auto-tune/apply/{strategy_id}",
    tag = "Analytics",
    params(("strategy_id" = String, Path, description = "Strategy id to update")),
    responses((status = 200, description = "Apply result", body = BacktestAutoTuneApplyResponse))
)]
pub async fn apply_backtest_auto_tune_to_strategy(
    State(state): State<AppState>,
    Path(strategy_id): Path<String>,
) -> ApiResult<Json<BacktestAutoTuneApplyResponse>> {
    let winner = AUTO_TUNE_STATUS
        .read()
        .await
        .latest_winner
        .clone()
        .ok_or_else(|| ApiError::bad_request("No auto-tune winner available yet"))?;

    let mut strategies = state.strategies.write().await;
    let strategy = strategies
        .get_mut(strategy_id.trim())
        .ok_or_else(|| ApiError::not_found("Strategy not found"))?;
    let cfg_obj = strategy
        .config
        .as_object_mut()
        .ok_or_else(|| ApiError::internal("strategy config must be object"))?;

    let mut params = cfg_obj
        .get("parameters")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    if !params.is_object() {
        params = serde_json::json!({});
    }
    let p = params.as_object_mut().expect("object");
    p.insert(
        "range_width_pct".to_string(),
        serde_json::json!(winner.width_pct),
    );

    let s = winner.strategy.to_ascii_lowercase();
    let strategy_type = if s.starts_with("threshold_") {
        let pct_token = s
            .trim_start_matches("threshold_")
            .split('_')
            .next()
            .unwrap_or("0")
            .trim_end_matches('%');
        if let Ok(v) = pct_token.parse::<f64>() {
            p.insert("rebalance_threshold_pct".to_string(), serde_json::json!(v));
        }
        if let Some(ix) = s.find("_min")
            && let Some(rest) = s.get(ix + 4..)
            && let Some(hs) = rest.split('h').next()
            && let Ok(h) = hs.parse::<u64>()
        {
            p.insert(
                "min_rebalance_interval_minutes".to_string(),
                serde_json::json!(h.saturating_mul(60)),
            );
        }
        if s.contains("_oordelayed") {
            p.insert(
                "rebalance_on_range_exit_immediately".to_string(),
                serde_json::json!(false),
            );
        } else if s.contains("_oorimmediate") {
            p.insert(
                "rebalance_on_range_exit_immediately".to_string(),
                serde_json::json!(true),
            );
        }
        "threshold"
    } else if s.starts_with("periodic_") {
        let hs = s
            .trim_start_matches("periodic_")
            .trim_end_matches('h')
            .split('_')
            .next()
            .unwrap_or("24");
        if let Ok(h) = hs.parse::<u64>() {
            p.insert(
                "min_rebalance_interval_minutes".to_string(),
                serde_json::json!(h.saturating_mul(60)),
            );
        }
        "periodic"
    } else if s == "oor_recenter" {
        "oor_recenter"
    } else if s.starts_with("retouch_shift") {
        if let Some(rest) = s.strip_prefix("retouch_shift_off")
            && let Some(pct_s) = rest.strip_suffix("pct")
            && let Ok(pct) = pct_s.parse::<f64>()
        {
            p.insert("retouch_offset_pct".to_string(), serde_json::json!(pct));
        }
        "retouch_shift"
    } else if s.starts_with("il_limit_") {
        "il_limit"
    } else if s.starts_with("last_candle_") {
        "last_candle"
    } else {
        "static_range"
    };

    cfg_obj.insert("strategy_type".to_string(), serde_json::json!(strategy_type));
    cfg_obj.insert("parameters".to_string(), params);
    strategy.updated_at = chrono::Utc::now();
    let snapshot = strategies.clone();
    drop(strategies);
    crate::state::try_persist_strategies_best_effort(&snapshot);

    Ok(Json(BacktestAutoTuneApplyResponse {
        strategy_id,
        updated: true,
        note: "Applied latest auto-tune winner to strategy config. Restart strategy if running to reload executor config.".to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use super::{
        ReadinessConfig, SnapshotVariant, aggregate_readiness_status, classify_readiness_status,
        evaluate_snapshot_readiness, hours_grid_to_steps, parse_snapshot_variants,
        readiness_cache_key, snapshot_file_name_for_variant,
    };
    use crate::models::BacktestDataReadinessRow;
    use chrono::{Duration, Utc};
    use std::io::Write;

    #[test]
    fn parse_snapshot_variants_defaults_and_deduplicates() {
        let default_variants = parse_snapshot_variants(None).expect("default variants should parse");
        assert_eq!(default_variants.len(), 1);
        assert_eq!(default_variants[0].label, "10m");
        assert_eq!(default_variants[0].cadence_minutes, 10);

        let raw = vec!["5m".to_string(), "10m".to_string(), "5m".to_string()];
        let variants =
            parse_snapshot_variants(Some(&raw)).expect("provided variants should parse and dedupe");
        assert_eq!(variants.len(), 2);
        assert_eq!(variants[0].label, "10m");
        assert_eq!(variants[1].label, "5m");
    }

    #[test]
    fn parse_snapshot_variants_rejects_unknown_or_empty() {
        let empty = vec!["".to_string(), "   ".to_string()];
        assert!(parse_snapshot_variants(Some(&empty)).is_err());

        let bad = vec!["1m".to_string()];
        assert!(parse_snapshot_variants(Some(&bad)).is_err());
    }

    #[test]
    fn hours_grid_to_steps_converts_and_filters_invalid_values() {
        let hours = vec![2.0, 4.5, 0.0, -1.0, f64::NAN];
        let steps_10m = hours_grid_to_steps(&hours, 10);
        assert_eq!(steps_10m, vec![12, 27]);

        let steps_5m = hours_grid_to_steps(&[2.0], 5);
        assert_eq!(steps_5m, vec![24]);
    }

    #[test]
    fn readiness_cache_key_is_order_invariant() {
        let p1 = vec!["b".to_string(), "a".to_string()];
        let v1 = vec!["5m".to_string(), "10m".to_string()];
        let p2 = vec!["a".to_string(), "b".to_string()];
        let v2 = vec!["10m".to_string(), "5m".to_string()];
        assert_eq!(
            readiness_cache_key(&p1, &v1, None, None),
            readiness_cache_key(&p2, &v2, None, None)
        );
    }

    #[test]
    fn snapshot_file_name_matches_variant() {
        let v10 = SnapshotVariant {
            label: "10m",
            suffix: None,
            cadence_minutes: 10,
        };
        let v5 = SnapshotVariant {
            label: "5m",
            suffix: Some("5m"),
            cadence_minutes: 5,
        };
        assert_eq!(snapshot_file_name_for_variant(v10), "snapshots.jsonl");
        assert_eq!(snapshot_file_name_for_variant(v5), "snapshots_5m.jsonl");
    }

    #[test]
    fn evaluate_snapshot_readiness_handles_short_window() {
        let mut tmp = tempfile::NamedTempFile::new().expect("tmp file");
        // 6 points every 10 minutes => 50 minutes of continuous data.
        for i in 0..6 {
            let ts = format!("2026-01-01T00:{:02}:00Z", i * 10);
            let line = format!(r#"{{"ts_utc":"{ts}"}}"#);
            writeln!(tmp, "{line}").expect("write line");
        }

        let cfg = ReadinessConfig {
            cache_ttl_secs: 120,
            db_max_age_secs: 600,
            hard_gap_multiplier: 6,
            recommended_coverage_pct: 95.0,
            recommended_gap_multiplier: 2,
            recommended_fallback_ratio: 0.7,
        };
        let result =
            evaluate_snapshot_readiness(tmp.path(), 10, &cfg, None, None).expect("readiness");
        assert_eq!(result.0, 6);
        assert_eq!(result.6, 0);
        assert_eq!(result.7, 0);
        assert_eq!(result.8, None);
        assert!(result.5.unwrap_or(0.0) > 90.0);
    }

    #[test]
    fn evaluate_snapshot_readiness_applies_recommended_fallback_on_large_gap() {
        let mut tmp = tempfile::NamedTempFile::new().expect("tmp file");
        // Gap: 90 minutes between the first and second row.
        let lines = [
            r#"{"ts_utc":"2026-01-01T00:00:00Z"}"#,
            r#"{"ts_utc":"2026-01-01T01:30:00Z"}"#,
            r#"{"ts_utc":"2026-01-01T01:40:00Z"}"#,
            r#"{"ts_utc":"2026-01-01T01:50:00Z"}"#,
            r#"{"ts_utc":"2026-01-01T02:00:00Z"}"#,
            r#"{"ts_utc":"2026-01-01T02:10:00Z"}"#,
            r#"{"ts_utc":"2026-01-01T02:20:00Z"}"#,
        ];
        for line in lines {
            writeln!(tmp, "{line}").expect("write line");
        }

        let cfg = ReadinessConfig {
            cache_ttl_secs: 120,
            db_max_age_secs: 600,
            hard_gap_multiplier: 20,
            recommended_coverage_pct: 95.0,
            recommended_gap_multiplier: 2,
            recommended_fallback_ratio: 0.5,
        };
        let result =
            evaluate_snapshot_readiness(tmp.path(), 10, &cfg, None, None).expect("readiness");
        assert_eq!(result.6, 2);
        assert_eq!(result.7, 1);
        assert_eq!(result.8, None);
    }

    #[test]
    fn classify_readiness_status_marks_stale_rows_as_degraded() {
        let now = Utc::now();
        let row = BacktestDataReadinessRow {
            pool_id: "p".to_string(),
            pool_label: "p".to_string(),
            protocol: "orca".to_string(),
            pool_address: "x".to_string(),
            snapshot_variant: "10m".to_string(),
            cadence_minutes: 10,
            rows: 10,
            oldest_ts_utc: None,
            latest_ts_utc: Some((now - Duration::minutes(30)).to_rfc3339()),
            oldest_continuous_ts_utc: None,
            max_gap_minutes: Some(10.0),
            coverage_pct: Some(99.0),
            max_backtest_hours_hard: 10,
            max_backtest_hours_recommended: 10,
            status: "ok".to_string(),
            status_reason: None,
            latest_age_secs: None,
            note: None,
        };
        let (status, reason, age) = classify_readiness_status(&row, false, now);
        assert_eq!(status, "degraded");
        assert_eq!(reason.as_deref(), Some("stale_latest_snapshot"));
        assert!(age.unwrap_or_default() > 20 * 60);
    }

    #[test]
    fn classify_readiness_status_uses_range_logic_for_custom_window() {
        let now = Utc::now();
        let row = BacktestDataReadinessRow {
            pool_id: "p".to_string(),
            pool_label: "p".to_string(),
            protocol: "orca".to_string(),
            pool_address: "x".to_string(),
            snapshot_variant: "5m".to_string(),
            cadence_minutes: 5,
            rows: 50,
            oldest_ts_utc: None,
            latest_ts_utc: Some((now - Duration::hours(6)).to_rfc3339()),
            oldest_continuous_ts_utc: None,
            max_gap_minutes: Some(8.0),
            coverage_pct: Some(92.0),
            max_backtest_hours_hard: 24,
            max_backtest_hours_recommended: 0,
            status: "ok".to_string(),
            status_reason: None,
            latest_age_secs: None,
            note: Some("Selected range incomplete".to_string()),
        };
        let (status, reason, _) = classify_readiness_status(&row, true, now);
        assert_eq!(status, "degraded");
        assert_eq!(reason.as_deref(), Some("range_quality_below_recommended"));
    }

    #[test]
    fn aggregate_readiness_status_prioritizes_degraded_over_recovering() {
        let rows = vec![
            BacktestDataReadinessRow {
                pool_id: "a".to_string(),
                pool_label: "a".to_string(),
                protocol: "orca".to_string(),
                pool_address: "a".to_string(),
                snapshot_variant: "10m".to_string(),
                cadence_minutes: 10,
                rows: 1,
                oldest_ts_utc: None,
                latest_ts_utc: None,
                oldest_continuous_ts_utc: None,
                max_gap_minutes: None,
                coverage_pct: None,
                max_backtest_hours_hard: 0,
                max_backtest_hours_recommended: 0,
                status: "recovering".to_string(),
                status_reason: None,
                latest_age_secs: None,
                note: None,
            },
            BacktestDataReadinessRow {
                pool_id: "b".to_string(),
                pool_label: "b".to_string(),
                protocol: "orca".to_string(),
                pool_address: "b".to_string(),
                snapshot_variant: "10m".to_string(),
                cadence_minutes: 10,
                rows: 1,
                oldest_ts_utc: None,
                latest_ts_utc: None,
                oldest_continuous_ts_utc: None,
                max_gap_minutes: None,
                coverage_pct: None,
                max_backtest_hours_hard: 0,
                max_backtest_hours_recommended: 0,
                status: "degraded".to_string(),
                status_reason: None,
                latest_age_secs: None,
                note: None,
            },
        ];
        let (status, ok, degraded, recovering, missing) = aggregate_readiness_status(&rows);
        assert_eq!(status, "degraded");
        assert_eq!(ok, 0);
        assert_eq!(degraded, 1);
        assert_eq!(recovering, 1);
        assert_eq!(missing, 0);
    }
}
