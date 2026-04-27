//! OpenAPI documentation configuration.
//!
//! Provides Swagger UI and OpenAPI spec generation using utoipa.

use crate::handlers;
use crate::models::{
    AgentChatMessage, AgentChatResponse, AgentChatUiPayload, AgentDecisionRow,
    AgentDecisionWriteRequest, AgentDecisionWriteResponse, AgentDecisionsQuery,
    AgentDecisionsResponse, AgentLlmContext, AgentLlmReplyRequest, AgentLlmReplyResponse,
    AgentMessageRequest, AgentPositionSession, AgentPositionSupervisorResponse, AgentScanRequest,
    AgentScanResponse, AgentSessionRequest, AgentSessionResponse, AgentSupervisorScenario,
    ActiveSignerResponse, AgentWorkerSettings, AgentWorkerSettingsUpdateRequest, AgentWorkerStatus,
    ApiSignerWalletResponse,
    BacktestFromClosedPositionRequest,
    BacktestFromOpenPositionRequest, BacktestJobResponse, BacktestJobStatusResponse, BotActivityJsonlResponse,
    BotRegistryJsonlResponse, BuildUnsignedTxRequest, BuildUnsignedTxResponse, ClosedPositionEntry,
    ClosedPositionsResponse, ConvertSolDirection, ConvertSolRequest, ConvertSolResponse,
    CreateWalletRequest, CreateWalletResponse,
    CreateStrategyRequest, DecreaseLiquidityRequest, EventBusMetricsResponse, HealthResponse,
    JupiterPricesResponse, LineageChainCostSummary, LinkPositionStrategyRequest, ListPoolsResponse,
    ListPositionsResponse, ListStrategiesResponse, MarketDataQuery, MarketSnapshotRow,
    MarketSnapshotsResponse, MarketSwapRow, MarketSwapsResponse, MessageResponse, MetricsResponse,
    OpenPositionRequest, OrcaLockResponse, OrcaOwnerPositionEntry, OrcaOwnerPositionsResponse,
    OrcaProtocolResponse, OrcaTokenListResponse, OrcaTokenResponse, PhantomChallengeRequest,
    PhantomChallengeResponse, PhantomSessionResponse, PhantomVerifyRequest, PnLResponse,
    PoolResponse, PoolStateResponse, PortfolioAnalyticsResponse, PositionDiagnosticsResponse,
    PositionExperimentConfigResponse, PositionLastEvalSnapshot, PositionLifecycleEvent,
    PositionLifecycleSessionSummary, PositionLifecycleSummaryResponse, PositionOpenResponse,
    PositionResponse, PositionStrategyDiagnostics, PositionStreamLineageNode,
    PositionStreamLineageResponse, PositionStreamPerformanceResponse, PositionStreamPnLResponse,
    QuoteOpenBudgetRequest, QuoteOpenBudgetResponse, RebalanceRequest, RunScriptRequest,
    ScriptCatalogItem, ScriptRunRecord, ScriptsListResponse, SimulationRequest, SimulationResponse,
    SlackActivitySummaryRequest, SlackActivitySummaryResponse, SlipstreamSlot0Response,
    StrandedRebalanceItem, StrandedRebalancesResponse, StrategyPerformanceResponse,
    StrategyPositionExecutorRequest, StrategyResponse, SubmitSignedTxRequest,
    SetActiveSignerRequest, SubmitSignedTxResponse, SwapBeforeOpenRequest, SwapBeforeOpenResponse,
    SwapCostEstimateResponse, SwapInPoolBeforeOpen, UncollectedFeesInfo, WalletBalancesResponse,
    WalletEntry, WalletReplicationStatus, WalletTokenBalance, WalletTransferRequest,
    WalletTransferResponse, WalletsListResponse,
};
use utoipa::OpenApi;

