//! Position monitor for real-time tracking.

use crate::alerts::{Alert, AlertRule};
use clmm_lp_domain::metrics::impermanent_loss::calculate_il_concentrated;
use clmm_lp_protocols::prelude::*;
use rust_decimal::Decimal;
use solana_sdk::pubkey::Pubkey;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::interval;
use tracing::{debug, error, info, warn};

/// Configuration for position monitoring.
#[derive(Debug, Clone)]
pub struct MonitorConfig {
    /// Polling interval in seconds.
    pub poll_interval_secs: u64,
    /// Whether to enable alerts.
    pub alerts_enabled: bool,
    /// IL threshold for warning alerts (as percentage).
    pub il_warning_threshold: Decimal,
    /// IL threshold for critical alerts (as percentage).
    pub il_critical_threshold: Decimal,
    /// Range exit alert enabled.
    pub range_exit_alert: bool,
}

impl Default for MonitorConfig {
    fn default() -> Self {
        Self {
            poll_interval_secs: 30,
            alerts_enabled: true,
            il_warning_threshold: Decimal::new(5, 2),   // 5%
            il_critical_threshold: Decimal::new(10, 2), // 10%
            range_exit_alert: true,
        }
    }
}

/// Monitored position state.
#[derive(Debug, Clone)]
pub struct MonitoredPosition {
    /// Position address.
    pub address: Pubkey,
    /// Pool address.
    pub pool: Pubkey,
    /// Current on-chain state.
    pub on_chain: OnChainPosition,
    /// PnL tracker for this position.
    pub pnl: PositionPnL,
    /// Whether position is currently in range.
    pub in_range: bool,
    /// Last update timestamp.
    pub last_updated: chrono::DateTime<chrono::Utc>,
}

/// PnL data for a position.
#[derive(Debug, Clone, Default)]
pub struct PositionPnL {
    /// Entry price (token B per token A) captured when the position is first added.
    /// Used to compute IL for `IlLimit` mode.
    pub entry_price: Option<Decimal>,
    /// Entry value in USD.
    pub entry_value_usd: Decimal,
    /// Current value in USD.
    pub current_value_usd: Decimal,
    /// Fees earned in token A.
    pub fees_earned_a: u64,
    /// Fees earned in token B.
    pub fees_earned_b: u64,
    /// Fees in USD.
    pub fees_usd: Decimal,
    /// Impermanent loss percentage.
    pub il_pct: Decimal,
    /// Net PnL in USD.
    pub net_pnl_usd: Decimal,
    /// Net PnL percentage.
    pub net_pnl_pct: Decimal,
    /// Annualized return.
    pub apy: Decimal,
}

/// Position monitor for tracking multiple positions.
pub struct PositionMonitor {
    /// RPC provider.
    #[allow(dead_code)]
    provider: Arc<RpcProvider>,
    /// Whirlpool reader.
    pool_reader: WhirlpoolReader,
    /// Position reader.
    position_reader: PositionReader,
    /// Monitored positions.
    positions: Arc<RwLock<HashMap<Pubkey, MonitoredPosition>>>,
    /// Configuration.
    config: MonitorConfig,
    /// Alert rules.
    alert_rules: Vec<AlertRule>,
    /// Alert callback.
    #[allow(dead_code)]
    alert_callback: Option<Box<dyn Fn(Alert) + Send + Sync>>,
}

impl PositionMonitor {
    /// Creates a new position monitor.
    pub fn new(provider: Arc<RpcProvider>, config: MonitorConfig) -> Self {
        let pool_reader = WhirlpoolReader::new(provider.clone());
        let position_reader = PositionReader::new(provider.clone());

        Self {
            provider,
            pool_reader,
            position_reader,
            positions: Arc::new(RwLock::new(HashMap::new())),
            config,
            alert_rules: Vec::new(),
            alert_callback: None,
        }
    }

    /// Adds a position to monitor.
    pub async fn add_position(&self, position_address: &str) -> anyhow::Result<()> {
        let position = self.position_reader.get_position(position_address).await?;

        let pool_state = self
            .pool_reader
            .get_pool_state(&position.pool.to_string())
            .await?;
        let in_range = pool_state.is_tick_in_range(position.tick_lower, position.tick_upper);
        let entry_price = pool_state.price;
        let lower_price = clmm_lp_protocols::prelude::tick_to_price(position.tick_lower);
        let upper_price = clmm_lp_protocols::prelude::tick_to_price(position.tick_upper);
        let il_pct = calculate_il_concentrated(entry_price, entry_price, lower_price, upper_price)
            .unwrap_or(Decimal::ZERO);

        // Ensure IL starts at 0 (or very close) right after being added.
        let pnl = PositionPnL {
            entry_price: Some(entry_price),
            il_pct,
            ..PositionPnL::default()
        };

        let monitored = MonitoredPosition {
            address: position.address,
            pool: position.pool,
            on_chain: position.clone(),
            pnl,
            in_range,
            last_updated: chrono::Utc::now(),
        };

        let mut positions = self.positions.write().await;
        positions.insert(position.address, monitored);

        info!(position = position_address, "Added position to monitor");

        Ok(())
    }

    /// Removes a position from monitoring.
    pub async fn remove_position(&self, position_address: &Pubkey) {
        let mut positions = self.positions.write().await;
        positions.remove(position_address);

        info!(
            position = %position_address,
            "Removed position from monitor"
        );
    }

