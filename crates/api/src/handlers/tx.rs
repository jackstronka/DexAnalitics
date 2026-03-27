//! Transaction build/submit handlers (unsigned tx flow).

use crate::error::{ApiError, ApiResult};
use crate::models::{
    BuildUnsignedTxRequest, BuildUnsignedTxResponse, SubmitSignedTxRequest, SubmitSignedTxResponse,
};
use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use borsh::BorshDeserialize;
use clmm_lp_protocols::prelude::WhirlpoolPosition as ApiWhirlpoolPosition;
use orca_whirlpools::{
    DecreaseLiquidityParam, IncreaseLiquidityParam, WhirlpoolsConfigInput,
    close_position_instructions, decrease_liquidity_instructions, harvest_position_instructions,
    increase_liquidity_instructions, open_position_instructions_with_tick_bounds,
    set_whirlpools_config_address,
};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::instruction::Instruction;
use solana_sdk::message::Message;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Keypair;
use solana_sdk::signature::Signature;
use solana_sdk::transaction::Transaction;
use std::str::FromStr;

const ALLOWED_PROGRAMS: &[&str] = &[
    "11111111111111111111111111111111",            // System
    "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA", // SPL Token
    "ATokenGPvbdGVxr1h4fVnQJQYZ6h8QqKaQqM8y3A5f7", // ATA
    // ATA program id variant used by the Orca Whirlpool SDK.
    "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL",
    "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc", // Whirlpool
    "ComputeBudget111111111111111111111111111",    // Compute budget
    "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb", // Token-2022
];
const MAX_SLIPPAGE_BPS: u16 = 2_000;
const WHIRLPOOL_PROGRAM_ID: &str = "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc";

#[derive(Debug, Clone, Copy)]
enum TxOp {
    Open,
    Increase,
    Decrease,
    Collect,
    Close,
}

fn parse_pubkey(label: &str, v: &str) -> Result<Pubkey, ApiError> {
    Pubkey::from_str(v).map_err(|_| ApiError::bad_request(format!("Invalid {label} pubkey")))
}

