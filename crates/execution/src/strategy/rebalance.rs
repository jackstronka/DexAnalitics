//! Rebalancing execution logic.

use crate::lifecycle::{FeesCollectedData, LifecycleTracker, RebalanceData, RebalanceReason};
use crate::transaction::TransactionManager;
use crate::wallet::Wallet;
use anyhow::Context;
use clmm_lp_protocols::prelude::*;
use rust_decimal::Decimal;
use rust_decimal::MathematicalOps;
use rust_decimal::prelude::ToPrimitive;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Signature;
use spl_token::solana_program::program_pack::Pack;
use spl_token::state::Account as SplTokenAccount;
use spl_token::state::Mint as SplMint;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tracing::{debug, error, info, warn};

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
        .unwrap_or(6)
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

/// Synthetic **relative** USD prices: A = 1, B = B_ui/A_ui from pool, so wallet notional matches spot mix scale.
fn synthetic_prices_for_deposit_quote(pool_price: Decimal, dec_a: u8, dec_b: u8) -> (f64, f64) {
    let exp = i32::from(dec_a) - i32::from(dec_b);
    let b_per_a_ui = pool_price * Decimal::from(10).powi(i64::from(exp));
    let pb = b_per_a_ui.to_f64().unwrap_or(1.0).max(1e-18);
    (1.0_f64, pb)
}

