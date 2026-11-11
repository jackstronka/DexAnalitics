//! API-H4: hermetic HTTP for bot stranded/pending-open, WS event shape, backtest job status.

use super::backtests::{seed_backtest_job_for_tests, seed_full_backtest_job_for_tests};
use super::http_test_support::{
    get_json, request_body, request_body_with_headers, test_router, test_state,
};
use crate::events::EVENT_POSITION_UPDATED;
use crate::models::{
    BacktestFullJobResponse, BacktestJobResponse, PendingOpenRecoveryResponse,
    StrandedRebalancesResponse,
};
use crate::state::PositionUpdate;
use crate::test_env::EnvGuard;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use std::io::Write;
use tower::util::ServiceExt;

fn hermetic_bot_paths(
    env: &mut EnvGuard,
    dir: &std::path::Path,
) -> (std::path::PathBuf, std::path::PathBuf) {
    let ledger = dir.join("lifecycle.jsonl");
    let pending = dir.join("pending-open-recovery.json");
    let dismissed = dir.join("stranded-dismissed.json");
    env.set("CLMM_POSITION_LIFECYCLE_LEDGER_READ_PATH", &ledger);
    env.set("CLMM_POSITION_LIFECYCLE_LEDGER_PATH", &ledger);
    env.set("CLMM_PENDING_OPEN_RECOVERY_PATH", &pending);
    env.set("CLMM_STRANDED_DISMISSED_PATH", &dismissed);
    env.remove("CLMM_IL_LEDGER_PATH");
    env.remove("CLMM_POSITION_LIFECYCLE_USE_ENRICHED");
    (ledger, pending)
}

