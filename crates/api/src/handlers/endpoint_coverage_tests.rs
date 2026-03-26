use crate::models::{CreateStrategyRequest, SimulationRequest, StrategyParameters, StrategyType};
use crate::routes::create_versioned_router;
use crate::state::{ApiConfig, AppState, StrategyState};
use axum::body::{to_bytes, Body};
use axum::http::{Method, Request, StatusCode};
use clmm_lp_protocols::prelude::RpcConfig;
use httpmock::Method::GET;
use httpmock::MockServer;
use rust_decimal::Decimal;
use tower::util::ServiceExt;

fn test_state() -> AppState {
    let rpc_config = RpcConfig {
        primary_url: "http://127.0.0.1:1".to_string(),
        ..Default::default()
    };
    AppState::new(rpc_config, ApiConfig::default())
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

async fn request(router: axum::Router, method: Method, path: &str, body: Option<serde_json::Value>) -> StatusCode {
    let mut req = Request::builder().method(method).uri(path);
    if body.is_some() {
        req = req.header("content-type", "application/json");
    }
    let body = body.map(|v| Body::from(v.to_string())).unwrap_or_else(Body::empty);
    let resp = router.oneshot(req.body(body).unwrap()).await.unwrap();
    resp.status()
}

#[tokio::test]
async fn all_health_endpoints_are_reachable() {
    let state = test_state();
    let router = create_versioned_router(state.clone());
    assert_eq!(request(router.clone(), Method::GET, "/api/v1/health/live", None).await, StatusCode::OK);
    let ready = request(router.clone(), Method::GET, "/api/v1/health/ready", None).await;
    assert!(ready == StatusCode::OK || ready == StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(request(router.clone(), Method::GET, "/api/v1/health", None).await, StatusCode::OK);
    assert_eq!(request(router, Method::GET, "/api/v1/metrics", None).await, StatusCode::OK);
}

#[tokio::test]
async fn all_position_endpoints_are_reachable() {
    let state = test_state();
    let router = create_versioned_router(state);
    assert_eq!(request(router.clone(), Method::GET, "/api/v1/positions", None).await, StatusCode::OK);
    assert_eq!(
        request(router.clone(), Method::GET, "/api/v1/positions/invalid", None).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(router.clone(), Method::DELETE, "/api/v1/positions/invalid", None).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(router.clone(), Method::POST, "/api/v1/positions/invalid/collect", None).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            router.clone(),
            Method::POST,
            "/api/v1/positions/invalid/decrease",
            Some(serde_json::json!({"liquidity_amount":1})),
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
        request(router, Method::GET, "/api/v1/positions/invalid/pnl", None).await,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn all_strategy_endpoints_are_reachable() {
    let state = test_state();
    seed_strategy(&state, "s1").await;
    let router = create_versioned_router(state);
    assert_eq!(request(router.clone(), Method::GET, "/api/v1/strategies", None).await, StatusCode::OK);
    assert_eq!(request(router.clone(), Method::GET, "/api/v1/strategies/s1", None).await, StatusCode::OK);
    assert_eq!(
        request(
            router.clone(),
            Method::PUT,
            "/api/v1/strategies/s1",
            Some(serde_json::json!(CreateStrategyRequest {
                name: "u".to_string(),
                pool_address: "pool".to_string(),
                strategy_type: StrategyType::StaticRange,
                parameters: StrategyParameters::default(),
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
                pool_address: "pool".to_string(),
                strategy_type: StrategyType::StaticRange,
                parameters: StrategyParameters::default(),
                auto_execute: false,
                dry_run: true
            })),
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(
        request(router.clone(), Method::POST, "/api/v1/strategies/s1/start", None).await,
        StatusCode::OK
    );
    assert_eq!(
        request(router.clone(), Method::POST, "/api/v1/strategies/s1/stop", None).await,
        StatusCode::OK
    );
    assert_eq!(
        request(router.clone(), Method::GET, "/api/v1/strategies/s1/performance", None).await,
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
    let router = create_versioned_router(state);
    let pools_status = request(router.clone(), Method::GET, "/api/v1/pools", None).await;
    assert_ne!(pools_status, StatusCode::NOT_FOUND);
    assert_eq!(
        request(router.clone(), Method::GET, "/api/v1/pools/invalid", None).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(router.clone(), Method::GET, "/api/v1/pools/invalid/state", None).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(router.clone(), Method::GET, "/api/v1/analytics/portfolio", None).await,
        StatusCode::OK
    );
    let sim = SimulationRequest {
        pool_address: "pool".to_string(),
        tick_lower: 10,
        tick_upper: 20,
        initial_capital_usd: Decimal::new(100, 0),
        start_date: chrono::NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
        end_date: chrono::NaiveDate::from_ymd_opt(2025, 1, 2).unwrap(),
        strategy_type: None,
    };
    assert_eq!(
        request(
            router,
            Method::POST,
            "/api/v1/analytics/simulate",
            Some(serde_json::json!(sim)),
        )
        .await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn all_orca_proxy_endpoints_are_reachable() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET).path("/pools");
        then.status(200).json_body(serde_json::json!({"data":[],"meta":{"next":null,"previous":null}}));
    });
    server.mock(|when, then| {
        when.method(GET).path("/pools/search");
        then.status(200).json_body(serde_json::json!({"data":[],"meta":{"next":null,"previous":null}}));
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
        then.status(200).json_body(serde_json::json!({"data":[],"meta":{"next":null,"previous":null}}));
    });
    server.mock(|when, then| {
        when.method(GET).path("/tokens/search");
        then.status(200).json_body(serde_json::json!({"data":[],"meta":{"next":null,"previous":null}}));
    });
    server.mock(|when, then| {
        when.method(GET).path("/tokens/MINT1");
        then.status(200).json_body(serde_json::json!({"data":{"mint":"MINT1"},"meta":{"next":null,"previous":null}}));
    });
    server.mock(|when, then| {
        when.method(GET).path("/protocol");
        then.status(200).json_body(serde_json::json!({"data":{"tvlUsdc":"1.0"},"meta":{"next":null,"previous":null}}));
    });
    let mut cfg = ApiConfig::default();
    cfg.orca_public_api_base_url = Some(server.base_url());
    let state = AppState::new(RpcConfig::default(), cfg);
    let router = create_versioned_router(state);
    assert_eq!(
        request(router.clone(), Method::GET, "/api/v1/orca/pools", None).await,
        StatusCode::OK
    );
    assert_eq!(
        request(router.clone(), Method::GET, "/api/v1/orca/pools/search?q=SOL", None).await,
        StatusCode::OK
    );
    assert_eq!(request(router.clone(), Method::GET, "/api/v1/orca/pools/POOL1", None).await, StatusCode::OK);
    assert_eq!(request(router.clone(), Method::GET, "/api/v1/orca/lock/POOL1", None).await, StatusCode::OK);
    assert_eq!(request(router.clone(), Method::GET, "/api/v1/orca/tokens", None).await, StatusCode::OK);
    assert_eq!(
        request(router.clone(), Method::GET, "/api/v1/orca/tokens/search?q=ORCA", None).await,
        StatusCode::OK
    );
    assert_eq!(request(router.clone(), Method::GET, "/api/v1/orca/tokens/MINT1", None).await, StatusCode::OK);
    assert_eq!(request(router, Method::GET, "/api/v1/orca/protocol", None).await, StatusCode::OK);
}

#[tokio::test]
async fn auth_and_ws_endpoints_are_reachable() {
    let state = test_state();
    let router = create_versioned_router(state);
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
    assert!(resp.status() == StatusCode::SWITCHING_PROTOCOLS || resp.status() == StatusCode::UPGRADE_REQUIRED);

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
    assert!(resp2.status() == StatusCode::SWITCHING_PROTOCOLS || resp2.status() == StatusCode::UPGRADE_REQUIRED);
}

#[tokio::test]
async fn unknown_route_is_404() {
    let state = test_state();
    let router = create_versioned_router(state);
    let status = request(router, Method::GET, "/api/v1/nope", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn open_position_endpoint_is_reachable() {
    let state = test_state();
    let router = create_versioned_router(state);
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
