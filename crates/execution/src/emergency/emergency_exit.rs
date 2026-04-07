//! Emergency exit procedures for positions.

use crate::monitor::PositionMonitor;
use crate::transaction::TransactionManager;
use solana_sdk::pubkey::Pubkey;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

/// Emergency exit status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitStatus {
    /// Exit not started.
    Pending,
    /// Collecting fees.
    CollectingFees,
    /// Decreasing liquidity.
    DecreasingLiquidity,
    /// Closing position.
    ClosingPosition,
    /// Exit completed.
    Completed,
    /// Exit failed.
    Failed,
}

/// Result of an emergency exit.
#[derive(Debug, Clone)]
pub struct ExitResult {
    /// Position address.
    pub position: Pubkey,
    /// Exit status.
    pub status: ExitStatus,
    /// Error message if failed.
    pub error: Option<String>,
    /// Fees collected (token A, token B).
    pub fees_collected: Option<(u64, u64)>,
    /// Liquidity removed.
    pub liquidity_removed: Option<u128>,
}

/// Configuration for emergency exit.
#[derive(Debug, Clone)]
pub struct EmergencyExitConfig {
    /// Whether to collect fees before exiting.
    pub collect_fees: bool,
    /// Maximum slippage for exit (as basis points).
    pub max_slippage_bps: u16,
    /// Whether to retry on failure.
    pub retry_on_failure: bool,
    /// Maximum retries.
    pub max_retries: u32,
}

impl Default for EmergencyExitConfig {
    fn default() -> Self {
        Self {
            collect_fees: true,
            max_slippage_bps: 100, // 1%
            retry_on_failure: true,
            max_retries: 3,
        }
    }
}

/// Emergency exit manager for closing positions quickly.
pub struct EmergencyExitManager {
    /// Position monitor.
    monitor: Arc<PositionMonitor>,
    /// Transaction manager.
    #[allow(dead_code)]
    tx_manager: Arc<TransactionManager>,
    /// When set, performs real collect / decrease / close via Orca.
    rebalance_executor: Option<Arc<crate::strategy::RebalanceExecutor>>,
    /// Configuration.
    config: EmergencyExitConfig,
    /// Exit results.
    results: Arc<RwLock<Vec<ExitResult>>>,
    /// Whether an exit is in progress.
    in_progress: Arc<RwLock<bool>>,
}

impl EmergencyExitManager {
    /// Creates a new emergency exit manager.
    pub fn new(
        monitor: Arc<PositionMonitor>,
        tx_manager: Arc<TransactionManager>,
        config: EmergencyExitConfig,
    ) -> Self {
        Self {
            monitor,
            tx_manager,
            rebalance_executor: None,
            config,
            results: Arc::new(RwLock::new(Vec::new())),
            in_progress: Arc::new(RwLock::new(false)),
        }
    }

    /// Use the same signing + Orca path as [`crate::strategy::RebalanceExecutor`].
    pub fn new_with_rebalance(
        monitor: Arc<PositionMonitor>,
        tx_manager: Arc<TransactionManager>,
        rebalance_executor: Arc<crate::strategy::RebalanceExecutor>,
        config: EmergencyExitConfig,
    ) -> Self {
        Self {
            monitor,
            tx_manager,
            rebalance_executor: Some(rebalance_executor),
            config,
            results: Arc::new(RwLock::new(Vec::new())),
            in_progress: Arc::new(RwLock::new(false)),
        }
    }

    /// Executes emergency exit for all positions.
    pub async fn exit_all(&self) -> Vec<ExitResult> {
        // Check if already in progress
        {
            let mut in_progress = self.in_progress.write().await;
            if *in_progress {
                warn!("Emergency exit already in progress");
                return self.results.read().await.clone();
            }
            *in_progress = true;
        }

        info!("Starting emergency exit for all positions");

        let positions = self.monitor.get_positions().await;
        let mut results = Vec::new();

        for position in positions {
            let result = self.exit_position(&position.address).await;
            results.push(result);
        }

        // Store results
        *self.results.write().await = results.clone();
        *self.in_progress.write().await = false;

        info!(
            total = results.len(),
            completed = results
                .iter()
                .filter(|r| r.status == ExitStatus::Completed)
                .count(),
            failed = results
                .iter()
                .filter(|r| r.status == ExitStatus::Failed)
                .count(),
            "Emergency exit completed"
        );

        results
    }

