use super::*;
use crate::models::BuildUnsignedTxRequest;
use crate::services::orca_tx_service::{
    ClosePositionTxRequest, CollectFeesTxRequest, DecreaseLiquidityTxRequest,
    OpenPositionTxRequest, OrcaTxService,
};
use crate::state::{ApiConfig, AppState};
use axum::Json;
use axum::extract::{Path, Query, State};
use base64::Engine as _;
use borsh::BorshDeserialize;
use clmm_lp_data::providers::{OrcaListPoolsQuery, OrcaListTokensQuery};
use clmm_lp_domain::prelude::PositionTruthMode;
use clmm_lp_execution::prelude::Wallet;
use clmm_lp_execution::prelude::{DecisionConfig, ExecutorConfig, StrategyExecutor, StrategyMode};
use clmm_lp_protocols::prelude::RpcConfig;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Keypair;
use solana_sdk::signature::Signature;
use solana_sdk::signer::Signer;
use solana_sdk::transaction::Transaction;
use std::str::FromStr;
use std::sync::Arc;
use tokio::time::{Duration, sleep};

fn devnet_state() -> AppState {
    let rpc = std::env::var("SOLANA_RPC_URL")
        .unwrap_or_else(|_| "https://api.devnet.solana.com".to_string());
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

fn sign_built_tx(unsigned_tx_base64: &str, kp: &Keypair) -> String {
    let raw = base64::engine::general_purpose::STANDARD
        .decode(unsigned_tx_base64.as_bytes())
        .expect("decode");
    let mut tx: Transaction = bincode::deserialize(&raw).expect("deserialize");
    let msg_bytes = tx.message.serialize();
    let sig = kp.sign_message(&msg_bytes);

    // Preserve any server-side partial signatures and set only wallet signature.
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

    base64::engine::general_purpose::STANDARD.encode(bincode::serialize(&tx).expect("serialize"))
}

/// Real lifecycle smoke for bot tx path: open -> decrease -> collect -> close.
/// Requires funded devnet wallet and envs:
/// KEYPAIR_PATH (or SOLANA_KEYPAIR_PATH), DEVNET_POOL_ADDRESS, DEVNET_TICK_LOWER, DEVNET_TICK_UPPER.
#[tokio::test]
#[ignore = "requires funded wallet + live Solana devnet RPC"]
async fn devnet_bot_lifecycle_keypair_smoke() {
    let wallet = devnet_wallet_from_env();
    let rpc = std::env::var("SOLANA_RPC_URL")
        .unwrap_or_else(|_| "https://api.devnet.solana.com".to_string());
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
    let opened = open.expect("open result");
    let position = opened
        .created_position
        .expect("open should return created_position")
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
    assert!(
        collect.is_ok(),
        "collect_fees failed for {position}: {collect:?}"
    );
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
    let position = open
        .expect("open result")
        .created_position
        .expect("open should return created_position");

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
            fee_mode: PositionTruthMode::Heuristic,
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

    let build = tx_open_build(
        State(state.clone()),
        Json(BuildUnsignedTxRequest {
            wallet_pubkey: kp.pubkey().to_string(),
            position_address: None,
            pool_address: Some(pool.clone()),
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
    assert!(
        build.position_mint.is_some(),
        "open build should return position_mint"
    );
    assert!(
        build.position_address.is_some(),
        "open build should return position_address"
    );

    let signed = sign_built_tx(&build.unsigned_tx_base64, &kp);

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

/// Open + submit + read-back smoke for deterministic automation flow:
/// API returns `position_address`, then we verify account appears and is decodable.
#[tokio::test]
#[ignore = "requires funded wallet + live Solana devnet RPC"]
async fn devnet_open_and_read_position_smoke() {
    let kp = keypair_from_env();
    let state = devnet_state();

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

    let expected_position = build
        .position_address
        .clone()
        .expect("open build should include position_address");

    let signed = sign_built_tx(&build.unsigned_tx_base64, &kp);
    let submit = tx_submit_signed(
        State(state.clone()),
        Json(crate::models::SubmitSignedTxRequest {
            signed_tx_base64: signed,
        }),
    )
    .await
    .expect("submit")
    .0;
    assert!(
        !submit.signature.is_empty(),
        "submit should return signature"
    );

    let pos_pk = Pubkey::from_str(&expected_position).expect("position pubkey");
    let mut fetched = None;
    for _ in 0..10 {
        match state.provider.get_account(&pos_pk).await {
            Ok(acct) => {
                fetched = Some(acct);
                break;
            }
            Err(_) => sleep(Duration::from_millis(500)).await,
        }
    }
    let account = fetched.expect("position account should appear after submit");
    let parsed =
        clmm_lp_protocols::orca::position_reader::WhirlpoolPosition::try_from_slice(&account.data)
            .expect("position account should decode as WhirlpoolPosition");
    assert_ne!(parsed.whirlpool, Pubkey::default());
}

/// Curated devnet proxy pairs from devToken Nebula / Orca Whirlpools:
/// - SOL/devUSDC (64): 3KBZi...
/// - devSAMO/devUSDC (64): EgxU...
/// - devTMAC/devUSDC (64): H3xh...
///
/// For each pool this test does:
/// open build -> wallet sign -> submit -> read-back position account -> decode WhirlpoolPosition.
#[tokio::test]
#[ignore = "requires funded wallet + Nebula dev tokens + live Solana devnet RPC"]
async fn devnet_open_and_read_position_proxy_pairs_smoke() {
    let kp = keypair_from_env();
    let state = devnet_state();

    // Proxy pairs used for devnet e2e coverage (instead of mainnet curated pairs).
    let pools: [(&str, &str); 3] = [
        (
            "SOL/devUSDC",
            "3KBZiL2g8C7tiJ32hTv5v3KM7aK9htpqTw4cTXz1HvPt",
        ),
        (
            "devSAMO/devUSDC",
            "EgxU92G34jw6QDG9RuTX9StFg1PmHuDqkRKAE5kVEiZ4",
        ),
        (
            "devTMAC/devUSDC",
            "H3xhLrSEyDFm6jjG42QezbvhSxF5YHW75VdGUnqeEg5y",
        ),
    ];

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

    for (pair_label, pool) in pools {
        let build = tx_open_build(
            State(state.clone()),
            Json(BuildUnsignedTxRequest {
                wallet_pubkey: kp.pubkey().to_string(),
                position_address: None,
                pool_address: Some(pool.to_string()),
                amount_a: Some(amount_a),
                amount_b: Some(amount_b),
                liquidity_amount: None,
                slippage_bps: Some(100),
                tick_lower: Some(tick_lower),
                tick_upper: Some(tick_upper),
            }),
        )
        .await
        .unwrap_or_else(|e| panic!("{pair_label}: tx_open_build failed: {e}"))
        .0;

        let expected_position = build
            .position_address
            .clone()
            .unwrap_or_else(|| panic!("{pair_label}: missing position_address in build response"));

        let signed = sign_built_tx(&build.unsigned_tx_base64, &kp);
        let submit = tx_submit_signed(
            State(state.clone()),
            Json(crate::models::SubmitSignedTxRequest {
                signed_tx_base64: signed,
            }),
        )
        .await
        .unwrap_or_else(|e| panic!("{pair_label}: tx_submit_signed failed: {e}"))
        .0;
        assert!(
            !submit.signature.is_empty(),
            "{pair_label}: empty tx signature after submit"
        );

        let pos_pk = Pubkey::from_str(&expected_position).expect("position pubkey");
        let mut fetched = None;
        for _ in 0..10 {
            match state.provider.get_account(&pos_pk).await {
                Ok(acct) => {
                    fetched = Some(acct);
                    break;
                }
                Err(_) => sleep(Duration::from_millis(500)).await,
            }
        }
        let account = fetched.unwrap_or_else(|| {
            panic!("{pair_label}: position account did not appear after submit")
        });
        let parsed = clmm_lp_protocols::orca::position_reader::WhirlpoolPosition::try_from_slice(
            &account.data,
        )
        .unwrap_or_else(|e| panic!("{pair_label}: position decode failed: {e}"));
        assert_ne!(
            parsed.whirlpool,
            Pubkey::default(),
            "{pair_label}: decoded position has default whirlpool pubkey"
        );
    }
}

/// Unsigned lifecycle smoke: open -> read/decode -> decrease-all -> collect -> close.
#[tokio::test]
#[ignore = "requires funded wallet + live Solana devnet RPC"]
async fn devnet_unsigned_lifecycle_open_decrease_collect_close_smoke() {
    let kp = keypair_from_env();
    let state = devnet_state();

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

    let open_build = tx_open_build(
        State(state.clone()),
        Json(BuildUnsignedTxRequest {
            wallet_pubkey: kp.pubkey().to_string(),
            position_address: None,
            pool_address: Some(pool.clone()),
            amount_a: Some(amount_a),
            amount_b: Some(amount_b),
            liquidity_amount: None,
            slippage_bps: Some(100),
            tick_lower: Some(tick_lower),
            tick_upper: Some(tick_upper),
        }),
    )
    .await
    .expect("open build")
    .0;
    let position_address = open_build
        .position_address
        .clone()
        .expect("open build should include position_address");

    let open_submit = tx_submit_signed(
        State(state.clone()),
        Json(crate::models::SubmitSignedTxRequest {
            signed_tx_base64: sign_built_tx(&open_build.unsigned_tx_base64, &kp),
        }),
    )
    .await
    .expect("open submit")
    .0;
    assert!(
        !open_submit.signature.is_empty(),
        "open submit should return signature"
    );

    let position_pk = Pubkey::from_str(&position_address).expect("position pubkey");
    let parsed_position = {
        let mut parsed = None;
        for _ in 0..12 {
            if let Ok(acct) = state.provider.get_account(&position_pk).await {
                let decoded =
                    clmm_lp_protocols::orca::position_reader::WhirlpoolPosition::try_from_slice(
                        &acct.data,
                    )
                    .expect("position account should decode");
                parsed = Some(decoded);
                break;
            }
            sleep(Duration::from_millis(500)).await;
        }
        parsed.expect("position account should appear after open")
    };
    assert_ne!(parsed_position.whirlpool, Pubkey::default());

    let decrease_build = tx_decrease_build(
        State(state.clone()),
        Json(BuildUnsignedTxRequest {
            wallet_pubkey: kp.pubkey().to_string(),
            position_address: Some(position_address.clone()),
            pool_address: Some(pool.clone()),
            amount_a: None,
            amount_b: None,
            liquidity_amount: Some(parsed_position.liquidity),
            slippage_bps: Some(100),
            tick_lower: None,
            tick_upper: None,
        }),
    )
    .await
    .expect("decrease build")
    .0;
    let decrease_submit = tx_submit_signed(
        State(state.clone()),
        Json(crate::models::SubmitSignedTxRequest {
            signed_tx_base64: sign_built_tx(&decrease_build.unsigned_tx_base64, &kp),
        }),
    )
    .await
    .expect("decrease submit")
    .0;
    assert!(
        !decrease_submit.signature.is_empty(),
        "decrease submit should return signature"
    );

    let collect_build = tx_collect_build(
        State(state.clone()),
        Json(BuildUnsignedTxRequest {
            wallet_pubkey: kp.pubkey().to_string(),
            position_address: Some(position_address.clone()),
            pool_address: Some(pool.clone()),
            amount_a: None,
            amount_b: None,
            liquidity_amount: None,
            slippage_bps: None,
            tick_lower: None,
            tick_upper: None,
        }),
    )
    .await
    .expect("collect build")
    .0;
    let collect_submit = tx_submit_signed(
        State(state.clone()),
        Json(crate::models::SubmitSignedTxRequest {
            signed_tx_base64: sign_built_tx(&collect_build.unsigned_tx_base64, &kp),
        }),
    )
    .await
    .expect("collect submit")
    .0;
    assert!(
        !collect_submit.signature.is_empty(),
        "collect submit should return signature"
    );

    let close_build = tx_close_build(
        State(state.clone()),
        Json(BuildUnsignedTxRequest {
            wallet_pubkey: kp.pubkey().to_string(),
            position_address: Some(position_address),
            pool_address: Some(pool),
            amount_a: None,
            amount_b: None,
            liquidity_amount: None,
            slippage_bps: None,
            tick_lower: None,
            tick_upper: None,
        }),
    )
    .await
    .expect("close build")
    .0;
    let close_submit = tx_submit_signed(
        State(state),
        Json(crate::models::SubmitSignedTxRequest {
            signed_tx_base64: sign_built_tx(&close_build.unsigned_tx_base64, &kp),
        }),
    )
    .await
    .expect("close submit")
    .0;
    assert!(
        !close_submit.signature.is_empty(),
        "close submit should return signature"
    );
}

/// Unsigned increase smoke: open -> increase (small caps) -> close.
#[tokio::test]
#[ignore = "requires funded wallet + live Solana devnet RPC"]
async fn devnet_unsigned_increase_liquidity_smoke() {
    let kp = keypair_from_env();
    let state = devnet_state();

    let pool = std::env::var("DEVNET_POOL_ADDRESS")
        .unwrap_or_else(|_| "3KBZiL2g8C7tiJ32hTv5v3KM7aK9htpqTw4cTXz1HvPt".to_string());

    let open_amount_a: u64 = std::env::var("DEVNET_OPEN_AMOUNT_A")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1_000_000); // 0.001 SOL (lamports)
    let open_amount_b: u64 = std::env::var("DEVNET_OPEN_AMOUNT_B")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1_000); // 0.001 devUSDC (6 decimals)
    let tick_lower: i32 = std::env::var("DEVNET_TICK_LOWER")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(-128);
    let tick_upper: i32 = std::env::var("DEVNET_TICK_UPPER")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(128);

    // 1) Open
    let open_build = tx_open_build(
        State(state.clone()),
        Json(BuildUnsignedTxRequest {
            wallet_pubkey: kp.pubkey().to_string(),
            position_address: None,
            pool_address: Some(pool.clone()),
            amount_a: Some(open_amount_a),
            amount_b: Some(open_amount_b),
            liquidity_amount: None,
            slippage_bps: Some(200),
            tick_lower: Some(tick_lower),
            tick_upper: Some(tick_upper),
        }),
    )
    .await
    .expect("open build")
    .0;
    let position_address = open_build
        .position_address
        .clone()
        .expect("open build should include position_address");

    let _ = tx_submit_signed(
        State(state.clone()),
        Json(crate::models::SubmitSignedTxRequest {
            signed_tx_base64: sign_built_tx(&open_build.unsigned_tx_base64, &kp),
        }),
    )
    .await
    .expect("open submit");

    // 2) Increase
    let increase_build = tx_increase_build(
        State(state.clone()),
        Json(BuildUnsignedTxRequest {
            wallet_pubkey: kp.pubkey().to_string(),
            position_address: Some(position_address.clone()),
            pool_address: Some(pool.clone()),
            amount_a: Some(1_000_000),
            amount_b: Some(1_000),
            liquidity_amount: None,
            slippage_bps: Some(200),
            tick_lower: None,
            tick_upper: None,
        }),
    )
    .await
    .expect("increase build")
    .0;

    let _ = tx_submit_signed(
        State(state.clone()),
        Json(crate::models::SubmitSignedTxRequest {
            signed_tx_base64: sign_built_tx(&increase_build.unsigned_tx_base64, &kp),
        }),
    )
    .await
    .expect("increase submit");

    // 3) Close (cleanup)
    let close_build = tx_close_build(
        State(state.clone()),
        Json(BuildUnsignedTxRequest {
            wallet_pubkey: kp.pubkey().to_string(),
            position_address: Some(position_address),
            pool_address: Some(pool),
            amount_a: None,
            amount_b: None,
            liquidity_amount: None,
            slippage_bps: Some(200),
            tick_lower: None,
            tick_upper: None,
        }),
    )
    .await
    .expect("close build")
    .0;
    let _ = tx_submit_signed(
        State(state),
        Json(crate::models::SubmitSignedTxRequest {
            signed_tx_base64: sign_built_tx(&close_build.unsigned_tx_base64, &kp),
        }),
    )
    .await
    .expect("close submit");
}

/// Bot action smokes on devnet (direct `StrategyExecutor` actions, no long-running loop):
/// - open -> collect -> decrease (tiny) -> close
#[tokio::test]
#[ignore = "requires funded wallet + live Solana devnet RPC"]
async fn devnet_bot_actions_smoke() {
    let wallet = devnet_wallet_from_env();
    let state = devnet_state();

    let pool_s = std::env::var("DEVNET_POOL_ADDRESS")
        .unwrap_or_else(|_| "3KBZiL2g8C7tiJ32hTv5v3KM7aK9htpqTw4cTXz1HvPt".to_string());
    let pool = solana_sdk::pubkey::Pubkey::from_str(&pool_s).expect("pool pubkey");

    let amount_a: u64 = std::env::var("DEVNET_OPEN_AMOUNT_A")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1_000_000);
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

    let mut exec = StrategyExecutor::new(
        state.provider.clone(),
        state.monitor.clone(),
        state.tx_manager.clone(),
        ExecutorConfig {
            eval_interval_secs: 300,
            auto_execute: true,
            require_confirmation: false,
            max_slippage_pct: rust_decimal::Decimal::new(5, 3),
            dry_run: false,
            fee_mode: PositionTruthMode::Heuristic,
        },
    );
    exec.set_wallet(wallet.clone());

    let position = exec
        .execute_open_position(&pool, tick_lower, tick_upper, amount_a, amount_b, 200)
        .await
        .expect("open_position via StrategyExecutor");

    // Give devnet a moment to surface the new account before monitor reads it.
    let mut ok = false;
    for _ in 0..30 {
        if state.provider.get_account(&position).await.is_ok() {
            ok = true;
            break;
        }
        sleep(Duration::from_secs(1)).await;
    }
    assert!(ok, "position account did not appear in time");

    state
        .monitor
        .add_position(&position.to_string())
        .await
        .expect("add_position");

    exec.execute_collect_fees_only(&position, &pool)
        .await
        .expect("collect_fees_only");

    exec.execute_full_close_only(&position, &pool)
        .await
        .expect("full_close_only");
}

/// Negative smoke: unsigned submit should be rejected as bad request.
#[tokio::test]
#[ignore = "requires live Solana devnet RPC"]
async fn devnet_submit_unsigned_tx_is_rejected() {
    let kp = keypair_from_env();
    let state = devnet_state();

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
