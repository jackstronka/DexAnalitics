//! Application state shared across handlers.

#[cfg(feature = "broker-event-bus")]
use crate::events::BrokerEventBus;
use crate::events::{
    EVENT_ALERT_RAISED, EVENT_POSITION_UPDATED, EventBus, EventEnvelope, InProcessEventBus,
    publish_with_retry,
};
use clmm_lp_data::repositories::Database;
use clmm_lp_execution::prelude::{
    CircuitBreaker, LifecycleTracker, PositionMonitor, StrategyExecutor, TransactionManager,
};
use clmm_lp_protocols::prelude::{RpcConfig, RpcProvider};
use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;
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
    /// PostgreSQL connection wrapper (optional; API still serves read-only endpoints without it).
    pub db: Option<Database>,
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
    /// Best-effort throttling for DB ingest of JSONL ledgers (avoid re-reading files too often).
    pub ledger_ingest_last_at: Arc<RwLock<Option<Instant>>>,
    /// Optional active signer wallet id selected via API (`/wallets/active-signer`).
    pub active_signer_wallet_id: Arc<RwLock<Option<String>>>,
}

impl AppState {
    /// Creates a new application state.
    pub fn new(rpc_config: RpcConfig, api_config: ApiConfig, db: Option<Database>) -> Self {
        // For safety, default to dry-run when DRY_RUN is not set.
        let dry_run = env::var("DRY_RUN")
            .ok()
            .map(|v| {
                matches!(
                    v.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "y"
                )
            })
            .unwrap_or(true);

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

        // Load persisted strategies (best-effort). If the store is missing or invalid, start empty.
        let persisted_strategies =
            crate::strategy_store::try_load_persisted_strategies().unwrap_or_default();
        let persisted_map: HashMap<String, StrategyState> = persisted_strategies
            .into_iter()
            .map(|p| {
                let id = p.id.clone();
                let strategy = StrategyState {
                    id: id.clone(),
                    name: p.name,
                    running: false, // Do not auto-start after API restart.
                    config: p.config,
                    created_at: p.created_at,
                    updated_at: p.updated_at,
                };
                (id, strategy)
            })
            .collect();

        Self {
            provider,
            db,
            monitor,
            tx_manager,
            circuit_breaker,
            lifecycle,
            strategies: Arc::new(RwLock::new(persisted_map)),
            position_updates: position_tx,
            alert_updates: alert_tx,
            config: api_config,
            executors: Arc::new(RwLock::new(HashMap::new())),
            optimization_busy: Arc::new(RwLock::new(HashMap::new())),
            dry_run,
            phantom_nonces: Arc::new(RwLock::new(HashMap::new())),
            event_bus,
            ledger_ingest_last_at: Arc::new(RwLock::new(None)),
            active_signer_wallet_id: Arc::new(RwLock::new(None)),
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
    /// Request timeout in seconds for **on-chain / tx** endpoints (open/close/swap/rebalance).
    pub onchain_request_timeout_secs: u64,
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
    /// Repository root on the API host (`tools/scripts-manifest.json`, `data/script_runs.jsonl`).
    pub repo_root: Option<String>,
    /// Local script runner base URL, e.g. `http://127.0.0.1:9847` (see `tools/script_runner/`).
    pub script_runner_url: Option<String>,
    /// Bearer token shared with the runner (`CLMM_SCRIPT_RUNNER_TOKEN` on the runner host).
    pub script_runner_token: Option<String>,
    /// Directory with wallet keypair JSON files on the API host (used by `GET /wallets`).
    pub wallets_dir: Option<String>,
    /// Primary wallet directory on API host (preferred over `wallets_dir` when set).
    pub wallets_dir_primary: Option<String>,
    /// Secondary wallet directory on API host for redundancy.
    pub wallets_dir_secondary: Option<String>,
    /// Base (EVM) JSON-RPC URL for read-only calls (`BASE_RPC_URL`). Optional: Slipstream endpoints return 503 if unset.
    pub base_rpc_url: Option<String>,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 8080,
            api_keys: vec![],
            enable_cors: true,
            request_timeout_secs: 30,
            onchain_request_timeout_secs: 120,
            rate_limit_per_minute: 100,
            orca_public_api_base_url: None,
            event_bus_mode: "inprocess".to_string(),
            event_bus_backend: "nats".to_string(),
            event_bus_shadow_mode: true,
            event_bus_max_retries: 3,
            repo_root: None,
            script_runner_url: None,
            script_runner_token: None,
            wallets_dir: None,
            wallets_dir_primary: None,
            wallets_dir_secondary: None,
            base_rpc_url: None,
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

/// Persist strategies to disk (best-effort; never fails the request).
pub(crate) fn try_persist_strategies_best_effort(strategies: &HashMap<String, StrategyState>) {
    if !crate::strategy_store::enabled() {
        return;
    }

    let persisted: Vec<crate::strategy_store::PersistedStrategy> = strategies
        .values()
        .map(|s| crate::strategy_store::PersistedStrategy {
            id: s.id.clone(),
            name: s.name.clone(),
            config: s.config.clone(),
            created_at: s.created_at,
            updated_at: s.updated_at,
        })
        .collect();

    if let Err(e) = crate::strategy_store::try_save_persisted_strategies(&persisted) {
        tracing::warn!(error = %e, "Failed to persist strategies store");
    }
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
