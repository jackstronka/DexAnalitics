//! Strategy executor for automated position management.

use super::pending_open;
use super::{
    Decision, DecisionConfig, DecisionContext, DecisionEngine, RebalanceConfig, RebalanceExecutor,
    RebalanceParams, RecoverOpenParams, StrategyMode,
};
use crate::alerts::{Alert, AlertLevel, AlertType, MultiNotifier};
use crate::emergency::CircuitBreaker;
use crate::lifecycle::{
    CloseReason, FeesCollectedData, LifecycleTracker, PositionClosedData, RebalanceReason,
};
use crate::monitor::PositionMonitor;
use crate::transaction::TransactionManager;
use crate::wallet::Wallet;
use clmm_lp_domain::prelude::{CheckpointSource, PositionFeeCheckpoint, PositionTruthMode};
use clmm_lp_protocols::prelude::*;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use solana_sdk::pubkey::Pubkey;
use std::collections::{HashMap, HashSet};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::sync::{Mutex, RwLock};
use tokio::time::interval;
use tracing::{debug, error, info, warn};

/// Snapshot of the latest evaluation outcome for a position.
#[derive(Debug, Clone)]
pub struct PositionEvalSnapshot {
    pub ts_utc: String,
    pub in_range: bool,
    pub pool_tick_current: i32,
    pub decision: String,
    pub requires_transaction: bool,
    pub auto_execute: bool,
    /// `None` when unknown (no lifecycle summary) — avoid showing `u64::MAX` in UIs.
    pub hours_since_rebalance: Option<u64>,
    /// `None` when unknown (no lifecycle summary).
    pub minutes_since_rebalance: Option<u64>,
}

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
    /// Fee accounting mode: existing heuristic or Tier3 position-truth.
    pub fee_mode: PositionTruthMode,
}

impl Default for ExecutorConfig {
    fn default() -> Self {
        Self {
            eval_interval_secs: 300, // 5 minutes
            auto_execute: false,     // Require manual confirmation by default
            require_confirmation: true,
            max_slippage_pct: Decimal::new(5, 3), // 0.5%
            dry_run: false,
            fee_mode: PositionTruthMode::Heuristic,
        }
    }
}

type ReopenHook = Arc<dyn Fn(Pubkey, Pubkey) + Send + Sync>;
static GLOBAL_PENDING_OPEN_CLAIMS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

fn pending_open_claims() -> &'static Mutex<HashSet<String>> {
    GLOBAL_PENDING_OPEN_CLAIMS.get_or_init(|| Mutex::new(HashSet::new()))
}

fn pending_open_claim_key(item: &pending_open::PendingOpenItem) -> String {
    if let Some(sid) = item
        .rebalance_session_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return format!("sid:{sid}");
    }
    format!("pool:{}|closed:{}", item.pool.trim(), item.closed_position_nft.trim())
}

#[derive(Debug, Clone, Copy)]
struct PriceSample {
    ts_unix: i64,
    price_ab: Decimal,
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
    rebalance_executor: Arc<RebalanceExecutor>,
    /// Circuit breaker.
    circuit_breaker: Arc<CircuitBreaker>,
    /// Lifecycle tracker.
    lifecycle: Arc<LifecycleTracker>,
    /// Configuration.
    config: ExecutorConfig,
    /// Running flag.
    running: std::sync::atomic::AtomicBool,
    /// Pool reader for fetching state.
    pool_reader: WhirlpoolReader,
    /// Tick reader for boundary fee-growth outside (Tier3).
    tick_reader: WhirlpoolTickReader,
    /// For `RetouchShift`: gating to allow only one retouch per out-of-range episode.
    retouch_armed: Arc<RwLock<HashMap<solana_sdk::pubkey::Pubkey, bool>>>,
    /// Latest optimization profile id (for IL ledger continuity / auditing).
    optimization_run_id: Mutex<Option<String>>,
    /// PDAs to skip in `evaluate_all` (strategy automation off for these positions).
    skip_evaluation_for: Arc<RwLock<HashSet<solana_sdk::pubkey::Pubkey>>>,
    /// Latest evaluation snapshot per position (best-effort, in-memory only).
    last_eval: Arc<RwLock<HashMap<solana_sdk::pubkey::Pubkey, PositionEvalSnapshot>>>,
    /// In-memory pool price samples per position for `LastCandle` strategy mode.
    price_samples: Arc<RwLock<HashMap<solana_sdk::pubkey::Pubkey, VecDeque<PriceSample>>>>,
    /// Optional JSON file for [`pending_open`] recovery (`CLMM_PENDING_OPEN_RECOVERY_PATH`).
    pending_open_recovery_path: Mutex<Option<PathBuf>>,
    /// Optional notifier for executor-level alerts (e.g. rebalance incomplete).
    alert_notifier: Mutex<Option<Arc<MultiNotifier>>>,
    /// Optional callback fired after a successful close+open cycle (old PDA → new PDA).
    /// Used by the API layer to keep `strategies.json` position links in sync without manual steps.
    reopen_hook: Mutex<Option<ReopenHook>>,
    /// Guardrail: when set, executor evaluates **only** these positions (prevents growth from stale monitor entries).
    managed_allowlist: Arc<RwLock<Option<HashSet<Pubkey>>>>,
    /// Guardrail: desired number of positions in `managed_allowlist` (fixed at strategy start).
    managed_target_count: Arc<Mutex<Option<usize>>>,
}

impl StrategyExecutor {
    fn managed_allowlist_state_from_positions(
        positions: Vec<Pubkey>,
    ) -> (Option<HashSet<Pubkey>>, Option<usize>) {
        let mut set = HashSet::new();
        for p in positions {
            set.insert(p);
        }
        let target = Some(set.len());
        (Some(set), target)
    }

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
        let tick_reader = WhirlpoolTickReader::new(provider.clone());
        let retouch_armed = Arc::new(RwLock::new(HashMap::new()));

