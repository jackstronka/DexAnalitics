//! Rebalancing execution logic.

use crate::lifecycle::{LifecycleTracker, RebalanceData, RebalanceReason};
use crate::transaction::TransactionManager;
use crate::wallet::Wallet;
use anyhow::Context;
use clmm_lp_protocols::prelude::*;
use rust_decimal::Decimal;
use rust_decimal::MathematicalOps;
use rust_decimal::prelude::ToPrimitive;
use solana_sdk::pubkey;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Signature;
use solana_sdk::signature::Signer;
use spl_token::solana_program::program_pack::Pack;
use spl_token::state::Account as SplTokenAccount;
use spl_token::state::Mint as SplMint;
use std::collections::{BTreeSet, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::LazyLock;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Optional hook after successful on-chain steps; argument is chain-history anchor pubkey (base58).
pub type ChainHistoryMaterializeHook = Arc<dyn Fn(&str) + Send + Sync>;

/// Shallow-merge optional operator fields (e.g. `open_origin`) into open `details` for lifecycle.
fn merge_open_ledger_details(
    base: serde_json::Value,
    extra: Option<serde_json::Value>,
) -> serde_json::Value {
    let Some(extra) = extra else {
        return base;
    };
    let serde_json::Value::Object(mut bm) = base else {
        return extra;
    };
    if let serde_json::Value::Object(em) = extra {
        for (k, v) in em {
            bm.insert(k, v);
        }
        serde_json::Value::Object(bm)
    } else {
        serde_json::Value::Object(bm)
    }
}

/// Pubkey strings for optional chain-history materialize hook after successful on-chain steps.
fn chain_history_hook_anchors_from_success(
    op_name: &str,
    position: Option<Pubkey>,
    result: &clmm_lp_protocols::orca::executor::ExecutionResult,
) -> Vec<String> {
    let mut out: BTreeSet<String> = BTreeSet::new();
    match op_name {
        "open_position" | "open_full_range_position" => {
            if let Some(p) = result.created_position {
                out.insert(p.to_string());
            }
        }
        "close_position" | "collect_fees" | "decrease_liquidity" | "swap_exact_in" => {
            if let Some(p) = position {
                out.insert(p.to_string());
            }
        }
        _ => {}
    }
    out.into_iter().collect()
}

/// Retries for `open_position` after a successful close (`CLMM_REBALANCE_OPEN_MAX_ATTEMPTS`, 1..=20, default 5).
fn rebalance_open_max_attempts() -> u32 {
    std::env::var("CLMM_REBALANCE_OPEN_MAX_ATTEMPTS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n| (1..=20).contains(&n))
        .unwrap_or(5)
}

/// In-pool swap rounds to align wallet with [`quote_deposit_budget_in_range`] before open.
fn swap_mix_max_rounds() -> u32 {
    std::env::var("CLMM_REBALANCE_SWAP_MAX_ROUNDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n| (1..=20).contains(&n))
        .unwrap_or(10)
}

/// Swap-mix convergence tolerance (in USD notional). Prevents infinite dust loops due to fees/rounding.
fn swap_mix_deficit_usd_epsilon() -> f64 {
    std::env::var("CLMM_SWAP_MIX_DEFICIT_USD_EPS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|v: &f64| v.is_finite() && *v >= 0.0 && *v <= 5.0)
        .unwrap_or(0.25)
}

/// Adaptive swap-mix epsilon when env override is not set.
///
/// Rationale: using a fixed USD epsilon can cause dust loops (too strict) or unnecessary swaps (too loose)
/// across very different position sizes. We scale gently with `target_usd` and keep safe caps.
fn swap_mix_deficit_usd_epsilon_for_target(target_usd: f64) -> f64 {
    // If operator set an explicit epsilon, always respect it.
    if std::env::var("CLMM_SWAP_MIX_DEFICIT_USD_EPS")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .is_some()
    {
        return swap_mix_deficit_usd_epsilon();
    }
    if !target_usd.is_finite() || target_usd <= 0.0 {
        return swap_mix_deficit_usd_epsilon();
    }
    // ~0.3% of target + small floor, capped.
    let eps = 0.03_f64 + 0.003_f64 * target_usd;
    eps.clamp(0.05, 0.50)
}

/// Max fraction of the surplus leg to spend in **one** `swap_exact_in` during swap-mix.
///
/// Previously hardcoded at 0.92, which often left several % of the leg unused vs the deposit quote,
/// forcing a **second** swap (extra `meta.fee`). Default leaves ~1.2% headroom for pool fee + rounding.
fn swap_mix_spend_cap_pct() -> f64 {
    std::env::var("CLMM_SWAP_MIX_SPEND_CAP_PCT")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|v: &f64| v.is_finite() && *v > 0.5 && *v <= 1.0)
        .unwrap_or(0.988)
}

/// Multiplier on USD-derived `amount_in` (covers slippage / small price move between quote and tx).
fn swap_mix_amount_in_buffer_pct() -> f64 {
    std::env::var("CLMM_SWAP_MIX_AMOUNT_IN_BUFFER_PCT")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|v: &f64| v.is_finite() && *v >= 1.0 && *v <= 1.2)
        .unwrap_or(1.03)
}

fn ui_from_raw(raw: u64, decimals: u8) -> f64 {
    (raw as f64) / 10f64.powi(i32::from(decimals))
}

/// Native SOL held back for fees/rent before wrap or in-pool swap during swap-mix.
const SWAP_MIX_NATIVE_SOL_RESERVE_LAMPORTS: u64 = 10_000_000;

#[must_use]
fn swap_mix_native_spendable_lamports(native_lamports: u64) -> u64 {
    native_lamports.saturating_sub(SWAP_MIX_NATIVE_SOL_RESERVE_LAMPORTS)
}

/// SPL balances plus spendable **native** SOL counted on whichever pool leg is WSOL (SOL-first notional).
struct SwapMixWalletInputs<'a> {
    token_mint_a: &'a Pubkey,
    token_mint_b: &'a Pubkey,
    wsol_mint_pk: &'a Pubkey,
    balance_a_raw: u64,
    balance_b_raw: u64,
    decimals_a: u8,
    decimals_b: u8,
    spendable_lamports: u64,
}

fn swap_mix_wallet_ui_sol_first(inputs: &SwapMixWalletInputs<'_>) -> (f64, f64) {
    let mut a_ui = ui_from_raw(inputs.balance_a_raw, inputs.decimals_a);
    let mut b_ui = ui_from_raw(inputs.balance_b_raw, inputs.decimals_b);
    let spendable_ui = inputs.spendable_lamports as f64 / 1e9;
    if inputs.token_mint_a == inputs.wsol_mint_pk {
        a_ui = a_ui.max(spendable_ui);
    }
    if inputs.token_mint_b == inputs.wsol_mint_pk {
        b_ui = b_ui.max(spendable_ui);
    }
    (a_ui, b_ui)
}

/// When opening on a WSOL pool leg, treat spendable native SOL as available on that leg.
///
/// This matches upstream Whirlpool bot behavior (native SOL vs WSOL ATA) and our swap-mix sizing.
fn apply_session_caps_to_wallet_raw(
    balance_a_raw: u64,
    balance_b_raw: u64,
    spendable_lamports: u64,
    token_mint_a: &Pubkey,
    token_mint_b: &Pubkey,
    wsol_mint_pk: &Pubkey,
    session: Option<&super::session_capital::SessionMintCaps>,
) -> (u64, u64, u64) {
    use super::session_capital::cap_rpc_with_session;
    let wa = cap_rpc_with_session(balance_a_raw, token_mint_a, session);
    let wb = cap_rpc_with_session(balance_b_raw, token_mint_b, session);
    let spend = if token_mint_a == wsol_mint_pk {
        cap_rpc_with_session(spendable_lamports, token_mint_a, session)
    } else if token_mint_b == wsol_mint_pk {
        cap_rpc_with_session(spendable_lamports, token_mint_b, session)
    } else {
        spendable_lamports
    };
    (wa, wb, spend)
}

fn open_wallet_notional_and_caps_sol_first(
    inputs: &SwapMixWalletInputs<'_>,
    price_a_usd: f64,
    price_b_usd: f64,
) -> (f64, u64, u64) {
    let (a_ui, b_ui) = swap_mix_wallet_ui_sol_first(inputs);
    let wallet_notional_usd = a_ui * price_a_usd + b_ui * price_b_usd;

    let mut cap_a = inputs.balance_a_raw;
    let mut cap_b = inputs.balance_b_raw;
    if inputs.token_mint_a == inputs.wsol_mint_pk {
        cap_a = cap_a.max(inputs.spendable_lamports);
    }
    if inputs.token_mint_b == inputs.wsol_mint_pk {
        cap_b = cap_b.max(inputs.spendable_lamports);
    }

    (wallet_notional_usd, cap_a, cap_b)
}

fn prev_end_value_usd_from_close_amounts(
    amount_a_raw: u64,
    amount_b_raw: u64,
    decimals_a: u8,
    decimals_b: u8,
    price_a_usd: f64,
    price_b_usd: f64,
) -> f64 {
    let a_ui = ui_from_raw(amount_a_raw, decimals_a);
    let b_ui = ui_from_raw(amount_b_raw, decimals_b);
    a_ui * price_a_usd + b_ui * price_b_usd
}

fn target_usd_from_prev_end_clamped(prev_end_value_usd: f64, wallet_notional_usd: f64) -> f64 {
    // Keep a small margin for rounding / dust; matches legacy wallet-notional logic.
    let wallet_cap = (wallet_notional_usd * 0.995).max(0.0);
    if !(prev_end_value_usd.is_finite() && prev_end_value_usd > 0.0) {
        return wallet_cap;
    }
    prev_end_value_usd.min(wallet_cap)
}

/// Post-close reopen `target_usd`: ledger-derived `prev_end_value_usd` minus dust margin **only**.
///
/// Spec §2.2 / §2.5 / A4: do **not** clamp to wallet notional here — a low `wallet_notional` may be
/// stale RPC or cross-session contention; silent downsizing is avoided. When swap-mix cannot
/// reach the quote, the executor surfaces an error → pending-open recovery.
///
/// When `prev_end_value_usd` is unknown/zero (e.g. missing lifecycle row), callers should fall back
/// to [`target_usd_from_prev_end_clamped`] with `prev_end_value_usd = 0` (wallet-cap sizing).
fn target_usd_for_reopen_sizing(prev_end_value_usd: f64) -> f64 {
    if !(prev_end_value_usd.is_finite() && prev_end_value_usd > 0.0) {
        return 0.0;
    }
    (prev_end_value_usd * 0.995).max(0.0)
}

fn target_usd_for_swap_mix_and_open(prev_end_value_usd: f64, wallet_notional_usd: f64) -> f64 {
    if prev_end_value_usd.is_finite() && prev_end_value_usd > 0.0 {
        target_usd_for_reopen_sizing(prev_end_value_usd)
    } else {
        target_usd_from_prev_end_clamped(0.0, wallet_notional_usd)
    }
}

/// USD budget for [`no_close_unless_reopen_feasible`] deposit quote **before** closing.
///
/// SPL balances are read **pre-close**; liquidity is still in the position. Spendable value after a
/// successful close is approximately `wallet_notional + prev_end_value_usd` at the same synthetic
/// prices. Do not use [`target_usd_from_prev_end_clamped`] here: it clamps `prev_end` to the
/// (empty) wallet and yields `target_usd = 0`, blocking every close+reopen.
fn target_usd_for_close_reopen_preflight(
    prev_end_value_usd: f64,
    wallet_notional_before_close: f64,
) -> f64 {
    let wallet_cap = (wallet_notional_before_close * 0.995).max(0.0);
    if !(prev_end_value_usd.is_finite() && prev_end_value_usd > 0.0) {
        return wallet_cap;
    }
    let post_close_spendable_usd = wallet_notional_before_close + prev_end_value_usd;
    let spendable_cap = (post_close_spendable_usd * 0.995).max(0.0);
    prev_end_value_usd.min(spendable_cap)
}

/// Guardrail: when enabled, do not close a position unless a reopen is feasible (preflight quote).
fn no_close_unless_reopen_feasible() -> bool {
    std::env::var("CLMM_NO_CLOSE_UNLESS_REOPEN_FEASIBLE")
        .ok()
        .map(|s| s.trim().to_ascii_lowercase())
        .as_deref()
        .map(|v| v == "1" || v == "true" || v == "yes" || v == "on")
        .unwrap_or(true)
}

/// Per-process guardrail for "at most one open per rebalance_session_id".
///
/// - `inflight`: currently executing open path for a session id.
/// - `completed`: an open already succeeded for that session id in this process lifetime.
static OPEN_GUARD_INFLIGHT: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));
static OPEN_GUARD_COMPLETED: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

fn try_reserve_open_session(session_id: &str) -> bool {
    let sid = session_id.trim();
    if sid.is_empty() {
        return true;
    }
    {
        let done = OPEN_GUARD_COMPLETED.lock().unwrap();
        if done.contains(sid) {
            return false;
        }
    }
    let mut inflight = OPEN_GUARD_INFLIGHT.lock().unwrap();
    if inflight.contains(sid) {
        return false;
    }
    inflight.insert(sid.to_string());
    true
}

fn release_open_session_reservation(session_id: &str, mark_completed: bool) {
    let sid = session_id.trim();
    if sid.is_empty() {
        return;
    }
    {
        let mut inflight = OPEN_GUARD_INFLIGHT.lock().unwrap();
        inflight.remove(sid);
    }
    if mark_completed {
        let mut done = OPEN_GUARD_COMPLETED.lock().unwrap();
        done.insert(sid.to_string());
    }
}

/// Swap-mix/open: widen tick range when reopen quote is too small.
fn reopen_auto_widen_enabled() -> bool {
    std::env::var("CLMM_REOPEN_AUTO_WIDEN_TICKS")
        .ok()
        .map(|s| s.trim().to_ascii_lowercase())
        .as_deref()
        .map(|v| v == "1" || v == "true" || v == "yes" || v == "on")
        .unwrap_or(true)
}

/// Max widen steps (each step expands width ~x2 around current tick).
fn reopen_auto_widen_max_steps() -> u32 {
    std::env::var("CLMM_REOPEN_AUTO_WIDEN_MAX_STEPS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n| (0..=10).contains(&n))
        .unwrap_or(4)
}

/// §2.2: balance reads when comparing wallet notional `W` vs reopen target before swap-mix.
fn reopen_wallet_refresh_attempts() -> u32 {
    std::env::var("CLMM_REOPEN_WALLET_REFRESH_ATTEMPTS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n| (1..=20).contains(&n))
        .unwrap_or(3)
}

fn reopen_wallet_refresh_gap_ms() -> u64 {
    std::env::var("CLMM_REOPEN_WALLET_REFRESH_GAP_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n| n <= 10_000)
        .unwrap_or(350)
}

/// `ε_wallet` for `W >= T * (1 - ε)` (spec §2.2); default **0.5%**.
fn reopen_wallet_notional_epsilon() -> f64 {
    std::env::var("CLMM_REOPEN_WALLET_NOTIONAL_EPSILON")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|e| (0.0..0.5).contains(e))
        .unwrap_or(0.005)
}

/// Whether to block rebalance when [`RebalanceExecutor::is_profitable`] is false.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RebalanceProfitabilityMode {
    /// Do not use profitability estimate (same as pre-bot-onboarding behavior).
    #[default]
    Off,
    /// Log a warning but execute.
    Warn,
    /// Skip rebalance and return [`RebalanceResult::error`].
    Block,
}

fn rebalance_profitability_mode_from_env() -> RebalanceProfitabilityMode {
    match std::env::var("CLMM_REBALANCE_PROFITABILITY")
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Ok("warn") => RebalanceProfitabilityMode::Warn,
        Ok("block") => RebalanceProfitabilityMode::Block,
        _ => RebalanceProfitabilityMode::Off,
    }
}

