//! Normalized market-data feeds from local JSONL files.

use crate::error::{ApiError, ApiResult};
use crate::models::{
    AgentDecisionRow, AgentDecisionWriteRequest, AgentDecisionWriteResponse, AgentDecisionsQuery,
    AgentDecisionsResponse, MarketDataQuery, MarketSnapshotRow, MarketSnapshotsResponse, MarketSwapRow,
    MarketSwapsResponse,
};
use axum::{Json, extract::Query};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

fn parse_decimal_opt(v: Option<&Value>) -> Option<Decimal> {
    match v {
        Some(Value::String(s)) => s.parse::<f64>().ok().and_then(Decimal::from_f64_retain),
        Some(Value::Number(n)) => n.as_f64().and_then(Decimal::from_f64_retain),
        _ => None,
    }
}

fn parse_ts_utc(v: &Value) -> Option<String> {
    let raw = v
        .get("ts_utc")
        .and_then(Value::as_str)
        .or_else(|| v.get("timestamp").and_then(Value::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    let dt = DateTime::parse_from_rfc3339(raw).ok()?;
    Some(dt.with_timezone(&Utc).to_rfc3339())
}

fn parse_time_bound(raw: &Option<String>) -> Result<Option<DateTime<Utc>>, ApiError> {
    let Some(v) = raw.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let dt = DateTime::parse_from_rfc3339(v)
        .map_err(|_| ApiError::bad_request(format!("Invalid RFC3339 timestamp: {v}")))?;
    Ok(Some(dt.with_timezone(&Utc)))
}

fn within_time_window(ts_utc: &str, from: Option<DateTime<Utc>>, to: Option<DateTime<Utc>>) -> bool {
    let Ok(dt_fixed) = DateTime::parse_from_rfc3339(ts_utc) else {
        return false;
    };
    let dt = dt_fixed.with_timezone(&Utc);
    if let Some(f) = from && dt < f {
        return false;
    }
    if let Some(t) = to && dt > t {
        return false;
    }
    true
}

fn collect_jsonl_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() {
            collect_jsonl_files(&p, out);
            continue;
        }
        if p.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            out.push(p);
        }
    }
}

fn file_name_contains(path: &Path, needle: &str) -> bool {
    path.file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase().contains(needle))
        .unwrap_or(false)
}

/// Read normalized snapshot rows from local JSONL store.
#[utoipa::path(
    get,
    path = "/data/snapshots",
    tag = "Data",
    params(
        ("protocol" = Option<String>, Query, description = "Protocol filter: orca|raydium|meteora"),
        ("pool" = Option<String>, Query, description = "Pool address filter"),
        ("from" = Option<String>, Query, description = "Lower RFC3339 timestamp bound"),
        ("to" = Option<String>, Query, description = "Upper RFC3339 timestamp bound"),
        ("limit" = Option<u32>, Query, description = "Max rows returned (default 500, max 10000)")
    ),
    responses(
        (status = 200, description = "Normalized snapshots feed", body = MarketSnapshotsResponse)
    )
)]
pub async fn get_data_snapshots(
    Query(q): Query<MarketDataQuery>,
) -> ApiResult<Json<MarketSnapshotsResponse>> {
    let from = parse_time_bound(&q.from)?;
    let to = parse_time_bound(&q.to)?;
    let limit = q.limit.unwrap_or(500).min(10_000) as usize;
    let protocol_filter = q.protocol.as_ref().map(|s| s.trim().to_ascii_lowercase());
    let pool_filter = q.pool.as_ref().map(|s| s.trim().to_string());

    let root = PathBuf::from("data").join("pool-snapshots");
    let mut files = Vec::new();
    collect_jsonl_files(&root, &mut files);
    files.retain(|p| file_name_contains(p, "snapshot"));

    let mut rows = Vec::<MarketSnapshotRow>::new();
    let scanned_files = files.len();
    for f in files {
        let comps: Vec<String> = f
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect();
        let Some(ix) = comps.iter().position(|c| c == "pool-snapshots") else {
            continue;
        };
        if comps.len() <= ix + 2 {
            continue;
        }
        let protocol = comps[ix + 1].to_ascii_lowercase();
        let pool_address = comps[ix + 2].clone();
        if let Some(ref p) = protocol_filter && &protocol != p {
            continue;
        }
        if let Some(ref p) = pool_filter && &pool_address != p {
            continue;
        }

        let file =
            fs::File::open(&f).map_err(|e| ApiError::internal(format!("open {}: {e}", f.display())))?;
        let reader = BufReader::new(file);
        for line in reader.lines().map_while(Result::ok) {
            let t = line.trim();
            if t.is_empty() {
                continue;
            }
            let Ok(v) = serde_json::from_str::<Value>(t) else {
                continue;
            };
            let Some(ts_utc) = parse_ts_utc(&v) else {
                continue;
            };
            if !within_time_window(&ts_utc, from, to) {
                continue;
            }
            rows.push(MarketSnapshotRow {
                ts_utc,
                protocol: protocol.clone(),
                pool_address: pool_address.clone(),
                source_path: f.to_string_lossy().to_string(),
                price_ab: parse_decimal_opt(v.get("price_ab")),
                liquidity_active_raw: v.get("liquidity_active").and_then(Value::as_u64).map(u128::from),
                position_id: None,
                chain_id: None,
                session_id: None,
            });
        }
    }

    rows.sort_by(|a, b| a.ts_utc.cmp(&b.ts_utc));
    if rows.len() > limit {
        rows = rows[rows.len() - limit..].to_vec();
    }

    Ok(Json(MarketSnapshotsResponse {
        scanned_files,
        rows_returned: rows.len(),
        rows,
    }))
}

