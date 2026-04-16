//! Read append-only JSONL ledgers (Orca lifecycle + position registry) for dashboard / Slack digests.

use crate::error::{ApiError, ApiResult};
use crate::models::{
    BotActivityJsonlResponse, BotRegistryJsonlResponse, PendingOpenRecoveryResponse,
    SlackActivitySummaryRequest, SlackActivitySummaryResponse, StrandedRebalancesResponse,
};
use crate::state::AppState;
use axum::{
    Json,
    extract::{Path as AxumPath, Query, State},
};
use clmm_lp_protocols::ledger::position_registry::registry_path;
use clmm_lp_protocols::ledger::tx_lifecycle::{il_ledger_path_from_env, ledger_read_path};
use serde::Deserialize;
use std::path::Path;
use std::sync::LazyLock;

static REQWEST: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .expect("reqwest client")
});

const MAX_LEDGER_ROWS: usize = 2000;

#[derive(Debug, Deserialize)]
pub struct BotJsonlQuery {
    #[serde(default = "default_limit")]
    limit: usize,
    /// How many newest matching rows to skip (pagination). `0` = newest page.
    #[serde(default)]
    offset: usize,
    /// If set, only lines whose JSON string contains this substring (e.g. position pubkey).
    filter: Option<String>,
}

fn default_limit() -> usize {
    200
}

fn clamp_limit(n: usize) -> usize {
    n.clamp(1, MAX_LEDGER_ROWS)
}

/// Read tail of the lifecycle JSONL used for dashboard summaries.
///
/// Uses [`clmm_lp_protocols::ledger::tx_lifecycle::ledger_read_path`] (see
/// `CLMM_POSITION_LIFECYCLE_LEDGER_READ_PATH`, `CLMM_POSITION_LIFECYCLE_USE_ENRICHED`, and canonical
/// `CLMM_POSITION_LIFECYCLE_LEDGER_PATH` for writes).
#[utoipa::path(
    get,
    path = "/bot-activity/ledger",
    tag = "Bot activity",
    params(
        ("limit" = Option<usize>, Query, description = "Max rows (1–2000, default 200)"),
        ("filter" = Option<String>, Query, description = "Substring filter on JSON text"),
    ),
    responses(
        (status = 200, description = "Ledger rows", body = BotActivityJsonlResponse)
    )
)]
pub async fn get_bot_ledger(
    State(_state): State<AppState>,
    Query(q): Query<BotJsonlQuery>,
) -> ApiResult<Json<BotActivityJsonlResponse>> {
    let path = ledger_read_path();
    let limit = clamp_limit(q.limit);
    let offset = q.offset;
    let filter = q.filter.as_deref().map(str::trim).filter(|s| !s.is_empty());

    let (file_missing, total_matching, rows) =
        read_jsonl_tail(path.as_path(), limit, offset, filter)
            .map_err(|e| ApiError::internal(format!("read ledger file: {e}")))?;
    let path_s = path.display().to_string();

    Ok(Json(BotActivityJsonlResponse {
        path: path_s,
        file_missing,
        total_matching_lines: total_matching,
        rows_returned: rows.len(),
        rows,
    }))
}

