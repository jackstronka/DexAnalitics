//! Compute best-effort USD valuations for positions.
//!
//! The monitor tracks on-chain state and IL/fees in raw token units, but it does not compute USD.
//! For dashboard/UI we derive USD value from:
//! - pool state (tick/sqrt_price) + position liquidity -> token amounts (raw u64)
//! - mint decimals (on-chain SPL mint)
//! - free USD prices (Jupiter v2 + fallbacks — see `price_fetch`)

use crate::error::ApiError;
use crate::models::UncollectedFeesInfo;
use crate::services::price_fetch::fetch_mint_prices_usd;
use anyhow::Context;
use clmm_lp_execution::monitor::{MonitoredPosition, PositionPnL};
use clmm_lp_protocols::orca::pool_reader::WhirlpoolReader;
use clmm_lp_protocols::orca::position_reader::PositionReader;
use clmm_lp_protocols::rpc::RpcProvider;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use rust_decimal::prelude::ToPrimitive;
use solana_sdk::pubkey::Pubkey;
use spl_token::solana_program::program_pack::Pack;
use spl_token::state::Mint;
use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;
use std::sync::Arc;

/// Tick range expressed as **USDC per 1 unit of the non-USDC token** when the pool has exactly one USDC leg.
#[derive(Debug, Clone)]
pub struct TickRangeUsdc {
    pub lower: Decimal,
    pub upper: Decimal,
    /// e.g. `per 1 SOL`
    pub quote: String,
}

#[derive(Debug, Clone)]
pub struct PositionUsdValuation {
    pub value_usd: Decimal,
    pub fees_usd: Decimal,
    pub amount_a_raw: u64,
    pub amount_b_raw: u64,
    pub range_usdc: Option<TickRangeUsdc>,
    /// Spot is inside `[tick_lower, tick_upper)` using **fresh** pool tick from RPC.
    pub in_range: bool,
    /// Human-token uncollected fees (Whirlpool `fee_owed_*` × 10^-decimals).
    pub fees_owed_a_ui: Decimal,
    pub fees_owed_b_ui: Decimal,
    pub token_a_label: String,
    pub token_b_label: String,
}

/// Wrapped SOL (native mint) — same as SPL `spl_token::native_mint::ID`.
const WSOL_MINT: &str = "So11111111111111111111111111111111111111112";

fn is_usdc_mint(mint: &Pubkey) -> bool {
    matches!(
        mint.to_string().as_str(),
        // Mainnet + devnet USDC (common SPL mints)
        "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
            | "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU"
    )
}

fn is_wsol_mint(mint: &Pubkey) -> bool {
    mint.to_string().as_str() == WSOL_MINT
}

/// When Jupiter/Gecko omit **WSOL** in the mint→USD map, `unwrap_or(0.0)` drops the entire SOL leg
/// (~half the position value on SOL/USDC). Derive **USD per 1 SOL** from the pool tick when the pool
/// is exactly USDC + WSOL (Orca Whirlpool `token_b` per `token_a` convention).
/// SOL spot in USD should stay in a loose band; feeds sometimes return **wrong small positives**
/// (broken id / unit mix-up), which skips `<= 0` fallback and nukes the USD leg (~\$1–2 total).
const WSOL_USD_SANITY_MIN: f64 = 10.0;
const WSOL_USD_SANITY_MAX: f64 = 2500.0;

fn wsol_feed_usd_looks_bogus(p: f64) -> bool {
    !p.is_finite() || p <= 0.0 || p < WSOL_USD_SANITY_MIN || p > WSOL_USD_SANITY_MAX
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
        // B per A = USDC per SOL
        Some(ratio)
    } else if *mint_b == wsol && a_usdc {
        // B per A = SOL per USDC → USD per SOL
        Some(1.0 / ratio)
    } else {
        None
    }
}

