//! Strategy executor for automated position management.

use super::{
    Decision, DecisionConfig, DecisionContext, DecisionEngine, RebalanceConfig, RebalanceExecutor,
    RebalanceParams, StrategyMode,
};
use crate::emergency::CircuitBreaker;
use crate::lifecycle::{
    CloseReason, FeesCollectedData, LifecycleTracker, PositionClosedData, RebalanceReason,
};
use crate::monitor::PositionMonitor;
use crate::transaction::TransactionManager;
use crate::wallet::Wallet;
use clmm_lp_protocols::prelude::*;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::interval;
use tracing::{debug, error, info, warn};

/// Configuration for strategy execution.
#[derive(Debug, Clone)]
pub struct ExecutorConfig {
    /// Evaluation interval in seconds.
    pub eval_interval_secs: u64,
    /// Whether to execute decisions automatically.
    pub auto_execute: bool,
    /// Whether to require confirmation before executing.
    pub require_confirmation: bool,
    /// Maximum slippage tolerance (as percentage).
    pub max_slippage_pct: Decimal,
    /// Dry run mode - simulate but don't execute.
    pub dry_run: bool,
}

impl Default for ExecutorConfig {
    fn default() -> Self {
        Self {
            eval_interval_secs: 300, // 5 minutes
            auto_execute: false,     // Require manual confirmation by default
            require_confirmation: true,
            max_slippage_pct: Decimal::new(5, 3), // 0.5%
            dry_run: false,
        }
    }
}

/// Strategy executor for automated position management.
pub struct StrategyExecutor {
    /// Position monitor.
    monitor: Arc<PositionMonitor>,
    /// Decision engine.
    decision_engine: DecisionEngine,
    /// Transaction manager.
    #[allow(dead_code)]
    tx_manager: Arc<TransactionManager>,
    /// Rebalance executor.
    rebalance_executor: RebalanceExecutor,
    /// Circuit breaker.
    circuit_breaker: Arc<CircuitBreaker>,
    /// Lifecycle tracker.
    lifecycle: Arc<LifecycleTracker>,
    /// Wallet for signing.
    wallet: Option<Arc<Wallet>>,
    /// Configuration.
    config: ExecutorConfig,
    /// Running flag.
    running: std::sync::atomic::AtomicBool,
    /// Pool reader for fetching state.
    pool_reader: WhirlpoolReader,
    /// For `RetouchShift`: gating to allow only one retouch per out-of-range episode.
    retouch_armed: Arc<RwLock<HashMap<solana_sdk::pubkey::Pubkey, bool>>>,
    /// Latest optimization profile id (for IL ledger continuity / auditing).
    optimization_run_id: Mutex<Option<String>>,
}

impl StrategyExecutor {
    /// Creates a new strategy executor.
    pub fn new(
        provider: Arc<RpcProvider>,
        monitor: Arc<PositionMonitor>,
        tx_manager: Arc<TransactionManager>,
        config: ExecutorConfig,
    ) -> Self {
        let lifecycle = Arc::new(LifecycleTracker::new());
        let circuit_breaker = Arc::new(CircuitBreaker::default());
        let pool_reader = WhirlpoolReader::new(provider.clone());
        let retouch_armed = Arc::new(RwLock::new(HashMap::new()));

        let mut rebalance_executor = RebalanceExecutor::new(
            provider,
            tx_manager.clone(),
            lifecycle.clone(),
            RebalanceConfig::default(),
        );
        rebalance_executor.set_dry_run(config.dry_run);

        Self {
            monitor,
            decision_engine: DecisionEngine::default(),
            tx_manager,
            rebalance_executor,
            circuit_breaker,
            lifecycle,
            wallet: None,
            config,
            running: std::sync::atomic::AtomicBool::new(false),
            pool_reader,
            retouch_armed,
            optimization_run_id: Mutex::new(None),
        }
    }

    /// Sets the wallet for signing transactions.
    pub fn set_wallet(&mut self, wallet: Arc<Wallet>) {
        self.wallet = Some(wallet.clone());
        self.rebalance_executor.set_wallet(wallet);
    }

    /// Sets the decision engine configuration.
    pub fn set_decision_config(&self, config: DecisionConfig) {
        self.decision_engine.set_config(config);
    }

    /// Sets the current optimization run id used to stamp lifecycle/ledger rows.
    pub fn set_optimization_run_id(&self, run_id: Option<String>) {
        let mut g = self
            .optimization_run_id
            .lock()
            .expect("optimization_run_id lock");
        *g = run_id;
    }

    /// Enables or disables dry run mode.
    pub fn set_dry_run(&mut self, dry_run: bool) {
        self.config.dry_run = dry_run;
        self.rebalance_executor.set_dry_run(dry_run);
    }