/// Read normalized swap rows from local JSONL store.
#[utoipa::path(
    get,
    path = "/data/swaps",
    tag = "Data",
    params(
        ("protocol" = Option<String>, Query, description = "Protocol filter: orca|raydium|meteora"),
        ("pool" = Option<String>, Query, description = "Pool address filter"),
        ("from" = Option<String>, Query, description = "Lower RFC3339 timestamp bound"),
        ("to" = Option<String>, Query, description = "Upper RFC3339 timestamp bound"),
        ("limit" = Option<u32>, Query, description = "Max rows returned (default 500, max 10000)")
    ),
    responses(
        (status = 200, description = "Normalized swaps feed", body = MarketSwapsResponse)
    )
)]
pub async fn get_data_swaps(Query(q): Query<MarketDataQuery>) -> ApiResult<Json<MarketSwapsResponse>> {
    let from = parse_time_bound(&q.from)?;
    let to = parse_time_bound(&q.to)?;
    let limit = q.limit.unwrap_or(500).min(10_000) as usize;
    let protocol_filter = q.protocol.as_ref().map(|s| s.trim().to_ascii_lowercase());
    let pool_filter = q.pool.as_ref().map(|s| s.trim().to_string());

    let root = PathBuf::from("data").join("swaps");
    let mut files = Vec::new();
    collect_jsonl_files(&root, &mut files);
    files.retain(|p| file_name_contains(p, "swap"));

    let mut rows = Vec::<MarketSwapRow>::new();
    let scanned_files = files.len();
    for f in files {
        let comps: Vec<String> = f
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect();
        let Some(ix) = comps.iter().position(|c| c == "swaps") else {
            continue;
        };
        if comps.len() <= ix + 2 {
            continue;
        }
        let protocol = comps[ix + 1].to_ascii_lowercase();
        let pool_address = comps[ix + 2].clone();
        if let Some(ref p) = protocol_filter && &protocol != p {
            continue;
        }
        if let Some(ref p) = pool_filter && &pool_address != p {
            continue;
        }

        let file =
            fs::File::open(&f).map_err(|e| ApiError::internal(format!("open {}: {e}", f.display())))?;
        let reader = BufReader::new(file);
        for line in reader.lines().map_while(Result::ok) {
            let t = line.trim();
            if t.is_empty() {
                continue;
            }
            let Ok(v) = serde_json::from_str::<Value>(t) else {
                continue;
            };
            let Some(ts_utc) = parse_ts_utc(&v) else {
                continue;
            };
            if !within_time_window(&ts_utc, from, to) {
                continue;
            }
            rows.push(MarketSwapRow {
                ts_utc,
                protocol: protocol.clone(),
                pool_address: pool_address.clone(),
                source_path: f.to_string_lossy().to_string(),
                tx_signature: v
                    .get("signature")
                    .and_then(Value::as_str)
                    .or_else(|| v.get("tx_signature").and_then(Value::as_str))
                    .map(ToString::to_string),
                amount_in: parse_decimal_opt(v.get("amount_in")),
                amount_out: parse_decimal_opt(v.get("amount_out")),
                fee_usd: parse_decimal_opt(v.get("fee_usd")),
                position_id: None,
                chain_id: None,
                session_id: v
                    .get("rebalance_session_id")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
            });
        }
    }

    rows.sort_by(|a, b| a.ts_utc.cmp(&b.ts_utc));
    if rows.len() > limit {
        rows = rows[rows.len() - limit..].to_vec();
    }

    Ok(Json(MarketSwapsResponse {
        scanned_files,
        rows_returned: rows.len(),
        rows,
    }))
}

