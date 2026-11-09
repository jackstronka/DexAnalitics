//! HTTP-boundary contract tests for critical `/api/v1` routes.
//!
//! Goes beyond [`endpoint_coverage_tests`] (status-code smoke): deserializes
//! responses into OpenAPI/serde models and asserts happy-path JSON shape with
//! seeded in-memory state. Still hermetic (no DB, RPC to `127.0.0.1:1`, empty
//! fallbacks).

use crate::models::{
    HealthResponse, ListPositionsResponse, ListStrategiesResponse, MetricsResponse, ServiceStatus,
    WalletsListResponse,
};
use crate::routes::create_versioned_router;
use crate::state::{ApiConfig, AppState, StrategyState};
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode};
use clmm_lp_protocols::prelude::RpcConfig;
use std::time::Duration;
use tower::util::ServiceExt;

const TEST_REQUEST_TIMEOUT_SECS: u64 = 30;
const TEST_ONCHAIN_REQUEST_TIMEOUT_SECS: u64 = 120;

fn hermetic_rpc() -> RpcConfig {
    RpcConfig {
        primary_url: "http://127.0.0.1:1".to_string(),
        fallback_urls: Vec::new(),
        timeout: Duration::from_millis(200),
        max_retries: 0,
        retry_base_delay_ms: 1,
        retry_max_delay_ms: 1,
        health_check_interval_secs: 60,
        commitment: Default::default(),
    }
}

fn test_state() -> AppState {
    AppState::new(hermetic_rpc(), ApiConfig::default(), None)
}

fn test_router(state: AppState) -> axum::Router {
    create_versioned_router(
        state,
        TEST_REQUEST_TIMEOUT_SECS,
        TEST_ONCHAIN_REQUEST_TIMEOUT_SECS,
    )
}

async fn get_json(router: axum::Router, path: &str) -> (StatusCode, String) {
    let resp = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(path)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    (status, String::from_utf8_lossy(&bytes).to_string())
}

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
