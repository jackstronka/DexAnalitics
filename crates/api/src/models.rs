//! API request and response models.

use clmm_lp_domain::agent_decision::AgentDecision;
use clmm_lp_domain::optimize_result::OptimizeResultFile;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

// ============================================================================
// Position Models
// ============================================================================

/// Optional **Orca Whirlpool swap in the same pool** before opening liquidity (ExactIn).
///
/// Use the pool's token A or B mint as `specified_mint` (same semantics as `orca swap` / `swap_instructions`).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SwapInPoolBeforeOpen {
    /// SPL mint for ExactIn direction (must equal pool `token_mint_a` or `token_mint_b`).
    pub specified_mint: String,
    /// Raw amount (smallest units) of `specified_mint` to swap before the open-position tx.
    pub amount_in: u64,
}

/// Request to execute **only** an Orca Whirlpool swap (ExactIn) inside a pool.
///
/// Intended for a 2-step UI flow:
/// 1) SWAP to cover token mix
/// 2) later OPEN a position (optionally with the same `cost_session_id` for bookkeeping).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SwapBeforeOpenRequest {
    /// Pool address.
    pub pool_address: String,
    /// SPL mint for ExactIn direction (must equal pool `token_mint_a` or `token_mint_b`).
    pub specified_mint: String,
    /// Raw amount (smallest units) of `specified_mint` to swap.
    pub amount_in: u64,
    /// Swap slippage tolerance in basis points.
    #[serde(default = "default_slippage")]
    pub slippage_tolerance_bps: u16,
    /// Optional bookkeeping id; same value groups swap + open rows in `orca_position_lifecycle.jsonl`.
    #[serde(default)]
    pub cost_session_id: Option<String>,
}

/// Response for `POST /positions/swap-before-open`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SwapBeforeOpenResponse {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub swap_signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_session_id: Option<String>,
}

/// Request to open a new position.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct OpenPositionRequest {
    /// Pool address.
    pub pool_address: String,
    /// Lower tick of the range (ignored when `full_range` is true).
    pub tick_lower: i32,
    /// Upper tick of the range (ignored when `full_range` is true).
    pub tick_upper: i32,
    /// Amount of token A to deposit.
    pub amount_a: u64,
    /// Amount of token B to deposit.
    pub amount_b: u64,
    /// Slippage tolerance in basis points.
    #[serde(default = "default_slippage")]
    pub slippage_tolerance_bps: u16,
    /// Open a **full-range** (Splash-style) position; on-chain tick bounds come from pool spacing.
    #[serde(default)]
    pub full_range: bool,
    /// If set, after a successful open the new position PDA is appended to this strategy's
    /// `parameters.position_addresses`.
    #[serde(default)]
    pub strategy_id: Option<String>,
    /// When set, API executes an Orca **swap in this pool** first (server wallet), then opens the position.
    /// Useful to rebalance token mix; same primitive will be reused for automated rebalance flows.
    #[serde(default)]
    pub swap_before_open: Option<SwapInPoolBeforeOpen>,
    /// Optional id for **bookkeeping**: same value groups swap + open rows in `orca_position_lifecycle.jsonl`
    /// (`rebalance_session_id`) so costs can be summed **per opened position** after the fact.
    #[serde(default)]
    pub cost_session_id: Option<String>,
}

fn default_slippage() -> u16 {
    50
}

/// Link or move a position to a strategy (`parameters.position_addresses`), or unlink from all.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LinkPositionStrategyRequest {
    /// Target strategy id. When `null` or omitted, removes this position PDA from every strategy.
    #[serde(default)]
    pub strategy_id: Option<String>,
}

/// How the API should choose the new tick range for `POST /positions/{address}/rebalance`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RebalanceInput {
    /// Explicit tick bounds (legacy / advanced).
    #[default]
    Ticks,
    /// Center on current pool tick; width from linked strategy and/or `range_width_pct`.
    StrategyRange,
    /// Center on `center_price` (token B per token A, same convention as pool price); width from `range_width_pct`.
    PriceBand,
}

/// Request to rebalance a position.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RebalanceRequest {
    /// How to derive `new_tick_lower` / `new_tick_upper`. Defaults to `ticks` for backward compatibility.
    #[serde(default)]
    pub input: RebalanceInput,
    /// New lower tick when `input` is `ticks`.
    #[serde(default)]
    pub new_tick_lower: i32,
    /// New upper tick when `input` is `ticks`.
    #[serde(default)]
    pub new_tick_upper: i32,
    /// When `input` is `strategy_range`: load `parameters.range_width_pct` from this strategy (optional if `range_width_pct` is set).
    #[serde(default)]
    pub strategy_id: Option<String>,
    /// When `input` is `strategy_range` or `price_band`: range width in percent (e.g. `1.0` = 1%). Overrides strategy when both are set.
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    pub range_width_pct: Option<Decimal>,
    /// When `input` is `price_band`: center price (B per A) for the new range.
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    pub center_price: Option<Decimal>,
    /// Slippage tolerance in basis points.
    #[serde(default = "default_slippage")]
    pub slippage_tolerance_bps: u16,
}