fn token_short_label(mint: &Pubkey) -> String {
    match mint.to_string().as_str() {
        "So11111111111111111111111111111111111111112" => "SOL".to_string(),
        "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v" => "USDC".to_string(),
        "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB" => "USDT".to_string(),
        s => {
            if s.len() > 10 {
                format!("{}…{}", &s[..4], &s[s.len().saturating_sub(4)..])
            } else {
                s.to_string()
            }
        }
    }
}

/// Raw Whirlpool price `1.0001^tick` (token B per token A, raw) → UI ratio (B human per 1 A human).
///
/// Implemented as `exp(tick * ln(1.0001) + (dec_a - dec_b) * ln(10))`. The protocols helper
/// `tick_to_price` uses `f64.powi(tick)` on the raw ratio alone, which **underflows to 0** for
/// large negative ticks (e.g. -25276) before decimals are applied — breaking USDC range display.
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

/// USDC-denominated bounds for the position range when the pool is USDC / one other token.
pub fn compute_tick_range_usdc(
    tick_lower: i32,
    tick_upper: i32,
    mint_a: &Pubkey,
    mint_b: &Pubkey,
    dec_a: u8,
    dec_b: u8,
) -> Option<TickRangeUsdc> {
    let a_usdc = is_usdc_mint(mint_a);
    let b_usdc = is_usdc_mint(mint_b);
    if !a_usdc && !b_usdc || a_usdc && b_usdc {
        return None;
    }

    let v_lo_tick = b_per_a_ui_decimal(tick_lower, dec_a, dec_b);
    let v_hi_tick = b_per_a_ui_decimal(tick_upper, dec_a, dec_b);

    let usdc_lo = if b_usdc {
        v_lo_tick
    } else {
        if v_lo_tick.is_zero() {
            return None;
        }
        Decimal::ONE / v_lo_tick
    };
    let usdc_hi = if b_usdc {
        v_hi_tick
    } else {
        if v_hi_tick.is_zero() {
            return None;
        }
        Decimal::ONE / v_hi_tick
    };

    let lower = usdc_lo.min(usdc_hi);
    let upper = usdc_lo.max(usdc_hi);

    let quote = if b_usdc {
        format!("per 1 {}", token_short_label(mint_a))
    } else {
        format!("per 1 {}", token_short_label(mint_b))
    };

    Some(TickRangeUsdc {
        lower,
        upper,
        quote,
    })
}

/// One pool RPC read: optional USDC range line + **in range** (spot inside `[tick_lower, tick_upper)`).
pub async fn range_usdc_and_in_range_for_pool_ticks(
    provider: Arc<RpcProvider>,
    pool: &Pubkey,
    tick_lower: i32,
    tick_upper: i32,
) -> (Option<TickRangeUsdc>, bool) {
    let pool_reader = WhirlpoolReader::new(provider.clone());
    let Some(pool_state) = pool_reader.get_pool_state(&pool.to_string()).await.ok() else {
        return (None, false);
    };
    let in_range = pool_state.is_tick_in_range(tick_lower, tick_upper);
    let Some(dec_a) = fetch_mint_decimals(provider.as_ref(), &pool_state.token_mint_a)
        .await
        .ok()
    else {
        return (None, in_range);
    };
    let Some(dec_b) = fetch_mint_decimals(provider.as_ref(), &pool_state.token_mint_b)
        .await
        .ok()
    else {
        return (None, in_range);
    };
    let range = compute_tick_range_usdc(
        tick_lower,
        tick_upper,
        &pool_state.token_mint_a,
        &pool_state.token_mint_b,
        dec_a,
        dec_b,
    );
    (range, in_range)
}

/// Pool + ticks only (no `MonitoredPosition`); used by Orca RPC scan and callers that already have ticks.
pub async fn tick_range_usdc_for_pool_ticks(
    provider: Arc<RpcProvider>,
    pool: &Pubkey,
    tick_lower: i32,
    tick_upper: i32,
) -> Option<TickRangeUsdc> {
    range_usdc_and_in_range_for_pool_ticks(provider, pool, tick_lower, tick_upper)
        .await
        .0
}