/// Read persisted agent decisions from local JSONL store.
#[utoipa::path(
    get,
    path = "/data/agent/decisions",
    tag = "Data",
    params(
        ("strategy_id" = Option<String>, Query, description = "Strategy id filter"),
        ("source" = Option<String>, Query, description = "Decision source filter"),
        ("from" = Option<String>, Query, description = "Lower RFC3339 timestamp bound"),
        ("to" = Option<String>, Query, description = "Upper RFC3339 timestamp bound"),
        ("limit" = Option<u32>, Query, description = "Max rows returned (default 500, max 10000)")
    ),
    responses(
        (status = 200, description = "Persisted agent decisions", body = AgentDecisionsResponse)
    )
)]
pub async fn get_agent_decisions(
    Query(q): Query<AgentDecisionsQuery>,
) -> ApiResult<Json<AgentDecisionsResponse>> {
    let from = parse_time_bound(&q.from)?;
    let to = parse_time_bound(&q.to)?;
    let limit = q.limit.unwrap_or(500).min(10_000) as usize;
    let strategy_filter = q.strategy_id.as_ref().map(|s| s.trim().to_string());
    let source_filter = q.source.as_ref().map(|s| s.trim().to_ascii_lowercase());
    let path = PathBuf::from("data").join("agent").join("agent_decisions.jsonl");
    if !path.exists() {
        return Ok(Json(AgentDecisionsResponse {
            path: path.to_string_lossy().to_string(),
            file_missing: true,
            rows_returned: 0,
            rows: Vec::new(),
        }));
    }

    let file = fs::File::open(&path)
        .map_err(|e| ApiError::internal(format!("open {}: {e}", path.display())))?;
    let reader = BufReader::new(file);
    let mut rows = Vec::<AgentDecisionRow>::new();
    for line in reader.lines().map_while(Result::ok) {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let Ok(row) = serde_json::from_str::<AgentDecisionRow>(t) else {
            continue;
        };
        if !within_time_window(&row.ts_utc, from, to) {
            continue;
        }
        if let Some(ref sf) = strategy_filter
            && row.strategy_id.as_deref() != Some(sf.as_str())
        {
            continue;
        }
        if let Some(ref src) = source_filter
            && row.source.to_ascii_lowercase() != *src
        {
            continue;
        }
        rows.push(row);
    }
    rows.sort_by(|a, b| a.ts_utc.cmp(&b.ts_utc));
    if rows.len() > limit {
        rows = rows[rows.len() - limit..].to_vec();
    }

    Ok(Json(AgentDecisionsResponse {
        path: path.to_string_lossy().to_string(),
        file_missing: false,
        rows_returned: rows.len(),
        rows,
    }))
}

/// Append one agent decision row to local JSONL store.
#[utoipa::path(
    post,
    path = "/data/agent/decisions",
    tag = "Data",
    request_body = AgentDecisionWriteRequest,
    responses(
        (status = 200, description = "Agent decision appended", body = AgentDecisionWriteResponse)
    )
)]
pub async fn post_agent_decision(
    Json(req): Json<AgentDecisionWriteRequest>,
) -> ApiResult<Json<AgentDecisionWriteResponse>> {
    let source = req.source.trim().to_string();
    if source.is_empty() {
        return Err(ApiError::bad_request("source cannot be empty"));
    }
    let ts_utc = match req.ts_utc.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(v) => DateTime::parse_from_rfc3339(v)
            .map_err(|_| ApiError::bad_request(format!("Invalid RFC3339 timestamp: {v}")))?
            .with_timezone(&Utc)
            .to_rfc3339(),
        None => Utc::now().to_rfc3339(),
    };

    let row = AgentDecisionRow {
        ts_utc,
        source,
        strategy_id: req.strategy_id.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
        position_id: req.position_id.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
        chain_id: req.chain_id.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
        session_id: req.session_id.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
        decision: req.decision,
    };

    let dir = PathBuf::from("data").join("agent");
    fs::create_dir_all(&dir)
        .map_err(|e| ApiError::internal(format!("create_dir_all {}: {e}", dir.display())))?;
    let path = dir.join("agent_decisions.jsonl");
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| ApiError::internal(format!("open {}: {e}", path.display())))?;
    let line = serde_json::to_string(&row)
        .map_err(|e| ApiError::internal(format!("serialize decision row: {e}")))?;
    writeln!(file, "{line}")
        .map_err(|e| ApiError::internal(format!("write {}: {e}", path.display())))?;

    Ok(Json(AgentDecisionWriteResponse {
        path: path.to_string_lossy().to_string(),
        written: true,
        row,
    }))
}