/// Request to decrease liquidity in a position.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DecreaseLiquidityRequest {
    /// Liquidity amount to remove (base units as decimal string; supports full u128 range in JSON).
    pub liquidity_amount: String,
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
    /// When the pool is USDC vs one other token: lower bound of the range in **USDC per 1 unit of that token** (same convention as DEX UI).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub range_lower_usdc: Option<Decimal>,
    /// Upper bound (see `range_lower_usdc`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub range_upper_usdc: Option<Decimal>,
    /// e.g. `per 1 SOL` — only set when `range_*_usdc` are present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range_usdc_quote: Option<String>,
    /// Generic lower bound in UI price units (token B per 1 token A).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub range_lower_price: Option<Decimal>,
    /// Generic upper bound (see `range_lower_price`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub range_upper_price: Option<Decimal>,
    /// e.g. `whETH per 1 SOL` — only set when `range_*_price` are present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range_price_quote: Option<String>,
    /// Pool token A label (e.g. SOL) when valuation succeeded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_a_label: Option<String>,
    /// Pool token B label (e.g. USDC) when valuation succeeded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_b_label: Option<String>,
    /// Pool token A mint (base58).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_mint_a: Option<String>,
    /// Pool token B mint (base58).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_mint_b: Option<String>,
    /// Best-effort USD price for one UI unit of token A (free feeds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_price_a_usd: Option<f64>,
    /// Best-effort USD price for one UI unit of token B.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_price_b_usd: Option<f64>,
    /// Per-token uncollected fees (on-chain), when pool + mint decimals could be resolved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uncollected_fees: Option<UncollectedFeesInfo>,
    /// Liquidity amount.
    pub liquidity: String,
    /// Whether position is in range.
    pub in_range: bool,
    /// Current value in USD.
    #[schema(value_type = String)]
    pub value_usd: Decimal,
    /// Source quality for `value_usd`:
    /// - `live_valuation`: fresh on-chain valuation path succeeded
    /// - `fallback_monitor`: fallback to monitor cache (valuation failed)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valuation_source: Option<String>,
    /// PnL details.
    pub pnl: PnLResponse,
    /// Position status.
    pub status: PositionStatus,
    /// Created timestamp.
    #[schema(value_type = Option<String>)]
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Diagnostics for "why didn't this position rebalance?" (best-effort, read-only).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PositionDiagnosticsResponse {
    /// Position PDA (base58).
    pub address: String,
    /// Whether the position exists in the in-memory monitor.
    pub in_monitor: bool,
    /// Latest `in_range` from monitor (may lag if monitor is stale).
    pub monitor_in_range: Option<bool>,
    /// Strategy ids that include this PDA in `parameters.position_addresses`.
    pub linked_strategies: Vec<PositionStrategyDiagnostics>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PositionStrategyDiagnostics {
    pub strategy_id: String,
    pub name: String,
    pub strategy_type: StrategyType,
    pub running: bool,
    pub dry_run: bool,
    pub auto_execute: bool,
    /// True when this position is in `executor_disabled_position_addresses`.
    pub automation_disabled_for_position: bool,
    /// Last evaluation snapshot from the running executor (if available).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_eval: Option<PositionLastEvalSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PositionLastEvalSnapshot {
    pub ts_utc: String,
    pub in_range: bool,
    pub pool_tick_current: i32,
    pub decision: String,
    pub requires_transaction: bool,
    pub auto_execute: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hours_since_rebalance: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minutes_since_rebalance: Option<u64>,
}

/// Uncollected LP fees from the Whirlpool position account (`fee_owed_a` / `fee_owed_b`), in human
/// token units — same semantics as the Orca app “uncollected fees” before a collect transaction.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UncollectedFeesInfo {
    /// Short label for pool token A (e.g. SOL, USDC, or truncated mint).
    pub token_a_label: String,
    pub token_b_label: String,
    #[schema(value_type = String)]
    pub amount_a: Decimal,
    #[schema(value_type = String)]
    pub amount_b: Decimal,
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

/// Stream-level aggregates for a position PDA (across close->open rotations).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PositionStreamPerformanceResponse {
    /// Position PDA used as the entry point for graph traversal.
    pub position_address: String,
    /// All known PDAs connected to this stream (from IL ledger edges).
    pub positions: Vec<String>,
    /// All known `rebalance_session_id`s connected to this stream.
    pub sessions: Vec<String>,
    /// Total network fee across matching lifecycle rows (lamports).
    pub total_tx_fee_lamports: u64,
    /// Total network fee in USD (best-effort, SOL/USD from free price fetch).
    #[schema(value_type = String)]
    pub total_tx_fee_usd: Decimal,
    /// Number of `bot_collect_fees` events included.
    pub collect_events: u32,
    /// Sum of `fee_payer_token_a_delta_ui` across collect events (token A UI units).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub collected_token_a_ui: Option<Decimal>,
    /// Sum of `fee_payer_token_b_delta_ui` across collect events (token B UI units).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub collected_token_b_ui: Option<Decimal>,
    /// Optional info about limitations / data quality.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Human-readable separation of **economic** chain PnL vs **IL benchmark** (Polish copy for dashboard).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Default)]
pub struct StreamPnLInterpretation {
    /// What `net_pnl_*` means (cashflow-inclusive stream result).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub economic_net_pnl_caption_pl: String,
    /// What `il_*` / `hodl_value_usd` mean vs economic PnL.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub il_vs_initial_hodl_caption_pl: String,
}

/// Stream-level Net PnL / IL across rotated position PDAs (best-effort).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PositionStreamPnLResponse {
    /// Position PDA used as the entry point.
    pub position_address: String,
    /// Baseline snapshot timestamp (earliest known for the stream).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_ts_utc: Option<String>,
    /// Current snapshot timestamp (latest known for the stream).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_ts_utc: Option<String>,
    /// Baseline value in USD (from earliest valuation snapshot).
    #[schema(value_type = String)]
    pub baseline_value_usd: Decimal,
    /// Current value in USD (from latest valuation snapshot).
    #[schema(value_type = String)]
    pub current_value_usd: Decimal,
    /// HODL value in USD for the baseline basket at current prices (used for IL).
    #[schema(value_type = String)]
    pub hodl_value_usd: Decimal,
    /// Clean IL in USD (LP principal - HODL) for the baseline basket.
    ///
    /// Kept for backward compatibility; same value as `clean_il_usd`.
    #[schema(value_type = String)]
    pub il_usd: Decimal,
    /// Clean IL% (LP principal - HODL) / HODL.
    ///
    /// Kept for backward compatibility; same value as `clean_il_pct`.
    #[schema(value_type = String)]
    pub il_pct: Decimal,
    /// Clean IL in USD (LP principal mark - HODL), excluding LP fees.
    #[schema(value_type = String)]
    pub clean_il_usd: Decimal,
    /// Clean IL% vs HODL, excluding LP fees.
    #[schema(value_type = String)]
    pub clean_il_pct: Decimal,
    /// Realized LP fees in USD from collect/close fee legs, excluding principal.
    #[schema(value_type = String)]
    pub realized_lp_fees_usd: Decimal,
    /// Uncollected/claimable LP fees in USD for the active final PDA (0 for closed/end-close streams).
    #[schema(value_type = String)]
    pub uncollected_lp_fees_usd: Decimal,
    /// Total LP fees included in the fee-inclusive LP-vs-HODL benchmark.
    #[schema(value_type = String)]
    pub lp_fees_total_usd: Decimal,
    /// LP-vs-HODL in USD after adding realized + uncollected LP fees.
    #[schema(value_type = String)]
    pub lp_vs_hodl_with_fees_usd: Decimal,
    /// LP-vs-HODL with LP fees, divided by HODL value.
    #[schema(value_type = String)]
    pub lp_vs_hodl_with_fees_pct: Decimal,
    /// Price basis used for HODL/IL valuation (`at_tx_event`, `live_price`, `free_price_fallback`, etc.).
    pub valuation_price_time_kind: String,
    /// Human-readable price/fee component note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_basis_note: Option<String>,
    /// Total network fees in USD (tx_fee_lamports × SOL/USD).
    #[schema(value_type = String)]
    pub tx_fees_usd: Decimal,
    /// Realized cashflow estimate in USD from lifecycle `fee_payer_token_deltas` for pool leg mints.
    #[schema(value_type = String)]
    pub realized_cashflow_usd: Decimal,
    /// Net PnL in USD: current_value + realized_cashflow - baseline_value - tx_fees.
    #[schema(value_type = String)]
    pub net_pnl_usd: Decimal,
    /// Net PnL% vs baseline value.
    #[schema(value_type = String)]
    pub net_pnl_pct: Decimal,
    /// Short Polish captions so UI can show economic PnL and IL benchmark side-by-side without mixing them.
    #[serde(default)]
    pub interpretation: StreamPnLInterpretation,
    /// Notes about data quality / limitations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Request to backfill synthetic valuation snapshots for rotated streams from lifecycle JSONL.
///
/// This is meant as a **best-effort** bridge for older/closed positions where DB snapshots were not
/// collected historically: we convert lifecycle open/close leg deltas into two DB snapshots (open + close)
/// using **current free USD prices**, tagged by `price_source`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Default)]
pub struct BackfillValuationSnapshotsRequest {
    /// Max number of distinct position PDAs to process (stable order by first-seen open time).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit_positions: Option<u32>,
    /// When true, performs all computations but does NOT write to DB.
    #[serde(default)]
    pub dry_run: bool,
}

/// Response for `POST /positions/backfill-valuation-snapshots`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BackfillValuationSnapshotsResponse {
    pub ok: bool,
    /// Distinct PDAs found in lifecycle (after filtering).
    pub positions_considered: u32,
    /// How many PDAs had a usable open row (baseline snapshot candidate).
    pub positions_with_open: u32,
    /// How many PDAs had a usable close row (end snapshot candidate).
    pub positions_with_close: u32,
    /// Total rows inserted into `position_stream_valuation_snapshots`.
    pub rows_inserted: u32,
    /// Source tag used for `price_source`.
    pub price_source: String,
    /// Optional info about skipped rows / limitations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Suggest a strategy link for a position (best-effort).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SuggestStrategyLinkResponse {
    /// Strategy id to link to, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy_id: Option<String>,
    /// Human-readable reason for the suggestion (or why none was found).
    pub reason: String,
}

/// One node (one PDA) in a rotated position stream lineage.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LineageCollectZeroDiagnostics {
    /// Best-effort share of sampled time where tick was inside node range (0..100).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub in_range_time_share_pct_est: Option<Decimal>,
    /// Number of checkpoint samples used for in-range estimation.
    pub in_range_samples: u32,
    /// Best-effort number of swap events in this node's pool and time window.
    pub swap_events_in_window_est: u32,
    /// Best-effort position liquidity share vs max sampled liquidity in same pool window (0..100).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub position_share_pct_est: Option<Decimal>,
    /// Human-readable explanation of what this estimate means.
    pub methodology_note: String,
}

/// One node (one PDA) in a rotated position stream lineage.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PositionStreamLineageNode {
    /// Position PDA (base58).
    pub position_address: String,
    /// Optional token labels/mints for the pool legs (best-effort; enables pair display for closed PDAs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_a_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_b_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_mint_a: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_mint_b: Option<String>,
    /// Earliest valuation snapshot timestamp for this PDA (best-effort open time).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opened_ts_utc: Option<String>,
    /// Latest valuation snapshot timestamp for this PDA (best-effort close/current time).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_ts_utc: Option<String>,
    /// Baseline value for this PDA (earliest known valuation snapshot).
    #[schema(value_type = String)]
    pub baseline_value_usd: Decimal,
    /// Data quality/source tag for baseline valuation (e.g. `exact`, `missing_price`, `fallback`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_valuation_quality: Option<String>,
    /// Latest known value for this PDA (latest valuation snapshot).
    #[schema(value_type = String)]
    pub current_value_usd: Decimal,
    /// Data quality/source tag for current/end valuation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_valuation_quality: Option<String>,
    /// Sum of Solana **network** tx fees (`meta.fee`) in lamports for all lifecycle rows for this PDA.
    pub tx_fee_lamports: u64,
    /// Network tx fees in USD (`tx_fee_lamports` × SOL/USD).
    #[schema(value_type = String)]
    pub tx_fees_usd: Decimal,
    /// LP **fees collected** in USD: positive token deltas from `bot_collect_fees` rows for pool mints × current USD prices.
    #[schema(value_type = String)]
    pub fees_collected_usd: Decimal,
    /// Best-effort collected LP fees for pool token A (UI units) from `bot_collect_fees` rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub fees_collected_token_a_ui: Option<Decimal>,
    /// Best-effort collected LP fees for pool token B (UI units).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub fees_collected_token_b_ui: Option<Decimal>,
    /// Same as `fees_collected_token_*_ui`, but in smallest units (SPL base units; for SOL mint this is lamports).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fees_collected_token_a_raw: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fees_collected_token_b_raw: Option<u64>,
    /// Count of `bot_collect_fees` ledger rows for this PDA.
    pub collect_events: u32,
    /// Realized cashflow estimate in USD from `fee_payer_token_deltas` (pool legs) for this PDA.
    #[schema(value_type = String)]
    pub realized_cashflow_usd: Decimal,
    /// Net PnL estimate for this PDA: current_value + realized_cashflow - baseline_value - tx_fees.
    #[schema(value_type = String)]
    pub net_pnl_usd: Decimal,
    /// Net PnL% vs baseline value.
    #[schema(value_type = String)]
    pub net_pnl_pct: Decimal,
    /// Notes about data quality / limitations for this node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Extra diagnostics for collect rows that show zero LP fees.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collect_zero_diagnostics: Option<LineageCollectZeroDiagnostics>,
}

