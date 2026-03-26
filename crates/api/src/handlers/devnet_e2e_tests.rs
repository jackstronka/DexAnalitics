use super::*;
use crate::models::BuildUnsignedTxRequest;
use crate::services::orca_tx_service::{
    ClosePositionTxRequest, CollectFeesTxRequest, DecreaseLiquidityTxRequest, OpenPositionTxRequest,
    OrcaTxService,
};
use crate::state::{ApiConfig, AppState};
use axum::extract::{Path, Query, State};
use axum::Json;
use clmm_lp_data::providers::{OrcaListPoolsQuery, OrcaListTokensQuery};
use clmm_lp_execution::prelude::Wallet;
use clmm_lp_execution::prelude::{DecisionConfig, ExecutorConfig, StrategyExecutor, StrategyMode};
use clmm_lp_protocols::prelude::{RpcConfig, derive_whirlpool_position_address};
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Keypair;
use solana_sdk::signature::Signature;
use solana_sdk::signer::Signer;
use solana_sdk::transaction::Transaction;
use std::str::FromStr;
use std::sync::Arc;
use base64::Engine as _;
use tokio::time::{sleep, Duration};

fn devnet_state() -> AppState {
    let rpc = std::env::var("SOLANA_RPC_URL").unwrap_or_else(|_| "https://api.devnet.solana.com".to_string());
    let rpc_config = RpcConfig {
        primary_url: rpc,
        ..Default::default()
    };
    AppState::new(rpc_config, ApiConfig::default())
}

/// Manual smoke test for API->RPC path on devnet.
/// Run with: `cargo test -p clmm-lp-api devnet_pool_state_smoke -- --ignored`
#[tokio::test]
#[ignore = "requires live Solana devnet RPC"]
async fn devnet_pool_state_smoke() {
    // Orca docs reference devnet SOL/devUSDC pool.
    let pool = "3KBZiL2g8C7tiJ32hTv5v3KM7aK9htpqTw4cTXz1HvPt".to_string();
    let state = devnet_state();
    let res = get_pool_state(State(state), Path(pool)).await;
    assert!(res.is_ok(), "devnet pool state fetch failed");
}

/// Manual smoke for live Orca REST proxy `/orca/pools`.
#[tokio::test]
#[ignore = "requires live Orca API + network"]
async fn devnet_orca_list_pools_smoke() {
    let mut cfg = ApiConfig::default();
    cfg.orca_public_api_base_url = Some("https://api.orca.so/v2/solana".to_string());
    let state = AppState::new(RpcConfig::default(), cfg);
    let res = orca_list_pools(State(state), Query(OrcaListPoolsQuery::default())).await;
    assert!(res.is_ok(), "orca list pools failed");
    let body = res.unwrap().0;
    assert!(!body.pools.is_empty(), "orca pools response is empty");
}

/// Manual smoke for live Orca REST proxy `/orca/tokens`.
#[tokio::test]
#[ignore = "requires live Orca API + network"]
async fn devnet_orca_list_tokens_smoke() {
    let mut cfg = ApiConfig::default();
    cfg.orca_public_api_base_url = Some("https://api.orca.so/v2/solana".to_string());
    let state = AppState::new(RpcConfig::default(), cfg);
    let res = orca_list_tokens(State(state), Query(OrcaListTokensQuery::default())).await;
    assert!(res.is_ok(), "orca list tokens failed");
    let body = res.unwrap().0;
    assert!(!body.tokens.is_empty(), "orca tokens response is empty");
}

/// Manual smoke for live Orca REST proxy `/orca/protocol`.
#[tokio::test]
#[ignore = "requires live Orca API + network"]
async fn devnet_orca_protocol_smoke() {
    let mut cfg = ApiConfig::default();
    cfg.orca_public_api_base_url = Some("https://api.orca.so/v2/solana".to_string());
    let state = AppState::new(RpcConfig::default(), cfg);
    let res = orca_get_protocol(State(state)).await;
    assert!(res.is_ok(), "orca protocol failed");
}

fn require_keypair_path() -> String {
    std::env::var("KEYPAIR_PATH")
        .ok()
        .or_else(|| std::env::var("SOLANA_KEYPAIR_PATH").ok())
        .expect("set KEYPAIR_PATH or SOLANA_KEYPAIR_PATH for devnet e2e tests")
}