fn rebalance_est_tx_cost_lamports() -> u64 {
    std::env::var("CLMM_REBALANCE_EST_TX_COST_LAMPORTS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(500_000)
}

async fn spl_mint_decimals(provider: &RpcProvider, mint: &Pubkey) -> anyhow::Result<u8> {
    let acc = provider.get_account(mint).await?;
    let m = SplMint::unpack(&acc.data).context("unpack SPL mint")?;
    Ok(m.decimals)
}

/// Synthetic USD prices used for swap-mix sizing + `target_usd`.
///
/// Important: `WhirlpoolState.price` is not guaranteed to be "B per A" across all upstreams; we've
/// observed cases where it behaves as "A per B" (inverse). For pools involving a stablecoin leg we
/// use a stable-aware heuristic so `wallet_notional` and `target_usd` don't get inflated.
fn synthetic_prices_for_deposit_quote(
    pool_price: Decimal,
    mint_a: &Pubkey,
    mint_b: &Pubkey,
    dec_a: u8,
    dec_b: u8,
) -> (f64, f64, &'static str) {
    let exp = i32::from(dec_a) - i32::from(dec_b);
    let b_per_a_ui = pool_price * Decimal::from(10).powi(i64::from(exp));
    let mut b_per_a = b_per_a_ui.to_f64().unwrap_or(1.0).max(1e-18);

    const MIN_POSITIVE: f64 = 1e-18;
    const INVERT_IF_LT: f64 = 0.2; // e.g. 0.08 USDC/SOL is almost certainly inverted (SOL/USDC)

    // Stablecoin mint IDs (mainnet + common devnet Nebula proxy).
    const USDC: Pubkey = pubkey!("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");
    const USDT: Pubkey = pubkey!("Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB");
    const DEVNET_DEV_USDC: Pubkey = pubkey!("BRjpCHtyQLNCo8gqRUr8jtdAj5AjPYQaoqbvcZiHok1k");
    let is_stable = |m: &Pubkey| *m == USDC || *m == USDT || *m == DEVNET_DEV_USDC;

    // If B is stablecoin: interpret price as stable_per_A (USD per token A). If it's tiny, assume inverse.
    if is_stable(mint_b) {
        if b_per_a < INVERT_IF_LT {
            b_per_a = (1.0 / b_per_a).max(MIN_POSITIVE);
            return (b_per_a, 1.0, "stable_b_inverted");
        }
        return (b_per_a, 1.0, "stable_b");
    }
    // If A is stablecoin: b_per_a is tokenB per 1 stable. USD per tokenB is 1 / b_per_a.
    if is_stable(mint_a) {
        // We have observed upstreams where `pool_price` effectively behaves as "A per B" (inverse),
        // which would make `b_per_a` huge and thus `1 / b_per_a` ~ 0.
        // When the computed USD per tokenB looks unrealistically tiny, assume inverse convention.
        let mut usd_per_b = (1.0 / b_per_a).max(MIN_POSITIVE);
        if usd_per_b < INVERT_IF_LT {
            usd_per_b = b_per_a.max(MIN_POSITIVE);
            return (1.0, usd_per_b, "stable_a_inverted");
        }
        return (1.0, usd_per_b, "stable_a");
    }

    // Fallback: relative scale only (A=1).
    (1.0_f64, b_per_a, "relative")
}

#[cfg(test)]
mod synthetic_price_tests {
    use super::*;
    use rust_decimal::Decimal;

    #[test]
    fn stable_b_inverts_tiny_price() {
        // Simulate a case where upstream already returns UI-scaled tiny stable_per_A
        // (e.g. due to inverted convention / missing decimal scaling in the upstream value).
        let mint_a = Pubkey::new_unique();
        let mint_b = pubkey!("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"); // USDC
        let (pa, pb, mode) = synthetic_prices_for_deposit_quote(
            Decimal::from_f64_retain(0.00008).unwrap(), // after UI scaling -> 0.08 (< INVERT_IF_LT) => invert
            &mint_a,
            &mint_b,
            9,
            6,
        );
        assert_eq!(pb, 1.0);
        assert!(pa > 1.0);
        assert_eq!(mode, "stable_b_inverted");
    }

    #[test]
    fn stable_a_uses_inverse_for_b() {
        // Simulate USDC/SOL where price is SOL per USDC (~0.0125) and A is stable.
        let mint_a = pubkey!("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"); // USDC
        let mint_b = Pubkey::new_unique();
        let (_pa, pb, mode) = synthetic_prices_for_deposit_quote(
            Decimal::from_f64_retain(0.0125).unwrap(),
            &mint_a,
            &mint_b,
            6,
            9,
        );
        assert!(pb > 1.0);
        assert_eq!(mode, "stable_a");
    }

    #[test]
    fn stable_a_inverts_huge_b_per_a() {
        // Simulate USDC/SOL where upstream returns USDC per SOL (~80) even though A is stable.
        // Without inversion this would yield USD per SOL ~= 0.0125 (nonsense).
        let mint_a = pubkey!("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"); // USDC
        let mint_b = Pubkey::new_unique();
        let (_pa, pb, mode) = synthetic_prices_for_deposit_quote(
            // Note: function applies UI scaling by 10^(dec_a - dec_b) = 10^-3 for 6 vs 9,
            // so we pass 80_000 to yield b_per_a_ui ~= 80 after scaling.
            Decimal::from_f64_retain(80_000.0).unwrap(),
            &mint_a,
            &mint_b,
            6,
            9,
        );
        assert!(pb > 1.0);
        assert_eq!(mode, "stable_a_inverted");
    }
}

#[cfg(test)]
mod swap_mix_sol_first_tests {
    use super::*;

    #[test]
    fn wallet_ui_counts_native_when_wsol_is_token_b() {
        let wsol: Pubkey = clmm_lp_protocols::orca::executor::WSOL_MINT
            .parse()
            .expect("WSOL mint");
        let usdc = pubkey!("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");
        let (a_ui, b_ui) = swap_mix_wallet_ui_sol_first(&SwapMixWalletInputs {
            token_mint_a: &usdc,
            token_mint_b: &wsol,
            wsol_mint_pk: &wsol,
            balance_a_raw: 0,
            balance_b_raw: 0,
            decimals_a: 6,
            decimals_b: 9,
            spendable_lamports: 500_000_000,
        });
        assert!((a_ui - 0.0).abs() < 1e-12);
        assert!((b_ui - 0.5).abs() < 1e-12);
    }

    #[test]
    fn wallet_ui_counts_native_when_wsol_is_token_a() {
        let wsol: Pubkey = clmm_lp_protocols::orca::executor::WSOL_MINT
            .parse()
            .expect("WSOL mint");
        let usdc = pubkey!("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");
        let (a_ui, b_ui) = swap_mix_wallet_ui_sol_first(&SwapMixWalletInputs {
            token_mint_a: &wsol,
            token_mint_b: &usdc,
            wsol_mint_pk: &wsol,
            balance_a_raw: 0,
            balance_b_raw: 0,
            decimals_a: 9,
            decimals_b: 6,
            spendable_lamports: 400_000_000,
        });
        assert!((a_ui - 0.4).abs() < 1e-12);
        assert!((b_ui - 0.0).abs() < 1e-12);
    }
}

fn balances_cover_deposit_quote(wa: u64, wb: u64, q: &DepositBudgetQuote) -> bool {
    let tol_a = (q.amount_a / 100).max(1);
    let tol_b = (q.amount_b / 100).max(1);
    wa >= q.amount_a.saturating_sub(tol_a) && wb >= q.amount_b.saturating_sub(tol_b)
}

fn final_caps_cover_deposit_quote(cap_a: u64, cap_b: u64, q: &DepositBudgetQuote) -> bool {
    balances_cover_deposit_quote(cap_a, cap_b, q)
}

fn widen_ticks_around_current(
    tick_current: i32,
    tick_spacing: u16,
    tick_lower: i32,
    tick_upper: i32,
    step: u32,
) -> (i32, i32) {
    let spacing = tick_spacing as i32;
    let width = (tick_upper - tick_lower).abs().max(spacing.max(1));
    // Expand width by ~2^step, clamped to avoid overflow.
    let shift = step.min(10);
    let factor = 1i32.checked_shl(shift).unwrap_or(1024);
    let new_width = width.saturating_mul(factor).max(spacing.max(1));
    let half = new_width / 2;
    let mut lo = tick_current.saturating_sub(half);
    let mut hi = tick_current.saturating_add(half);
    if spacing > 0 {
        lo = (lo / spacing) * spacing;
        hi = (hi / spacing) * spacing;
    }
    if hi <= lo {
        hi = lo.saturating_add(spacing.max(1));
    }
    (lo, hi)
}

fn adapt_recover_open_ticks_if_needed(
    tick_current: i32,
    tick_spacing: u16,
    tick_lower: i32,
    tick_upper: i32,
) -> ((i32, i32), bool) {
    // Happy path: intended range still contains current spot tick.
    if tick_current >= tick_lower && tick_current < tick_upper {
        return ((tick_lower, tick_upper), false);
    }
    if !reopen_auto_widen_enabled() {
        return ((tick_lower, tick_upper), false);
    }
    for step in 1..=reopen_auto_widen_max_steps() {
        let (lo, hi) =
            widen_ticks_around_current(tick_current, tick_spacing, tick_lower, tick_upper, step);
        if tick_current >= lo && tick_current < hi {
            return ((lo, hi), true);
        }
    }
    ((tick_lower, tick_upper), false)
}

fn recover_plan_ttl_secs() -> i64 {
    std::env::var("CLMM_RECOVER_PLAN_TTL_SECS")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .filter(|v| (30..=3600).contains(v))
        .unwrap_or(180)
}

fn recover_plan_drift_threshold_pct() -> Decimal {
    std::env::var("CLMM_RECOVER_PLAN_MAX_DRIFT_PCT")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .and_then(Decimal::from_f64_retain)
        .filter(|v| *v > Decimal::ZERO && *v <= Decimal::new(100, 0))
        .map(|v| v / Decimal::new(100, 0))
        .unwrap_or(Decimal::new(1, 2))
}

fn recenter_ticks_keep_width(
    tick_current: i32,
    tick_spacing: u16,
    tick_lower: i32,
    tick_upper: i32,
) -> (i32, i32) {
    let spacing = i32::from(tick_spacing).max(1);
    let width = (tick_upper - tick_lower).abs().max(spacing);
    let half = width / 2;
    let mut lo = tick_current.saturating_sub(half);
    let mut hi = lo.saturating_add(width);
    lo = (lo / spacing) * spacing;
    hi = (hi / spacing) * spacing;
    if hi <= lo {
        hi = lo.saturating_add(spacing);
    }
    (lo, hi)
}

/// SPL Associated Token Account (classic SPL token program), same derivation as `spl_associated_token_account`.
fn associated_token_address(owner: &Pubkey, mint: &Pubkey) -> Pubkey {
    spl_associated_token_address::get_associated_token_address(owner, mint, &spl_token::id())
}

mod spl_associated_token_address {
    use solana_sdk::pubkey;
    use solana_sdk::pubkey::Pubkey;

    /// `spl_associated_token_account` program id (mainnet/devnet/testnet).
    const ASSOCIATED_TOKEN_PROGRAM_ID: Pubkey =
        pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

    pub fn get_associated_token_address(
        wallet_address: &Pubkey,
        token_mint_address: &Pubkey,
        token_program_id: &Pubkey,
    ) -> Pubkey {
        Pubkey::find_program_address(
            &[
                wallet_address.as_ref(),
                token_program_id.as_ref(),
                token_mint_address.as_ref(),
            ],
            &ASSOCIATED_TOKEN_PROGRAM_ID,
        )
        .0
    }
}

/// SPL token balance (raw amount) for `owner`'s ATA for `mint`. Returns 0 if ATA missing / unpack fails.
async fn spl_token_balance_raw(provider: &RpcProvider, owner: &Pubkey, mint: &Pubkey) -> u64 {
    let ata = associated_token_address(owner, mint);
    match provider.get_account(&ata).await {
        Ok(acc) => SplTokenAccount::unpack(&acc.data)
            .map(|t| t.amount)
            .unwrap_or(0),
        Err(_) => 0,
    }
}

/// Configuration for rebalancing.
#[derive(Debug, Clone)]
pub struct RebalanceConfig {
    /// Maximum slippage tolerance in basis points.
    pub max_slippage_bps: u16,
    /// Minimum profit multiplier for rebalance to be worthwhile.
    pub min_profit_multiplier: Decimal,
    /// Whether to collect fees before rebalancing.
    pub collect_fees_first: bool,
    /// Priority fee level.
    pub priority_level: crate::transaction::PriorityLevel,
    /// Heuristic profitability gate (`CLMM_REBALANCE_PROFITABILITY`).
    pub profitability_mode: RebalanceProfitabilityMode,
    /// Estimated total tx cost in lamports for profitability compare (`CLMM_REBALANCE_EST_TX_COST_LAMPORTS`).
    pub est_tx_cost_lamports: u64,
}

impl Default for RebalanceConfig {
    fn default() -> Self {
        Self {
            max_slippage_bps: 50,                      // 0.5%
            min_profit_multiplier: Decimal::new(2, 0), // 2x tx cost
            collect_fees_first: true,
            priority_level: crate::transaction::PriorityLevel::Medium,
            profitability_mode: RebalanceProfitabilityMode::Off,
            est_tx_cost_lamports: rebalance_est_tx_cost_lamports(),
        }
    }
}

impl RebalanceConfig {
    /// Default merged with `CLMM_REBALANCE_*` env overrides where applicable.
    #[must_use]
    pub fn from_env() -> Self {
        let max_slippage_bps = std::env::var("CLMM_REBALANCE_MAX_SLIPPAGE_BPS")
            .ok()
            .and_then(|s| s.trim().parse::<u16>().ok())
            .filter(|v| (1..=10_000).contains(v))
            .unwrap_or(Self::default().max_slippage_bps);
        Self {
            max_slippage_bps,
            profitability_mode: rebalance_profitability_mode_from_env(),
            est_tx_cost_lamports: rebalance_est_tx_cost_lamports(),
            ..Self::default()
        }
    }
}

/// Parameters for a rebalance operation.
#[derive(Debug, Clone)]
pub struct RebalanceParams {
    /// Position to rebalance.
    pub position: Pubkey,
    /// Pool address.
    pub pool: Pubkey,
    /// Current tick lower.
    pub current_tick_lower: i32,
    /// Current tick upper.
    pub current_tick_upper: i32,
    /// New tick lower.
    pub new_tick_lower: i32,
    /// New tick upper.
    pub new_tick_upper: i32,
    /// Current liquidity.
    pub current_liquidity: u128,
    /// Current pool tick at the time of decision (for IL reconstruction).
    pub pool_tick_current: i32,
    /// Current pool sqrt_price (Q64.64) at the time of decision (for IL reconstruction).
    pub pool_sqrt_price: u128,
    /// Reason for rebalancing.
    pub reason: RebalanceReason,
    /// Current IL percentage.
    pub current_il_pct: Decimal,
    /// IL ledger: token balances before (raw units), if known.
    pub amount_a_before: Option<u64>,
    pub amount_b_before: Option<u64>,
    /// **Token B per token A** before rebalance.
    pub price_ab_before: Option<Decimal>,
    /// After rebalance (filled when known).
    pub amount_a_after: Option<u64>,
    pub amount_b_after: Option<u64>,
    pub price_ab_after: Option<Decimal>,
    pub optimization_run_id: Option<String>,
}

/// Resume an open after `rebalance_incomplete` (funds in wallet; same pool/ticks as intended).
#[derive(Debug, Clone)]
pub struct RecoverOpenParams {
    pub pool: Pubkey,
    pub new_tick_lower: i32,
    pub new_tick_upper: i32,
    pub planned_at_utc: Option<String>,
    pub planned_price_ab: Option<Decimal>,
    pub reason: RebalanceReason,
    pub closed_position_nft: Pubkey,
    pub rebalance_session_id: Option<String>,
    pub optimization_run_id: Option<String>,
}

/// Result of a rebalance operation.
#[derive(Debug, Clone)]
pub struct RebalanceResult {
    /// Whether rebalance was successful.
    pub success: bool,
    /// `true` after the old position was closed on-chain. If [`Self::success`] is false and this is true,
    /// the old NFT no longer exists (open failed or another partial step failed after close).
    pub old_position_closed_on_chain: bool,
    /// Old position address.
    pub old_position: Pubkey,
    /// New position address (if created).
    pub new_position: Option<Pubkey>,
    /// Fees collected.
    pub fees_collected: Option<(u64, u64)>,
    /// Liquidity removed from old position.
    pub liquidity_removed: u128,
    /// Liquidity added to new position.
    pub liquidity_added: u128,
    /// Transaction cost in lamports.
    pub tx_cost_lamports: u64,
    /// Error message if failed.
    pub error: Option<String>,
    /// Session id used to tag collect/close/swap/open lifecycle rows for this rebalance attempt.
    pub rebalance_session_id: Option<String>,
}

/// Executor for rebalancing operations.
pub struct RebalanceExecutor {
    /// RPC provider.
    #[allow(dead_code)]
    provider: Arc<RpcProvider>,
    /// Transaction manager.
    tx_manager: Arc<TransactionManager>,
    /// Wallet for signing.
    wallet: Mutex<Option<Arc<Wallet>>>,
    /// Lifecycle tracker.
    lifecycle: Arc<LifecycleTracker>,
    /// Configuration.
    config: RebalanceConfig,
    /// Dry run mode.
    dry_run: AtomicBool,
    /// Optional hook after successful on-chain steps (e.g. API chain-history materialize).
    chain_history_hook: Mutex<Option<ChainHistoryMaterializeHook>>,
    /// Optional Postgres for SESSION cap resolution (API bot path).
    session_db: std::sync::Mutex<Option<std::sync::Arc<clmm_lp_data::repositories::Database>>>,
}

impl RebalanceExecutor {
    /// Creates a new rebalance executor.
    pub fn new(
        provider: Arc<RpcProvider>,
        tx_manager: Arc<TransactionManager>,
        lifecycle: Arc<LifecycleTracker>,
        config: RebalanceConfig,
    ) -> Self {
        Self {
            provider,
            tx_manager,
            wallet: Mutex::new(None),
            lifecycle,
            config,
            dry_run: AtomicBool::new(false),
            chain_history_hook: Mutex::new(None),
            session_db: std::sync::Mutex::new(None),
        }
    }

    /// Attach Postgres for `SESSION:{id}` cap reads (`CLMM_REOPEN_USE_SESSION_CAPITAL=1`).
    pub fn set_session_database(&self, db: std::sync::Arc<clmm_lp_data::repositories::Database>) {
        if let Ok(mut g) = self.session_db.lock() {
            *g = Some(db);
        }
    }

    /// Access RPC provider (for best-effort diagnostics).
    #[must_use]
    pub fn provider(&self) -> &Arc<RpcProvider> {
        &self.provider
    }

    #[inline]
    fn is_dry_run(&self) -> bool {
        self.dry_run.load(Ordering::SeqCst)
    }

    /// Sets the wallet for signing.
    pub fn set_wallet(&self, wallet: Arc<Wallet>) {
        if let Ok(mut g) = self.wallet.lock() {
            *g = Some(wallet);
        }
    }

    /// Signing wallet pubkey when configured.
    #[must_use]
    pub fn wallet_pubkey(&self) -> Option<Pubkey> {
        self.wallet
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|w| w.pubkey()))
    }

    /// Enables or disables dry run mode.
    pub fn set_dry_run(&self, dry_run: bool) {
        self.dry_run.store(dry_run, Ordering::SeqCst);
    }

    /// Optional hook invoked after successful on-chain operations (see [`chain_history_hook_anchors_from_success`]).
    pub fn set_chain_history_hook(&self, hook: Option<ChainHistoryMaterializeHook>) {
        if let Ok(mut g) = self.chain_history_hook.lock() {
            *g = hook;
        }
    }

    fn invoke_chain_history_hook(
        &self,
        op_name: &str,
        position: Option<Pubkey>,
        result: &clmm_lp_protocols::orca::executor::ExecutionResult,
    ) {
        let hook = match self.chain_history_hook.lock() {
            Ok(g) => g.clone(),
            Err(_) => return,
        };
        let Some(h) = hook else {
            return;
        };
        for a in chain_history_hook_anchors_from_success(op_name, position, result) {
            let t = a.trim();
            if !t.is_empty() {
                h(t);
            }
        }
    }

    fn require_wallet(&self) -> anyhow::Result<Arc<Wallet>> {
        self.wallet
            .lock()
            .map_err(|_| anyhow::anyhow!("wallet mutex poisoned"))?
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Wallet not set on RebalanceExecutor"))
    }

    async fn session_caps_for_reopen(
        &self,
        ledger_session_id: Option<&str>,
    ) -> Option<super::session_capital::SessionMintCaps> {
        let sid = ledger_session_id.map(str::trim).filter(|s| !s.is_empty())?;
        let owner = self.wallet_pubkey().map(|p| p.to_string());
        let db = self
            .session_db
            .lock()
            .ok()
            .and_then(|g| g.clone());
        super::session_capital::load_session_mint_caps(db.as_deref(), sid, owner.as_deref()).await
    }

    fn session_capital_error_if_strict(session: &super::session_capital::SessionMintCaps) -> Option<String> {
        if !super::session_capital::reopen_use_session_capital() {
            return None;
        }
        if !super::session_capital::reopen_session_strict_empty() {
            return None;
        }
        if session.is_empty() {
            Some(format!(
                "session_capital_unknown: no SESSION inventory for {} (source={})",
                session.session_id,
                super::session_capital::session_caps_source_label(session.source)
            ))
        } else {
            None
        }
    }

    /// Checks if a rebalance is profitable.
    pub async fn is_profitable(&self, params: &RebalanceParams) -> ProfitabilityCheck {
        // Estimate transaction costs
        let estimated_tx_cost = self.estimate_tx_cost().await;

        // Estimate expected benefit from rebalancing
        let expected_benefit = self.estimate_benefit(params).await;

        let is_profitable =
            expected_benefit > Decimal::from(estimated_tx_cost) * self.config.min_profit_multiplier;

        ProfitabilityCheck {
            is_profitable,
            estimated_tx_cost,
            expected_benefit,
            min_required_benefit: Decimal::from(estimated_tx_cost)
                * self.config.min_profit_multiplier,
        }
    }

    /// Collect fees (Orca) — for emergency exit and tooling.
    pub async fn emergency_collect_fees(
        &self,
        position: &Pubkey,
        pool: &Pubkey,
    ) -> anyhow::Result<(u64, u64)> {
        self.collect_fees(position, pool, None).await
    }

    /// Remove all liquidity, then usable for close.
    pub async fn emergency_decrease_all_liquidity(
        &self,
        position: &Pubkey,
        pool: &Pubkey,
    ) -> anyhow::Result<u128> {
        let reader = PositionReader::new(self.provider.clone());
        let pos = reader
            .get_position(&position.to_string())
            .await
            .context("get_position for decrease_all")?;
        let liq = pos.liquidity;
        if liq == 0 {
            return Ok(0);
        }
        self.decrease_liquidity(position, pool, liq).await?;
        Ok(liq)
    }

    /// Close Whirlpool position NFT.
    pub async fn emergency_close_position(
        &self,
        position: &Pubkey,
        pool: &Pubkey,
    ) -> anyhow::Result<()> {
        // Keep emergency close consistent with the normal close policy:
        // always collect fees immediately before closing.
        self.execute_full_close_only(position, pool, None, None)
            .await
    }

    /// Estimates transaction cost for rebalancing.
    async fn estimate_tx_cost(&self) -> u64 {
        self.config.est_tx_cost_lamports
    }

    /// Estimates expected benefit from rebalancing.
    async fn estimate_benefit(&self, params: &RebalanceParams) -> Decimal {
        // Simplified estimation based on IL recovery
        // In a real implementation, this would use historical data and simulations
        let il_recovery = params.current_il_pct.abs() * Decimal::new(5, 1); // Assume 50% IL recovery
        il_recovery * Decimal::from(1000) // Convert to USD equivalent
    }

    /// §2.2: After close, compare wallet notional `W` vs full returned USD `T` from `returned_*_raw`;
    /// re-read SPL/native balances up to [`reopen_wallet_refresh_attempts`] before swap-mix.
    ///
    /// Skipped when returned notional is unknown/zero (wallet-cap sizing path). Fails with
    /// `wallet_below_target_after_refresh` when still `W < T * (1 - ε)` after all reads.
    async fn wallet_notional_refresh_until_reopen_target_met(
        &self,
        pool: &Pubkey,
        owner: &Pubkey,
        returned_a_raw: u64,
        returned_b_raw: u64,
        log_position: &Pubkey,
        ledger_session_id: Option<String>,
    ) -> Result<(), String> {
        let session_caps = self
            .session_caps_for_reopen(ledger_session_id.as_deref())
            .await;
        if let Some(ref sc) = session_caps
            && let Some(err) = Self::session_capital_error_if_strict(sc)
        {
            return Err(err);
        }

        let wsol_mint_pk: Pubkey = clmm_lp_protocols::orca::executor::WSOL_MINT
            .parse()
            .expect("WSOL mint");
        let attempts = reopen_wallet_refresh_attempts();
        let gap = Duration::from_millis(reopen_wallet_refresh_gap_ms());
        let eps = reopen_wallet_notional_epsilon();
        let pool_reader = WhirlpoolReader::new(self.provider.clone());
        let pool_s = pool.to_string();
        let mut last_prev_end = 0.0_f64;
        let mut last_wallet = 0.0_f64;
        let mut last_threshold = 0.0_f64;
        let session_mode = session_caps.is_some();

        for attempt in 0..attempts {
            let pool_live = pool_reader
                .get_pool_state(&pool_s)
                .await
                .map_err(|e| format!("reopen wallet vs target: get_pool_state: {e}"))?;
            let dec_a = spl_mint_decimals(self.provider.as_ref(), &pool_live.token_mint_a)
                .await
                .unwrap_or(0);
            let dec_b = spl_mint_decimals(self.provider.as_ref(), &pool_live.token_mint_b)
                .await
                .unwrap_or(0);
            let wa =
                spl_token_balance_raw(self.provider.as_ref(), owner, &pool_live.token_mint_a).await;
            let wb =
                spl_token_balance_raw(self.provider.as_ref(), owner, &pool_live.token_mint_b).await;
            let native_lamports = self.provider.get_balance(owner).await.unwrap_or(0);
            let native_spendable = swap_mix_native_spendable_lamports(native_lamports);
            let (wa, wb, native_spendable) = apply_session_caps_to_wallet_raw(
                wa,
                wb,
                native_spendable,
                &pool_live.token_mint_a,
                &pool_live.token_mint_b,
                &wsol_mint_pk,
                session_caps.as_ref(),
            );
            let (pa, pb, _) = synthetic_prices_for_deposit_quote(
                pool_live.price,
                &pool_live.token_mint_a,
                &pool_live.token_mint_b,
                dec_a,
                dec_b,
            );
            let wallet_inputs = SwapMixWalletInputs {
                token_mint_a: &pool_live.token_mint_a,
                token_mint_b: &pool_live.token_mint_b,
                wsol_mint_pk: &wsol_mint_pk,
                balance_a_raw: wa,
                balance_b_raw: wb,
                decimals_a: dec_a,
                decimals_b: dec_b,
                spendable_lamports: native_spendable,
            };
            let (wallet_notional, _, _) =
                open_wallet_notional_and_caps_sol_first(&wallet_inputs, pa, pb);
            let prev_end_usd = prev_end_value_usd_from_close_amounts(
                returned_a_raw,
                returned_b_raw,
                dec_a,
                dec_b,
                pa,
                pb,
            );
            if !(prev_end_usd.is_finite() && prev_end_usd > 0.0) {
                return Ok(());
            }
            let threshold = prev_end_usd * (1.0 - eps);
            last_prev_end = prev_end_usd;
            last_wallet = wallet_notional;
            last_threshold = threshold;
            if wallet_notional >= threshold {
                if attempt > 0 {
                    info!(
                        op = "orca_rebalance",
                        stage = "reopen_wallet_refresh",
                        attempt = attempt + 1,
                        max_attempts = attempts,
                        wallet_notional,
                        prev_end_usd,
                        threshold,
                        session_mode,
                        position = %log_position,
                        "Wallet notional met reopen target after refresh"
                    );
                }
                return Ok(());
            }
            let diag_event = if session_mode {
                "bot_reopen_session_below_target"
            } else {
                "bot_reopen_wallet_below_target"
            };
            warn!(
                op = "orca_rebalance",
                stage = "reopen_wallet_refresh",
                attempt = attempt + 1,
                max_attempts = attempts,
                wallet_notional,
                prev_end_usd,
                threshold,
                session_mode,
                position = %log_position,
                "Wallet/session notional below reopen target (may be stale read or contention)"
            );
            clmm_lp_protocols::ledger::tx_lifecycle::try_append_bot_diagnostic_row(
                self.provider.as_ref(),
                diag_event,
                "reopen_wallet_check",
                Some(*pool),
                Some(*log_position),
                ledger_session_id.clone(),
                serde_json::json!({
                    "attempt": attempt + 1,
                    "max_attempts": attempts,
                    "wallet_notional": wallet_notional,
                    "prev_end_value_usd": prev_end_usd,
                    "threshold_usd": threshold,
                    "epsilon": eps,
                }),
            )
            .await;
            if attempt + 1 < attempts {
                tokio::time::sleep(gap).await;
            }
        }

        let err_prefix = if session_mode {
            "session_below_target_after_refresh"
        } else {
            "wallet_below_target_after_refresh"
        };
        Err(format!(
            "{err_prefix}: notional={last_wallet:.8} still below threshold={last_threshold:.8} (prev_end_usd={last_prev_end:.8} after {attempts} reads, spec §2.2)"
        ))
    }

    /// In-pool Orca swaps (ExactIn) until wallet balances match [`quote_deposit_budget_in_range`]
    /// for the new tick range — same building block as API `quote-open-budget` + swap-before-open.
    ///
    /// Uses **synthetic** relative prices from `pool.price` and mint decimals (no paid price API).
    /// Returns the number of swap transactions submitted.
    #[allow(clippy::too_many_arguments)]
    async fn ensure_swap_mix_for_rebalance_open(
        &self,
        pool: &Pubkey,
        tick_lower: i32,
        tick_upper: i32,
        owner: &Pubkey,
        amount_a_before_raw: u64,
        amount_b_before_raw: u64,
        log_position: &Pubkey,
        ledger_session_id: Option<String>,
        session_caps: Option<super::session_capital::SessionMintCaps>,
    ) -> anyhow::Result<u32> {
        if self.is_dry_run() {
            return Ok(0);
        }
        let max_rounds = swap_mix_max_rounds();
        let mut swaps: u32 = 0;
        let mut last_round_details: Option<serde_json::Value> = None;
        let mut prev_deficit_usd: Option<f64> = None;
        const MIN_SWAP: u64 = 1;

        info!(
            op = "orca_rebalance",
            stage = "swap_mix",
            pool = %pool,
            tick_lower,
            tick_upper,
            owner = %owner,
            max_rounds,
            "swap-mix: align wallet to deposit quote before open"
        );

        for round in 0..max_rounds {
            let mut buffer_pct = swap_mix_amount_in_buffer_pct();
            let spend_cap = {
                let base = swap_mix_spend_cap_pct();
                if round > 0 {
                    // After the first swap, push harder so we rarely need a third tx fee.
                    base.clamp(0.998, 0.9995)
                } else {
                    base
                }
            };

            let pool_reader = WhirlpoolReader::new(self.provider.clone());
            let pool_state = pool_reader
                .get_pool_state(&pool.to_string())
                .await
                .map_err(|e| {
                    error!(
                        op = "orca_rebalance",
                        stage = "swap_mix",
                        pool = %pool,
                        round,
                        error = %e,
                        "swap-mix: get_pool_state failed"
                    );
                    e
                })?;
            if !pool_state.is_tick_in_range(tick_lower, tick_upper) {
                error!(
                    op = "orca_rebalance",
                    stage = "swap_mix",
                    pool = %pool,
                    tick_current = pool_state.tick_current,
                    tick_lower,
                    tick_upper,
                    round,
                    "swap-mix: spot tick outside new range — cannot quote deposit"
                );
                anyhow::bail!(
                    "pool tick {} not in new range [{}, {}): cannot quote deposit for open",
                    pool_state.tick_current,
                    tick_lower,
                    tick_upper
                );
            }
            let dec_a = spl_mint_decimals(self.provider.as_ref(), &pool_state.token_mint_a).await?;
            let dec_b = spl_mint_decimals(self.provider.as_ref(), &pool_state.token_mint_b).await?;
            let wa = spl_token_balance_raw(self.provider.as_ref(), owner, &pool_state.token_mint_a)
                .await;
            let wb = spl_token_balance_raw(self.provider.as_ref(), owner, &pool_state.token_mint_b)
                .await;
            let native_lamports = self.provider.get_balance(owner).await.unwrap_or(0);
            let wsol_mint_pk: Pubkey = clmm_lp_protocols::orca::executor::WSOL_MINT
                .parse()
                .expect("WSOL mint");
            let native_spendable = swap_mix_native_spendable_lamports(native_lamports);
            let (wa, wb, native_spendable) = apply_session_caps_to_wallet_raw(
                wa,
                wb,
                native_spendable,
                &pool_state.token_mint_a,
                &pool_state.token_mint_b,
                &wsol_mint_pk,
                session_caps.as_ref(),
            );
            let pool_has_wsol =
                pool_state.token_mint_a == wsol_mint_pk || pool_state.token_mint_b == wsol_mint_pk;
            if wa == 0 && wb == 0 {
                if !(pool_has_wsol && native_spendable > MIN_SWAP) {
                    error!(
                        op = "orca_rebalance",
                        stage = "swap_mix",
                        pool = %pool,
                        round,
                        native_lamports,
                        native_spendable,
                        "swap-mix: both SPL legs zero and no spendable native SOL for WSOL pool"
                    );
                    anyhow::bail!(
                        "wallet has zero SPL for both pool legs and insufficient native SOL (or pool has no WSOL leg); cannot swap-mix"
                    );
                }
                info!(
                    op = "orca_rebalance",
                    stage = "swap_mix",
                    pool = %pool,
                    round,
                    native_spendable,
                    "swap-mix: both SPL balances zero — continuing on native SOL (SOL-first, WSOL leg)"
                );
            }
            let (pa, pb, price_mode) = synthetic_prices_for_deposit_quote(
                pool_state.price,
                &pool_state.token_mint_a,
                &pool_state.token_mint_b,
                dec_a,
                dec_b,
            );
            let wallet_inputs = SwapMixWalletInputs {
                token_mint_a: &pool_state.token_mint_a,
                token_mint_b: &pool_state.token_mint_b,
                wsol_mint_pk: &wsol_mint_pk,
                balance_a_raw: wa,
                balance_b_raw: wb,
                decimals_a: dec_a,
                decimals_b: dec_b,
                spendable_lamports: native_spendable,
            };
            let (a_ui, b_ui) = swap_mix_wallet_ui_sol_first(&wallet_inputs);
            let wallet_notional = a_ui * pa + b_ui * pb;
            if !wallet_notional.is_finite() || wallet_notional <= 0.0 {
                error!(
                    op = "orca_rebalance",
                    stage = "swap_mix",
                    pool = %pool,
                    round,
                    wa,
                    wb,
                    wallet_notional,
                    price = %pool_state.price,
                    "swap-mix: wallet notional invalid"
                );
                anyhow::bail!("wallet notional invalid after close");
            }
            let prev_end_value_usd = prev_end_value_usd_from_close_amounts(
                amount_a_before_raw,
                amount_b_before_raw,
                dec_a,
                dec_b,
                pa,
                pb,
            );
            let target_usd = target_usd_for_swap_mix_and_open(prev_end_value_usd, wallet_notional);
            let q = quote_deposit_budget_in_range(
                tick_lower,
                tick_upper,
                pool_state.tick_current,
                pool_state.sqrt_price,
                dec_a,
                dec_b,
                pa,
                pb,
                target_usd,
            )
            .map_err(|m| {
                error!(
                    op = "orca_rebalance",
                    stage = "swap_mix",
                    pool = %pool,
                    round,
                    tick_lower,
                    tick_upper,
                    tick_current = pool_state.tick_current,
                    target_usd,
                    prev_end_value_usd,
                    wallet_notional,
                    price_mode,
                    quote_err = %m,
                    "swap-mix: quote_deposit_budget_in_range failed"
                );
                anyhow::anyhow!("deposit quote: {m}")
            })?;

            if balances_cover_deposit_quote(wa, wb, &q) {
                if swaps > 0 {
                    info!(round, swaps, "deposit mix OK after in-pool swaps");
                }
                return Ok(swaps);
            }

            let deficit_a = q.amount_a.saturating_sub(wa);
            let deficit_b = q.amount_b.saturating_sub(wb);

            // Treat tiny post-quote deficits as converged (dust / fees / rounding).
            let deficit_a_ui = deficit_a as f64 / 10f64.powi(i32::from(dec_a));
            let deficit_b_ui = deficit_b as f64 / 10f64.powi(i32::from(dec_b));
            let deficit_usd = (deficit_a_ui * pa + deficit_b_ui * pb).max(0.0);
            let eps_usd = swap_mix_deficit_usd_epsilon_for_target(target_usd);
            if deficit_usd.is_finite() && deficit_usd <= eps_usd {
                info!(
                    op = "orca_rebalance",
                    stage = "swap_mix",
                    pool = %pool,
                    round,
                    swaps_done = swaps,
                    deficit_usd,
                    eps_usd,
                    "swap-mix: converged by deficit epsilon"
                );
                return Ok(swaps);
            }

            let stagnation_push = if round > 0
                && swaps == 1
                && deficit_usd.is_finite()
                && prev_deficit_usd.is_some_and(|p| p.is_finite() && p > 0.0)
            {
                let p = prev_deficit_usd.unwrap_or(deficit_usd);
                deficit_usd >= p * 0.95
            } else {
                false
            };
            if stagnation_push {
                buffer_pct = buffer_pct.clamp(1.08, 1.15);
            }

            // Prefer at most 1-2 swap transactions in swap-mix to reduce drift + fees.
            // Wrapping native SOL into WSOL ATA does not increment `swaps`.
            if swaps >= 2 {
                error!(
                    op = "orca_rebalance",
                    stage = "swap_mix",
                    pool = %pool,
                    round,
                    swaps_done = swaps,
                    deficit_usd,
                    eps_usd,
                    "swap-mix: remaining deficit after 2 swaps; refusing further swaps in this attempt"
                );
                break;
            }

            let can_wrap_native_sol_for_wsol_leg_a = pool_state.token_mint_a == wsol_mint_pk
                && deficit_a > 0
                && native_lamports > SWAP_MIX_NATIVE_SOL_RESERVE_LAMPORTS.saturating_add(MIN_SWAP);

            let can_wrap_native_sol_for_wsol_leg_b = pool_state.token_mint_b == wsol_mint_pk
                && deficit_a > 0
                && native_lamports > SWAP_MIX_NATIVE_SOL_RESERVE_LAMPORTS.saturating_add(MIN_SWAP);

            if deficit_a > 0 && (wb > MIN_SWAP || can_wrap_native_sol_for_wsol_leg_b) {
                // Swap B -> A to cover deficit in A (estimate using synthetic USD prices).
                // Special case: if token A is WSOL and we have spendable **native** SOL but no WSOL SPL,
                // prefer wrapping native SOL into the WSOL ATA instead of swapping the other leg.
                // This prevents wasteful USDC->WSOL swaps when the wallet already holds SOL.
                if pool_state.token_mint_a == wsol_mint_pk
                    && wa <= MIN_SWAP
                    && can_wrap_native_sol_for_wsol_leg_a
                {
                    let deficit_a_ui = deficit_a as f64 / 10f64.powi(i32::from(dec_a));
                    let usd_need = (deficit_a_ui * pa).max(0.0);
                    let mut fund_a_ui = if pa > 0.0 {
                        // If A is WSOL, this reduces to ~deficit_a_ui * buffer_pct.
                        (usd_need / pa) * buffer_pct
                    } else {
                        0.0
                    };
                    if !fund_a_ui.is_finite() || fund_a_ui <= 0.0 {
                        fund_a_ui = (swap_mix_native_spendable_lamports(native_lamports) as f64
                            / 1e9)
                            * 0.25;
                    }
                    let raw_est_wrap = (fund_a_ui * 10f64.powi(i32::from(dec_a))).round() as u64;
                    let wrap_amt =
                        raw_est_wrap.min(swap_mix_native_spendable_lamports(native_lamports));
                    if wrap_amt >= MIN_SWAP {
                        let wallet = self.require_wallet()?;
                        let orca = WhirlpoolExecutor::new(self.provider.clone());
                        info!(
                            op = "orca_rebalance",
                            stage = "swap_mix",
                            pool = %pool,
                            round,
                            wrap_amt,
                            native_lamports,
                            wa_spl = wa,
                            "swap-mix: pre-wrap native SOL into wSOL ATA (token A is wSOL; SPL wa was 0)"
                        );
                        orca.submit_wsol_wrap_if_needed(wrap_amt, wallet.keypair())
                            .await
                            .map_err(|e| anyhow::anyhow!("swap-mix wsol pre-wrap (leg A): {e}"))?;
                        continue;
                    }
                }
                if pool_state.token_mint_b == wsol_mint_pk
                    && wb <= MIN_SWAP
                    && can_wrap_native_sol_for_wsol_leg_b
                {
                    let deficit_a_ui = deficit_a as f64 / 10f64.powi(i32::from(dec_a));
                    let usd_need = (deficit_a_ui * pa).max(0.0);
                    let mut fund_b_ui = if pb > 0.0 {
                        (usd_need / pb) * buffer_pct
                    } else {
                        0.0
                    };
                    if !fund_b_ui.is_finite() || fund_b_ui <= 0.0 {
                        fund_b_ui = (swap_mix_native_spendable_lamports(native_lamports) as f64
                            / 1e9)
                            * 0.25;
                    }
                    let raw_est_wrap = (fund_b_ui * 10f64.powi(i32::from(dec_b))).round() as u64;
                    let wrap_amt =
                        raw_est_wrap.min(swap_mix_native_spendable_lamports(native_lamports));
                    if wrap_amt >= MIN_SWAP {
                        let wallet = self.require_wallet()?;
                        let orca = WhirlpoolExecutor::new(self.provider.clone());
                        info!(
                            op = "orca_rebalance",
                            stage = "swap_mix",
                            pool = %pool,
                            round,
                            wrap_amt,
                            native_lamports,
                            wb_spl = wb,
                            "swap-mix: pre-wrap native SOL into wSOL ATA (SPL wb was 0; leg B is WSOL)"
                        );
                        orca.submit_wsol_wrap_if_needed(wrap_amt, wallet.keypair())
                            .await
                            .map_err(|e| anyhow::anyhow!("swap-mix wsol pre-wrap (leg B): {e}"))?;
                        continue;
                    }
                }
                let deficit_a_ui = deficit_a as f64 / 10f64.powi(i32::from(dec_a));
                let usd_need = (deficit_a_ui * pa).max(0.0);
                let mut fund_b_ui = if pb > 0.0 {
                    (usd_need / pb) * buffer_pct
                } else {
                    0.0
                };
                if !fund_b_ui.is_finite() || fund_b_ui <= 0.0 {
                    fund_b_ui = (wb as f64 / 10f64.powi(i32::from(dec_b))) * 0.5;
                }
                let raw_est = (fund_b_ui * 10f64.powi(i32::from(dec_b))).round() as i128;
                // Single-shot sizing: spend from the **surplus** leg (keep enough for the quote),
                // not from the entire balance. This avoids ping-pong when we deplete the needed leg.
                let surplus_b = wb.saturating_sub(q.amount_b);
                let max_raw = ((surplus_b as f64) * spend_cap).floor() as i128;
                let max_raw_u64 = max_raw.max(0) as u64;
                let amount_in = raw_est
                    .clamp(i128::from(MIN_SWAP), max_raw.max(i128::from(MIN_SWAP)))
                    .min(i128::from(wb)) as u64;
                let amount_in_ui = (amount_in as f64) / 10f64.powi(i32::from(dec_b));
                let amount_in_usd_est = amount_in_ui * pb;
                let need_a_ui = q.amount_a as f64 / 10f64.powi(i32::from(dec_a));
                let need_b_ui = q.amount_b as f64 / 10f64.powi(i32::from(dec_b));
                let deficit_a_ui = deficit_a as f64 / 10f64.powi(i32::from(dec_a));
                let deficit_b_ui = deficit_b as f64 / 10f64.powi(i32::from(dec_b));
                last_round_details = Some(serde_json::json!({
                    "round": round,
                    "leg": "B_to_A",
                    "wa": wa,
                    "wb": wb,
                    "wa_ui": a_ui,
                    "wb_ui": b_ui,
                    "need_a": q.amount_a,
                    "need_b": q.amount_b,
                    "need_a_ui": need_a_ui,
                    "need_b_ui": need_b_ui,
                    "deficit_a": deficit_a,
                    "deficit_b": deficit_b,
                    "deficit_a_ui": deficit_a_ui,
                    "deficit_b_ui": deficit_b_ui,
                    "wallet_notional": wallet_notional,
                    "target_usd": target_usd,
                    "specified_mint": pool_state.token_mint_b.to_string(),
                    "amount_in": amount_in,
                    "amount_in_raw_est": raw_est,
                    "max_spend_raw": max_raw_u64,
                    "surplus_b": surplus_b,
                    "amount_in_ui": amount_in_ui,
                    "amount_in_usd_est": amount_in_usd_est,
                    "slippage_bps": self.config.max_slippage_bps,
                    "stagnation_push": stagnation_push,
                }));
                clmm_lp_protocols::ledger::tx_lifecycle::try_append_bot_diagnostic_row(
                    self.provider.as_ref(),
                    "bot_swap_mix_round",
                    "swap_mix",
                    Some(*pool),
                    Some(*log_position),
                    ledger_session_id.clone(),
                    serde_json::json!({
                        "round": round,
                        "max_rounds": max_rounds,
                        "leg": "B_to_A",
                        "amount_in": amount_in,
                        "amount_in_raw_est": raw_est,
                        "max_spend_raw": max_raw_u64,
                        "surplus_b": surplus_b,
                        "amount_in_ui": amount_in_ui,
                        "amount_in_usd_est": amount_in_usd_est,
                        "wa": wa,
                        "wb": wb,
                        "wa_ui": a_ui,
                        "wb_ui": b_ui,
                        "need_a": q.amount_a,
                        "need_b": q.amount_b,
                        "need_a_ui": need_a_ui,
                        "need_b_ui": need_b_ui,
                        "deficit_a": deficit_a,
                        "deficit_b": deficit_b,
                        "deficit_a_ui": deficit_a_ui,
                        "deficit_b_ui": deficit_b_ui,
                        "tick_lower": tick_lower,
                        "tick_upper": tick_upper,
                        "tick_current": pool_state.tick_current,
                        "price": pool_state.price,
                        "target_usd": target_usd,
                        "wallet_notional": wallet_notional,
                        "pa": pa,
                        "pb": pb,
                        "price_mode": price_mode,
                        "amount_in_est_mode": "deficit_usd",
                        "spend_cap_pct": spend_cap,
                        "amount_in_buffer_pct": buffer_pct,
                        "stagnation_push": stagnation_push,
                    }),
                )
                .await;
                clmm_lp_protocols::ledger::tx_lifecycle::try_append_bot_diagnostic_row(
                    self.provider.as_ref(),
                    "bot_swap_exact_in_attempt",
                    "swap_exact_in",
                    Some(*pool),
                    Some(*log_position),
                    ledger_session_id.clone(),
                    serde_json::json!({
                        "round": round,
                        "leg": "B_to_A",
                        "specified_mint": pool_state.token_mint_b.to_string(),
                        "amount_in": amount_in,
                        "slippage_bps": self.config.max_slippage_bps,
                    }),
                )
                .await;
                info!(
                    round,
                    amount_in, "rebalance: swap ExactIn token B toward mix for open"
                );
                let sig = match self
                    .execute_swap_exact_in(
                        pool,
                        &pool_state.token_mint_b,
                        amount_in,
                        self.config.max_slippage_bps,
                        Some(*log_position),
                        ledger_session_id.clone(),
                    )
                    .await
                {
                    Ok(sig) => sig,
                    Err(e) => {
                        let msg = format!("{e:#}");
                        clmm_lp_protocols::ledger::tx_lifecycle::try_append_bot_diagnostic_row(
                            self.provider.as_ref(),
                            "bot_swap_exact_in_failed",
                            "swap_exact_in",
                            Some(*pool),
                            Some(*log_position),
                            ledger_session_id.clone(),
                            serde_json::json!({
                                "round": round,
                                "leg": "B_to_A",
                                "specified_mint": pool_state.token_mint_b.to_string(),
                                "amount_in": amount_in,
                                "slippage_bps": self.config.max_slippage_bps,
                                "error": msg,
                            }),
                        )
                        .await;
                        error!(
                            op = "orca_rebalance",
                            stage = "swap_mix",
                            pool = %pool,
                            round,
                            leg = "B_to_A",
                            amount_in,
                            error = %e,
                            "swap-mix: swap_exact_in (token B) failed"
                        );
                        return Err(e);
                    }
                };
                if let Some(sig) = sig {
                    clmm_lp_protocols::ledger::tx_lifecycle::try_append_bot_diagnostic_row(
                        self.provider.as_ref(),
                        "bot_swap_exact_in_submitted",
                        "swap_exact_in",
                        Some(*pool),
                        Some(*log_position),
                        ledger_session_id.clone(),
                        serde_json::json!({
                            "round": round,
                            "leg": "B_to_A",
                            "signature": sig.to_string(),
                            "specified_mint": pool_state.token_mint_b.to_string(),
                            "amount_in": amount_in,
                            "slippage_bps": self.config.max_slippage_bps,
                        }),
                    )
                    .await;
                }
                swaps += 1;
                prev_deficit_usd = Some(deficit_usd);
                continue;
            }
            if deficit_b > 0 && (wa > MIN_SWAP || can_wrap_native_sol_for_wsol_leg_a) {
                // Swap A -> B to cover deficit in B (estimate using synthetic USD prices).
                if pool_state.token_mint_a == wsol_mint_pk
                    && wa <= MIN_SWAP
                    && can_wrap_native_sol_for_wsol_leg_a
                {
                    let deficit_b_ui = deficit_b as f64 / 10f64.powi(i32::from(dec_b));
                    let usd_need = (deficit_b_ui * pb).max(0.0);
                    let mut fund_a_ui = if pa > 0.0 {
                        (usd_need / pa) * buffer_pct
                    } else {
                        0.0
                    };
                    if !fund_a_ui.is_finite() || fund_a_ui <= 0.0 {
                        fund_a_ui = (swap_mix_native_spendable_lamports(native_lamports) as f64
                            / 1e9)
                            * 0.25;
                    }
                    let raw_est_wrap = (fund_a_ui * 10f64.powi(i32::from(dec_a))).round() as u64;
                    let wrap_amt =
                        raw_est_wrap.min(swap_mix_native_spendable_lamports(native_lamports));
                    if wrap_amt >= MIN_SWAP {
                        let wallet = self.require_wallet()?;
                        let orca = WhirlpoolExecutor::new(self.provider.clone());
                        info!(
                            op = "orca_rebalance",
                            stage = "swap_mix",
                            pool = %pool,
                            round,
                            wrap_amt,
                            native_lamports,
                            wa_spl = wa,
                            "swap-mix: pre-wrap native SOL into wSOL ATA (SPL wa was 0; Orca uses wSOL SPL)"
                        );
                        orca.submit_wsol_wrap_if_needed(wrap_amt, wallet.keypair())
                            .await
                            .map_err(|e| anyhow::anyhow!("swap-mix wsol pre-wrap: {e}"))?;
                        continue;
                    }
                }
                let deficit_b_ui = deficit_b as f64 / 10f64.powi(i32::from(dec_b));
                let usd_need = (deficit_b_ui * pb).max(0.0);
                let mut fund_a_ui = if pa > 0.0 {
                    (usd_need / pa) * buffer_pct
                } else {
                    0.0
                };
                if !fund_a_ui.is_finite() || fund_a_ui <= 0.0 {
                    fund_a_ui = (wa as f64 / 10f64.powi(i32::from(dec_a))) * 0.5;
                }
                let raw_est = (fund_a_ui * 10f64.powi(i32::from(dec_a))).round() as i128;
                // Single-shot sizing: spend from the **surplus** leg (keep enough for the quote),
                // not from the entire balance.
                let surplus_a = wa.saturating_sub(q.amount_a);
                let max_raw = ((surplus_a as f64) * spend_cap).floor() as i128;
                let max_raw_u64 = max_raw.max(0) as u64;
                let amount_in = raw_est
                    .clamp(i128::from(MIN_SWAP), max_raw.max(i128::from(MIN_SWAP)))
                    .min(i128::from(wa)) as u64;
                let amount_in_ui = (amount_in as f64) / 10f64.powi(i32::from(dec_a));
                let amount_in_usd_est = amount_in_ui * pa;
                let need_a_ui = q.amount_a as f64 / 10f64.powi(i32::from(dec_a));
                let need_b_ui = q.amount_b as f64 / 10f64.powi(i32::from(dec_b));
                let deficit_a_ui = deficit_a as f64 / 10f64.powi(i32::from(dec_a));
                let deficit_b_ui = deficit_b as f64 / 10f64.powi(i32::from(dec_b));
                last_round_details = Some(serde_json::json!({
                    "round": round,
                    "leg": "A_to_B",
                    "wa": wa,
                    "wb": wb,
                    "wa_ui": a_ui,
                    "wb_ui": b_ui,
                    "need_a": q.amount_a,
                    "need_b": q.amount_b,
                    "need_a_ui": need_a_ui,
                    "need_b_ui": need_b_ui,
                    "deficit_a": deficit_a,
                    "deficit_b": deficit_b,
                    "deficit_a_ui": deficit_a_ui,
                    "deficit_b_ui": deficit_b_ui,
                    "wallet_notional": wallet_notional,
                    "target_usd": target_usd,
                    "specified_mint": pool_state.token_mint_a.to_string(),
                    "amount_in": amount_in,
                    "amount_in_raw_est": raw_est,
                    "max_spend_raw": max_raw_u64,
                    "surplus_a": surplus_a,
                    "amount_in_ui": amount_in_ui,
                    "amount_in_usd_est": amount_in_usd_est,
                    "slippage_bps": self.config.max_slippage_bps,
                    "stagnation_push": stagnation_push,
                }));
                clmm_lp_protocols::ledger::tx_lifecycle::try_append_bot_diagnostic_row(
                    self.provider.as_ref(),
                    "bot_swap_mix_round",
                    "swap_mix",
                    Some(*pool),
                    Some(*log_position),
                    ledger_session_id.clone(),
                    serde_json::json!({
                        "round": round,
                        "max_rounds": max_rounds,
                        "leg": "A_to_B",
                        "amount_in": amount_in,
                        "amount_in_raw_est": raw_est,
                        "max_spend_raw": max_raw_u64,
                        "surplus_a": surplus_a,
                        "amount_in_ui": amount_in_ui,
                        "amount_in_usd_est": amount_in_usd_est,
                        "wa": wa,
                        "wb": wb,
                        "wa_ui": a_ui,
                        "wb_ui": b_ui,
                        "need_a": q.amount_a,
                        "need_b": q.amount_b,
                        "need_a_ui": need_a_ui,
                        "need_b_ui": need_b_ui,
                        "deficit_a": deficit_a,
                        "deficit_b": deficit_b,
                        "deficit_a_ui": deficit_a_ui,
                        "deficit_b_ui": deficit_b_ui,
                        "tick_lower": tick_lower,
                        "tick_upper": tick_upper,
                        "tick_current": pool_state.tick_current,
                        "price": pool_state.price,
                        "target_usd": target_usd,
                        "wallet_notional": wallet_notional,
                        "pa": pa,
                        "pb": pb,
                        "price_mode": price_mode,
                        "amount_in_est_mode": "deficit_usd",
                        "spend_cap_pct": spend_cap,
                        "amount_in_buffer_pct": buffer_pct,
                        "stagnation_push": stagnation_push,
                    }),
                )
                .await;
                clmm_lp_protocols::ledger::tx_lifecycle::try_append_bot_diagnostic_row(
                    self.provider.as_ref(),
                    "bot_swap_exact_in_attempt",
                    "swap_exact_in",
                    Some(*pool),
                    Some(*log_position),
                    ledger_session_id.clone(),
                    serde_json::json!({
                        "round": round,
                        "leg": "A_to_B",
                        "specified_mint": pool_state.token_mint_a.to_string(),
                        "amount_in": amount_in,
                        "slippage_bps": self.config.max_slippage_bps,
                    }),
                )
                .await;
                info!(
                    round,
                    amount_in, "rebalance: swap ExactIn token A toward mix for open"
                );
                let sig = match self
                    .execute_swap_exact_in(
                        pool,
                        &pool_state.token_mint_a,
                        amount_in,
                        self.config.max_slippage_bps,
                        Some(*log_position),
                        ledger_session_id.clone(),
                    )
                    .await
                {
                    Ok(sig) => sig,
                    Err(e) => {
                        let msg = format!("{e:#}");
                        clmm_lp_protocols::ledger::tx_lifecycle::try_append_bot_diagnostic_row(
                            self.provider.as_ref(),
                            "bot_swap_exact_in_failed",
                            "swap_exact_in",
                            Some(*pool),
                            Some(*log_position),
                            ledger_session_id.clone(),
                            serde_json::json!({
                                "round": round,
                                "leg": "A_to_B",
                                "specified_mint": pool_state.token_mint_a.to_string(),
                                "amount_in": amount_in,
                                "slippage_bps": self.config.max_slippage_bps,
                                "error": msg,
                            }),
                        )
                        .await;
                        error!(
                            op = "orca_rebalance",
                            stage = "swap_mix",
                            pool = %pool,
                            round,
                            leg = "A_to_B",
                            amount_in,
                            error = %e,
                            "swap-mix: swap_exact_in (token A) failed"
                        );
                        return Err(e);
                    }
                };
                if let Some(sig) = sig {
                    clmm_lp_protocols::ledger::tx_lifecycle::try_append_bot_diagnostic_row(
                        self.provider.as_ref(),
                        "bot_swap_exact_in_submitted",
                        "swap_exact_in",
                        Some(*pool),
                        Some(*log_position),
                        ledger_session_id.clone(),
                        serde_json::json!({
                            "round": round,
                            "leg": "A_to_B",
                            "signature": sig.to_string(),
                            "specified_mint": pool_state.token_mint_a.to_string(),
                            "amount_in": amount_in,
                            "slippage_bps": self.config.max_slippage_bps,
                        }),
                    )
                    .await;
                }
                swaps += 1;
                prev_deficit_usd = Some(deficit_usd);
                continue;
            }

            error!(
                op = "orca_rebalance",
                stage = "swap_mix",
                pool = %pool,
                round,
                swaps_done = swaps,
                wa,
                wb,
                need_a = q.amount_a,
                need_b = q.amount_b,
                deficit_a,
                deficit_b,
                "swap-mix: cannot route swap (no spendable leg or both legs short vs quote)"
            );
            anyhow::bail!(
                "cannot swap toward deposit mix: wa={wa} wb={wb} need_a={} need_b={}",
                q.amount_a,
                q.amount_b
            );
        }
        error!(
            op = "orca_rebalance",
            stage = "swap_mix",
            pool = %pool,
            max_rounds,
            swaps_done = swaps,
            "swap-mix: exhausted rounds without matching deposit quote"
        );
        clmm_lp_protocols::ledger::tx_lifecycle::try_append_bot_diagnostic_row(
            self.provider.as_ref(),
            "bot_swap_mix_failed",
            "swap_mix",
            Some(*pool),
            Some(*log_position),
            ledger_session_id.clone(),
            serde_json::json!({
                "max_rounds": max_rounds,
                "swaps_done": swaps,
                "tick_lower": tick_lower,
                "tick_upper": tick_upper,
                "last_round": last_round_details
            }),
        )
        .await;
        anyhow::bail!(
            "swap mix: exhausted {} rounds without matching deposit quote",
            max_rounds
        );
    }

    /// Swap-mix alignment + `open_position` retries (shared by full rebalance and incomplete recovery).
    ///
    /// `amount_a_before_calc` / `amount_b_before_calc` are **returned** pool-leg raws (§6.1):
    /// principal from close + LP fees collected on that close tx, or the same sum read back from
    /// `bot_close_position` lifecycle rows for recovery.
    #[allow(clippy::too_many_arguments)]
    async fn open_new_range_with_wallet_mix(
        &self,
        pool: &Pubkey,
        new_tick_lower: i32,
        new_tick_upper: i32,
        prev_tick_lower: Option<i32>,
        prev_tick_upper: Option<i32>,
        pool_state: &WhirlpoolState,
        amount_a_before_calc: u64,
        amount_b_before_calc: u64,
        log_position: &Pubkey,
        ledger_session_id: Option<String>,
    ) -> Result<(Pubkey, u32), String> {
        let Some(owner) = self.wallet_pubkey() else {
            return Err(
                "wallet missing on RebalanceExecutor after close — cannot open new position"
                    .to_string(),
            );
        };

        let session_caps = self
            .session_caps_for_reopen(ledger_session_id.as_deref())
            .await;
        if let Some(ref sc) = session_caps
            && let Some(err) = Self::session_capital_error_if_strict(sc)
        {
            return Err(err);
        }

        if let Some(sid) = ledger_session_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if clmm_lp_protocols::ledger::tx_lifecycle::session_has_bot_open_position(sid) {
                clmm_lp_protocols::ledger::tx_lifecycle::try_append_bot_diagnostic_row(
                    self.provider.as_ref(),
                    "bot_open_guard_blocked",
                    "open_guard",
                    Some(*pool),
                    Some(*log_position),
                    Some(sid.to_string()),
                    serde_json::json!({
                        "reason": "session_already_has_open_row",
                        "new_tick_lower": new_tick_lower,
                        "new_tick_upper": new_tick_upper
                    }),
                )
                .await;
                return Err(format!(
                    "open guard: rebalance session {sid} already has bot_open_position row; blocking duplicate open"
                ));
            }
            if !try_reserve_open_session(sid) {
                clmm_lp_protocols::ledger::tx_lifecycle::try_append_bot_diagnostic_row(
                    self.provider.as_ref(),
                    "bot_open_guard_blocked",
                    "open_guard",
                    Some(*pool),
                    Some(*log_position),
                    Some(sid.to_string()),
                    serde_json::json!({
                        "reason": "session_open_inflight_or_completed",
                        "new_tick_lower": new_tick_lower,
                        "new_tick_upper": new_tick_upper
                    }),
                )
                .await;
                return Err(format!(
                    "open guard: rebalance session {sid} already inflight/completed in this process; blocking duplicate open"
                ));
            }
        }

        if let Err(e) = self
            .wallet_notional_refresh_until_reopen_target_met(
                pool,
                &owner,
                amount_a_before_calc,
                amount_b_before_calc,
                log_position,
                ledger_session_id.clone(),
            )
            .await
        {
            if let Some(sid) = ledger_session_id.as_deref() {
                release_open_session_reservation(sid, false);
            }
            return Err(e);
        }

        let swap_rounds = self
            .ensure_swap_mix_for_rebalance_open(
                pool,
                new_tick_lower,
                new_tick_upper,
                &owner,
                amount_a_before_calc,
                amount_b_before_calc,
                log_position,
                ledger_session_id.clone(),
                session_caps.clone(),
            )
            .await;
        let swap_rounds = match swap_rounds {
            Ok(v) => v,
            Err(e) => {
                if let Some(sid) = ledger_session_id.as_deref() {
                    release_open_session_reservation(sid, false);
                }
                return Err(e.to_string());
            }
        };

        let max_open_attempts = rebalance_open_max_attempts();
        let mut new_position: Option<Pubkey> = None;
        let mut last_open_err: Option<String> = None;
        let mut last_cap_a: u64 = 0;
        let mut last_cap_b: u64 = 0;
        let mut tried_operational_sol_topup = false;

        // Swap-mix refetches pool state each round; the `pool_state` passed in is typically the
        // post-close snapshot. Using stale tick/√P for `quote_deposit_budget_in_range` after swaps
        // can mis-size caps vs on-chain reality and prevent reopen despite successful mix.
        let pool_reader = WhirlpoolReader::new(self.provider.clone());
        let pool_addr = pool.to_string();

        for attempt in 1..=max_open_attempts {
            let pool_live = match pool_reader.get_pool_state(&pool_addr).await {
                Ok(s) => s,
                Err(e) => {
                    let msg = format!("fetch pool state for open quote (attempt {attempt}): {e}");
                    last_open_err = Some(msg);
                    warn!(
                        op = "orca_rebalance",
                        stage = "open_position",
                        attempt,
                        max_attempts = max_open_attempts,
                        error = %e,
                        "Failed to refetch pool for deposit quote; will retry if attempts remain"
                    );
                    if attempt < max_open_attempts {
                        tokio::time::sleep(Duration::from_millis(250 * u64::from(attempt))).await;
                    }
                    continue;
                }
            };

            let wa = spl_token_balance_raw(self.provider.as_ref(), &owner, &pool_live.token_mint_a)
                .await;
            let wb = spl_token_balance_raw(self.provider.as_ref(), &owner, &pool_live.token_mint_b)
                .await;
            let native_lamports = self.provider.get_balance(&owner).await.unwrap_or(0);
            let native_spendable = swap_mix_native_spendable_lamports(native_lamports);
            let wsol_mint_pk: Pubkey = clmm_lp_protocols::orca::executor::WSOL_MINT
                .parse()
                .expect("WSOL mint");
            let (wa, wb, native_spendable) = apply_session_caps_to_wallet_raw(
                wa,
                wb,
                native_spendable,
                &pool_live.token_mint_a,
                &pool_live.token_mint_b,
                &wsol_mint_pk,
                session_caps.as_ref(),
            );

            let dec_a = spl_mint_decimals(self.provider.as_ref(), &pool_live.token_mint_a)
                .await
                .unwrap_or(0);
            let dec_b = spl_mint_decimals(self.provider.as_ref(), &pool_live.token_mint_b)
                .await
                .unwrap_or(0);
            let (pa, pb, _price_mode) = synthetic_prices_for_deposit_quote(
                pool_live.price,
                &pool_live.token_mint_a,
                &pool_live.token_mint_b,
                dec_a,
                dec_b,
            );
            let wallet_inputs = SwapMixWalletInputs {
                token_mint_a: &pool_live.token_mint_a,
                token_mint_b: &pool_live.token_mint_b,
                wsol_mint_pk: &wsol_mint_pk,
                balance_a_raw: wa,
                balance_b_raw: wb,
                decimals_a: dec_a,
                decimals_b: dec_b,
                spendable_lamports: native_spendable,
            };
            let (wallet_notional, mut cap_a, mut cap_b) =
                open_wallet_notional_and_caps_sol_first(&wallet_inputs, pa, pb);
            let prev_end_value_usd = prev_end_value_usd_from_close_amounts(
                amount_a_before_calc,
                amount_b_before_calc,
                dec_a,
                dec_b,
                pa,
                pb,
            );
            let target_usd = target_usd_for_swap_mix_and_open(prev_end_value_usd, wallet_notional);

            let quote_opt = quote_deposit_budget_in_range(
                new_tick_lower,
                new_tick_upper,
                pool_live.tick_current,
                pool_live.sqrt_price,
                dec_a,
                dec_b,
                pa,
                pb,
                target_usd,
            )
            .ok();

            if let Some(q) = quote_opt.as_ref() {
                cap_a = cap_a.min(q.token_max_a);
                cap_b = cap_b.min(q.token_max_b);
            }

            if cap_a == 0 && cap_b == 0 {
                cap_a = amount_a_before_calc.max(1);
                cap_b = amount_b_before_calc.max(1);
                cap_a = super::session_capital::cap_rpc_with_session(
                    cap_a,
                    &pool_live.token_mint_a,
                    session_caps.as_ref(),
                );
                cap_b = super::session_capital::cap_rpc_with_session(
                    cap_b,
                    &pool_live.token_mint_b,
                    session_caps.as_ref(),
                );
                if attempt == 1 {
                    warn!(
                        op = "orca_rebalance",
                        stage = "open_position",
                        position = %log_position,
                        cap_a,
                        cap_b,
                        "Post-close SPL balances were 0; falling back to pre-close token amounts as open caps"
                    );
                }
            }
            last_cap_a = cap_a;
            last_cap_b = cap_b;
            if let Some(q) = quote_opt.as_ref()
                && !final_caps_cover_deposit_quote(cap_a, cap_b, q)
            {
                let err_s = format!(
                    "reopen_final_caps_below_target: final caps do not cover deposit quote \
                     (cap_a={cap_a}, cap_b={cap_b}, quote_amount_a={}, quote_amount_b={}, \
                     quote_token_max_a={}, quote_token_max_b={}, target_usd={target_usd:.8}, \
                     quote_estimated_value_usd={:.8}, wallet_notional={wallet_notional:.8}, \
                     prev_end_value_usd={prev_end_value_usd:.8})",
                    q.amount_a, q.amount_b, q.token_max_a, q.token_max_b, q.estimated_value_usd
                );
                warn!(
                    op = "orca_rebalance",
                    stage = "open_position",
                    outcome = "final_caps_below_quote",
                    attempt,
                    max_attempts = max_open_attempts,
                    position = %log_position,
                    pool = %pool,
                    cap_a,
                    cap_b,
                    quote_amount_a = q.amount_a,
                    quote_amount_b = q.amount_b,
                    quote_token_max_a = q.token_max_a,
                    quote_token_max_b = q.token_max_b,
                    target_usd,
                    quote_estimated_value_usd = q.estimated_value_usd,
                    wallet_notional,
                    prev_end_value_usd,
                    "Blocking undersized reopen before open_position"
                );
                clmm_lp_protocols::ledger::tx_lifecycle::try_append_bot_diagnostic_row(
                    self.provider.as_ref(),
                    "bot_reopen_final_caps_below_target",
                    "open_position",
                    Some(*pool),
                    Some(*log_position),
                    ledger_session_id.clone(),
                    serde_json::json!({
                        "attempt": attempt,
                        "max_attempts": max_open_attempts,
                        "cap_a": cap_a,
                        "cap_b": cap_b,
                        "quote_amount_a": q.amount_a,
                        "quote_amount_b": q.amount_b,
                        "quote_token_max_a": q.token_max_a,
                        "quote_token_max_b": q.token_max_b,
                        "target_usd": target_usd,
                        "quote_estimated_value_usd": q.estimated_value_usd,
                        "wallet_notional": wallet_notional,
                        "prev_end_value_usd": prev_end_value_usd,
                        "note": "Final pre-open guard: do not silently downsize reopen below deposit quote; retry/pending-open instead."
                    }),
                )
                .await;
                last_open_err = Some(err_s);
                if attempt < max_open_attempts {
                    tokio::time::sleep(Duration::from_millis(250 * u64::from(attempt))).await;
                }
                continue;
            }
            if attempt == 1 {
                info!(
                    position = %log_position,
                    cap_a,
                    cap_b,
                    mint_a = %pool_live.token_mint_a,
                    mint_b = %pool_live.token_mint_b,
                    max_attempts = max_open_attempts,
                    "Open position token caps (swap-mix path)"
                );
            } else {
                info!(
                    attempt,
                    max_attempts = max_open_attempts,
                    cap_a,
                    cap_b,
                    "Retry open_position (fresh SPL balances)"
                );
            }

            match self
                .open_position(
                    pool,
                    new_tick_lower,
                    new_tick_upper,
                    cap_a,
                    cap_b,
                    ledger_session_id.clone(),
                    Some({
                        let mut v = serde_json::json!({
                            "open_target_usd": target_usd,
                            "open_prev_end_value_usd": prev_end_value_usd,
                            "open_wallet_notional_usd": wallet_notional
                        });
                        if let Some(obj) = v.as_object_mut() {
                            if let Some(prev) = prev_tick_lower {
                                obj.insert("prev_tick_lower".to_string(), serde_json::json!(prev));
                            }
                            if let Some(prev) = prev_tick_upper {
                                obj.insert("prev_tick_upper".to_string(), serde_json::json!(prev));
                            }
                            obj.insert(
                                "new_tick_lower".to_string(),
                                serde_json::json!(new_tick_lower),
                            );
                            obj.insert(
                                "new_tick_upper".to_string(),
                                serde_json::json!(new_tick_upper),
                            );
                            obj.insert(
                                "open_quote_pool_tick_current".to_string(),
                                serde_json::json!(pool_live.tick_current),
                            );
                            obj.insert(
                                "open_quote_pool_sqrt_price".to_string(),
                                serde_json::json!(pool_live.sqrt_price.to_string()),
                            );
                        }
                        if let Some(q) = quote_opt.as_ref()
                            && let Some(obj) = v.as_object_mut()
                        {
                            obj.insert(
                                "open_quote_estimated_value_usd".to_string(),
                                serde_json::json!(q.estimated_value_usd),
                            );
                            obj.insert(
                                "open_quote_token_max_a".to_string(),
                                serde_json::json!(q.token_max_a),
                            );
                            obj.insert(
                                "open_quote_token_max_b".to_string(),
                                serde_json::json!(q.token_max_b),
                            );
                            obj.insert(
                                "open_quote_amount_a_raw".to_string(),
                                serde_json::json!(q.amount_a),
                            );
                            obj.insert(
                                "open_quote_amount_b_raw".to_string(),
                                serde_json::json!(q.amount_b),
                            );
                            obj.insert(
                                "open_quote_liquidity".to_string(),
                                serde_json::json!(q.liquidity.to_string()),
                            );
                        }
                        v
                    }),
                )
                .await
            {
                Ok(pos) => {
                    if attempt > 1 {
                        info!(
                            attempt,
                            position = %pos,
                            "open_position succeeded after retry"
                        );
                    }
                    new_position = Some(pos);
                    break;
                }
                Err(e) => {
                    let err_s = e.to_string();
                    last_open_err = Some(err_s.clone());
                    warn!(
                        op = "orca_rebalance",
                        stage = "open_position",
                        attempt,
                        max_attempts = max_open_attempts,
                        cap_a,
                        cap_b,
                        new_tick_lower,
                        new_tick_upper,
                        error = %err_s,
                        "open_position failed"
                    );

                    // Pending-open / reopen reliability: if open preflight fails due to insufficient
                    // **native SOL** (rent/fees), attempt a SOL-first operational top-up from wallet
                    // assets (prefer WSOL->SOL unwrap, else swap USDC->WSOL then unwrap), then retry.
                    if !tried_operational_sol_topup
                        && err_s.contains("open preflight exact-plan: insufficient native SOL")
                    {
                        tried_operational_sol_topup = true;
                        if let Some((required_with_margin, native_balance)) =
                            Self::parse_open_preflight_required_native_lamports(&err_s)
                        {
                            if let Err(topup_err) = self
                                .ensure_operational_native_sol_for_open(
                                    pool,
                                    &pool_live,
                                    required_with_margin,
                                    native_balance,
                                    log_position,
                                    ledger_session_id.clone(),
                                )
                                .await
                            {
                                warn!(
                                    op = "orca_rebalance",
                                    stage = "open_position",
                                    attempt,
                                    required_with_margin,
                                    native_balance,
                                    error = %topup_err,
                                    "open_position operational SOL top-up failed; will continue retry loop"
                                );
                            } else {
                                info!(
                                    op = "orca_rebalance",
                                    stage = "open_position",
                                    attempt,
                                    required_with_margin,
                                    native_balance,
                                    "open_position operational SOL top-up executed; retrying open"
                                );
                            }
                        }
                    }
                    if attempt < max_open_attempts {
                        tokio::time::sleep(Duration::from_millis(250 * u64::from(attempt))).await;
                    }
                }
            }
        }

        let new_position = match new_position {
            Some(p) => p,
            None => {
                if let Some(sid) = ledger_session_id.as_deref() {
                    release_open_session_reservation(sid, false);
                }
                let e = last_open_err.unwrap_or_else(|| "unknown error".to_string());
                let hint = " Close succeeded but open failed after retries — funds should be in wallet ATAs; token mix for new range may still require a swap before open.";
                error!(
                    op = "orca_rebalance",
                    stage = "open_position",
                    outcome = "failed_after_retries",
                    position = %log_position,
                    pool = %pool,
                    attempts = max_open_attempts,
                    last_cap_a,
                    last_cap_b,
                    new_tick_lower,
                    new_tick_upper,
                    mint_a = %pool_state.token_mint_a,
                    mint_b = %pool_state.token_mint_b,
                    error = %e,
                    "Failed to open new position"
                );
                return Err(format!("{e}{hint}"));
            }
        };

        if let Some(sid) = ledger_session_id.as_deref() {
            release_open_session_reservation(sid, true);
        }

        Ok((new_position, swap_rounds))
    }

    fn parse_first_u64_after(haystack: &str, marker: &str) -> Option<u64> {
        let idx = haystack.find(marker)?;
        let mut j = idx + marker.len();
        while j < haystack.len() && !haystack.as_bytes()[j].is_ascii_digit() {
            j += 1;
        }
        let start = j;
        while j < haystack.len() && haystack.as_bytes()[j].is_ascii_digit() {
            j += 1;
        }
        if start == j {
            return None;
        }
        haystack[start..j].parse::<u64>().ok()
    }

    /// Parse the "exact-plan" open preflight error for required native lamports + current balance.
    ///
    /// Example shape (from Orca executor):
    /// `... require {required_with_margin}. Current native balance {native_balance}. ...`
    fn parse_open_preflight_required_native_lamports(err: &str) -> Option<(u64, u64)> {
        let required = Self::parse_first_u64_after(err, "require ")?;
        let native = Self::parse_first_u64_after(err, "Current native balance ")?;
        Some((required, native))
    }

    fn wsol_mint_pk() -> Pubkey {
        pubkey!("So11111111111111111111111111111111111111112")
    }

    /// SPL mint to spend (exact-in) to obtain WSOL for operational SOL top-up.
    ///
    /// - If `CLMM_STABLE_MINT_FOR_SOL_TOPUP` is set to a valid pubkey, uses that mint.
    /// - Else, if the pool has a WSOL leg, uses the **other** mint (mainnet USDC, devnet devUSDC, …).
    fn stable_mint_for_operational_sol_topup(pool_live: &WhirlpoolState) -> Option<Pubkey> {
        if let Ok(s) = std::env::var("CLMM_STABLE_MINT_FOR_SOL_TOPUP") {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return Pubkey::from_str(trimmed).ok();
            }
        }
        let wsol = Self::wsol_mint_pk();
        if pool_live.token_mint_a == wsol {
            Some(pool_live.token_mint_b)
        } else if pool_live.token_mint_b == wsol {
            Some(pool_live.token_mint_a)
        } else {
            None
        }
    }

    /// Rough stable-token input (raw) to cover a native SOL deficit, for WSOL/stable pools.
    fn estimate_stable_raw_for_sol_deficit(
        pool_live: &WhirlpoolState,
        stable_dec: u8,
        sol_needed_ui: f64,
    ) -> u64 {
        let wsol = Self::wsol_mint_pk();
        let price_ab = pool_live.price.to_f64().unwrap_or(0.0);
        let scale = 10f64.powi(i32::from(stable_dec));
        // Same heuristic as legacy USDC top-up: SOL/stable pools often report `price_ab ~ 0.087`;
        // multiply by 1000 for ~stable UI per 1 SOL when stable has 6 decimals (USDC / devUSDC).
        let stable_per_sol = if (pool_live.token_mint_a == wsol || pool_live.token_mint_b == wsol)
            && stable_dec == 6
            && price_ab > 0.0
        {
            price_ab * 1000.0
        } else {
            price_ab.max(1e-12)
        };
        let stable_ui = (sol_needed_ui * stable_per_sol) * 1.05;
        (stable_ui * scale).ceil() as u64
    }

    /// Ensure the signing wallet has enough **native SOL** for an `open_position` transaction.
    ///
    /// SOL-first policy: we prefer having **native SOL** (operational fees/rent) and keeping WSOL
    /// transient. If open preflight says native is short, we:
    /// - unwrap WSOL -> native SOL (if any)
    /// - else swap a small amount of USDC -> WSOL in-pool, then unwrap to native SOL
    async fn ensure_operational_native_sol_for_open(
        &self,
        pool: &Pubkey,
        pool_live: &WhirlpoolState,
        required_with_margin: u64,
        native_balance: u64,
        log_position: &Pubkey,
        ledger_session_id: Option<String>,
    ) -> anyhow::Result<()> {
        if native_balance >= required_with_margin {
            return Ok(());
        }
        let deficit = required_with_margin.saturating_sub(native_balance);
        let wallet = self.require_wallet()?;
        let payer = wallet.keypair();
        let orca = WhirlpoolExecutor::new(self.provider.clone());

        let owner = payer.pubkey();
        let mut native_now = self
            .provider
            .get_balance(&owner)
            .await
            .unwrap_or(native_balance);
        if native_now >= required_with_margin {
            return Ok(());
        }

        // 1) Prefer WSOL -> native SOL (partial unwrap) when WSOL exists.
        let mut wsol_raw = orca.read_wsol_balance_raw(&owner).await.unwrap_or(0);
        if wsol_raw > 0 {
            let want_unwrap = deficit.min(wsol_raw).max(1);
            let sig = orca
                .submit_wsol_unwrap_with_signature(want_unwrap, payer)
                .await
                .context("unwrap WSOL for operational SOL")?;
            clmm_lp_protocols::ledger::tx_lifecycle::try_append_bot_diagnostic_row(
                self.provider.as_ref(),
                "bot_operational_sol_topup_unwrap_wsol",
                "operational_sol_topup",
                Some(*pool),
                Some(*log_position),
                ledger_session_id.clone(),
                serde_json::json!({
                    "required_with_margin": required_with_margin,
                    "native_before": native_now,
                    "unwrap_wsol_raw": want_unwrap,
                    "signature": sig.to_string()
                }),
            )
            .await;
            native_now = self
                .provider
                .get_balance(&owner)
                .await
                .unwrap_or(native_now);
            if native_now >= required_with_margin {
                return Ok(());
            }
        }

        // 2) If still short, swap stable (non-WSOL pool leg) -> WSOL in-pool, then unwrap.
        // Covers mainnet USDC as well as devnet devUSDC (or any SPL leg paired with WSOL).
        let wsol = Self::wsol_mint_pk();
        let stable_mint = Self::stable_mint_for_operational_sol_topup(pool_live).ok_or_else(|| {
            anyhow::anyhow!(
                "operational SOL topup: pool has no WSOL leg or could not resolve stable mint (mint_a={}, mint_b={}); set CLMM_STABLE_MINT_FOR_SOL_TOPUP",
                pool_live.token_mint_a,
                pool_live.token_mint_b
            )
        })?;
        if stable_mint == wsol {
            anyhow::bail!("operational SOL topup: stable mint resolved to WSOL (invalid)");
        }
        let stable_dec = spl_mint_decimals(self.provider.as_ref(), &stable_mint)
            .await
            .unwrap_or(6);
        let remaining = required_with_margin.saturating_sub(native_now);
        let sol_needed = (remaining as f64) / 1e9;
        let stable_in_raw =
            Self::estimate_stable_raw_for_sol_deficit(pool_live, stable_dec, sol_needed).max(1);

        // Swap exact-in: spend stable, receive WSOL SPL.
        let _ = self
            .execute_swap_exact_in(
                pool,
                &stable_mint,
                stable_in_raw,
                self.config.max_slippage_bps,
                Some(*log_position),
                ledger_session_id.clone(),
            )
            .await
            .context("swap stable->WSOL for operational SOL topup")?;

        // Unwrap as much WSOL as we now have, but only if it moves the needle.
        wsol_raw = orca.read_wsol_balance_raw(&owner).await.unwrap_or(0);
        if wsol_raw == 0 {
            anyhow::bail!("operational SOL topup: swap produced 0 WSOL; cannot proceed");
        }
        let sig2 = orca
            .submit_wsol_unwrap_with_signature(wsol_raw, payer)
            .await
            .context("unwrap swapped WSOL to native SOL")?;
        clmm_lp_protocols::ledger::tx_lifecycle::try_append_bot_diagnostic_row(
            self.provider.as_ref(),
            "bot_operational_sol_topup_swap_usdc_then_unwrap",
            "operational_sol_topup",
            Some(*pool),
            Some(*log_position),
            ledger_session_id.clone(),
            serde_json::json!({
                "required_with_margin": required_with_margin,
                "native_before": native_now,
                "swap_stable_in_raw": stable_in_raw,
                "stable_mint": stable_mint.to_string(),
                "unwrap_wsol_raw": wsol_raw,
                "unwrap_signature": sig2.to_string()
            }),
        )
        .await;

        Ok(())
    }

    /// Executes a rebalance operation.
    pub async fn execute(&self, params: RebalanceParams) -> RebalanceResult {
        info!(
            op = "orca_rebalance",
            stage = "start",
            position = %params.position,
            pool = %params.pool,
            old_range = format!("[{}, {}]", params.current_tick_lower, params.current_tick_upper),
            new_range = format!("[{}, {}]", params.new_tick_lower, params.new_tick_upper),
            reason = ?params.reason,
            dry_run = self.is_dry_run(),
            "Executing rebalance"
        );

        let mut result = RebalanceResult {
            success: false,
            old_position_closed_on_chain: false,
            old_position: params.position,
            new_position: None,
            fees_collected: None,
            liquidity_removed: 0,
            liquidity_added: 0,
            tx_cost_lamports: 0,
            error: None,
            rebalance_session_id: None,
        };
        let rebalance_session_id = Uuid::new_v4().to_string();
        result.rebalance_session_id = Some(rebalance_session_id.clone());

        if self.is_dry_run() {
            info!("Dry run mode - simulating rebalance");
            result.success = true;
            result.liquidity_removed = params.current_liquidity;
            result.liquidity_added = params.current_liquidity;
            return result;
        }

        if self.config.profitability_mode != RebalanceProfitabilityMode::Off {
            let check = self.is_profitable(&params).await;
            if !check.is_profitable {
                if matches!(
                    self.config.profitability_mode,
                    RebalanceProfitabilityMode::Warn
                ) {
                    warn!(
                        op = "orca_rebalance",
                        stage = "profitability",
                        expected_benefit = %check.expected_benefit,
                        min_required = %check.min_required_benefit,
                        est_tx_lamports = check.estimated_tx_cost,
                        "Rebalance not profitable by heuristic — continuing (Warn mode)"
                    );
                } else {
                    let msg = format!(
                        "rebalance blocked by profitability gate: expected_benefit={} min_required={} (est tx {} lamports); set CLMM_REBALANCE_PROFITABILITY=off or warn",
                        check.expected_benefit, check.min_required_benefit, check.estimated_tx_cost
                    );
                    error!(op = "orca_rebalance", stage = "profitability", "{}", msg);
                    result.error = Some(msg);
                    return result;
                }
            }
        }

        // IL ledger: compute token split from on-chain liquidity + current pool state.
        // This gives us a consistent way to reconstruct LP value "before" rebalance.
        let (amount_a_before_calc, amount_b_before_calc) = {
            let reader = PositionReader::new(self.provider.clone());
            let dummy_pos = OnChainPosition {
                address: params.position,
                pool: params.pool,
                owner: Pubkey::default(),
                tick_lower: params.current_tick_lower,
                tick_upper: params.current_tick_upper,
                liquidity: params.current_liquidity,
                fee_growth_inside_a: 0,
                fee_growth_inside_b: 0,
                fees_owed_a: 0,
                fees_owed_b: 0,
            };
            reader.calculate_token_amounts(
                &dummy_pos,
                params.pool_tick_current,
                params.pool_sqrt_price,
            )
        };

        let amount_a_before = params.amount_a_before.or(Some(amount_a_before_calc));
        let amount_b_before = params.amount_b_before.or(Some(amount_b_before_calc));

        // Step 1 (guardrail): ensure reopen is feasible *before* closing. Wallet SPL is pre-close;
        // budget uses estimated post-close spendable (wallet + position value) — see
        // `target_usd_for_close_reopen_preflight`.
        // If not feasible, we skip the close so we don't leave the operator with 0 positions.
        let mut planned_tick_lower = params.new_tick_lower;
        let mut planned_tick_upper = params.new_tick_upper;
        if no_close_unless_reopen_feasible() {
            let Some(owner) = self.wallet_pubkey() else {
                result.error = Some(
                    "wallet missing on RebalanceExecutor — cannot preflight reopen feasibility"
                        .to_string(),
                );
                return result;
            };
            let pool_reader = WhirlpoolReader::new(self.provider.clone());
            let pool_state = match pool_reader.get_pool_state(&params.pool.to_string()).await {
                Ok(s) => s,
                Err(e) => {
                    result.error = Some(format!("preflight: get_pool_state failed: {e}"));
                    return result;
                }
            };

            let dec_a = spl_mint_decimals(self.provider.as_ref(), &pool_state.token_mint_a)
                .await
                .unwrap_or(0);
            let dec_b = spl_mint_decimals(self.provider.as_ref(), &pool_state.token_mint_b)
                .await
                .unwrap_or(0);
            let wa =
                spl_token_balance_raw(self.provider.as_ref(), &owner, &pool_state.token_mint_a)
                    .await;
            let wb =
                spl_token_balance_raw(self.provider.as_ref(), &owner, &pool_state.token_mint_b)
                    .await;
            let (pa, pb, price_mode) = synthetic_prices_for_deposit_quote(
                pool_state.price,
                &pool_state.token_mint_a,
                &pool_state.token_mint_b,
                dec_a,
                dec_b,
            );
            let a_ui = ui_from_raw(wa, dec_a);
            let b_ui = ui_from_raw(wb, dec_b);
            let wallet_notional = a_ui * pa + b_ui * pb;
            let position_reader = PositionReader::new(self.provider.clone());
            let (preflight_returned_a, preflight_returned_b) = match position_reader
                .get_position(&params.position.to_string())
                .await
            {
                Ok(pos) => {
                    let (pa_amt, pb_amt) = position_reader.calculate_token_amounts(
                        &pos,
                        pool_state.tick_current,
                        pool_state.sqrt_price,
                    );
                    (
                        pa_amt.saturating_add(pos.fees_owed_a),
                        pb_amt.saturating_add(pos.fees_owed_b),
                    )
                }
                Err(_) => (amount_a_before_calc, amount_b_before_calc),
            };
            let prev_end_value_usd = prev_end_value_usd_from_close_amounts(
                preflight_returned_a,
                preflight_returned_b,
                dec_a,
                dec_b,
                pa,
                pb,
            );
            let target_usd =
                target_usd_for_close_reopen_preflight(prev_end_value_usd, wallet_notional);

            let mut ok = quote_deposit_budget_in_range(
                planned_tick_lower,
                planned_tick_upper,
                pool_state.tick_current,
                pool_state.sqrt_price,
                dec_a,
                dec_b,
                pa,
                pb,
                target_usd,
            )
            .is_ok();

            if !ok && reopen_auto_widen_enabled() {
                for step in 1..=reopen_auto_widen_max_steps() {
                    let (lo, hi) = widen_ticks_around_current(
                        pool_state.tick_current,
                        pool_state.tick_spacing,
                        planned_tick_lower,
                        planned_tick_upper,
                        step,
                    );
                    let try_ok = quote_deposit_budget_in_range(
                        lo,
                        hi,
                        pool_state.tick_current,
                        pool_state.sqrt_price,
                        dec_a,
                        dec_b,
                        pa,
                        pb,
                        target_usd,
                    )
                    .is_ok();
                    clmm_lp_protocols::ledger::tx_lifecycle::try_append_bot_diagnostic_row(
                        self.provider.as_ref(),
                        "bot_reopen_widen_ticks",
                        "reopen_preflight",
                        Some(params.pool),
                        Some(params.position),
                        None,
                        serde_json::json!({
                            "step": step,
                            "old_tick_lower": planned_tick_lower,
                            "old_tick_upper": planned_tick_upper,
                            "new_tick_lower": lo,
                            "new_tick_upper": hi,
                            "tick_current": pool_state.tick_current,
                            "tick_spacing": pool_state.tick_spacing,
                            "wa": wa,
                            "wb": wb,
                            "wa_ui": a_ui,
                            "wb_ui": b_ui,
                            "wallet_notional": wallet_notional,
                            "target_usd": target_usd,
                        "prev_end_value_usd": prev_end_value_usd,
                            "pa": pa,
                            "pb": pb,
                            "price_mode": price_mode,
                            "quote_ok": try_ok,
                        }),
                    )
                    .await;
                    if try_ok {
                        planned_tick_lower = lo;
                        planned_tick_upper = hi;
                        ok = true;
                        break;
                    }
                }
            }

            if !ok {
                clmm_lp_protocols::ledger::tx_lifecycle::try_append_bot_diagnostic_row(
                    self.provider.as_ref(),
                    "bot_reopen_preflight_failed",
                    "reopen_preflight",
                    Some(params.pool),
                    Some(params.position),
                    None,
                    serde_json::json!({
                        "tick_lower": planned_tick_lower,
                        "tick_upper": planned_tick_upper,
                        "tick_current": pool_state.tick_current,
                        "tick_spacing": pool_state.tick_spacing,
                        "wa": wa,
                        "wb": wb,
                        "wa_ui": a_ui,
                        "wb_ui": b_ui,
                        "wallet_notional": wallet_notional,
                        "target_usd": target_usd,
                        "prev_end_value_usd": prev_end_value_usd,
                        "pa": pa,
                        "pb": pb,
                        "price_mode": price_mode,
                        "note": "Guardrail: skip close because reopen quote failed for planned range (preflight budget = post-close spendable estimate: wallet_notional + prev_end_value_usd)."
                    }),
                )
                .await;
                result.error = Some(
                    "reopen preflight failed (quote rejected) — skipping close (no-close-unless-reopen-feasible)"
                        .to_string(),
                );
                return result;
            }
        }

        // Step 2: Close old position (includes decreasing all liquidity + collecting remaining fees)
        result.liquidity_removed = params.current_liquidity;
        let (close_amount_a_raw, close_amount_b_raw) = self
            .read_close_amounts_best_effort(&params.position, &params.pool)
            .await
            .unwrap_or((amount_a_before_calc, amount_b_before_calc));
        let close_ledger_details = with_close_amounts_in_details(
            Some(serde_json::json!({
                "close_kind":"rotation",
                "old_tick_lower": params.current_tick_lower,
                "old_tick_upper": params.current_tick_upper,
                "planned_new_tick_lower": planned_tick_lower,
                "planned_new_tick_upper": planned_tick_upper
            })),
            close_amount_a_raw,
            close_amount_b_raw,
        );
        let (lp_a_on_close, lp_b_on_close) = match self
            .close_position(
                &params.position,
                &params.pool,
                Some(rebalance_session_id.clone()),
                close_ledger_details,
            )
            .await
        {
            Ok(v) => v,
            Err(e) => {
                error!(
                    op = "orca_rebalance",
                    stage = "close_position",
                    position = %params.position,
                    pool = %params.pool,
                    reason = ?params.reason,
                    error = %e,
                    "Failed to close position"
                );
                result.error = Some(e.to_string());
                return result;
            }
        };
        result.old_position_closed_on_chain = true;
        result.tx_cost_lamports += 5000;

        let returned_a_raw = close_amount_a_raw.saturating_add(lp_a_on_close);
        let returned_b_raw = close_amount_b_raw.saturating_add(lp_b_on_close);

        // Step 3: Open new position — swap-mix + retries (see [`Self::open_new_range_with_wallet_mix`]).
        let pool_reader = WhirlpoolReader::new(self.provider.clone());
        let pool_state = match pool_reader.get_pool_state(&params.pool.to_string()).await {
            Ok(s) => s,
            Err(e) => {
                let msg = format!("fetch pool after close for open caps: {e}");
                error!(
                    op = "orca_rebalance",
                    stage = "open_position",
                    outcome = "fetch_pool_failed",
                    position = %params.position,
                    pool = %params.pool,
                    error = %e,
                    "{}", msg
                );
                result.error = Some(msg);
                return result;
            }
        };

        match self
            .open_new_range_with_wallet_mix(
                &params.pool,
                planned_tick_lower,
                planned_tick_upper,
                Some(params.current_tick_lower),
                Some(params.current_tick_upper),
                &pool_state,
                returned_a_raw,
                returned_b_raw,
                &params.position,
                Some(rebalance_session_id.clone()),
            )
            .await
        {
            Ok((new_position, swap_rounds)) => {
                result.tx_cost_lamports = result
                    .tx_cost_lamports
                    .saturating_add(5000u64.saturating_mul(u64::from(swap_rounds)));
                result.new_position = Some(new_position);
                result.tx_cost_lamports += 5000;
            }
            Err(e) => {
                if e.contains("wallet missing") {
                    error!(
                        op = "orca_rebalance",
                        stage = "open_position",
                        outcome = "no_wallet",
                        position = %params.position,
                        pool = %params.pool,
                        "{}", e
                    );
                } else if e.contains("swap") || e.contains("mix") || e.contains("deposit") {
                    error!(
                        op = "orca_rebalance",
                        stage = "swap_mix",
                        outcome = "failed",
                        position = %params.position,
                        pool = %params.pool,
                        error = %e,
                        "swap-before-open mix failed"
                    );
                }
                result.error = Some(e);
                return result;
            }
        }
        // Orca open_position() already performs the initial liquidity increase.
        result.liquidity_added = params.current_liquidity;

        let Some(new_position) = result.new_position else {
            result.error = Some("internal: new_position missing after open".to_string());
            return result;
        };

        let (fa, fb) = result.fees_collected.unwrap_or((0, 0));

        // IL ledger: compute token split "after" rebalance using the new on-chain state.
        let (amount_a_after, amount_b_after, price_ab_after) = {
            let pool_reader = WhirlpoolReader::new(self.provider.clone());
            let pool_state = pool_reader
                .get_pool_state(&params.pool.to_string())
                .await
                .ok();
            if let Some(pool_state) = pool_state {
                let pos_reader = PositionReader::new(self.provider.clone());
                if let Ok(on_chain_pos) = pos_reader.get_position(&new_position.to_string()).await {
                    let (a, b) = pos_reader.calculate_token_amounts(
                        &on_chain_pos,
                        pool_state.tick_current,
                        pool_state.sqrt_price,
                    );
                    (Some(a), Some(b), Some(pool_state.price))
                } else {
                    (None, None, None)
                }
            } else {
                (None, None, None)
            }
        };

        // Record rebalance in lifecycle
        self.lifecycle
            .record_rebalance(
                new_position,
                params.pool,
                RebalanceData {
                    old_tick_lower: params.current_tick_lower,
                    old_tick_upper: params.current_tick_upper,
                    new_tick_lower: params.new_tick_lower,
                    new_tick_upper: params.new_tick_upper,
                    old_liquidity: params.current_liquidity,
                    new_liquidity: result.liquidity_added,
                    tx_cost_lamports: result.tx_cost_lamports,
                    il_at_rebalance: params.current_il_pct,
                    reason: params.reason,
                    amount_a_before,
                    amount_b_before,
                    amount_a_after,
                    amount_b_after,
                    price_ab_before: params.price_ab_before,
                    price_ab_after,
                    fees_a_collected: Some(fa),
                    fees_b_collected: Some(fb),
                    optimization_run_id: params.optimization_run_id.clone(),
                    range_adjustment_reason: None,
                    old_position: Some(params.position.to_string()),
                },
            )
            .await;

        result.success = true;
        info!(
            old_position = %params.position,
            new_position = %new_position,
            tx_cost = result.tx_cost_lamports,
            "Rebalance completed successfully"
        );

        result
    }

    /// Complete only the **open** leg after a failed rebalance (close already on-chain).
    pub async fn recover_open_after_incomplete(&self, p: RecoverOpenParams) -> RebalanceResult {
        let mut result = RebalanceResult {
            success: false,
            old_position_closed_on_chain: false,
            old_position: p.closed_position_nft,
            new_position: None,
            fees_collected: None,
            liquidity_removed: 0,
            liquidity_added: 0,
            tx_cost_lamports: 0,
            error: None,
            rebalance_session_id: p.rebalance_session_id.clone(),
        };

        info!(
            op = "orca_rebalance",
            stage = "recover_open",
            pool = %p.pool,
            new_tick_lower = p.new_tick_lower,
            new_tick_upper = p.new_tick_upper,
            closed_nft = %p.closed_position_nft,
            "recover_open_after_incomplete"
        );

        if self.is_dry_run() {
            result.error = Some("dry run: recover_open skipped".to_string());
            return result;
        }

        let pool_reader = WhirlpoolReader::new(self.provider.clone());
        let pool_state = match pool_reader.get_pool_state(&p.pool.to_string()).await {
            Ok(s) => s,
            Err(e) => {
                result.error = Some(format!("fetch pool for recover_open: {e}"));
                return result;
            }
        };
        let mut planned_tick_lower = p.new_tick_lower;
        let mut planned_tick_upper = p.new_tick_upper;
        let mut range_adjustment_reason: Option<String> = None;

        // If the original plan is stale or drifted too far from current price, replan before recovery open.
        // For RetouchShift this avoids reopening with an outdated range after long delays.
        if p.reason == RebalanceReason::RetouchShift {
            let ttl_secs = recover_plan_ttl_secs();
            let drift_threshold = recover_plan_drift_threshold_pct();
            let now = chrono::Utc::now();

            let stale_plan = p
                .planned_at_utc
                .as_deref()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|ts| (now - ts.with_timezone(&chrono::Utc)).num_seconds() > ttl_secs)
                .unwrap_or(false);

            let drifted_plan = p.planned_price_ab.is_some_and(|planned_price| {
                if planned_price <= Decimal::ZERO || pool_state.price <= Decimal::ZERO {
                    return false;
                }
                let drift = (pool_state.price - planned_price).abs() / planned_price;
                drift > drift_threshold
            });

            if stale_plan || drifted_plan {
                let (lo, hi) = recenter_ticks_keep_width(
                    pool_state.tick_current,
                    pool_state.tick_spacing,
                    planned_tick_lower,
                    planned_tick_upper,
                );
                let reason = if stale_plan && drifted_plan {
                    "recover_plan_stale_and_price_drift_replanned"
                } else if stale_plan {
                    "recover_plan_stale_replanned"
                } else {
                    "recover_plan_price_drift_replanned"
                };
                info!(
                    op = "orca_rebalance",
                    stage = "recover_open",
                    tick_current = pool_state.tick_current,
                    old_tick_lower = planned_tick_lower,
                    old_tick_upper = planned_tick_upper,
                    new_tick_lower = lo,
                    new_tick_upper = hi,
                    reason = reason,
                    "recover_open replanned intended range before open"
                );
                clmm_lp_protocols::ledger::tx_lifecycle::try_append_bot_diagnostic_row(
                    self.provider.as_ref(),
                    "bot_recover_open_replanned",
                    "recover_open",
                    Some(p.pool),
                    Some(p.closed_position_nft),
                    p.rebalance_session_id.clone(),
                    serde_json::json!({
                        "reason": reason,
                        "old_tick_lower": planned_tick_lower,
                        "old_tick_upper": planned_tick_upper,
                        "new_tick_lower": lo,
                        "new_tick_upper": hi,
                        "tick_current": pool_state.tick_current,
                        "planned_at_utc": p.planned_at_utc,
                        "planned_price_ab": p.planned_price_ab,
                        "current_price_ab": pool_state.price
                    }),
                )
                .await;
                planned_tick_lower = lo;
                planned_tick_upper = hi;
                range_adjustment_reason = Some(reason.to_string());
            }
        }

        let ((adapted_lower, adapted_upper), adapted_ticks) = adapt_recover_open_ticks_if_needed(
            pool_state.tick_current,
            pool_state.tick_spacing,
            planned_tick_lower,
            planned_tick_upper,
        );
        planned_tick_lower = adapted_lower;
        planned_tick_upper = adapted_upper;
        if adapted_ticks {
            info!(
                op = "orca_rebalance",
                stage = "recover_open",
                tick_current = pool_state.tick_current,
                old_tick_lower = p.new_tick_lower,
                old_tick_upper = p.new_tick_upper,
                new_tick_lower = planned_tick_lower,
                new_tick_upper = planned_tick_upper,
                "recover_open adapted stale intended range to include current tick"
            );
            if range_adjustment_reason.is_none() {
                range_adjustment_reason =
                    Some("recover_open_adapted_to_include_current_tick".to_string());
            }
        }

        let (recovered_amount_a_raw, recovered_amount_b_raw) =
            match close_amounts_from_lifecycle_best_effort(
                &p.closed_position_nft,
                p.rebalance_session_id.as_deref(),
            ) {
                Some(v) => v,
                None => {
                    // Unknown close amounts must not force tiny fallback notional for reopen sizing.
                    warn!(
                        op = "orca_rebalance",
                        stage = "recover_open",
                        closed_nft = %p.closed_position_nft,
                        rebalance_session_id = ?p.rebalance_session_id,
                        "recover_open missing close amounts; using wallet-cap sizing fallback"
                    );
                    (0, 0)
                }
            };

        match self
            .open_new_range_with_wallet_mix(
                &p.pool,
                planned_tick_lower,
                planned_tick_upper,
                Some(p.new_tick_lower),
                Some(p.new_tick_upper),
                &pool_state,
                recovered_amount_a_raw,
                recovered_amount_b_raw,
                &p.closed_position_nft,
                p.rebalance_session_id.clone(),
            )
            .await
        {
            Ok((new_position, swap_rounds)) => {
                result.tx_cost_lamports = 5000u64
                    .saturating_mul(u64::from(swap_rounds))
                    .saturating_add(5000);
                result.new_position = Some(new_position);
            }
            Err(e) => {
                result.error = Some(e);
                return result;
            }
        }

        let Some(new_position) = result.new_position else {
            result.error = Some("internal: new_position missing after recover_open".to_string());
            return result;
        };

        let (amount_a_after, amount_b_after, price_ab_after) = {
            let pool_reader = WhirlpoolReader::new(self.provider.clone());
            let pool_state = pool_reader.get_pool_state(&p.pool.to_string()).await.ok();
            if let Some(pool_state) = pool_state {
                let pos_reader = PositionReader::new(self.provider.clone());
                if let Ok(on_chain_pos) = pos_reader.get_position(&new_position.to_string()).await {
                    let liq = on_chain_pos.liquidity;
                    let (a, b) = pos_reader.calculate_token_amounts(
                        &on_chain_pos,
                        pool_state.tick_current,
                        pool_state.sqrt_price,
                    );
                    result.liquidity_added = liq;
                    (Some(a), Some(b), Some(pool_state.price))
                } else {
                    (None, None, None)
                }
            } else {
                (None, None, None)
            }
        };

        self.lifecycle
            .record_rebalance(
                new_position,
                p.pool,
                RebalanceData {
                    old_tick_lower: p.new_tick_lower,
                    old_tick_upper: p.new_tick_upper,
                    new_tick_lower: planned_tick_lower,
                    new_tick_upper: planned_tick_upper,
                    old_liquidity: 0,
                    new_liquidity: result.liquidity_added,
                    tx_cost_lamports: result.tx_cost_lamports,
                    il_at_rebalance: Decimal::ZERO,
                    reason: p.reason,
                    amount_a_before: None,
                    amount_b_before: None,
                    amount_a_after,
                    amount_b_after,
                    price_ab_before: None,
                    price_ab_after,
                    fees_a_collected: None,
                    fees_b_collected: None,
                    optimization_run_id: p.optimization_run_id.clone(),
                    range_adjustment_reason,
                    old_position: Some(p.closed_position_nft.to_string()),
                },
            )
            .await;

        result.success = true;
        info!(
            new_position = %new_position,
            pool = %p.pool,
            "recover_open_after_incomplete completed"
        );

        result
    }

    /// Collect fees only (no rebalance). Used by `Decision::CollectFees` / strategy loop.
    pub async fn execute_collect_fees_only(
        &self,
        position: &Pubkey,
        pool: &Pubkey,
        ledger_session_id: Option<String>,
    ) -> anyhow::Result<()> {
        if self.is_dry_run() {
            info!("Dry run: would collect fees");
            return Ok(());
        }
        self.collect_fees(position, pool, ledger_session_id).await?;
        Ok(())
    }

    /// Full on-chain close (decrease all + collect + close NFT). Used by `Decision::Close`.
    pub async fn execute_full_close_only(
        &self,
        position: &Pubkey,
        pool: &Pubkey,
        ledger_session_id: Option<String>,
        ledger_details: Option<serde_json::Value>,
    ) -> anyhow::Result<()> {
        if self.is_dry_run() {
            info!("Dry run: would close position");
            return Ok(());
        }
        // Policy: always collect fees immediately before close.
        // This makes fee earnings explicit in the lifecycle ledger (`bot_collect_fees`) and avoids
        // mixing "principal vs fees" in the close outflow heuristics.
        self.collect_fees(position, pool, ledger_session_id.clone())
            .await?;
        let (close_amount_a_raw, close_amount_b_raw) = self
            .read_close_amounts_best_effort(position, pool)
            .await
            .unwrap_or((0, 0));
        let close_details =
            with_close_amounts_in_details(ledger_details, close_amount_a_raw, close_amount_b_raw);
        let _lp_on_close = self
            .close_position(position, pool, ledger_session_id, close_details)
            .await?;
        Ok(())
    }

    /// Bulk close: one on-chain tx (Orca close includes fee collection). Skips separate `collect_fees`.
    pub async fn execute_bulk_close_only(
        &self,
        position: &Pubkey,
        pool: &Pubkey,
        ledger_session_id: Option<String>,
        ledger_details: Option<serde_json::Value>,
    ) -> anyhow::Result<()> {
        if self.is_dry_run() {
            info!("Dry run: would bulk-close position");
            return Ok(());
        }
        let (close_amount_a_raw, close_amount_b_raw) = self
            .read_close_amounts_best_effort(position, pool)
            .await
            .unwrap_or((0, 0));
        let close_details =
            with_close_amounts_in_details(ledger_details, close_amount_a_raw, close_amount_b_raw);
        let _lp_on_close = self
            .close_position(position, pool, ledger_session_id, close_details)
            .await?;
        Ok(())
    }

    /// Bulk close send-first: broadcast close tx, return before confirmation.
    pub async fn execute_bulk_close_submit_only(
        &self,
        position: &Pubkey,
        pool: &Pubkey,
        ledger_details: Option<serde_json::Value>,
        slippage_bps: Option<u16>,
    ) -> anyhow::Result<clmm_lp_protocols::orca::executor::ExecutionResult> {
        if self.is_dry_run() {
            info!("Dry run: would bulk-close position (submit only)");
            return Ok(clmm_lp_protocols::orca::executor::ExecutionResult::submitted(
                solana_sdk::signature::Signature::default(),
            ));
        }
        let (close_amount_a_raw, close_amount_b_raw) = self
            .read_close_amounts_best_effort(position, pool)
            .await
            .unwrap_or((0, 0));
        let close_details =
            with_close_amounts_in_details(ledger_details, close_amount_a_raw, close_amount_b_raw);
        self.close_position_submit_only(position, pool, close_details, slippage_bps)
            .await
    }

    /// After send-first: wait for confirm, then lifecycle/registry/ledger hooks.
    pub async fn finalize_bulk_close_after_confirm(
        &self,
        submitted: &clmm_lp_protocols::orca::executor::ExecutionResult,
        position: &Pubkey,
        pool: &Pubkey,
        ledger_session_id: Option<String>,
        ledger_details: Option<serde_json::Value>,
    ) -> anyhow::Result<()> {
        let tx_res = self
            .tx_manager
            .wait_for_confirmation(&submitted.signature)
            .await
            .map_err(|e| anyhow::anyhow!("close confirm: {e}"))?;
        let confirmed = clmm_lp_protocols::orca::executor::ExecutionResult {
            signature: submitted.signature,
            success: true,
            slot: Some(tx_res.slot),
            error: None,
            created_position: None,
            collect_fee_owed_a_raw: submitted.collect_fee_owed_a_raw,
            collect_fee_owed_b_raw: submitted.collect_fee_owed_b_raw,
        };
        self.record_execution_success(
            "close_position",
            &confirmed,
            Some(*pool),
            Some(*position),
            ledger_session_id,
            ledger_details,
            submitted.collect_fee_owed_a_raw,
            submitted.collect_fee_owed_b_raw,
        )
        .await
    }

    /// Remove `liquidity_amount` from an existing position (partial exit). `token_min_*` = 0 (max slippage).
    pub async fn execute_partial_decrease(
        &self,
        position: &Pubkey,
        pool: &Pubkey,
        liquidity_amount: u128,
    ) -> anyhow::Result<()> {
        if liquidity_amount == 0 {
            anyhow::bail!("liquidity_amount must be > 0");
        }
        if self.is_dry_run() {
            info!(
                position = %position,
                liquidity = liquidity_amount,
                "Dry run: would decrease liquidity"
            );
            return Ok(());
        }
        self.decrease_liquidity(position, pool, liquidity_amount)
            .await?;
        Ok(())
    }

    /// Collects fees from a position.
    async fn collect_fees(
        &self,
        position: &Pubkey,
        pool: &Pubkey,
        ledger_session_id: Option<String>,
    ) -> anyhow::Result<(u64, u64)> {
        if self.is_dry_run() {
            debug!(position = %position, "Dry run: skipping collect_fees");
            return Ok((0, 0));
        }
        let wallet = self.require_wallet()?;
        let reader = PositionReader::new(self.provider.clone());
        let (fee_owed_a, fee_owed_b) = match reader.get_position(&position.to_string()).await {
            Ok(pos_state) => (Some(pos_state.fees_owed_a), Some(pos_state.fees_owed_b)),
            Err(e) => {
                warn!(
                    position = %position,
                    error = %e,
                    "collect_fees: cannot read position fee_owed_*; continuing collect without authoritative pre-tx legs"
                );
                (None, None)
            }
        };

        let orca = WhirlpoolExecutor::new(self.provider.clone());
        let payer = wallet.keypair();
        let res = orca.collect_fees(position, pool, payer).await?;
        let fee_owed_a = res.collect_fee_owed_a_raw.or(fee_owed_a);
        let fee_owed_b = res.collect_fee_owed_b_raw.or(fee_owed_b);
        self.ensure_execution_success(
            "collect_fees",
            &res,
            Some(*pool),
            Some(*position),
            ledger_session_id,
            None,
            fee_owed_a,
            fee_owed_b,
        )
        .await?;

        debug!(
            position = %position,
            quote_fee_owed_a = ?res.collect_fee_owed_a_raw,
            quote_fee_owed_b = ?res.collect_fee_owed_b_raw,
            fee_owed_a = ?fee_owed_a,
            fee_owed_b = ?fee_owed_b,
            "Collect fees succeeded (using Orca harvest quote for both legs when available)"
        );
        Ok((fee_owed_a.unwrap_or(0), fee_owed_b.unwrap_or(0)))
    }

    /// Decreases liquidity on-chain (`token_min_*` = 0 — set stricter mins when wiring slippage).
    async fn decrease_liquidity(
        &self,
        position: &Pubkey,
        pool: &Pubkey,
        liquidity_amount: u128,
    ) -> anyhow::Result<()> {
        let wallet = self.require_wallet()?;
        let orca = WhirlpoolExecutor::new(self.provider.clone());
        let payer = wallet.keypair();
        let params = DecreaseLiquidityParams {
            position: *position,
            pool: *pool,
            liquidity_amount,
            token_min_a: 0,
            token_min_b: 0,
        };
        let res = orca.decrease_liquidity(&params, payer).await?;
        self.ensure_execution_success(
            "decrease_liquidity",
            &res,
            Some(*pool),
            Some(*position),
            None,
            None,
            None,
            None,
        )
        .await?;
        debug!(
            position = %position,
            liquidity = liquidity_amount,
            "Decrease liquidity submitted"
        );
        Ok(())
    }

    /// Closes a position. Returns `(lp_collected_token_a_raw, lp_collected_token_b_raw)` from the
    /// close instruction quote (same values written to lifecycle `lp_collected_token_*` on the row).
    async fn close_position(
        &self,
        position: &Pubkey,
        pool: &Pubkey,
        ledger_session_id: Option<String>,
        ledger_details: Option<serde_json::Value>,
    ) -> anyhow::Result<(u64, u64)> {
        let wallet = self.require_wallet()?;
        let orca = WhirlpoolExecutor::new(self.provider.clone());

        let payer = wallet.keypair();
        let res = orca.close_position(position, pool, payer, None).await?;
        let lp_a = res.collect_fee_owed_a_raw.unwrap_or(0);
        let lp_b = res.collect_fee_owed_b_raw.unwrap_or(0);
        self.ensure_execution_success(
            "close_position",
            &res,
            Some(*pool),
            Some(*position),
            ledger_session_id,
            ledger_details,
            res.collect_fee_owed_a_raw,
            res.collect_fee_owed_b_raw,
        )
        .await?;
        debug!(position = %position, "Close position submitted");
        Ok((lp_a, lp_b))
    }

    /// Send-only close (bulk send-first); caller must `finalize_bulk_close_after_confirm`.
    async fn close_position_submit_only(
        &self,
        position: &Pubkey,
        pool: &Pubkey,
        ledger_details: Option<serde_json::Value>,
        slippage_bps: Option<u16>,
    ) -> anyhow::Result<clmm_lp_protocols::orca::executor::ExecutionResult> {
        let wallet = self.require_wallet()?;
        let orca = WhirlpoolExecutor::new(self.provider.clone());
        let payer = wallet.keypair();
        let res = orca
            .close_position_submit_only(position, pool, payer, slippage_bps)
            .await?;
        validate_execution_result("close_position", &res)?;
        debug!(
            position = %position,
            signature = %res.signature,
            "Close position submitted (send-first)"
        );
        let _ = ledger_details;
        Ok(res)
    }

    /// Best-effort authoritative position leg amounts (raw) immediately before close.
    ///
    /// We persist these values in lifecycle close `details` so lineage `end value` can be
    /// reconstructed even when `fee_payer_token_deltas` is missing one pool leg (e.g. WSOL path).
    async fn read_close_amounts_best_effort(
        &self,
        position: &Pubkey,
        pool: &Pubkey,
    ) -> anyhow::Result<(u64, u64)> {
        let pos_reader = PositionReader::new(self.provider.clone());
        let pool_reader = WhirlpoolReader::new(self.provider.clone());
        let pos = pos_reader
            .get_position(&position.to_string())
            .await
            .with_context(|| format!("read position state before close for {}", position))?;
        let pool_state = pool_reader
            .get_pool_state(&pool.to_string())
            .await
            .with_context(|| format!("read pool state before close for {}", pool))?;
        Ok(
            pos_reader.calculate_token_amounts(
                &pos,
                pool_state.tick_current,
                pool_state.sqrt_price,
            ),
        )
    }

    /// Opens a new position.
    #[allow(clippy::too_many_arguments)]
    async fn open_position(
        &self,
        _pool: &Pubkey,
        tick_lower: i32,
        tick_upper: i32,
        cap_a: u64,
        cap_b: u64,
        ledger_session_id: Option<String>,
        ledger_open_details: Option<serde_json::Value>,
    ) -> anyhow::Result<Pubkey> {
        let (p, _, _) = self
            .open_position_with_caps(
                _pool,
                tick_lower,
                tick_upper,
                cap_a,
                cap_b,
                self.config.max_slippage_bps,
                ledger_session_id,
                ledger_open_details,
            )
            .await?;
        Ok(p)
    }

    /// Orca swap **ExactIn** in the given Whirlpool (same pool as subsequent open / rebalance).
    ///
    /// Returns `None` in dry-run mode; otherwise the swap transaction signature.
    pub async fn execute_swap_exact_in(
        &self,
        pool: &Pubkey,
        specified_mint: &Pubkey,
        amount_in: u64,
        slippage_bps: u16,
        // Optional position PDA to attach to the lifecycle ledger row for matching in UI summaries.
        // Swap-mix flows have a "current position" (old PDA) even though the swap tx itself doesn't.
        position_for_ledger: Option<Pubkey>,
        ledger_session_id: Option<String>,
    ) -> anyhow::Result<Option<Signature>> {
        if self.is_dry_run() {
            info!(
                pool = %pool,
                specified_mint = %specified_mint,
                amount_in = amount_in,
                "Dry run: would swap in pool before next step"
            );
            return Ok(None);
        }
        let wallet = self.require_wallet()?;
        let orca = WhirlpoolExecutor::new(self.provider.clone());
        let payer = wallet.keypair();

        let reader = WhirlpoolReader::new(self.provider.clone());
        let pool_state = reader.get_pool_state(&pool.to_string()).await.ok();
        let dec_in = spl_mint_decimals(self.provider.as_ref(), specified_mint)
            .await
            .unwrap_or(0);
        let amount_in_ui = (amount_in as f64) / 10f64.powi(i32::from(dec_in));
        let (token_a_s, token_b_s, other_out) = match &pool_state {
            Some(s) => {
                let ta = s.token_mint_a.to_string();
                let tb = s.token_mint_b.to_string();
                let other = if *specified_mint == s.token_mint_a {
                    tb.clone()
                } else if *specified_mint == s.token_mint_b {
                    ta.clone()
                } else {
                    "specified_mint_not_in_pool".to_string()
                };
                (ta, tb, other)
            }
            None => (String::new(), String::new(), String::new()),
        };
        let swap_details = serde_json::json!({
            "kind": "orca_whirlpool_swap_exact_in",
            "pool": pool.to_string(),
            "token_mint_a": token_a_s,
            "token_mint_b": token_b_s,
            "specified_mint": specified_mint.to_string(),
            "other_mint_expected_output": other_out,
            "amount_in_raw": amount_in,
            "specified_mint_decimals": dec_in,
            "amount_in_ui": amount_in_ui,
            "slippage_bps": slippage_bps,
            "note": "ExactIn: amount_out_min/actual not in ExecutionResult; use RPC getTransaction meta or extend Orca executor later."
        });

        let res = orca
            .swap_exact_in(*pool, *specified_mint, amount_in, slippage_bps, payer)
            .await?;
        let sig = res.signature;
        self.ensure_execution_success(
            "swap_exact_in",
            &res,
            Some(*pool),
            position_for_ledger,
            ledger_session_id,
            Some(swap_details),
            None,
            None,
        )
        .await?;
        Ok(Some(sig))
    }

    /// Opens a new position with explicit token caps and slippage.
    ///
    /// In dry-run mode returns the derived Whirlpool position PDA without requiring wallet.
    /// Returns `(position_pda, effective_tick_lower, effective_tick_upper)`.
    #[allow(clippy::too_many_arguments)]
    pub async fn execute_open_position(
        &self,
        pool: &Pubkey,
        tick_lower: i32,
        tick_upper: i32,
        amount_a: u64,
        amount_b: u64,
        slippage_bps: u16,
        full_range: bool,
        ledger_session_id: Option<String>,
        // Merged into lifecycle `details` on successful open (e.g. `open_origin` for lineage).
        ledger_open_details: Option<serde_json::Value>,
    ) -> anyhow::Result<(Pubkey, i32, i32)> {
        if self.is_dry_run() {
            if full_range {
                let reader = WhirlpoolReader::new(self.provider.clone());
                let state = reader
                    .get_pool_state(&pool.to_string())
                    .await
                    .context("fetch pool for full-range dry-run")?;
                let (tl, tu) = full_range_tick_indexes(state.tick_spacing);
                return Ok((derive_whirlpool_position_address(pool, tl, tu), tl, tu));
            }
            return Ok((
                derive_whirlpool_position_address(pool, tick_lower, tick_upper),
                tick_lower,
                tick_upper,
            ));
        }
        if full_range {
            return self
                .open_full_range_position_with_caps(
                    pool,
                    amount_a,
                    amount_b,
                    slippage_bps,
                    ledger_session_id,
                    ledger_open_details,
                )
                .await;
        }
        self.open_position_with_caps(
            pool,
            tick_lower,
            tick_upper,
            amount_a,
            amount_b,
            slippage_bps,
            ledger_session_id,
            ledger_open_details,
        )
        .await
    }

    async fn open_full_range_position_with_caps(
        &self,
        pool: &Pubkey,
        amount_a: u64,
        amount_b: u64,
        slippage_bps: u16,
        ledger_session_id: Option<String>,
        ledger_open_details: Option<serde_json::Value>,
    ) -> anyhow::Result<(Pubkey, i32, i32)> {
        let wallet = self.require_wallet()?;
        let orca = WhirlpoolExecutor::new(self.provider.clone());
        let payer = wallet.keypair();
        let params = OpenFullRangeParams {
            pool: *pool,
            amount_a,
            amount_b,
            slippage_bps,
        };
        let res = orca.open_full_range_position(&params, payer).await?;
        let details = merge_open_ledger_details(
            serde_json::json!({
                "slippage_bps": slippage_bps,
                "amount_a_cap": amount_a,
                "amount_b_cap": amount_b,
                "open_kind": "full_range"
            }),
            ledger_open_details,
        );
        self.ensure_execution_success(
            "open_full_range_position",
            &res,
            Some(*pool),
            None,
            ledger_session_id,
            Some(details),
            None,
            None,
        )
        .await?;
        let new_position = res.created_position.ok_or_else(|| {
            anyhow::anyhow!(
                "open_full_range_position succeeded but did not return created_position; cannot continue safely"
            )
        })?;
        let reader = WhirlpoolReader::new(self.provider.clone());
        let state = reader
            .get_pool_state(&pool.to_string())
            .await
            .context("fetch pool after full-range open")?;
        let (tl, tu) = full_range_tick_indexes(state.tick_spacing);
        debug!(
            new_position = %new_position,
            tick_lower = tl,
            tick_upper = tu,
            "Open full-range position submitted"
        );
        Ok((new_position, tl, tu))
    }

    #[allow(clippy::too_many_arguments)]
    async fn open_position_with_caps(
        &self,
        pool: &Pubkey,
        tick_lower: i32,
        tick_upper: i32,
        amount_a: u64,
        amount_b: u64,
        slippage_bps: u16,
        ledger_session_id: Option<String>,
        ledger_open_details: Option<serde_json::Value>,
    ) -> anyhow::Result<(Pubkey, i32, i32)> {
        let wallet = self.require_wallet()?;
        let orca = WhirlpoolExecutor::new(self.provider.clone());

        let payer = wallet.keypair();

        // Send maximal token caps so the program uses the required amounts from wallet balances.
        let params = OpenPositionParams {
            pool: *pool,
            tick_lower,
            tick_upper,
            amount_a,
            amount_b,
            slippage_bps,
        };

        let res = orca.open_position(&params, payer).await?;
        let details = merge_open_ledger_details(
            serde_json::json!({
                "tick_lower": tick_lower,
                "tick_upper": tick_upper,
                "slippage_bps": slippage_bps,
                "amount_a_cap": amount_a,
                "amount_b_cap": amount_b,
                "open_kind": "tick_range"
            }),
            ledger_open_details,
        );
        self.ensure_execution_success(
            "open_position",
            &res,
            Some(*pool),
            None,
            ledger_session_id,
            Some(details),
            None,
            None,
        )
        .await?;
        let new_position = res.created_position.ok_or_else(|| {
            anyhow::anyhow!(
                "open_position succeeded but did not return created_position; cannot continue safely"
            )
        })?;
        debug!(
            new_position = %new_position,
            tick_lower = tick_lower,
            tick_upper = tick_upper,
            "Open position submitted"
        );
        Ok((new_position, tick_lower, tick_upper))
    }

    /// Increases liquidity in a position.
    #[allow(dead_code)]
    async fn increase_liquidity(
        &self,
        _position: &Pubkey,
        liquidity: u128,
    ) -> anyhow::Result<u128> {
        // TODO: Implement actual liquidity increase via Whirlpool instruction
        debug!(liquidity = liquidity, "Would increase liquidity");
        Ok(liquidity)
    }

    #[allow(clippy::too_many_arguments)]
    async fn record_execution_success(
        &self,
        op_name: &str,
        result: &clmm_lp_protocols::orca::executor::ExecutionResult,
        pool: Option<Pubkey>,
        position: Option<Pubkey>,
        ledger_session_id: Option<String>,
        ledger_details: Option<serde_json::Value>,
        lp_collected_token_a_raw: Option<u64>,
        lp_collected_token_b_raw: Option<u64>,
    ) -> anyhow::Result<()> {
        validate_execution_result(op_name, result)?;

        if result.success {
            self.invoke_chain_history_hook(op_name, position, result);

            let fee_payer = self
                .wallet
                .lock()
                .ok()
                .and_then(|g| g.as_ref().map(|w| w.pubkey()));
            if let Some(fee_payer) = fee_payer {
                let ledger_for_append = if matches!(
                    op_name,
                    "open_position" | "open_full_range_position" | "close_position"
                ) {
                    Some(
                        enrich_open_close_ledger_details(
                            self.provider.clone(),
                            pool,
                            result,
                            ledger_details.clone(),
                        )
                        .await,
                    )
                } else {
                    ledger_details.clone()
                };

                clmm_lp_protocols::ledger::tx_lifecycle::try_append_rebalance_executor_tx_cost(
                    self.provider.as_ref(),
                    &fee_payer,
                    &result.signature,
                    op_name,
                    pool,
                    position,
                    result.created_position,
                    ledger_session_id.clone(),
                    ledger_for_append.clone(),
                    lp_collected_token_a_raw,
                    lp_collected_token_b_raw,
                )
                .await;

                if op_name == "close_position"
                    && let (Some(pool_pk), Some(pos_pk)) = (pool, position)
                {
                    let close_kind = ledger_details
                        .as_ref()
                        .and_then(|d| d.get("close_kind"))
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(|s| if s == "manual" { "manual" } else { "strategy" });
                    clmm_lp_protocols::ledger::position_registry::try_append_registry_close(
                        self.provider.as_ref(),
                        "orca_bot",
                        &pos_pk,
                        &pool_pk,
                        &fee_payer,
                        &result.signature,
                        ledger_session_id.clone(),
                        close_kind,
                    )
                    .await;
                }
                if matches!(op_name, "open_position" | "open_full_range_position")
                    && let (Some(pool_pk), Some(created)) = (pool, result.created_position)
                {
                    clmm_lp_protocols::ledger::position_registry::try_append_registry_open(
                        self.provider.as_ref(),
                        "orca_bot",
                        &created,
                        &pool_pk,
                        &fee_payer,
                        &result.signature,
                        ledger_session_id,
                        ledger_for_append.clone(),
                    )
                    .await;
                }
            }
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn ensure_execution_success(
        &self,
        op_name: &str,
        result: &clmm_lp_protocols::orca::executor::ExecutionResult,
        pool: Option<Pubkey>,
        position: Option<Pubkey>,
        ledger_session_id: Option<String>,
        ledger_details: Option<serde_json::Value>,
        lp_collected_token_a_raw: Option<u64>,
        lp_collected_token_b_raw: Option<u64>,
    ) -> anyhow::Result<()> {
        self.record_execution_success(
            op_name,
            result,
            pool,
            position,
            ledger_session_id,
            ledger_details,
            lp_collected_token_a_raw,
            lp_collected_token_b_raw,
        )
        .await?;

        // Best-effort post-check through the common transaction manager path.
        // Some providers may not return status immediately for very fresh signatures.
        match tokio::time::timeout(
            std::time::Duration::from_secs(15),
            self.tx_manager.wait_for_confirmation(&result.signature),
        )
        .await
        {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => {
                warn!(
                    operation = op_name,
                    signature = %result.signature,
                    error = %e,
                    "Post-confirmation check failed; continuing because executor already reported success"
                );
            }
            Err(_) => {
                warn!(
                    operation = op_name,
                    signature = %result.signature,
                    "Post-confirmation check timed out; continuing because executor already reported success"
                );
            }
        }

        Ok(())
    }
}

/// Merge `event_slot` + best-effort pool mint USD spot into lifecycle `details` (open/close only).
async fn enrich_open_close_ledger_details(
    provider: Arc<RpcProvider>,
    pool: Option<Pubkey>,
    result: &clmm_lp_protocols::orca::executor::ExecutionResult,
    ledger_details: Option<serde_json::Value>,
) -> serde_json::Value {
    let mut base = match ledger_details {
        Some(serde_json::Value::Object(m)) => serde_json::Value::Object(m),
        Some(other) => {
            let mut m = serde_json::Map::new();
            m.insert("_non_object_ledger_details".to_string(), other);
            serde_json::Value::Object(m)
        }
        None => serde_json::json!({}),
    };
    if let Some(slot) = result.slot
        && let Some(obj) = base.as_object_mut()
    {
        obj.insert("event_slot".to_string(), slot.into());
    }
    if let Some(pool_pk) = pool {
        if let Ok(Ok(pool_state)) = tokio::time::timeout(
            std::time::Duration::from_secs(8),
            WhirlpoolReader::new(provider.clone()).get_pool_state(&pool_pk.to_string()),
        )
        .await
        {
            if let Some(obj) = base.as_object_mut() {
                obj.insert(
                    "token_mint_a".to_string(),
                    serde_json::json!(pool_state.token_mint_a.to_string()),
                );
                obj.insert(
                    "token_mint_b".to_string(),
                    serde_json::json!(pool_state.token_mint_b.to_string()),
                );
            }
        }
        match tokio::time::timeout(
            std::time::Duration::from_secs(8),
            clmm_lp_protocols::orca::event_pool_mint_usd::fetch_event_pool_mint_usd_prices(
                provider.clone(),
                &pool_pk,
            ),
        )
        .await
        {
            Ok(Some(ev)) => {
                if let Some(obj) = base.as_object_mut() {
                    obj.insert(
                        "event_price_a_usd".to_string(),
                        serde_json::json!(ev.price_a_usd),
                    );
                    obj.insert(
                        "event_price_b_usd".to_string(),
                        serde_json::json!(ev.price_b_usd),
                    );
                    obj.insert(
                        "event_price_source".to_string(),
                        serde_json::json!(ev.price_source),
                    );
                }
            }
            Ok(None) => {
                warn!(
                    pool = %pool_pk,
                    "event-time pool USD enrichment skipped (no prices from pool read + feed)"
                );
            }
            Err(_) => {
                warn!(pool = %pool_pk, "event-time pool USD enrichment timed out");
            }
        }
    }

    // Best-effort: on successful open flows, record *measured* token amounts by reading the created
    // position + pool state on-chain and computing amounts from liquidity.
    if let (Some(pool_pk), Some(pos_pk)) = (pool, result.created_position) {
        match tokio::time::timeout(std::time::Duration::from_secs(30), async {
            let pool_reader = WhirlpoolReader::new(provider.clone());
            let pos_reader = PositionReader::new(provider.clone());

            // RPC can lag right after confirmation; retry a few times before giving up.
            let mut last_err: Option<anyhow::Error> = None;
            for attempt in 1..=8u32 {
                let pool_state = match pool_reader.get_pool_state(&pool_pk.to_string()).await {
                    Ok(s) => s,
                    Err(e) => {
                        last_err = Some(e);
                        tokio::time::sleep(std::time::Duration::from_millis(
                            400 * u64::from(attempt),
                        ))
                        .await;
                        continue;
                    }
                };
                let pos = match pos_reader.get_position(&pos_pk.to_string()).await {
                    Ok(p) => p,
                    Err(e) => {
                        last_err = Some(e);
                        tokio::time::sleep(std::time::Duration::from_millis(
                            400 * u64::from(attempt),
                        ))
                        .await;
                        continue;
                    }
                };

                let (a_raw, b_raw) = pos_reader.calculate_token_amounts(
                    &pos,
                    pool_state.tick_current,
                    pool_state.sqrt_price,
                );
                return Ok::<(u64, u64, Pubkey, Pubkey), anyhow::Error>((
                    a_raw,
                    b_raw,
                    pool_state.token_mint_a,
                    pool_state.token_mint_b,
                ));
            }
            Err(last_err.unwrap_or_else(|| anyhow::anyhow!("unknown RPC error")))
        })
        .await
        {
            Ok(Ok((a_raw, b_raw, mint_a, mint_b))) => {
                let dec_a =
                    fetch_mint_decimals_best_effort(provider.as_ref(), &mint_a).await;
                let dec_b =
                    fetch_mint_decimals_best_effort(provider.as_ref(), &mint_b).await;
                if let Some(obj) = base.as_object_mut() {
                    obj.insert("open_amount_a_raw".to_string(), serde_json::json!(a_raw));
                    obj.insert("open_amount_b_raw".to_string(), serde_json::json!(b_raw));
                    obj.insert(
                        "open_amounts_source".to_string(),
                        serde_json::json!("onchain_after_open"),
                    );
                    insert_open_quote_usd_fields(obj, dec_a, dec_b);
                }
            }
            Ok(Err(e)) => {
                if let Some(obj) = base.as_object_mut() {
                    obj.insert("open_amounts_pending".to_string(), serde_json::json!(true));
                }
                warn!(error = %e, "post-open on-chain amount enrichment failed; continuing");
            }
            Err(_) => {
                if let Some(obj) = base.as_object_mut() {
                    obj.insert("open_amounts_pending".to_string(), serde_json::json!(true));
                }
                warn!("post-open on-chain amount enrichment timed out; continuing");
            }
        }
    }
    base
}

async fn fetch_mint_decimals_best_effort(provider: &RpcProvider, mint: &Pubkey) -> u8 {
    match provider.get_account(mint).await {
        Ok(account) => SplMint::unpack(&account.data)
            .map(|m| m.decimals)
            .unwrap_or(9),
        Err(_) => 9,
    }
}

fn raw_token_ui(raw: u64, decimals: u8) -> f64 {
    raw as f64 / 10f64.powi(i32::from(decimals))
}

fn insert_open_quote_usd_fields(
    obj: &mut serde_json::Map<String, serde_json::Value>,
    decimals_a: u8,
    decimals_b: u8,
) {
    let pa = obj
        .get("event_price_a_usd")
        .and_then(|v| v.as_f64())
        .filter(|x| x.is_finite() && *x > 0.0);
    let pb = obj
        .get("event_price_b_usd")
        .and_then(|v| v.as_f64())
        .filter(|x| x.is_finite() && *x > 0.0);
    let (Some(pa), Some(pb)) = (pa, pb) else {
        return;
    };

    let usd_from_amounts = || -> Option<f64> {
        let a_raw = obj.get("open_amount_a_raw")?.as_u64()?;
        let b_raw = obj.get("open_amount_b_raw")?.as_u64()?;
        Some(raw_token_ui(a_raw, decimals_a) * pa + raw_token_ui(b_raw, decimals_b) * pb)
    };
    let usd_from_caps = || -> Option<f64> {
        let a_raw = obj.get("amount_a_cap")?.as_u64()?;
        let b_raw = obj.get("amount_b_cap")?.as_u64()?;
        Some(raw_token_ui(a_raw, decimals_a) * pa + raw_token_ui(b_raw, decimals_b) * pb)
    };

    if let Some(usd) = usd_from_amounts()
        .or_else(usd_from_caps)
        .filter(|x| x.is_finite() && *x > 0.0)
    {
        obj.insert(
            "open_quote_estimated_value_usd".to_string(),
            serde_json::json!(usd),
        );
        obj.entry("open_target_usd".to_string())
            .or_insert(serde_json::json!(usd));
    }
}

fn with_close_amounts_in_details(
    details: Option<serde_json::Value>,
    close_amount_a_raw: u64,
    close_amount_b_raw: u64,
) -> Option<serde_json::Value> {
    let mut obj = match details {
        Some(serde_json::Value::Object(map)) => map,
        Some(other) => {
            let mut map = serde_json::Map::new();
            map.insert("_non_object_ledger_details".to_string(), other);
            map
        }
        None => serde_json::Map::new(),
    };
    obj.insert(
        "close_amount_a_raw".to_string(),
        serde_json::json!(close_amount_a_raw),
    );
    obj.insert(
        "close_amount_b_raw".to_string(),
        serde_json::json!(close_amount_b_raw),
    );
    Some(serde_json::Value::Object(obj))
}

/// `bot_close_position` row → **returned** pool-leg raw amounts (§6.1):
/// `close_amount_*_raw` + `lp_collected_token_*_raw` (top-level row fields from lifecycle append,
/// or same keys inside `details` for older rows). Missing fee legs count as 0.
fn close_amounts_from_lifecycle_row(
    row: &serde_json::Value,
    closed_position: &Pubkey,
    rebalance_session_id: Option<&str>,
) -> Option<(u64, u64)> {
    let event = row.get("event")?.as_str()?.trim();
    if event != "bot_close_position" {
        return None;
    }
    let row_position = row.get("position_pubkey")?.as_str()?.trim();
    if row_position != closed_position.to_string() {
        return None;
    }
    if let Some(sid) = rebalance_session_id {
        let row_sid = row
            .get("rebalance_session_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if row_sid != sid {
            return None;
        }
    }
    let details = row.get("details")?.as_object()?;
    let close_amount_a_raw = details.get("close_amount_a_raw")?.as_u64()?;
    let close_amount_b_raw = details.get("close_amount_b_raw")?.as_u64()?;
    let lp_a = row
        .get("lp_collected_token_a_raw")
        .and_then(|v| v.as_u64())
        .or_else(|| {
            details
                .get("lp_collected_token_a_raw")
                .and_then(|v| v.as_u64())
        })
        .unwrap_or(0);
    let lp_b = row
        .get("lp_collected_token_b_raw")
        .and_then(|v| v.as_u64())
        .or_else(|| {
            details
                .get("lp_collected_token_b_raw")
                .and_then(|v| v.as_u64())
        })
        .unwrap_or(0);
    Some((
        close_amount_a_raw.saturating_add(lp_a),
        close_amount_b_raw.saturating_add(lp_b),
    ))
}

fn close_amounts_from_lifecycle_best_effort(
    closed_position: &Pubkey,
    rebalance_session_id: Option<&str>,
) -> Option<(u64, u64)> {
    let path = clmm_lp_protocols::ledger::tx_lifecycle::ledger_read_path();
    let file = File::open(path).ok()?;
    let reader = BufReader::new(file);
    let mut latest_match: Option<(u64, u64)> = None;
    for line in reader.lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(row) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };
        if let Some(amounts) =
            close_amounts_from_lifecycle_row(&row, closed_position, rebalance_session_id)
        {
            latest_match = Some(amounts);
        }
    }
    latest_match
}