/// Aggregated **network costs** and **LP fees collected** across the full rotation chain.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LineageChainCostSummary {
    /// Sum of `tx_fee_lamports` over all PDAs in `chain`.
    pub tx_fee_lamports_total: u64,
    /// Sum of `tx_fees_usd` over all nodes.
    #[schema(value_type = String)]
    pub tx_fees_usd_total: Decimal,
    /// Sum of `fees_collected_usd` over all nodes.
    #[schema(value_type = String)]
    pub fees_collected_usd_total: Decimal,
    /// Best-effort collected LP fees (token A UI units) summed across chain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub fees_collected_token_a_ui_total: Option<Decimal>,
    /// Best-effort collected LP fees (token B UI units) summed across chain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub fees_collected_token_b_ui_total: Option<Decimal>,
    /// Same as `*_ui_total` but in smallest units.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fees_collected_token_a_raw_total: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fees_collected_token_b_raw_total: Option<u64>,
    /// Sum of `collect_events` over all nodes.
    pub collect_events_total: u32,
}

/// Ordered lineage for a position stream (root → … → current) plus per-node metrics.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PositionStreamLineageResponse {
    /// Entry point PDA (base58) used for traversal.
    pub position_address: String,
    /// Ordered chain of PDAs (best-effort).
    pub chain: Vec<String>,
    /// Same chain as `chain`, but enriched with per-node aggregates.
    pub nodes: Vec<PositionStreamLineageNode>,
    /// Optional stream totals (same as `GET /positions/{address}/stream-pnl`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub totals: Option<PositionStreamPnLResponse>,
    /// Whole-chain rollup of network tx costs vs LP fees collected (sum of per-node fields).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain_cost_summary: Option<LineageChainCostSummary>,
    /// Notes about lineage reconstruction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
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

/// One closed position entry from the append-only registry (`data/positions/registry.jsonl`).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ClosedPositionEntry {
    /// Position PDA (base58).
    pub position_address: String,
    /// Pool address (base58) as recorded on open/close.
    pub pool_address: String,
    /// Pool leg mint A (base58) when resolvable from pool on-chain (best-effort).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_mint_a: Option<String>,
    /// Pool leg mint B (base58) when resolvable from pool on-chain (best-effort).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_mint_b: Option<String>,
    /// Short label for token A (e.g. SOL, USDC) when recognized (best-effort).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_a_label: Option<String>,
    /// Short label for token B (e.g. SOL, USDC) when recognized (best-effort).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_b_label: Option<String>,
    /// Owner pubkey (base58) as recorded on open/close.
    pub owner: String,
    /// Close classification when known (`manual` vs `strategy` vs `rotation`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub close_kind: Option<String>,
    /// Open timestamp (ISO-8601 from registry row).
    #[schema(value_type = Option<String>)]
    pub opened_ts_utc: Option<String>,
    /// Close timestamp (ISO-8601 from registry row).
    #[schema(value_type = Option<String>)]
    pub closed_ts_utc: Option<String>,
    /// Correlation id when present (`rebalance_session_id` from registry rows).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_rebalance_session_id: Option<String>,
}

/// `GET /positions/closed` response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ClosedPositionsResponse {
    pub total: usize,
    pub items: Vec<ClosedPositionEntry>,
    /// Notes about data quality / limitations (e.g. missing registry file).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// One lifecycle/ledger row (normalized) for UI summaries.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PositionLifecycleEvent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ts_utc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pool_address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position_pubkey: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rebalance_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tx_fee_lamports: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fee_payer_net_lamports_delta: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    pub fee_payer_token_deltas: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PositionLifecycleSessionSummary {
    /// `rebalance_session_id` (or `_no_session` for rows without one).
    pub session_id: String,
    pub events: Vec<PositionLifecycleEvent>,
    pub total_tx_fee_lamports: u64,
    /// Count of events where `event` contains `rebalance` or `close+open` cycle signals (best-effort).
    pub rebalance_related_events: u32,
}

/// `GET /positions/{address}/lifecycle-summary` response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PositionLifecycleSummaryResponse {
    pub position_address: String,
    /// All PDAs connected to this stream when IL edges are available; otherwise only the entry PDA.
    pub positions: Vec<String>,
    /// Sessions connected to the stream (from IL edges + lifecycle rows).
    pub sessions: Vec<String>,
    pub total_tx_fee_lamports: u64,
    #[schema(value_type = String)]
    pub total_tx_fee_usd: Decimal,
    pub collect_events: u32,
    /// Best-effort sum of **collected LP fees** (token A UI units) from `bot_collect_fees` rows.
    ///
    /// Derived from `fee_payer_token_deltas` (positive deltas) for the pool's token A mint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub collected_fee_token_a_ui: Option<Decimal>,
    /// Same as `collected_fee_token_a_ui`, but in smallest units (SPL base units; for SOL mint this is lamports).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collected_fee_token_a_raw: Option<u64>,
    /// Best-effort sum of **collected LP fees** (token B UI units) from `bot_collect_fees` rows.
    ///
    /// Derived from `fee_payer_token_deltas` (positive deltas) for the pool's token B mint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub collected_fee_token_b_ui: Option<Decimal>,
    /// Same as `collected_fee_token_b_ui`, but in smallest units.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collected_fee_token_b_raw: Option<u64>,
    /// Best-effort USD value of collected LP fees (A/B legs) at **current** mint prices.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub collected_fees_usd: Option<Decimal>,
    #[schema(value_type = String)]
    pub realized_cashflow_usd: Decimal,
    pub session_summaries: Vec<PositionLifecycleSessionSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Best-effort canonical “experiment config” for a position (from `registry_open.details` + ledger).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PositionExperimentConfigResponse {
    pub position_address: String,
    /// `rebalance_session_id` recorded on `registry_open` when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_session_id: Option<String>,
    /// Raw `details` JSON stored on `registry_open`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    pub open_details: Option<serde_json::Value>,
    /// `tick_lower` extracted from `open_details` when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tick_lower: Option<i32>,
    /// `tick_upper` extracted from `open_details` when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tick_upper: Option<i32>,
    /// Derived `lower` price (A/B) from `tick_lower` using `tick_to_price`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derived_lower: Option<f64>,
    /// Derived `upper` price (A/B) from `tick_upper` using `tick_to_price`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derived_upper: Option<f64>,
    /// Derived initial capital (USD) from open-session `fee_payer_token_deltas` (best-effort).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derived_initial_capital_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// On-chain Orca Whirlpool positions for a wallet (RPC scan; same source as `orca-positions-list` CLI).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct OrcaOwnerPositionsResponse {
    /// Owner pubkey (base58).
    pub owner: String,
    /// RPC URL used for the scan.
    pub rpc_url: String,
    /// Number of position rows (bundles expand to one row per bundled position).
    pub total: usize,
    pub entries: Vec<OrcaOwnerPositionEntry>,
}

/// One Whirlpool position row from `fetch_positions_for_owner`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct OrcaOwnerPositionEntry {
    /// `position` or `bundled_position`.
    pub kind: String,
    pub position_address: String,
    pub pool_address: String,
    pub tick_lower: i32,
    pub tick_upper: i32,
    /// Same semantics as [`PositionResponse::range_lower_usdc`] when the pool has a USDC leg.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub range_lower_usdc: Option<Decimal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub range_upper_usdc: Option<Decimal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range_usdc_quote: Option<String>,
    /// Generic lower bound in UI price units (token B per 1 token A).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub range_lower_price: Option<Decimal>,
    /// Generic upper bound (see `range_lower_price`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub range_upper_price: Option<Decimal>,
    /// e.g. `whETH per 1 SOL` — only set when `range_*_price` are present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range_price_quote: Option<String>,
    /// Raw liquidity (u128 as decimal string).
    pub liquidity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position_mint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position_bundle_address: Option<String>,
    /// Pool spot tick inside `[tick_lower, tick_upper)` (same rule as monitor `in_range`).
    #[serde(default)]
    pub in_range: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_a_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_b_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_mint_a: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_mint_b: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_price_a_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_price_b_usd: Option<f64>,
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

    /// Optional Whirlpool tick upper bound (required for `open` build unless `full_range` is true).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tick_upper: Option<i32>,

    /// When `Some(true)`, build Orca full-range open (Splash-style); tick fields are ignored.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_range: Option<bool>,
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
    /// Position mint created for open-position flow (if applicable).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position_mint: Option<String>,
    /// Position PDA derived from position mint (if applicable).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position_address: Option<String>,
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

/// Toggle per-position automation for a running strategy (`executor_disabled_position_addresses`).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StrategyPositionExecutorRequest {
    /// Position PDA (same string as in `parameters.position_addresses`).
    pub position_address: String,
    /// When `false`, append to `executor_disabled_position_addresses`; when `true`, remove.
    pub enabled: bool,
}