/// OpenAPI documentation structure.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Bociarz LP API",
        version = "0.2.0-alpha.1",
        description = "REST API for Bociarz LP (derived from CLMM Liquidity Provider). \
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
        (name = "Analytics", description = "Portfolio analytics and simulations"),
        (name = "Bot activity", description = "Orca JSONL ledger + registry (CLI/bot history); Slack digest"),
        (name = "Scripts", description = "tools/*.ps1 manifest, run history, localhost runner proxy"),
        (name = "Wallets", description = "Local keypairs directory + on-chain balances"),
        (name = "Prices", description = "Free external price sources (server-side fetch)"),
        (name = "Evm", description = "Base chain JSON-RPC read helpers (Aerodrome Slipstream)"),
        (name = "Data", description = "Normalized local JSONL feeds for orchestration and joins"),
        (name = "Agent", description = "Per-position agent supervision and chat")
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
        handlers::list_closed_positions,
        handlers::get_position,
        handlers::get_position_agent_chat,
        handlers::get_position_agent_chat_ui,
        handlers::start_position_agent,
        handlers::post_position_agent_message,
        handlers::post_position_agent_llm_reply,
        handlers::trigger_position_agent_scan,
        handlers::get_position_agent_supervisor,
        handlers::get_agent_worker_settings,
        handlers::put_agent_worker_settings,
        handlers::get_agent_worker_status,
        handlers::link_position_strategy,
        handlers::heal_position_strategy_link,
        handlers::open_position,
        handlers::swap_before_open,
        handlers::close_position,
        handlers::collect_fees,
        handlers::decrease_liquidity,
        handlers::rebalance_position,
        handlers::get_position_pnl,
        handlers::get_position_diagnostics,
        handlers::get_position_stream_performance,
        handlers::get_position_stream_pnl,
        handlers::get_position_stream_lineage,
        handlers::get_position_lifecycle_summary,
        handlers::get_position_experiment_config,
        // Strategy endpoints
        handlers::list_strategies,
        handlers::get_strategy,
        handlers::create_strategy,
        handlers::update_strategy,
        handlers::delete_strategy,
        handlers::start_strategy,
        handlers::set_strategy_position_executor,
        handlers::stop_strategy,
        handlers::apply_optimize_result,
        handlers::get_strategy_performance,
        // Pool endpoints
        handlers::list_pools,
        handlers::get_pool,
        handlers::get_pool_state,
        handlers::get_swap_cost_estimate,
        handlers::quote_open_budget,
        handlers::get_data_snapshots,
        handlers::get_data_swaps,
        handlers::get_agent_decisions,
        handlers::post_agent_decision,
        // Orca REST proxy endpoints
        handlers::orca_list_pools,
        handlers::orca_search_pools,
        handlers::orca_get_pool,
        handlers::orca_get_lock_info,
        handlers::orca_list_tokens,
        handlers::orca_search_tokens,
        handlers::orca_get_token,
        handlers::orca_get_protocol,
        handlers::orca_positions_by_owner,
        // Unsigned tx flow endpoints
        handlers::tx_open_build,
        handlers::tx_increase_build,
        handlers::tx_decrease_build,
        handlers::tx_collect_build,
        handlers::tx_close_build,
        handlers::tx_submit_signed,
        // Analytics endpoints
        handlers::get_portfolio_analytics,
        handlers::run_simulation,
        handlers::backtest_from_closed_position,
        handlers::backtest_from_open_position,
        handlers::get_backtest_job,
        handlers::start_backtest_full,
        handlers::get_backtest_full_job,
        handlers::start_backtest_auto_tune,
        handlers::stop_backtest_auto_tune,
        handlers::get_backtest_auto_tune_status,
        handlers::apply_backtest_auto_tune_to_strategy,
        // Bot activity
        handlers::get_bot_ledger,
        handlers::get_bot_il_ledger,
        handlers::get_bot_registry,
        handlers::get_pending_open_recovery,
        handlers::get_stranded_rebalances,
        handlers::reconcile_stranded_rebalances,
        handlers::dismiss_stranded_rebalance,
        handlers::post_bot_slack_summary,
        // Scripts
        handlers::list_scripts,
        handlers::run_script,
        // Wallets
        handlers::list_wallets,
        handlers::create_wallet,
        handlers::get_wallet_balances,
        handlers::get_api_signer_wallet,
        handlers::get_active_signer,
        handlers::set_active_signer,
        handlers::convert_sol,
        handlers::transfer_sol_between_wallets,
        // Prices
        handlers::get_jupiter_prices,
        handlers::get_aerodrome_slipstream_pool_slot0,
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
            AgentSessionRequest,
            AgentPositionSession,
            AgentSessionResponse,
            AgentMessageRequest,
            AgentLlmContext,
            AgentLlmReplyRequest,
            AgentLlmReplyResponse,
            AgentChatMessage,
            AgentChatResponse,
            AgentChatUiPayload,
            AgentScanRequest,
            AgentScanResponse,
            AgentSupervisorScenario,
            AgentPositionSupervisorResponse,
            AgentWorkerSettings,
            AgentWorkerSettingsUpdateRequest,
            AgentWorkerStatus,
            PnLResponse,
            UncollectedFeesInfo,
            ClosedPositionsResponse,
            ClosedPositionEntry,
            PositionLifecycleSummaryResponse,
            PositionLifecycleSessionSummary,
            PositionLifecycleEvent,
            PositionExperimentConfigResponse,
            PositionDiagnosticsResponse,
            PositionStrategyDiagnostics,
            PositionLastEvalSnapshot,
            PositionStreamPerformanceResponse,
            PositionStreamPnLResponse,
            LineageChainCostSummary,
            PositionStreamLineageNode,
            PositionStreamLineageResponse,
            OpenPositionRequest,
            SwapBeforeOpenRequest,
            SwapInPoolBeforeOpen,
            DecreaseLiquidityRequest,
            LinkPositionStrategyRequest,
            RebalanceRequest,
            MessageResponse,
            PositionOpenResponse,
            SwapBeforeOpenResponse,
            // Strategies
            ListStrategiesResponse,
            StrategyResponse,
            StrategyPerformanceResponse,
            CreateStrategyRequest,
            StrategyPositionExecutorRequest,
            // Pools
            ListPoolsResponse,
            PoolResponse,
            PoolStateResponse,
            SwapCostEstimateResponse,
            QuoteOpenBudgetRequest,
            QuoteOpenBudgetResponse,
            MarketDataQuery,
            MarketSnapshotRow,
            MarketSnapshotsResponse,
            MarketSwapRow,
            MarketSwapsResponse,
            AgentDecisionsQuery,
            AgentDecisionWriteRequest,
            AgentDecisionRow,
            AgentDecisionsResponse,
            AgentDecisionWriteResponse,
            // Orca REST proxy
            OrcaLockResponse,
            OrcaTokenResponse,
            OrcaTokenListResponse,
            OrcaProtocolResponse,
            OrcaOwnerPositionsResponse,
            OrcaOwnerPositionEntry,
            // Transactions
            BuildUnsignedTxRequest,
            BuildUnsignedTxResponse,
            SubmitSignedTxRequest,
            SubmitSignedTxResponse,
            // Analytics
            PortfolioAnalyticsResponse,
            SimulationRequest,
            SimulationResponse,
            BacktestFromClosedPositionRequest,
            BacktestFromOpenPositionRequest,
            BacktestJobStatusResponse,
            BacktestJobResponse,
            // Bot activity
            BotActivityJsonlResponse,
            BotRegistryJsonlResponse,
            StrandedRebalanceItem,
            StrandedRebalancesResponse,
            SlackActivitySummaryRequest,
            SlackActivitySummaryResponse,
            // Scripts
            ScriptsListResponse,
            ScriptCatalogItem,
            ScriptRunRecord,
            RunScriptRequest,
            // Wallets
            WalletsListResponse,
            WalletEntry,
            WalletReplicationStatus,
            WalletBalancesResponse,
            WalletTokenBalance,
            CreateWalletRequest,
            CreateWalletResponse,
            SetActiveSignerRequest,
            ActiveSignerResponse,
            WalletTransferRequest,
            WalletTransferResponse,
            ApiSignerWalletResponse,
            ConvertSolDirection,
            ConvertSolRequest,
            ConvertSolResponse,
            // Prices
            JupiterPricesResponse,
            // Base EVM / Slipstream
            SlipstreamSlot0Response,
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
        assert!(json.contains("Bociarz LP API"));
    }

    #[test]
    fn test_openapi_yaml() {
        let yaml = openapi_yaml();
        assert!(!yaml.is_empty());
        assert!(yaml.contains("Bociarz LP API"));
    }
}