fn validate_execution_result(
    op_name: &str,
    result: &clmm_lp_protocols::orca::executor::ExecutionResult,
) -> anyhow::Result<()> {
    if !result.success {
        let mut msg = result
            .error
            .clone()
            .unwrap_or_else(|| "unknown execution error".to_string());
        if op_name == "close_position" && (msg.contains("6018") || msg.contains("0x1782")) {
            msg.push_str(
                " | Hint: Whirlpool 6018 (TokenMinSubceeded) — min-out too tight vs. pool move. \
                 Prefer low slippage: retry once, collect fees first, then if needed raise only for that close \
                 (CLI `--slippage-bps 500`…`1000`, or `WHIRLPOOL_CLOSE_SLIPPAGE_BPS` on the API host). \
                 Default remains 100 bps unless env overrides.",
            );
        }
        return Err(anyhow::anyhow!("{} failed: {}", op_name, msg));
    }
    Ok(())
}

/// Result of profitability check.
#[derive(Debug, Clone)]
pub struct ProfitabilityCheck {
    /// Whether rebalance is profitable.
    pub is_profitable: bool,
    /// Estimated transaction cost in lamports.
    pub estimated_tx_cost: u64,
    /// Expected benefit in USD.
    pub expected_benefit: Decimal,
    /// Minimum required benefit.
    pub min_required_benefit: Decimal,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clmm_lp_protocols::orca::executor::ExecutionResult;
    use solana_sdk::signature::Signature;

