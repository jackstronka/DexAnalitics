//! API-H3: hermetic HTTP for chain-portfolio envelopes + wallet balances/api-signer.

use super::http_test_support::{
    get_json, request_body, test_router, test_state, test_state_with_rpc_url,
};
use crate::models::{
    ApiSignerWalletResponse, WalletBalancesResponse, WalletChainPortfolioResponse,
    WalletEffectiveBalancesResponse, WalletSessionBalanceRow,
};
use crate::services::chain_portfolio::{
    chain_balance_usd_legs_from_balances, portfolio_balance_usd_from_balances,
};
use crate::test_env::EnvGuard;
use axum::http::{Method, StatusCode};
use httpmock::Method::POST;
use httpmock::MockServer;
use solana_sdk::pubkey::Pubkey;
use std::collections::BTreeMap;

const WSOL: &str = "So11111111111111111111111111111111111111112";
const USDC: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

fn bal(mint: &str, raw: &str) -> WalletSessionBalanceRow {
    WalletSessionBalanceRow {
        mint: mint.to_string(),
        amount_raw: raw.to_string(),
        decimals: None,
    }
}

/// API-H3.1: no DB → typed empty CHAIN portfolio (not 500).
#[tokio::test]
async fn chain_portfolio_no_db_deserializes_empty_envelope() {
    let (status, body) = get_json(
        test_router(test_state()),
        "/api/v1/wallets/chain-portfolio?chain_session_id=chain-h3-fixture",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let resp: WalletChainPortfolioResponse =
        serde_json::from_str(&body).expect("WalletChainPortfolioResponse");
    assert_eq!(resp.chain_session_id, "chain-h3-fixture");
    assert!(
        resp.source == "gl_chain_shadow_no_db" || resp.source == "gl_chain_shadow_disabled",
        "source={}",
        resp.source
    );
    assert!(
        resp.quality == "no_db" || resp.quality == "disabled",
        "quality={}",
        resp.quality
    );
    assert!(resp.balances.is_empty());
    assert!(resp.chain_balance_usd_legs.is_empty());
    assert!(resp.portfolio_balance_usd.is_none());
}

#[tokio::test]
async fn chain_portfolio_requires_chain_session_id() {
    let (status, body) =
        get_json(test_router(test_state()), "/api/v1/wallets/chain-portfolio").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");
}

/// API-H3.1: B6 footer numbers via the same helpers the HTTP handler uses (no snap edit).
#[test]
fn chain_portfolio_b6_footer_legs_match_http_dto_helpers() {
    let spot = BTreeMap::from([(WSOL.to_string(), 150.0), (USDC.to_string(), 1.0)]);
    let leftover = vec![bal(WSOL, "50000000"), bal(USDC, "1000000")];
    let legs = chain_balance_usd_legs_from_balances(&leftover, &spot);
    let footer = portfolio_balance_usd_from_balances(&leftover, &spot);
    assert_eq!(legs.len(), 2);
    assert_eq!(legs[0].mint, WSOL);
    assert_eq!(legs[0].value_usd.as_deref(), Some("7.50000000"));
    assert_eq!(legs[1].mint, USDC);
    assert_eq!(legs[1].value_usd.as_deref(), Some("1.00000000"));
    assert_eq!(footer.as_deref(), Some("8.50000000"));
}

#[tokio::test]
async fn chain_portfolio_history_requires_db() {
    let (status, body) = get_json(
        test_router(test_state()),
        "/api/v1/wallets/chain-portfolio/history?chain_session_id=chain-h3",
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body={body}");
    assert!(
        body.to_ascii_lowercase().contains("database")
            || body.to_ascii_lowercase().contains("not connected"),
        "body={body}"
    );
}

/// API-H3.2: without DB, backfill fails stably (2nd call same class of error — no silent success).
#[tokio::test]
async fn chain_portfolio_backfill_without_db_is_stable_error() {
    let router = test_router(test_state());
    let (s1, b1) = request_body(
        router.clone(),
        Method::POST,
        "/api/v1/wallets/chain-portfolio/backfill?chain_session_id=chain-h3",
        None,
    )
    .await;
    let (s2, b2) = request_body(
        router,
        Method::POST,
        "/api/v1/wallets/chain-portfolio/backfill?chain_session_id=chain-h3",
        None,
    )
    .await;
    assert_eq!(s1, StatusCode::INTERNAL_SERVER_ERROR, "body={b1}");
    assert_eq!(s2, StatusCode::INTERNAL_SERVER_ERROR, "body={b2}");
    assert!(b1.to_ascii_lowercase().contains("database"));
    assert!(b2.to_ascii_lowercase().contains("database"));
}

#[tokio::test]
async fn wallet_balances_rejects_invalid_owner() {
    let router = test_router(test_state());
    let (status, body) = get_json(router.clone(), "/api/v1/wallets/balances").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");

    let (status, body) = get_json(router, "/api/v1/wallets/balances?owner=not-a-pubkey").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");
}

#[tokio::test]
async fn wallet_effective_balances_rejects_invalid_owner() {
    let (status, body) = get_json(
        test_router(test_state()),
        "/api/v1/wallets/effective-balances?owner=",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");
}

#[tokio::test]
async fn wallet_effective_balances_warmup_deserializes_without_force() {
    let mut env = EnvGuard::lock().await;
    let cache_dir = tempfile::tempdir().expect("tempdir");
    env.set(
        "CLMM_WALLET_EFFECTIVE_CACHE_PATH",
        cache_dir.path().join("eff.json"),
    );
    let owner = Pubkey::new_unique().to_string();
    let (status, body) = get_json(
        test_router(test_state()),
        &format!("/api/v1/wallets/effective-balances?owner={owner}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let resp: WalletEffectiveBalancesResponse =
        serde_json::from_str(&body).expect("WalletEffectiveBalancesResponse");
    assert_eq!(resp.owner, owner);
    // Warmup path: stale and/or not verified until background refresh finishes.
    assert!(
        resp.is_stale
            || !matches!(
                resp.confidence,
                crate::models::WalletBalanceConfidence::Verified
            )
    );
}

#[tokio::test]
async fn api_signer_unconfigured_deserializes() {
    let mut env = EnvGuard::lock().await;
    env.remove("KEYPAIR_PATH");
    env.remove("SOLANA_KEYPAIR_PATH");
    env.remove("WALLET_KEYPAIR_PATH");
    env.remove("SOLANA_KEYPAIR");
    env.remove("WALLET_KEYPAIR_BASE58");
    let (status, body) = get_json(test_router(test_state()), "/api/v1/wallets/api-signer").await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let resp: ApiSignerWalletResponse =
        serde_json::from_str(&body).expect("ApiSignerWalletResponse");
    assert!(!resp.configured);
    assert!(resp.pubkey.is_none());
    assert!(resp.note.is_some());
}

/// API-H3.3: httpmock JSON-RPC getBalance + empty token accounts → typed balances.
#[tokio::test]
async fn wallet_balances_via_httpmock_rpc() {
    let mut env = EnvGuard::lock().await;
    env.set("CLMM_WALLET_BALANCES_TOKEN_TIMEOUT_MS", "2000");
    env.set("CLMM_WALLET_BALANCES_NATIVE_TIMEOUT_MS", "2000");
    env.set("CLMM_WALLET_BALANCES_FANOUT", "1");

    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(POST).body_includes("getBalance");
        then.status(200).json_body(serde_json::json!({
            "jsonrpc": "2.0",
            "result": { "context": { "slot": 1 }, "value": 1_500_000_000u64 },
            "id": 1
        }));
    });
    server.mock(|when, then| {
        when.method(POST).body_includes("getTokenAccountsByOwner");
        then.status(200).json_body(serde_json::json!({
            "jsonrpc": "2.0",
            "result": { "context": { "slot": 1 }, "value": [] },
            "id": 1
        }));
    });

    let owner = Pubkey::new_unique().to_string();
    let (status, body) = get_json(
        test_router(test_state_with_rpc_url(server.base_url())),
        &format!("/api/v1/wallets/balances?owner={owner}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let resp: WalletBalancesResponse = serde_json::from_str(&body).expect("WalletBalancesResponse");
    assert_eq!(resp.owner, owner);
    assert_eq!(resp.lamports, 1_500_000_000);
    assert!(resp.sol.starts_with("1.5"));
    assert!(resp.tokens.is_empty());
}
