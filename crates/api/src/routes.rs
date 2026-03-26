//! Route definitions.

use crate::handlers;
use crate::state::AppState;
use crate::websocket;
use axum::{
    Router,
    routing::{delete, get, post, put},
};

/// Creates the API router with all routes.
pub fn create_router(state: AppState) -> Router {
    Router::new()
        // Health routes
        .route("/health", get(handlers::health_check))
        .route("/health/live", get(handlers::liveness))
        .route("/health/ready", get(handlers::readiness))
        .route("/metrics", get(handlers::metrics))
        // Auth routes
        .route("/auth/phantom/challenge", post(handlers::phantom_challenge))
        .route("/auth/phantom/verify", post(handlers::phantom_verify))
        // Position routes
        .route("/positions", get(handlers::list_positions))
        .route("/positions", post(handlers::open_position))
        .route("/positions/{address}", get(handlers::get_position))
        .route("/positions/{address}", delete(handlers::close_position))
        .route("/positions/{address}/collect", post(handlers::collect_fees))
        .route(
            "/positions/{address}/decrease",
            post(handlers::decrease_liquidity),
        )
        .route(
            "/positions/{address}/rebalance",
            post(handlers::rebalance_position),
        )
        .route("/positions/{address}/pnl", get(handlers::get_position_pnl))
        // Strategy routes
        .route("/strategies", get(handlers::list_strategies))
        .route("/strategies", post(handlers::create_strategy))
        .route("/strategies/{id}", get(handlers::get_strategy))
        .route("/strategies/{id}", put(handlers::update_strategy))
        .route("/strategies/{id}", delete(handlers::delete_strategy))
        .route("/strategies/{id}/start", post(handlers::start_strategy))
        .route("/strategies/{id}/stop", post(handlers::stop_strategy))
        .route(
            "/strategies/{id}/apply-optimize-result",
            post(handlers::apply_optimize_result),
        )
        .route(
            "/strategies/{id}/performance",
            get(handlers::get_strategy_performance),
        )
        // Pool routes
        .route("/pools", get(handlers::list_pools))
        .route("/pools/{address}", get(handlers::get_pool))
        .route("/pools/{address}/state", get(handlers::get_pool_state))
        // Orca REST proxy routes
        .route("/orca/pools", get(handlers::orca_list_pools))
        .route("/orca/pools/search", get(handlers::orca_search_pools))
        .route("/orca/pools/{address}", get(handlers::orca_get_pool))
        .route("/orca/lock/{address}", get(handlers::orca_get_lock_info))
        .route("/orca/tokens", get(handlers::orca_list_tokens))
        .route("/orca/tokens/search", get(handlers::orca_search_tokens))
        .route("/orca/tokens/{mint}", get(handlers::orca_get_token))
        .route("/orca/protocol", get(handlers::orca_get_protocol))
        // Unsiged tx flow routes
        .route("/tx/open/build", post(handlers::tx_open_build))
        .route("/tx/decrease/build", post(handlers::tx_decrease_build))
        .route("/tx/collect/build", post(handlers::tx_collect_build))
        .route("/tx/close/build", post(handlers::tx_close_build))
        .route("/tx/submit-signed", post(handlers::tx_submit_signed))
        // Analytics routes
        .route(
            "/analytics/portfolio",
            get(handlers::get_portfolio_analytics),
        )
        .route("/analytics/simulate", post(handlers::run_simulation))
        // WebSocket routes
        .route("/ws/positions", get(websocket::positions_ws))
        .route("/ws/alerts", get(websocket::alerts_ws))
        // Add state
        .with_state(state)
}

/// Creates the API router with versioning prefix.
pub fn create_versioned_router(state: AppState) -> Router {
    Router::new().nest("/api/v1", create_router(state))
}