async fn build_unsigned(
    state: &AppState,
    req: &BuildUnsignedTxRequest,
    op: TxOp,
) -> Result<BuildUnsignedTxResponse, ApiError> {
    let wallet = parse_pubkey("wallet_pubkey", &req.wallet_pubkey)?;
    validate_build_request(req, op)?;
    let blockhash = state
        .provider
        .get_latest_blockhash()
        .await
        .map_err(|e| ApiError::internal(format!("Failed to fetch blockhash: {e}")))?;

    let instructions: Vec<Instruction>;
    let additional_signers: Vec<Keypair>;
    let mut position_mint_out: Option<Pubkey> = None;
    let mut position_address_out: Option<Pubkey> = None;

    let pool = req
        .pool_address
        .as_ref()
        .map(|s| parse_pubkey("pool_address", s))
        .transpose()?;
    let position = req
        .position_address
        .as_ref()
        .map(|s| parse_pubkey("position_address", s))
        .transpose()?;

    let endpoint = state.provider.current_endpoint().await;
    let config = if endpoint.contains("devnet") {
        WhirlpoolsConfigInput::SolanaDevnet
    } else {
        WhirlpoolsConfigInput::SolanaMainnet
    };
    set_whirlpools_config_address(config).map_err(|e| {
        ApiError::internal(format!("orca set_whirlpools_config_address failed: {e}"))
    })?;
    let rpc = RpcClient::new(endpoint);

    match op {
        TxOp::Open => {
            let pool = pool.expect("pool parsed by validate_build_request");
            let tick_lower = req
                .tick_lower
                .expect("tick_lower parsed by validate_build_request");
            let tick_upper = req
                .tick_upper
                .expect("tick_upper parsed by validate_build_request");

            let amount_a = req
                .amount_a
                .expect("amount_a parsed by validate_build_request");
            let amount_b = req
                .amount_b
                .expect("amount_b parsed by validate_build_request");
            let slippage_bps = req
                .slippage_bps
                .expect("slippage_bps parsed by validate_build_request");
            let param = IncreaseLiquidityParam {
                token_max_a: amount_a,
                token_max_b: amount_b,
            };

            // Use Orca SDK for real instruction building (unsigned build).
            let opened = open_position_instructions_with_tick_bounds(
                &rpc,
                pool,
                tick_lower,
                tick_upper,
                param,
                Some(slippage_bps),
                Some(wallet),
            )
            .await
            .map_err(|e| {
                ApiError::internal(format!("orca open_position_instructions failed: {e}"))
            })?;

            instructions = opened.instructions;
            additional_signers = opened.additional_signers;
            position_mint_out = Some(opened.position_mint);
            let whirlpool_program = Pubkey::from_str(WHIRLPOOL_PROGRAM_ID)
                .map_err(|e| ApiError::internal(format!("Invalid whirlpool program id: {e}")))?;
            let (position_pda, _) = Pubkey::find_program_address(
                &[b"position", opened.position_mint.as_ref()],
                &whirlpool_program,
            );
            position_address_out = Some(position_pda);
        }
        TxOp::Increase => {
            let _pool = pool.expect("pool parsed by validate_build_request");
            let position = position.expect("position parsed by validate_build_request");

            let pos_account = state.provider.get_account(&position).await.map_err(|e| {
                ApiError::internal(format!("Failed to fetch position account: {e}"))
            })?;

            let parsed = ApiWhirlpoolPosition::try_from_slice(&pos_account.data).map_err(|e| {
                ApiError::internal(format!(
                    "Failed to parse WhirlpoolPosition (Borsh) for {position}: {e}"
                ))
            })?;

            let amount_a = req
                .amount_a
                .expect("amount_a parsed by validate_build_request");
            let amount_b = req
                .amount_b
                .expect("amount_b parsed by validate_build_request");
            let slippage = req.slippage_bps;

            let increased = increase_liquidity_instructions(
                &rpc,
                parsed.position_mint,
                IncreaseLiquidityParam {
                    token_max_a: amount_a,
                    token_max_b: amount_b,
                },
                slippage,
                Some(wallet),
            )
            .await
            .map_err(|e| {
                ApiError::internal(format!("orca increase_liquidity_instructions failed: {e}"))
            })?;

            instructions = increased.instructions;
            additional_signers = increased.additional_signers;
        }
        TxOp::Decrease => {
            let _pool = pool.expect("pool parsed by validate_build_request");
            let position = position.expect("position parsed by validate_build_request");

            // Orca SDK needs the position NFT mint; our API passes the position PDA.
            let pos_account = state.provider.get_account(&position).await.map_err(|e| {
                ApiError::internal(format!("Failed to fetch position account: {e}"))
            })?;

            let parsed = ApiWhirlpoolPosition::try_from_slice(&pos_account.data).map_err(|e| {
                ApiError::internal(format!(
                    "Failed to parse WhirlpoolPosition (Borsh) for {position}: {e}"
                ))
            })?;

            let liquidity_amount = req
                .liquidity_amount
                .expect("liquidity_amount parsed by validate_build_request");

            let param = DecreaseLiquidityParam::Liquidity(liquidity_amount);
            let slippage = req.slippage_bps;

            let decreased = decrease_liquidity_instructions(
                &rpc,
                parsed.position_mint,
                param,
                slippage,
                Some(wallet),
            )
            .await
            .map_err(|e| {
                ApiError::internal(format!("orca decrease_liquidity_instructions failed: {e}"))
            })?;

            instructions = decreased.instructions;
            additional_signers = decreased.additional_signers;
        }
        TxOp::Collect => {
            let _pool = pool.expect("pool parsed by validate_build_request");
            let position = position.expect("position parsed by validate_build_request");

            // Orca SDK needs the position NFT mint; our API passes the position PDA.
            let pos_account = state.provider.get_account(&position).await.map_err(|e| {
                ApiError::internal(format!("Failed to fetch position account: {e}"))
            })?;

            let parsed = ApiWhirlpoolPosition::try_from_slice(&pos_account.data).map_err(|e| {
                ApiError::internal(format!(
                    "Failed to parse WhirlpoolPosition (Borsh) for {position}: {e}"
                ))
            })?;

            let harvested = harvest_position_instructions(&rpc, parsed.position_mint, Some(wallet))
                .await
                .map_err(|e| {
                    ApiError::internal(format!("orca harvest_position_instructions failed: {e}"))
                })?;

            instructions = harvested.instructions;
            additional_signers = harvested.additional_signers;
        }
        TxOp::Close => {
            let _pool = pool.expect("pool parsed by validate_build_request");
            let position = position.expect("position parsed by validate_build_request");

            // Orca SDK needs the position NFT mint; our API passes the position PDA.
            let pos_account = state.provider.get_account(&position).await.map_err(|e| {
                ApiError::internal(format!("Failed to fetch position account: {e}"))
            })?;

            let parsed = ApiWhirlpoolPosition::try_from_slice(&pos_account.data).map_err(|e| {
                ApiError::internal(format!(
                    "Failed to parse WhirlpoolPosition (Borsh) for {position}: {e}"
                ))
            })?;

            let slippage = req.slippage_bps;

            let closed =
                close_position_instructions(&rpc, parsed.position_mint, slippage, Some(wallet))
                    .await
                    .map_err(|e| {
                        ApiError::internal(format!("orca close_position_instructions failed: {e}"))
                    })?;

            instructions = closed.instructions;
            additional_signers = closed.additional_signers;
        }
    }

    // Unsigned tx shell with real instruction(s) for client-side signing.
    let message = Message::new(&instructions, Some(&wallet));
    let tx = Transaction::new_unsigned(message);
    let mut tx = tx;
    tx.message.recent_blockhash = blockhash;

    // Pre-sign any additional accounts the Orca SDK needs (partial signature).
    if !additional_signers.is_empty() {
        let refs: Vec<&Keypair> = additional_signers.iter().collect();
        tx.partial_sign(&refs, blockhash);
    }

    let bytes = bincode::serialize(&tx)
        .map_err(|e| ApiError::internal(format!("Failed to serialize tx: {e}")))?;
    Ok(BuildUnsignedTxResponse {
        unsigned_tx_base64: BASE64.encode(bytes),
        correlation_id: uuid::Uuid::new_v4().to_string(),
        expected_program_ids: ALLOWED_PROGRAMS.iter().map(|p| p.to_string()).collect(),
        position_mint: position_mint_out.map(|p| p.to_string()),
        position_address: position_address_out.map(|p| p.to_string()),
    })
}