    /// Gets all monitored positions.
    pub async fn get_positions(&self) -> Vec<MonitoredPosition> {
        let positions = self.positions.read().await;
        positions.values().cloned().collect()
    }

    /// Gets a specific position.
    pub async fn get_position(&self, address: &Pubkey) -> Option<MonitoredPosition> {
        let positions = self.positions.read().await;
        positions.get(address).cloned()
    }

    /// Updates all monitored positions.
    pub async fn update_all(&self) -> anyhow::Result<()> {
        let position_addresses: Vec<Pubkey> = {
            let positions = self.positions.read().await;
            positions.keys().copied().collect()
        };

        for address in position_addresses {
            if let Err(e) = self.update_position(&address).await {
                error!(
                    position = %address,
                    error = %e,
                    "Failed to update position"
                );
            }
        }

        Ok(())
    }

    /// Updates a single position.
    async fn update_position(&self, address: &Pubkey) -> anyhow::Result<()> {
        let position = self
            .position_reader
            .get_position(&address.to_string())
            .await?;
        let pool_state = self
            .pool_reader
            .get_pool_state(&position.pool.to_string())
            .await?;

        // Check if in range
        let in_range = pool_state.is_tick_in_range(position.tick_lower, position.tick_upper);

        // Calculate token amounts
        let (amount_a, amount_b) = self.position_reader.calculate_token_amounts(
            &position,
            pool_state.tick_current,
            pool_state.sqrt_price,
        );

        // Update position state
        let mut positions = self.positions.write().await;
        if let Some(monitored) = positions.get_mut(address) {
            let was_in_range = monitored.in_range;

            monitored.on_chain = position.clone();
            monitored.in_range = in_range;
            monitored.last_updated = chrono::Utc::now();

            // Update PnL
            monitored.pnl.fees_earned_a = position.fees_owed_a;
            monitored.pnl.fees_earned_b = position.fees_owed_b;

            // Update IL percentage if we have an entry price.
            if let Some(entry_price) = monitored.pnl.entry_price {
                let lower_price = clmm_lp_protocols::prelude::tick_to_price(position.tick_lower);
                let upper_price = clmm_lp_protocols::prelude::tick_to_price(position.tick_upper);
                let il = calculate_il_concentrated(
                    entry_price,
                    pool_state.price,
                    lower_price,
                    upper_price,
                )
                .unwrap_or(Decimal::ZERO);
                monitored.pnl.il_pct = il;
            } else {
                // If entry price wasn't set, treat current price as entry to avoid NaNs.
                monitored.pnl.entry_price = Some(pool_state.price);
                monitored.pnl.il_pct = Decimal::ZERO;
            }

            debug!(
                position = %address,
                in_range = in_range,
                amount_a = amount_a,
                amount_b = amount_b,
                "Updated position state"
            );

            // Check for range exit
            if was_in_range && !in_range && self.config.range_exit_alert {
                warn!(
                    position = %address,
                    "Position exited range"
                );
                // TODO: Trigger alert
            }
        }

        Ok(())
    }

    /// Starts the monitoring loop.
    pub async fn start(&self) {
        let poll_interval = Duration::from_secs(self.config.poll_interval_secs);
        let mut ticker = interval(poll_interval);

        info!(
            interval_secs = self.config.poll_interval_secs,
            "Starting position monitor"
        );

        loop {
            ticker.tick().await;

            if let Err(e) = self.update_all().await {
                error!(error = %e, "Monitor update failed");
            }
        }
    }

    /// Adds an alert rule.
    pub fn add_alert_rule(&mut self, rule: AlertRule) {
        self.alert_rules.push(rule);
    }

    /// Sets the alert callback.
    pub fn set_alert_callback<F>(&mut self, callback: F)
    where
        F: Fn(Alert) + Send + Sync + 'static,
    {
        self.alert_callback = Some(Box::new(callback));
    }

    /// Gets aggregate portfolio metrics.
    pub async fn get_portfolio_metrics(&self) -> PortfolioMetrics {
        let positions = self.positions.read().await;

        let mut metrics = PortfolioMetrics::default();

        for pos in positions.values() {
            metrics.total_positions += 1;
            metrics.total_value_usd += pos.pnl.current_value_usd;
            metrics.total_fees_usd += pos.pnl.fees_usd;
            metrics.total_pnl_usd += pos.pnl.net_pnl_usd;

            if pos.in_range {
                metrics.positions_in_range += 1;
            }
        }

        if metrics.total_positions > 0 {
            metrics.avg_il_pct = positions.values().map(|p| p.pnl.il_pct).sum::<Decimal>()
                / Decimal::from(metrics.total_positions);
        }

        metrics
    }
}

#[cfg(test)]
impl PositionMonitor {
    /// Inserts a monitored position without RPC (for unit tests).
    pub async fn insert_test_monitored_position(&self, pos: MonitoredPosition) {
        let mut positions = self.positions.write().await;
        positions.insert(pos.address, pos);
    }
}

/// Aggregate portfolio metrics.
#[derive(Debug, Clone, Default)]
pub struct PortfolioMetrics {
    /// Total number of positions.
    pub total_positions: u32,
    /// Positions currently in range.
    pub positions_in_range: u32,
    /// Total portfolio value in USD.
    pub total_value_usd: Decimal,
    /// Total fees earned in USD.
    pub total_fees_usd: Decimal,
    /// Total PnL in USD.
    pub total_pnl_usd: Decimal,
    /// Average IL percentage.
    pub avg_il_pct: Decimal,
}
