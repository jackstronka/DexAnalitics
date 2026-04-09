//! Server configuration and startup.

use crate::handlers::health::init_start_time;
use crate::middleware::{RateLimiter, request_logging};
use crate::openapi::ApiDoc;
use crate::position_registry_seed::seed_monitor_from_registry;
use crate::routes::create_versioned_router;
use crate::state::{ApiConfig, AppState};
use axum::{Router, middleware};
use clmm_lp_data::repositories::Database;
use clmm_lp_protocols::prelude::RpcConfig;
use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::time::{Duration, timeout};
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};
use tracing::info;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

fn env_autostart_strategies_on_boot() -> bool {
    match std::env::var("CLMM_STRATEGY_AUTOSTART_ON_BOOT") {
        Ok(v) => matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "y" | "on"
        ),
        Err(_) => false,
    }
}

async fn maybe_autostart_strategies(state: &AppState) {
    if !env_autostart_strategies_on_boot() {
        return;
    }

    let ids: Vec<String> = {
        let map = state.strategies.read().await;
        map.values()
            .filter_map(|s| {
                let auto_start = s
                    .config
                    .get("parameters")
                    .and_then(|p| p.get("auto_start"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if auto_start {
                    Some(s.id.clone())
                } else {
                    None
                }
            })
            .collect()
    };

    if ids.is_empty() {
        info!("Strategy autostart enabled but no strategies opted in (parameters.auto_start=true)");
        return;
    }

    info!(count = ids.len(), "Auto-starting opted-in strategies on API boot");
    let svc = crate::services::StrategyService::new(state.clone());
    for id in ids {
        match svc.start_strategy(&id).await {
            Ok(res) => {
                if res.success {
                    info!(strategy_id = %id, "Auto-started strategy");
                } else {
                    tracing::warn!(
                        strategy_id = %id,
                        error = ?res.error,
                        "Strategy autostart failed"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(strategy_id = %id, error = %e, "Strategy autostart errored");
            }
        }
    }
}

/// Server configuration.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Host to bind to.
    pub host: String,
    /// Port to bind to.
    pub port: u16,
    /// RPC configuration.
    pub rpc_config: RpcConfig,
    /// API configuration.
    pub api_config: ApiConfig,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 8080,
            rpc_config: RpcConfig::default(),
            api_config: ApiConfig::default(),
        }
    }
}

/// API server.
pub struct ApiServer {
    /// Server configuration.
    config: ServerConfig,
    /// Application state.
    state: AppState,
}

impl ApiServer {
    /// Creates a new API server.
    pub async fn new(config: ServerConfig) -> Self {
        let db = connect_db_best_effort().await;
        let state = AppState::new(config.rpc_config.clone(), config.api_config.clone(), db);
        Self { config, state }
    }

    /// Creates a new API server with custom state.
    pub fn with_state(config: ServerConfig, state: AppState) -> Self {
        Self { config, state }
    }

    /// Gets the application state.
    pub fn state(&self) -> &AppState {
        &self.state
    }

    /// Builds the router with all middleware.
    pub fn build_router(&self) -> Router {
        let _api_keys: HashSet<String> = self.config.api_config.api_keys.iter().cloned().collect();
        let _rate_limiter = Arc::new(RateLimiter::new(
            self.config.api_config.rate_limit_per_minute,
        ));

        let mut router = create_versioned_router(
            self.state.clone(),
            self.config.api_config.request_timeout_secs,
            self.config.api_config.onchain_request_timeout_secs,
        );

        // Add Swagger UI at /docs
        router =
            router.merge(SwaggerUi::new("/docs").url("/api-docs/openapi.json", ApiDoc::openapi()));

        // Add middleware
        router = router.layer(middleware::from_fn(request_logging));

        // Add CORS if enabled
        if self.config.api_config.enable_cors {
            let cors = CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any);
            router = router.layer(cors);
        }

        // Timeouts are applied per-router in `routes`:
        // - regular requests: `request_timeout_secs`
        // - on-chain tx endpoints: `onchain_request_timeout_secs`

        // Add tracing
        router = router.layer(TraceLayer::new_for_http());

        router
    }

    /// Starts the server.
    pub async fn run(self) -> anyhow::Result<()> {
        init_start_time();

        let addr: SocketAddr = format!("{}:{}", self.config.host, self.config.port).parse()?;

        // Best-effort: re-add open positions into monitor after restart.
        seed_monitor_from_registry(self.state.monitor.clone()).await;
        maybe_autostart_strategies(&self.state).await;

        let router = self.build_router();

        info!(address = %addr, "Starting API server");

        let listener = TcpListener::bind(addr).await?;
        axum::serve(listener, router).await?;

        Ok(())
    }

    /// Starts the server with graceful shutdown.
    pub async fn run_with_shutdown(
        self,
        shutdown_signal: impl std::future::Future<Output = ()> + Send + 'static,
    ) -> anyhow::Result<()> {
        init_start_time();

        let addr: SocketAddr = format!("{}:{}", self.config.host, self.config.port).parse()?;

        // Best-effort: re-add open positions into monitor after restart.
        seed_monitor_from_registry(self.state.monitor.clone()).await;
        maybe_autostart_strategies(&self.state).await;

        let router = self.build_router();

        info!(address = %addr, "Starting API server with graceful shutdown");

        let listener = TcpListener::bind(addr).await?;
        axum::serve(listener, router)
            .with_graceful_shutdown(shutdown_signal)
            .await?;

        info!("API server stopped");

        Ok(())
    }
}

async fn connect_db_best_effort() -> Option<Database> {
    let url = std::env::var("DATABASE_URL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let Some(database_url) = url else {
        tracing::warn!("DATABASE_URL not set; DB-backed stream performance will be unavailable");
        return None;
    };

    // On dev machines Postgres may be stopped; never block API startup on DB.
    let connect_timeout = Duration::from_secs(3);
    let db = match timeout(connect_timeout, Database::connect(&database_url)).await {
        Ok(Ok(db)) => db,
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "DB connect failed (continuing without DB)");
            return None;
        }
        Err(_) => {
            tracing::warn!(
                timeout_secs = connect_timeout.as_secs(),
                "DB connect timed out (continuing without DB)"
            );
            return None;
        }
    };

    match timeout(connect_timeout, db.migrate()).await {
        Ok(Ok(())) => Some(db),
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "DB migrate failed (continuing without DB)");
            None
        }
        Err(_) => {
            tracing::warn!(
                timeout_secs = connect_timeout.as_secs(),
                "DB migrate timed out (continuing without DB)"
            );
            None
        }
    }
}

/// Creates a shutdown signal that listens for Ctrl+C.
pub async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to install Ctrl+C handler");
    info!("Shutdown signal received");
}