fn require_pubkey_field(label: &str, value: &Option<String>) -> Result<(), ApiError> {
    let v = value
        .as_deref()
        .ok_or_else(|| ApiError::bad_request(format!("Missing required field: {label}")))?;
    let _ = parse_pubkey(label, v)?;
    Ok(())
}

fn validate_build_request(req: &BuildUnsignedTxRequest, op: TxOp) -> Result<(), ApiError> {
    match op {
        TxOp::Open => {
            require_pubkey_field("pool_address", &req.pool_address)?;
            let amount_a = req
                .amount_a
                .ok_or_else(|| ApiError::bad_request("Missing required field: amount_a"))?;
            let amount_b = req
                .amount_b
                .ok_or_else(|| ApiError::bad_request("Missing required field: amount_b"))?;
            let slippage = req
                .slippage_bps
                .ok_or_else(|| ApiError::bad_request("Missing required field: slippage_bps"))?;

            let tick_lower = req
                .tick_lower
                .ok_or_else(|| ApiError::bad_request("Missing required field: tick_lower"))?;
            let tick_upper = req
                .tick_upper
                .ok_or_else(|| ApiError::bad_request("Missing required field: tick_upper"))?;
            if tick_lower >= tick_upper {
                return Err(ApiError::bad_request("tick_lower must be < tick_upper"));
            }

            // Currently encoded into unsigned tx as Whirlpool `open_position` data.
            // Real production correctness still requires full accounts (tick arrays, vaults).
            let _ = (tick_lower, tick_upper, slippage);
            if amount_a == 0 || amount_b == 0 {
                return Err(ApiError::bad_request("amount_a and amount_b must be > 0"));
            }
            if slippage > MAX_SLIPPAGE_BPS {
                return Err(ApiError::Validation(format!(
                    "slippage_bps too high (max {MAX_SLIPPAGE_BPS})"
                )));
            }
        }
        TxOp::Increase => {
            require_pubkey_field("pool_address", &req.pool_address)?;
            require_pubkey_field("position_address", &req.position_address)?;
            let amount_a = req
                .amount_a
                .ok_or_else(|| ApiError::bad_request("Missing required field: amount_a"))?;
            let amount_b = req
                .amount_b
                .ok_or_else(|| ApiError::bad_request("Missing required field: amount_b"))?;
            if amount_a == 0 || amount_b == 0 {
                return Err(ApiError::bad_request("amount_a and amount_b must be > 0"));
            }
            if let Some(slippage) = req.slippage_bps {
                if slippage > MAX_SLIPPAGE_BPS {
                    return Err(ApiError::Validation(format!(
                        "slippage_bps too high (max {MAX_SLIPPAGE_BPS})"
                    )));
                }
            }
        }
        TxOp::Decrease => {
            require_pubkey_field("pool_address", &req.pool_address)?;
            require_pubkey_field("position_address", &req.position_address)?;
            let liq = req
                .liquidity_amount
                .ok_or_else(|| ApiError::bad_request("Missing required field: liquidity_amount"))?;
            if liq == 0 {
                return Err(ApiError::bad_request("liquidity_amount must be > 0"));
            }
            if let Some(slippage) = req.slippage_bps {
                if slippage > MAX_SLIPPAGE_BPS {
                    return Err(ApiError::Validation(format!(
                        "slippage_bps too high (max {MAX_SLIPPAGE_BPS})"
                    )));
                }
            }
        }
        TxOp::Collect => {
            require_pubkey_field("pool_address", &req.pool_address)?;
            require_pubkey_field("position_address", &req.position_address)?;
        }
        TxOp::Close => {
            require_pubkey_field("pool_address", &req.pool_address)?;
            require_pubkey_field("position_address", &req.position_address)?;
            if let Some(slippage) = req.slippage_bps {
                if slippage > MAX_SLIPPAGE_BPS {
                    return Err(ApiError::Validation(format!(
                        "slippage_bps too high (max {MAX_SLIPPAGE_BPS})"
                    )));
                }
            }
        }
    }
    Ok(())
}