fn devnet_wallet_from_env() -> Arc<Wallet> {
    let path = require_keypair_path();
    Arc::new(
        Wallet::from_file(path, "devnet-e2e")
            .expect("failed to load wallet from KEYPAIR_PATH/SOLANA_KEYPAIR_PATH"),
    )
}

fn keypair_from_env() -> Keypair {
    let path = require_keypair_path();
    let bytes = std::fs::read_to_string(path).expect("read keypair");
    let arr: Vec<u8> = serde_json::from_str(&bytes).expect("json keypair");
    Keypair::new_from_array(arr[..32].try_into().expect("32 bytes"))
}

/// Real lifecycle smoke for bot tx path: open -> decrease -> collect -> close.
/// Requires funded devnet wallet and envs:
/// KEYPAIR_PATH (or SOLANA_KEYPAIR_PATH), DEVNET_POOL_ADDRESS, DEVNET_TICK_LOWER, DEVNET_TICK_UPPER.
#[tokio::test]
#[ignore = "requires funded wallet + live Solana devnet RPC"]
async fn devnet_bot_lifecycle_keypair_smoke() {
    let wallet = devnet_wallet_from_env();
    let rpc = std::env::var("SOLANA_RPC_URL").unwrap_or_else(|_| "https://api.devnet.solana.com".to_string());
    let provider = Arc::new(clmm_lp_protocols::prelude::RpcProvider::new(RpcConfig {
        primary_url: rpc,
        ..Default::default()
    }));
    let mut tx = OrcaTxService::new(provider);
    tx.set_wallet(wallet);

    let pool = std::env::var("DEVNET_POOL_ADDRESS")
        .unwrap_or_else(|_| "3KBZiL2g8C7tiJ32hTv5v3KM7aK9htpqTw4cTXz1HvPt".to_string());
    let amount_a: u64 = std::env::var("DEVNET_OPEN_AMOUNT_A")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1_000);
    let amount_b: u64 = std::env::var("DEVNET_OPEN_AMOUNT_B")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1_000);

    let tick_lower: i32 = std::env::var("DEVNET_TICK_LOWER")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(-128);
    let tick_upper: i32 = std::env::var("DEVNET_TICK_UPPER")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(128);

    let open = tx
        .open_position(OpenPositionTxRequest {
            pool_address: pool.clone(),
            tick_lower,
            tick_upper,
            amount_a,
            amount_b,
            slippage_bps: 100,
        })
        .await;
    assert!(open.is_ok(), "open_position failed: {open:?}");

    let position = derive_whirlpool_position_address(
        &Pubkey::from_str(&pool).expect("pool"),
        tick_lower,
        tick_upper,
    )
    .to_string();

    let decrease = tx
        .decrease_liquidity(DecreaseLiquidityTxRequest {
            position_address: position.clone(),
            pool_address: pool.clone(),
            liquidity_amount: 1,
            token_min_a: 0,
            token_min_b: 0,
        })
        .await;
    assert!(
        decrease.is_ok(),
        "decrease_liquidity failed for {position}: {decrease:?}"
    );
    let collect = tx
        .collect_fees(CollectFeesTxRequest {
            position_address: position.clone(),
            pool_address: pool.clone(),
        })
        .await;
    assert!(collect.is_ok(), "collect_fees failed for {position}: {collect:?}");
    let close = tx
        .close_position(ClosePositionTxRequest {
            position_address: position,
            pool_address: pool,
        })
        .await;
    assert!(close.is_ok(), "close_position failed: {close:?}");
}

