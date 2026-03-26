//! API request and response models.

use clmm_lp_domain::agent_decision::AgentDecision;
use clmm_lp_domain::optimize_result::OptimizeResultFile;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

// ============================================================================
// Position Models
// ============================================================================

/// Request to open a new position.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct OpenPositionRequest {
    /// Pool address.
    pub pool_address: String,
    /// Lower tick of the range.
    pub tick_lower: i32,
    /// Upper tick of the range.
    pub tick_upper: i32,
    /// Amount of token A to deposit.
    pub amount_a: u64,
    /// Amount of token B to deposit.
    pub amount_b: u64,
    /// Slippage tolerance in basis points.
    #[serde(default = "default_slippage")]
    pub slippage_tolerance_bps: u16,
}

fn default_slippage() -> u16 {
    50
}

/// Request to rebalance a position.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RebalanceRequest {
    /// New lower tick.
    pub new_tick_lower: i32,
    /// New upper tick.
    pub new_tick_upper: i32,
    /// Slippage tolerance in basis points.
    #[serde(default = "default_slippage")]
    pub slippage_tolerance_bps: u16,
}

/// Request to decrease liquidity in a position.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DecreaseLiquidityRequest {
    /// Liquidity amount to remove.
    pub liquidity_amount: u128,
}

/// Position response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PositionResponse {
    /// Position address.
    pub address: String,
    /// Pool address.
    pub pool_address: String,
    /// Owner address.
    pub owner: String,
    /// Lower tick.
    pub tick_lower: i32,
    /// Upper tick.
    pub tick_upper: i32,
    /// Liquidity amount.
    pub liquidity: String,
    /// Whether position is in range.
    pub in_range: bool,
    /// Current value in USD.
    #[schema(value_type = String)]
    pub value_usd: Decimal,
    /// PnL details.
    pub pnl: PnLResponse,
    /// Position status.
    pub status: PositionStatus,
    /// Created timestamp.
    #[schema(value_type = Option<String>)]
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// PnL response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PnLResponse {
    /// Unrealized PnL in USD.
    #[schema(value_type = String)]
    pub unrealized_pnl_usd: Decimal,
    /// Unrealized PnL percentage.
    #[schema(value_type = String)]
    pub unrealized_pnl_pct: Decimal,
    /// Fees earned (token A).
    pub fees_earned_a: u64,
    /// Fees earned (token B).
    pub fees_earned_b: u64,
    /// Fees earned in USD.
    #[schema(value_type = String)]
    pub fees_earned_usd: Decimal,
    /// Impermanent loss percentage.
    #[schema(value_type = String)]
    pub il_pct: Decimal,
    /// Net PnL in USD.
    #[schema(value_type = String)]
    pub net_pnl_usd: Decimal,
    /// Net PnL percentage.
    #[schema(value_type = String)]
    pub net_pnl_pct: Decimal,
}

/// Position status.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PositionStatus {
    /// Position is active.
    Active,
    /// Position is out of range.
    OutOfRange,
    /// Position is closed.
    Closed,
    /// Position is pending.
    Pending,
}

/// List positions response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ListPositionsResponse {
    /// List of positions.
    pub positions: Vec<PositionResponse>,
    /// Total count.
    pub total: usize,
}

// ============================================================================
// Auth (Phantom) Models
// ============================================================================

/// Request a Phantom signMessage challenge.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PhantomChallengeRequest {
    /// Wallet public key (base58).
    pub wallet_pubkey: String,
}

/// Challenge response to be signed by Phantom (`signMessage`).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PhantomChallengeResponse {
    /// Random nonce (base58/uuid).
    pub nonce: String,
    /// Message bytes (UTF-8) to sign.
    pub message: String,
    /// Expiration time (Unix timestamp).
    pub expires_at: u64,
}

/// Verify Phantom signature and create a short-lived session token.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PhantomVerifyRequest {
    /// Wallet public key (base58).
    pub wallet_pubkey: String,
    /// Nonce previously issued by challenge.
    pub nonce: String,
    /// Signature over the challenge message (base58).
    pub signature: String,
}

/// JWT session response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PhantomSessionResponse {
    /// Bearer token.
    pub token: String,
    /// Seconds until expiry.
    pub expires_in_secs: u64,
}

/// Build unsigned tx request for position operations.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BuildUnsignedTxRequest {
    /// Wallet public key that will sign and pay fees.
    pub wallet_pubkey: String,
    /// Position address if operation requires one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position_address: Option<String>,
    /// Pool address if operation requires one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pool_address: Option<String>,
    /// Optional amount A.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount_a: Option<u64>,
    /// Optional amount B.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount_b: Option<u64>,
    /// Optional liquidity amount.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub liquidity_amount: Option<u128>,
    /// Optional slippage tolerance in bps.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slippage_bps: Option<u16>,

    /// Optional Whirlpool tick lower bound (required for `open` build).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tick_lower: Option<i32>,

    /// Optional Whirlpool tick upper bound (required for `open` build).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tick_upper: Option<i32>,
}