fn policy_gate(tx: &Transaction) -> Result<(), ApiError> {
    for ix in &tx.message.instructions {
        let idx = usize::from(ix.program_id_index);
        let program = tx
            .message
            .account_keys
            .get(idx)
            .ok_or_else(|| ApiError::bad_request("Invalid instruction program index"))?;
        let program_s = program.to_string();
        if !ALLOWED_PROGRAMS.iter().any(|p| *p == program_s) {
            return Err(ApiError::Forbidden(format!(
                "Program {program_s} is not allowed by policy gate"
            )));
        }
    }
    Ok(())
}

/// Build unsigned tx for opening position.
#[utoipa::path(post, path = "/tx/open/build", tag = "Transactions", request_body = BuildUnsignedTxRequest, responses((status=200, body=BuildUnsignedTxResponse)))]
pub async fn tx_open_build(
    State(state): State<AppState>,
    Json(req): Json<BuildUnsignedTxRequest>,
) -> ApiResult<Json<BuildUnsignedTxResponse>> {
    Ok(Json(build_unsigned(&state, &req, TxOp::Open).await?))
}

/// Build unsigned tx for decreasing liquidity.
#[utoipa::path(post, path = "/tx/decrease/build", tag = "Transactions", request_body = BuildUnsignedTxRequest, responses((status=200, body=BuildUnsignedTxResponse)))]
pub async fn tx_decrease_build(
    State(state): State<AppState>,
    Json(req): Json<BuildUnsignedTxRequest>,
) -> ApiResult<Json<BuildUnsignedTxResponse>> {
    Ok(Json(build_unsigned(&state, &req, TxOp::Decrease).await?))
}