    #[test]
    fn insert_open_quote_usd_fields_from_onchain_amounts() {
        let mut obj = serde_json::Map::new();
        obj.insert("open_amount_a_raw".to_string(), serde_json::json!(50_000_000u64));
        obj.insert("open_amount_b_raw".to_string(), serde_json::json!(5_000_000u64));
        obj.insert("event_price_a_usd".to_string(), serde_json::json!(100.0));
        obj.insert("event_price_b_usd".to_string(), serde_json::json!(1.0));
        insert_open_quote_usd_fields(&mut obj, 9, 6);
        let usd = obj
            .get("open_quote_estimated_value_usd")
            .and_then(|v| v.as_f64())
            .expect("quote usd");
        assert!((usd - 10.0).abs() < 1e-6);
        assert_eq!(
            obj.get("open_target_usd").and_then(|v| v.as_f64()),
            Some(usd)
        );
    }

    #[tokio::test]
    async fn test_rebalance_config_default() {
        let config = RebalanceConfig::default();
        assert_eq!(config.max_slippage_bps, 50);
        assert!(config.collect_fees_first);
    }

    #[test]
    fn test_validate_execution_result_success() {
        let res = ExecutionResult::success(Signature::default(), 1);
        assert!(validate_execution_result("open_position", &res).is_ok());
    }

