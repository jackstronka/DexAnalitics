use super::*;
use crate::state::{ApiConfig, AppState};
use axum::extract::{Query, State};
use clmm_lp_data::providers::{
    OrcaListPoolsQuery, OrcaListTokensQuery, OrcaSearchPoolsQuery, OrcaSearchTokensQuery,
};
use clmm_lp_protocols::prelude::RpcConfig;
use httpmock::Method::GET;
use httpmock::MockServer;

#[tokio::test]
async fn orca_list_pools_proxies_rest() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET).path("/pools");
        then.status(200).json_body(serde_json::json!({
            "data": [{
                "address": "POOL1",
                "tickSpacing": 64,
                "feeRate": 300,
                "liquidity": "1",
                "sqrtPrice": "1",
                "tickCurrentIndex": 0,
                "tokenMintA": "A",
                "tokenMintB": "B",
                "price": "1.0",
                "tvlUsdc": "10.0"
            }],
            "meta": { "next": null, "previous": null }
        }));
    });

    let mut cfg = ApiConfig::default();
    cfg.orca_public_api_base_url = Some(server.base_url());
    let state = AppState::new(RpcConfig::default(), cfg);
    let res = orca_list_pools(State(state), Query(OrcaListPoolsQuery::default()))
        .await
        .unwrap()
        .0;
    assert_eq!(res.total, 1);
    assert_eq!(res.pools[0].address, "POOL1");
}

#[tokio::test]
async fn orca_search_pools_proxies_rest() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET).path("/pools/search");
        then.status(200).json_body(serde_json::json!({
            "data": [{
                "address": "POOL2",
                "tickSpacing": 8,
                "feeRate": 300,
                "liquidity": "1",
                "sqrtPrice": "1",
                "tickCurrentIndex": 0,
                "tokenMintA": "A",
                "tokenMintB": "B",
                "price": "1.0",
                "tvlUsdc": "10.0"
            }],
            "meta": { "next": null, "previous": null }
        }));
    });

    let mut cfg = ApiConfig::default();
    cfg.orca_public_api_base_url = Some(server.base_url());
    let state = AppState::new(RpcConfig::default(), cfg);
    let res = orca_search_pools(
        State(state),
        Query(OrcaSearchPoolsQuery {
            q: "SOL".to_string(),
            ..Default::default()
        }),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(res.total, 1);
    assert_eq!(res.pools[0].address, "POOL2");
}

#[tokio::test]
async fn orca_get_pool_proxies_rest() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET).path("/pools/POOLX");
        then.status(200).json_body(serde_json::json!({
            "data": {
                "address": "POOLX",
                "tickSpacing": 64,
                "feeRate": 300,
                "liquidity": "1",
                "sqrtPrice": "1",
                "tickCurrentIndex": 0,
                "tokenMintA": "A",
                "tokenMintB": "B",
                "price": "1.0",
                "tvlUsdc": "10.0"
            },
            "meta": { "next": null, "previous": null }
        }));
    });

    let mut cfg = ApiConfig::default();
    cfg.orca_public_api_base_url = Some(server.base_url());
    let state = AppState::new(RpcConfig::default(), cfg);
    let res = orca_get_pool(State(state), axum::extract::Path("POOLX".to_string()))
        .await
        .unwrap()
        .0;
    assert_eq!(res.address, "POOLX");
}

#[tokio::test]
async fn orca_lock_proxies_rest() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET).path("/lock/POOLX");
        then.status(200).json_body(serde_json::json!([
            { "name": "TestLock", "lockedPercentage": "45.5" }
        ]));
    });

    let mut cfg = ApiConfig::default();
    cfg.orca_public_api_base_url = Some(server.base_url());
    let state = AppState::new(RpcConfig::default(), cfg);
    let res = orca_get_lock_info(State(state), axum::extract::Path("POOLX".to_string()))
        .await
        .unwrap()
        .0;
    assert_eq!(res.address, "POOLX");
    assert_eq!(res.locks.len(), 1);
    assert_eq!(res.locks[0].locked_percentage, "45.5");
}

#[tokio::test]
async fn orca_list_tokens_proxies_rest() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET).path("/tokens");
        then.status(200).json_body(serde_json::json!({
            "data": [{
                "mint": "MINT1",
                "symbol": "AAA",
                "name": "Token AAA",
                "decimals": 6,
                "verified": true,
                "priceUsdc": "1.25"
            }],
            "meta": { "next": null, "previous": null }
        }));
    });
    let mut cfg = ApiConfig::default();
    cfg.orca_public_api_base_url = Some(server.base_url());
    let state = AppState::new(RpcConfig::default(), cfg);
    let res = orca_list_tokens(State(state), Query(OrcaListTokensQuery::default()))
        .await
        .unwrap()
        .0;
    assert_eq!(res.total, 1);
    assert_eq!(res.tokens[0].mint, "MINT1");
}

#[tokio::test]
async fn orca_search_tokens_proxies_rest() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET).path("/tokens/search");
        then.status(200).json_body(serde_json::json!({
            "data": [{
                "mint": "MINT2",
                "symbol": "BBB"
            }],
            "meta": { "next": null, "previous": null }
        }));
    });
    let mut cfg = ApiConfig::default();
    cfg.orca_public_api_base_url = Some(server.base_url());
    let state = AppState::new(RpcConfig::default(), cfg);
    let res = orca_search_tokens(
        State(state),
        Query(OrcaSearchTokensQuery {
            q: "BBB".to_string(),
            ..Default::default()
        }),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(res.total, 1);
    assert_eq!(res.tokens[0].mint, "MINT2");
}

#[tokio::test]
async fn orca_get_token_proxies_rest() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET).path("/tokens/MINTX");
        then.status(200).json_body(serde_json::json!({
            "data": {
                "mint": "MINTX",
                "symbol": "XXX",
                "priceUsdc": "0.9"
            },
            "meta": { "next": null, "previous": null }
        }));
    });
    let mut cfg = ApiConfig::default();
    cfg.orca_public_api_base_url = Some(server.base_url());
    let state = AppState::new(RpcConfig::default(), cfg);
    let res = orca_get_token(State(state), axum::extract::Path("MINTX".to_string()))
        .await
        .unwrap()
        .0;
    assert_eq!(res.mint, "MINTX");
}

#[tokio::test]
async fn orca_get_protocol_proxies_rest() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET).path("/protocol");
        then.status(200).json_body(serde_json::json!({
            "data": {
                "tvlUsdc": "100.1",
                "volume24hUsdc": "10.2",
                "volume7dUsdc": "70.3"
            },
            "meta": { "next": null, "previous": null }
        }));
    });
    let mut cfg = ApiConfig::default();
    cfg.orca_public_api_base_url = Some(server.base_url());
    let state = AppState::new(RpcConfig::default(), cfg);
    let res = orca_get_protocol(State(state)).await.unwrap().0;
    assert!(res.tvl_usdc.is_some());
}
