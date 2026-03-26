use super::*;
use crate::state::{ApiConfig, AppState};
use clmm_lp_protocols::prelude::RpcConfig;
use axum::extract::State;
use httpmock::Method::GET;
use httpmock::MockServer;

#[tokio::test]
async fn list_pools_uses_orca_rest_base_url_env() {
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
                "tokenMintA": "MINTA",
                "tokenMintB": "MINTB",
                "price": "1.0",
                "tvlUsdc": "10.0"
            }],
            "meta": { "next": null, "previous": null }
        }));
    });

    let old = std::env::var("ORCA_PUBLIC_API_BASE_URL").ok();
    unsafe { std::env::set_var("ORCA_PUBLIC_API_BASE_URL", server.base_url()) };

    let state = AppState::new(RpcConfig::default(), ApiConfig::default());
    let res = list_pools(State(state)).await.unwrap().0;
    assert_eq!(res.total, 1);
    assert_eq!(res.pools.len(), 1);
    assert_eq!(res.pools[0].address, "POOL1");

    match old {
        Some(v) => unsafe { std::env::set_var("ORCA_PUBLIC_API_BASE_URL", v) },
        None => unsafe { std::env::remove_var("ORCA_PUBLIC_API_BASE_URL") },
    }
}

