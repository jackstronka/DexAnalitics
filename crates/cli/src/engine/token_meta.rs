use anyhow::{Context, Result};
use clmm_lp_protocols::rpc::RpcProvider;
use solana_sdk::pubkey::Pubkey;
use spl_token::state::Mint;
use spl_token::solana_program::program_pack::Pack;
use std::str::FromStr;

pub async fn fetch_mint_decimals(rpc: &RpcProvider, mint: &str) -> Result<u8> {
    let pk = Pubkey::from_str(mint).context("Invalid mint pubkey")?;
    let account = rpc.get_account(&pk).await.context("Failed to fetch mint account")?;
    let mint_state = Mint::unpack(&account.data).context("Failed to unpack SPL Mint")?;
    Ok(mint_state.decimals)
}