/// Read tail of the IL / rebalance JSONL file when **`CLMM_IL_LEDGER_PATH`** is set (same path as `orca-bot-run --il-ledger-path`).
///
/// Rows use `event: "rebalance"` (and related IL schema); filter matches substring on JSON text (e.g. position or old position pubkey).
#[utoipa::path(
    get,
    path = "/bot-activity/il-ledger",
    tag = "Bot activity",
    params(
        ("limit" = Option<usize>, Query, description = "Max rows (1–2000, default 200)"),
        ("filter" = Option<String>, Query, description = "Substring filter on JSON text"),
    ),
    responses(
        (status = 200, description = "IL ledger rows", body = BotActivityJsonlResponse)
    )
)]
pub async fn get_bot_il_ledger(
    State(_state): State<AppState>,
    Query(q): Query<BotJsonlQuery>,
) -> ApiResult<Json<BotActivityJsonlResponse>> {
    let limit = clamp_limit(q.limit);
    let offset = q.offset;
    let filter = q.filter.as_deref().map(str::trim).filter(|s| !s.is_empty());

    let Some(path) = il_ledger_path_from_env() else {
        return Ok(Json(BotActivityJsonlResponse {
            path: "(unset: CLMM_IL_LEDGER_PATH — same file as orca-bot-run --il-ledger-path)"
                .to_string(),
            file_missing: true,
            total_matching_lines: 0,
            rows_returned: 0,
            rows: Vec::new(),
        }));
    };

    let path_s = path.display().to_string();
    let (file_missing, total_matching, rows) =
        read_jsonl_tail(path.as_path(), limit, offset, filter)
            .map_err(|e| ApiError::internal(format!("read IL ledger file: {e}")))?;

    Ok(Json(BotActivityJsonlResponse {
        path: path_s,
        file_missing,
        total_matching_lines: total_matching,
        rows_returned: rows.len(),
        rows,
    }))
}

/// Read tail of `data/positions/registry.jsonl` (override: `CLMM_POSITION_REGISTRY_PATH`).
#[utoipa::path(
    get,
    path = "/bot-activity/registry",
    tag = "Bot activity",
    params(
        ("limit" = Option<usize>, Query, description = "Max rows (1–2000, default 200)"),
        ("filter" = Option<String>, Query, description = "Substring filter on JSON text"),
    ),
    responses(
        (status = 200, description = "Registry rows", body = BotRegistryJsonlResponse)
    )
)]
pub async fn get_bot_registry(
    State(_state): State<AppState>,
    Query(q): Query<BotJsonlQuery>,
) -> ApiResult<Json<BotRegistryJsonlResponse>> {
    let path = registry_path();
    let limit = clamp_limit(q.limit);
    let offset = q.offset;
    let filter = q.filter.as_deref().map(str::trim).filter(|s| !s.is_empty());

    let (file_missing, total_matching, rows) =
        read_jsonl_tail(path.as_path(), limit, offset, filter)
            .map_err(|e| ApiError::internal(format!("read registry file: {e}")))?;
    let path_s = path.display().to_string();

    Ok(Json(BotRegistryJsonlResponse {
        path: path_s,
        file_missing,
        total_matching_lines: total_matching,
        rows_returned: rows.len(),
        rows,
    }))
}

/// Read pending-open recovery JSON (`CLMM_PENDING_OPEN_RECOVERY_PATH`).
#[utoipa::path(
    get,
    path = "/bot-activity/pending-open",
    tag = "Bot activity",
    responses(
        (status = 200, description = "Pending open recovery document", body = PendingOpenRecoveryResponse)
    )
)]
pub async fn get_pending_open_recovery(
    State(_state): State<AppState>,
) -> ApiResult<Json<PendingOpenRecoveryResponse>> {
    let path = std::env::var("CLMM_PENDING_OPEN_RECOVERY_PATH")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "data/pending-open-recovery.json".to_string());

    let p = std::path::PathBuf::from(&path);
    if !p.exists() {
        return Ok(Json(PendingOpenRecoveryResponse {
            path,
            file_missing: true,
            data: None,
        }));
    }

    let txt = std::fs::read_to_string(&p)
        .map_err(|e| ApiError::internal(format!("read pending-open file: {e}")))?;
    let trimmed = txt.trim();
    if trimmed.is_empty() {
        return Ok(Json(PendingOpenRecoveryResponse {
            path,
            file_missing: false,
            data: Some(serde_json::json!({})),
        }));
    }
    let v: serde_json::Value = serde_json::from_str(trimmed)
        .map_err(|e| ApiError::internal(format!("parse pending-open JSON: {e}")))?;

    Ok(Json(PendingOpenRecoveryResponse {
        path,
        file_missing: false,
        data: Some(v),
    }))
}

