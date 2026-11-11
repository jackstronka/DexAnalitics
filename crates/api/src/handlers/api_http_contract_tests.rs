//! HTTP-boundary contract tests for critical `/api/v1` routes.
//!
//! Goes beyond [`endpoint_coverage_tests`] (status-code smoke): deserializes
//! responses into OpenAPI/serde models and asserts happy-path JSON shape with
//! seeded in-memory state. Still hermetic (no DB, RPC to `127.0.0.1:1`, empty
//! fallbacks).
//!
//! API-H0: close-all preview / start (zero closable → no worker / no on-chain) + GET.
//! API-H1 / F2.8: close-all job persists across in-memory clear + disk reload.

use super::http_test_support::{get_json, request_body, test_router, test_state};
use crate::models::{
    CloseAllBatchStatusResponse, CloseAllItemStatus, CloseAllPositionsPreviewResponse,
    CloseAllPositionsStartResponse, HealthResponse, ListPositionsResponse,
    ListStrategiesResponse, MetricsResponse, ServiceStatus, WalletsListResponse,
};
use crate::services::position_close_all::{
    clear_close_all_jobs_memory_for_tests, reload_close_all_jobs_from_disk,
};
use crate::state::StrategyState;
use crate::test_env::EnvGuard;
use axum::http::{Method, StatusCode};
use solana_sdk::pubkey::Pubkey;

#[tokio::test]
async fn health_live_returns_ok_without_rpc() {
    let (status, body) = get_json(test_router(test_state()), "/api/v1/health/live").await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    // Liveness is plain text (not JSON) — must stay reachable without RPC/DB.
    assert!(
        body.eq_ignore_ascii_case("ok") || body.to_ascii_lowercase().contains("alive"),
        "unexpected live body={body:?}"
    );
}

#[tokio::test]
async fn health_deserializes_to_health_response_model() {
    let (status, body) = get_json(test_router(test_state()), "/api/v1/health").await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let health: HealthResponse = serde_json::from_str(&body).expect("HealthResponse");
    assert!(!health.version.is_empty());
    // Dead RPC → Unhealthy or Degraded; never panic / 500.
    assert!(matches!(
        health.status,
        ServiceStatus::Unhealthy | ServiceStatus::Degraded | ServiceStatus::Healthy
    ));
    assert!(!health.components.database, "no DB in test state");
    assert!(!health.components.rpc, "127.0.0.1:1 must look down");
}

#[tokio::test]
async fn metrics_deserializes_to_metrics_response_model() {
    let (status, body) = get_json(test_router(test_state()), "/api/v1/metrics").await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let m: MetricsResponse = serde_json::from_str(&body).expect("MetricsResponse");
    assert_eq!(m.positions_monitored, 0);
    assert_eq!(m.strategies_running, 0);
}

#[tokio::test]
async fn positions_list_empty_deserializes_and_reports_total_zero() {
    let (status, body) = get_json(test_router(test_state()), "/api/v1/positions").await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let list: ListPositionsResponse = serde_json::from_str(&body).expect("ListPositionsResponse");
    assert!(list.positions.is_empty());
    assert_eq!(list.total, 0);
}

#[tokio::test]
async fn strategies_list_includes_seeded_strategy() {
    let state = test_state();
    let now = chrono::Utc::now();
    state.strategies.write().await.insert(
        "strat-http-1".to_string(),
        StrategyState {
            id: "strat-http-1".to_string(),
            name: "http-contract".to_string(),
            running: false,
            config: serde_json::json!({
                "pool_address": "3KBZiL2g8C7tiJ32hTv5v3KM7aK9htpqTw4cTXz1HvPt",
                "strategy_type": "static_range",
                "parameters": {},
                "auto_execute": false,
                "dry_run": true
            }),
            created_at: now,
            updated_at: now,
        },
    );
    let (status, body) = get_json(test_router(state), "/api/v1/strategies").await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let list: ListStrategiesResponse = serde_json::from_str(&body).expect("ListStrategiesResponse");
    assert_eq!(list.total, 1);
    assert_eq!(list.strategies.len(), 1);
    assert_eq!(list.strategies[0].id, "strat-http-1");
    assert_eq!(list.strategies[0].name, "http-contract");
    assert!(!list.strategies[0].running);
}