/// Pool fetch + decimals; used when USD valuation fails but we still want a USDC range line.
pub async fn tick_range_usdc_for_position(
    provider: Arc<RpcProvider>,
    position: &MonitoredPosition,
) -> Option<TickRangeUsdc> {
    tick_range_usdc_for_pool_ticks(
        provider,
        &position.pool,
        position.on_chain.tick_lower,
        position.on_chain.tick_upper,
    )
    .await
}

/// Per-token `fee_owed` in human units (pool + mint decimals only — no liquidity math, no USD prices).
///
/// Use when [`compute_position_usd_valuation`] fails: the dashboard can still show Orca-style rows
/// while USDC range often comes from [`tick_range_usdc_for_position`], which tolerates the same
/// RPC calls with `.ok()` and may succeed on a fresh fetch.
pub async fn uncollected_fees_info_for_position(
    provider: Arc<RpcProvider>,
    position: &MonitoredPosition,
) -> Option<UncollectedFeesInfo> {
    let pool_reader = WhirlpoolReader::new(provider.clone());
    let pool_state = pool_reader
        .get_pool_state(&position.pool.to_string())
        .await
        .ok()?;
    let dec_a = fetch_mint_decimals(provider.as_ref(), &pool_state.token_mint_a)
        .await
        .ok()?;
    let dec_b = fetch_mint_decimals(provider.as_ref(), &pool_state.token_mint_b)
        .await
        .ok()?;
    let fees_a_ui = ui_amount(position.on_chain.fees_owed_a, dec_a);
    let fees_b_ui = ui_amount(position.on_chain.fees_owed_b, dec_b);
    Some(UncollectedFeesInfo {
        token_a_label: token_short_label(&pool_state.token_mint_a),
        token_b_label: token_short_label(&pool_state.token_mint_b),
        amount_a: Decimal::from_f64_retain(fees_a_ui).unwrap_or(Decimal::ZERO),
        amount_b: Decimal::from_f64_retain(fees_b_ui).unwrap_or(Decimal::ZERO),
    })
}

async fn fetch_mint_decimals(provider: &RpcProvider, mint: &Pubkey) -> anyhow::Result<u8> {
    let account = provider
        .get_account(mint)
        .await
        .context("fetch mint account")?;
    let mint_state = Mint::unpack(&account.data).context("unpack SPL Mint")?;
    Ok(mint_state.decimals)
}

fn ui_amount(raw: u64, decimals: u8) -> f64 {
    if decimals == 0 {
        return raw as f64;
    }
    let denom = 10f64.powi(i32::from(decimals));
    (raw as f64) / denom
}

/// Refresh `fee_owed_*` from one RPC read so `GET /positions/:addr` matches Orca / wallet UIs without
/// waiting for the monitor poll (stale fees were a common source of `$0.000` vs tiny real balances).
pub async fn refresh_position_fees_from_chain(
    provider: Arc<RpcProvider>,
    position: &mut MonitoredPosition,
) {
    let reader = PositionReader::new(provider);
    let Ok(fresh) = reader.get_position(&position.address.to_string()).await else {
        return;
    };
    position.on_chain.fees_owed_a = fresh.fees_owed_a;
    position.on_chain.fees_owed_b = fresh.fees_owed_b;
    position.pnl.fees_earned_a = fresh.fees_owed_a;
    position.pnl.fees_earned_b = fresh.fees_owed_b;
}