        let rebalance_executor = Arc::new(RebalanceExecutor::new(
            provider,
            tx_manager.clone(),
            lifecycle.clone(),
            RebalanceConfig::from_env(),
        ));
        rebalance_executor.set_dry_run(config.dry_run);

        Self {
            monitor,
            decision_engine: DecisionEngine::default(),
            tx_manager,
            rebalance_executor,
            circuit_breaker,
            lifecycle,
            config,
            running: std::sync::atomic::AtomicBool::new(false),
            pool_reader,
            tick_reader,
            retouch_armed,
            optimization_run_id: Mutex::new(None),
            skip_evaluation_for: Arc::new(RwLock::new(HashSet::new())),
            last_eval: Arc::new(RwLock::new(HashMap::new())),
            price_samples: Arc::new(RwLock::new(HashMap::new())),
            pending_open_recovery_path: Mutex::new(
                std::env::var("CLMM_PENDING_OPEN_RECOVERY_PATH")
                    .ok()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from)
                    .or_else(|| Some(PathBuf::from("data/pending-open-recovery.json"))),
            ),
            alert_notifier: Mutex::new(None),
            reopen_hook: Mutex::new(None),
            managed_allowlist: Arc::new(RwLock::new(None)),
            managed_target_count: Arc::new(Mutex::new(None)),
        }
    }

    /// Guardrail: restrict evaluation to a fixed set of positions.
    ///
    /// This is intended to prevent “2 positions became 10” when `strategies.json` (or the monitor)
    /// contains stale historical PDAs. When set, the executor will **only** act on these PDAs, and
    /// a successful reopen will replace `old` with `new` inside this set.
    pub async fn set_managed_allowlist(&self, positions: Vec<Pubkey>) {
        let (allow, target) = Self::managed_allowlist_state_from_positions(positions);
        *self.managed_target_count.lock().await = target;
        *self.managed_allowlist.write().await = allow;
    }

    /// Set an optional hook that runs after a successful rebalance close→open (old PDA → new PDA).
    pub async fn set_reopen_hook(&self, hook: Option<ReopenHook>) {
        *self.reopen_hook.lock().await = hook;
    }

    /// Override pending-open recovery path (e.g. from CLI). `None` disables persistence.
    pub async fn set_pending_open_recovery_path(&self, path: Option<PathBuf>) {
        *self.pending_open_recovery_path.lock().await = path;
    }

    /// Webhook / multi notifier for [`AlertType::RebalanceIncomplete`] and similar.
    pub async fn set_alert_notifier(&self, notifier: Option<Arc<MultiNotifier>>) {
        *self.alert_notifier.lock().await = notifier;
    }

    /// Shared handle for [`EmergencyExitManager`] or custom tooling.
    #[must_use]
    pub fn rebalance_executor_handle(&self) -> Arc<RebalanceExecutor> {
        self.rebalance_executor.clone()
    }

    /// Replaces the set of position addresses for which this executor skips decisions/transactions.
    pub async fn set_skip_evaluation_for_addresses(&self, addresses: &[String]) {
        let mut set = HashSet::new();
        for s in addresses {
            if let Ok(pk) = solana_sdk::pubkey::Pubkey::from_str(s.trim()) {
                set.insert(pk);
            }
        }
        let mut w = self.skip_evaluation_for.write().await;
        *w = set;
    }

    async fn capture_position_fee_checkpoint(
        &self,
        position: &solana_sdk::pubkey::Pubkey,
        event_type: &str,
        source: CheckpointSource,
    ) -> Option<PositionFeeCheckpoint> {
        // Always refresh position first so checkpoints are contemporaneous.
        if self.monitor.refresh_position(position).await.is_err() {
            return None;
        }
        let monitored = self.monitor.get_position(position).await?;

        let pool_state = self
            .pool_reader
            .get_pool_state(&monitored.pool.to_string())
            .await
            .ok()?;

        // Tier3: fetch tick boundary feeGrowthOutside and compute feeGrowthInside.
        let lower = self
            .tick_reader
            .get_tick_boundary_state(
                &monitored.pool,
                monitored.on_chain.tick_lower,
                pool_state.tick_spacing,
            )
            .await
            .ok();
        let upper = self
            .tick_reader
            .get_tick_boundary_state(
                &monitored.pool,
                monitored.on_chain.tick_upper,
                pool_state.tick_spacing,
            )
            .await
            .ok();

        let (fee_growth_outside_lower_a, fee_growth_outside_lower_b) = lower
            .as_ref()
            .map(|t| (Some(t.fee_growth_outside_a), Some(t.fee_growth_outside_b)))
            .unwrap_or((None, None));
        let (fee_growth_outside_upper_a, fee_growth_outside_upper_b) = upper
            .as_ref()
            .map(|t| (Some(t.fee_growth_outside_a), Some(t.fee_growth_outside_b)))
            .unwrap_or((None, None));

        let fee_growth_inside_a = match (fee_growth_outside_lower_a, fee_growth_outside_upper_a) {
            (Some(lo), Some(up)) => Some(compute_fee_growth_inside_single(
                pool_state.fee_growth_global_a,
                lo,
                up,
                pool_state.tick_current,
                monitored.on_chain.tick_lower,
                monitored.on_chain.tick_upper,
            )),
            _ => None,
        };
        let fee_growth_inside_b = match (fee_growth_outside_lower_b, fee_growth_outside_upper_b) {
            (Some(lo), Some(up)) => Some(compute_fee_growth_inside_single(
                pool_state.fee_growth_global_b,
                lo,
                up,
                pool_state.tick_current,
                monitored.on_chain.tick_lower,
                monitored.on_chain.tick_upper,
            )),
            _ => None,
        };

        Some(PositionFeeCheckpoint {
            ts_utc: chrono::Utc::now().to_rfc3339(),
            position: position.to_string(),
            pool: monitored.pool.to_string(),
            event_type: event_type.to_string(),
            tick_lower: monitored.on_chain.tick_lower,
            tick_upper: monitored.on_chain.tick_upper,
            tick_current: Some(pool_state.tick_current),
            liquidity: monitored.on_chain.liquidity.to_string(),
            fees_owed_a: monitored.on_chain.fees_owed_a,
            fees_owed_b: monitored.on_chain.fees_owed_b,
            fee_growth_checkpoint_a: Some(monitored.on_chain.fee_growth_inside_a.to_string()),
            fee_growth_checkpoint_b: Some(monitored.on_chain.fee_growth_inside_b.to_string()),
            fee_growth_global_a: Some(pool_state.fee_growth_global_a.to_string()),
            fee_growth_global_b: Some(pool_state.fee_growth_global_b.to_string()),
            fee_growth_outside_lower_a: fee_growth_outside_lower_a.map(|v| v.to_string()),
            fee_growth_outside_lower_b: fee_growth_outside_lower_b.map(|v| v.to_string()),
            fee_growth_outside_upper_a: fee_growth_outside_upper_a.map(|v| v.to_string()),
            fee_growth_outside_upper_b: fee_growth_outside_upper_b.map(|v| v.to_string()),
            fee_growth_inside_a: fee_growth_inside_a.map(|v| v.to_string()),
            fee_growth_inside_b: fee_growth_inside_b.map(|v| v.to_string()),
            sqrt_price_x64: Some(pool_state.sqrt_price.to_string()),
            collected_a: 0,
            collected_b: 0,
            source,
        })
    }

    /// Sets the wallet for signing transactions.
    pub fn set_wallet(&self, wallet: Arc<Wallet>) {
        self.rebalance_executor.set_wallet(wallet);
    }

    /// Returns the configured wallet pubkey (if any).
    #[must_use]
    pub fn wallet_pubkey(&self) -> Option<solana_sdk::pubkey::Pubkey> {
        self.rebalance_executor.wallet_pubkey()
    }

    /// Whether this executor is in dry-run mode (no on-chain txs from rebalance/close paths).
    #[must_use]
    pub fn is_dry_run(&self) -> bool {
        self.config.dry_run
    }

    /// Sets the decision engine configuration.
    pub fn set_decision_config(&self, config: DecisionConfig) {
        self.decision_engine.set_config(config);
    }

    /// Sets the current optimization run id used to stamp lifecycle/ledger rows.
    pub fn set_optimization_run_id(&self, run_id: Option<String>) {
        *self.optimization_run_id.blocking_lock() = run_id;
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

    /// Returns the most recent evaluation snapshot for the given position, if any.
    pub async fn last_evaluation_for_position(
        &self,
        position: &solana_sdk::pubkey::Pubkey,
    ) -> Option<PositionEvalSnapshot> {
        self.last_eval.read().await.get(position).cloned()
    }

    /// Optional JSONL path for IL / rebalance ledger (see `LifecycleTracker::set_il_ledger_path`).
    pub fn set_il_ledger_path(&self, path: Option<std::path::PathBuf>) {
        self.lifecycle.set_il_ledger_path(path);
    }

    /// Optional JSONL path for Tier3 position-fee checkpoints.
    pub fn set_position_fee_ledger_path(&self, path: Option<std::path::PathBuf>) {
        self.lifecycle.set_position_fee_ledger_path(path);
    }

    /// Partially decrease liquidity on-chain (delegates to [`RebalanceExecutor`]).
    pub async fn execute_partial_decrease_liquidity(
        &self,
        position: &solana_sdk::pubkey::Pubkey,
        pool: &solana_sdk::pubkey::Pubkey,
        liquidity_amount: u128,
    ) -> anyhow::Result<()> {
        if self.config.fee_mode == PositionTruthMode::PositionTruth
            && let Some(cp) = self
                .capture_position_fee_checkpoint(
                    position,
                    "decrease_liquidity_pre",
                    CheckpointSource::Onchain,
                )
                .await
        {
            self.lifecycle.record_fee_checkpoint(cp).await;
        }
        self.rebalance_executor
            .execute_partial_decrease(position, pool, liquidity_amount)
            .await?;

        if self.config.fee_mode == PositionTruthMode::PositionTruth
            && let Some(cp) = self
                .capture_position_fee_checkpoint(
                    position,
                    "decrease_liquidity_post",
                    CheckpointSource::Onchain,
                )
                .await
        {
            self.lifecycle.record_fee_checkpoint(cp).await;
        }
        Ok(())
    }

    /// Orca swap ExactIn in pool (server wallet) — e.g. before `execute_open_position`.
    ///
    /// `ledger_session_id` ties lifecycle JSONL rows (swap + open) for per-position cost sums.
    pub async fn execute_swap_exact_in(
        &self,
        pool: &solana_sdk::pubkey::Pubkey,
        specified_mint: &solana_sdk::pubkey::Pubkey,
        amount_in: u64,
        slippage_bps: u16,
        // Optional position PDA to attach to the lifecycle ledger row (useful for swap-mix during rebalances).
        position_for_ledger: Option<solana_sdk::pubkey::Pubkey>,
        ledger_session_id: Option<String>,
    ) -> anyhow::Result<Option<solana_sdk::signature::Signature>> {
        self.rebalance_executor
            .execute_swap_exact_in(
                pool,
                specified_mint,
                amount_in,
                slippage_bps,
                position_for_ledger,
                ledger_session_id,
            )
            .await
    }

    /// Opens a new Whirlpool position using explicit token caps.
    ///
    /// In dry-run mode this returns the derived position PDA without requiring wallet.
    /// Set `full_range` for Splash-style full-range opens (ignores `tick_lower` / `tick_upper` on-chain).
    ///
    /// `ledger_open_details`: merged into lifecycle `details` on success (e.g. API passes
    /// `open_origin: "operator_api"` for lineage). Strategy callers should pass `None`.
    #[allow(clippy::too_many_arguments)]
    pub async fn execute_open_position(
        &self,
        pool: &solana_sdk::pubkey::Pubkey,
        tick_lower: i32,
        tick_upper: i32,
        amount_a: u64,
        amount_b: u64,
        slippage_bps: u16,
        full_range: bool,
        ledger_session_id: Option<String>,
        ledger_open_details: Option<serde_json::Value>,
    ) -> anyhow::Result<solana_sdk::pubkey::Pubkey> {
        let (position, eff_tl, eff_tu) = self
            .rebalance_executor
            .execute_open_position(
                pool,
                tick_lower,
                tick_upper,
                amount_a,
                amount_b,
                slippage_bps,
                full_range,
                ledger_session_id,
                ledger_open_details,
            )
            .await?;

        if self.config.fee_mode == PositionTruthMode::PositionTruth {
            // After opening, capture on-chain snapshot (position should now exist).
            if let Some(cp) = self
                .capture_position_fee_checkpoint(&position, "open_post", CheckpointSource::Onchain)
                .await
            {
                self.lifecycle.record_fee_checkpoint(cp).await;
            } else {
                // Fallback minimal derived row for auditability.
                self.lifecycle
                    .record_fee_checkpoint(PositionFeeCheckpoint {
                        ts_utc: chrono::Utc::now().to_rfc3339(),
                        position: position.to_string(),
                        pool: pool.to_string(),
                        event_type: "open_post_missing".to_string(),
                        tick_lower: eff_tl,
                        tick_upper: eff_tu,
                        tick_current: None,
                        liquidity: "0".to_string(),
                        fees_owed_a: 0,
                        fees_owed_b: 0,
                        fee_growth_checkpoint_a: None,
                        fee_growth_checkpoint_b: None,
                        fee_growth_global_a: None,
                        fee_growth_global_b: None,
                        fee_growth_outside_lower_a: None,
                        fee_growth_outside_lower_b: None,
                        fee_growth_outside_upper_a: None,
                        fee_growth_outside_upper_b: None,
                        fee_growth_inside_a: None,
                        fee_growth_inside_b: None,
                        sqrt_price_x64: None,
                        collected_a: 0,
                        collected_b: 0,
                        source: CheckpointSource::Missing,
                    })
                    .await;
            }
        }
        Ok(position)
    }

    /// Collects Whirlpool fees for a given position.
    pub async fn execute_collect_fees_only(
        &self,
        position: &solana_sdk::pubkey::Pubkey,
        pool: &solana_sdk::pubkey::Pubkey,
        ledger_session_id: Option<String>,
    ) -> anyhow::Result<()> {
        if self.config.fee_mode == PositionTruthMode::PositionTruth
            && let Some(cp) = self
                .capture_position_fee_checkpoint(position, "collect_pre", CheckpointSource::Onchain)
                .await
        {
            self.lifecycle.record_fee_checkpoint(cp).await;
        }
        self.rebalance_executor
            .execute_collect_fees_only(position, pool, ledger_session_id)
            .await?;
        if self.config.fee_mode == PositionTruthMode::PositionTruth
            && let Some(cp) = self
                .capture_position_fee_checkpoint(
                    position,
                    "collect_post",
                    CheckpointSource::Onchain,
                )
                .await
        {
            self.lifecycle.record_fee_checkpoint(cp).await;
        }
        Ok(())
    }

    /// Closes Whirlpool position by decreasing all liquidity, collecting, and closing NFT.
    pub async fn execute_full_close_only(
        &self,
        position: &solana_sdk::pubkey::Pubkey,
        pool: &solana_sdk::pubkey::Pubkey,
        ledger_session_id: Option<String>,
        ledger_details: Option<serde_json::Value>,
    ) -> anyhow::Result<()> {
        if self.config.fee_mode == PositionTruthMode::PositionTruth
            && let Some(cp) = self
                .capture_position_fee_checkpoint(position, "close_pre", CheckpointSource::Onchain)
                .await
        {
            self.lifecycle.record_fee_checkpoint(cp).await;
        }
        self.rebalance_executor
            .execute_full_close_only(position, pool, ledger_session_id, ledger_details)
            .await?;
        if self.config.fee_mode == PositionTruthMode::PositionTruth {
            // After close, position PDA may still exist briefly; best-effort capture.
            if let Some(cp) = self
                .capture_position_fee_checkpoint(position, "close_post", CheckpointSource::Onchain)
                .await
            {
                self.lifecycle.record_fee_checkpoint(cp).await;
            }
        }
        Ok(())
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
            fee_mode = ?self.config.fee_mode,
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

    async fn emit_executor_alert(&self, alert: Alert) {
        let notifier = self.alert_notifier.lock().await.clone();
        if let Some(n) = notifier {
            n.notify_all(&alert).await;
        }
    }

    /// Retry `open` for rows in [`pending_open::PendingOpenStore`] (after `rebalance_incomplete`).
    async fn process_pending_open_recoveries(&self) -> anyhow::Result<()> {
        let path = self.pending_open_recovery_path.lock().await.clone();
        let Some(path) = path else {
            return Ok(());
        };
        if !self.config.auto_execute || self.config.dry_run {
            return Ok(());
        }
        if self.wallet_pubkey().is_none() {
            return Ok(());
        }

        let mut store = pending_open::load(&path).unwrap_or_default();
        if store.items.is_empty() {
            return Ok(());
        }
        let max_a = pending_open::max_recovery_attempts();
        let alert_threshold = pending_open::attempts_alert_threshold();
        let mut kept: Vec<super::pending_open::PendingOpenItem> = Vec::new();

        for mut item in std::mem::take(&mut store.items) {
            let claim_key = pending_open_claim_key(&item);
            let claimed = {
                let mut claims = pending_open_claims().lock().await;
                claims.insert(claim_key.clone())
            };
            if !claimed {
                // Another executor/cycle is already handling this pending-open item.
                // Keep it in queue unchanged (do not burn attempts on duplicate workers).
                kept.push(item);
                continue;
            }
            if item.attempts >= max_a {
                {
                    let mut claims = pending_open_claims().lock().await;
                    claims.remove(&claim_key);
                }
                continue;
            }
            item.attempts += 1;
            item.last_attempt_at = Some(chrono::Utc::now().to_rfc3339());
            let pool = match solana_sdk::pubkey::Pubkey::from_str(item.pool.trim()) {
                Ok(v) => v,
                Err(e) => {
                    let mut claims = pending_open_claims().lock().await;
                    claims.remove(&claim_key);
                    return Err(anyhow::anyhow!("pending pool pubkey: {e}"));
                }
            };
            let closed = match solana_sdk::pubkey::Pubkey::from_str(item.closed_position_nft.trim())
            {
                Ok(v) => v,
                Err(e) => {
                    let mut claims = pending_open_claims().lock().await;
                    claims.remove(&claim_key);
                    return Err(anyhow::anyhow!("pending closed NFT pubkey: {e}"));
                }
            };

            let res = self
                .rebalance_executor
                .recover_open_after_incomplete(RecoverOpenParams {
                    pool,
                    new_tick_lower: item.intended_tick_lower,
                    new_tick_upper: item.intended_tick_upper,
                    planned_at_utc: item.planned_at_utc.clone(),
                    planned_price_ab: item.planned_price_ab,
                    reason: item.reason.clone(),
                    closed_position_nft: closed,
                    rebalance_session_id: item.rebalance_session_id.clone(),
                    optimization_run_id: item.optimization_run_id.clone(),
                })
                .await;

            if res.success {
                if let Some(np) = res.new_position {
                    if let Err(e) = self.monitor.add_position(&np.to_string()).await {
                        warn!(
                            error = %e,
                            new_position = %np,
                            "pending_open: add_position failed after recover"
                        );
                        item.last_error = Some(format!("add_position: {e}"));
                        if item.attempts < max_a {
                            kept.push(item);
                        }
                    } else {
                        // Keep managed set stable on recovery too (replace old->new; never grow).
                        if let Some(ref mut allow) = self.managed_allowlist.write().await.as_mut()
                            && allow.remove(&closed)
                        {
                            allow.insert(np);
                            if let Some(target) = *self.managed_target_count.lock().await {
                                while allow.len() > target {
                                    if let Some(extra) = allow.iter().copied().find(|p| p != &np) {
                                        allow.remove(&extra);
                                    } else {
                                        break;
                                    }
                                }
                            }
                        }
                        if let Some(h) = self.reopen_hook.lock().await.clone() {
                            // Best-effort: keep strategy links in sync after pending-open recovery.
                            h(closed, np);
                        }
                        info!(
                            new_position = %np,
                            pool = %pool,
                            "pending_open recovery succeeded"
                        );
                    }
                }
            } else {
                item.last_error = res.error.clone();
                let stuck_reason = classify_pending_open_stuck_reason(item.last_error.as_deref());
                if item.stuck_reason.as_deref() != Some(stuck_reason) {
                    item.stuck_since = Some(chrono::Utc::now().to_rfc3339());
                }
                item.stuck_reason = Some(stuck_reason.to_string());

                if should_emit_pending_open_stuck_alert(
                    item.attempts,
                    alert_threshold,
                    item.last_alert_attempts,
                ) {
                    item.last_alert_attempts = Some(item.attempts);
                    warn!(
                        pool = %pool,
                        closed_position = %closed,
                        attempts = item.attempts,
                        threshold = alert_threshold,
                        stuck_reason = stuck_reason,
                        error = item.last_error.as_deref().unwrap_or("unknown"),
                        "pending_open recovery marked as stuck"
                    );
                    self.emit_executor_alert(
                        Alert::new(
                            AlertLevel::Warning,
                            AlertType::Custom("Pending Open Stuck".to_string()),
                            format!(
                                "Pending-open stuck: pool {} position {} attempts={} reason={}",
                                pool, closed, item.attempts, stuck_reason
                            ),
                        )
                        .with_position(&closed)
                        .with_pool(&pool),
                    )
                    .await;
                }
                if item.attempts < max_a {
                    kept.push(item);
                } else {
                    warn!(
                        pool = %pool,
                        error = ?res.error,
                        "pending_open recovery failed (max attempts)"
                    );
                }
            }
            {
                let mut claims = pending_open_claims().lock().await;
                claims.remove(&claim_key);
            }
        }

        store.items = kept;
        pending_open::save(&path, &store)?;
        Ok(())
    }

    /// Evaluates all monitored positions.
    async fn evaluate_all(&self) -> anyhow::Result<()> {
        if let Err(e) = self.process_pending_open_recoveries().await {
            warn!(error = %e, "pending_open recovery pass failed");
        }

        let positions = self.monitor.get_positions().await;

        debug!(count = positions.len(), "Evaluating positions");

        let allow = self.managed_allowlist.read().await.clone();
        for position in positions {
            if let Some(ref allow) = allow
                && !allow.contains(&position.address)
            {
                continue;
            }
            if self
                .skip_evaluation_for
                .read()
                .await
                .contains(&position.address)
            {
                debug!(
                    position = %position.address,
                    "Skipping strategy evaluation (disabled for this position)"
                );
                continue;
            }
            if self.config.fee_mode == PositionTruthMode::PositionTruth
                && let Some(cp) = self
                    .capture_position_fee_checkpoint(
                        &position.address,
                        "poll",
                        CheckpointSource::Onchain,
                    )
                    .await
            {
                self.lifecycle.record_fee_checkpoint(cp).await;
            }
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
        // Always refresh the monitored snapshot before deciding.
        //
        // Important safety guard: if refresh fails OR refresh removed the position (e.g. manual close),
        // skip evaluation instead of using stale cached data. Acting on stale snapshots can trigger an
        // unintended close+open cycle right after the operator manually closed a position.
        let position = match self.monitor.refresh_position(&position.address).await {
            Ok(()) => match self.monitor.get_position(&position.address).await {
                Some(p) => p,
                None => {
                    tracing::info!(
                        position = %position.address,
                        "evaluate_position: position removed from monitor after refresh; skipping"
                    );
                    return Ok(());
                }
            },
            Err(e) => {
                tracing::warn!(
                    position = %position.address,
                    error = %e,
                    "evaluate_position: refresh_position failed; skipping this cycle to avoid stale actions"
                );
                return Ok(());
            }
        };

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

        // Calculate minutes since last rebalance from lifecycle.
        let minutes_since_rebalance = self
            .calculate_minutes_since_rebalance(&position.address)
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
        let cfg = self.decision_engine.config();
        let last_candle_ticks = if matches!(
            cfg.strategy_mode,
            StrategyMode::LastCandle | StrategyMode::LastCandlePeriodic
        ) {
            self.record_price_and_compute_last_closed_candle_ticks(
                &position.address,
                &pool,
                cfg.last_candle_seconds.max(60),
            )
            .await
        } else {
            None
        };

        let context = DecisionContext {
            position: position.clone(),
            pool: pool.clone(),
            minutes_since_rebalance,
            retouch_armed,
            last_candle_ticks,
        };

        let decision = self.decision_engine.decide(&context);

        {
            let snap = PositionEvalSnapshot {
                ts_utc: chrono::Utc::now().to_rfc3339(),
                in_range: position.in_range,
                pool_tick_current: pool.tick_current,
                decision: decision.description(),
                requires_transaction: decision.requires_transaction(),
                auto_execute: self.config.auto_execute,
                hours_since_rebalance: if minutes_since_rebalance == u64::MAX {
                    None
                } else {
                    Some(minutes_since_rebalance / 60)
                },
                minutes_since_rebalance: if minutes_since_rebalance == u64::MAX {
                    None
                } else {
                    Some(minutes_since_rebalance)
                },
            };
            let mut m = self.last_eval.write().await;
            m.insert(position.address, snap);
        }

        if decision.requires_transaction() {
            info!(
                position = %position.address,
                decision = %decision.description(),
                dry_run = self.config.dry_run,
                "Decision requires action"
            );

            if self.config.auto_execute {
                self.execute_decision(&position, &decision, &pool).await?;
            }
        }

        Ok(())
    }

    /// Calculates minutes since last rebalance.
    async fn calculate_minutes_since_rebalance(
        &self,
        position: &solana_sdk::pubkey::Pubkey,
    ) -> u64 {
        let events = self.lifecycle.get_events(position).await;

        // Find the last rebalance event
        for event in events.iter().rev() {
            if event.event_type == crate::lifecycle::LifecycleEventType::Rebalanced {
                let duration = chrono::Utc::now() - event.timestamp;
                return duration.num_minutes().max(0) as u64;
            }
        }

        // If no rebalance, use position open time
        if let Some(summary) = self.lifecycle.get_summary(position).await {
            let duration = chrono::Utc::now() - summary.opened_at;
            return duration.num_minutes().max(0) as u64;
        }

        // Default to a large value to allow rebalancing
        u64::MAX
    }

    async fn record_price_and_compute_last_closed_candle_ticks(
        &self,
        position: &solana_sdk::pubkey::Pubkey,
        pool: &WhirlpoolState,
        candle_seconds: u64,
    ) -> Option<(i32, i32)> {
        let now = chrono::Utc::now().timestamp();
        let cs = candle_seconds.max(60) as i64;
        let current_bucket = now / cs;
        let last_closed_bucket = current_bucket.saturating_sub(1);
        let keep_after = now.saturating_sub(cs.saturating_mul(4));

        let mut samples = self.price_samples.write().await;
        let entry = samples.entry(*position).or_insert_with(VecDeque::new);
        entry.push_back(PriceSample {
            ts_unix: now,
            price_ab: pool.price,
        });
        while entry
            .front()
            .is_some_and(|s| s.ts_unix < keep_after || entry.len() > 4096)
        {
            entry.pop_front();
        }

        let mut low = Decimal::MAX;
        let mut high = Decimal::ZERO;
        let mut found = false;
        for s in entry.iter() {
            if s.ts_unix / cs == last_closed_bucket {
                low = low.min(s.price_ab);
                high = high.max(s.price_ab);
                found = true;
            }
        }

        if !found || low <= Decimal::ZERO || high <= low {
            return None;
        }

        let mut lo = clmm_lp_protocols::prelude::price_to_tick(low);
        let mut hi = clmm_lp_protocols::prelude::price_to_tick(high);
        let spacing = pool.tick_spacing as i32;
        if spacing > 0 {
            lo = lo.div_euclid(spacing) * spacing;
            hi = ((hi + spacing - 1).div_euclid(spacing)) * spacing;
        }
        if hi <= lo {
            hi = lo + spacing.max(1);
        }
        Some((lo, hi))
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
                    StrategyMode::LastCandle => RebalanceReason::RangeExit,
                    StrategyMode::LastCandlePeriodic => RebalanceReason::Periodic,
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
                let optimization_run_id = self.optimization_run_id.lock().await.clone();
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
                    reason: reason.clone(),
                    current_il_pct: position.pnl.il_pct,
                    amount_a_before: None,
                    amount_b_before: None,
                    price_ab_before: Some(pool.price),
                    amount_a_after: None,
                    amount_b_after: None,
                    price_ab_after: None,
                    optimization_run_id,
                };

                let result = self.rebalance_executor.execute(params).await;

                if !result.success {
                    if result.old_position_closed_on_chain && result.new_position.is_none() {
                        error!(
                            op = "orca_rebalance",
                            outcome = "incomplete",
                            position = %position.address,
                            pool = %position.pool,
                            tick_lower_old = position.on_chain.tick_lower,
                            tick_upper_old = position.on_chain.tick_upper,
                            tick_lower_new = *new_tick_lower,
                            tick_upper_new = *new_tick_upper,
                            tick_current = pool.tick_current,
                            reason = ?reason,
                            error = result.error.as_deref(),
                            "Rebalance incomplete: old position closed on-chain but new position was not opened; removing stale monitor entry (restart with a new position NFT or re-open manually)"
                        );
                        self.lifecycle
                            .record_rebalance_incomplete(
                                position.address,
                                position.pool,
                                position.on_chain.tick_lower,
                                position.on_chain.tick_upper,
                                *new_tick_lower,
                                *new_tick_upper,
                                reason.clone(),
                                result.error.as_deref(),
                                self.optimization_run_id.lock().await.clone(),
                            )
                            .await;
                        let path_opt = self.pending_open_recovery_path.lock().await.clone();
                        if let Some(ref path) = path_opt {
                            let mut s = pending_open::load(path).unwrap_or_default();
                            pending_open::upsert(
                                &mut s,
                                pending_open::PendingOpenItem {
                                    pool: position.pool.to_string(),
                                    intended_tick_lower: *new_tick_lower,
                                    intended_tick_upper: *new_tick_upper,
                                    closed_position_nft: position.address.to_string(),
                                    rebalance_session_id: result.rebalance_session_id.clone(),
                                    planned_at_utc: Some(chrono::Utc::now().to_rfc3339()),
                                    planned_price_ab: Some(pool.price),
                                    reason: reason.clone(),
                                    optimization_run_id: self
                                        .optimization_run_id
                                        .lock()
                                        .await
                                        .clone(),
                                    attempts: 0,
                                    last_error: result.error.clone(),
                                    last_attempt_at: None,
                                    stuck_reason: Some(
                                        classify_pending_open_stuck_reason(result.error.as_deref())
                                            .to_string(),
                                    ),
                                    stuck_since: Some(chrono::Utc::now().to_rfc3339()),
                                    last_alert_attempts: None,
                                    created_at: chrono::Utc::now().to_rfc3339(),
                                },
                            );
                            let _ = pending_open::save(path, &s);
                        }
                        self.emit_executor_alert(
                            Alert::new(
                                AlertLevel::Critical,
                                AlertType::RebalanceIncomplete,
                                format!(
                                    "Rebalance incomplete: pool {} position {} — {}",
                                    position.pool,
                                    position.address,
                                    result.error.as_deref().unwrap_or("unknown")
                                ),
                            )
                            .with_position(&position.address)
                            .with_pool(&position.pool),
                        )
                        .await;
                        let old_addr = position.address;
                        self.monitor.remove_position(&old_addr).await;
                        if self.decision_engine.config().strategy_mode == StrategyMode::RetouchShift
                        {
                            let mut m = self.retouch_armed.write().await;
                            m.remove(&old_addr);
                        }
                    } else if let Some(ref err) = result.error {
                        error!(
                            op = "orca_rebalance",
                            outcome = "failed",
                            position = %position.address,
                            pool = %position.pool,
                            tick_lower_old = position.on_chain.tick_lower,
                            tick_upper_old = position.on_chain.tick_upper,
                            tick_lower_new = *new_tick_lower,
                            tick_upper_new = *new_tick_upper,
                            tick_current = pool.tick_current,
                            reason = ?reason,
                            old_position_closed = result.old_position_closed_on_chain,
                            error = %err,
                            "Rebalance failed"
                        );
                    }
                }

                // Keep the monitor set in sync with the actual rebalance outcome:
                // - old position is closed
                // - new position is opened
                if result.success {
                    let old_addr = position.address;
                    self.lifecycle
                        .record_fee_checkpoint(PositionFeeCheckpoint {
                            ts_utc: chrono::Utc::now().to_rfc3339(),
                            position: old_addr.to_string(),
                            pool: position.pool.to_string(),
                            event_type: "rebalance_out".to_string(),
                            tick_lower: position.on_chain.tick_lower,
                            tick_upper: position.on_chain.tick_upper,
                            tick_current: Some(pool.tick_current),
                            liquidity: position.on_chain.liquidity.to_string(),
                            fees_owed_a: position.on_chain.fees_owed_a,
                            fees_owed_b: position.on_chain.fees_owed_b,
                            fee_growth_checkpoint_a: Some(
                                position.on_chain.fee_growth_inside_a.to_string(),
                            ),
                            fee_growth_checkpoint_b: Some(
                                position.on_chain.fee_growth_inside_b.to_string(),
                            ),
                            fee_growth_global_a: Some(pool.fee_growth_global_a.to_string()),
                            fee_growth_global_b: Some(pool.fee_growth_global_b.to_string()),
                            fee_growth_outside_lower_a: None,
                            fee_growth_outside_lower_b: None,
                            fee_growth_outside_upper_a: None,
                            fee_growth_outside_upper_b: None,
                            fee_growth_inside_a: None,
                            fee_growth_inside_b: None,
                            sqrt_price_x64: Some(pool.sqrt_price.to_string()),
                            collected_a: result.fees_collected.map(|x| x.0).unwrap_or(0),
                            collected_b: result.fees_collected.map(|x| x.1).unwrap_or(0),
                            source: CheckpointSource::Onchain,
                        })
                        .await;

                    self.monitor.remove_position(&old_addr).await;

                    if let Some(new_pos) = result.new_position {
                        // Guardrail: keep managed set size constant (replace old→new; never grow).
                        if let Some(ref mut allow) = self.managed_allowlist.write().await.as_mut() {
                            allow.remove(&old_addr);
                            allow.insert(new_pos);
                            if let Some(target) = *self.managed_target_count.lock().await {
                                // Best-effort: if something went wrong and the set grew, trim it back.
                                while allow.len() > target {
                                    // Remove an arbitrary extra (not the newly opened).
                                    if let Some(extra) =
                                        allow.iter().copied().find(|p| p != &new_pos)
                                    {
                                        allow.remove(&extra);
                                    } else {
                                        break;
                                    }
                                }
                            }
                        }
                        if let Some(h) = self.reopen_hook.lock().await.clone() {
                            // Best-effort: keep strategy links in sync in the API layer.
                            h(old_addr, new_pos);
                        }
                        self.lifecycle
                            .record_fee_checkpoint(PositionFeeCheckpoint {
                                ts_utc: chrono::Utc::now().to_rfc3339(),
                                position: new_pos.to_string(),
                                pool: position.pool.to_string(),
                                event_type: "rebalance_in".to_string(),
                                tick_lower: *new_tick_lower,
                                tick_upper: *new_tick_upper,
                                tick_current: Some(pool.tick_current),
                                liquidity: result.liquidity_added.to_string(),
                                fees_owed_a: 0,
                                fees_owed_b: 0,
                                fee_growth_checkpoint_a: None,
                                fee_growth_checkpoint_b: None,
                                fee_growth_global_a: Some(pool.fee_growth_global_a.to_string()),
                                fee_growth_global_b: Some(pool.fee_growth_global_b.to_string()),
                                fee_growth_outside_lower_a: None,
                                fee_growth_outside_lower_b: None,
                                fee_growth_outside_upper_a: None,
                                fee_growth_outside_upper_b: None,
                                fee_growth_inside_a: None,
                                fee_growth_inside_b: None,
                                sqrt_price_x64: Some(pool.sqrt_price.to_string()),
                                collected_a: 0,
                                collected_b: 0,
                                source: CheckpointSource::Derived,
                            })
                            .await;
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
                    .execute_full_close_only(
                        &addr,
                        &pool_pk,
                        None,
                        Some(serde_json::json!({"close_kind":"strategy"})),
                    )
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
                    clamp_decimal_liquidity_to_u128(amount, position.on_chain.liquidity)
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
                    .execute_collect_fees_only(&position.address, &position.pool, None)
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

fn classify_pending_open_stuck_reason(last_error: Option<&str>) -> &'static str {
    let Some(err) = last_error.map(str::trim).filter(|s| !s.is_empty()) else {
        return "unknown";
    };
    let e = err.to_ascii_lowercase();
    if e.contains("not in new range") || e.contains("tick") && e.contains("range") {
        return "tick_out_of_range";
    }
    if e.contains("quote") || e.contains("cannot quote deposit") {
        return "quote_failed";
    }
    if e.contains("timeout") || e.contains("timed out") {
        return "rpc_timeout";
    }
    if e.contains("insufficient")
        || e.contains("insufficient funds")
        || e.contains("insufficient balance")
    {
        return "insufficient_balance";
    }
    "unknown"
}

fn should_emit_pending_open_stuck_alert(
    attempts: u32,
    threshold: u32,
    last_alert_attempts: Option<u32>,
) -> bool {
    if attempts < threshold || threshold == 0 {
        return false;
    }
    last_alert_attempts.is_none_or(|last| last < threshold)
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

    #[test]
    fn empty_managed_allowlist_stays_restrictive() {
        let (allow, target) = StrategyExecutor::managed_allowlist_state_from_positions(Vec::new());
        assert_eq!(target, Some(0));
        assert!(allow.is_some());
        assert_eq!(allow.expect("allowlist should exist").len(), 0);
    }

    #[test]
    fn classify_pending_open_stuck_reason_prefers_tick_out_of_range() {
        let reason = classify_pending_open_stuck_reason(Some(
            "pool tick -24299 not in new range [-24264, -24160): cannot quote deposit for open",
        ));
        assert_eq!(reason, "tick_out_of_range");
    }

    #[test]
    fn classify_pending_open_stuck_reason_detects_timeout_and_balance() {
        assert_eq!(
            classify_pending_open_stuck_reason(Some("rpc timed out during simulation")),
            "rpc_timeout"
        );
        assert_eq!(
            classify_pending_open_stuck_reason(Some("insufficient funds for instruction")),
            "insufficient_balance"
        );
    }

    #[test]
    fn pending_open_stuck_alert_threshold_emits_once_per_item() {
        assert!(!should_emit_pending_open_stuck_alert(4, 5, None));
        assert!(should_emit_pending_open_stuck_alert(5, 5, None));
        assert!(!should_emit_pending_open_stuck_alert(6, 5, Some(5)));
    }
}
