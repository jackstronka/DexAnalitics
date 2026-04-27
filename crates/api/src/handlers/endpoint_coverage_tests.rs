use crate::models::{CreateStrategyRequest, SimulationRequest, StrategyParameters, StrategyType};
use crate::routes::create_versioned_router;
use crate::state::{ApiConfig, AppState, StrategyState};
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode};
use clmm_lp_protocols::prelude::RpcConfig;
use httpmock::Method::GET;
use httpmock::MockServer;
use rust_decimal::Decimal;
use solana_sdk::pubkey::Pubkey;
use tower::util::ServiceExt;

const TEST_REQUEST_TIMEOUT_SECS: u64 = 30;
const TEST_ONCHAIN_REQUEST_TIMEOUT_SECS: u64 = 120;

fn test_state() -> AppState {
    let rpc_config = RpcConfig {
        primary_url: "http://127.0.0.1:1".to_string(),
        ..Default::default()
    };
    AppState::new(rpc_config, ApiConfig::default(), None)
}

fn test_router(state: AppState) -> axum::Router {
    create_versioned_router(
        state,
        TEST_REQUEST_TIMEOUT_SECS,
        TEST_ONCHAIN_REQUEST_TIMEOUT_SECS,
    )
}

async fn seed_strategy(state: &AppState, id: &str) {
    let now = chrono::Utc::now();
    let s = StrategyState {
        id: id.to_string(),
        name: "seed".to_string(),
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
    };
    state.strategies.write().await.insert(id.to_string(), s);
}