/// Build a [`MonitoredPosition`] from RPC when the address is **not** in the in-memory monitor
/// (e.g. opened before `monitor.add_position`, or API restarted). Used by `GET /positions/:address`.
pub async fn monitored_position_from_chain(
    provider: Arc<RpcProvider>,
    position_address: &Pubkey,
) -> Result<MonitoredPosition, ApiError> {
    let position_reader = PositionReader::new(provider.clone());
    let on_chain = position_reader
        .get_position(&position_address.to_string())
        .await
        .map_err(|e| ApiError::not_found(format!("Position not found: {e}")))?;

    let pool_reader = WhirlpoolReader::new(provider.clone());
    let pool_state = pool_reader
        .get_pool_state(&on_chain.pool.to_string())
        .await
        .map_err(|e| ApiError::internal(format!("pool state: {e}")))?;
    let in_range = pool_state.is_tick_in_range(on_chain.tick_lower, on_chain.tick_upper);

    let pnl = PositionPnL {
        fees_earned_a: on_chain.fees_owed_a,
        fees_earned_b: on_chain.fees_owed_b,
        ..PositionPnL::default()
    };

    Ok(MonitoredPosition {
        address: on_chain.address,
        pool: on_chain.pool,
        on_chain,
        pnl,
        in_range,
        last_updated: chrono::Utc::now(),
    })
}

/// Compute USD valuation for a single monitored position (best-effort).
pub async fn compute_position_usd_valuation(
    provider: Arc<RpcProvider>,
    position: &MonitoredPosition,
    prices_usd: &BTreeMap<String, f64>,
) -> Result<PositionUsdValuation, ApiError> {
    let pool_reader = WhirlpoolReader::new(provider.clone());
    let position_reader = PositionReader::new(provider.clone());

    let pool_state = pool_reader
        .get_pool_state(&position.pool.to_string())
        .await
        .map_err(|e| ApiError::internal(format!("pool state fetch failed: {e}")))?;

    let in_range =
        pool_state.is_tick_in_range(position.on_chain.tick_lower, position.on_chain.tick_upper);

    let (amount_a_raw, amount_b_raw) = position_reader.calculate_token_amounts(
        &position.on_chain,
        pool_state.tick_current,
        pool_state.sqrt_price,
    );

    let dec_a = fetch_mint_decimals(provider.as_ref(), &pool_state.token_mint_a)
        .await
        .map_err(|e| ApiError::internal(format!("mint decimals fetch failed (A): {e}")))?;
    let dec_b = fetch_mint_decimals(provider.as_ref(), &pool_state.token_mint_b)
        .await
        .map_err(|e| ApiError::internal(format!("mint decimals fetch failed (B): {e}")))?;

    let mut pa = prices_usd
        .get(&pool_state.token_mint_a.to_string())
        .copied()
        .unwrap_or(0.0);
    let mut pb = prices_usd
        .get(&pool_state.token_mint_b.to_string())
        .copied()
        .unwrap_or(0.0);

    // Never drop the USDC leg when a stable mint is missing from the aggregator map.
    if is_usdc_mint(&pool_state.token_mint_a) && (!pa.is_finite() || pa <= 0.0) {
        pa = 1.0;
    }
    if is_usdc_mint(&pool_state.token_mint_b) && (!pb.is_finite() || pb <= 0.0) {
        pb = 1.0;
    }

    let implied_sol_usd = wsol_usd_from_usdc_pair_tick(
        pool_state.tick_current,
        &pool_state.token_mint_a,
        &pool_state.token_mint_b,
        dec_a,
        dec_b,
    );

    if amount_a_raw > 0 && is_wsol_mint(&pool_state.token_mint_a) && wsol_feed_usd_looks_bogus(pa) {
        if let Some(p) = implied_sol_usd {
            tracing::info!(
                pool = %position.pool,
                tick = pool_state.tick_current,
                feed_usd = pa,
                implied_sol_usd = p,
                "WSOL USD from feed missing or implausible; using pool tick implied USDC/SOL"
            );
            pa = p;
        }
    }
    if amount_b_raw > 0 && is_wsol_mint(&pool_state.token_mint_b) && wsol_feed_usd_looks_bogus(pb) {
        if let Some(p) = implied_sol_usd {
            tracing::info!(
                pool = %position.pool,
                tick = pool_state.tick_current,
                feed_usd = pb,
                implied_sol_usd = p,
                "WSOL USD from feed missing or implausible; using pool tick implied USDC/SOL"
            );
            pb = p;
        }
    }

    let a_ui = ui_amount(amount_a_raw, dec_a);
    let b_ui = ui_amount(amount_b_raw, dec_b);
    let fees_a_ui = ui_amount(position.on_chain.fees_owed_a, dec_a);
    let fees_b_ui = ui_amount(position.on_chain.fees_owed_b, dec_b);

    let value_usd_f = a_ui * pa + b_ui * pb;

    let fees_owed_a_ui = Decimal::from_f64_retain(fees_a_ui).unwrap_or(Decimal::ZERO);
    let fees_owed_b_ui = Decimal::from_f64_retain(fees_b_ui).unwrap_or(Decimal::ZERO);
    let pa_d = Decimal::from_f64_retain(pa).unwrap_or(Decimal::ZERO);
    let pb_d = Decimal::from_f64_retain(pb).unwrap_or(Decimal::ZERO);
    // Decimal end-to-end (avoid f64 underflow on tiny fee × price).
    let fees_usd = fees_owed_a_ui * pa_d + fees_owed_b_ui * pb_d;

    let range_usdc = compute_tick_range_usdc(
        position.on_chain.tick_lower,
        position.on_chain.tick_upper,
        &pool_state.token_mint_a,
        &pool_state.token_mint_b,
        dec_a,
        dec_b,
    );

    let token_a_label = token_short_label(&pool_state.token_mint_a);
    let token_b_label = token_short_label(&pool_state.token_mint_b);

    if (position.on_chain.fees_owed_a > 0 || position.on_chain.fees_owed_b > 0)
        && fees_usd.is_zero()
    {
        tracing::warn!(
            mint_a = %pool_state.token_mint_a,
            mint_b = %pool_state.token_mint_b,
            pa,
            pb,
            fee_owed_a = position.on_chain.fees_owed_a,
            fee_owed_b = position.on_chain.fees_owed_b,
            "non-zero fee_owed but USD fees 0 — check price feed for pool mints (unwrap_or 0.0 if missing)"
        );
    }

    Ok(PositionUsdValuation {
        value_usd: Decimal::from_f64(value_usd_f).unwrap_or(Decimal::ZERO),
        fees_usd,
        amount_a_raw,
        amount_b_raw,
        range_usdc,
        in_range,
        fees_owed_a_ui,
        fees_owed_b_ui,
        token_a_label,
        token_b_label,
    })
}