/// Request to create a strategy.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateStrategyRequest {
    /// Strategy name.
    pub name: String,
    /// Strategy type.
    pub strategy_type: StrategyType,
    /// Strategy parameters.
    pub parameters: StrategyParameters,
    /// Legacy pool on strategy (optional). New flows use pool on **Open Position**; omit or leave empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pool_address: Option<String>,
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
    /// Bollinger Bands strategy (window + k; range from rolling bands).
    Bollinger,
    /// Rebalance only when out of range (backtest `OorRecenter`).
    OorRecenter,
    /// IL limit strategy.
    IlLimit,
    /// Shift only the exiting edge of the range towards current price.
    RetouchShift,
    /// Recenter from the last fully closed candle band (low/high), fallback to width%.
    LastCandle,
    /// Rebalance periodically (time-based) using last closed candle band (or width fallback).
    LastCandlePeriodic,
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
    /// Bollinger: rolling window size in points/samples.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bollinger_window: Option<u64>,
    /// Bollinger: standard deviation multiplier k.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub bollinger_k: Option<Decimal>,
    /// RetouchShift only: shift full retouched band by this percent (e.g. 0.1 => +0.1%).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub retouch_offset_pct: Option<Decimal>,
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
    /// Minimum rebalance interval in minutes (preferred live/UI field).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_rebalance_interval_minutes: Option<u64>,
    /// Candle size for `last_candle` in seconds (e.g. 900 = 15m, 3600 = 1h).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candle_seconds: Option<u64>,

    /// If true, `Periodic` strategy triggers only when the position is out of range.
    /// Default (when omitted) is `true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub periodic_requires_out_of_range: Option<bool>,

    /// If true, range-exit may trigger immediate close+open (rebalance) instead of waiting for the interval.
    /// Default (when omitted) is `false`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rebalance_on_range_exit_immediately: Option<bool>,

    /// Position PDAs linked via Open Position (`strategy_id`) or API; seeded when the strategy starts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position_addresses: Option<Vec<String>>,

    /// Position PDAs for which this strategy's executor skips automation (decisions / txs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executor_disabled_position_addresses: Option<Vec<String>>,

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

    /// If true, API will auto-start this strategy on boot unless `CLMM_STRATEGY_AUTOSTART_ON_BOOT` is set to a false-ish value (unset env ⇒ autostart allowed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_start: Option<bool>,
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
#[allow(clippy::large_enum_variant)]
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
    /// Legacy: pool was previously stored on the strategy. New strategies use per-position pools;
    /// this is `None` unless present in stored config.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pool_address: Option<String>,
    /// Strategy type.
    pub strategy_type: StrategyType,
    /// Strategy parameters.
    pub parameters: StrategyParameters,
    /// Whether strategy is running.
    pub running: bool,
    /// Whether in dry-run mode.
    pub dry_run: bool,
    /// Whether the executor may submit transactions without manual confirmation (requires wallet when not dry-run).
    #[serde(default)]
    pub auto_execute: bool,
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
    /// 1h volume in USD.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub volume_1h_usd: Option<Decimal>,
    /// 5m volume in USD.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub volume_5m_usd: Option<Decimal>,
    /// 7d volume in USD.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub volume_7d_usd: Option<Decimal>,
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

/// Query params for reading persisted Orca volume snapshots.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct OrcaVolumeHistoryQuery {
    /// Optional pool address filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pool_address: Option<String>,
    /// Max rows returned (default: 200, max: 5000).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// One persisted Orca volume snapshot row (JSONL).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct OrcaVolumeSnapshotRow {
    pub ts_utc: String,
    pub source: String,
    pub pool_address: String,
    pub token_mint_a: String,
    pub token_mint_b: String,
    pub fee_rate_bps: u16,
    #[schema(value_type = Option<String>)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tvl_usd: Option<Decimal>,
    #[schema(value_type = Option<String>)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume_5m_usd: Option<Decimal>,
    #[schema(value_type = Option<String>)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume_1h_usd: Option<Decimal>,
    #[schema(value_type = Option<String>)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume_24h_usd: Option<Decimal>,
    #[schema(value_type = Option<String>)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume_7d_usd: Option<Decimal>,
}

/// Response from collecting and persisting Orca volume snapshots.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct OrcaVolumeCollectResponse {
    pub collected_at_utc: String,
    pub path: String,
    pub rows_appended: usize,
    pub stats_windows: Vec<String>,
}

/// Response for reading persisted Orca volume snapshots.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct OrcaVolumeHistoryResponse {
    pub path: String,
    pub rows: Vec<OrcaVolumeSnapshotRow>,
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
// Base EVM — Aerodrome Slipstream (read-only)
// ============================================================================

/// `slot0()` on a Slipstream / Uniswap-v3–style pool contract (`GET .../slot0`).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SlipstreamSlot0Response {
    /// Base mainnet chain id (`8453`).
    pub chain_id: u64,
    /// Pool contract address (`0x` + 40 hex).
    pub pool: String,
    /// `sqrtPriceX96` as a decimal string (fits uint160; string avoids JSON precision loss).
    pub sqrt_price_x96: String,
    /// Current tick.
    pub tick: i32,
    pub observation_index: u16,
    pub observation_cardinality: u16,
    pub observation_cardinality_next: u16,
    pub fee_protocol: u8,
    pub unlocked: bool,
}

/// Rough **network fee** estimate for an Orca swap in a pool (`meta.fee` band), from local ledger history + default.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SwapCostEstimateResponse {
    pub pool_address: String,
    /// Median `tx_fee_lamports` from prior `swap_exact_in` rows for this pool (if any).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub historical_median_network_fee_lamports: Option<u64>,
    /// Rows used for the median (same pool).
    pub historical_sample_count: u32,
    /// Fallback when there is no history (typical base + small priority band).
    pub default_network_fee_lamports: u64,
    /// Value to display: `max(default, median)` when median is present, else default.
    pub estimated_network_fee_lamports: u64,
    /// Explains that full wallet delta is logged after confirmation in `orca_position_lifecycle.jsonl`.
    pub note: String,
}

/// Body: size an in-range open so **on-chain notional** is close to `target_usd` (Whirlpool caps).
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct QuoteOpenBudgetRequest {
    pub tick_lower: i32,
    pub tick_upper: i32,
    /// Desired position value in USD (both legs, at server price snapshot).
    pub target_usd: f64,
}

/// Suggested `amount_a` / `amount_b` for `POST /positions` (raw + UI).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct QuoteOpenBudgetResponse {
    pub token_max_a: u64,
    pub token_max_b: u64,
    pub amount_a: u64,
    pub amount_b: u64,
    pub amount_a_ui: f64,
    pub amount_b_ui: f64,
    pub estimated_value_usd: f64,
    pub liquidity: String,
    /// True when pool spot lies inside `[tick_lower, tick_upper)`.
    pub in_range: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

// ============================================================================
// Analytics Models
// ============================================================================

/// Cumulative fee collection credits from lifecycle JSONL (`bot_collect_fees` rows).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct FeesCollectedFromLedger {
    /// True when the ledger file is missing or unreadable on the API host.
    pub file_missing: bool,
    /// Number of `bot_collect_fees` events in the file.
    pub collect_events: u32,
    /// Sum of `fee_payer_token_a_delta_ui` when present on rows.
    #[schema(value_type = Option<String>)]
    pub sum_token_a_ui: Option<Decimal>,
    /// Sum of `fee_payer_token_b_delta_ui` when present on rows.
    #[schema(value_type = Option<String>)]
    pub sum_token_b_ui: Option<Decimal>,
}

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
    /// Sums from `orca_position_lifecycle.jsonl` collect events (all positions).
    pub fees_collected_from_ledger: FeesCollectedFromLedger,
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
    /// Strategy type (default: static range — no rebalance).
    #[serde(default = "default_sim_strategy")]
    pub strategy_type: StrategyType,
    /// Annualized GBM volatility for the **synthetic** price path (0.55 ≈ 55%).
    #[serde(default = "default_gbm_vol")]
    pub gbm_volatility: f64,
    /// Annualized drift for GBM (usually 0).
    #[serde(default)]
    pub gbm_drift: f64,
    /// For `Periodic`: rebalance every N simulation steps (default 7 with daily steps ≈ weekly).
    #[serde(default)]
    pub periodic_interval_steps: Option<u64>,
    /// For `Threshold` / `OorRecenter`: midpoint deviation threshold (e.g. 0.05 = 5%). Ignored for OOR-only if using defaults below.
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    pub threshold_pct: Option<Decimal>,
    /// For `IlLimit`: max |IL| before rebalance (e.g. 0.08 = 8%).
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    pub il_limit_pct: Option<Decimal>,
}

fn default_sim_strategy() -> StrategyType {
    StrategyType::StaticRange
}

fn default_gbm_vol() -> f64 {
    0.55
}