    #[test]
    fn test_validate_execution_result_failure() {
        let res = ExecutionResult::failure(Signature::default(), "boom".to_string());
        let err = validate_execution_result("open_position", &res).expect_err("must fail");
        assert!(err.to_string().contains("open_position failed: boom"));
    }

    #[test]
    fn with_close_amounts_in_details_preserves_manual_fields() {
        let out = with_close_amounts_in_details(
            Some(serde_json::json!({
                "close_kind": "manual",
                "close_source": "api"
            })),
            123,
            456,
        )
        .expect("details");
        let obj = out.as_object().expect("object");
        assert_eq!(
            obj.get("close_kind").and_then(|v| v.as_str()),
            Some("manual")
        );
        assert_eq!(
            obj.get("close_source").and_then(|v| v.as_str()),
            Some("api")
        );
        assert_eq!(
            obj.get("close_amount_a_raw").and_then(|v| v.as_u64()),
            Some(123)
        );
        assert_eq!(
            obj.get("close_amount_b_raw").and_then(|v| v.as_u64()),
            Some(456)
        );
    }

    #[test]
    fn close_amounts_from_lifecycle_row_parses_matching_close() {
        let closed_position = Pubkey::new_unique();
        let sid = "sid-123";
        let row = serde_json::json!({
            "event": "bot_close_position",
            "position_pubkey": closed_position.to_string(),
            "rebalance_session_id": sid,
            "details": {
                "close_amount_a_raw": 3648,
                "close_amount_b_raw": 588
            }
        });
        let parsed =
            close_amounts_from_lifecycle_row(&row, &closed_position, Some(sid)).expect("parsed");
        assert_eq!(parsed, (3648, 588));
    }