/// Build a mint->price map for all pools referenced by the given positions.
pub async fn fetch_prices_for_positions(
    provider: Arc<RpcProvider>,
    positions: &[MonitoredPosition],
) -> BTreeMap<String, f64> {
    let pool_reader = WhirlpoolReader::new(provider.clone());

    let mut mints: BTreeSet<String> = BTreeSet::new();
    for p in positions {
        if let Ok(pool_state) = pool_reader.get_pool_state(&p.pool.to_string()).await {
            mints.insert(pool_state.token_mint_a.to_string());
            mints.insert(pool_state.token_mint_b.to_string());
        }
    }

    fetch_mint_prices_usd(&mints).await.0
}

#[cfg(test)]
mod tick_range_usdc_tests {
    use super::compute_tick_range_usdc;
    use super::wsol_feed_usd_looks_bogus;
    use super::wsol_usd_from_usdc_pair_tick;
    use rust_decimal::Decimal;
    use solana_sdk::pubkey::Pubkey;
    use std::str::FromStr;

    #[test]
    fn sol_usdc_deep_ticks_produce_nonzero_usdc_range() {
        let sol = Pubkey::from_str("So11111111111111111111111111111111111111112").unwrap();
        let usdc = Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap();
        let r = compute_tick_range_usdc(-25276, -25172, &sol, &usdc, 9, 6).expect("range");
        assert!(r.lower > Decimal::ZERO);
        assert!(r.upper > r.lower);
    }

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
}
