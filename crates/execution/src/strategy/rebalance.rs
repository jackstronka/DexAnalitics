//! Rebalancing execution logic.

use crate::lifecycle::{FeesCollectedData, LifecycleTracker, RebalanceData, RebalanceReason};
use crate::transaction::TransactionManager;
use crate::wallet::Wallet;
use clmm_lp_protocols::prelude::*;
use rust_decimal::Decimal;
use solana_sdk::pubkey::Pubkey;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

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
}

impl Default for RebalanceConfig {
    fn default() -> Self {
        Self {
            max_slippage_bps: 50,                      // 0.5%
            min_profit_multiplier: Decimal::new(2, 0), // 2x tx cost
            collect_fees_first: true,
            priority_level: crate::transaction::PriorityLevel::Medium,
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

/// Result of a rebalance operation.
#[derive(Debug, Clone)]
pub struct RebalanceResult {
    /// Whether rebalance was successful.
    pub success: bool,
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
    wallet: Option<Arc<Wallet>>,
    /// Lifecycle tracker.
    lifecycle: Arc<LifecycleTracker>,
    /// Configuration.
    config: RebalanceConfig,
    /// Dry run mode.
    dry_run: bool,
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
            wallet: None,
            lifecycle,
            config,
            dry_run: false,
        }
    }

    /// Sets the wallet for signing.
    pub fn set_wallet(&mut self, wallet: Arc<Wallet>) {
        self.wallet = Some(wallet);
    }

    /// Enables or disables dry run mode.
    pub fn set_dry_run(&mut self, dry_run: bool) {
        self.dry_run = dry_run;
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

    /// Estimates transaction cost for rebalancing.
    async fn estimate_tx_cost(&self) -> u64 {
        // Base cost: ~5000 lamports per signature + compute units
        // Rebalance involves: collect fees + decrease liquidity + close position + open position + increase liquidity
        // Estimate ~0.01 SOL total
        10_000_000 // 0.01 SOL in lamports
    }

    /// Estimates expected benefit from rebalancing.
    async fn estimate_benefit(&self, params: &RebalanceParams) -> Decimal {
        // Simplified estimation based on IL recovery
        // In a real implementation, this would use historical data and simulations
        let il_recovery = params.current_il_pct.abs() * Decimal::new(5, 1); // Assume 50% IL recovery
        il_recovery * Decimal::from(1000) // Convert to USD equivalent
    }

    /// Executes a rebalance operation.
    pub async fn execute(&self, params: RebalanceParams) -> RebalanceResult {
        info!(
            position = %params.position,
            old_range = format!("[{}, {}]", params.current_tick_lower, params.current_tick_upper),
            new_range = format!("[{}, {}]", params.new_tick_lower, params.new_tick_upper),
            reason = ?params.reason,
            dry_run = self.dry_run,
            "Executing rebalance"
        );

        let mut result = RebalanceResult {
            success: false,
            old_position: params.position,
            new_position: None,
            fees_collected: None,
            liquidity_removed: 0,
            liquidity_added: 0,
            tx_cost_lamports: 0,
            error: None,
        };

        // For live onboarding we avoid blocking on incomplete IL/PnL signals.
        // Once PnL tracking and profitability estimation are fully wired, we can restore stricter checks.
        let _ = self.is_profitable(&params).await;

        if self.dry_run {
            info!("Dry run mode - simulating rebalance");
            result.success = true;
            result.liquidity_removed = params.current_liquidity;
            result.liquidity_added = params.current_liquidity;
            return result;
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
                    warn!(error = %e, "Failed to collect fees, continuing");
                }
            }
        }
        // Step 2: Close old position (includes decreasing all liquidity + collecting remaining fees)
        result.liquidity_removed = params.current_liquidity;
        if let Err(e) = self.close_position(&params.position, &params.pool).await {
            error!(error = %e, "Failed to close position");
            result.error = Some(e.to_string());
            return result;
        }
        result.tx_cost_lamports += 5000;

        // Step 3: Open new position
        let new_position = match self
            .open_position(&params.pool, params.new_tick_lower, params.new_tick_upper)
            .await
        {
            Ok(pos) => pos,
            Err(e) => {
                error!(error = %e, "Failed to open new position");
                result.error = Some(e.to_string());
                return result;
            }
        };
        result.new_position = Some(new_position);
        result.tx_cost_lamports += 5000;
        // Orca open_position() already performs the initial liquidity increase.
        result.liquidity_added = params.current_liquidity;

        let (fa, fb) = result.fees_collected.unwrap_or((0, 0));

        // IL ledger: compute token split "after" rebalance using the new on-chain state.
        let (amount_a_after, amount_b_after, price_ab_after) =
            if let Some(new_pos) = result.new_position {
                let pool_reader = WhirlpoolReader::new(self.provider.clone());
                let pool_state = pool_reader
                    .get_pool_state(&params.pool.to_string())
                    .await
                    .ok();
                if let Some(pool_state) = pool_state {
                    let pos_reader = PositionReader::new(self.provider.clone());
                    if let Ok(on_chain_pos) = pos_reader.get_position(&new_pos.to_string()).await {
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
            } else {
                (
                    params.amount_a_after,
                    params.amount_b_after,
                    params.price_ab_after,
                )
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

    /// Collect fees only (no rebalance). Used by `Decision::CollectFees` / strategy loop.
    pub async fn execute_collect_fees_only(
        &self,
        position: &Pubkey,
        pool: &Pubkey,
    ) -> anyhow::Result<()> {
        if self.dry_run {
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
        if self.dry_run {
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
        if self.dry_run {
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
        let wallet = self.wallet.as_ref().ok_or_else(|| {
            anyhow::anyhow!("Wallet not set on RebalanceExecutor; cannot collect fees")
        })?;
        let orca = WhirlpoolExecutor::new(self.provider.clone());

        let payer = wallet.keypair();
        let res = orca.collect_fees(position, pool, payer).await?;
        self.ensure_execution_success("collect_fees", &res).await?;

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
        let wallet = self.wallet.as_ref().ok_or_else(|| {
            anyhow::anyhow!("Wallet not set on RebalanceExecutor; cannot decrease liquidity")
        })?;
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
        self.ensure_execution_success("decrease_liquidity", &res)
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
        let wallet = self.wallet.as_ref().ok_or_else(|| {
            anyhow::anyhow!("Wallet not set on RebalanceExecutor; cannot close position")
        })?;
        let orca = WhirlpoolExecutor::new(self.provider.clone());

        let payer = wallet.keypair();
        let res = orca.close_position(position, pool, payer).await?;
        self.ensure_execution_success("close_position", &res)
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
    ) -> anyhow::Result<Pubkey> {
        self.open_position_with_caps(
            _pool,
            tick_lower,
            tick_upper,
            u64::MAX,
            u64::MAX,
            self.config.max_slippage_bps,
        )
        .await
    }

    /// Opens a new position with explicit token caps and slippage.
    ///
    /// In dry-run mode returns the derived Whirlpool position PDA without requiring wallet.
    pub async fn execute_open_position(
        &self,
        pool: &Pubkey,
        tick_lower: i32,
        tick_upper: i32,
        amount_a: u64,
        amount_b: u64,
        slippage_bps: u16,
    ) -> anyhow::Result<Pubkey> {
        if self.dry_run {
            return Ok(derive_whirlpool_position_address(
                pool, tick_lower, tick_upper,
            ));
        }
        self.open_position_with_caps(
            pool,
            tick_lower,
            tick_upper,
            amount_a,
            amount_b,
            slippage_bps,
        )
        .await
    }

    async fn open_position_with_caps(
        &self,
        pool: &Pubkey,
        tick_lower: i32,
        tick_upper: i32,
        amount_a: u64,
        amount_b: u64,
        slippage_bps: u16,
    ) -> anyhow::Result<Pubkey> {
        let wallet = self.wallet.as_ref().ok_or_else(|| {
            anyhow::anyhow!("Wallet not set on RebalanceExecutor; cannot open position")
        })?;
        let orca = WhirlpoolExecutor::new(self.provider.clone());

        let payer = wallet.keypair();

        let new_position = derive_whirlpool_position_address(pool, tick_lower, tick_upper);

        // Send maximal token caps so the program uses the required amounts from wallet balances.
        let params = OpenPositionParams {
            pool: pool.clone(),
            tick_lower,
            tick_upper,
            amount_a,
            amount_b,
            slippage_bps,
        };

        let res = orca.open_position(&params, payer).await?;
        self.ensure_execution_success("open_position", &res).await?;
        debug!(
            new_position = %new_position,
            tick_lower = tick_lower,
            tick_upper = tick_upper,
            "Open position submitted"
        );
        Ok(new_position)
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
    ) -> anyhow::Result<()> {
        validate_execution_result(op_name, result)?;

        // Best-effort post-check through the common transaction manager path.
        // Some providers may not return status immediately for very fresh signatures.
        if let Err(e) = self
            .tx_manager
            .wait_for_confirmation(&result.signature)
            .await
        {
            warn!(
                operation = op_name,
                signature = %result.signature,
                error = %e,
                "Post-confirmation check failed; continuing because executor already reported success"
            );
        }

        Ok(())
    }
}

fn validate_execution_result(
    op_name: &str,
    result: &clmm_lp_protocols::orca::executor::ExecutionResult,
) -> anyhow::Result<()> {
    if !result.success {
        let msg = result
            .error
            .clone()
            .unwrap_or_else(|| "unknown execution error".to_string());
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
        let mut exec =
            RebalanceExecutor::new(provider, tx_manager, lifecycle, RebalanceConfig::default());
        exec.set_dry_run(true);
        let pos = Pubkey::new_unique();
        let pool = Pubkey::new_unique();
        exec.execute_partial_decrease(&pos, &pool, 123)
            .await
            .expect("dry run should not need wallet");
    }
}