    /// Gets the circuit breaker.
    pub fn circuit_breaker(&self) -> &Arc<CircuitBreaker> {
        &self.circuit_breaker
    }

    /// Gets the lifecycle tracker.
    pub fn lifecycle(&self) -> &Arc<LifecycleTracker> {
        &self.lifecycle
    }

    /// Optional JSONL path for IL / rebalance ledger (see `LifecycleTracker::set_il_ledger_path`).
    pub fn set_il_ledger_path(&self, path: Option<std::path::PathBuf>) {
        self.lifecycle.set_il_ledger_path(path);
    }

    /// Partially decrease liquidity on-chain (delegates to [`RebalanceExecutor`]).
    pub async fn execute_partial_decrease_liquidity(
        &self,
        position: &solana_sdk::pubkey::Pubkey,
        pool: &solana_sdk::pubkey::Pubkey,
        liquidity_amount: u128,
    ) -> anyhow::Result<()> {
        self.rebalance_executor
            .execute_partial_decrease(position, pool, liquidity_amount)
            .await
    }

    /// Opens a new Whirlpool position using explicit token caps.
    ///
    /// In dry-run mode this returns the derived position PDA without requiring wallet.
    pub async fn execute_open_position(
        &self,
        pool: &solana_sdk::pubkey::Pubkey,
        tick_lower: i32,
        tick_upper: i32,
        amount_a: u64,
        amount_b: u64,
        slippage_bps: u16,
    ) -> anyhow::Result<solana_sdk::pubkey::Pubkey> {
        self.rebalance_executor
            .execute_open_position(
                pool,
                tick_lower,
                tick_upper,
                amount_a,
                amount_b,
                slippage_bps,
            )
            .await
    }

    /// Collects Whirlpool fees for a given position.
    pub async fn execute_collect_fees_only(
        &self,
        position: &solana_sdk::pubkey::Pubkey,
        pool: &solana_sdk::pubkey::Pubkey,
    ) -> anyhow::Result<()> {
        self.rebalance_executor
            .execute_collect_fees_only(position, pool)
            .await
    }

    /// Closes Whirlpool position by decreasing all liquidity, collecting, and closing NFT.
    pub async fn execute_full_close_only(
        &self,
        position: &solana_sdk::pubkey::Pubkey,
        pool: &solana_sdk::pubkey::Pubkey,
    ) -> anyhow::Result<()> {
        self.rebalance_executor
            .execute_full_close_only(position, pool)
            .await
    }

    /// Starts the strategy execution loop.
    pub async fn start(&self) {
        self.running
            .store(true, std::sync::atomic::Ordering::SeqCst);

        let eval_interval = Duration::from_secs(self.config.eval_interval_secs);
        let mut ticker = interval(eval_interval);

        info!(
            interval_secs = self.config.eval_interval_secs,
            auto_execute = self.config.auto_execute,
            dry_run = self.config.dry_run,
            "Starting strategy executor"
        );

        while self.running.load(std::sync::atomic::Ordering::SeqCst) {
            ticker.tick().await;

            // Check circuit breaker
            if !self.circuit_breaker.is_allowed().await {
                warn!("Circuit breaker open, skipping evaluation");
                continue;
            }

            if let Err(e) = self.evaluate_all().await {
                error!(error = %e, "Strategy evaluation failed");
                self.circuit_breaker.record_failure().await;
            } else {
                self.circuit_breaker.record_success().await;
            }
        }

        info!("Strategy executor stopped");
    }

    /// Stops the strategy execution loop.
    pub fn stop(&self) {
        self.running
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }

    /// Evaluates all monitored positions.
    async fn evaluate_all(&self) -> anyhow::Result<()> {
        let positions = self.monitor.get_positions().await;

        debug!(count = positions.len(), "Evaluating positions");

        for position in positions {
            if let Err(e) = self.evaluate_position(&position).await {
                warn!(
                    position = %position.address,
                    error = %e,
                    "Failed to evaluate position"
                );
            }
        }

        Ok(())
    }