async fn request(
    router: axum::Router,
    method: Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> StatusCode {
    let mut req = Request::builder().method(method).uri(path);
    if body.is_some() {
        req = req.header("content-type", "application/json");
    }
    let body = body
        .map(|v| Body::from(v.to_string()))
        .unwrap_or_else(Body::empty);
    let resp = router.oneshot(req.body(body).unwrap()).await.unwrap();
    resp.status()
}

async fn request_body(
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

#[tokio::test]
async fn all_health_endpoints_are_reachable() {
    let state = test_state();
    let router = test_router(state.clone());
    assert_eq!(
        request(router.clone(), Method::GET, "/api/v1/health/live", None).await,
        StatusCode::OK
    );
    let ready = request(router.clone(), Method::GET, "/api/v1/health/ready", None).await;
    assert!(ready == StatusCode::OK || ready == StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        request(router.clone(), Method::GET, "/api/v1/health", None).await,
        StatusCode::OK
    );
    assert_eq!(
        request(router, Method::GET, "/api/v1/metrics", None).await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn all_position_endpoints_are_reachable() {
    let state = test_state();
    let router = test_router(state);
    assert_eq!(
        request(router.clone(), Method::GET, "/api/v1/positions", None).await,
        StatusCode::OK
    );
    assert_eq!(
        request(
            router.clone(),
            Method::POST,
            "/api/v1/positions/swap-before-open",
            Some(serde_json::json!({}))
        )
        .await,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        request(
            router.clone(),
            Method::GET,
            "/api/v1/positions/invalid",
            None
        )
        .await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            router.clone(),
            Method::DELETE,
            "/api/v1/positions/invalid",
            None
        )
        .await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            router.clone(),
            Method::POST,
            "/api/v1/positions/invalid/collect",
            None
        )
        .await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            router.clone(),
            Method::POST,
            "/api/v1/positions/invalid/decrease",
            Some(serde_json::json!({"liquidity_amount":"1"})),
        )
        .await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            router.clone(),
            Method::POST,
            "/api/v1/positions/invalid/rebalance",
            Some(serde_json::json!({"new_tick_lower": 1, "new_tick_upper": 2, "slippage_tolerance_bps": 50})),
        )
        .await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            router.clone(),
            Method::POST,
            "/api/v1/positions/invalid/strategy",
            Some(serde_json::json!({ "strategy_id": null })),
        )
        .await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(router, Method::GET, "/api/v1/positions/invalid/pnl", None).await,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn position_action_buttons_have_dry_run_path() {
    let mut state = test_state();
    state.set_dry_run(true);

    // Seed one fake position in the monitor so collect/close/decrease/rebalance paths can validate existence.
    let position_pk = Pubkey::new_unique();
    let pool_pk = Pubkey::new_unique();

    let monitored = clmm_lp_execution::monitor::MonitoredPosition {
        address: position_pk,
        pool: pool_pk,
        on_chain: clmm_lp_protocols::events::OnChainPosition {
            address: position_pk,
            pool: pool_pk,
            owner: Pubkey::new_unique(),
            tick_lower: -128,
            tick_upper: 128,
            liquidity: 10_000u128,
            fee_growth_inside_a: 0,
            fee_growth_inside_b: 0,
            fees_owed_a: 123,
            fees_owed_b: 456,
        },
        pnl: clmm_lp_execution::monitor::PositionPnL::default(),
        in_range: true,
        last_updated: chrono::Utc::now(),
    };
    state
        .monitor
        .insert_test_monitored_position(monitored)
        .await;

    let router = test_router(state);

    // 1) Swap (dry-run)
    let (st, body) = request_body(
        router.clone(),
        Method::POST,
        "/api/v1/positions/swap-before-open",
        Some(serde_json::json!({
            "pool_address": pool_pk.to_string(),
            "specified_mint": Pubkey::new_unique().to_string(),
            "amount_in": 1,
            "slippage_tolerance_bps": 50,
            "cost_session_id": "test-session-1"
        })),
    )
    .await;
    // Pool validation will fail on dummy pool (no RPC) -> 404/503 is acceptable; this test focuses on action endpoints below.
    assert!(
        st == StatusCode::OK
            || st == StatusCode::NOT_FOUND
            || st == StatusCode::SERVICE_UNAVAILABLE
            || st == StatusCode::UNPROCESSABLE_ENTITY,
        "unexpected status for swap-before-open dry-run: {st} body={body}"
    );

    // 2) Open (dry-run) - should not require executor.
    let (st, body) = request_body(
        router.clone(),
        Method::POST,
        "/api/v1/positions",
        Some(serde_json::json!({
          "pool_address": pool_pk.to_string(),
          "tick_lower": -128,
          "tick_upper": 128,
          "amount_a": 1,
          "amount_b": 1,
          "slippage_tolerance_bps": 50,
          "full_range": false,
          "swap_before_open": null,
          "strategy_id": null,
          "cost_session_id": "test-open-1"
        })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "open dry-run failed: body={body}");
    assert!(
        body.contains("Would open") || body.contains("DRY-RUN") || body.contains("dry_run"),
        "open response does not look like dry-run: {body}"
    );

    // 3) Collect fees (dry-run)
    let (st, body) = request_body(
        router.clone(),
        Method::POST,
        &format!("/api/v1/positions/{}/collect", position_pk),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "collect dry-run failed: body={body}");
    assert!(
        body.contains("DRY-RUN") || body.contains("Would collect"),
        "collect not dry-run: {body}"
    );

    // 4) Decrease liquidity (dry-run)
    let (st, body) = request_body(
        router.clone(),
        Method::POST,
        &format!("/api/v1/positions/{}/decrease", position_pk),
        Some(serde_json::json!({"liquidity_amount":"1"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "decrease dry-run failed: body={body}");
    assert!(
        body.contains("Would decrease") || body.contains("dry_run"),
        "decrease not dry-run: {body}"
    );

    // 5) Rebalance (dry-run)
    let (st, body) = request_body(
        router.clone(),
        Method::POST,
        &format!("/api/v1/positions/{}/rebalance", position_pk),
        Some(serde_json::json!({
          "new_tick_lower": -64,
          "new_tick_upper": 64,
          "slippage_tolerance_bps": 50,
          "reason": "manual"
        })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "rebalance dry-run failed: body={body}");
    assert!(
        body.contains("DRY-RUN") || body.contains("Would") || body.contains("dry_run"),
        "rebalance not dry-run: {body}"
    );

    // 6) Close (dry-run)
    let (st, body) = request_body(
        router,
        Method::DELETE,
        &format!("/api/v1/positions/{}", position_pk),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "close dry-run failed: body={body}");
    assert!(
        body.contains("DRY-RUN") || body.contains("Would close"),
        "close not dry-run: {body}"
    );
}

#[tokio::test]
async fn all_strategy_endpoints_are_reachable() {
    let state = test_state();
    seed_strategy(&state, "s1").await;
    let router = test_router(state);
    assert_eq!(
        request(router.clone(), Method::GET, "/api/v1/strategies", None).await,
        StatusCode::OK
    );
    assert_eq!(
        request(router.clone(), Method::GET, "/api/v1/strategies/s1", None).await,
        StatusCode::OK
    );
    assert_eq!(
        request(
            router.clone(),
            Method::PUT,
            "/api/v1/strategies/s1",
            Some(serde_json::json!(CreateStrategyRequest {
                name: "u".to_string(),
                strategy_type: StrategyType::StaticRange,
                parameters: StrategyParameters::default(),
                pool_address: None,
                auto_execute: false,
                dry_run: true
            })),
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(
        request(
            router.clone(),
            Method::POST,
            "/api/v1/strategies",
            Some(serde_json::json!(CreateStrategyRequest {
                name: "c".to_string(),
                strategy_type: StrategyType::StaticRange,
                parameters: StrategyParameters::default(),
                pool_address: None,
                auto_execute: false,
                dry_run: true
            })),
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(
        request(
            router.clone(),
            Method::POST,
            "/api/v1/strategies/s1/start",
            None
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(
        request(
            router.clone(),
            Method::POST,
            "/api/v1/strategies/s1/stop",
            None
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(
        request(
            router.clone(),
            Method::GET,
            "/api/v1/strategies/s1/performance",
            None
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(
        request(
            router.clone(),
            Method::POST,
            "/api/v1/strategies/s1/apply-optimize-result",
            Some(serde_json::json!({"decision":{"schema_version":1,"approved":false}})),
        )
        .await,
        StatusCode::CONFLICT
    );
    assert_eq!(
        request(router, Method::DELETE, "/api/v1/strategies/s1", None).await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn all_pool_and_analytics_endpoints_are_reachable() {
    let state = test_state();
    let router = test_router(state);
    let pools_status = request(router.clone(), Method::GET, "/api/v1/pools", None).await;
    assert_ne!(pools_status, StatusCode::NOT_FOUND);
    assert_eq!(
        request(router.clone(), Method::GET, "/api/v1/pools/invalid", None).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            router.clone(),
            Method::GET,
            "/api/v1/pools/invalid/state",
            None
        )
        .await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            router.clone(),
            Method::GET,
            "/api/v1/pools/invalid/estimate-swap-cost",
            None
        )
        .await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            router.clone(),
            Method::POST,
            "/api/v1/pools/invalid/quote-open-budget",
            Some(serde_json::json!({
                "tick_lower": 0,
                "tick_upper": 10,
                "target_usd": 3.0
            })),
        )
        .await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            router.clone(),
            Method::GET,
            "/api/v1/analytics/portfolio",
            None
        )
        .await,
        StatusCode::OK
    );
    let sim = SimulationRequest {
        pool_address: "pool".to_string(),
        tick_lower: 10,
        tick_upper: 20,
        initial_capital_usd: Decimal::new(100, 0),
        start_date: chrono::NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
        end_date: chrono::NaiveDate::from_ymd_opt(2025, 1, 2).unwrap(),
        ..Default::default()
    };
    // Invalid pool id or RPC failure → 400/500; endpoint must be reachable.
    let sim_code = request(
        router,
        Method::POST,
        "/api/v1/analytics/simulate",
        Some(serde_json::json!(sim)),
    )
    .await;
    assert!(
        sim_code == StatusCode::BAD_REQUEST
            || sim_code == StatusCode::INTERNAL_SERVER_ERROR
            || sim_code == StatusCode::OK
    );
}

#[tokio::test]
async fn all_orca_proxy_endpoints_are_reachable() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET).path("/pools");
        then.status(200)
            .json_body(serde_json::json!({"data":[],"meta":{"next":null,"previous":null}}));
    });
    server.mock(|when, then| {
        when.method(GET).path("/pools/search");
        then.status(200)
            .json_body(serde_json::json!({"data":[],"meta":{"next":null,"previous":null}}));
    });
    server.mock(|when, then| {
        when.method(GET).path("/pools/POOL1");
        then.status(200).json_body(serde_json::json!({"data":{"address":"POOL1","tickSpacing":64,"feeRate":300,"liquidity":"1","sqrtPrice":"1","tickCurrentIndex":0,"tokenMintA":"A","tokenMintB":"B","price":"1.0","tvlUsdc":"1.0"},"meta":{"next":null,"previous":null}}));
    });
    server.mock(|when, then| {
        when.method(GET).path("/lock/POOL1");
        then.status(200).json_body(serde_json::json!([]));
    });
    server.mock(|when, then| {
        when.method(GET).path("/tokens");
        then.status(200)
            .json_body(serde_json::json!({"data":[],"meta":{"next":null,"previous":null}}));
    });
    server.mock(|when, then| {
        when.method(GET).path("/tokens/search");
        then.status(200)
            .json_body(serde_json::json!({"data":[],"meta":{"next":null,"previous":null}}));
    });
    server.mock(|when, then| {
        when.method(GET).path("/tokens/MINT1");
        then.status(200).json_body(
            serde_json::json!({"data":{"mint":"MINT1"},"meta":{"next":null,"previous":null}}),
        );
    });
    server.mock(|when, then| {
        when.method(GET).path("/protocol");
        then.status(200).json_body(
            serde_json::json!({"data":{"tvlUsdc":"1.0"},"meta":{"next":null,"previous":null}}),
        );
    });
    let cfg = ApiConfig {
        orca_public_api_base_url: Some(server.base_url()),
        ..ApiConfig::default()
    };
    let state = AppState::new(RpcConfig::default(), cfg, None);
    let router = test_router(state);
    assert_eq!(
        request(router.clone(), Method::GET, "/api/v1/orca/pools", None).await,
        StatusCode::OK
    );
    assert_eq!(
        request(
            router.clone(),
            Method::GET,
            "/api/v1/orca/pools/search?q=SOL",
            None
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(
        request(
            router.clone(),
            Method::GET,
            "/api/v1/orca/pools/POOL1",
            None
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(
        request(router.clone(), Method::GET, "/api/v1/orca/lock/POOL1", None).await,
        StatusCode::OK
    );
    assert_eq!(
        request(router.clone(), Method::GET, "/api/v1/orca/tokens", None).await,
        StatusCode::OK
    );
    assert_eq!(
        request(
            router.clone(),
            Method::GET,
            "/api/v1/orca/tokens/search?q=ORCA",
            None
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(
        request(
            router.clone(),
            Method::GET,
            "/api/v1/orca/tokens/MINT1",
            None
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(
        request(router, Method::GET, "/api/v1/orca/protocol", None).await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn orca_positions_by_owner_validates_query() {
    let state = test_state();
    let router = test_router(state);
    assert_eq!(
        request(
            router.clone(),
            Method::GET,
            "/api/v1/orca/positions-by-owner",
            None,
        )
        .await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            router,
            Method::GET,
            "/api/v1/orca/positions-by-owner?owner=not-a-valid-pubkey",
            None,
        )
        .await,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn auth_and_ws_endpoints_are_reachable() {
    let state = test_state();
    let router = test_router(state);
    // auth endpoints
    assert_eq!(
        request(
            router.clone(),
            Method::POST,
            "/api/v1/auth/phantom/challenge",
            Some(serde_json::json!({"wallet_pubkey":"invalid"})),
        )
        .await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            router.clone(),
            Method::POST,
            "/api/v1/auth/phantom/verify",
            Some(serde_json::json!({"wallet_pubkey":"invalid","nonce":"n","signature":"s"})),
        )
        .await,
        StatusCode::BAD_REQUEST
    );

    // unsigned tx endpoints
    let build = serde_json::json!({"wallet_pubkey":"invalid"});
    assert_eq!(
        request(
            router.clone(),
            Method::POST,
            "/api/v1/tx/open/build",
            Some(build.clone()),
        )
        .await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            router.clone(),
            Method::POST,
            "/api/v1/tx/decrease/build",
            Some(build.clone()),
        )
        .await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            router.clone(),
            Method::POST,
            "/api/v1/tx/collect/build",
            Some(build.clone()),
        )
        .await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            router.clone(),
            Method::POST,
            "/api/v1/tx/close/build",
            Some(build),
        )
        .await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            router.clone(),
            Method::POST,
            "/api/v1/tx/submit-signed",
            Some(serde_json::json!({"signed_tx_base64":"bad"})),
        )
        .await,
        StatusCode::BAD_REQUEST
    );

    // websocket upgrade check
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/v1/ws/positions")
        .header("connection", "upgrade")
        .header("upgrade", "websocket")
        .header("sec-websocket-version", "13")
        .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
        .body(Body::empty())
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert!(
        resp.status() == StatusCode::SWITCHING_PROTOCOLS
            || resp.status() == StatusCode::UPGRADE_REQUIRED
    );

    let req2 = Request::builder()
        .method(Method::GET)
        .uri("/api/v1/ws/alerts")
        .header("connection", "upgrade")
        .header("upgrade", "websocket")
        .header("sec-websocket-version", "13")
        .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
        .body(Body::empty())
        .unwrap();
    let resp2 = router.oneshot(req2).await.unwrap();
    assert!(
        resp2.status() == StatusCode::SWITCHING_PROTOCOLS
            || resp2.status() == StatusCode::UPGRADE_REQUIRED
    );
}

#[tokio::test]
async fn scripts_list_endpoint_is_reachable() {
    let state = test_state();
    let router = test_router(state);
    assert_eq!(
        request(router, Method::GET, "/api/v1/scripts", None).await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn wallet_management_endpoints_are_reachable() {
    let state = test_state();
    let router = test_router(state);
    assert_eq!(
        request(router.clone(), Method::GET, "/api/v1/wallets", None).await,
        StatusCode::OK
    );
    assert_eq!(
        request(
            router.clone(),
            Method::GET,
            "/api/v1/wallets/active-signer",
            None
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(
        request(
            router.clone(),
            Method::POST,
            "/api/v1/wallets/active-signer",
            Some(serde_json::json!({ "wallet_id": "" })),
        )
        .await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            router.clone(),
            Method::POST,
            "/api/v1/wallets/create",
            Some(serde_json::json!({ "wallet_id": "bad id" })),
        )
        .await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            router,
            Method::POST,
            "/api/v1/wallets/transfer",
            Some(serde_json::json!({
                "from_wallet_id": "missing",
                "to_pubkey": "bad",
                "lamports": 0
            })),
        )
        .await,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn bot_activity_il_ledger_endpoint_is_reachable() {
    let state = test_state();
    let router = test_router(state);
    assert_eq!(
        request(router, Method::GET, "/api/v1/bot-activity/il-ledger", None,).await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn scripts_run_returns_503_without_runner() {
    let state = test_state();
    let router = test_router(state);
    assert_eq!(
        request(
            router,
            Method::POST,
            "/api/v1/scripts/quick_verify_data/run",
            Some(serde_json::json!({})),
        )
        .await,
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[tokio::test]
async fn unknown_route_is_404() {
    let state = test_state();
    let router = test_router(state);
    let status = request(router, Method::GET, "/api/v1/nope", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn open_position_endpoint_is_reachable() {
    let state = test_state();
    let router = test_router(state);
    let status = request(
        router.clone(),
        Method::POST,
        "/api/v1/positions",
        Some(serde_json::json!({
            "pool_address": "invalid",
            "tick_lower": 1,
            "tick_upper": 2,
            "amount_a": 1,
            "amount_b": 1,
            "slippage_tolerance_bps": 50
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let _ = to_bytes(
        router
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/v1/positions")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
            .into_body(),
        usize::MAX,
    )
    .await
    .unwrap();
}