impl Default for SimulationRequest {
    fn default() -> Self {
        Self {
            pool_address: String::new(),
            tick_lower: -100,
            tick_upper: 100,
            initial_capital_usd: Decimal::new(1_000, 0),
            start_date: chrono::NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid date"),
            end_date: chrono::NaiveDate::from_ymd_opt(2024, 1, 10).expect("valid date"),
            strategy_type: StrategyType::StaticRange,
            gbm_volatility: default_gbm_vol(),
            gbm_drift: 0.0,
            periodic_interval_steps: None,
            threshold_pct: None,
            il_limit_pct: None,
        }
    }
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
    /// Fee earnings percentage (fees / initial capital).
    #[schema(value_type = String)]
    pub fee_earnings_pct: Decimal,
    /// IL percentage.
    #[schema(value_type = String)]
    pub il_pct: Decimal,
    /// Sharpe ratio (annualized, heuristic from PnL path).
    #[schema(value_type = String)]
    pub sharpe_ratio: Decimal,
    /// Max drawdown percentage.
    #[schema(value_type = String)]
    pub max_drawdown_pct: Decimal,
    /// Fraction of steps in range (0–1).
    #[schema(value_type = String)]
    pub time_in_range_pct: Decimal,
    /// Final value minus HODL of initial capital at final/entry price ratio.
    #[schema(value_type = String)]
    pub vs_hodl_usd: Decimal,
    /// Number of rebalances.
    pub rebalance_count: u32,
    /// How the price path and strategy were chosen (transparency for operators).
    pub methodology_note: String,
}

/// Request for `POST /backtests/from-closed-position`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BacktestFromClosedPositionRequest {
    pub position_address: String,
    /// Override: lower bound price (A/B). By default derived from `tick_lower` in registry details.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lower: Option<f64>,
    /// Override: upper bound price (A/B). By default derived from `tick_upper` in registry details.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upper: Option<f64>,
    /// Override: initial capital USD. By default derived from open-session token deltas.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capital: Option<f64>,
    /// Strategy string accepted by CLI (`static|periodic|threshold` etc.). Defaults to `static`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy: Option<String>,
    /// Optional start date (UTC) YYYY-MM-DD. If omitted, inferred from registry_open timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_date: Option<String>,
    /// Optional end date (UTC) `YYYY-MM-DD` — forwarded to CLI as the exclusive upper bound (`ts < end`).
    /// If omitted, the API uses the registry close **calendar day** and passes **the next day** to the CLI so intraday snapshots on the close day are not dropped (same calendar day as `--start-date` would otherwise yield an empty window).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_date: Option<String>,
    /// Fee source accepted by CLI (default `snapshots`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fee_source: Option<String>,
    /// Price path source accepted by CLI (default `snapshots`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_path_source: Option<String>,
    /// Snapshot protocol accepted by CLI (default `orca`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_protocol: Option<String>,
}

/// Request for `POST /backtests/from-open-position`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BacktestFromOpenPositionRequest {
    pub position_address: String,
    /// Override: lower bound price (A/B). By default derived from `tick_lower` in registry open details.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lower: Option<f64>,
    /// Override: upper bound price (A/B). By default derived from `tick_upper` in registry open details.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upper: Option<f64>,
    /// Override: initial capital USD. By default derived from open-session token deltas.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capital: Option<f64>,
    /// Strategy string accepted by CLI (`static|periodic|threshold` etc.). Defaults to `static`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy: Option<String>,
    /// Optional start date (UTC) YYYY-MM-DD. If omitted, inferred from registry_open timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_date: Option<String>,
    /// Optional end date (UTC) `YYYY-MM-DD` (exclusive upper bound for CLI `ts < end`).
    /// If omitted, API uses the next UTC calendar day from "now" so the current day is included.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_date: Option<String>,
    /// Fee source accepted by CLI (default `snapshots`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fee_source: Option<String>,
    /// Price path source accepted by CLI (default `snapshots`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_path_source: Option<String>,
    /// Snapshot protocol accepted by CLI (default `orca`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_protocol: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BacktestJobStatusResponse {
    pub id: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BacktestJobResponse {
    pub id: String,
    pub position_address: String,
    pub pool_address: String,
    pub status: String,
    pub started_ts_utc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_ts_utc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// One strategy family available in FULL backtest matrix UI.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BacktestStrategyCatalogEntry {
    /// Stable id for filtering (e.g. `static`, `threshold`, `last_candle`).
    pub id: String,
    /// Display label.
    pub label: String,
    /// Human-readable parameter hints for this family.
    pub parameters: Vec<String>,
}

/// `GET /backtests/strategy-catalog` response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BacktestStrategyCatalogResponse {
    pub strategies: Vec<BacktestStrategyCatalogEntry>,
}

/// Request for `POST /backtests/full` (matrix run across pools/windows with full optimize ranking).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BacktestFullRequest {
    /// Hours windows to run (expected: 24, 48, 72, 96).
    pub windows_hours: Vec<u32>,
    /// Optional strategy-family filters for grid generation (`--include-strategy-families`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_strategy_ids: Option<Vec<String>>,
    /// Include indicator families (Bollinger + LastCandle variants) in optimize grid.
    #[serde(default = "default_true")]
    pub include_indicator_strategies: bool,
    /// Objective for optimize ranking (`vs-hodl`, `fees`, `composite`, `pnl`, `risk-adj`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub objective: Option<String>,
    /// Optional curated pool ids subset (default: all curated pools with snapshots).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pool_ids: Option<Vec<String>>,
    /// Optional snapshot variants to compare in one run: `10m`, `5m`.
    /// If omitted, API defaults to `["10m"]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_variants: Option<Vec<String>>,
    /// Optional LP share override (recommended for Meteora snapshot-only mode).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lp_share: Option<f64>,
    /// Capital in USD used in each simulation run (`--capital` for `backtest-optimize`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capital_usd: Option<f64>,
    /// Optional target threshold: keep only rows with `vs_hodl >= target_vs_hodl_usd`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_vs_hodl_usd: Option<f64>,
    /// Optional fixed static deviation from entry in percent (`±X%` around entry).
    /// When set, optimize range grid is pinned to one width:
    /// `width_pct = 2 * static_deviation_pct` (so `10` => range `entry * (1±10%)`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub static_deviation_pct: Option<f64>,
    /// Static only (single selected pool): manual lower range bound in price units.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub static_manual_lower: Option<f64>,
    /// Static only (single selected pool): manual upper range bound in price units.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub static_manual_upper: Option<f64>,
    /// Optional fixed OOR-recenter deviation from entry in percent (`±X%` around entry).
    /// When set, optimize range grid is pinned to one width:
    /// `width_pct = 2 * oor_recenter_deviation_pct` (so `10` => range `entry * (1±10%)`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oor_recenter_deviation_pct: Option<f64>,
    /// Override threshold grid in percent, e.g. `[2,3,5,7,10,15]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold_grid_pct: Option<Vec<f64>>,
    /// Threshold mode: minimum interval (hours) before OOR-triggered rebalance when immediate OOR is disabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold_min_rebalance_interval_hours: Option<u64>,
    /// Threshold mode: if true, OOR triggers immediate rebalance (bot parity default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold_rebalance_on_range_exit_immediately: Option<bool>,
    /// Override periodic strategy grid in hours (legacy field name kept), e.g. `[12,24,48,72]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub periodic_grid_steps: Option<Vec<u64>>,
    /// RetouchShift only: shift full retouched band by this percent (UI value, e.g. `0.1` => +0.1%).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retouch_offset_pct: Option<f64>,
    /// Override bollinger windows, e.g. `[20,30]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bollinger_window_grid: Option<Vec<u64>>,
    /// Override bollinger k values, e.g. `[1.5,2.0,2.5]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bollinger_k_grid: Option<Vec<f64>>,
    /// Override bollinger rebalance steps, e.g. `[24,48]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bollinger_rebalance_steps_grid: Option<Vec<u64>>,
    /// Override bollinger rebalance cadence in hours (preferred), e.g. `[2,4,8]`.
    /// API converts hours to step counts depending on selected snapshot cadence (`10m`/`5m`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bollinger_rebalance_hours_grid: Option<Vec<f64>>,
    /// Override last-candle candle steps (non-snapshot mode), e.g. `[1,2,3,4]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_candle_steps_grid: Option<Vec<u64>>,
    /// Override last-candle rebalance steps (non-snapshot mode), e.g. `[4,16,48]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_candle_rebalance_steps_grid: Option<Vec<u64>>,
    /// Override last-candle candle seconds (snapshot mode), e.g. `[900,1800,3600]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_candle_seconds_grid: Option<Vec<u64>>,
    /// Override last-candle rebalance seconds (snapshot mode), e.g. `[900,1800,3600,14400]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_candle_rebalance_seconds_grid: Option<Vec<u64>>,
}

/// Request for data readiness diagnostics used by Backtests/Data Quality UI.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BacktestDataReadinessRequest {
    /// Optional curated pool ids subset (default: all curated pools with snapshots).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pool_ids: Option<Vec<String>>,
    /// Snapshot variants to inspect (`10m`, `5m`). Defaults to `["10m"]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_variants: Option<Vec<String>>,
    /// Optional analysis window lower bound (inclusive, RFC3339 UTC).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range_start_utc: Option<String>,
    /// Optional analysis window upper bound (inclusive, RFC3339 UTC).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range_end_utc: Option<String>,
}