    #[test]
    fn close_amounts_from_lifecycle_row_adds_lp_collected_from_row() {
        let closed_position = Pubkey::new_unique();
        let sid = "sid-lp";
        let row = serde_json::json!({
            "event": "bot_close_position",
            "position_pubkey": closed_position.to_string(),
            "rebalance_session_id": sid,
            "lp_collected_token_a_raw": 10,
            "lp_collected_token_b_raw": 20,
            "details": {
                "close_amount_a_raw": 100,
                "close_amount_b_raw": 200
            }
        });
        let parsed =
            close_amounts_from_lifecycle_row(&row, &closed_position, Some(sid)).expect("parsed");
        assert_eq!(parsed, (110, 220));
    }

    #[test]
    fn target_usd_reopen_sizing_applies_dust_margin_only() {
        let t = target_usd_for_reopen_sizing(10.0);
        assert!((t - 9.95).abs() < 1e-9);
    }

    #[test]
    fn target_usd_reopen_sizing_does_not_clamp_to_smaller_wallet() {
        // Spec §2.2 / A4: reopen target is not `min(prev_end, wallet)`.
        let t = target_usd_for_swap_mix_and_open(10.0, 8.0);
        assert!((t - 9.95).abs() < 1e-9);
    }

    #[test]
    fn target_usd_for_swap_mix_falls_back_to_wallet_cap_when_prev_end_zero() {
        let t = target_usd_for_swap_mix_and_open(0.0, 10.0);
        assert!((t - (10.0 * 0.995)).abs() < 1e-9);
    }

