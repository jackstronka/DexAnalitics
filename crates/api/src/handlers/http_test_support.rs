//! Shared hermetic helpers for API HTTP `oneshot` tests (API-H0).
//!
//! Keep RPC pointed at a black-hole localhost URL with **empty** fallbacks so
//! coverage/contract suites cannot accidentally hit public endpoints.

use crate::routes::create_versioned_router;
use crate::state::{ApiConfig, AppState};
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode};
use clmm_lp_protocols::prelude::RpcConfig;
use std::time::Duration;
use tower::util::ServiceExt;

pub const TEST_REQUEST_TIMEOUT_SECS: u64 = 30;
pub const TEST_ONCHAIN_REQUEST_TIMEOUT_SECS: u64 = 120;

/// Dead primary + no public fallbacks + zero retries (hermetic CI).
pub fn hermetic_rpc() -> RpcConfig {
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

pub fn test_state() -> AppState {
    AppState::new(hermetic_rpc(), ApiConfig::default(), None)
}

pub fn test_state_with_config(cfg: ApiConfig) -> AppState {
    AppState::new(hermetic_rpc(), cfg, None)
}

/// AppState with a custom primary RPC URL (e.g. httpmock) and empty fallbacks.
pub fn test_state_with_rpc_url(primary_url: impl Into<String>) -> AppState {
    let rpc = RpcConfig {
        primary_url: primary_url.into(),
        fallback_urls: Vec::new(),
        timeout: Duration::from_millis(2_000),
        max_retries: 0,
        retry_base_delay_ms: 1,
        retry_max_delay_ms: 1,
        health_check_interval_secs: 60,
        commitment: Default::default(),
    };
    AppState::new(rpc, ApiConfig::default(), None)
}

pub fn test_router(state: AppState) -> axum::Router {
    create_versioned_router(
        state,
        TEST_REQUEST_TIMEOUT_SECS,
        TEST_ONCHAIN_REQUEST_TIMEOUT_SECS,
    )
}

pub async fn request(
    router: axum::Router,
    method: Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> StatusCode {
    request_body(router, method, path, body).await.0
}

pub async fn request_with_headers(
    router: axum::Router,
    method: Method,
    path: &str,
    headers: &[(&'static str, &'static str)],
) -> StatusCode {
    let mut req = Request::builder().method(method).uri(path);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let resp = router
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap();
    resp.status()
}

pub async fn request_body(
    router: axum::Router,
    method: Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, String) {
    let mut req = Request::builder().method(method).uri(path);
    if body.is_some() {
        req = req.header("content-type", "application/json");
    }
    let body = body
        .map(|v| Body::from(v.to_string()))
        .unwrap_or_else(Body::empty);
    let resp = router.oneshot(req.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    (status, String::from_utf8_lossy(&bytes).to_string())
}

pub async fn get_json(router: axum::Router, path: &str) -> (StatusCode, String) {
    request_body(router, Method::GET, path, None).await
}

pub async fn request_body_with_headers(
    router: axum::Router,
    method: Method,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<serde_json::Value>,
) -> (StatusCode, String) {
    let mut req = Request::builder().method(method).uri(path);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    if body.is_some() {
        req = req.header("content-type", "application/json");
    }
    let body = body
        .map(|v| Body::from(v.to_string()))
        .unwrap_or_else(Body::empty);
    let resp = router.oneshot(req.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    (status, String::from_utf8_lossy(&bytes).to_string())
}