/// Unsigned tx response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BuildUnsignedTxResponse {
    /// Base64 serialized transaction.
    pub unsigned_tx_base64: String,
    /// Correlation identifier for audit.
    pub correlation_id: String,
    /// Programs expected in message.
    pub expected_program_ids: Vec<String>,
}

/// Submit signed tx request.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SubmitSignedTxRequest {
    /// Base64 serialized signed transaction.
    pub signed_tx_base64: String,
}

/// Submit signed tx response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SubmitSignedTxResponse {
    /// Signature returned by RPC.
    pub signature: String,
}

// ============================================================================
// Strategy Models
// ============================================================================

/// Who may apply `OptimizeResultFile` updates from grid search (periodic subprocess vs HTTP vs both).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum OptimizeApplyPolicy {
    /// Only the in-process periodic `backtest-optimize` subprocess may apply; `POST /apply-optimize-result` returns 409.
    PeriodicSubprocess,
    /// Only `POST /apply-optimize-result` applies; set `optimize_interval_secs` to 0 when using [`crate::services::StrategyService`].
    ExternalHttp,
    /// Subprocess and HTTP may both apply; shared per-strategy lock serializes with the subprocess busy flag.
    #[default]
    Combined,
}

/// Request to create a strategy.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateStrategyRequest {
    /// Strategy name.
    pub name: String,
    /// Pool address.
    pub pool_address: String,
    /// Strategy type.
    pub strategy_type: StrategyType,
    /// Strategy parameters.
    pub parameters: StrategyParameters,
    /// Whether to auto-execute.
    #[serde(default)]
    pub auto_execute: bool,
    /// Whether to run in dry-run mode.
    #[serde(default)]
    pub dry_run: bool,
}

/// Strategy type.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum StrategyType {
    /// Static range strategy.
    StaticRange,
    /// Periodic rebalancing.
    Periodic,
    /// Threshold-based rebalancing.
    Threshold,
    /// Rebalance only when out of range (backtest `OorRecenter`).
    OorRecenter,
    /// IL limit strategy.
    IlLimit,
    /// Shift only the exiting edge of the range towards current price.
    RetouchShift,
}

/// Strategy parameters.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct StrategyParameters {
    /// Tick range width.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tick_width: Option<i32>,
    /// Range width percentage (e.g. 4.0 for 4%).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub range_width_pct: Option<Decimal>,
    /// Rebalance threshold percentage.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub rebalance_threshold_pct: Option<Decimal>,
    /// Maximum IL percentage.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub max_il_pct: Option<Decimal>,
    /// Evaluation interval in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eval_interval_secs: Option<u64>,
    /// Minimum rebalance interval in hours.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_rebalance_interval_hours: Option<u64>,

    /// Run `clmm-lp-cli backtest-optimize` once when the strategy starts (before the executor loop).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub optimize_on_start: Option<bool>,
    /// Period in seconds between background optimize runs (0 = disabled).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub optimize_interval_secs: Option<u64>,
    /// argv for the CLI: `[program, subcommand, ...]` e.g. `["clmm-lp-cli","backtest-optimize",...]`.
    /// If `--optimize-result-json` is omitted, the API appends it using `optimize_result_json_path`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub optimize_command: Option<Vec<String>>,
    /// Path passed to `--optimize-result-json` (written by CLI, read by API).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub optimize_result_json_path: Option<String>,
    /// Append IL / rebalance ledger lines (JSONL) to this file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub il_ledger_path: Option<String>,
    /// When using `POST .../apply-optimize-result` with an agent envelope, cap `|Δ winner.width_pct|` vs `baseline_optimize_result` (same units as backtest: fraction, e.g. `0.02` = 2 percentage points).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_max_width_pct_delta: Option<f64>,
    /// Whether periodic subprocess, external HTTP apply, or both may update the executor from grid results (see `OptimizeApplyPolicy`).
    #[serde(default)]
    pub optimize_apply_policy: OptimizeApplyPolicy,
}

/// Agent envelope for [`ApplyOptimizeResultRequest::Agent`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentApplyEnvelope {
    /// Approval + optional full `OptimizeResultFile` to apply.
    pub decision: AgentDecision,
    /// Baseline grid result for optional `agent_max_width_pct_delta` checks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_optimize_result: Option<OptimizeResultFile>,
}

/// Body for `POST /strategies/{id}/apply-optimize-result`: raw optimize JSON or agent envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ApplyOptimizeResultRequest {
    /// Structured agent decision (try this variant first in JSON; see `AgentDecision`).
    Agent(AgentApplyEnvelope),
    /// Direct `OptimizeResultFile` from `backtest-optimize --optimize-result-json`.
    Direct(OptimizeResultFile),
}