/// Readiness metrics for one pool + snapshot variant.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BacktestDataReadinessRow {
    pub pool_id: String,
    pub pool_label: String,
    pub protocol: String,
    pub pool_address: String,
    pub snapshot_variant: String,
    pub cadence_minutes: u64,
    pub rows: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oldest_ts_utc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_ts_utc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oldest_continuous_ts_utc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_gap_minutes: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coverage_pct: Option<f64>,
    pub max_backtest_hours_hard: u64,
    pub max_backtest_hours_recommended: u64,
    /// Operational status for near-real-time data quality.
    pub status: String,
    /// Optional machine-readable reason explaining current status.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<String>,
    /// Age of latest snapshot row relative to now (seconds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_age_secs: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Aggregated readiness for selected pools/variants.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BacktestDataReadinessAggregate {
    pub pool_count: u64,
    pub variant_count: u64,
    pub max_backtest_hours_hard: u64,
    pub max_backtest_hours_recommended: u64,
    /// Aggregate operational status (`ok`, `degraded`, `recovering`, `missing`).
    pub status: String,
    pub status_ok_count: u64,
    pub status_degraded_count: u64,
    pub status_recovering_count: u64,
    pub status_missing_count: u64,
    /// Data source used for this response (`db` or `fallback`).
    pub source: String,
    /// Number of selected pool+variant rows present in DB but older than `db_max_age_secs`.
    pub db_stale_rows: u64,
}

/// Active thresholds used by readiness evaluator (resolved from ENV/defaults).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BacktestDataReadinessThresholds {
    pub cache_ttl_secs: i64,
    pub db_max_age_secs: i64,
    pub hard_gap_multiplier: u64,
    pub recommended_coverage_pct: f64,
    pub recommended_gap_multiplier: u64,
    pub recommended_fallback_ratio: f64,
}

/// Response for data readiness diagnostics.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BacktestDataReadinessResponse {
    pub rows: Vec<BacktestDataReadinessRow>,
    pub aggregate: BacktestDataReadinessAggregate,
    pub thresholds: BacktestDataReadinessThresholds,
}

fn default_true() -> bool {
    true
}

/// Request for starting periodic auto-tune loop based on FULL backtests.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BacktestAutoTuneStartRequest {
    /// Interval between full optimize cycles (minutes), e.g. 15 or 30.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval_minutes: Option<u64>,
    /// Template request used for each `POST /backtests/full` cycle.
    pub full_request: BacktestFullRequest,
}

/// Latest winner snapshot extracted from FULL optimize results.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BacktestAutoTuneWinner {
    pub pool_id: String,
    pub pool_label: String,
    pub window_hours: u32,
    pub strategy: String,
    pub width_pct: f64,
    pub score: f64,
    pub pnl: f64,
    pub vs_hodl: f64,
    pub fees: f64,
    pub rebalances: u32,
    pub tir_pct: f64,
}

/// Current auto-tune loop status.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BacktestAutoTuneStatusResponse {
    pub running: bool,
    pub interval_minutes: u64,
    pub started_ts_utc: Option<String>,
    pub last_tick_ts_utc: Option<String>,
    pub next_tick_ts_utc: Option<String>,
    pub latest_job_id: Option<String>,
    pub latest_winner: Option<BacktestAutoTuneWinner>,
    pub note: Option<String>,
}

/// Response for applying latest auto-tune winner to a strategy config.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BacktestAutoTuneApplyResponse {
    pub strategy_id: String,
    pub updated: bool,
    pub note: String,
}

/// One row from optimize ranking table (parsed from CLI output).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BacktestFullMetricRow {
    pub rank: u32,
    pub strategy: String,
    /// Lower bound of the tested LP range (USD).
    pub lower_usd: f64,
    /// Upper bound of the tested LP range (USD).
    pub upper_usd: f64,
    /// Width of the tested range in percent (`(upper-lower)/mid * 100`).
    pub width_pct: f64,
    pub score: f64,
    pub fees: f64,
    pub rebalances: u32,
    pub pnl: f64,
    pub vs_hodl: f64,
    pub tir_pct: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub il_like_pct: Option<f64>,
}

/// One pool+window matrix result.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BacktestFullWindowResult {
    pub pool_id: String,
    pub pool_label: String,
    pub pool_address: String,
    pub protocol: String,
    /// Snapshot variant used for this result (`10m` or `5m`).
    pub snapshot_variant: String,
    pub window_hours: u32,
    pub metrics: Vec<BacktestFullMetricRow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BacktestFullJobStatusResponse {
    pub id: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BacktestFullJobResponse {
    pub id: String,
    pub status: String,
    pub started_ts_utc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_ts_utc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub results: Option<Vec<BacktestFullWindowResult>>,
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
    /// Wallet WS monitor metrics.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wallet_ws: Option<WalletWsMetricsResponse>,
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

/// Wallet WS monitor metrics.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WalletWsMetricsResponse {
    /// Owners with active WS monitor workers.
    pub owners_monitored: u32,
    /// WS events received from account/program/log subscriptions.
    pub events_total: u64,
    /// Number of reconnect loops started after WS worker errors.
    pub reconnects_total: u64,
    /// Number of failed WS-triggered cache refreshes.
    pub refresh_failures_total: u64,
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

/// Response for `POST /positions` (open position), including optional swap / session metadata.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PositionOpenResponse {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position_pda: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub swap_signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_session_id: Option<String>,
}

// ============================================================================
// Bot activity (JSONL ledger / registry → web + Slack)
// ============================================================================

/// Last *matching* JSON lines from `orca_position_lifecycle.jsonl` (CLI + bot tx costs).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BotActivityJsonlResponse {
    /// Resolved filesystem path.
    pub path: String,
    /// True when the file does not exist yet.
    pub file_missing: bool,
    /// Lines successfully parsed (after optional substring filter).
    pub total_matching_lines: usize,
    /// Rows returned (tail slice, max `limit`).
    pub rows_returned: usize,
    /// Parsed JSON objects (newest matching lines last).
    #[schema(value_type = Vec<Object>)]
    pub rows: Vec<serde_json::Value>,
}

/// Open / close registry rows (`registry.jsonl`).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BotRegistryJsonlResponse {
    pub path: String,
    pub file_missing: bool,
    pub total_matching_lines: usize,
    pub rows_returned: usize,
    #[schema(value_type = Vec<Object>)]
    pub rows: Vec<serde_json::Value>,
}

/// Pending-open recovery queue (`CLMM_PENDING_OPEN_RECOVERY_PATH`) used after `rebalance_incomplete`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PendingOpenRecoveryResponse {
    /// Resolved filesystem path.
    pub path: String,
    /// True when the file does not exist yet.
    pub file_missing: bool,
    /// Parsed JSON document when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Object)]
    pub data: Option<serde_json::Value>,
}

/// One rebalance session where close was observed but corresponding open is missing.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StrandedRebalanceItem {
    pub rebalance_session_id: String,
    pub close_seen: bool,
    pub open_seen: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub close_ts_utc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_ts_utc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_position: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_position: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pool_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_mint_a: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_mint_b: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_a_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_b_label: Option<String>,
    pub rebalance_incomplete_logged: bool,
    pub in_pending_open_queue: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intended_tick_lower: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intended_tick_upper: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub can_auto_enqueue: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Watchdog response with detected stranded rebalance sessions and optional enqueue count.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StrandedRebalancesResponse {
    pub lifecycle_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub il_ledger_path: Option<String>,
    pub pending_open_path: String,
    pub rows_scanned: usize,
    pub auto_enqueued: usize,
    pub items: Vec<StrandedRebalanceItem>,
}

/// POST body: how many recent ledger rows to include in the Slack digest.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SlackActivitySummaryRequest {
    /// Max rows from the **tail** of the lifecycle ledger (after parse). Capped at 80.
    #[serde(default = "default_slack_activity_limit")]
    pub limit: usize,
}

fn default_slack_activity_limit() -> usize {
    40
}

impl Default for SlackActivitySummaryRequest {
    fn default() -> Self {
        Self {
            limit: default_slack_activity_limit(),
        }
    }
}

/// Result of posting a digest to Slack Incoming Webhook.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SlackActivitySummaryResponse {
    pub ok: bool,
    /// Error or provider message when `ok` is false.
    pub error: Option<String>,
    /// Rows formatted into the message body.
    pub rows_included: usize,
    /// Whether `SLACK_WEBHOOK_URL` was set.
    pub webhook_configured: bool,
}

// ============================================================================
// Tools scripts (manifest + script_runs.jsonl + runner proxy)
// ============================================================================

/// One row appended to `data/script_runs.jsonl` by the localhost runner.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ScriptRunRecord {
    #[serde(default)]
    pub schema_version: u32,
    pub script_id: String,
    pub ts_utc: String,
    pub ok: bool,
    pub exit_code: i32,
    pub duration_ms: u64,
    #[serde(default)]
    pub stdout_excerpt: Option<String>,
    #[serde(default)]
    pub stderr_excerpt: Option<String>,
    #[serde(default)]
    pub error_excerpt: Option<String>,
    #[serde(default)]
    pub triggered_by: Option<String>,
}