    /// Executes emergency exit for a single position.
    pub async fn exit_position(&self, position: &Pubkey) -> ExitResult {
        info!(position = %position, "Starting emergency exit for position");

        let mut result = ExitResult {
            position: *position,
            status: ExitStatus::Pending,
            error: None,
            fees_collected: None,
            liquidity_removed: None,
        };

        // Step 1: Collect fees if configured
        if self.config.collect_fees {
            result.status = ExitStatus::CollectingFees;
            match self.collect_fees(position).await {
                Ok(fees) => {
                    result.fees_collected = Some(fees);
                    info!(position = %position, fees_a = fees.0, fees_b = fees.1, "Fees collected");
                }
                Err(e) => {
                    warn!(position = %position, error = %e, "Failed to collect fees, continuing");
                }
            }
        }

        // Step 2: Decrease liquidity to zero
        result.status = ExitStatus::DecreasingLiquidity;
        match self.decrease_all_liquidity(position).await {
            Ok(liquidity) => {
                result.liquidity_removed = Some(liquidity);
                info!(position = %position, liquidity = liquidity, "Liquidity removed");
            }
            Err(e) => {
                error!(position = %position, error = %e, "Failed to decrease liquidity");
                result.status = ExitStatus::Failed;
                result.error = Some(e.to_string());
                return result;
            }
        }

        // Step 3: Close position
        result.status = ExitStatus::ClosingPosition;
        match self.close_position(position).await {
            Ok(()) => {
                result.status = ExitStatus::Completed;
                info!(position = %position, "Position closed successfully");
            }
            Err(e) => {
                error!(position = %position, error = %e, "Failed to close position");
                result.status = ExitStatus::Failed;
                result.error = Some(e.to_string());
            }
        }

        result
    }

    /// Collects fees from a position.
    async fn collect_fees(&self, position: &Pubkey) -> anyhow::Result<(u64, u64)> {
        let monitored = self
            .monitor
            .get_position(position)
            .await
            .ok_or_else(|| anyhow::anyhow!("position not in monitor"))?;
        let pool = monitored.pool;
        if let Some(ref r) = self.rebalance_executor {
            r.emergency_collect_fees(position, &pool).await
        } else {
            warn!(position = %position, "EmergencyExitManager: no RebalanceExecutor — skipping fee collection");
            Ok((0, 0))
        }
    }

    /// Decreases all liquidity from a position.
    async fn decrease_all_liquidity(&self, position: &Pubkey) -> anyhow::Result<u128> {
        let monitored = self
            .monitor
            .get_position(position)
            .await
            .ok_or_else(|| anyhow::anyhow!("position not in monitor"))?;
        let pool = monitored.pool;
        if let Some(ref r) = self.rebalance_executor {
            r.emergency_decrease_all_liquidity(position, &pool).await
        } else {
            warn!(position = %position, "EmergencyExitManager: no RebalanceExecutor — skipping decrease");
            Ok(0)
        }
    }

    /// Closes a position.
    async fn close_position(&self, position: &Pubkey) -> anyhow::Result<()> {
        let monitored = self
            .monitor
            .get_position(position)
            .await
            .ok_or_else(|| anyhow::anyhow!("position not in monitor"))?;
        let pool = monitored.pool;
        if let Some(ref r) = self.rebalance_executor {
            r.emergency_close_position(position, &pool).await
        } else {
            warn!(position = %position, "EmergencyExitManager: no RebalanceExecutor — skipping close");
            Ok(())
        }
    }

    /// Gets the results of the last exit.
    pub async fn get_results(&self) -> Vec<ExitResult> {
        self.results.read().await.clone()
    }

    /// Checks if an exit is in progress.
    pub async fn is_in_progress(&self) -> bool {
        *self.in_progress.read().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_exit_config_default() {
        let config = EmergencyExitConfig::default();
        assert!(config.collect_fees);
        assert_eq!(config.max_slippage_bps, 100);
    }
}
