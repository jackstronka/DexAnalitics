//! API-H2: hermetic HTTP contract for `/dip/*` (zones/runtime/propose + Trigger mocks).

use super::http_test_support::{
    get_json, request_body, request_body_with_headers, test_router, test_state,
};
use crate::models::{
    DipProposeResponse, DipRuntimeResponse, DipSnapshotResponse, DipTriggerDepositCraftResponse,
    DipTriggerSessionStatusDto, DipTriggerVaultResponse,
};
use crate::test_env::EnvGuard;
use axum::http::{Method, StatusCode};
use clmm_lp_data::dip::{DipTriggerSession, save_trigger_session};
use clmm_lp_data::providers::TriggerAuthTokens;
use httpmock::Method::{GET, POST};
use httpmock::MockServer;
use std::fs;
use std::path::Path;

fn write_zones(path: &Path) {
    fs::write(
        path,
        r#"{
  "version": 1,
  "symbols": {
    "SOL": {
      "a": { "min": 100.0, "max": 200.0 },
      "b": { "min": 50.0, "max": 99.99 },
      "c": { "min": 0.0, "max": 49.99 }
    }
  }
}"#,
    )
    .expect("zones.json");
}

fn write_runtime_propose_only(path: &Path) {
    fs::write(
        path,
        "mode: propose-only\nladder:\n  base_order_usd: 25\n  safety_order_usd: 50\n  deviation_pct: 2.5\n  deviation_step_scale: 1.1\n  volume_scale: 1.2\n  max_safety_orders: 3\n  take_profit_pct: 1.5\n  min_order_usd: 10\n",
    )
    .expect("runtime.yaml");
}

#[tokio::test]
async fn dip_snapshot_deserializes_with_zones_path_override() {
    let dir = tempfile::tempdir().expect("tempdir");
    let zones = dir.path().join("zones.json");
    write_zones(&zones);
    let path_q = zones.to_string_lossy().replace('\\', "/");
    let uri = format!("/api/v1/dip/snapshot?include_orders=false&zones_path={path_q}");
    let (status, body) = get_json(test_router(test_state()), &uri).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let snap: DipSnapshotResponse = serde_json::from_str(&body).expect("DipSnapshotResponse");
    assert_eq!(snap.symbol, "SOL");
    assert_eq!(snap.zones.a.min, 100.0);
    assert_eq!(snap.zones.a.max, 200.0);
    assert_eq!(snap.orders_source, "skipped");
    assert!(snap.buy_dip_orders.is_empty());
}

#[tokio::test]
async fn dip_runtime_deserializes_propose_only_from_temp_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let runtime = dir.path().join("runtime.yaml");
    write_runtime_propose_only(&runtime);
    let path_q = runtime.to_string_lossy().replace('\\', "/");
    let uri = format!("/api/v1/dip/runtime?runtime_path={path_q}");
    let (status, body) = get_json(test_router(test_state()), &uri).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let rt: DipRuntimeResponse = serde_json::from_str(&body).expect("DipRuntimeResponse");
    assert!(!rt.file_missing);
    assert_eq!(rt.mode, "propose-only");
    assert_eq!(rt.ladder.base_order_usd, 25.0);
    // Never leak secrets in the public runtime DTO.
    assert!(!body.to_ascii_lowercase().contains("access_token"));
}

