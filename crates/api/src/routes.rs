//! Route definitions.

use crate::handlers;
use crate::state::AppState;
use crate::websocket;
use axum::{
    Router,
    routing::{delete, get, post, put},
};
use std::time::Duration;
use tower_http::timeout::TimeoutLayer;

/// Creates the API router with all routes.
///
/// Note: timeouts are applied in [`create_versioned_router`]. This helper is kept for internal
/// callers (e.g. tests / prelude) and returns the merged router without timeout layers.
pub fn create_router(state: AppState) -> Router {
    create_base_router(state.clone()).merge(create_onchain_router(state))
}

fn create_base_router(state: AppState) -> Router {
    Router::new()
        // Health routes
        .route("/health", get(handlers::health_check))
        .route("/health/live", get(handlers::liveness))
        .route("/health/ready", get(handlers::readiness))
        .route("/metrics", get(handlers::metrics))
        // Auth routes
        .route("/auth/phantom/challenge", post(handlers::phantom_challenge))
        .route("/auth/phantom/verify", post(handlers::phantom_verify))
        // Position routes (read-only + lightweight)
        .route("/positions", get(handlers::list_positions))
        .route("/positions/closed", get(handlers::list_closed_positions))
        .route("/positions/{address}/pnl", get(handlers::get_position_pnl))
        .route(
            "/positions/{address}/suggest-strategy",
            get(handlers::suggest_position_strategy),
        )
        .route(
            "/positions/{address}/stream-performance",
            get(handlers::get_position_stream_performance),
        )
        .route(
            "/positions/{address}/stream-pnl",
            get(handlers::get_position_stream_pnl),
        )
        .route(
            "/positions/{address}/diagnostics",
            get(handlers::get_position_diagnostics),
        )
        .route(
            "/positions/{address}/experiment-config",
            get(handlers::get_position_experiment_config),
        )
        // Strategy routes
        .route("/strategies", get(handlers::list_strategies))
        .route("/strategies", post(handlers::create_strategy))
        .route("/strategies/{id}", get(handlers::get_strategy))
        .route("/strategies/{id}", put(handlers::update_strategy))
        .route("/strategies/{id}", delete(handlers::delete_strategy))
        .route("/strategies/{id}/start", post(handlers::start_strategy))
        .route(
            "/strategies/{id}/position-executor",
            post(handlers::set_strategy_position_executor),
        )
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
        .route(
            "/pools/{address}/estimate-swap-cost",
            get(handlers::get_swap_cost_estimate),
        )
        .route(
            "/pools/{address}/quote-open-budget",
            post(handlers::quote_open_budget),
        )
        // Orca REST proxy routes
        .route("/orca/pools", get(handlers::orca_list_pools))
        .route("/orca/pools/search", get(handlers::orca_search_pools))
        .route("/orca/pools/{address}", get(handlers::orca_get_pool))
        .route("/orca/lock/{address}", get(handlers::orca_get_lock_info))
        .route("/orca/tokens", get(handlers::orca_list_tokens))
        .route("/orca/tokens/search", get(handlers::orca_search_tokens))
        .route("/orca/tokens/{mint}", get(handlers::orca_get_token))
        .route("/orca/protocol", get(handlers::orca_get_protocol))
        .route(
            "/orca/positions-by-owner",
            get(handlers::orca_positions_by_owner),
        )
        // Unsiged tx flow routes (build = lightweight)
        .route("/tx/open/build", post(handlers::tx_open_build))
        .route("/tx/increase/build", post(handlers::tx_increase_build))
        .route("/tx/decrease/build", post(handlers::tx_decrease_build))
        .route("/tx/collect/build", post(handlers::tx_collect_build))
        .route("/tx/close/build", post(handlers::tx_close_build))
        // Analytics routes
        .route(
            "/analytics/portfolio",
            get(handlers::get_portfolio_analytics),
        )
        .route("/analytics/simulate", post(handlers::run_simulation))
        // Backtests (CLI subprocess)
        .route(
            "/backtests/from-closed-position",
            post(handlers::backtest_from_closed_position),
        )
        .route(
            "/backtests/from-open-position",
            post(handlers::backtest_from_open_position),
        )
        .route("/backtests/{id}", get(handlers::get_backtest_job))
        // Bot activity (JSONL ledger / registry; Slack digest)
        .route("/bot-activity/ledger", get(handlers::get_bot_ledger))
        .route("/bot-activity/il-ledger", get(handlers::get_bot_il_ledger))
        .route("/bot-activity/registry", get(handlers::get_bot_registry))
        .route(
            "/bot-activity/pending-open",
            get(handlers::get_pending_open_recovery),
        )
        .route(
            "/bot-activity/stranded-rebalances",
            get(handlers::get_stranded_rebalances),
        )
        .route(
            "/bot-activity/stranded-rebalances/reconcile",
            post(handlers::reconcile_stranded_rebalances),
        )
        .route(
            "/bot-activity/stranded-rebalances/{session_id}/dismiss",
            post(handlers::dismiss_stranded_rebalance),
        )
        .route(
            "/bot-activity/slack-summary",
            post(handlers::post_bot_slack_summary),
        )
        // Tools scripts (manifest + runner proxy)
        .route("/scripts", get(handlers::list_scripts))
        .route("/scripts/{id}/run", post(handlers::run_script))
        // Wallets (local keypairs directory + on-chain balances)
        .route("/wallets", get(handlers::list_wallets))
        .route("/wallets/balances", get(handlers::get_wallet_balances))
        .route("/wallets/api-signer", get(handlers::get_api_signer_wallet))
        .route("/wallets/convert-sol", post(handlers::convert_sol))
        // Prices (free external sources; server-side fetch)
        .route("/prices/jupiter", get(handlers::get_jupiter_prices))
        // Base EVM — Aerodrome Slipstream (read-only; needs BASE_RPC_URL)
        .route(
            "/evm/base/aerodrome-slipstream/pools/{pool}/slot0",
            get(handlers::get_aerodrome_slipstream_pool_slot0),
        )
        // WebSocket routes
        .route("/ws/positions", get(websocket::positions_ws))
        .route("/ws/alerts", get(websocket::alerts_ws))
        // Add state
        .with_state(state)
}