/// Strategy response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StrategyResponse {
    /// Strategy ID.
    pub id: String,
    /// Strategy name.
    pub name: String,
    /// Pool address.
    pub pool_address: String,
    /// Strategy type.
    pub strategy_type: StrategyType,
    /// Strategy parameters.
    pub parameters: StrategyParameters,
    /// Whether strategy is running.
    pub running: bool,
    /// Whether in dry-run mode.
    pub dry_run: bool,
    /// Created timestamp.
    #[schema(value_type = String)]
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Updated timestamp.
    #[schema(value_type = String)]
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// List strategies response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ListStrategiesResponse {
    /// List of strategies.
    pub strategies: Vec<StrategyResponse>,
    /// Total count.
    pub total: usize,
}

/// Strategy performance response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StrategyPerformanceResponse {
    /// Strategy ID.
    pub strategy_id: String,
    /// Total PnL in USD.
    #[schema(value_type = String)]
    pub total_pnl_usd: Decimal,
    /// Total PnL percentage.
    #[schema(value_type = String)]
    pub total_pnl_pct: Decimal,
    /// Total fees earned in USD.
    #[schema(value_type = String)]
    pub total_fees_usd: Decimal,
    /// Total IL percentage.
    #[schema(value_type = String)]
    pub total_il_pct: Decimal,
    /// Number of rebalances.
    pub rebalance_count: u32,
    /// Total transaction costs in lamports.
    pub total_tx_costs_lamports: u64,
    /// Win rate percentage.
    #[schema(value_type = String)]
    pub win_rate_pct: Decimal,
}

// ============================================================================
// Pool Models
// ============================================================================

/// Pool response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PoolResponse {
    /// Pool address.
    pub address: String,
    /// Protocol name.
    pub protocol: String,
    /// Token A mint.
    pub token_mint_a: String,
    /// Token B mint.
    pub token_mint_b: String,
    /// Current tick.
    pub current_tick: i32,
    /// Tick spacing.
    pub tick_spacing: i32,
    /// Current price.
    #[schema(value_type = String)]
    pub price: Decimal,
    /// Total liquidity.
    pub liquidity: String,
    /// Fee rate in basis points.
    pub fee_rate_bps: u16,
    /// 24h volume in USD.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub volume_24h_usd: Option<Decimal>,
    /// TVL in USD.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub tvl_usd: Option<Decimal>,
    /// APY estimate.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub apy_estimate: Option<Decimal>,
}

/// List pools response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ListPoolsResponse {
    /// List of pools.
    pub pools: Vec<PoolResponse>,
    /// Total count.
    pub total: usize,
}

/// Orca lock info (proxy of Orca Public REST `/lock/{address}`).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct OrcaLockInfoResponse {
    pub name: String,
    pub locked_percentage: String,
}

/// Orca lock info response wrapper.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct OrcaLockResponse {
    pub address: String,
    pub locks: Vec<OrcaLockInfoResponse>,
}

/// Orca token response (proxy of Orca Public REST `/tokens*`).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct OrcaTokenResponse {
    pub mint: String,
    pub symbol: Option<String>,
    pub name: Option<String>,
    pub decimals: Option<u8>,
    pub verified: Option<bool>,
    #[schema(value_type = Option<String>)]
    pub price_usdc: Option<Decimal>,
}

/// Orca token list response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct OrcaTokenListResponse {
    pub tokens: Vec<OrcaTokenResponse>,
    pub total: usize,
}

/// Orca protocol stats response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct OrcaProtocolResponse {
    #[schema(value_type = Option<String>)]
    pub tvl_usdc: Option<Decimal>,
    #[schema(value_type = Option<String>)]
    pub volume_24h_usdc: Option<Decimal>,
    #[schema(value_type = Option<String>)]
    pub volume_7d_usdc: Option<Decimal>,
}

/// Pool state response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PoolStateResponse {
    /// Pool address.
    pub address: String,
    /// Current tick.
    pub current_tick: i32,
    /// Sqrt price X64.
    pub sqrt_price: String,
    /// Current price.
    #[schema(value_type = String)]
    pub price: Decimal,
    /// Total liquidity.
    pub liquidity: String,
    /// Fee growth global A.
    pub fee_growth_global_a: String,
    /// Fee growth global B.
    pub fee_growth_global_b: String,
    /// Timestamp.
    #[schema(value_type = String)]
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

// ============================================================================
// Analytics Models
// ============================================================================