/// Strategy-driven bot smoke on devnet:
/// - open a position
/// - add it to monitor
/// - start StrategyExecutor with Periodic mode (interval=0h) + auto_execute
/// - wait for Rebalanced lifecycle event
///
/// Run with: `cargo test -p clmm-lp-api devnet_strategy_driven_rebalance_smoke -- --ignored`
#[tokio::test]
#[ignore = "requires funded wallet + live Solana devnet RPC"]
async fn devnet_strategy_driven_rebalance_smoke() {
    let wallet = devnet_wallet_from_env();
    let rpc = std::env::var("SOLANA_RPC_URL")
        .unwrap_or_else(|_| "https://api.devnet.solana.com".to_string());

    let state = devnet_state();

    // 1) Open an initial position via on-chain tx service.
    let provider = Arc::new(clmm_lp_protocols::prelude::RpcProvider::new(RpcConfig {
        primary_url: rpc,
        ..Default::default()
    }));
    let mut tx = OrcaTxService::new(provider);
    tx.set_wallet(wallet.clone());

    let pool = std::env::var("DEVNET_POOL_ADDRESS")
        .unwrap_or_else(|_| "3KBZiL2g8C7tiJ32hTv5v3KM7aK9htpqTw4cTXz1HvPt".to_string());
    let amount_a: u64 = std::env::var("DEVNET_OPEN_AMOUNT_A")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1_000);
    let amount_b: u64 = std::env::var("DEVNET_OPEN_AMOUNT_B")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1_000);
    let tick_lower: i32 = std::env::var("DEVNET_TICK_LOWER")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(-128);
    let tick_upper: i32 = std::env::var("DEVNET_TICK_UPPER")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(128);

    let open = tx
        .open_position(OpenPositionTxRequest {
            pool_address: pool.clone(),
            tick_lower,
            tick_upper,
            amount_a,
            amount_b,
            slippage_bps: 100,
        })
        .await;
    assert!(open.is_ok(), "open_position failed: {open:?}");

    let pool_pk = Pubkey::from_str(&pool).expect("pool pubkey");
    let position = derive_whirlpool_position_address(&pool_pk, tick_lower, tick_upper);

    // 2) Seed monitor with this position so executor has something to evaluate.
    state
        .monitor
        .add_position(&position.to_string())
        .await
        .expect("add_position");

    // 3) Start StrategyExecutor with auto_execute and strategy mode that always rebalances.
    let mut exec = StrategyExecutor::new(
        state.provider.clone(),
        state.monitor.clone(),
        state.tx_manager.clone(),
        ExecutorConfig {
            eval_interval_secs: 2,
            auto_execute: true,
            require_confirmation: false,
            max_slippage_pct: rust_decimal::Decimal::new(5, 3),
            dry_run: false,
        },
    );
    exec.set_wallet(wallet.clone());

    let mut cfg = DecisionConfig::default();
    cfg.strategy_mode = StrategyMode::Periodic;
    cfg.periodic_interval_hours = 0; // always eligible
    exec.set_decision_config(cfg);

    let lifecycle = exec.lifecycle().clone();
    let exec = Arc::new(exec);
    let exec_task = {
        let exec = exec.clone();
        tokio::spawn(async move { exec.start().await })
    };

    // 4) Wait until we observe Rebalanced event for the old position.
    let mut ok = false;
    for _ in 0..60 {
        let events = lifecycle.get_events(&position).await;
        if events
            .iter()
            .any(|e| e.event_type == clmm_lp_execution::lifecycle::LifecycleEventType::Rebalanced)
        {
            ok = true;
            break;
        }
        sleep(Duration::from_secs(2)).await;
    }

    exec.stop();
    let _ = exec_task.await;

    assert!(ok, "did not observe Rebalanced event within timeout");
}