    /// Evaluates a single position.
    async fn evaluate_position(
        &self,
        position: &crate::monitor::MonitoredPosition,
    ) -> anyhow::Result<()> {
        // Fetch current pool state
        let pool = self
            .pool_reader
            .get_pool_state(&position.pool.to_string())
            .await
            .unwrap_or_else(|_| WhirlpoolState {
                address: position.pool.to_string(),
                token_mint_a: solana_sdk::pubkey::Pubkey::default(),
                token_mint_b: solana_sdk::pubkey::Pubkey::default(),
                token_vault_a: solana_sdk::pubkey::Pubkey::default(),
                token_vault_b: solana_sdk::pubkey::Pubkey::default(),
                tick_current: 0,
                tick_spacing: 64,
                sqrt_price: 1 << 64,
                price: Decimal::ONE,
                liquidity: 0,
                fee_rate_bps: 30,
                protocol_fee_rate_bps: 0,
                protocol_fee_owed_a: 0,
                protocol_fee_owed_b: 0,
                fee_growth_global_a: 0,
                fee_growth_global_b: 0,
            });

        // Calculate hours since last rebalance from lifecycle
        let hours_since_rebalance = self
            .calculate_hours_since_rebalance(&position.address)
            .await;

        let retouch_armed =
            if self.decision_engine.config().strategy_mode == StrategyMode::RetouchShift {
                let mut map = self.retouch_armed.write().await;
                let entry = map.entry(position.address).or_insert(true);
                if position.in_range {
                    *entry = true;
                }
                Some(*entry)
            } else {
                None
            };

        let context = DecisionContext {
            position: position.clone(),
            pool: pool.clone(),
            hours_since_rebalance,
            retouch_armed,
        };

        let decision = self.decision_engine.decide(&context);

        if decision.requires_transaction() {
            info!(
                position = %position.address,
                decision = %decision.description(),
                dry_run = self.config.dry_run,
                "Decision requires action"
            );

            if self.config.auto_execute {
                self.execute_decision(position, &decision, &pool).await?;
            }
        }

        Ok(())
    }

    /// Calculates hours since last rebalance.
    async fn calculate_hours_since_rebalance(&self, position: &solana_sdk::pubkey::Pubkey) -> u64 {
        let events = self.lifecycle.get_events(position).await;

        // Find the last rebalance event
        for event in events.iter().rev() {
            if event.event_type == crate::lifecycle::LifecycleEventType::Rebalanced {
                let duration = chrono::Utc::now() - event.timestamp;
                return duration.num_hours().max(0) as u64;
            }
        }

        // If no rebalance, use position open time
        if let Some(summary) = self.lifecycle.get_summary(position).await {
            let duration = chrono::Utc::now() - summary.opened_at;
            return duration.num_hours().max(0) as u64;
        }

        // Default to a large value to allow rebalancing
        u64::MAX
    }

