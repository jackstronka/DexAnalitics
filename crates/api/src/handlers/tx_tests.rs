use super::*;
use crate::models::{BuildUnsignedTxRequest, SubmitSignedTxRequest};
use crate::state::{ApiConfig, AppState};
use axum::Json;
use axum::extract::State;
use clmm_lp_protocols::prelude::RpcConfig;
use solana_sdk::signature::Keypair;
use solana_sdk::signer::Signer;

fn state() -> AppState {
    AppState::new(RpcConfig::default(), ApiConfig::default())
}

#[tokio::test]
async fn tx_build_rejects_invalid_wallet_pubkey() {
    let s = state();
    let err = tx_open_build(
        State(s),
        Json(BuildUnsignedTxRequest {
            wallet_pubkey: "invalid".to_string(),
            position_address: None,
            pool_address: None,
            amount_a: None,
            amount_b: None,
            liquidity_amount: None,
            slippage_bps: None,
            tick_lower: None,
            tick_upper: None,
        }),
    )
    .await
    .unwrap_err();
    assert_eq!(err.status_code(), axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn tx_submit_rejects_invalid_base64() {
    let s = state();
    let err = tx_submit_signed(
        State(s),
        Json(SubmitSignedTxRequest {
            signed_tx_base64: "not-base64".to_string(),
        }),
    )
    .await
    .unwrap_err();
    assert_eq!(err.status_code(), axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn tx_open_build_requires_pool_amounts_and_slippage() {
    let s = state();
    let kp = Keypair::new();
    let err = tx_open_build(
        State(s),
        Json(BuildUnsignedTxRequest {
            wallet_pubkey: kp.pubkey().to_string(),
            position_address: None,
            pool_address: None,
            amount_a: None,
            amount_b: None,
            liquidity_amount: None,
            slippage_bps: None,
            tick_lower: None,
            tick_upper: None,
        }),
    )
    .await
    .unwrap_err();
    assert_eq!(err.status_code(), axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn tx_decrease_build_requires_position_pool_and_liquidity() {
    let s = state();
    let kp = Keypair::new();
    let err = tx_decrease_build(
        State(s),
        Json(BuildUnsignedTxRequest {
            wallet_pubkey: kp.pubkey().to_string(),
            position_address: None,
            pool_address: None,
            amount_a: None,
            amount_b: None,
            liquidity_amount: None,
            slippage_bps: None,
            tick_lower: None,
            tick_upper: None,
        }),
    )
    .await
    .unwrap_err();
    assert_eq!(err.status_code(), axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn tx_increase_build_requires_position_pool_and_amounts() {
    let s = state();
    let kp = Keypair::new();
    let err = tx_increase_build(
        State(s),
        Json(BuildUnsignedTxRequest {
            wallet_pubkey: kp.pubkey().to_string(),
            position_address: None,
            pool_address: None,
            amount_a: None,
            amount_b: None,
            liquidity_amount: None,
            slippage_bps: None,
            tick_lower: None,
            tick_upper: None,
        }),
    )
    .await
    .unwrap_err();
    assert_eq!(err.status_code(), axum::http::StatusCode::BAD_REQUEST);
}