    #[test]
    fn final_caps_guard_rejects_materially_undersized_quote_leg() {
        let q = DepositBudgetQuote {
            amount_a: 1_000_000,
            amount_b: 5_000_000,
            token_max_a: 1_005_000,
            token_max_b: 5_025_000,
            estimated_value_usd: 9.95,
            liquidity: 42,
        };

        assert!(final_caps_cover_deposit_quote(
            q.token_max_a,
            q.token_max_b,
            &q
        ));
        assert!(final_caps_cover_deposit_quote(
            q.amount_a - 9_999,
            q.amount_b - 49_999,
            &q
        ));
        assert!(!final_caps_cover_deposit_quote(
            q.amount_a,
            q.amount_b / 2,
            &q
        ));
    }

    #[test]
    fn preflight_target_usd_includes_position_value_when_wallet_spl_empty() {
        let prev_end = 10.0;
        let wallet = 0.0;
        let target = target_usd_for_close_reopen_preflight(prev_end, wallet);
        assert!((target - (prev_end * 0.995)).abs() < 1e-9);
    }

    #[test]
    fn preflight_target_usd_matches_legacy_clamp_when_wallet_already_covers_prev_end() {
        let prev_end = 10.0;
        let wallet = 100.0;
        let preflight = target_usd_for_close_reopen_preflight(prev_end, wallet);
        let legacy = target_usd_from_prev_end_clamped(prev_end, wallet);
        assert!((preflight - legacy).abs() < 1e-9);
    }