#[tokio::test]
async fn dip_trigger_session_unattached_deserializes() {
    let mut env = EnvGuard::lock().await;
    let dir = tempfile::tempdir().expect("tempdir");
    env.set("CLMM_DIP_TRIGGER_SESSION_DIR", dir.path());
    let (status, body) = get_json(
        test_router(test_state()),
        "/api/v1/dip/trigger/session?wallet_id=w1",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let s: DipTriggerSessionStatusDto =
        serde_json::from_str(&body).expect("DipTriggerSessionStatusDto");
    assert_eq!(s.wallet_id, "w1");
    assert!(!s.attached);
    assert!(s.wallet_pubkey.is_empty());
}

#[tokio::test]
async fn dip_propose_appends_jsonl_in_tempdir() {
    let dir = tempfile::tempdir().expect("tempdir");
    let jsonl = dir.path().join("agent_decisions.jsonl");
    let (status, body) = request_body(
        test_router(test_state()),
        Method::POST,
        "/api/v1/dip/propose",
        Some(serde_json::json!({
            "action": "hold",
            "mode": "propose-only",
            "rationale": "API-H2 hermetic propose",
            "refs": ["config/dip/zones.json"],
            "zone": "none",
            "path": jsonl.to_string_lossy()
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let resp: DipProposeResponse = serde_json::from_str(&body).expect("DipProposeResponse");
    assert!(resp.written);
    assert!(jsonl.is_file(), "jsonl missing");
    let raw = fs::read_to_string(&jsonl).expect("read jsonl");
    assert!(raw.contains("dip_propose") || raw.contains("hold"));
}

#[tokio::test]
async fn dip_live_create_rejects_without_confirm_execute() {
    let (status, body) = request_body(
        test_router(test_state()),
        Method::POST,
        "/api/v1/dip/trigger/orders/price",
        Some(serde_json::json!({
            "wallet_id": "w1",
            "confirm_execute": false,
            "input_mint": "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            "output_mint": "So11111111111111111111111111111111111111112",
            "amount": "1000000",
            "trigger_mint": "So11111111111111111111111111111111111111112",
            "trigger_condition": "below",
            "trigger_price_usd": 90.0
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");
    assert!(
        body.contains("confirm_execute"),
        "expected confirm_execute gate, body={body}"
    );
}

#[tokio::test]
async fn dip_live_create_rejects_propose_only_even_with_confirm() {
    let mut env = EnvGuard::lock().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let runtime = dir.path().join("runtime.yaml");
    write_runtime_propose_only(&runtime);
    env.set("CLMM_DIP_RUNTIME_PATH", &runtime);

    let (status, body) = request_body(
        test_router(test_state()),
        Method::POST,
        "/api/v1/dip/trigger/orders/price",
        Some(serde_json::json!({
            "wallet_id": "w1",
            "confirm_execute": true,
            "input_mint": "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            "output_mint": "So11111111111111111111111111111111111111112",
            "amount": "1000000",
            "trigger_mint": "So11111111111111111111111111111111111111112",
            "trigger_condition": "below",
            "trigger_price_usd": 90.0
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");
    assert!(
        body.contains("propose-only") || body.contains("live"),
        "expected mode gate, body={body}"
    );
}

/// API-H2.2: snapshot orders via httpmock Trigger history (no public Jupiter).
#[tokio::test]
async fn dip_snapshot_orders_history_via_httpmock() {
    let mut env = EnvGuard::lock().await;
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET)
            .path("/orders/history")
            .header("x-api-key", "test-key")
            .header("Authorization", "Bearer jwt-1");
        then.status(200).json_body(serde_json::json!({
            "orders": [{
                "id": "ord-a",
                "orderType": "single",
                "orderState": "open",
                "inputMint": "USDC",
                "outputMint": "SOL",
                "triggerMint": "SOL",
                "triggerCondition": "below",
                "triggerPriceUsd": 90.5,
                "remainingInputAmount": "1000"
            }],
            "pagination": { "total": 1, "limit": 50, "offset": 0 }
        }));
    });
    env.set("CLMM_JUPITER_TRIGGER_BASE_URL", server.base_url());

    let dir = tempfile::tempdir().expect("tempdir");
    let zones = dir.path().join("zones.json");
    write_zones(&zones);
    let path_q = zones.to_string_lossy().replace('\\', "/");
    let uri = format!("/api/v1/dip/snapshot?include_orders=true&zones_path={path_q}");

    let (status, body) = request_body_with_headers(
        test_router(test_state()),
        Method::GET,
        &uri,
        &[
            ("x-api-key", "test-key"),
            ("Authorization", "Bearer jwt-1"),
        ],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let snap: DipSnapshotResponse = serde_json::from_str(&body).expect("DipSnapshotResponse");
    assert_eq!(snap.orders_source, "trigger_v2_orders_history");
    assert_eq!(snap.buy_dip_orders.len(), 1);
    assert_eq!(snap.buy_dip_orders[0].id, "ord-a");
    assert_eq!(snap.buy_dip_orders[0].trigger_price_usd, Some(90.5));
}

/// API-H2.2: vault + deposit/craft via httpmock + seeded Trigger session file.
#[tokio::test]
async fn dip_trigger_vault_and_craft_via_httpmock() {
    let mut env = EnvGuard::lock().await;
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET)
            .path("/vault")
            .header("Authorization", "Bearer jwt-v");
        then.status(200).json_body(serde_json::json!({
            "userPubkey": "User111",
            "vaultPubkey": "Vault222",
            "privyVaultId": "privy-1"
        }));
    });
    server.mock(|when, then| {
        when.method(POST)
            .path("/deposit/craft")
            .header("Authorization", "Bearer jwt-v");
        then.status(200).json_body(serde_json::json!({
            "transaction": "AQIDBA==",
            "requestId": "req-1",
            "receiverAddress": "Vault222",
            "mint": "So11111111111111111111111111111111111111112",
            "amount": "1000000",
            "tokenDecimals": 6
        }));
    });
    env.set("CLMM_JUPITER_TRIGGER_BASE_URL", server.base_url());

    let sess_dir = tempfile::tempdir().expect("sess");
    env.set("CLMM_DIP_TRIGGER_SESSION_DIR", sess_dir.path());
    save_trigger_session(
        sess_dir.path(),
        &DipTriggerSession {
            wallet_id: "wmock".into(),
            wallet_pubkey: "User111".into(),
            tokens: TriggerAuthTokens {
                access_token: "jwt-v".into(),
                refresh_token: None,
                expires_at: Some("2099-01-01T00:00:00Z".into()),
                wallet_pubkey: "User111".into(),
            },
            attached_at: "2026-11-11T00:00:00Z".into(),
        },
    )
    .expect("save session");

    let (status, body) = request_body_with_headers(
        test_router(test_state()),
        Method::GET,
        "/api/v1/dip/trigger/vault?wallet_id=wmock",
        &[("x-api-key", "test-key")],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let vault: DipTriggerVaultResponse =
        serde_json::from_str(&body).expect("DipTriggerVaultResponse");
    assert_eq!(vault.vault_pubkey, "Vault222");
    assert_eq!(vault.wallet_id, "wmock");

    let (status, body) = request_body_with_headers(
        test_router(test_state()),
        Method::POST,
        "/api/v1/dip/trigger/deposit/craft",
        &[("x-api-key", "test-key")],
        Some(serde_json::json!({
            "wallet_id": "wmock",
            "input_mint": "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            "output_mint": "So11111111111111111111111111111111111111112",
            "amount": "1000000",
            "order_type": "price",
            "order_sub_type": "single"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let craft: DipTriggerDepositCraftResponse =
        serde_json::from_str(&body).expect("DipTriggerDepositCraftResponse");
    assert_eq!(craft.request_id, "req-1");
    assert_eq!(craft.transaction_base64, "AQIDBA==");
}
