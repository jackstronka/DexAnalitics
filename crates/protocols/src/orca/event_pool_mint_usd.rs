//! Spot USD prices for pool mint A/B aligned with API **Performance** heuristics:
//! GeckoTerminal feed + USDC peg + WSOL implied from pool tick on WSOL/USDC pairs.

use std::sync::Arc;

use anyhow::Context;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use rust_decimal::prelude::ToPrimitive;
use solana_sdk::pubkey::Pubkey;
use spl_token::solana_program::program_pack::Pack;
use spl_token::state::Mint;
use std::str::FromStr;

use crate::orca::pool_reader::WhirlpoolReader;
use crate::rpc::RpcProvider;
use crate::simple_mint_price::{fetch_gecko_solana_mint_prices_usd, stablecoin_usd_if_applicable};

/// Wrapped SOL mint.
const WSOL_MINT: &str = "So11111111111111111111111111111111111111112";

const WSOL_USD_SANITY_MIN: f64 = 10.0;
const WSOL_USD_SANITY_MAX: f64 = 2500.0;

fn is_usdc_mint(mint: &Pubkey) -> bool {
    matches!(
        mint.to_string().as_str(),
        "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
            | "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU"
    )
}

fn is_wsol_mint(mint: &Pubkey) -> bool {
    mint.to_string().as_str() == WSOL_MINT
}

fn wsol_feed_usd_looks_bogus(p: f64) -> bool {
    !p.is_finite() || p <= 0.0 || !(WSOL_USD_SANITY_MIN..=WSOL_USD_SANITY_MAX).contains(&p)
}

/// Raw Whirlpool tick → UI ratio token B per 1 token A (human decimals).
fn b_per_a_ui_decimal(tick: i32, dec_a: u8, dec_b: u8) -> Decimal {
    let ln_10001 = 1.0001_f64.ln();
    let ln_ui =
        (tick as f64) * ln_10001 + ((dec_a as f64) - (dec_b as f64)) * std::f64::consts::LN_10;
    if !ln_ui.is_finite() {
        return Decimal::ZERO;
    }
    let ui = ln_ui.exp();
    if !ui.is_finite() || ui <= 0.0 {
        return Decimal::ZERO;
    }
    Decimal::from_f64(ui).unwrap_or(Decimal::ZERO)
}

fn wsol_usd_from_usdc_pair_tick(
    tick: i32,
    mint_a: &Pubkey,
    mint_b: &Pubkey,
    dec_a: u8,
    dec_b: u8,
) -> Option<f64> {
    let a_usdc = is_usdc_mint(mint_a);
    let b_usdc = is_usdc_mint(mint_b);
    if !(a_usdc ^ b_usdc) {
        return None;
    }
    let wsol = Pubkey::from_str(WSOL_MINT).ok()?;
    if *mint_a != wsol && *mint_b != wsol {
        return None;
    }
    let b_per_a = b_per_a_ui_decimal(tick, dec_a, dec_b);
    if b_per_a.is_zero() {
        return None;
    }
    let ratio = b_per_a.to_f64()?;
    if !ratio.is_finite() || ratio <= 0.0 {
        return None;
    }
    if *mint_a == wsol && b_usdc {
        Some(ratio)
    } else if *mint_b == wsol && a_usdc {
        Some(1.0 / ratio)
    } else {
        None
    }
}

/// Apply WSOL/USDC pool-tick-implied USD to `pa` / `pb` (same rules as API Performance valuation).
///
/// When `position_amounts_raw` is `Some((a,b))`, only adjusts the WSOL leg if that leg's raw token
/// balance is **> 0** (live position card). When `None`, adjusts whenever the WSOL mint heuristic
/// matches (event-time snapshot right after open/close).
#[allow(clippy::too_many_arguments)]
pub fn adjust_pool_mint_usd_with_wsol_tick(
    tick: i32,
    mint_a: &Pubkey,
    mint_b: &Pubkey,
    dec_a: u8,
    dec_b: u8,
    pa: &mut f64,
    pb: &mut f64,
    position_amounts_raw: Option<(u64, u64)>,
) -> bool {
    let Some(implied_sol) = wsol_usd_from_usdc_pair_tick(tick, mint_a, mint_b, dec_a, dec_b) else {
        return false;
    };
    let (a_ok, b_ok) = match position_amounts_raw {
        Some((a, b)) => (a > 0, b > 0),
        None => (true, true),
    };
    let mut changed = false;
    if a_ok && is_wsol_mint(mint_a) && (wsol_feed_usd_looks_bogus(*pa) || is_usdc_mint(mint_b)) {
        *pa = implied_sol;
        changed = true;
    }
    if b_ok && is_wsol_mint(mint_b) && (wsol_feed_usd_looks_bogus(*pb) || is_usdc_mint(mint_a)) {
        *pb = implied_sol;
        changed = true;
    }
    changed
}

