//! Phantom (wallet) auth: challenge + verify via signMessage.

use crate::auth::{AuthConfig, AuthState, Role};
use crate::error::{ApiError, ApiResult};
use crate::models::{
    PhantomChallengeRequest, PhantomChallengeResponse, PhantomSessionResponse, PhantomVerifyRequest,
};
use crate::state::{AppState, PhantomNonceEntry};
use axum::Json;
use axum::extract::State;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Signature;
use std::str::FromStr;
use uuid::Uuid;

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn build_challenge_message(wallet_pubkey: &str, nonce: &str, expires_at: u64) -> String {
    // Keep the message stable and explicit to avoid ambiguity.
    // This is what Phantom will display in UI as "Sign message".
    format!(
        "Bociarz LP Strategy Lab\n\
Sign-in request\n\
\n\
wallet: {wallet_pubkey}\n\
nonce: {nonce}\n\
expires_at: {expires_at}\n"
    )
}

fn auth_state_from_env() -> AuthState {
    let mut cfg = AuthConfig::default();
    if let Ok(s) = std::env::var("API_JWT_SECRET") {
        cfg.jwt_secret = s;
    }
    if let Ok(v) = std::env::var("API_JWT_EXPIRY_SECS") {
        if let Ok(secs) = v.parse::<u64>() {
            cfg.token_expiry_secs = secs;
        }
    }
    cfg.require_auth = false;
    AuthState::new(cfg)
}

fn auth_expiry_secs_from_env() -> u64 {
    std::env::var("API_JWT_EXPIRY_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(AuthConfig::default().token_expiry_secs)
}

/// Request a Phantom signMessage challenge.
#[utoipa::path(
    post,
    path = "/auth/phantom/challenge",
    tag = "Auth",
    request_body = PhantomChallengeRequest,
    responses(
        (status = 200, description = "Challenge message to sign", body = PhantomChallengeResponse),
        (status = 400, description = "Bad request")
    )
)]
pub async fn phantom_challenge(
    State(state): State<AppState>,
    Json(req): Json<PhantomChallengeRequest>,
) -> ApiResult<Json<PhantomChallengeResponse>> {
    let _ = Pubkey::from_str(&req.wallet_pubkey)
        .map_err(|_| ApiError::bad_request("Invalid wallet_pubkey"))?;

    let nonce = Uuid::new_v4().to_string();
    let expires_at = now_secs() + 5 * 60;
    let message = build_challenge_message(&req.wallet_pubkey, &nonce, expires_at);

    let mut guard = state.phantom_nonces.write().await;
    guard.insert(
        nonce.clone(),
        PhantomNonceEntry {
            message: message.clone(),
            expires_at,
        },
    );

    Ok(Json(PhantomChallengeResponse {
        nonce,
        message,
        expires_at,
    }))
}

/// Verify Phantom signature and return a session token (JWT).
#[utoipa::path(
    post,
    path = "/auth/phantom/verify",
    tag = "Auth",
    request_body = PhantomVerifyRequest,
    responses(
        (status = 200, description = "Session created", body = PhantomSessionResponse),
        (status = 401, description = "Invalid signature or expired nonce"),
        (status = 400, description = "Bad request")
    )
)]
pub async fn phantom_verify(
    State(state): State<AppState>,
    Json(req): Json<PhantomVerifyRequest>,
) -> ApiResult<Json<PhantomSessionResponse>> {
    let pubkey = Pubkey::from_str(&req.wallet_pubkey)
        .map_err(|_| ApiError::bad_request("Invalid wallet_pubkey"))?;
    let signature = Signature::from_str(&req.signature)
        .map_err(|_| ApiError::bad_request("Invalid signature"))?;

    let entry = {
        let mut guard = state.phantom_nonces.write().await;
        let Some(e) = guard.remove(&req.nonce) else {
            return Err(ApiError::unauthorized("Invalid or already-used nonce"));
        };
        e
    };

    if entry.expires_at < now_secs() {
        return Err(ApiError::unauthorized("Nonce expired"));
    }

    // Verify that the signature was produced by the wallet over our message.
    // `Signature` implements ed25519 verification.
    if !signature.verify(pubkey.as_ref(), entry.message.as_bytes()) {
        return Err(ApiError::unauthorized("Signature verification failed"));
    }

    let auth = auth_state_from_env();
    let token = auth
        .create_token(&req.wallet_pubkey, vec![Role::Execute.as_str().to_string()])
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(PhantomSessionResponse {
        token,
        expires_in_secs: auth_expiry_secs_from_env(),
    }))
}