    #[test]
    fn preflight_target_usd_clamps_when_prev_end_exceeds_post_close_spendable_cap() {
        let prev_end = 200.0;
        let wallet = 1.0;
        let spendable_cap = (wallet + prev_end) * 0.995;
        let target = target_usd_for_close_reopen_preflight(prev_end, wallet);
        assert!((target - prev_end.min(spendable_cap)).abs() < 1e-6);
        assert!(target < prev_end);
    }

    #[test]
    fn prev_end_value_usd_from_close_amounts_uses_decimals_and_prices() {
        // 1.5 * $2 + 3.0 * $1 = $6
        let v = prev_end_value_usd_from_close_amounts(1_500_000, 3_000_000, 6, 6, 2.0, 1.0);
        assert!((v - 6.0).abs() < 1e-9);
    }

    #[test]
    fn apply_session_caps_to_wallet_raw_limits_per_mint() {
        let wsol: Pubkey = clmm_lp_protocols::orca::executor::WSOL_MINT
            .parse()
            .expect("WSOL");
        let usdc = Pubkey::new_unique();
        let mut caps = clmm_lp_data::wallet_session::SessionMintCaps::empty("sess-cap");
        caps.caps_by_mint.insert(wsol.to_string(), 50);
        caps.caps_by_mint.insert(usdc.to_string(), 200);
        unsafe {
            std::env::set_var("CLMM_REOPEN_USE_SESSION_CAPITAL", "1");
        }
        let (wa, wb, spend) = apply_session_caps_to_wallet_raw(
            1_000,
            500,
            2_000_000_000,
            &wsol,
            &usdc,
            &wsol,
            Some(&caps),
        );
        assert_eq!(wa, 50);
        assert_eq!(wb, 200);
        assert_eq!(spend, 50); // native capped to WSOL session leg
        unsafe {
            std::env::remove_var("CLMM_REOPEN_USE_SESSION_CAPITAL");
        }
    }

    #[test]
    fn session_capital_error_if_strict_on_empty_session() {
        let empty = clmm_lp_data::wallet_session::SessionMintCaps::empty("sess-empty");
        unsafe {
            std::env::set_var("CLMM_REOPEN_USE_SESSION_CAPITAL", "1");
            std::env::set_var("CLMM_REOPEN_SESSION_STRICT_EMPTY", "1");
        }
        let err = RebalanceExecutor::session_capital_error_if_strict(&empty).expect("err");
        assert!(err.contains("session_capital_unknown"));
        assert!(err.contains("sess-empty"));
        unsafe {
            std::env::remove_var("CLMM_REOPEN_USE_SESSION_CAPITAL");
            std::env::remove_var("CLMM_REOPEN_SESSION_STRICT_EMPTY");
        }
    }

    #[test]
    fn session_cap_rpc_min_when_env_on_off() {
        let mint = Pubkey::new_unique();
        let mut caps = clmm_lp_data::wallet_session::SessionMintCaps::empty("sess-1");
        caps.caps_by_mint.insert(mint.to_string(), 40);
        unsafe {
            std::env::set_var("CLMM_REOPEN_USE_SESSION_CAPITAL", "1");
        }
        assert_eq!(
            crate::strategy::session_capital::cap_rpc_with_session(100, &mint, Some(&caps)),
            40
        );
        unsafe {
            std::env::remove_var("CLMM_REOPEN_USE_SESSION_CAPITAL");
        }
        assert_eq!(
            crate::strategy::session_capital::cap_rpc_with_session(100, &mint, Some(&caps)),
            100
        );
    }

    #[test]
    fn open_wallet_notional_and_caps_counts_native_sol_on_wsol_leg() {
        let wsol_mint_pk: Pubkey = clmm_lp_protocols::orca::executor::WSOL_MINT
            .parse()
            .expect("WSOL mint");
        let usdc_mint = Pubkey::new_unique();

        // No WSOL ATA balance, but we do have spendable native SOL.
        let wa_wsol = 0u64;
        let wb_usdc = 0u64;
        let dec_a = 9u8; // WSOL
        let dec_b = 6u8; // USDC
        let spendable_lamports = 500_000_000u64; // 0.5 SOL
        let price_a_usd = 100.0;
        let price_b_usd = 1.0;

        let wallet_inputs = SwapMixWalletInputs {
            token_mint_a: &wsol_mint_pk,
            token_mint_b: &usdc_mint,
            wsol_mint_pk: &wsol_mint_pk,
            balance_a_raw: wa_wsol,
            balance_b_raw: wb_usdc,
            decimals_a: dec_a,
            decimals_b: dec_b,
            spendable_lamports,
        };
        let (wallet_notional, cap_a, cap_b) =
            open_wallet_notional_and_caps_sol_first(&wallet_inputs, price_a_usd, price_b_usd);

        // Notional must reflect native SOL on the WSOL leg.
        assert!(wallet_notional > 0.0);
        assert!((wallet_notional - 50.0).abs() < 1e-6);
        // Cap on the WSOL leg should allow using native SOL.
        assert_eq!(cap_a, spendable_lamports);
        assert_eq!(cap_b, 0);
    }

    #[test]
    fn adapt_recover_open_ticks_keeps_in_range_unchanged() {
        let ((lo, hi), changed) = adapt_recover_open_ticks_if_needed(-100, 64, -128, 0);
        assert_eq!((lo, hi), (-128, 0));
        assert!(!changed);
    }

    #[test]
    fn adapt_recover_open_ticks_widens_stale_range() {
        let ((lo, hi), changed) = adapt_recover_open_ticks_if_needed(-24299, 64, -24264, -24160);
        assert!(changed);
        assert!(lo <= -24299 && -24299 < hi);
    }

    #[test]
    fn integration_recovery_tick_drift_adapts_to_current_tick() {
        // Integration-style invariant for pending-open recovery:
        // stale intended range must be adapted so a retry quote can target a range containing spot.
        let drifted_tick = -24299;
        let intended = (-24264, -24160);
        let ((lo, hi), changed) =
            adapt_recover_open_ticks_if_needed(drifted_tick, 64, intended.0, intended.1);
        assert!(changed);
        assert_ne!((lo, hi), intended);
        assert!(lo <= drifted_tick && drifted_tick < hi);
    }

    #[tokio::test]
    async fn execute_partial_decrease_rejects_zero() {
        let provider = Arc::new(RpcProvider::new(RpcConfig::default()));
        let tx_manager = Arc::new(TransactionManager::new(
            provider.clone(),
            crate::transaction::TransactionConfig::default(),
        ));
        let lifecycle = Arc::new(LifecycleTracker::new());
        let exec =
            RebalanceExecutor::new(provider, tx_manager, lifecycle, RebalanceConfig::default());
        let pos = Pubkey::new_unique();
        let pool = Pubkey::new_unique();
        let err = exec
            .execute_partial_decrease(&pos, &pool, 0)
            .await
            .expect_err("zero liquidity");
        assert!(err.to_string().contains("must be > 0"));
    }

    #[tokio::test]
    async fn execute_partial_decrease_dry_run_ok_without_wallet() {
        let provider = Arc::new(RpcProvider::new(RpcConfig::default()));
        let tx_manager = Arc::new(TransactionManager::new(
            provider.clone(),
            crate::transaction::TransactionConfig::default(),
        ));
        let lifecycle = Arc::new(LifecycleTracker::new());
        let exec =
            RebalanceExecutor::new(provider, tx_manager, lifecycle, RebalanceConfig::default());
        exec.set_dry_run(true);
        let pos = Pubkey::new_unique();
        let pool = Pubkey::new_unique();
        exec.execute_partial_decrease(&pos, &pool, 123)
            .await
            .expect("dry run should not need wallet");
    }

    #[test]
    fn swap_mix_eps_scales_with_target_when_no_env_override() {
        // Ensure unset override for this unit test.
        // NOTE: env mutation is `unsafe` on Rust 2024 due to data race potential with other tests.
        unsafe { std::env::remove_var("CLMM_SWAP_MIX_DEFICIT_USD_EPS") };
        let small = swap_mix_deficit_usd_epsilon_for_target(10.0);
        let big = swap_mix_deficit_usd_epsilon_for_target(1000.0);
        assert!((0.05..=0.50).contains(&small));
        assert!(big >= small);
        assert!(big <= 0.50);
    }

    #[test]
    fn parse_open_preflight_native_lamports_extracts_require_and_balance() {
        let err = "open preflight exact-plan: insufficient native SOL, require 12000000. Current native balance 500000.";
        let (req, nat) =
            RebalanceExecutor::parse_open_preflight_required_native_lamports(err).expect("parse");
        assert_eq!(req, 12_000_000);
        assert_eq!(nat, 500_000);
    }

    #[test]
    fn stable_mint_for_operational_topup_resolves_non_wsol_leg() {
        let wsol = pubkey!("So11111111111111111111111111111111111111112");
        let dev = pubkey!("BRjpCHtyQLNCo8gqRUr8jtdAj5AjPYQaoqbvcZiHok1k");
        let pool = WhirlpoolState {
            address: String::new(),
            token_mint_a: wsol,
            token_mint_b: dev,
            token_vault_a: Pubkey::default(),
            token_vault_b: Pubkey::default(),
            tick_current: 0,
            tick_spacing: 64,
            sqrt_price: 1 << 64,
            price: Decimal::ONE,
            liquidity: 0,
            fee_rate_bps: 0,
            protocol_fee_rate_bps: 0,
            protocol_fee_owed_a: 0,
            protocol_fee_owed_b: 0,
            fee_growth_global_a: 0,
            fee_growth_global_b: 0,
        };
        assert_eq!(
            RebalanceExecutor::stable_mint_for_operational_sol_topup(&pool).unwrap(),
            dev
        );
    }

    #[test]
    fn estimate_stable_raw_for_sol_deficit_nonzero() {
        let wsol = pubkey!("So11111111111111111111111111111111111111112");
        let dev = pubkey!("BRjpCHtyQLNCo8gqRUr8jtdAj5AjPYQaoqbvcZiHok1k");
        let pool = WhirlpoolState {
            address: String::new(),
            token_mint_a: wsol,
            token_mint_b: dev,
            token_vault_a: Pubkey::default(),
            token_vault_b: Pubkey::default(),
            tick_current: 0,
            tick_spacing: 64,
            sqrt_price: 1 << 64,
            price: Decimal::from_f64_retain(0.087).unwrap(),
            liquidity: 0,
            fee_rate_bps: 0,
            protocol_fee_rate_bps: 0,
            protocol_fee_owed_a: 0,
            protocol_fee_owed_b: 0,
            fee_growth_global_a: 0,
            fee_growth_global_b: 0,
        };
        let raw = RebalanceExecutor::estimate_stable_raw_for_sol_deficit(&pool, 6, 0.01);
        assert!(raw > 0);
    }
}
