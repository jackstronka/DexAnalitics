//! One-off Orca Whirlpool swaps (e.g. SOL -> devUSDC on devnet).

use anyhow::{Context, Result};
use clmm_lp_protocols::prelude::{RpcConfig, RpcProvider};
use orca_whirlpools::{
    SwapInstructions, SwapType, WhirlpoolsConfigInput, set_whirlpools_config_address,
    swap_instructions,
};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::signature::Keypair;
use solana_sdk::signer::Signer;
use solana_sdk::transaction::Transaction;
use solana_sdk::{hash::Hash, pubkey::Pubkey};
use std::str::FromStr;
use std::sync::Arc;
use tracing::info;

use super::orca_wallet::load_signing_wallet;

#[derive(Debug, Clone, Copy)]
pub enum CliSwapType {
    ExactIn,
    ExactOut,
}

impl CliSwapType {
    fn into_sdk(self) -> SwapType {
        match self {
            Self::ExactIn => SwapType::ExactIn,
            Self::ExactOut => SwapType::ExactOut,
        }
    }
}

fn build_and_sign(
    payer: &Keypair,
    additional_signers: &[Keypair],
    instructions: Vec<solana_sdk::instruction::Instruction>,
    recent_blockhash: Hash,
) -> Transaction {
    let mut tx = Transaction::new_with_payer(&instructions, Some(&payer.pubkey()));
    let mut signers: Vec<&Keypair> = Vec::with_capacity(1 + additional_signers.len());
    signers.push(payer);
    for s in additional_signers {
        signers.push(s);
    }
    tx.sign(&signers, recent_blockhash);
    tx
}

/// Execute an Orca Whirlpool swap using `orca_whirlpools` SDK.
///
/// Notes:
/// - For swapping SOL, pass `--specified-mint So11111111111111111111111111111111111111112`
///   (native mint / wSOL). The SDK will wrap/unwrap as needed.
pub async fn run_orca_swap(
    pool: String,
    specified_mint: String,
    swap_type: CliSwapType,
    amount: u64,
    slippage_bps: u16,
    keypair: Option<std::path::PathBuf>,
    dry_run: bool,
) -> Result<()> {
    let pool_pk = Pubkey::from_str(pool.trim()).context("invalid pool pubkey")?;
    let mint_pk =
        Pubkey::from_str(specified_mint.trim()).context("invalid specified mint pubkey")?;

    let wallet = load_signing_wallet(keypair)?;
    let provider = Arc::new(RpcProvider::new(RpcConfig::default()));

    let endpoint = provider.current_endpoint().await;
    let config = if endpoint.contains("devnet") {
        WhirlpoolsConfigInput::SolanaDevnet
    } else {
        WhirlpoolsConfigInput::SolanaMainnet
    };
    set_whirlpools_config_address(config)
        .map_err(|e| anyhow::anyhow!("orca set_whirlpools_config_address failed: {e}"))?;
    let rpc = RpcClient::new(endpoint);

    info!(
        pool = %pool_pk,
        specified_mint = %mint_pk,
        swap_type = ?swap_type,
        amount = amount,
        slippage_bps = slippage_bps,
        dry_run = dry_run,
        "Orca swap (orca_whirlpools SDK)"
    );

    let swap_ix: SwapInstructions = swap_instructions(
        &rpc,
        pool_pk,
        amount,
        mint_pk,
        swap_type.into_sdk(),
        Some(slippage_bps),
        Some(wallet.pubkey()),
    )
    .await
    .map_err(|e| anyhow::anyhow!("orca swap_instructions failed: {e}"))?;

    if dry_run {
        println!(
            "dry-run: would send swap tx with {} instructions",
            swap_ix.instructions.len()
        );
        println!("quote: {:?}", swap_ix.quote);
        return Ok(());
    }

    let recent = provider
        .get_latest_blockhash()
        .await
        .context("get blockhash")?;
    let tx = build_and_sign(
        wallet.keypair(),
        &swap_ix.additional_signers,
        swap_ix.instructions,
        recent,
    );

    let sig = provider
        .send_and_confirm_transaction(&tx)
        .await
        .context("send+confirm swap tx")?;
    println!("signature: {sig}");
    println!("quote: {:?}", swap_ix.quote);
    Ok(())
}