/// Unsigned tx flow smoke (Phantom emulation by keypair): build -> sign -> submit.
#[tokio::test]
#[ignore = "requires funded wallet + live Solana devnet RPC"]
async fn devnet_unsigned_tx_sign_submit_smoke() {
    let kp = keypair_from_env();

    let state = devnet_state();

    let pool = std::env::var("DEVNET_POOL_ADDRESS").unwrap_or_else(|_| {
        "3KBZiL2g8C7tiJ32hTv5v3KM7aK9htpqTw4cTXz1HvPt".to_string()
    });
    let amount_a: u64 = std::env::var("DEVNET_OPEN_AMOUNT_A")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1_000);
    let amount_b: u64 = std::env::var("DEVNET_OPEN_AMOUNT_B")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1_000);

    let tick_lower: i32 = std::env::var("DEVNET_TICK_LOWER")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(-128);
    let tick_upper: i32 = std::env::var("DEVNET_TICK_UPPER")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(128);

    let build = tx_open_build(
        State(state.clone()),
        Json(BuildUnsignedTxRequest {
            wallet_pubkey: kp.pubkey().to_string(),
            position_address: None,
            pool_address: Some(pool),
            amount_a: Some(amount_a),
            amount_b: Some(amount_b),
            liquidity_amount: None,
            slippage_bps: Some(100),
            tick_lower: Some(tick_lower),
            tick_upper: Some(tick_upper),
        }),
    )
    .await
    .expect("build")
    .0;

    let raw = base64::engine::general_purpose::STANDARD
        .decode(build.unsigned_tx_base64.as_bytes())
        .expect("decode");
    let mut tx: Transaction = bincode::deserialize(&raw).expect("deserialize");
    let msg_bytes = tx.message.serialize();
    let sig = kp.sign_message(&msg_bytes);

    // Preserve any server-side partial signatures (e.g. for additional signers),
    // and only set the wallet signature in the correct slot.
    let required = tx.message.header.num_required_signatures as usize;
    let wallet_pubkey = kp.pubkey();
    let mut set = false;
    for i in 0..required {
        if tx.message.account_keys.get(i) == Some(&wallet_pubkey) {
            tx.signatures[i] = sig;
            set = true;
            break;
        }
    }
    assert!(set, "wallet pubkey not found among required signers");

    let signed = base64::engine::general_purpose::STANDARD
        .encode(bincode::serialize(&tx).expect("serialize"));

    let submit = tx_submit_signed(
        State(state),
        Json(crate::models::SubmitSignedTxRequest {
            signed_tx_base64: signed,
        }),
    )
    .await;
    if let Err(err) = submit {
        assert!(
            err.status_code() == axum::http::StatusCode::UNPROCESSABLE_ENTITY
                || err.status_code() == axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "unexpected submit status: {}",
            err.status_code()
        );
    }
}

/// Negative smoke: unsigned submit should be rejected as bad request.
#[tokio::test]
#[ignore = "requires live Solana devnet RPC"]
async fn devnet_submit_unsigned_tx_is_rejected() {
    let kp = keypair_from_env();
    let state = devnet_state();

    let pool = std::env::var("DEVNET_POOL_ADDRESS").unwrap_or_else(|_| {
        "3KBZiL2g8C7tiJ32hTv5v3KM7aK9htpqTw4cTXz1HvPt".to_string()
    });
    let amount_a: u64 = std::env::var("DEVNET_OPEN_AMOUNT_A")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1_000);
    let amount_b: u64 = std::env::var("DEVNET_OPEN_AMOUNT_B")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1_000);

    let tick_lower: i32 = std::env::var("DEVNET_TICK_LOWER")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(-128);
    let tick_upper: i32 = std::env::var("DEVNET_TICK_UPPER")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(128);

    let build = tx_open_build(
        State(state.clone()),
        Json(BuildUnsignedTxRequest {
            wallet_pubkey: kp.pubkey().to_string(),
            position_address: None,
            pool_address: Some(pool),
            amount_a: Some(amount_a),
            amount_b: Some(amount_b),
            liquidity_amount: None,
            slippage_bps: Some(100),
            tick_lower: Some(tick_lower),
            tick_upper: Some(tick_upper),
        }),
    )
    .await
    .expect("build")
    .0;

    // Force tx to be truly "unsigned" by clearing all required signatures.
    let raw = base64::engine::general_purpose::STANDARD
        .decode(build.unsigned_tx_base64.as_bytes())
        .expect("decode");
    let mut tx: Transaction = bincode::deserialize(&raw).expect("deserialize");
    let required = tx.message.header.num_required_signatures as usize;
    tx.signatures = vec![Signature::default(); required];
    let signed_tx_base64 = base64::engine::general_purpose::STANDARD
        .encode(bincode::serialize(&tx).expect("serialize"));

    let err = tx_submit_signed(
        State(state),
        Json(crate::models::SubmitSignedTxRequest {
            signed_tx_base64: signed_tx_base64,
        }),
    )
    .await
    .expect_err("unsigned transaction must be rejected");
    assert_eq!(err.status_code(), axum::http::StatusCode::BAD_REQUEST);
}

/// Negative smoke: malformed base64 should be rejected at API boundary.
#[tokio::test]
#[ignore = "requires live Solana devnet RPC"]
async fn devnet_submit_invalid_base64_is_rejected() {
    let state = devnet_state();
    let err = tx_submit_signed(
        State(state),
        Json(crate::models::SubmitSignedTxRequest {
            signed_tx_base64: "%%%not-base64%%%".to_string(),
        }),
    )
    .await
    .expect_err("invalid base64 must be rejected");
    assert_eq!(err.status_code(), axum::http::StatusCode::BAD_REQUEST);
}