/// Portfolio analytics response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PortfolioAnalyticsResponse {
    /// Total value in USD.
    #[schema(value_type = String)]
    pub total_value_usd: Decimal,
    /// Total PnL in USD.
    #[schema(value_type = String)]
    pub total_pnl_usd: Decimal,
    /// Total PnL percentage.
    #[schema(value_type = String)]
    pub total_pnl_pct: Decimal,
    /// Total fees earned in USD.
    #[schema(value_type = String)]
    pub total_fees_usd: Decimal,
    /// Total IL percentage.
    #[schema(value_type = String)]
    pub total_il_pct: Decimal,
    /// Number of active positions.
    pub active_positions: u32,
    /// Number of positions in range.
    pub positions_in_range: u32,
    /// Best performing position.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub best_position: Option<String>,
    /// Worst performing position.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worst_position: Option<String>,
}

/// Simulation request.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SimulationRequest {
    /// Pool address.
    pub pool_address: String,
    /// Lower tick.
    pub tick_lower: i32,
    /// Upper tick.
    pub tick_upper: i32,
    /// Initial capital in USD.
    #[schema(value_type = String)]
    pub initial_capital_usd: Decimal,
    /// Start date.
    #[schema(value_type = String)]
    pub start_date: chrono::NaiveDate,
    /// End date.
    #[schema(value_type = String)]
    pub end_date: chrono::NaiveDate,
    /// Strategy type.
    #[serde(default)]
    pub strategy_type: Option<StrategyType>,
}

/// Simulation response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SimulationResponse {
    /// Simulation ID.
    pub id: String,
    /// Pool address.
    pub pool_address: String,
    /// Tick range.
    pub tick_lower: i32,
    /// Tick range.
    pub tick_upper: i32,
    /// Initial capital.
    #[schema(value_type = String)]
    pub initial_capital_usd: Decimal,
    /// Final value.
    #[schema(value_type = String)]
    pub final_value_usd: Decimal,
    /// Total return percentage.
    #[schema(value_type = String)]
    pub total_return_pct: Decimal,
    /// Fee earnings percentage.
    #[schema(value_type = String)]
    pub fee_earnings_pct: Decimal,
    /// IL percentage.
    #[schema(value_type = String)]
    pub il_pct: Decimal,
    /// Sharpe ratio.
    #[schema(value_type = String)]
    pub sharpe_ratio: Decimal,
    /// Max drawdown percentage.
    #[schema(value_type = String)]
    pub max_drawdown_pct: Decimal,
    /// Number of rebalances.
    pub rebalance_count: u32,
}

// ============================================================================
// Health Models
// ============================================================================

/// Health check response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct HealthResponse {
    /// Service status.
    pub status: ServiceStatus,
    /// Version.
    pub version: String,
    /// Uptime in seconds.
    pub uptime_secs: u64,
    /// Component health.
    pub components: ComponentHealth,
}

/// Service status.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ServiceStatus {
    /// Service is healthy.
    Healthy,
    /// Service is degraded.
    Degraded,
    /// Service is unhealthy.
    Unhealthy,
}

/// Component health status.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ComponentHealth {
    /// RPC connection status.
    pub rpc: bool,
    /// Database status.
    pub database: bool,
    /// Circuit breaker status.
    pub circuit_breaker: CircuitBreakerStatus,
}

/// Circuit breaker status.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CircuitBreakerStatus {
    /// Circuit is closed (normal).
    Closed,
    /// Circuit is open (blocking).
    Open,
    /// Circuit is half-open (testing).
    HalfOpen,
}

/// Metrics response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MetricsResponse {
    /// Request count.
    pub request_count: u64,
    /// Error count.
    pub error_count: u64,
    /// Average response time in milliseconds.
    pub avg_response_time_ms: f64,
    /// Active WebSocket connections.
    pub active_ws_connections: u32,
    /// Positions monitored.
    pub positions_monitored: u32,
    /// Strategies running.
    pub strategies_running: u32,
    /// Event bus metrics.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_bus: Option<EventBusMetricsResponse>,
}

/// Event bus operational metrics.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EventBusMetricsResponse {
    pub published: u64,
    pub retries: u64,
    pub duplicates: u64,
    pub failed: u64,
    pub dlq_size: usize,
}

// ============================================================================
// Common Models
// ============================================================================

/// Success response wrapper.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SuccessResponse<T> {
    /// Success flag.
    pub success: bool,
    /// Response data.
    pub data: T,
}

impl<T> SuccessResponse<T> {
    /// Creates a new success response.
    pub fn new(data: T) -> Self {
        Self {
            success: true,
            data,
        }
    }
}

/// Message response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MessageResponse {
    /// Message.
    pub message: String,
}

impl MessageResponse {
    /// Creates a new message response.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::AgentApplyEnvelope;

    #[test]
    fn agent_apply_envelope_rejects_unknown_fields() {
        let j = r#"{"decision":{"schema_version":1,"approved":false},"unknown_extra":1}"#;
        assert!(serde_json::from_str::<AgentApplyEnvelope>(j).is_err());
    }
}