/// Entry from `tools/scripts-manifest.json` (and/or auto-scan `tools/*.ps1`) plus optional last run metadata.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ScriptCatalogItem {
    pub id: String,
    pub path: String,
    pub summary: String,
    #[serde(default)]
    pub when_to_use: Option<String>,
    #[serde(default)]
    pub risk: Option<String>,
    #[serde(default = "default_runnable_true")]
    pub runnable: bool,
    #[serde(default)]
    pub actions: Vec<String>,
    #[serde(default)]
    pub last_run: Option<ScriptRunRecord>,
    /// True when the row came from filesystem scan only (no row in `scripts-manifest.json`).
    #[serde(default)]
    pub auto_discovered: bool,
}

fn default_runnable_true() -> bool {
    true
}

/// `GET /scripts` — manifest + last run per script (from JSONL).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ScriptsListResponse {
    pub repo_root: String,
    pub manifest_path: String,
    pub manifest_missing: bool,
    pub script_runs_path: String,
    pub script_runs_missing: bool,
    /// `SCRIPT_RUNNER_URL` and `SCRIPT_RUNNER_TOKEN` are set (run may still fail if runner is down).
    pub runner_configured: bool,
    pub scripts: Vec<ScriptCatalogItem>,
}

/// `POST /scripts/{id}/run` — forwarded to localhost runner when configured.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Default)]
pub struct RunScriptRequest {
    #[serde(default)]
    pub triggered_by: Option<String>,
}

// ============================================================================
// Position Agent (per-position supervision + chat)
// ============================================================================

/// Start/ensure per-position agent supervision.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Default)]
pub struct AgentSessionRequest {
    /// Background scan interval in hours (default: 4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scan_interval_hours: Option<u64>,
}

/// Agent supervision session tied to one position.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AgentPositionSession {
    pub position_address: String,
    /// `active` or future states (paused/stopped).
    pub status: String,
    pub started_ts_utc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_scan_ts_utc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_scan_ts_utc: Option<String>,
    pub scan_interval_hours: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AgentSessionResponse {
    pub session: AgentPositionSession,
}

/// User message to the position agent chat.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AgentMessageRequest {
    pub content: String,
}

/// Optional context for LLM answer generation.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Default)]
pub struct AgentLlmContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<serde_json::Value>,
}

/// Explicit request to generate one agent answer (LLM plugin path with fallback).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AgentLlmReplyRequest {
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<AgentLlmContext>,
}

/// One chat message in a position agent thread.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AgentChatMessage {
    pub id: String,
    pub position_address: String,
    pub ts_utc: String,
    /// `user` or `agent`.
    pub role: String,
    /// `question` | `info` | `insight` | `action`.
    pub kind: String,
    pub content: String,
}

/// Position chat timeline with session metadata.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AgentChatResponse {
    pub position_address: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<AgentPositionSession>,
    pub messages: Vec<AgentChatMessage>,
}

/// Trigger immediate analysis for this position.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Default)]
pub struct AgentScanRequest {
    /// Include cross-pair opportunity suggestions (default: true).
    #[serde(default = "default_true")]
    pub include_cross_pair_scan: bool,
}

/// Result of immediate agent scan.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AgentScanResponse {
    pub position_address: String,
    pub scanned_ts_utc: String,
    pub include_cross_pair_scan: bool,
    pub recommendations: Vec<String>,
    pub session: AgentPositionSession,
}

/// Point-in-time scenario used by position supervisor.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AgentSupervisorScenario {
    /// `bullish`, `bearish`, `sideways`.
    pub scenario: String,
    /// One-line expectation for this market regime.
    pub expectation: String,
    /// Suggested operator action for this scenario.
    pub suggested_action: String,
}

/// Cost/profit supervision snapshot for one position stream.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AgentPositionSupervisorResponse {
    pub position_address: String,
    /// Earliest known baseline value for the chain (`stream_pnl.baseline_value_usd`).
    #[schema(value_type = String)]
    pub entry_capital_usd: Decimal,
    /// Current live value from `GET /positions/{address}` valuation path when available.
    #[schema(value_type = String)]
    pub current_value_usd: Decimal,
    /// Realized + collected LP earnings proxy across chain.
    #[schema(value_type = String)]
    pub earnings_total_usd: Decimal,
    /// Network tx costs across chain.
    #[schema(value_type = String)]
    pub costs_total_usd: Decimal,
    /// Since-entry net result: `current + earnings - entry - costs`.
    #[schema(value_type = String)]
    pub net_since_entry_usd: Decimal,
    /// Since-entry net result in percent of entry capital.
    #[schema(value_type = String)]
    pub net_since_entry_pct: Decimal,
    /// Number of rebalance/open-close sessions known for this chain.
    pub rebalance_count: u64,
    /// Elapsed wall time from chain baseline timestamp to now (best-effort).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_hours: Option<i64>,
    /// Baseline token leg quantities, if available from lineage head node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub entry_token_a_ui: Option<Decimal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub entry_token_b_ui: Option<Decimal>,
    /// Baseline token leg labels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_token_a_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_token_b_label: Option<String>,
    /// Scenario playbook derived from current stream condition.
    pub scenarios: Vec<AgentSupervisorScenario>,
    /// Data quality / provenance note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Global settings for the position-agent background worker.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AgentWorkerSettings {
    pub enabled: bool,
    /// Default per-position scan interval used when starting session without explicit value.
    pub default_position_scan_interval_hours: u64,
    /// Cross-pair scan cadence hint for recommendations.
    pub cross_pair_scan_interval_hours: u64,
    /// Whether background scans include cross-pair recommendations by default.
    pub include_cross_pair_scan: bool,
}

impl Default for AgentWorkerSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            default_position_scan_interval_hours: 4,
            cross_pair_scan_interval_hours: 4,
            include_cross_pair_scan: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Default)]
pub struct AgentWorkerSettingsUpdateRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_position_scan_interval_hours: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cross_pair_scan_interval_hours: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_cross_pair_scan: Option<bool>,
}

/// Last-known background worker runtime status.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AgentWorkerStatus {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_tick_ts_utc: Option<String>,
    pub ticks_total: u64,
    pub scanned_positions_total: u64,
    pub scanned_positions_last_tick: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

/// UI-focused payload for position-agent tab.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AgentChatUiPayload {
    pub position_address: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<AgentPositionSession>,
    pub messages: Vec<AgentChatMessage>,
    pub quick_actions: Vec<String>,
    pub suggested_prompts: Vec<String>,
}

/// Source metadata for generated answer.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AgentLlmReplyMeta {
    /// `disabled_fallback` or provider name (e.g. `openai_compatible`).
    pub provider: String,
    pub used_fallback: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AgentLlmReplyResponse {
    pub position_address: String,
    pub message: AgentChatMessage,
    pub meta: AgentLlmReplyMeta,
}

// ============================================================================
// Wallets (local keypairs directory + on-chain balances)
// ============================================================================

/// One wallet keypair discovered on disk (API host).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WalletEntry {
    /// Stable id for UI (usually filename stem).
    pub id: String,
    /// Filename under the wallets directory.
    pub filename: String,
    /// Solana pubkey (base58).
    pub pubkey: String,
    /// Wallet file exists in primary storage.
    #[serde(default)]
    pub present_in_primary: bool,
    /// Wallet file exists in secondary storage.
    #[serde(default)]
    pub present_in_secondary: bool,
    /// Replication health across wallet stores.
    #[serde(default)]
    pub replication_status: WalletReplicationStatus,
    /// SHA-256 fingerprint of wallet file bytes (hex, best-effort).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum WalletReplicationStatus {
    #[default]
    Healthy,
    Degraded,
    Conflict,
}