/// API-H4.1: missing pending-open file → typed envelope with `file_missing`.
#[tokio::test]
async fn pending_open_missing_file_deserializes() {
    let mut env = EnvGuard::lock().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let (_ledger, pending) = hermetic_bot_paths(&mut env, dir.path());
    assert!(!pending.exists());

    let (status, body) = get_json(
        test_router(test_state()),
        "/api/v1/bot-activity/pending-open",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let resp: PendingOpenRecoveryResponse =
        serde_json::from_str(&body).expect("PendingOpenRecoveryResponse");
    assert!(resp.file_missing);
    assert!(resp.data.is_none());
    assert!(resp.path.contains("pending-open-recovery.json"));
}

/// API-H4.1: seeded pending-open JSON → `data` present.
#[tokio::test]
async fn pending_open_seeded_file_deserializes() {
    let mut env = EnvGuard::lock().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let (_ledger, pending) = hermetic_bot_paths(&mut env, dir.path());
    let payload = serde_json::json!({
        "items": [{
            "pool": "Pool1111111111111111111111111111111111111",
            "intended_tick_lower": -10,
            "intended_tick_upper": 10,
            "old_position": "OldPos111111111111111111111111111111111",
            "note": "h4-fixture"
        }],
        "dismissed_session_ids": []
    });
    std::fs::write(&pending, serde_json::to_string_pretty(&payload).unwrap())
        .expect("write pending");

    let (status, body) = get_json(
        test_router(test_state()),
        "/api/v1/bot-activity/pending-open",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let resp: PendingOpenRecoveryResponse =
        serde_json::from_str(&body).expect("PendingOpenRecoveryResponse");
    assert!(!resp.file_missing);
    let data = resp.data.expect("data");
    assert_eq!(data["items"][0]["note"], "h4-fixture");
}

/// API-H4.1: lifecycle close-without-open → stranded item; dismiss removes it.
#[tokio::test]
async fn stranded_list_and_dismiss_round_trip() {
    let mut env = EnvGuard::lock().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let (ledger, _pending) = hermetic_bot_paths(&mut env, dir.path());
    // Empty pending file so load path is hermetic (not repo data/).
    std::fs::write(
        dir.path().join("pending-open-recovery.json"),
        r#"{"items":[],"dismissed_session_ids":[]}"#,
    )
    .expect("write pending");

    let sid = "sess-h4-stranded-1";
    let mut f = std::fs::File::create(&ledger).expect("create ledger");
    writeln!(
        f,
        "{}",
        serde_json::json!({
            "source": "orca_bot",
            "rebalance_session_id": sid,
            "event": "bot_close_position",
            "ts_utc": "2026-01-01T00:00:00Z",
            "position_pubkey": "OldNftH4aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "pool_address": "PoolH4bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        })
    )
    .expect("write row");

    let router = test_router(test_state());
    let (status, body) = get_json(router.clone(), "/api/v1/bot-activity/stranded-rebalances").await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let snap: StrandedRebalancesResponse =
        serde_json::from_str(&body).expect("StrandedRebalancesResponse");
    assert_eq!(snap.rows_scanned, 1);
    assert_eq!(snap.items.len(), 1);
    assert_eq!(snap.items[0].rebalance_session_id, sid);
    assert!(snap.items[0].close_seen);
    assert!(!snap.items[0].open_seen);

    let (status, body) = request_body(
        router.clone(),
        Method::POST,
        &format!("/api/v1/bot-activity/stranded-rebalances/{sid}/dismiss"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let after: StrandedRebalancesResponse =
        serde_json::from_str(&body).expect("StrandedRebalancesResponse after dismiss");
    assert!(
        after.items.iter().all(|it| it.rebalance_session_id != sid),
        "dismissed session still listed: {after:?}"
    );

    let (status, body) = get_json(router, "/api/v1/bot-activity/stranded-rebalances").await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let again: StrandedRebalancesResponse =
        serde_json::from_str(&body).expect("StrandedRebalancesResponse reload");
    assert!(
        again.items.iter().all(|it| it.rebalance_session_id != sid),
        "dismiss not durable across GET: {again:?}"
    );
}

#[tokio::test]
async fn stranded_dismiss_empty_session_id_is_bad_request() {
    let mut env = EnvGuard::lock().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let _ = hermetic_bot_paths(&mut env, dir.path());
    std::fs::write(&dir.path().join("lifecycle.jsonl"), "").ok();
    std::fs::write(
        dir.path().join("pending-open-recovery.json"),
        r#"{"items":[],"dismissed_session_ids":[]}"#,
    )
    .ok();

    let (status, body) = request_body(
        test_router(test_state()),
        Method::POST,
        "/api/v1/bot-activity/stranded-rebalances/%20/dismiss",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");
}

/// API-H4.2: WS upgrade accepts handshake; injected event_bus payload matches WS text shape.
#[tokio::test]
async fn positions_ws_upgrade_and_injected_event_shape() {
    let state = test_state();
    let mut rx = state.event_bus.subscribe(EVENT_POSITION_UPDATED);

    let router = test_router(state.clone());
    let (status, _body) = request_body_with_headers(
        router,
        Method::GET,
        "/api/v1/ws/positions",
        &[
            ("connection", "upgrade"),
            ("upgrade", "websocket"),
            ("sec-websocket-version", "13"),
            ("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ=="),
        ],
        None,
    )
    .await;
    assert!(
        status == StatusCode::SWITCHING_PROTOCOLS || status == StatusCode::UPGRADE_REQUIRED,
        "status={status}"
    );

    state
        .broadcast_position_update(PositionUpdate {
            update_type: "h4_fixture".to_string(),
            position_address: "PosH4ccccccccccccccccccccccccccccccccc".to_string(),
            timestamp: chrono::Utc::now(),
            data: serde_json::json!({ "source": "ops_http_tests" }),
        })
        .await;

    let event = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
        .await
        .expect("timeout waiting for event")
        .expect("recv event");
    assert_eq!(event.event_type, EVENT_POSITION_UPDATED);
    // Same serialization path as `handle_positions_ws` (`serde_json::to_string(&event.payload)`).
    let text = serde_json::to_string(&event.payload).expect("payload json");
    let v: serde_json::Value = serde_json::from_str(&text).expect("payload object");
    assert_eq!(v["update_type"], "h4_fixture");
    assert_eq!(
        v["position_address"],
        "PosH4ccccccccccccccccccccccccccccccccc"
    );
    assert_eq!(v["data"]["source"], "ops_http_tests");
    assert!(v.get("timestamp").is_some());
}

/// API-H4.3: seeded JOBS → GET typed status; missing id → 404; bad start body → 4xx (no CLI).
#[tokio::test]
async fn backtest_job_status_seed_and_validation() {
    let job_id = "job-h4-seed-1";
    seed_backtest_job_for_tests(BacktestJobResponse {
        id: job_id.to_string(),
        position_address: "PosH4ddddddddddddddddddddddddddddddddd".to_string(),
        pool_address: "PoolH4eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".to_string(),
        status: "completed".to_string(),
        started_ts_utc: "2026-01-01T00:00:00Z".to_string(),
        finished_ts_utc: Some("2026-01-01T00:01:00Z".to_string()),
        exit_code: Some(0),
        stdout: Some("ok".to_string()),
        stderr: None,
        note: Some("h4-seed".to_string()),
    })
    .await;

    let router = test_router(test_state());
    let (status, body) = get_json(router.clone(), &format!("/api/v1/backtests/{job_id}")).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let job: BacktestJobResponse = serde_json::from_str(&body).expect("BacktestJobResponse");
    assert_eq!(job.id, job_id);
    assert_eq!(job.status, "completed");
    assert_eq!(job.exit_code, Some(0));
    assert_eq!(job.note.as_deref(), Some("h4-seed"));

    let (status, body) = get_json(router.clone(), "/api/v1/backtests/missing-h4-job").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body={body}");

    let (status, body) = request_body(
        router.clone(),
        Method::POST,
        "/api/v1/backtests/from-closed-position",
        Some(serde_json::json!({ "position_address": "" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");
    assert!(
        body.to_ascii_lowercase().contains("position_address"),
        "body={body}"
    );

    let (status, body) = request_body(
        router.clone(),
        Method::POST,
        "/api/v1/backtests/full",
        Some(serde_json::json!({ "windows_hours": [0] })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");
    assert!(
        body.to_ascii_lowercase().contains("windows_hours"),
        "body={body}"
    );

    let full_id = "full-h4-seed-1";
    seed_full_backtest_job_for_tests(BacktestFullJobResponse {
        id: full_id.to_string(),
        status: "completed".to_string(),
        started_ts_utc: "2026-01-01T00:00:00Z".to_string(),
        finished_ts_utc: Some("2026-01-01T00:02:00Z".to_string()),
        stderr: None,
        note: Some("h4-full-seed".to_string()),
        results: Some(vec![]),
    })
    .await;
    let (status, body) = get_json(router, &format!("/api/v1/backtests/full/{full_id}")).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let full: BacktestFullJobResponse =
        serde_json::from_str(&body).expect("BacktestFullJobResponse");
    assert_eq!(full.id, full_id);
    assert_eq!(full.status, "completed");
    assert_eq!(full.note.as_deref(), Some("h4-full-seed"));
}

/// Sanity: oneshot without Upgrade headers is not a silent 200 HTML page.
#[tokio::test]
async fn positions_ws_without_upgrade_is_not_ok_json() {
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/v1/ws/positions")
        .body(Body::empty())
        .unwrap();
    let resp = test_router(test_state()).oneshot(req).await.unwrap();
    assert_ne!(resp.status(), StatusCode::OK);
}
