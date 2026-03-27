use super::*;
use crate::models::{PhantomChallengeRequest, PhantomVerifyRequest};
use crate::state::{ApiConfig, AppState};
use axum::Json;
use axum::extract::State;
use clmm_lp_protocols::prelude::RpcConfig;
use solana_sdk::signature::{Keypair, Signer};

#[tokio::test]
async fn phantom_challenge_then_verify_returns_token() {
    let state = AppState::new(RpcConfig::default(), ApiConfig::default());
    let kp = Keypair::new();
    let wallet = kp.pubkey().to_string();

    let challenge = phantom_challenge(
        State(state.clone()),
        Json(PhantomChallengeRequest {
            wallet_pubkey: wallet.clone(),
        }),
    )
    .await
    .unwrap()
    .0;

    let sig = kp.sign_message(challenge.message.as_bytes()).to_string();

    let session = phantom_verify(
        State(state),
        Json(PhantomVerifyRequest {
            wallet_pubkey: wallet,
            nonce: challenge.nonce,
            signature: sig,
        }),
    )
    .await
    .unwrap()
    .0;

    assert!(!session.token.is_empty());
    assert!(session.expires_in_secs > 0);
}

#[tokio::test]
async fn phantom_verify_rejects_replay_nonce() {
    let state = AppState::new(RpcConfig::default(), ApiConfig::default());
    let kp = Keypair::new();
    let wallet = kp.pubkey().to_string();

    let challenge = phantom_challenge(
        State(state.clone()),
        Json(PhantomChallengeRequest {
            wallet_pubkey: wallet.clone(),
        }),
    )
    .await
    .unwrap()
    .0;

    let sig = kp.sign_message(challenge.message.as_bytes()).to_string();

    // first ok
    let _ = phantom_verify(
        State(state.clone()),
        Json(PhantomVerifyRequest {
            wallet_pubkey: wallet.clone(),
            nonce: challenge.nonce.clone(),
            signature: sig.clone(),
        }),
    )
    .await
    .unwrap();

    // replay fails (nonce removed on first verify)
    let err = phantom_verify(
        State(state),
        Json(PhantomVerifyRequest {
            wallet_pubkey: wallet,
            nonce: challenge.nonce,
            signature: sig,
        }),
    )
    .await
    .unwrap_err();

    assert_eq!(err.status_code(), axum::http::StatusCode::UNAUTHORIZED);
}
