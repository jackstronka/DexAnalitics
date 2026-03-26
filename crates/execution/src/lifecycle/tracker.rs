//! Lifecycle tracker for position history.

use super::{
    EventData, FeesCollectedData, LifecycleEvent, LifecycleEventType, LiquidityChangeData,
    PositionClosedData, PositionOpenedData, RebalanceData,
};
use rust_decimal::Decimal;
use solana_sdk::pubkey::Pubkey;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Summary of a position's lifecycle.
#[derive(Debug, Clone)]
pub struct PositionSummary {
    /// Position address.
    pub position: Pubkey,
    /// Pool address.
    pub pool: Pubkey,
    /// When position was opened.
    pub opened_at: chrono::DateTime<chrono::Utc>,
    /// When position was closed (if closed).
    pub closed_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Initial entry value in USD.
    pub entry_value_usd: Decimal,
    /// Current/final value in USD.
    pub current_value_usd: Decimal,
    /// Total fees collected.
    pub total_fees_usd: Decimal,
    /// Number of rebalances.
    pub rebalance_count: u32,
    /// Total transaction costs in lamports.
    pub total_tx_costs_lamports: u64,
    /// Total IL percentage.
    pub total_il_pct: Decimal,
    /// Net PnL in USD.
    pub net_pnl_usd: Decimal,
    /// Net PnL percentage.
    pub net_pnl_pct: Decimal,
    /// Whether position is still open.
    pub is_open: bool,
}

/// Tracks lifecycle events for all positions.
pub struct LifecycleTracker {
    /// Events by position.
    events: Arc<RwLock<HashMap<Pubkey, Vec<LifecycleEvent>>>>,
    /// Position summaries.
    summaries: Arc<RwLock<HashMap<Pubkey, PositionSummary>>>,
    /// Optional JSONL path for IL / rebalance ledger lines.
    il_ledger_path: Arc<Mutex<Option<PathBuf>>>,
    /// Monotonic id for optimize applications (optional stamping on events).
    optimization_seq: Arc<AtomicU64>,
}

