//! Health check handlers.

use crate::error::ApiResult;
use crate::models::{
    CircuitBreakerStatus, ComponentHealth, EventBusMetricsResponse, HealthResponse,
    MetricsResponse, ServiceStatus,
};
use crate::state::AppState;
use axum::{Json, extract::State};
use clmm_lp_execution::prelude::CircuitState;
use std::time::Instant;

/// Start time for uptime calculation.
static START_TIME: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

/// Request counter.
static REQUEST_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Error counter.
static ERROR_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Initializes the start time.
pub fn init_start_time() {
    START_TIME.get_or_init(Instant::now);
}

/// Increments the request counter.
pub fn increment_request_count() {
    REQUEST_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// Increments the error counter.
pub fn increment_error_count() {
    ERROR_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// Health check endpoint.
///
/// Returns the current health status of the service.
#[utoipa::path(
    get,
    path = "/health",
    tag = "Health",
    responses(
        (status = 200, description = "Service health status", body = HealthResponse)
    )
)]
pub async fn health_check(State(state): State<AppState>) -> ApiResult<Json<HealthResponse>> {
    let uptime = START_TIME.get().map(|t| t.elapsed().as_secs()).unwrap_or(0);

    let circuit_state = state.circuit_breaker.state().await;
    let circuit_status = match circuit_state {
        CircuitState::Closed => CircuitBreakerStatus::Closed,
        CircuitState::Open => CircuitBreakerStatus::Open,
        CircuitState::HalfOpen => CircuitBreakerStatus::HalfOpen,
    };

    // Check RPC health
    let rpc_healthy = state.provider.get_slot().await.is_ok();

    let status = if rpc_healthy && circuit_state == CircuitState::Closed {
        ServiceStatus::Healthy
    } else if rpc_healthy {
        ServiceStatus::Degraded
    } else {
        ServiceStatus::Unhealthy
    };

    let response = HealthResponse {
        status,
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_secs: uptime,
        components: ComponentHealth {
            rpc: rpc_healthy,
            database: true, // Placeholder - no DB yet
            circuit_breaker: circuit_status,
        },
    };

    Ok(Json(response))
}

/// Liveness probe endpoint.
///
/// Simple endpoint that returns 200 if the service is running.
#[utoipa::path(
    get,
    path = "/health/live",
    tag = "Health",
    responses(
        (status = 200, description = "Service is alive")
    )
)]
pub async fn liveness() -> &'static str {
    "OK"
}

/// Readiness probe endpoint.
///
/// Returns 200 if the service is ready to accept requests.
#[utoipa::path(
    get,
    path = "/health/ready",
    tag = "Health",
    responses(
        (status = 200, description = "Service is ready"),
        (status = 503, description = "Service is not ready")
    )
)]
pub async fn readiness(State(state): State<AppState>) -> Result<&'static str, &'static str> {
    // Check if RPC is available
    if state.provider.get_slot().await.is_ok() {
        Ok("OK")
    } else {
        Err("NOT READY")
    }
}

/// Metrics endpoint.
///
/// Returns service metrics.
#[utoipa::path(
    get,
    path = "/metrics",
    tag = "Health",
    responses(
        (status = 200, description = "Service metrics", body = MetricsResponse)
    )
)]
pub async fn metrics(State(state): State<AppState>) -> ApiResult<Json<MetricsResponse>> {
    let positions = state.monitor.get_positions().await;
    let strategies = state.strategies.read().await;

    let response = MetricsResponse {
        request_count: REQUEST_COUNT.load(std::sync::atomic::Ordering::Relaxed),
        error_count: ERROR_COUNT.load(std::sync::atomic::Ordering::Relaxed),
        avg_response_time_ms: 0.0, // Placeholder
        active_ws_connections: 0,  // Placeholder
        positions_monitored: positions.len() as u32,
        strategies_running: strategies.values().filter(|s| s.running).count() as u32,
        event_bus: Some({
            let stats = state.event_bus.stats();
            EventBusMetricsResponse {
                published: stats.published,
                retries: stats.retries,
                duplicates: stats.duplicates,
                failed: stats.failed,
                dlq_size: stats.dlq_size,
            }
        }),
    };

    Ok(Json(response))
}