    /// Executes a decision.
    async fn execute_decision(
        &self,
        position: &crate::monitor::MonitoredPosition,
        decision: &Decision,
        pool: &WhirlpoolState,
    ) -> anyhow::Result<()> {
        info!(
            position = %position.address,
            decision = %decision.description(),
            "Executing decision"
        );

        match decision {
            Decision::Hold => {
                // Nothing to do
            }
            Decision::Rebalance {
                new_tick_lower,
                new_tick_upper,
            } => {
                // Update retouch gate once we decide to rebalance for RetouchShift.
                if self.decision_engine.config().strategy_mode == StrategyMode::RetouchShift {
                    let mut map = self.retouch_armed.write().await;
                    map.insert(position.address, false);
                }

                let reason = match self.decision_engine.config().strategy_mode {
                    StrategyMode::RetouchShift => RebalanceReason::RetouchShift,
                    StrategyMode::Periodic => RebalanceReason::Periodic,
                    StrategyMode::OorRecenter => RebalanceReason::RangeExit,
                    StrategyMode::Threshold => {
                        if !position.in_range {
                            RebalanceReason::RangeExit
                        } else {
                            RebalanceReason::Optimization
                        }
                    }
                    StrategyMode::StaticRange => RebalanceReason::Manual,
                    StrategyMode::IlLimit => {
                        if !position.in_range {
                            RebalanceReason::RangeExit
                        } else {
                            RebalanceReason::ILThreshold
                        }
                    }
                };
                let params = RebalanceParams {
                    position: position.address,
                    pool: position.pool,
                    current_tick_lower: position.on_chain.tick_lower,
                    current_tick_upper: position.on_chain.tick_upper,
                    new_tick_lower: *new_tick_lower,
                    new_tick_upper: *new_tick_upper,
                    current_liquidity: position.on_chain.liquidity,
                    pool_tick_current: pool.tick_current,
                    pool_sqrt_price: pool.sqrt_price,
                    reason,
                    current_il_pct: position.pnl.il_pct,
                    amount_a_before: None,
                    amount_b_before: None,
                    price_ab_before: Some(pool.price),
                    amount_a_after: None,
                    amount_b_after: None,
                    price_ab_after: None,
                    optimization_run_id: self
                        .optimization_run_id
                        .lock()
                        .expect("optimization_run_id lock")
                        .clone(),
                };

                let result = self.rebalance_executor.execute(params).await;

                if !result.success
                    && let Some(err) = result.error
                {
                    error!(error = %err, "Rebalance failed");
                }

                // Keep the monitor set in sync with the actual rebalance outcome:
                // - old position is closed
                // - new position is opened
                if result.success {
                    let old_addr = position.address;
                    self.monitor.remove_position(&old_addr).await;

                    if let Some(new_pos) = result.new_position {
                        if let Err(e) = self.monitor.add_position(&new_pos.to_string()).await {
                            warn!(
                                error = %e,
                                old_position = %old_addr,
                                new_position = %new_pos,
                                "Failed to add new position to monitor"
                            );
                        }
                    }

                    // Retouch gate housekeeping (avoid unbounded growth).
                    if self.decision_engine.config().strategy_mode == StrategyMode::RetouchShift {
                        let mut m = self.retouch_armed.write().await;
                        m.remove(&old_addr);
                    }
                }
            }
            Decision::Close => {
                let addr = position.address;
                let pool_pk = position.pool;
                let duration_hours = self
                    .lifecycle
                    .get_summary(&addr)
                    .await
                    .map(|s| (chrono::Utc::now() - s.opened_at).num_hours().max(0) as u64)
                    .unwrap_or(0);

                match self
                    .rebalance_executor
                    .execute_full_close_only(&addr, &pool_pk)
                    .await
                {
                    Ok(()) => {
                        self.monitor.remove_position(&addr).await;
                        self.lifecycle
                            .record_position_closed(
                                addr,
                                pool_pk,
                                PositionClosedData {
                                    liquidity_removed: position.on_chain.liquidity,
                                    amount_a: 0,
                                    amount_b: 0,
                                    price_ab: Some(pool.price),
                                    total_fees_a: position.on_chain.fees_owed_a,
                                    total_fees_b: position.on_chain.fees_owed_b,
                                    final_pnl_usd: position.pnl.net_pnl_usd,
                                    final_pnl_pct: position.pnl.net_pnl_pct,
                                    total_il_pct: position.pnl.il_pct,
                                    duration_hours,
                                    reason: CloseReason::ILThreshold,
                                },
                            )
                            .await;
                    }
                    Err(e) => error!(error = %e, "Close position failed"),
                }
            }
            Decision::IncreaseLiquidity { amount } => {
                warn!(
                    amount = %amount,
                    "IncreaseLiquidity is not emitted by current strategy modes; no-op"
                );
            }
            Decision::DecreaseLiquidity { amount } => {
                let Some(to_remove) =
                    clamp_decimal_liquidity_to_u128(&amount, position.on_chain.liquidity)
                else {
                    warn!(
                        amount = %amount,
                        "DecreaseLiquidity: invalid or non-positive amount, skipping"
                    );
                    return Ok(());
                };
                if to_remove == 0 {
                    return Ok(());
                }
                if let Err(e) = self
                    .rebalance_executor
                    .execute_partial_decrease(&position.address, &position.pool, to_remove)
                    .await
                {
                    error!(error = %e, "Decrease liquidity failed");
                }
            }
            Decision::CollectFees => {
                if let Err(e) = self
                    .rebalance_executor
                    .execute_collect_fees_only(&position.address, &position.pool)
                    .await
                {
                    error!(error = %e, "Collect fees failed");
                } else {
                    self.lifecycle
                        .record_fees_collected(
                            position.address,
                            position.pool,
                            FeesCollectedData {
                                fees_a: position.on_chain.fees_owed_a,
                                fees_b: position.on_chain.fees_owed_b,
                                fees_usd: position.pnl.fees_usd,
                            },
                        )
                        .await;
                }
            }
        }

        Ok(())
    }
}

/// Converts a strategy `Decimal` liquidity delta to `u128`, truncated and capped by on-chain liquidity.
fn clamp_decimal_liquidity_to_u128(amount: &Decimal, max_liquidity: u128) -> Option<u128> {
    let t = amount.trunc();
    if t <= Decimal::ZERO {
        return None;
    }
    let u = t.to_u128()?;
    Some(u.min(max_liquidity))
}

#[cfg(test)]
mod clamp_tests {
    use super::*;

    #[test]
    fn clamps_to_max() {
        let d = Decimal::from(500u64);
        assert_eq!(clamp_decimal_liquidity_to_u128(&d, 100), Some(100));
    }

    #[test]
    fn rejects_non_positive() {
        assert_eq!(clamp_decimal_liquidity_to_u128(&Decimal::ZERO, 100), None);
        assert_eq!(
            clamp_decimal_liquidity_to_u128(&Decimal::new(-1, 0), 100),
            None
        );
    }
}