fn balances_cover_deposit_quote(wa: u64, wb: u64, q: &DepositBudgetQuote) -> bool {
    let tol_a = (q.amount_a / 100).max(1);
    let tol_b = (q.amount_b / 100).max(1);
    wa >= q.amount_a.saturating_sub(tol_a) && wb >= q.amount_b.saturating_sub(tol_b)
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
        Self {
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
    pub reason: RebalanceReason,
    pub closed_position_nft: Pubkey,
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
        }
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

    fn require_wallet(&self) -> anyhow::Result<Arc<Wallet>> {
        self.wallet
            .lock()
            .map_err(|_| anyhow::anyhow!("wallet mutex poisoned"))?
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Wallet not set on RebalanceExecutor"))
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
        self.collect_fees(position, pool).await
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
        self.close_position(position, pool).await
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

    /// In-pool Orca swaps (ExactIn) until wallet balances match [`quote_deposit_budget_in_range`]
    /// for the new tick range — same building block as API `quote-open-budget` + swap-before-open.
    ///
    /// Uses **synthetic** relative prices from `pool.price` and mint decimals (no paid price API).
    /// Returns the number of swap transactions submitted.
    async fn ensure_swap_mix_for_rebalance_open(
        &self,
        pool: &Pubkey,
        tick_lower: i32,
        tick_upper: i32,
        owner: &Pubkey,
        log_position: &Pubkey,
    ) -> anyhow::Result<u32> {
        if self.is_dry_run() {
            return Ok(0);
        }
        let max_rounds = swap_mix_max_rounds();
        let mut swaps: u32 = 0;
        const MIN_SWAP: u64 = 1;
        const AMOUNT_IN_BUFFER_PCT: f64 = 1.02; // small buffer for pool price moves + fees
        const SPEND_CAP_PCT: f64 = 0.92; // avoid spending 100% of a leg due to rounding/fees

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
            if wa == 0 && wb == 0 {
                error!(
                    op = "orca_rebalance",
                    stage = "swap_mix",
                    pool = %pool,
                    round,
                    "swap-mix: wallet has zero of both tokens"
                );
                anyhow::bail!("wallet has zero of both tokens; cannot swap or open");
            }
            let (pa, pb) = synthetic_prices_for_deposit_quote(pool_state.price, dec_a, dec_b);
            let a_ui = wa as f64 / 10f64.powi(i32::from(dec_a));
            let b_ui = wb as f64 / 10f64.powi(i32::from(dec_b));
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
            let target_usd = wallet_notional * 0.995;
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

            if deficit_a > 0 && wb > MIN_SWAP {
                // Swap B -> A to cover deficit in A (estimate using synthetic USD prices).
                let deficit_a_ui = deficit_a as f64 / 10f64.powi(i32::from(dec_a));
                let usd_need = (deficit_a_ui * pa).max(0.0);
                let mut fund_b_ui = if pb > 0.0 { (usd_need / pb) * AMOUNT_IN_BUFFER_PCT } else { 0.0 };
                if !fund_b_ui.is_finite() || fund_b_ui <= 0.0 {
                    fund_b_ui = (wb as f64 / 10f64.powi(i32::from(dec_b))) * 0.5;
                }
                let raw_est = (fund_b_ui * 10f64.powi(i32::from(dec_b))).round() as i128;
                let max_raw = ((wb as f64) * SPEND_CAP_PCT).floor() as i128;
                let amount_in = raw_est
                    .clamp(i128::from(MIN_SWAP), max_raw.max(i128::from(MIN_SWAP)))
                    .min(i128::from(wb)) as u64;
                clmm_lp_protocols::ledger::tx_lifecycle::try_append_bot_diagnostic_row(
                    self.provider.as_ref(),
                    "bot_swap_mix_round",
                    "swap_mix",
                    Some(*pool),
                    Some(*log_position),
                    None,
                    serde_json::json!({
                        "round": round,
                        "max_rounds": max_rounds,
                        "leg": "B_to_A",
                        "amount_in": amount_in,
                        "wa": wa,
                        "wb": wb,
                        "need_a": q.amount_a,
                        "need_b": q.amount_b,
                        "deficit_a": deficit_a,
                        "deficit_b": deficit_b,
                        "tick_lower": tick_lower,
                        "tick_upper": tick_upper,
                        "tick_current": pool_state.tick_current,
                        "price": pool_state.price,
                        "target_usd": target_usd,
                        "pa": pa,
                        "pb": pb,
                        "amount_in_est_mode": "deficit_usd"
                    }),
                )
                .await;
                info!(
                    round,
                    amount_in, "rebalance: swap ExactIn token B toward mix for open"
                );
                self.execute_swap_exact_in(
                    pool,
                    &pool_state.token_mint_b,
                    amount_in,
                    self.config.max_slippage_bps,
                    None,
                )
                .await
                .map_err(|e| {
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
                    e
                })?;
                swaps += 1;
                continue;
            }
            if deficit_b > 0 && wa > MIN_SWAP {
                // Swap A -> B to cover deficit in B (estimate using synthetic USD prices).
                let deficit_b_ui = deficit_b as f64 / 10f64.powi(i32::from(dec_b));
                let usd_need = (deficit_b_ui * pb).max(0.0);
                let mut fund_a_ui = if pa > 0.0 { (usd_need / pa) * AMOUNT_IN_BUFFER_PCT } else { 0.0 };
                if !fund_a_ui.is_finite() || fund_a_ui <= 0.0 {
                    fund_a_ui = (wa as f64 / 10f64.powi(i32::from(dec_a))) * 0.5;
                }
                let raw_est = (fund_a_ui * 10f64.powi(i32::from(dec_a))).round() as i128;
                let max_raw = ((wa as f64) * SPEND_CAP_PCT).floor() as i128;
                let amount_in = raw_est
                    .clamp(i128::from(MIN_SWAP), max_raw.max(i128::from(MIN_SWAP)))
                    .min(i128::from(wa)) as u64;
                clmm_lp_protocols::ledger::tx_lifecycle::try_append_bot_diagnostic_row(
                    self.provider.as_ref(),
                    "bot_swap_mix_round",
                    "swap_mix",
                    Some(*pool),
                    Some(*log_position),
                    None,
                    serde_json::json!({
                        "round": round,
                        "max_rounds": max_rounds,
                        "leg": "A_to_B",
                        "amount_in": amount_in,
                        "wa": wa,
                        "wb": wb,
                        "need_a": q.amount_a,
                        "need_b": q.amount_b,
                        "deficit_a": deficit_a,
                        "deficit_b": deficit_b,
                        "tick_lower": tick_lower,
                        "tick_upper": tick_upper,
                        "tick_current": pool_state.tick_current,
                        "price": pool_state.price,
                        "target_usd": target_usd,
                        "pa": pa,
                        "pb": pb,
                        "amount_in_est_mode": "deficit_usd"
                    }),
                )
                .await;
                info!(
                    round,
                    amount_in, "rebalance: swap ExactIn token A toward mix for open"
                );
                self.execute_swap_exact_in(
                    pool,
                    &pool_state.token_mint_a,
                    amount_in,
                    self.config.max_slippage_bps,
                    None,
                )
                .await
                .map_err(|e| {
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
                    e
                })?;
                swaps += 1;
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
            None,
            serde_json::json!({
                "max_rounds": max_rounds,
                "swaps_done": swaps,
                "tick_lower": tick_lower,
                "tick_upper": tick_upper
            }),
        )
        .await;
        anyhow::bail!(
            "swap mix: exhausted {} rounds without matching deposit quote",
            max_rounds
        );
    }

    /// Swap-mix alignment + `open_position` retries (shared by full rebalance and incomplete recovery).
    async fn open_new_range_with_wallet_mix(
        &self,
        pool: &Pubkey,
        new_tick_lower: i32,
        new_tick_upper: i32,
        pool_state: &WhirlpoolState,
        amount_a_before_calc: u64,
        amount_b_before_calc: u64,
        log_position: &Pubkey,
    ) -> Result<(Pubkey, u32), String> {
        let Some(owner) = self.wallet_pubkey() else {
            return Err(
                "wallet missing on RebalanceExecutor after close — cannot open new position"
                    .to_string(),
            );
        };

        let swap_rounds = self
            .ensure_swap_mix_for_rebalance_open(
                pool,
                new_tick_lower,
                new_tick_upper,
                &owner,
                log_position,
            )
            .await
            .map_err(|e| e.to_string())?;

        let max_open_attempts = rebalance_open_max_attempts();
        let mut new_position: Option<Pubkey> = None;
        let mut last_open_err: Option<String> = None;
        let mut last_cap_a: u64 = 0;
        let mut last_cap_b: u64 = 0;

        for attempt in 1..=max_open_attempts {
            let mut cap_a =
                spl_token_balance_raw(self.provider.as_ref(), &owner, &pool_state.token_mint_a)
                    .await;
            let mut cap_b =
                spl_token_balance_raw(self.provider.as_ref(), &owner, &pool_state.token_mint_b)
                    .await;
            if cap_a == 0 && cap_b == 0 {
                cap_a = amount_a_before_calc.max(1);
                cap_b = amount_b_before_calc.max(1);
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
            if attempt == 1 {
                info!(
                    position = %log_position,
                    cap_a,
                    cap_b,
                    mint_a = %pool_state.token_mint_a,
                    mint_b = %pool_state.token_mint_b,
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
                .open_position(pool, new_tick_lower, new_tick_upper, cap_a, cap_b)
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
                    last_open_err = Some(e.to_string());
                    warn!(
                        op = "orca_rebalance",
                        stage = "open_position",
                        attempt,
                        max_attempts = max_open_attempts,
                        cap_a,
                        cap_b,
                        new_tick_lower,
                        new_tick_upper,
                        error = %e,
                        "open_position failed"
                    );
                    if attempt < max_open_attempts {
                        tokio::time::sleep(Duration::from_millis(250 * u64::from(attempt))).await;
                    }
                }
            }
        }

        let new_position = match new_position {
            Some(p) => p,
            None => {
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

        Ok((new_position, swap_rounds))
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
        };

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

        // Step 1: Collect fees if configured
        if self.config.collect_fees_first {
            match self.collect_fees(&params.position, &params.pool).await {
                Ok(fees) => {
                    result.fees_collected = Some(fees);
                    result.tx_cost_lamports += 5000; // Approximate

                    // Record in lifecycle
                    self.lifecycle
                        .record_fees_collected(
                            params.position,
                            params.pool,
                            FeesCollectedData {
                                fees_a: fees.0,
                                fees_b: fees.1,
                                fees_usd: Decimal::ZERO, // Would need price oracle
                            },
                        )
                        .await;
                }
                Err(e) => {
                    warn!(
                        op = "orca_rebalance",
                        stage = "collect_fees",
                        position = %params.position,
                        pool = %params.pool,
                        error = %e,
                        "Failed to collect fees, continuing"
                    );
                }
            }
        }
        // Step 2: Close old position (includes decreasing all liquidity + collecting remaining fees)
        result.liquidity_removed = params.current_liquidity;
        if let Err(e) = self.close_position(&params.position, &params.pool).await {
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
        result.old_position_closed_on_chain = true;
        result.tx_cost_lamports += 5000;

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
                params.new_tick_lower,
                params.new_tick_upper,
                &pool_state,
                amount_a_before_calc,
                amount_b_before_calc,
                &params.position,
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

        match self
            .open_new_range_with_wallet_mix(
                &p.pool,
                p.new_tick_lower,
                p.new_tick_upper,
                &pool_state,
                1,
                1,
                &p.closed_position_nft,
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
                    new_tick_lower: p.new_tick_lower,
                    new_tick_upper: p.new_tick_upper,
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
    ) -> anyhow::Result<()> {
        if self.is_dry_run() {
            info!("Dry run: would collect fees");
            return Ok(());
        }
        self.collect_fees(position, pool).await?;
        Ok(())
    }

    /// Full on-chain close (decrease all + collect + close NFT). Used by `Decision::Close`.
    pub async fn execute_full_close_only(
        &self,
        position: &Pubkey,
        pool: &Pubkey,
    ) -> anyhow::Result<()> {
        if self.is_dry_run() {
            info!("Dry run: would close position");
            return Ok(());
        }
        self.close_position(position, pool).await
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
    async fn collect_fees(&self, position: &Pubkey, pool: &Pubkey) -> anyhow::Result<(u64, u64)> {
        let wallet = self.require_wallet()?;
        let orca = WhirlpoolExecutor::new(self.provider.clone());

        let payer = wallet.keypair();
        let res = orca.collect_fees(position, pool, payer).await?;
        self.ensure_execution_success("collect_fees", &res, Some(*pool), Some(*position), None)
            .await?;

        // We currently don't parse fee amounts from on-chain state in this executor.
        // Returning (0,0) keeps lifecycle wiring intact while we tighten accounting later.
        debug!(position = %position, "Collect fees submitted");
        Ok((0, 0))
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
        )
        .await?;
        debug!(
            position = %position,
            liquidity = liquidity_amount,
            "Decrease liquidity submitted"
        );
        Ok(())
    }

    /// Closes a position.
    async fn close_position(&self, position: &Pubkey, pool: &Pubkey) -> anyhow::Result<()> {
        let wallet = self.require_wallet()?;
        let orca = WhirlpoolExecutor::new(self.provider.clone());

        let payer = wallet.keypair();
        let res = orca.close_position(position, pool, payer, None).await?;
        self.ensure_execution_success("close_position", &res, Some(*pool), Some(*position), None)
            .await?;
        debug!(position = %position, "Close position submitted");
        Ok(())
    }

    /// Opens a new position.
    async fn open_position(
        &self,
        _pool: &Pubkey,
        tick_lower: i32,
        tick_upper: i32,
        cap_a: u64,
        cap_b: u64,
    ) -> anyhow::Result<Pubkey> {
        let (p, _, _) = self
            .open_position_with_caps(
                _pool,
                tick_lower,
                tick_upper,
                cap_a,
                cap_b,
                self.config.max_slippage_bps,
                None,
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
        let res = orca
            .swap_exact_in(*pool, *specified_mint, amount_in, slippage_bps, payer)
            .await?;
        let sig = res.signature;
        self.ensure_execution_success("swap_exact_in", &res, Some(*pool), None, ledger_session_id)
            .await?;
        Ok(Some(sig))
    }

    /// Opens a new position with explicit token caps and slippage.
    ///
    /// In dry-run mode returns the derived Whirlpool position PDA without requiring wallet.
    /// Returns `(position_pda, effective_tick_lower, effective_tick_upper)`.
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
        self.ensure_execution_success(
            "open_full_range_position",
            &res,
            Some(*pool),
            None,
            ledger_session_id,
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

    async fn open_position_with_caps(
        &self,
        pool: &Pubkey,
        tick_lower: i32,
        tick_upper: i32,
        amount_a: u64,
        amount_b: u64,
        slippage_bps: u16,
        ledger_session_id: Option<String>,
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
        self.ensure_execution_success("open_position", &res, Some(*pool), None, ledger_session_id)
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

    async fn ensure_execution_success(
        &self,
        op_name: &str,
        result: &clmm_lp_protocols::orca::executor::ExecutionResult,
        pool: Option<Pubkey>,
        position: Option<Pubkey>,
        ledger_session_id: Option<String>,
    ) -> anyhow::Result<()> {
        validate_execution_result(op_name, result)?;

        if result.success {
            let fee_payer = self
                .wallet
                .lock()
                .ok()
                .and_then(|g| g.as_ref().map(|w| w.pubkey()));
            if let Some(fee_payer) = fee_payer {
                clmm_lp_protocols::ledger::tx_lifecycle::try_append_rebalance_executor_tx_cost(
                    self.provider.as_ref(),
                    &fee_payer,
                    &result.signature,
                    op_name,
                    pool,
                    position,
                    result.created_position,
                    ledger_session_id.clone(),
                )
                .await;

                if op_name == "close_position" {
                    if let (Some(pool_pk), Some(pos_pk)) = (pool, position) {
                        clmm_lp_protocols::ledger::position_registry::try_append_registry_close(
                            self.provider.as_ref(),
                            "orca_bot",
                            &pos_pk,
                            &pool_pk,
                            &fee_payer,
                            &result.signature,
                            ledger_session_id.clone(),
                        )
                        .await;
                    }
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
                    )
                    .await;
                }
            }
        }

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
}