#[utoipa::path(
    get,
    path = "/bot-activity/stranded-rebalances",
    tag = "Bot activity",
    responses(
        (status = 200, description = "Detected rebalance sessions with close and missing open", body = StrandedRebalancesResponse)
    )
)]
pub async fn get_stranded_rebalances(
    State(_state): State<AppState>,
) -> ApiResult<Json<StrandedRebalancesResponse>> {
    let r = crate::services::stranded_rebalance_watchdog::get_stranded_rebalances_snapshot()
        .map_err(|e| ApiError::internal(format!("stranded rebalances: {e}")))?;
    Ok(Json(r))
}

#[utoipa::path(
    post,
    path = "/bot-activity/stranded-rebalances/reconcile",
    tag = "Bot activity",
    responses(
        (status = 200, description = "Auto-enqueue eligible stranded sessions into pending-open recovery", body = StrandedRebalancesResponse)
    )
)]
pub async fn reconcile_stranded_rebalances(
    State(_state): State<AppState>,
) -> ApiResult<Json<StrandedRebalancesResponse>> {
    match crate::services::stranded_rebalance_watchdog::reconcile_stranded_rebalances_for_api(
        "watchdog auto-enqueue from POST /bot-activity/stranded-rebalances/reconcile",
    ) {
        Ok(r) => Ok(Json(r)),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("CLMM_IL_LEDGER_PATH") {
                Err(ApiError::bad_request(msg))
            } else {
                Err(ApiError::internal(msg))
            }
        }
    }
}

#[utoipa::path(
    post,
    path = "/bot-activity/stranded-rebalances/{session_id}/dismiss",
    tag = "Bot activity",
    params(
        ("session_id" = String, Path, description = "Rebalance session id to dismiss")
    ),
    responses(
        (status = 200, description = "Session dismissed and removed from pending-open automation", body = StrandedRebalancesResponse)
    )
)]
pub async fn dismiss_stranded_rebalance(
    State(_state): State<AppState>,
    AxumPath(session_id): AxumPath<String>,
) -> ApiResult<Json<StrandedRebalancesResponse>> {
    let sid = session_id.trim();
    if sid.is_empty() {
        return Err(ApiError::bad_request("session_id is required"));
    }
    let r =
        crate::services::stranded_rebalance_watchdog::dismiss_stranded_rebalance_session_for_api(
            sid,
        )
        .map_err(|e| ApiError::internal(format!("dismiss stranded rebalance: {e}")))?;
    Ok(Json(r))
}

/// Post a short plain-text digest of recent lifecycle ledger rows to Slack (`SLACK_WEBHOOK_URL`).
#[utoipa::path(
    post,
    path = "/bot-activity/slack-summary",
    tag = "Bot activity",
    request_body = SlackActivitySummaryRequest,
    responses(
        (status = 200, description = "Posted or skipped", body = SlackActivitySummaryResponse)
    )
)]
pub async fn post_bot_slack_summary(
    State(_state): State<AppState>,
    Json(body): Json<SlackActivitySummaryRequest>,
) -> ApiResult<Json<SlackActivitySummaryResponse>> {
    let webhook = std::env::var("SLACK_WEBHOOK_URL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    if webhook.is_none() {
        return Ok(Json(SlackActivitySummaryResponse {
            ok: false,
            error: Some(
                "SLACK_WEBHOOK_URL is not set (same env as tools/notify_slack_webhook.ps1)".into(),
            ),
            rows_included: 0,
            webhook_configured: false,
        }));
    }
    let webhook = webhook.expect("checked");

    let lim = body.limit.clamp(1, 80);
    let path = ledger_read_path();
    let (file_missing, _total, rows) = read_jsonl_tail(path.as_path(), lim, 0, None)
        .map_err(|e| ApiError::internal(format!("read ledger: {e}")))?;

    if file_missing {
        return Ok(Json(SlackActivitySummaryResponse {
            ok: false,
            error: Some(format!("ledger file missing: {}", path.display())),
            rows_included: 0,
            webhook_configured: true,
        }));
    }

    let text = format_slack_digest(&path.display().to_string(), &rows);
    let payload = serde_json::json!({
        "text": text,
        "username": "clmm-lp-bot-activity",
    });

    let resp = REQWEST
        .post(&webhook)
        .json(&payload)
        .send()
        .await
        .map_err(|e| ApiError::internal(format!("Slack POST: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Ok(Json(SlackActivitySummaryResponse {
            ok: false,
            error: Some(format!("Slack returned {status}: {body}")),
            rows_included: rows.len(),
            webhook_configured: true,
        }));
    }

    Ok(Json(SlackActivitySummaryResponse {
        ok: true,
        error: None,
        rows_included: rows.len(),
        webhook_configured: true,
    }))
}