fn create_onchain_router(state: AppState) -> Router {
    Router::new()
        // Position routes (on-chain / can take long)
        .route("/positions/{address}", get(handlers::get_position))
        .route("/positions", post(handlers::open_position))
        .route(
            "/positions/swap-before-open",
            post(handlers::swap_before_open),
        )
        // Backfill can scan lifecycle JSONL + write many DB rows.
        .route(
            "/positions/backfill-valuation-snapshots",
            post(handlers::backfill_valuation_snapshots),
        )
        .route(
            "/positions/{address}/strategy",
            post(handlers::link_position_strategy),
        )
        .route(
            "/positions/{address}/heal-strategy-link",
            post(handlers::heal_position_strategy_link),
        )
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
        // Long-running reads (JSONL scans / lineage reconstruction)
        .route(
            "/positions/{address}/stream-lineage",
            get(handlers::get_position_stream_lineage),
        )
        .route(
            "/positions/{address}/lifecycle-summary",
            get(handlers::get_position_lifecycle_summary),
        )
        // Unsiged tx flow routes (submit can take long)
        .route("/tx/submit-signed", post(handlers::tx_submit_signed))
        // Add state
        .with_state(state)
}

/// Creates the API router with versioning prefix.
pub fn create_versioned_router(
    state: AppState,
    request_timeout_secs: u64,
    onchain_request_timeout_secs: u64,
) -> Router {
    #[allow(deprecated)]
    let base = create_base_router(state.clone())
        .layer(TimeoutLayer::new(Duration::from_secs(request_timeout_secs)));
    #[allow(deprecated)]
    let onchain = create_onchain_router(state).layer(TimeoutLayer::new(Duration::from_secs(
        onchain_request_timeout_secs,
    )));
    Router::new().nest("/api/v1", base.merge(onchain))
}
