//! Application state shared across handlers.

use crate::events::{
    EVENT_ALERT_RAISED, EVENT_POSITION_UPDATED, EventBus, EventEnvelope, InProcessEventBus,
    publish_with_retry,
};
#[cfg(feature = "broker-event-bus")]
use crate::events::BrokerEventBus;
use clmm_lp_execution::prelude::{
    CircuitBreaker, LifecycleTracker, PositionMonitor, StrategyExecutor, TransactionManager,
};
use clmm_lp_protocols::prelude::{RpcConfig, RpcProvider};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::sync::{RwLock, broadcast};

#[derive(Debug, Clone)]
pub struct PhantomNonceEntry {
    pub message: String,
    pub expires_at: u64,
}

/// Application state shared across all handlers.
#[derive(Clone)]
pub struct AppState {
    /// RPC provider.
    pub provider: Arc<RpcProvider>,
    /// Position monitor.
    pub monitor: Arc<PositionMonitor>,
    /// Transaction manager.
    pub tx_manager: Arc<TransactionManager>,
    /// Circuit breaker.
    pub circuit_breaker: Arc<CircuitBreaker>,
    /// Lifecycle tracker.
    pub lifecycle: Arc<LifecycleTracker>,
    /// Active strategies.
    pub strategies: Arc<RwLock<HashMap<String, StrategyState>>>,
    /// WebSocket broadcast channel for position updates.
    pub position_updates: broadcast::Sender<PositionUpdate>,
    /// WebSocket broadcast channel for alerts.
    pub alert_updates: broadcast::Sender<AlertUpdate>,
    /// API configuration.
    pub config: ApiConfig,
    /// Strategy executors by ID.
    pub executors: Arc<RwLock<HashMap<String, Arc<RwLock<StrategyExecutor>>>>>,
    /// Prevents overlapping optimize subprocess cycles and `POST /apply-optimize-result` applies per strategy.
    pub optimization_busy: Arc<RwLock<HashMap<String, Arc<AtomicBool>>>>,
    /// Whether in dry-run mode.
    pub dry_run: bool,
    /// Phantom signMessage nonces (in-memory, short-lived).
    pub phantom_nonces: Arc<RwLock<HashMap<String, PhantomNonceEntry>>>,
    /// Async event bus for cross-component communication.
    pub event_bus: Arc<dyn EventBus>,
}

impl AppState {
    /// Creates a new application state.
    pub fn new(rpc_config: RpcConfig, api_config: ApiConfig) -> Self {
        let provider = Arc::new(RpcProvider::new(rpc_config));
        let monitor = Arc::new(PositionMonitor::new(
            provider.clone(),
            clmm_lp_execution::prelude::MonitorConfig::default(),
        ));
        let tx_manager = Arc::new(TransactionManager::new(
            provider.clone(),
            clmm_lp_execution::prelude::TransactionConfig::default(),
        ));
        let circuit_breaker = Arc::new(CircuitBreaker::default());
        let lifecycle = Arc::new(LifecycleTracker::new());

        let (position_tx, _) = broadcast::channel(1000);
        let (alert_tx, _) = broadcast::channel(1000);
        let event_bus: Arc<dyn EventBus> = match api_config.event_bus_mode.as_str() {
            "broker" => {
                #[cfg(feature = "broker-event-bus")]
                {
                    Arc::new(BrokerEventBus::new(
                        api_config.event_bus_backend.clone(),
                        api_config.event_bus_shadow_mode,
                    ))
                }
                #[cfg(not(feature = "broker-event-bus"))]
                {
                    tracing::warn!(
                        "EVENT_BUS_MODE=broker requested but crate feature `broker-event-bus` is disabled; using inprocess"
                    );
                    Arc::new(InProcessEventBus::new())
                }
            }
            _ => Arc::new(InProcessEventBus::new()),
        };

        Self {
            provider,
            monitor,
            tx_manager,
            circuit_breaker,
            lifecycle,
            strategies: Arc::new(RwLock::new(HashMap::new())),
            position_updates: position_tx,
            alert_updates: alert_tx,
            config: api_config,
            executors: Arc::new(RwLock::new(HashMap::new())),
            optimization_busy: Arc::new(RwLock::new(HashMap::new())),
            dry_run: true, // Default to dry-run for safety
            phantom_nonces: Arc::new(RwLock::new(HashMap::new())),
            event_bus,
        }
    }

