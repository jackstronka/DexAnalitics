//! API-H2.3: hermetic HTTP for position agent + worker settings (no live LLM).

use super::http_test_support::{get_json, request_body, test_router, test_state};
use crate::models::{
    AgentChatResponse, AgentSessionResponse, AgentWorkerSettings, AgentWorkerStatus,
};
use crate::test_env::EnvGuard;
use axum::http::{Method, StatusCode};
use solana_sdk::pubkey::Pubkey;

async fn agent_env() -> (EnvGuard, tempfile::TempDir) {
    let mut env = EnvGuard::lock().await;
    let dir = tempfile::tempdir().expect("tempdir");
    env.set("CLMM_AGENT_DATA_DIR", dir.path());
    (env, dir)
}

#[tokio::test]
async fn agent_worker_settings_and_status_deserializes() {
    let (_env, _dir) = agent_env().await;
    let router = test_router(test_state());
    let (status, body) = get_json(router.clone(), "/api/v1/agent/worker/settings").await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let settings: AgentWorkerSettings =
        serde_json::from_str(&body).expect("AgentWorkerSettings");
    assert!(settings.default_position_scan_interval_hours >= 1);

    let (status, body) = get_json(router, "/api/v1/agent/worker/status").await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let _status: AgentWorkerStatus = serde_json::from_str(&body).expect("AgentWorkerStatus");
}

#[tokio::test]
async fn position_agent_start_then_chat_deserializes() {
    let (_env, _dir) = agent_env().await;
    let pda = Pubkey::new_unique().to_string();
    let router = test_router(test_state());

    let (status, body) = request_body(
        router.clone(),
        Method::POST,
        &format!("/api/v1/positions/{pda}/agent/start"),
        Some(serde_json::json!({ "scan_interval_hours": 4 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let started: AgentSessionResponse =
        serde_json::from_str(&body).expect("AgentSessionResponse");
    assert_eq!(started.session.position_address, pda);
    assert_eq!(started.session.status, "active");
    assert_eq!(started.session.scan_interval_hours, 4);

    let (status, body) = get_json(
        router.clone(),
        &format!("/api/v1/positions/{pda}/agent-chat"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let chat: AgentChatResponse = serde_json::from_str(&body).expect("AgentChatResponse");
    assert_eq!(chat.position_address, pda);
    assert!(chat.session.is_some());
    assert!(
        !chat.messages.is_empty(),
        "start should append an agent info message"
    );
    assert_eq!(chat.messages[0].role, "agent");
}

#[tokio::test]
async fn position_agent_supervisor_stable_error_without_chain_or_db() {
    let (_env, _dir) = agent_env().await;
    let pda = Pubkey::new_unique().to_string();
    let (status, body) = get_json(
        test_router(test_state()),
        &format!("/api/v1/positions/{pda}/agent/supervisor"),
    )
    .await;
    // No DB + dead RPC: must not hang; expect a structured client/server error (not 200 fake economics).
    assert!(
        status == StatusCode::BAD_REQUEST
            || status == StatusCode::INTERNAL_SERVER_ERROR
            || status == StatusCode::SERVICE_UNAVAILABLE
            || status == StatusCode::NOT_FOUND,
        "unexpected supervisor status={status} body={body}"
    );
    let _ = serde_json::from_str::<serde_json::Value>(&body).expect("json error envelope");
}