impl LifecycleTracker {
    /// Creates a new lifecycle tracker.
    #[must_use]
    pub fn new() -> Self {
        Self {
            events: Arc::new(RwLock::new(HashMap::new())),
            summaries: Arc::new(RwLock::new(HashMap::new())),
            il_ledger_path: Arc::new(Mutex::new(None)),
            optimization_seq: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Sets optional JSONL path for IL-oriented lifecycle rows (rebalance, etc.).
    pub fn set_il_ledger_path(&self, path: Option<PathBuf>) {
        if let Ok(mut g) = self.il_ledger_path.lock() {
            *g = path;
        }
    }

    /// Bump after a successful optimize JSON apply; returns new id as string.
    pub fn bump_optimization_run_id(&self) -> String {
        let n = self.optimization_seq.fetch_add(1, Ordering::SeqCst) + 1;
        format!("opt-{n}")
    }

    async fn append_il_ledger_jsonl(&self, row: serde_json::Value) {
        let path_opt = self.il_ledger_path.lock().ok().and_then(|g| g.clone());
        let Some(path) = path_opt else {
            return;
        };
        let line = match serde_json::to_string(&row) {
            Ok(s) => s,
            Err(e) => {
                warn!(error = %e, "il ledger serialize failed");
                return;
            }
        };
        let res = tokio::task::spawn_blocking(move || {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)?;
            writeln!(f, "{line}")?;
            Ok::<(), std::io::Error>(())
        })
        .await;
        if let Err(e) = res {
            warn!(error = %e, "il ledger task failed");
        }
    }

    /// Records a position opened event.
    pub async fn record_position_opened(
        &self,
        position: Pubkey,
        pool: Pubkey,
        data: PositionOpenedData,
    ) {
        let event = LifecycleEvent::new(
            LifecycleEventType::PositionOpened,
            position,
            pool,
            EventData::PositionOpened(data.clone()),
        );

        self.add_event(position, event.clone()).await;

        // Create summary
        let summary = PositionSummary {
            position,
            pool,
            opened_at: event.timestamp,
            closed_at: None,
            entry_value_usd: data.entry_value_usd,
            current_value_usd: data.entry_value_usd,
            total_fees_usd: Decimal::ZERO,
            rebalance_count: 0,
            total_tx_costs_lamports: 0,
            total_il_pct: Decimal::ZERO,
            net_pnl_usd: Decimal::ZERO,
            net_pnl_pct: Decimal::ZERO,
            is_open: true,
        };

        self.summaries.write().await.insert(position, summary);

        info!(
            position = %position,
            tick_lower = data.tick_lower,
            tick_upper = data.tick_upper,
            "Position opened"
        );

        let row = serde_json::json!({
            "schema_version": 1,
            "event": "position_opened",
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "position": position.to_string(),
            "pool": pool.to_string(),
            "amount_a": data.amount_a,
            "amount_b": data.amount_b,
            "entry_price": data.entry_price.to_string(),
            "entry_value_usd": data.entry_value_usd.to_string(),
            "price_ab": data.price_ab.map(|d| d.to_string()),
        });
        self.append_il_ledger_jsonl(row).await;
    }

    /// Records a liquidity change event.
    pub async fn record_liquidity_change(
        &self,
        position: Pubkey,
        pool: Pubkey,
        data: LiquidityChangeData,
    ) {
        let event_type = if data.is_increase {
            LifecycleEventType::LiquidityIncreased
        } else {
            LifecycleEventType::LiquidityDecreased
        };

        let event = LifecycleEvent::new(
            event_type,
            position,
            pool,
            EventData::LiquidityChange(data.clone()),
        );

        self.add_event(position, event).await;

        debug!(
            position = %position,
            is_increase = data.is_increase,
            delta = data.liquidity_delta,
            "Liquidity changed"
        );
    }

    /// Records a rebalance event.
    pub async fn record_rebalance(&self, position: Pubkey, pool: Pubkey, data: RebalanceData) {
        let event = LifecycleEvent::new(
            LifecycleEventType::Rebalanced,
            position,
            pool,
            EventData::Rebalance(data.clone()),
        );

        self.add_event(position, event).await;

        // Update summary
        if let Some(summary) = self.summaries.write().await.get_mut(&position) {
            summary.rebalance_count += 1;
            summary.total_tx_costs_lamports += data.tx_cost_lamports;
        }

        info!(
            position = %position,
            old_range = format!("[{}, {}]", data.old_tick_lower, data.old_tick_upper),
            new_range = format!("[{}, {}]", data.new_tick_lower, data.new_tick_upper),
            reason = ?data.reason,
            "Position rebalanced"
        );

        let row = serde_json::json!({
            "schema_version": 1,
            "event": "rebalance",
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "position": position.to_string(),
            "pool": pool.to_string(),
            "amount_a_before": data.amount_a_before,
            "amount_b_before": data.amount_b_before,
            "amount_a_after": data.amount_a_after,
            "amount_b_after": data.amount_b_after,
            "price_ab_before": data.price_ab_before.map(|d| d.to_string()),
            "price_ab_after": data.price_ab_after.map(|d| d.to_string()),
            "fees_a_collected": data.fees_a_collected,
            "fees_b_collected": data.fees_b_collected,
            "il_at_rebalance": data.il_at_rebalance.to_string(),
            "reason": format!("{:?}", data.reason),
            "optimization_run_id": data.optimization_run_id,
        });
        self.append_il_ledger_jsonl(row).await;
    }

    /// Records a fees collected event.
    pub async fn record_fees_collected(
        &self,
        position: Pubkey,
        pool: Pubkey,
        data: FeesCollectedData,
    ) {
        let event = LifecycleEvent::new(
            LifecycleEventType::FeesCollected,
            position,
            pool,
            EventData::FeesCollected(data.clone()),
        );

        self.add_event(position, event).await;

        // Update summary
        if let Some(summary) = self.summaries.write().await.get_mut(&position) {
            summary.total_fees_usd += data.fees_usd;
        }

        info!(
            position = %position,
            fees_a = data.fees_a,
            fees_b = data.fees_b,
            fees_usd = %data.fees_usd,
            "Fees collected"
        );
    }

    /// Records a position closed event.
    pub async fn record_position_closed(
        &self,
        position: Pubkey,
        pool: Pubkey,
        data: PositionClosedData,
    ) {
        let event = LifecycleEvent::new(
            LifecycleEventType::PositionClosed,
            position,
            pool,
            EventData::PositionClosed(data.clone()),
        );

        self.add_event(position, event.clone()).await;

        // Update summary
        if let Some(summary) = self.summaries.write().await.get_mut(&position) {
            summary.closed_at = Some(event.timestamp);
            summary.is_open = false;
            summary.net_pnl_usd = data.final_pnl_usd;
            summary.net_pnl_pct = data.final_pnl_pct;
            summary.total_il_pct = data.total_il_pct;
        }

        info!(
            position = %position,
            pnl_usd = %data.final_pnl_usd,
            pnl_pct = %data.final_pnl_pct,
            duration_hours = data.duration_hours,
            reason = ?data.reason,
            "Position closed"
        );

        let row = serde_json::json!({
            "schema_version": 1,
            "event": "position_closed",
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "position": position.to_string(),
            "pool": pool.to_string(),
            "amount_a": data.amount_a,
            "amount_b": data.amount_b,
            "price_ab": data.price_ab.map(|d| d.to_string()),
            "total_il_pct": data.total_il_pct.to_string(),
            "final_pnl_usd": data.final_pnl_usd.to_string(),
        });
        self.append_il_ledger_jsonl(row).await;
    }

    /// Adds an event to the tracker.
    async fn add_event(&self, position: Pubkey, event: LifecycleEvent) {
        let mut events = self.events.write().await;
        events.entry(position).or_default().push(event);
    }

    /// Gets all events for a position.
    pub async fn get_events(&self, position: &Pubkey) -> Vec<LifecycleEvent> {
        self.events
            .read()
            .await
            .get(position)
            .cloned()
            .unwrap_or_default()
    }

    /// Gets the summary for a position.
    pub async fn get_summary(&self, position: &Pubkey) -> Option<PositionSummary> {
        self.summaries.read().await.get(position).cloned()
    }

    /// Gets all position summaries.
    pub async fn get_all_summaries(&self) -> Vec<PositionSummary> {
        self.summaries.read().await.values().cloned().collect()
    }

    /// Gets summaries for open positions only.
    pub async fn get_open_positions(&self) -> Vec<PositionSummary> {
        self.summaries
            .read()
            .await
            .values()
            .filter(|s| s.is_open)
            .cloned()
            .collect()
    }

    /// Gets summaries for closed positions only.
    pub async fn get_closed_positions(&self) -> Vec<PositionSummary> {
        self.summaries
            .read()
            .await
            .values()
            .filter(|s| !s.is_open)
            .cloned()
            .collect()
    }

    /// Gets aggregate statistics.
    pub async fn get_aggregate_stats(&self) -> AggregateStats {
        let summaries = self.summaries.read().await;

        let mut stats = AggregateStats::default();

        for summary in summaries.values() {
            stats.total_positions += 1;
            if summary.is_open {
                stats.open_positions += 1;
            } else {
                stats.closed_positions += 1;
            }

            stats.total_fees_usd += summary.total_fees_usd;
            stats.total_pnl_usd += summary.net_pnl_usd;
            stats.total_rebalances += summary.rebalance_count;
            stats.total_tx_costs_lamports += summary.total_tx_costs_lamports;
        }

        if stats.total_positions > 0 {
            stats.avg_pnl_pct = summaries.values().map(|s| s.net_pnl_pct).sum::<Decimal>()
                / Decimal::from(stats.total_positions);
        }

        stats
    }
}

impl Default for LifecycleTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Aggregate statistics across all positions.
#[derive(Debug, Clone, Default)]
pub struct AggregateStats {
    /// Total positions tracked.
    pub total_positions: u32,
    /// Currently open positions.
    pub open_positions: u32,
    /// Closed positions.
    pub closed_positions: u32,
    /// Total fees earned in USD.
    pub total_fees_usd: Decimal,
    /// Total PnL in USD.
    pub total_pnl_usd: Decimal,
    /// Average PnL percentage.
    pub avg_pnl_pct: Decimal,
    /// Total rebalances performed.
    pub total_rebalances: u32,
    /// Total transaction costs in lamports.
    pub total_tx_costs_lamports: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_lifecycle_tracker() {
        let tracker = LifecycleTracker::new();
        let position = Pubkey::new_unique();
        let pool = Pubkey::new_unique();

        // Record position opened
        tracker
            .record_position_opened(
                position,
                pool,
                PositionOpenedData {
                    tick_lower: -1000,
                    tick_upper: 1000,
                    liquidity: 1000000,
                    amount_a: 1000000000,
                    amount_b: 100000000,
                    entry_price: Decimal::new(100, 0),
                    entry_value_usd: Decimal::new(1000, 0),
                    price_ab: None,
                },
            )
            .await;

        let events = tracker.get_events(&position).await;
        assert_eq!(events.len(), 1);

        let summary = tracker.get_summary(&position).await;
        assert!(summary.is_some());
        assert!(summary.unwrap().is_open);
    }
}