/// `GET /wallets` — list wallets from a directory on the API host.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WalletsListResponse {
    /// Primary wallets directory scanned on API host.
    pub wallets_dir_primary: String,
    /// Secondary wallets directory scanned on API host (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wallets_dir_secondary: Option<String>,
    /// Minimum lamports per SOL transfer (dust guard).
    pub transfer_min_lamports: u64,
    /// Optional maximum lamports per SOL transfer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transfer_max_lamports: Option<u64>,
    pub wallets: Vec<WalletEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateWalletRequest {
    /// Optional wallet id (filename stem). When omitted, generated from timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wallet_id: Option<String>,
    /// Overwrite existing wallet file if present.
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateWalletResponse {
    pub wallet: WalletEntry,
    pub primary_written: bool,
    pub secondary_written: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SetActiveSignerRequest {
    pub wallet_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ActiveSignerResponse {
    pub wallet_id: Option<String>,
    pub pubkey: Option<String>,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WalletTransferRequest {
    /// Wallet id from local wallets storage (`/wallets`).
    pub from_wallet_id: String,
    /// Recipient pubkey.
    pub to_pubkey: String,
    /// Amount in lamports.
    pub lamports: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WalletTransferResponse {
    pub from_wallet_id: String,
    pub from_pubkey: String,
    pub to_pubkey: String,
    pub lamports: u64,
    pub signature: String,
}

/// One transfer log entry (append-only JSONL on API host).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WalletTransferLogEntry {
    /// UTC timestamp (RFC3339).
    pub ts_utc: String,
    pub from_wallet_id: String,
    pub from_pubkey: String,
    pub to_pubkey: String,
    pub lamports: u64,
    pub signature: String,
    /// RPC endpoint used for submission (best-effort).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rpc_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WalletTransfersListResponse {
    pub transfers: Vec<WalletTransferLogEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WalletReconcileItem {
    pub wallet_id: String,
    pub status: WalletReplicationStatus,
    pub repaired: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WalletReconcileResponse {
    pub primary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secondary: Option<String>,
    pub scanned: usize,
    pub repaired: usize,
    pub conflicts: usize,
    pub items: Vec<WalletReconcileItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WalletTokenBalance {
    pub mint: String,
    /// UI amount as string (from jsonParsed RPC).
    pub ui_amount: String,
}

/// `GET /wallets/balances` — on-chain read-only balances for an owner pubkey.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WalletBalancesResponse {
    pub owner: String,
    pub rpc_url: String,
    pub lamports: u64,
    pub sol: String,
    pub tokens: Vec<WalletTokenBalance>,
    /// Number of token-account rows discovered before mint-level merge.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_accounts_total: Option<u64>,
    /// Legacy SPL-Token program read status (`Tokenkeg...`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_legacy_ok: Option<bool>,
    /// Token-2022 program read status (`TokenzQd...`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_2022_ok: Option<bool>,
    /// Best-effort error text for legacy SPL read when failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_legacy_error: Option<String>,
    /// Best-effort error text for Token-2022 read when failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_2022_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum WalletBalanceConfidence {
    Verified,
    Projected,
    Degraded,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WalletEffectiveBalancesResponse {
    pub owner: String,
    pub as_of_utc: String,
    /// True when response comes from stale cache or warmup placeholder.
    pub is_stale: bool,
    /// Cache age in milliseconds for stale responses.
    pub stale_age_ms: u64,
    pub confidence: WalletBalanceConfidence,
    pub pending_ops_count: u64,
    pub native_onchain_lamports: u64,
    pub native_effective_lamports: u64,
    pub wsol_onchain_raw: u64,
    pub wsol_effective_raw: u64,
    /// Compatibility fields for existing Wallet/Swap UI.
    pub rpc_url: String,
    pub lamports: u64,
    pub sol: String,
    pub tokens: Vec<WalletTokenBalance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_accounts_total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_legacy_ok: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_2022_ok: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_legacy_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_2022_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WalletWsStatusResponse {
    pub owners_monitored: u32,
    pub owners: Vec<String>,
    pub events_total: u64,
    pub reconnects_total: u64,
    pub refresh_failures_total: u64,
}

/// `GET /wallets/api-signer` — API signing wallet (from KEYPAIR_PATH / SOLANA_KEYPAIR_PATH) and its SOL balance.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ApiSignerWalletResponse {
    /// Whether a signing wallet is configured on the API host.
    pub configured: bool,
    /// Wallet pubkey (base58) when configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pubkey: Option<String>,
    /// RPC URL used for the balance read (best-effort).
    pub rpc_url: String,
    /// Current SOL balance in lamports (when configured + RPC succeeded).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lamports: Option<u64>,
    /// Current SOL balance as decimal string (when configured + RPC succeeded).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sol: Option<String>,
    /// Minimum lamports for **open** / rent-heavy ops (`CLMM_MIN_OPEN_SOL_LAMPORTS`, default 0.012 SOL).
    pub min_open_lamports: u64,
    /// Minimum lamports for **swap-only** (fee + buffer; lower than open; `CLMM_MIN_SWAP_SOL_LAMPORTS`, default ~0.0015 SOL).
    pub min_swap_lamports: u64,
    /// Optional note/hint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConvertSolDirection {
    NativeToWsol,
    WsolToNative,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ConvertSolRequest {
    pub direction: ConvertSolDirection,
    /// Amount in lamports/raw (9 decimals).
    pub amount_raw: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum WalletReconciliationStatus {
    PendingConfirmation,
    ConfirmedUnreconciled,
    Reconciled,
    Mismatch,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WalletConvertOpResponse {
    pub op_id: String,
    pub owner_pubkey: String,
    pub direction: ConvertSolDirection,
    pub amount_raw: u64,
    pub reconciliation_status: WalletReconciliationStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    pub attempts: u32,
    pub created_at_utc: String,
    pub updated_at_utc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_verified_at_utc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post_native_lamports: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post_wsol_raw: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ConvertSolResponse {
    pub message: String,
    /// Backward-compatible primary signature (equals unwrap/wrap signature when present).
    pub signature: Option<String>,
    /// Signature of wrap tx (SOL -> WSOL) when submitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wrap_signature: Option<String>,
    /// Signature of unwrap tx (WSOL ATA close) when submitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unwrap_signature: Option<String>,
    /// Signature of remainder re-wrap tx for partial unwrap (when submitted).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rewrap_signature: Option<String>,
    /// True when request completed and all required on-chain steps succeeded.
    pub confirmed: bool,
    /// True when WSOL->SOL used close + remainder re-wrap path.
    pub partial: bool,
    pub op_id: String,
    pub reconciliation_status: WalletReconciliationStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    pub attempts: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_verified_at_utc: Option<String>,
    pub direction: ConvertSolDirection,
    pub amount_raw: u64,
    pub owner_pubkey: String,
    /// Post-conversion native SOL balance (lamports) observed by API.
    pub post_native_lamports: u64,
    /// Post-conversion WSOL ATA amount (raw lamports) observed by API.
    pub post_wsol_raw: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WalletOpsStatsResponse {
    pub total: u64,
    pub reconciled: u64,
    pub confirmed_unreconciled: u64,
    pub mismatch: u64,
    pub failed: u64,
    pub pending_confirmation: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mismatch_ratio: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_seconds_to_reconcile: Option<f64>,
}

// ============================================================================
// Prices (free external sources)
// ============================================================================

/// `GET /prices/jupiter` — server-side Jupiter price map (avoids browser CORS/adblock).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct JupiterPricesResponse {
    /// How prices were resolved (e.g. `stable+geckoterminal+jupiter_v2+dexscreener`).
    pub source: String,
    /// Requested ids count (unique, non-empty).
    pub requested: usize,
    /// Returned prices count.
    pub returned: usize,
    /// Map: mint -> USD price.
    pub prices: std::collections::BTreeMap<String, f64>,
}

// ============================================================================
// Data feed endpoints (normalized JSONL reads)
// ============================================================================

/// Common query params for normalized market-data feeds.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MarketDataQuery {
    /// Optional protocol filter (`orca`, `raydium`, `meteora`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    /// Optional pool address filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pool: Option<String>,
    /// Optional lower timestamp bound (RFC3339).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// Optional upper timestamp bound (RFC3339).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// Max rows returned (default: 500, max: 10_000).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// One normalized snapshots row from `data/pool-snapshots/**/snapshots*.jsonl`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MarketSnapshotRow {
    pub ts_utc: String,
    pub protocol: String,
    pub pool_address: String,
    pub source_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub price_ab: Option<Decimal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub liquidity_active_raw: Option<u128>,
    /// Join-friendly optional keys used by other feeds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// Response for `GET /data/snapshots`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MarketSnapshotsResponse {
    pub scanned_files: usize,
    pub rows_returned: usize,
    pub rows: Vec<MarketSnapshotRow>,
}

/// One normalized swaps row from `data/swaps/**/{swaps,decoded_swaps}.jsonl`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MarketSwapRow {
    pub ts_utc: String,
    pub protocol: String,
    pub pool_address: String,
    pub source_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tx_signature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub amount_in: Option<Decimal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub amount_out: Option<Decimal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub fee_usd: Option<Decimal>,
    /// Join-friendly optional keys used by other feeds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// Response for `GET /data/swaps`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MarketSwapsResponse {
    pub scanned_files: usize,
    pub rows_returned: usize,
    pub rows: Vec<MarketSwapRow>,
}

/// Query params for persisted agent decisions.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AgentDecisionsQuery {
    /// Optional strategy id filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy_id: Option<String>,
    /// Optional decision source filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Optional lower timestamp bound (RFC3339).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// Optional upper timestamp bound (RFC3339).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// Max rows returned (default: 500, max: 10000).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// Request body for appending one agent decision row to local JSONL.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AgentDecisionWriteRequest {
    /// Event timestamp (RFC3339 UTC recommended). If missing, API sets current UTC.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ts_utc: Option<String>,
    /// Decision source (e.g. `agent`, `operator`, `autotune`).
    pub source: String,
    /// Optional strategy id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy_id: Option<String>,
    /// Join-friendly optional ids.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Free-form payload with the decision details.
    #[schema(value_type = Object)]
    pub decision: serde_json::Value,
}

/// One persisted row from `data/agent/agent_decisions.jsonl`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AgentDecisionRow {
    pub ts_utc: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[schema(value_type = Object)]
    pub decision: serde_json::Value,
}

/// Response for `GET /data/agent/decisions`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AgentDecisionsResponse {
    pub path: String,
    pub file_missing: bool,
    pub rows_returned: usize,
    pub rows: Vec<AgentDecisionRow>,
}

/// Response for `POST /data/agent/decisions`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AgentDecisionWriteResponse {
    pub path: String,
    pub written: bool,
    pub row: AgentDecisionRow,
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