fn read_jsonl_tail(
    path: &Path,
    limit: usize,
    offset: usize,
    filter: Option<&str>,
) -> std::io::Result<(bool, usize, Vec<serde_json::Value>)> {
    if !path.exists() {
        return Ok((true, 0, Vec::new()));
    }

    let content = std::fs::read_to_string(path)?;
    let mut parsed: Vec<serde_json::Value> = Vec::new();

    for line in content.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(t) else {
            continue;
        };
        if let Some(f) = filter
            && !v.to_string().contains(f)
        {
            continue;
        }
        parsed.push(v);
    }

    let total = parsed.len();
    if total == 0 {
        return Ok((false, 0, Vec::new()));
    }
    let end_exclusive = total.saturating_sub(offset).min(total);
    let start_inclusive = end_exclusive.saturating_sub(limit);
    let out = parsed[start_inclusive..end_exclusive].to_vec();

    Ok((false, total, out))
}

fn format_slack_digest(ledger_path: &str, rows: &[serde_json::Value]) -> String {
    use std::fmt::Write;

    let mut s = String::new();
    let _ = writeln!(
        &mut s,
        "Bociarz LP — Orca lifecycle ledger (last {} rows)\n{}",
        rows.len(),
        ledger_path
    );

    if rows.is_empty() {
        let _ = writeln!(&mut s, "(no rows)");
        return s;
    }

    for v in rows {
        let ts = v.get("ts_utc").and_then(|x| x.as_str()).unwrap_or("?");
        let source = v.get("source").and_then(|x| x.as_str()).unwrap_or("?");
        let event = v.get("event").and_then(|x| x.as_str()).unwrap_or("?");
        let sig = v
            .get("signature")
            .and_then(|x| x.as_str())
            .map(|x| {
                if x.len() > 12 {
                    format!("{}…", &x[..12])
                } else {
                    x.to_string()
                }
            })
            .unwrap_or_else(|| "-".to_string());
        let fee = v
            .get("tx_fee_lamports")
            .and_then(|x| x.as_u64())
            .map(|n| n.to_string())
            .unwrap_or_else(|| "-".to_string());
        let pos = v
            .get("position_pda")
            .and_then(|x| x.as_str())
            .or_else(|| v.get("position_pubkey").and_then(|x| x.as_str()));

        let pos_short = pos.map(|p| {
            if p.len() > 8 {
                format!("{}…", &p[..8])
            } else {
                p.to_string()
            }
        });

        let _ = writeln!(
            &mut s,
            "• `{}` {} | {} | fee {} lamports | sig `{}`{}",
            ts,
            source,
            event,
            fee,
            sig,
            pos_short
                .map(|p| format!(" | pos `{p}`"))
                .unwrap_or_default()
        );
    }

    if s.len() > 3500 {
        s.truncate(3490);
        s.push_str("\n…");
    }

    s
}
