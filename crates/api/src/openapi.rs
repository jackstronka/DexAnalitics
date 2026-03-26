//! OpenAPI documentation configuration.
//!
//! Provides Swagger UI and OpenAPI spec generation using utoipa.

use crate::handlers;
use crate::models::{
    BuildUnsignedTxRequest, BuildUnsignedTxResponse, CreateStrategyRequest,
    DecreaseLiquidityRequest, EventBusMetricsResponse, HealthResponse, ListPoolsResponse,
    ListPositionsResponse, ListStrategiesResponse, MessageResponse, MetricsResponse,
    OpenPositionRequest, OrcaLockResponse, OrcaProtocolResponse, OrcaTokenListResponse,
    OrcaTokenResponse, PhantomChallengeRequest, PhantomChallengeResponse,
    PhantomSessionResponse, PhantomVerifyRequest, PnLResponse, PoolResponse, PoolStateResponse,
    PortfolioAnalyticsResponse, PositionResponse, RebalanceRequest, SimulationRequest,
    SimulationResponse, StrategyPerformanceResponse, StrategyResponse, SubmitSignedTxRequest,
    SubmitSignedTxResponse,
};
use utoipa::OpenApi;

/// OpenAPI documentation structure.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "CLMM LP Strategy Optimizer API",
        version = "0.1.1-alpha.3",
        description = "REST API for Bociarz LP Strategy Lab (derived from CLMM Liquidity Provider). \
                       Provides endpoints for position management, strategy automation, \
                       pool analysis, and portfolio analytics.",
        license(
            name = "MIT",
            url = "https://github.com/joaquinbejar/CLMM-Liquidity-Provider"
        ),
        contact(name = "Bociarz")
    ),
    servers(
        (url = "/api/v1", description = "API v1")
    ),
    tags(
        (name = "Health", description = "Health check and metrics endpoints"),
        (name = "Auth", description = "Authentication endpoints"),
        (name = "Positions", description = "LP position management"),
        (name = "Strategies", description = "Automated strategy management"),
        (name = "Pools", description = "Pool information and state"),
        (name = "Orca", description = "Orca Public REST proxy endpoints"),
        (name = "Transactions", description = "Unsigned tx build + submit endpoints"),
        (name = "Analytics", description = "Portfolio analytics and simulations")
    ),
    paths(
        // Health endpoints
        handlers::health_check,
        handlers::liveness,
        handlers::readiness,
        handlers::metrics,
        // Auth endpoints
        handlers::phantom_challenge,
        handlers::phantom_verify,
        // Position endpoints
        handlers::list_positions,
        handlers::get_position,
        handlers::open_position,
        handlers::close_position,
        handlers::collect_fees,
        handlers::decrease_liquidity,
        handlers::rebalance_position,
        handlers::get_position_pnl,
        // Strategy endpoints
        handlers::list_strategies,
        handlers::get_strategy,
        handlers::create_strategy,
        handlers::update_strategy,
        handlers::delete_strategy,
        handlers::start_strategy,
        handlers::stop_strategy,
        handlers::apply_optimize_result,
        handlers::get_strategy_performance,
        // Pool endpoints
        handlers::list_pools,
        handlers::get_pool,
        handlers::get_pool_state,
        // Orca REST proxy endpoints
        handlers::orca_list_pools,
        handlers::orca_search_pools,
        handlers::orca_get_pool,
        handlers::orca_get_lock_info,
        handlers::orca_list_tokens,
        handlers::orca_search_tokens,
        handlers::orca_get_token,
        handlers::orca_get_protocol,
        // Unsigned tx flow endpoints
        handlers::tx_open_build,
        handlers::tx_decrease_build,
        handlers::tx_collect_build,
        handlers::tx_close_build,
        handlers::tx_submit_signed,
        // Analytics endpoints
        handlers::get_portfolio_analytics,
        handlers::run_simulation,
    ),
    components(
        schemas(
            // Health
            HealthResponse,
            MetricsResponse,
            EventBusMetricsResponse,
            // Auth
            PhantomChallengeRequest,
            PhantomChallengeResponse,
            PhantomVerifyRequest,
            PhantomSessionResponse,
            // Positions
            ListPositionsResponse,
            PositionResponse,
            PnLResponse,
            OpenPositionRequest,
            DecreaseLiquidityRequest,
            RebalanceRequest,
            MessageResponse,
            // Strategies
            ListStrategiesResponse,
            StrategyResponse,
            StrategyPerformanceResponse,
            CreateStrategyRequest,
            // Pools
            ListPoolsResponse,
            PoolResponse,
            PoolStateResponse,
            // Orca REST proxy
            OrcaLockResponse,
            OrcaTokenResponse,
            OrcaTokenListResponse,
            OrcaProtocolResponse,
            // Transactions
            BuildUnsignedTxRequest,
            BuildUnsignedTxResponse,
            SubmitSignedTxRequest,
            SubmitSignedTxResponse,
            // Analytics
            PortfolioAnalyticsResponse,
            SimulationRequest,
            SimulationResponse,
        )
    ),
    modifiers(&SecurityAddon)
)]
pub struct ApiDoc;

/// Security addon for OpenAPI.
struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = openapi.components.as_mut() {
            components.add_security_scheme(
                "api_key",
                utoipa::openapi::security::SecurityScheme::ApiKey(
                    utoipa::openapi::security::ApiKey::Header(
                        utoipa::openapi::security::ApiKeyValue::new("X-API-Key"),
                    ),
                ),
            );
            components.add_security_scheme(
                "bearer_auth",
                utoipa::openapi::security::SecurityScheme::Http(
                    utoipa::openapi::security::HttpBuilder::new()
                        .scheme(utoipa::openapi::security::HttpAuthScheme::Bearer)
                        .bearer_format("JWT")
                        .build(),
                ),
            );
        }
    }
}

/// Returns the OpenAPI JSON specification.
#[must_use]
pub fn openapi_json() -> String {
    ApiDoc::openapi().to_json().unwrap_or_default()
}

/// Returns the OpenAPI YAML specification.
#[must_use]
pub fn openapi_yaml() -> String {
    ApiDoc::openapi().to_pretty_json().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_openapi_generation() {
        let json = openapi_json();
        assert!(!json.is_empty());
        assert!(json.contains("CLMM LP Strategy Optimizer API"));
    }

    #[test]
    fn test_openapi_yaml() {
        let yaml = openapi_yaml();
        assert!(!yaml.is_empty());
        assert!(yaml.contains("CLMM LP Strategy Optimizer API"));
    }
}