/// Build unsigned tx for increasing liquidity.
#[utoipa::path(post, path = "/tx/increase/build", tag = "Transactions", request_body = BuildUnsignedTxRequest, responses((status=200, body=BuildUnsignedTxResponse)))]
pub async fn tx_increase_build(
    State(state): State<AppState>,
    Json(req): Json<BuildUnsignedTxRequest>,
) -> ApiResult<Json<BuildUnsignedTxResponse>> {
    Ok(Json(build_unsigned(&state, &req, TxOp::Increase).await?))
}

/// Build unsigned tx for collecting fees.
#[utoipa::path(post, path = "/tx/collect/build", tag = "Transactions", request_body = BuildUnsignedTxRequest, responses((status=200, body=BuildUnsignedTxResponse)))]
pub async fn tx_collect_build(
    State(state): State<AppState>,
    Json(req): Json<BuildUnsignedTxRequest>,
) -> ApiResult<Json<BuildUnsignedTxResponse>> {
    Ok(Json(build_unsigned(&state, &req, TxOp::Collect).await?))
}

/// Build unsigned tx for closing position.
#[utoipa::path(post, path = "/tx/close/build", tag = "Transactions", request_body = BuildUnsignedTxRequest, responses((status=200, body=BuildUnsignedTxResponse)))]
pub async fn tx_close_build(
    State(state): State<AppState>,
    Json(req): Json<BuildUnsignedTxRequest>,
) -> ApiResult<Json<BuildUnsignedTxResponse>> {
    Ok(Json(build_unsigned(&state, &req, TxOp::Close).await?))
}

/// Submit signed tx (with policy gate + preflight).
#[utoipa::path(post, path = "/tx/submit-signed", tag = "Transactions", request_body = SubmitSignedTxRequest, responses((status=200, body=SubmitSignedTxResponse)))]
pub async fn tx_submit_signed(
    State(state): State<AppState>,
    Json(req): Json<SubmitSignedTxRequest>,
) -> ApiResult<Json<SubmitSignedTxResponse>> {
    let bytes = BASE64
        .decode(req.signed_tx_base64.as_bytes())
        .map_err(|_| ApiError::bad_request("Invalid signed_tx_base64"))?;
    let tx: Transaction = bincode::deserialize(&bytes)
        .map_err(|_| ApiError::bad_request("Invalid serialized transaction"))?;

    if tx.signatures.is_empty() || tx.signatures.iter().all(|s| *s == Signature::default()) {
        return Err(ApiError::bad_request("Transaction is not signed"));
    }
    policy_gate(&tx)?;

    let sim = state
        .provider
        .simulate_transaction(&tx)
        .await
        .map_err(|e| ApiError::internal(format!("simulate failed: {e}")))?;
    if let Some(err) = sim.err {
        return Err(ApiError::Validation(format!("simulate error: {err:?}")));
    }

    let sig = state
        .provider
        .send_transaction(&tx)
        .await
        .map_err(|e| ApiError::internal(format!("send failed: {e}")))?;
    Ok(Json(SubmitSignedTxResponse {
        signature: sig.to_string(),
    }))
}