    /// Sets dry-run mode.
    pub fn set_dry_run(&mut self, dry_run: bool) {
        self.dry_run = dry_run;
    }

    /// Broadcasts a position update.
    pub async fn broadcast_position_update(&self, update: PositionUpdate) {
        let _ = self.position_updates.send(update.clone());
        let event = EventEnvelope::new(
            EVENT_POSITION_UPDATED,
            "clmm-lp-api",
            serde_json::json!({
                "update_type": update.update_type,
                "position_address": update.position_address,
                "timestamp": update.timestamp,
                "data": update.data,
            }),
        );
        if let Err(e) = publish_with_retry(
            self.event_bus.as_ref(),
            event.clone(),
            self.config.event_bus_max_retries,
        )
        .await
        {
            tracing::warn!(error = %e, "event bus publish position.updated failed after retries");
        }
    }

    /// Broadcasts an alert update.
    pub async fn broadcast_alert(&self, alert: AlertUpdate) {
        let _ = self.alert_updates.send(alert.clone());
        let event = EventEnvelope::new(
            EVENT_ALERT_RAISED,
            "clmm-lp-api",
            serde_json::json!({
                "level": alert.level,
                "message": alert.message,
                "timestamp": alert.timestamp,
                "position_address": alert.position_address,
            }),
        );
        if let Err(e) = publish_with_retry(
            self.event_bus.as_ref(),
            event.clone(),
            self.config.event_bus_max_retries,
        )
        .await
        {
            tracing::warn!(error = %e, "event bus publish alert.raised failed after retries");
        }
    }

    /// Subscribes to position updates.
    pub fn subscribe_positions(&self) -> broadcast::Receiver<PositionUpdate> {
        self.position_updates.subscribe()
    }

    /// Subscribes to alert updates.
    pub fn subscribe_alerts(&self) -> broadcast::Receiver<AlertUpdate> {
        self.alert_updates.subscribe()
    }
}

/// API configuration.
#[derive(Debug, Clone)]
pub struct ApiConfig {
    /// Server host.
    pub host: String,
    /// Server port.
    pub port: u16,
    /// API keys for authentication.
    pub api_keys: Vec<String>,
    /// Whether to enable CORS.
    pub enable_cors: bool,
    /// Request timeout in seconds.
    pub request_timeout_secs: u64,
    /// Rate limit per minute.
    pub rate_limit_per_minute: u32,
    /// Override Orca public API base URL (otherwise env `ORCA_PUBLIC_API_BASE_URL` or default).
    pub orca_public_api_base_url: Option<String>,
    /// Async bus mode: `inprocess` or `broker`.
    pub event_bus_mode: String,
    /// Broker backend selection: `nats`, `redis`, `kafka` (adapter scaffold).
    pub event_bus_backend: String,
    /// If true, broker adapter runs in shadow mode.
    pub event_bus_shadow_mode: bool,
    /// Max publish retries before DLQ/failure path.
    pub event_bus_max_retries: u8,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 8080,
            api_keys: vec![],
            enable_cors: true,
            request_timeout_secs: 30,
            rate_limit_per_minute: 100,
            orca_public_api_base_url: None,
            event_bus_mode: "inprocess".to_string(),
            event_bus_backend: "nats".to_string(),
            event_bus_shadow_mode: true,
            event_bus_max_retries: 3,
        }
    }
}

/// State for an active strategy.
#[derive(Debug, Clone)]
pub struct StrategyState {
    /// Strategy ID.
    pub id: String,
    /// Strategy name.
    pub name: String,
    /// Whether strategy is running.
    pub running: bool,
    /// Strategy configuration as JSON.
    pub config: serde_json::Value,
    /// Created timestamp.
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Last updated timestamp.
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Position update for WebSocket broadcast.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PositionUpdate {
    /// Update type.
    pub update_type: String,
    /// Position address.
    pub position_address: String,
    /// Timestamp.
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Update data.
    pub data: serde_json::Value,
}

/// Alert update for WebSocket broadcast.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AlertUpdate {
    /// Alert level.
    pub level: String,
    /// Alert message.
    pub message: String,
    /// Timestamp.
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Related position (if any).
    pub position_address: Option<String>,
}
