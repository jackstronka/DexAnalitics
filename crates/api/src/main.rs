//! Bociarz LP Strategy Lab API Server.
//!
//! This binary starts the REST API server with WebSocket support.

use anyhow::Result;
use clmm_lp_api::server::{ApiServer, ServerConfig, shutdown_signal};
use clmm_lp_api::state::ApiConfig;
use clmm_lp_protocols::prelude::RpcConfig;
use std::env;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    info!("Starting Bociarz LP Strategy Lab API Server");

    // Load configuration from environment
    let config = load_config_from_env();

    info!(
        host = %config.host,
        port = config.port,
        "Server configuration loaded"
    );

    // Create and run server
    let server = ApiServer::new(config);
    server.run_with_shutdown(shutdown_signal()).await?;

    Ok(())
}

/// Loads server configuration from environment variables.
fn load_config_from_env() -> ServerConfig {
    let host = env::var("API_HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port = env::var("API_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);

    let rpc_url = env::var("SOLANA_RPC_URL")
        .unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".to_string());

    let rpc_config = RpcConfig {
        primary_url: rpc_url,
        ..Default::default()
    };

    let api_config = ApiConfig {
        orca_public_api_base_url: env::var("ORCA_PUBLIC_API_BASE_URL").ok(),
        event_bus_mode: env::var("EVENT_BUS_MODE").unwrap_or_else(|_| "inprocess".to_string()),
        event_bus_backend: env::var("EVENT_BUS_BACKEND").unwrap_or_else(|_| "nats".to_string()),
        event_bus_shadow_mode: env::var("EVENT_BUS_SHADOW_MODE")
            .map(|v| v == "true")
            .unwrap_or(true),
        event_bus_max_retries: env::var("EVENT_BUS_MAX_RETRIES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3),
        enable_cors: env::var("API_CORS_ALLOW_ALL")
            .map(|v| v == "true")
            .unwrap_or(true),
        rate_limit_per_minute: env::var("API_RATE_LIMIT_RPM")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(100),
        request_timeout_secs: env::var("API_REQUEST_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30),
        ..Default::default()
    };

    ServerConfig {
        host,
        port,
        rpc_config,
        api_config,
    }
}