async fn fetch_mint_decimals(provider: &RpcProvider, mint: &Pubkey) -> anyhow::Result<u8> {
    let account = provider
        .get_account(mint)
        .await
        .context("fetch mint account")?;
    let mint_state = Mint::unpack(&account.data).context("unpack SPL Mint")?;
    Ok(mint_state.decimals)
}

/// Result of best-effort **event-time** mint USD spot for a Whirlpool pool (post-tx pool read).
#[derive(Debug, Clone)]
pub struct EventPoolMintUsd {
    pub price_a_usd: f64,
    pub price_b_usd: f64,
    /// e.g. `gecko+pool_tick_wsol` or `gecko`
    pub price_source: String,
    pub tick_current: i32,
    pub token_mint_a: String,
    pub token_mint_b: String,
}

/// Fetch pool state, mint decimals, Gecko prices, then apply WSOL/USDC tick override like API valuation.
pub async fn fetch_event_pool_mint_usd_prices(
    provider: Arc<RpcProvider>,
    pool: &Pubkey,
) -> Option<EventPoolMintUsd> {
    let reader = WhirlpoolReader::new(provider.clone());
    let pool_state = reader.get_pool_state(&pool.to_string()).await.ok()?;
    let ma = pool_state.token_mint_a;
    let mb = pool_state.token_mint_b;
    let dec_a = fetch_mint_decimals(provider.as_ref(), &ma).await.ok()?;
    let dec_b = fetch_mint_decimals(provider.as_ref(), &mb).await.ok()?;

    let mints = vec![ma.to_string(), mb.to_string()];
    let px = fetch_gecko_solana_mint_prices_usd(&mints).await;

    let mut pa = *px.get(&ma.to_string()).unwrap_or(&0.0);
    let mut pb = *px.get(&mb.to_string()).unwrap_or(&0.0);

    if let Some(p) = stablecoin_usd_if_applicable(&ma.to_string())
        && (!pa.is_finite() || pa <= 0.0)
    {
        pa = p;
    }
    if let Some(p) = stablecoin_usd_if_applicable(&mb.to_string())
        && (!pb.is_finite() || pb <= 0.0)
    {
        pb = p;
    }

    let mut source = "gecko".to_string();
    if adjust_pool_mint_usd_with_wsol_tick(
        pool_state.tick_current,
        &ma,
        &mb,
        dec_a,
        dec_b,
        &mut pa,
        &mut pb,
        None,
    ) {
        source = "gecko+pool_tick_wsol".to_string();
    }

    if !pa.is_finite() || !pb.is_finite() || pa <= 0.0 || pb <= 0.0 {
        return None;
    }

    Some(EventPoolMintUsd {
        price_a_usd: pa,
        price_b_usd: pb,
        price_source: source,
        tick_current: pool_state.tick_current,
        token_mint_a: ma.to_string(),
        token_mint_b: mb.to_string(),
    })
}

#[cfg(test)]
mod wsol_tick_tests {
    use super::adjust_pool_mint_usd_with_wsol_tick;
    use super::wsol_feed_usd_looks_bogus;
    use super::wsol_usd_from_usdc_pair_tick;
    use solana_sdk::pubkey::Pubkey;
    use std::str::FromStr;

    #[test]
    fn wsol_feed_sanity_flags_tiny_positive_garbage() {
        assert!(wsol_feed_usd_looks_bogus(0.0088));
        assert!(!wsol_feed_usd_looks_bogus(135.0));
    }

    /// Orca SOL/USDC (0.04%) uses SOL = token A, USDC = B — implied SOL/USD from tick should match spot band.
    #[test]
    fn wsol_implied_usd_matches_tick_sol_a_usdc_b() {
        let sol = Pubkey::from_str("So11111111111111111111111111111111111111112").unwrap();
        let usdc = Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap();
        let p = wsol_usd_from_usdc_pair_tick(-25_268, &sol, &usdc, 9, 6).expect("implied SOL USD");
        assert!(
            p > 50.0 && p < 150.0,
            "implied SOL USD from tick -25268 (spot ~$80): got {p}"
        );
    }

    #[test]
    fn adjust_respects_zero_balance_leg() {
        let sol = Pubkey::from_str("So11111111111111111111111111111111111111112").unwrap();
        let usdc = Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap();
        let mut pa = 0.0088_f64;
        let mut pb = 1.0_f64;
        let changed = adjust_pool_mint_usd_with_wsol_tick(
            -25_268,
            &sol,
            &usdc,
            9,
            6,
            &mut pa,
            &mut pb,
            Some((0, 1)),
        );
        assert!(!changed, "SOL leg has 0 balance — should not override");
        assert!(wsol_feed_usd_looks_bogus(pa));
    }
}
