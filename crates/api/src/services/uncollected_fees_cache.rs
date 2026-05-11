use crate::services::position_executor::load_wallet_from_env;
use crate::state::{AppState, CachedUncollectedFees};
use clmm_lp_protocols::orca::executor::WhirlpoolExecutor;
use anyhow::Context;
use rust_decimal::Decimal;
use solana_sdk::program_pack::Pack;
use solana_sdk::pubkey::Pubkey;
use spl_token::state::Mint;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Semaphore;

fn refresh_interval_secs() -> u64 {
    std::env::var("CLMM_UNCOLLECTED_FEES_REFRESH_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(60)
}

fn max_parallel() -> usize {
    std::env::var("CLMM_UNCOLLECTED_FEES_MAX_PARALLEL")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|v| *v > 0 && *v <= 64)
        .unwrap_or(8)
}

/// Refresh in-memory cache of uncollected (claimable) fees for all monitored positions.
pub async fn refresh_uncollected_fees_cache_tick(state: &AppState) -> usize {
    // If there's no signer wallet configured, we can still compute quotes, but Orca SDK
    // likes having a payer for ATA derivations in some paths; keep it optional.
    let payer_pubkey = load_wallet_from_env()
        .ok()
        .flatten()
        .map(|w| w.pubkey());

    let positions = state.monitor.get_positions().await;
    if positions.is_empty() {
        return 0;
    }

    let sem = Arc::new(Semaphore::new(max_parallel()));
    let exec = Arc::new(WhirlpoolExecutor::new(state.provider.clone()));
    let mut out: HashMap<String, CachedUncollectedFees> = HashMap::new();
    let now = Instant::now();

    let mut tasks = Vec::with_capacity(positions.len());
    for p in positions {
        let exec = exec.clone();
        let provider = state.provider.clone();
        let sem = sem.clone();
        let payer_pubkey = payer_pubkey;
        let addr = p.address.to_string();
        let pool = p.pool;
        let permit_fut = sem.acquire_owned();
        tasks.push(tokio::spawn(async move {
            let _permit = permit_fut.await.ok()?;
            let pk = Pubkey::from_str(addr.trim()).ok()?;
            let pool_reader = clmm_lp_protocols::prelude::WhirlpoolReader::new(provider.clone());
            let ps = pool_reader.get_pool_state(&pool.to_string()).await.ok()?;
            let dec_a = fetch_mint_decimals(provider.as_ref(), &ps.token_mint_a).await.ok()?;
            let dec_b = fetch_mint_decimals(provider.as_ref(), &ps.token_mint_b).await.ok()?;
            let (raw_a, raw_b) = exec.collect_fees_quote(&pk, payer_pubkey).await.ok()?;
            let a_ui = decimal_ui_from_raw_u64(raw_a, dec_a);
            let b_ui = decimal_ui_from_raw_u64(raw_b, dec_b);
            Some((addr, a_ui, b_ui))
        }));
    }

    for t in tasks {
        if let Ok(Some((addr, a_ui, b_ui))) = t.await {
            out.insert(addr, CachedUncollectedFees { amount_a: a_ui, amount_b: b_ui, updated_at: now });
        }
    }

    if out.is_empty() {
        return 0;
    }

    // Merge into shared cache.
    let mut g = state.uncollected_fees_cache.write().await;
    for (k, v) in out {
        g.insert(k, v);
    }
    g.len()
}

pub fn uncollected_fees_refresh_interval_secs() -> u64 {
    refresh_interval_secs()
}

async fn fetch_mint_decimals(
    provider: &clmm_lp_protocols::prelude::RpcProvider,
    mint: &Pubkey,
) -> anyhow::Result<u8> {
    let account = provider.get_account(mint).await.context("fetch mint account")?;
    let mint_state = Mint::unpack(&account.data).context("unpack SPL Mint")?;
    Ok(mint_state.decimals)
}

fn decimal_ui_from_raw_u64(raw: u64, decimals: u8) -> Decimal {
    if decimals == 0 {
        return Decimal::from(raw);
    }
    let denom = Decimal::from(10u64.pow(decimals as u32));
    Decimal::from(raw) / denom
}