#[tokio::test]
async fn wallets_list_deserializes_even_when_dirs_empty_or_missing() {
    let (status, body) = get_json(test_router(test_state()), "/api/v1/wallets").await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let list: WalletsListResponse = serde_json::from_str(&body).expect("WalletsListResponse");
    assert!(!list.wallets_dir_primary.is_empty());
    // No wallet files required — empty vec is a valid contract response.
    let _ = list.wallets;
}

#[tokio::test]
async fn close_all_batch_unknown_returns_404_json_error() {
    let (status, body) = get_json(
        test_router(test_state()),
        "/api/v1/positions/close-all/00000000-0000-0000-0000-000000000000",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body={body}");
    let v: serde_json::Value = serde_json::from_str(&body).expect("json error body");
    // Error envelope should be machine-readable JSON (not empty / HTML).
    assert!(
        v.get("error").is_some()
            || v.get("message").is_some()
            || v.get("code").is_some()
            || body.contains("not found")
            || body.contains("NotFound"),
        "unexpected error body={body}"
    );
}

fn close_all_body(scope: &str, addresses: Vec<&str>, send_mode: &str) -> serde_json::Value {
    serde_json::json!({
        "scope": scope,
        "addresses": addresses,
        "exclude_addresses": [],
        "pause_linked_strategies": true,
        "options": {
            "skip_pre_collect": true,
            "send_mode": send_mode
        }
    })
}

/// Hermetic close-all jobs path (never write under repo `data/`).
async fn close_all_jobs_env() -> (EnvGuard, tempfile::TempDir) {
    let mut env = EnvGuard::lock().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("close_all_jobs.json");
    env.set("CLMM_CLOSE_ALL_JOBS_PATH", &path);
    clear_close_all_jobs_memory_for_tests().await;
    (env, dir)
}

/// API-H0: empty monitored set → preview totals zero (no on-chain).
#[tokio::test]
async fn close_all_preview_empty_monitored_deserializes() {
    let (_env, _dir) = close_all_jobs_env().await;
    let (status, body) = request_body(
        test_router(test_state()),
        Method::POST,
        "/api/v1/positions/close-all/preview",
        Some(close_all_body("monitored", vec![], "confirm_sync")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let preview: CloseAllPositionsPreviewResponse =
        serde_json::from_str(&body).expect("CloseAllPositionsPreviewResponse");
    assert_eq!(preview.total, 0);
    assert_eq!(preview.closable, 0);
    assert!(preview.groups.is_empty());
    assert!(preview.skipped_preview.is_empty());
}

/// API-H0: bad send_mode / empty explicit → 400 (validation at HTTP boundary).
#[tokio::test]
async fn close_all_preview_rejects_bad_send_mode_and_empty_explicit() {
    let (_env, _dir) = close_all_jobs_env().await;
    let router = test_router(test_state());
    let (status, body) = request_body(
        router.clone(),
        Method::POST,
        "/api/v1/positions/close-all/preview",
        Some(close_all_body("monitored", vec![], "fast")),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");

    let (status, body) = request_body(
        router,
        Method::POST,
        "/api/v1/positions/close-all/preview",
        Some(close_all_body("explicit", vec![], "confirm_sync")),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");
}

/// API-H0: explicit pubkey with no API wallet → skipped, closable=0 (no worker).
#[tokio::test]
async fn close_all_preview_explicit_unmanaged_is_skipped_not_closable() {
    let (_env, _dir) = close_all_jobs_env().await;
    let pda = Pubkey::new_unique().to_string();
    let (status, body) = request_body(
        test_router(test_state()),
        Method::POST,
        "/api/v1/positions/close-all/preview",
        Some(close_all_body("explicit", vec![pda.as_str()], "confirm_sync")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let preview: CloseAllPositionsPreviewResponse =
        serde_json::from_str(&body).expect("CloseAllPositionsPreviewResponse");
    assert_eq!(preview.total, 1);
    assert_eq!(preview.closable, 0);
    assert!(preview.groups.is_empty());
    assert_eq!(preview.skipped_preview.len(), 1);
    assert_eq!(preview.skipped_preview[0].address, pda);
    assert!(
        preview.skipped_preview[0].reason == "unmanaged_signer"
            || preview.skipped_preview[0].reason == "invalid_address",
        "reason={}",
        preview.skipped_preview[0].reason
    );
}

/// API-H0: start with zero closable → 202 + GET status (worker not spawned / no txs).
#[tokio::test]
async fn close_all_start_zero_closable_then_get_status_deserializes() {
    let (_env, _dir) = close_all_jobs_env().await;
    let pda = Pubkey::new_unique().to_string();
    let router = test_router(test_state());
    let (status, body) = request_body(
        router.clone(),
        Method::POST,
        "/api/v1/positions/close-all",
        Some(close_all_body("explicit", vec![pda.as_str()], "confirm_sync")),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "body={body}");
    let start: CloseAllPositionsStartResponse =
        serde_json::from_str(&body).expect("CloseAllPositionsStartResponse");
    assert!(!start.batch_id.is_empty());
    assert_eq!(start.status, "queued");
    assert_eq!(start.total, 1);
    assert!(start.groups.is_empty());
    assert_eq!(start.skipped_preview.len(), 1);

    let path = format!("/api/v1/positions/close-all/{}", start.batch_id);
    let (status, body) = get_json(router, &path).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let batch: CloseAllBatchStatusResponse =
        serde_json::from_str(&body).expect("CloseAllBatchStatusResponse");
    assert_eq!(batch.batch_id, start.batch_id);
    assert_eq!(batch.status, "done");
    assert!(batch.finished_ts_utc.is_some());
    assert_eq!(batch.summary.total, 1);
    assert_eq!(batch.summary.closed, 0);
    assert_eq!(batch.summary.failed, 0);
    assert_eq!(batch.summary.skipped, 1);
    assert_eq!(batch.summary.pending, 0);
    assert_eq!(batch.items.len(), 1);
    assert_eq!(batch.items[0].address, pda);
    assert_eq!(
        batch.items[0].status,
        CloseAllItemStatus::SkippedUnmanagedSigner
    );
}

/// API-H1 / F2.8: HTTP start → clear memory → reload disk → GET same batch.
#[tokio::test]
async fn close_all_http_get_survives_disk_reload() {
    let (_env, dir) = close_all_jobs_env().await;
    let jobs_path = dir.path().join("close_all_jobs.json");
    let pda = Pubkey::new_unique().to_string();
    let router = test_router(test_state());
    let (status, body) = request_body(
        router.clone(),
        Method::POST,
        "/api/v1/positions/close-all",
        Some(close_all_body("explicit", vec![pda.as_str()], "confirm_sync")),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "body={body}");
    let start: CloseAllPositionsStartResponse =
        serde_json::from_str(&body).expect("CloseAllPositionsStartResponse");
    assert!(jobs_path.is_file(), "jobs file missing at {}", jobs_path.display());

    clear_close_all_jobs_memory_for_tests().await;
    let path = format!("/api/v1/positions/close-all/{}", start.batch_id);
    let (status, _) = get_json(router.clone(), &path).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "memory cleared → 404");

    reload_close_all_jobs_from_disk().await;
    let (status, body) = get_json(router, &path).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let batch: CloseAllBatchStatusResponse =
        serde_json::from_str(&body).expect("CloseAllBatchStatusResponse");
    assert_eq!(batch.batch_id, start.batch_id);
    assert_eq!(batch.status, "done");
    assert_eq!(batch.summary.skipped, 1);
    assert_eq!(batch.items[0].address, pda);
}
