// API client for Bociarz LP backend

const API_BASE = '/api/v1'
const API_KEY = (import.meta as any).env?.VITE_API_KEY as string | undefined

export interface Position {
  address: string
  pool_address: string
  owner: string
  tick_lower: number
  tick_upper: number
  /** USDC per 1 unit of the other leg; only when pool is USDC vs one token. */
  range_lower_usdc?: string | number | null
  range_upper_usdc?: string | number | null
  /** e.g. `per 1 SOL` */
  range_usdc_quote?: string | null
  /** Token B per 1 token A in UI units (generic, non-USDC pairs too). */
  range_lower_price?: string | number | null
  range_upper_price?: string | number | null
  /** e.g. `whETH per 1 SOL` */
  range_price_quote?: string | null
  /** Pool leg labels when API valuation succeeded (e.g. SOL, USDC). */
  token_a_label?: string | null
  token_b_label?: string | null
  token_mint_a?: string | null
  token_mint_b?: string | null
  /** USD per 1 UI unit of token A/B (best-effort free feeds). */
  token_price_a_usd?: number | null
  token_price_b_usd?: number | null
  /** Uncollected fees (on-chain fee_owed_*), in UI token units. */
  uncollected_fees?: {
    token_a_label: string
    token_b_label: string
    amount_a: string
    amount_b: string
  } | null
  liquidity: string
  in_range: boolean
  value_usd: string
  valuation_source?: 'live_valuation' | 'fallback_monitor' | string | null
  pnl: PnL
  status: 'active' | 'closed' | 'pending'
  created_at: string | null
}

export interface PositionLastEvalSnapshot {
  ts_utc: string
  in_range: boolean
  pool_tick_current: number
  decision: string
  requires_transaction: boolean
  auto_execute: boolean
  hours_since_rebalance?: number | null
  minutes_since_rebalance?: number | null
}

export interface PositionStrategyDiagnostics {
  strategy_id: string
  name: string
  strategy_type: StrategyType
  running: boolean
  dry_run: boolean
  auto_execute: boolean
  automation_disabled_for_position: boolean
  last_eval?: PositionLastEvalSnapshot | null
}

export interface PositionDiagnosticsResponse {
  address: string
  in_monitor: boolean
  monitor_in_range?: boolean | null
  linked_strategies: PositionStrategyDiagnostics[]
}

export interface AgentPositionSession {
  position_address: string
  status: string
  started_ts_utc: string
  last_scan_ts_utc?: string | null
  next_scan_ts_utc?: string | null
  scan_interval_hours: number
}

export interface AgentChatMessage {
  id: string
  position_address: string
  ts_utc: string
  role: string
  kind: string
  content: string
}

export interface AgentChatUiPayload {
  position_address: string
  session?: AgentPositionSession | null
  messages: AgentChatMessage[]
  quick_actions: string[]
  suggested_prompts: string[]
}

export interface AgentLlmReplyMeta {
  provider: string
  used_fallback: boolean
  model?: string | null
}

export interface AgentLlmReplyResponse {
  position_address: string
  message: AgentChatMessage
  meta: AgentLlmReplyMeta
}

export interface AgentSupervisorScenario {
  scenario: 'bullish' | 'bearish' | 'sideways' | string
  expectation: string
  suggested_action: string
}

export interface AgentPositionSupervisor {
  position_address: string
  entry_capital_usd: string
  current_value_usd: string
  earnings_total_usd: string
  costs_total_usd: string
  net_since_entry_usd: string
  net_since_entry_pct: string
  rebalance_count: number
  elapsed_hours?: number | null
  entry_token_a_ui?: string | null
  entry_token_b_ui?: string | null
  entry_token_a_label?: string | null
  entry_token_b_label?: string | null
  scenarios: AgentSupervisorScenario[]
  note?: string | null
}

export interface PositionStreamPerformanceResponse {
  position_address: string
  positions: string[]
  sessions: string[]
  total_tx_fee_lamports: number
  total_tx_fee_usd: string
  collect_events: number
  collected_token_a_ui?: string | null
  collected_token_b_ui?: string | null
  note?: string | null
}

export interface StreamPnLInterpretation {
  economic_net_pnl_caption_pl?: string
  il_vs_initial_hodl_caption_pl?: string
}

export interface PositionStreamPnLResponse {
  position_address: string
  baseline_ts_utc?: string | null
  current_ts_utc?: string | null
  baseline_value_usd: string
  current_value_usd: string
  hodl_value_usd: string
  il_usd: string
  il_pct: string
  tx_fees_usd: string
  realized_cashflow_usd: string
  net_pnl_usd: string
  net_pnl_pct: string
  interpretation?: StreamPnLInterpretation
  note?: string | null
}

export interface PositionStreamLineageNode {
  position_address: string
  token_a_label?: string | null
  token_b_label?: string | null
  token_mint_a?: string | null
  token_mint_b?: string | null
  opened_ts_utc?: string | null
  closed_ts_utc?: string | null
  baseline_value_usd: string
  baseline_valuation_quality?: string | null
  current_value_usd: string
  current_valuation_quality?: string | null
  /** Sum of Solana network fees (lamports) for all txs logged for this PDA. */
  tx_fee_lamports: number
  tx_fees_usd: string
  /** Fees collected (USD): best-effort fee legs from collect + close rows × mint USD. */
  fees_collected_usd: string
  /** Best-effort collected fee (token A UI units). */
  fees_collected_token_a_ui?: string | null
  /** Best-effort collected fee (token B UI units). */
  fees_collected_token_b_ui?: string | null
  /** Same as `fees_collected_token_*_ui` but in smallest units (base units; for SOL mint this is lamports). */
  fees_collected_token_a_raw?: number | null
  fees_collected_token_b_raw?: number | null
  collect_events: number
  realized_cashflow_usd: string
  net_pnl_usd: string
  net_pnl_pct: string
  note?: string | null
  collect_zero_diagnostics?: {
    in_range_time_share_pct_est?: string | null
    in_range_samples: number
    swap_events_in_window_est: number
    position_share_pct_est?: string | null
    methodology_note: string
  } | null
}

/** Sums of per-node network costs vs collected fees across the full rotation chain. */
export interface LineageChainCostSummary {
  tx_fee_lamports_total: number
  tx_fees_usd_total: string
  fees_collected_usd_total: string
  fees_collected_token_a_ui_total?: string | null
  fees_collected_token_b_ui_total?: string | null
  fees_collected_token_a_raw_total?: number | null
  fees_collected_token_b_raw_total?: number | null
  collect_events_total: number
}

export interface PositionStreamLineageResponse {
  position_address: string
  chain: string[]
  nodes: PositionStreamLineageNode[]
  totals?: PositionStreamPnLResponse | null
  chain_cost_summary?: LineageChainCostSummary | null
  note?: string | null
}

export interface ClosedPositionEntry {
  position_address: string
  pool_address: string
  token_mint_a?: string | null
  token_mint_b?: string | null
  token_a_label?: string | null
  token_b_label?: string | null
  owner: string
  close_kind?: string | null
  opened_ts_utc?: string | null
  closed_ts_utc?: string | null
  last_rebalance_session_id?: string | null
}

export interface ClosedPositionsResponse {
  total: number
  items: ClosedPositionEntry[]
  note?: string | null
}

export interface SuggestStrategyLinkResponse {
  strategy_id?: string | null
  reason: string
}

export interface PositionLifecycleEvent {
  ts_utc?: string | null
  source?: string | null
  event?: string | null
  operation?: string | null
  signature?: string | null
  pool_address?: string | null
  position_pubkey?: string | null
  rebalance_session_id?: string | null
  tx_fee_lamports?: number | null
  fee_payer_net_lamports_delta?: number | null
  fee_payer_token_deltas?: Record<string, string> | null
}

export interface PositionLifecycleSessionSummary {
  session_id: string
  events: PositionLifecycleEvent[]
  total_tx_fee_lamports: number
  rebalance_related_events: number
}

export interface PositionLifecycleSummaryResponse {
  position_address: string
  positions: string[]
  sessions: string[]
  total_tx_fee_lamports: number
  total_tx_fee_usd: string
  collect_events: number
  collected_fee_token_a_ui?: string | null
  collected_fee_token_a_raw?: number | null
  collected_fee_token_b_ui?: string | null
  collected_fee_token_b_raw?: number | null
  collected_fees_usd?: string | null
  realized_cashflow_usd: string
  session_summaries: PositionLifecycleSessionSummary[]
  note?: string | null
}

export interface BacktestFromClosedPositionRequest {
  position_address: string
  lower?: number
  upper?: number
  capital?: number
  strategy?: string
  start_date?: string
  end_date?: string
  fee_source?: string
  price_path_source?: string
  snapshot_protocol?: string
}

export interface BacktestFromOpenPositionRequest {
  position_address: string
  lower?: number
  upper?: number
  capital?: number
  strategy?: string
  start_date?: string
  end_date?: string
  fee_source?: string
  price_path_source?: string
  snapshot_protocol?: string
}

export interface BacktestJobStatusResponse {
  id: string
  status: string
  note?: string | null
}

export interface BacktestJobResponse {
  id: string
  position_address: string
  pool_address: string
  status: string
  started_ts_utc: string
  finished_ts_utc?: string | null
  exit_code?: number | null
  stdout?: string | null
  stderr?: string | null
  note?: string | null
}

export interface BacktestStrategyCatalogEntry {
  id: string
  label: string
  parameters: string[]
}

export interface BacktestStrategyCatalogResponse {
  strategies: BacktestStrategyCatalogEntry[]
}

export interface BacktestFullRequest {
  windows_hours: number[]
  include_strategy_ids?: string[]
  include_indicator_strategies?: boolean
  objective?: string
  pool_ids?: string[]
  snapshot_variants?: string[]
  lp_share?: number
  capital_usd?: number
  target_vs_hodl_usd?: number
  /** Optional fixed static deviation from entry, e.g. 10 => range = entry * (1 ± 10%). */
  static_deviation_pct?: number
  /** Static (single pool): manual lower bound. */
  static_manual_lower?: number
  /** Static (single pool): manual upper bound. */
  static_manual_upper?: number
  /** Optional fixed OOR-recenter deviation from entry, e.g. 10 => range = entry * (1 ± 10%). */
  oor_recenter_deviation_pct?: number
  threshold_grid_pct?: number[]
  threshold_min_rebalance_interval_hours?: number
  threshold_rebalance_on_range_exit_immediately?: boolean
  periodic_grid_steps?: number[]
  retouch_offset_pct?: number
  bollinger_window_grid?: number[]
  bollinger_k_grid?: number[]
  bollinger_rebalance_steps_grid?: number[]
  bollinger_rebalance_hours_grid?: number[]
  last_candle_steps_grid?: number[]
  last_candle_rebalance_steps_grid?: number[]
  last_candle_seconds_grid?: number[]
  last_candle_rebalance_seconds_grid?: number[]
}

export interface BacktestDataReadinessRequest {
  pool_ids?: string[]
  snapshot_variants?: string[]
  range_start_utc?: string
  range_end_utc?: string
}

export interface BacktestDataReadinessRow {
  pool_id: string
  pool_label: string
  protocol: string
  pool_address: string
  snapshot_variant: string
  cadence_minutes: number
  rows: number
  oldest_ts_utc?: string | null
  latest_ts_utc?: string | null
  oldest_continuous_ts_utc?: string | null
  max_gap_minutes?: number | null
  coverage_pct?: number | null
  max_backtest_hours_hard: number
  max_backtest_hours_recommended: number
  status: 'ok' | 'degraded' | 'recovering' | 'missing'
  status_reason?: string | null
  latest_age_secs?: number | null
  note?: string | null
}

export interface BacktestDataReadinessAggregate {
  pool_count: number
  variant_count: number
  max_backtest_hours_hard: number
  max_backtest_hours_recommended: number
  status: 'ok' | 'degraded' | 'recovering' | 'missing'
  status_ok_count: number
  status_degraded_count: number
  status_recovering_count: number
  status_missing_count: number
  source: 'db' | 'fallback'
  db_stale_rows: number
}

export interface BacktestDataReadinessThresholds {
  cache_ttl_secs: number
  db_max_age_secs: number
  hard_gap_multiplier: number
  recommended_coverage_pct: number
  recommended_gap_multiplier: number
  recommended_fallback_ratio: number
}

export interface BacktestDataReadinessResponse {
  rows: BacktestDataReadinessRow[]
  aggregate: BacktestDataReadinessAggregate
  thresholds: BacktestDataReadinessThresholds
}

export interface BacktestFullMetricRow {
  rank: number
  strategy: string
  lower_usd: number
  upper_usd: number
  width_pct: number
  score: number
  fees: number
  rebalances: number
  pnl: number
  vs_hodl: number
  tir_pct: number
  il_like_pct?: number | null
}

export interface BacktestFullWindowResult {
  pool_id: string
  pool_label: string
  pool_address: string
  protocol: string
  snapshot_variant: string
  window_hours: number
  metrics: BacktestFullMetricRow[]
  note?: string | null
}

export interface BacktestFullJobStatusResponse {
  id: string
  status: string
  note?: string | null
}

export interface BacktestFullJobResponse {
  id: string
  status: string
  started_ts_utc: string
  finished_ts_utc?: string | null
  stderr?: string | null
  note?: string | null
  results?: BacktestFullWindowResult[] | null
}

export interface BacktestAutoTuneStartRequest {
  interval_minutes?: number
  full_request: BacktestFullRequest
}

export interface BacktestAutoTuneWinner {
  pool_id: string
  pool_label: string
  window_hours: number
  strategy: string
  width_pct: number
  score: number
  pnl: number
  vs_hodl: number
  fees: number
  rebalances: number
  tir_pct: number
}

export interface BacktestAutoTuneStatusResponse {
  running: boolean
  interval_minutes: number
  started_ts_utc?: string | null
  last_tick_ts_utc?: string | null
  next_tick_ts_utc?: string | null
  latest_job_id?: string | null
  latest_winner?: BacktestAutoTuneWinner | null
  note?: string | null
}

export interface BacktestAutoTuneApplyResponse {
  strategy_id: string
  updated: boolean
  note: string
}

export interface PositionExperimentConfigResponse {
  position_address: string
  open_session_id?: string | null
  open_details?: Record<string, unknown> | null
  tick_lower?: number | null
  tick_upper?: number | null
  derived_lower?: number | null
  derived_upper?: number | null
  derived_initial_capital_usd?: number | null
  note?: string | null
}

export interface PnL {
  unrealized_pnl_usd: string
  unrealized_pnl_pct: string
  fees_earned_a: number
  fees_earned_b: number
  fees_earned_usd: string
  il_pct: string
  net_pnl_usd: string
  net_pnl_pct: string
}

export type StrategyType =
  | 'static_range'
  | 'periodic'
  | 'threshold'
  | 'bollinger'
  | 'il_limit'
  | 'oor_recenter'
  | 'retouch_shift'
  | 'last_candle'
  | 'last_candle_periodic'

export interface Strategy {
  id: string
  name: string
  description?: string | null
  strategy_type: StrategyType
  /** Legacy; new strategies omit — pool is chosen per position. */
  pool_address?: string | null
  running: boolean
  dry_run?: boolean
  auto_execute?: boolean
  parameters: StrategyParameters
  created_at: string
  updated_at: string
}

export type OptimizeApplyPolicy =
  | 'periodic_subprocess'
  | 'external_http'
  | 'combined'

export interface StrategyParameters {
  rebalance_threshold_pct?: number
  /** Bollinger: rolling window length in points/samples. */
  bollinger_window?: number
  /** Bollinger: standard deviation multiplier (k). */
  bollinger_k?: number
  retouch_offset_pct?: number
  max_il_pct?: number
  /** Legacy fallback (hours). UI should prefer minutes. */
  min_rebalance_interval_hours?: number
  /** Preferred live interval field (minutes). */
  min_rebalance_interval_minutes?: number
  /** Candle size in seconds for `last_candle` / `last_candle_periodic` strategy. */
  candle_seconds?: number
  range_width_pct?: number
  /** Periodic: when true, rebalance only if position is out of range. */
  periodic_requires_out_of_range?: boolean
  /** If true, range exit can trigger immediate close+open (rebalance). */
  rebalance_on_range_exit_immediately?: boolean
  /** If true, API may auto-start this strategy on boot when server env enables it. */
  auto_start?: boolean
  /** Populated when positions are linked (e.g. Open Position). */
  position_addresses?: string[]
  /** PDAs excluded from this strategy’s executor (automation off for those positions). */
  executor_disabled_position_addresses?: string[]
  /** Who may apply grid JSON: subprocess only, HTTP only, or both (see PROJECT_OVERVIEW). */
  optimize_apply_policy?: OptimizeApplyPolicy
}

// Payload for creating/updating a strategy via API
export interface CreateStrategyRequest {
  name: string
  strategy_type: StrategyType
  parameters: StrategyParameters
  /** Legacy; optional. Omit for new strategies (pool is chosen when opening a position). */
  pool_address?: string | null
  auto_execute?: boolean
  dry_run?: boolean
}

/** Matches API `PoolResponse` (`crates/api/src/models.rs`). */
export interface Pool {
  address: string
  protocol: string
  token_mint_a: string
  token_mint_b: string
  current_tick: number
  tick_spacing: number
  /** Decimal string from API */
  price: string
  liquidity: string
  fee_rate_bps: number
  volume_24h_usd?: string | null
  volume_1h_usd?: string | null
  volume_5m_usd?: string | null
  volume_7d_usd?: string | null
  tvl_usd?: string | null
  apy_estimate?: string | null
}

/** Matches API `PoolStateResponse`. */
export interface PoolState {
  address: string
  current_tick: number
  sqrt_price: string
  price: string
  liquidity: string
  fee_growth_global_a: string
  fee_growth_global_b: string
  /** ISO-8601 from serde */
  timestamp: string
}

/** Matches API `OrcaTokenResponse` (Orca REST proxy). */
export interface OrcaTokenResponse {
  mint: string
  symbol?: string | null
  name?: string | null
  decimals?: number | null
  verified?: boolean | null
  price_usdc?: string | null
}

/** Sums `bot_collect_fees` rows in lifecycle JSONL (API host). */
export interface FeesCollectedFromLedger {
  file_missing: boolean
  collect_events: number
  sum_token_a_ui: string | number | null
  sum_token_b_ui: string | number | null
}

/** Matches API `PortfolioAnalyticsResponse` (IL is average % across monitored positions). */
export interface PortfolioAnalytics {
  total_value_usd: string
  total_pnl_usd: string
  total_pnl_pct: string
  total_fees_usd: string
  total_il_pct: string
  active_positions: number
  positions_in_range?: number
  best_position: string | null
  worst_position: string | null
  fees_collected_from_ledger: FeesCollectedFromLedger
}

/** Matches API `SimulationRequest` — strategy defaults to `static_range` if omitted (serde default). */
export interface SimulationRequest {
  pool_address: string
  tick_lower: number
  tick_upper: number
  /** Decimal string */
  initial_capital_usd: string
  start_date: string
  end_date: string
  strategy_type?:
    | 'static_range'
    | 'periodic'
    | 'threshold'
    | 'oor_recenter'
    | 'il_limit'
    | 'retouch_shift'
    | 'last_candle'
  gbm_volatility?: number
  gbm_drift?: number
  periodic_interval_steps?: number
  /** Decimal string, e.g. "0.05" */
  threshold_pct?: string
  il_limit_pct?: string
}

/** Matches API `SimulationResponse` after full `clmm_lp_simulation` run. */
export interface SimulationResponse {
  id: string
  pool_address: string
  tick_lower: number
  tick_upper: number
  initial_capital_usd: string
  final_value_usd: string
  total_return_pct: string
  fee_earnings_pct: string
  il_pct: string
  sharpe_ratio: string
  max_drawdown_pct: string
  time_in_range_pct: string
  vs_hodl_usd: string
  rebalance_count: number
  methodology_note: string
}

export interface HealthResponse {
  status: string
  version: string
  uptime_secs: number
  components: Record<string, ComponentHealth>
}

export interface ComponentHealth {
  status: string
  latency_ms: number | null
  message: string | null
}

// API functions

/** Best-effort message from failed fetch (JSON `message`, plain text, or HTML hint). */
function messageFromErrorBody(text: string, status: number): string {
  // Prefer app-selected locale (Settings) over browser language.
  // `I18nProvider` stores it under `clmm.locale`.
  const isPl = (() => {
    try {
      if (typeof window !== 'undefined') {
        const raw = window.localStorage.getItem('clmm.locale')
        if (raw === 'pl') return true
        if (raw === 'en') return false
      }
    } catch {
      /* ignore */
    }
    return (
      typeof navigator !== 'undefined' &&
      typeof navigator.language === 'string' &&
      navigator.language.toLowerCase().startsWith('pl')
    )
  })()
  const statusLine = `HTTP ${status}`
  const trimmed = text.trim()
  const localizeKnownError = (raw: string): string => {
    const partialUnwrap = raw.match(
      /wsol_to_native failed:\s*wsol unwrap failed:\s*partial unwrap.*requested (\d+) raw,\s*current WSOL (\d+) raw/i,
    )
    if (partialUnwrap) {
      const req = Number(partialUnwrap[1])
      const cur = Number(partialUnwrap[2])
      if (Number.isFinite(req) && Number.isFinite(cur)) {
        if (isPl) {
          return `Konwersja WSOL->SOL: częściowy unwrap jest teraz wspierany, ale ta próba nie powiodła się. Żądano ${req.toLocaleString()} lamportów, aktualne WSOL ${cur.toLocaleString()} lamportów. Spróbuj ponownie.`
        }
        return `WSOL->SOL conversion: partial unwrap is supported, but this attempt failed. Requested ${req.toLocaleString()} lamports, current WSOL ${cur.toLocaleString()} lamports. Retry the operation.`
      }
    }

    const legacyPartialUnwrap = raw.match(/partial unwrap is not supported yet in safe mode/i)
    if (legacyPartialUnwrap) {
      if (isPl) {
        return 'Konwersja częściowego WSOL->SOL była chwilowo niedostępna w trybie bezpiecznym. Zaktualizuj backend i spróbuj ponownie.'
      }
      return 'Partial WSOL->SOL conversion was temporarily unavailable in safe mode. Update backend and retry.'
    }

    const partialRewrapFailed = raw.match(/wsol unwrap partial: close succeeded but remainder re-wrap failed: (.+)$/i)
    if (partialRewrapFailed) {
      const reason = partialRewrapFailed[1]?.trim() || raw
      if (isPl) {
        return `Konwersja WSOL->SOL: zamknięcie WSOL się udało, ale odtworzenie pozostałej części WSOL nie powiodło się (${reason}). Sprawdź saldo i spróbuj ponownie.`
      }
      return `WSOL->SOL conversion: WSOL close succeeded, but re-wrapping the remainder failed (${reason}). Check balances and retry.`
    }

    const insufficientNative = raw.match(/insufficient native SOL balance \(have (\d+) raw, need (\d+) raw\)/i)
    if (insufficientNative) {
      const have = Number(insufficientNative[1])
      const need = Number(insufficientNative[2])
      if (Number.isFinite(have) && Number.isFinite(need)) {
        const fmtSol = (v: number) => (v / 1e9).toLocaleString(undefined, { maximumFractionDigits: 6 })
        if (isPl) {
          return `Za mało natywnego SOL. Masz ~${fmtSol(have)} SOL (${have.toLocaleString()} lamportów), wymagane ~${fmtSol(need)} SOL (${need.toLocaleString()} lamportów).`
        }
        return `Insufficient native SOL. Have ~${fmtSol(have)} SOL (${have.toLocaleString()} lamports), need ~${fmtSol(need)} SOL (${need.toLocaleString()} lamports).`
      }
    }

    const insufficientWsol = raw.match(/insufficient WSOL balance \(have (\d+) raw, need (\d+) raw\)/i)
    if (insufficientWsol) {
      const have = Number(insufficientWsol[1])
      const need = Number(insufficientWsol[2])
      if (Number.isFinite(have) && Number.isFinite(need)) {
        const fmt = (v: number) => (v / 1e9).toLocaleString(undefined, { maximumFractionDigits: 6 })
        if (isPl) {
          return `Za mało WSOL. Masz ~${fmt(have)} WSOL, wymagane ~${fmt(need)} WSOL.`
        }
        return `Insufficient WSOL. Have ~${fmt(have)} WSOL, need ~${fmt(need)} WSOL.`
      }
    }

    const transferInsufficientSol = raw.match(/insufficient SOL: have (\d+) lamports, need at least (\d+) \+ fee reserve/i)
    if (transferInsufficientSol) {
      const have = Number(transferInsufficientSol[1])
      const need = Number(transferInsufficientSol[2])
      if (Number.isFinite(have) && Number.isFinite(need)) {
        if (isPl) {
          return `Za mało SOL na transfer. Masz ${have.toLocaleString()} lamportów, potrzeba co najmniej ${need.toLocaleString()} lamportów + rezerwa na fee.`
        }
        return `Insufficient SOL for transfer. Have ${have.toLocaleString()} lamports, need at least ${need.toLocaleString()} lamports + fee reserve.`
      }
    }

    const normalizePrefix = (s: string) => s.toLowerCase()
    const lower = normalizePrefix(raw)
    if (lower.startsWith('open position failed: api host cannot sign transactions')) {
      return isPl
        ? 'Otwarcie pozycji nie powiodło się: host API nie może podpisać transakcji (brak skonfigurowanego portfela/executora).'
        : 'Open position failed: API host cannot sign transactions (missing configured wallet/executor).'
    }
    if (lower.startsWith('open position failed: slippage/min-out too tight')) {
      return isPl
        ? 'Otwarcie pozycji nie powiodło się: zbyt ciasny slippage/min-out względem ruchu ceny puli. Zwiększ slippage lub zmniejsz kwotę.'
        : 'Open position failed: slippage/min-out too tight vs pool move. Increase slippage or lower amount.'
    }
    if (lower.startsWith('open position failed: invalid tick bounds')) {
      return isPl
        ? 'Otwarcie pozycji nie powiodło się: nieprawidłowe ticki dla tej puli (spacing/range).'
        : 'Open position failed: invalid tick bounds for this pool (spacing/range).'
    }
    if (lower.startsWith('open position failed: insufficient funds/tokens')) {
      return isPl
        ? 'Otwarcie pozycji nie powiodło się: za mało środków/tokenów dla zadanych limitów. Doładuj portfel API signer albo zmniejsz kwoty.'
        : 'Open position failed: insufficient funds/tokens for requested caps. Fund API signer wallet or lower amounts.'
    }

    if (lower.startsWith('close position failed: whirlpool position is not empty yet')) {
      return isPl
        ? 'Zamknięcie pozycji nie powiodło się: pozycja Whirlpool nie jest pusta (najpierw zdejmij płynność/odbierz opłaty).'
        : 'Close position failed: Whirlpool position is not empty yet (remove liquidity/collect fees first).'
    }
    if (lower.startsWith('close position failed: whirlpool account ownership mismatch')) {
      return isPl
        ? 'Zamknięcie pozycji nie powiodło się: niezgodność właściciela konta Whirlpool.'
        : 'Close position failed: Whirlpool account ownership mismatch.'
    }
    if (lower.startsWith('close position failed: api host cannot sign transactions')) {
      return isPl
        ? 'Zamknięcie pozycji nie powiodło się: host API nie może podpisać transakcji (brak skonfigurowanego portfela/executora).'
        : 'Close position failed: API host cannot sign transactions (missing configured wallet/executor).'
    }
    if (lower.startsWith('close position failed: whirlpool min-out/slippage too tight')) {
      return isPl
        ? 'Zamknięcie pozycji nie powiodło się: zbyt ciasny min-out/slippage względem ruchu ceny puli.'
        : 'Close position failed: min-out/slippage too tight vs pool move.'
    }
    if (lower.startsWith('close position failed: insufficient funds/tokens')) {
      return isPl
        ? 'Zamknięcie pozycji nie powiodło się: za mało środków/tokenów na portfelu podpisującym.'
        : 'Close position failed: insufficient funds/tokens on signer wallet.'
    }

    return raw
  }
  if (!trimmed) {
    if (status === 408) {
      return `${statusLine} (empty body) — zwykle timeout warstwy HTTP API (Tower) albo proxy; endpoint /positions/:addr wymaga dłuższego limitu po stronie serwera (on-chain router) i wolnego RPC.`
    }
    return `${statusLine} (empty body)`
  }
  try {
    const j = JSON.parse(trimmed) as Record<string, unknown>
    const m = j.message ?? j.error ?? j.detail ?? j.title
    if (typeof m === 'string' && m.length > 0) {
      return localizeKnownError(m)
    }
  } catch {
    /* not JSON */
  }
  if (trimmed.startsWith('<!') || trimmed.startsWith('<html')) {
    return `${statusLine} — odpowiedź to HTML (zły URL API albo proxy Vite nie trafia na backend).`
  }
  const normalized = localizeKnownError(trimmed)
  return normalized.length > 400 ? `${normalized.slice(0, 400)}…` : normalized
}

async function fetchJsonWithTimeout<T>(
  url: string,
  timeoutMs: number,
  options?: RequestInit,
): Promise<T> {
  const ctrl = new AbortController()
  const t = setTimeout(() => ctrl.abort(), timeoutMs)
  let response: Response
  try {
    response = await fetch(`${API_BASE}${url}`, {
      ...options,
      signal: options?.signal ?? ctrl.signal,
      headers: {
        'Content-Type': 'application/json',
        Accept: 'application/json',
        ...(API_KEY && API_KEY.trim().length > 0 ? { 'X-API-Key': API_KEY.trim() } : {}),
        ...options?.headers,
      },
    })
  } catch (e) {
    // Browser abort (timeout or navigation). Make it actionable.
    if (e instanceof DOMException && e.name === 'AbortError') {
      throw new Error(
        `Request timed out in UI after ${(timeoutMs / 1000).toFixed(0)}s (endpoint ${API_BASE}${url}). ` +
          `API may still be processing the transaction; check Positions/Registry and retry if needed.`,
      )
    }
    throw e
  } finally {
    clearTimeout(t)
  }

  const text = await response.text()

  if (!response.ok) {
    throw new Error(messageFromErrorBody(text, response.status))
  }

  if (!text.trim()) {
    throw new Error(`Empty response from server (HTTP ${response.status})`)
  }

  try {
    return JSON.parse(text) as T
  } catch {
    const preview = text.slice(0, 280)
    throw new Error(
      `Invalid JSON from server (HTTP ${response.status}): ${preview}${text.length > 280 ? '…' : ''}`,
    )
  }
}

async function fetchJson<T>(url: string, options?: RequestInit): Promise<T> {
  return fetchJsonWithTimeout(url, 15_000, options)
}

async function fetchJsonLong<T>(url: string, options?: RequestInit): Promise<T> {
  // On-chain operations (swap/open/close) can take longer than regular API reads.
  // Keep this comfortably above API on-chain timeout (see API_ONCHAIN_REQUEST_TIMEOUT_SECS).
  return fetchJsonWithTimeout(url, 180_000, options)
}

// ============================================================================
// External (free) price sources
// ============================================================================

export type MintPricesUsdResponse = {
  source: string
  requested: number
  returned: number
  prices: Record<string, number>
}

export async function getMintPricesUsd(mints: string[]): Promise<MintPricesUsdResponse> {
  const ids = [...new Set(mints.map((m) => m.trim()).filter(Boolean))]
  if (ids.length === 0) return { source: 'none', requested: 0, returned: 0, prices: {} }
  // Prefer server-side proxy to avoid browser CORS/adblock issues.
  return await fetchJson<MintPricesUsdResponse>(
    `/prices/jupiter?${new URLSearchParams({ ids: ids.join(',') })}`,
  )
}

export async function getJupiterPricesUsd(mints: string[]): Promise<Record<string, number>> {
  const ids = [...new Set(mints.map((m) => m.trim()).filter(Boolean))]
  if (ids.length === 0) return {}
  // Prefer server-side proxy to avoid browser CORS/adblock issues.
  try {
    const r = await getMintPricesUsd(ids)
    return r.prices ?? {}
  } catch {
    // Legacy public `price.jup.ag/v4` is often down; pricing is resolved server-side in `/prices/jupiter`.
    return {}
  }
}

// Health
export const getHealth = () => fetchJson<HealthResponse>('/health')
export const getLiveness = async () => {
  const ctrl = new AbortController()
  const t = setTimeout(() => ctrl.abort(), 2000)
  try {
    const r = await fetch(`${API_BASE}/health/live`, { signal: ctrl.signal })
    if (!r.ok) throw new Error(`HTTP ${r.status}`)
    return await r.text()
  } finally {
    clearTimeout(t)
  }
}

/** On-chain Orca scan (`GET /orca/positions-by-owner`); same family as `orca-positions-list` CLI. */
export interface OrcaOwnerPositionEntry {
  kind: string
  position_address: string
  pool_address: string
  tick_lower: number
  tick_upper: number
  range_lower_usdc?: string | number | null
  range_upper_usdc?: string | number | null
  range_usdc_quote?: string | null
  range_lower_price?: string | number | null
  range_upper_price?: string | number | null
  range_price_quote?: string | null
  liquidity: string
  position_mint?: string | null
  position_bundle_address?: string | null
  in_range?: boolean
  token_a_label?: string | null
  token_b_label?: string | null
  token_mint_a?: string | null
  token_mint_b?: string | null
  token_price_a_usd?: number | null
  token_price_b_usd?: number | null
}

export interface OrcaOwnerPositionsResponse {
  owner: string
  rpc_url: string
  total: number
  entries: OrcaOwnerPositionEntry[]
}

export const getOrcaPositionsByOwner = (owner: string) =>
  fetchJson<OrcaOwnerPositionsResponse>(
    `/orca/positions-by-owner?${new URLSearchParams({ owner: owner.trim() })}`,
  )

export interface MarketSnapshotRow {
  ts_utc: string
  protocol: string
  pool_address: string
  source_path: string
  price_ab?: string | number | null
  liquidity_active_raw?: string | number | null
}

export interface MarketSnapshotsResponse {
  scanned_files: number
  rows_returned: number
  rows: MarketSnapshotRow[]
}

export interface MarketDataQueryParams {
  protocol?: string
  pool?: string
  from?: string
  to?: string
  limit?: number
}

export const getDataSnapshots = (params: MarketDataQueryParams) =>
  fetchJson<MarketSnapshotsResponse>(
    `/data/snapshots?${new URLSearchParams(
      Object.entries(params)
        .filter(([, v]) => v !== undefined && v !== null && String(v).trim() !== '')
        .map(([k, v]) => [k, String(v)]),
    )}`,
  )

// Positions
export const getPositions = () => fetchJson<{ positions: Position[] }>('/positions')
// Registry replay is fast; pool mint enrichment is one RPC per unique pool on this page only,
// but keep UI timeout above default 15s for slow RPC / large offsets.
export const getClosedPositions = (
  limit = 100,
  offset = 0,
  enrichPools: boolean = true,
) => {
  const params = new URLSearchParams({
    limit: String(limit),
    offset: String(offset),
  })
  if (!enrichPools) {
    params.set('enrich_pools', 'false')
  }
  return fetchJsonWithTimeout<ClosedPositionsResponse>(`/positions/closed?${params}`, 60_000)
}

/** React Query: one key for registry-only vs enriched list. */
export const closedPositionsListQueryKey = (
  limit: number,
  offset: number,
  enrichPools: boolean,
) => ['closed-positions', limit, offset, enrichPools] as const

export function closedPositionsListQueryOptions(
  limit = 100,
  offset = 0,
  enrichPools = true,
) {
  return {
    queryKey: closedPositionsListQueryKey(limit, offset, enrichPools),
    queryFn: () => getClosedPositions(limit, offset, enrichPools),
    staleTime: 1000 * 60 * 5,
    retry: 1 as const,
  }
}
/** Matches API on-chain router timeout (`API_ONCHAIN_REQUEST_TIMEOUT_SECS`, default 120s): many RPC + prices. */
export const getPosition = (address: string) =>
  fetchJsonWithTimeout<Position>(`/positions/${encodeURIComponent(address)}`, 120_000)
export const getPositionAgentChatUi = (address: string) =>
  fetchJson<AgentChatUiPayload>(`/positions/${encodeURIComponent(address)}/agent-chat/ui`)
export const startPositionAgent = (address: string, scanIntervalHours?: number) =>
  fetchJson<{ session: AgentPositionSession }>(`/positions/${encodeURIComponent(address)}/agent/start`, {
    method: 'POST',
    body: JSON.stringify(
      scanIntervalHours && Number.isFinite(scanIntervalHours)
        ? { scan_interval_hours: Math.max(1, Math.floor(scanIntervalHours)) }
        : {},
    ),
  })
export const sendPositionAgentMessage = (address: string, content: string) =>
  fetchJson<AgentChatMessage>(`/positions/${encodeURIComponent(address)}/agent/message`, {
    method: 'POST',
    body: JSON.stringify({ content }),
  })
export const sendPositionAgentLlmReply = (
  address: string,
  prompt: string,
  context?: Record<string, unknown>,
) =>
  fetchJson<AgentLlmReplyResponse>(`/positions/${encodeURIComponent(address)}/agent/llm-reply`, {
    method: 'POST',
    body: JSON.stringify(
      context && Object.keys(context).length > 0 ? { prompt, context } : { prompt },
    ),
  })
export const triggerPositionAgentScan = (address: string, includeCrossPairScan = true) =>
  fetchJson(`/positions/${encodeURIComponent(address)}/agent/scan-now`, {
    method: 'POST',
    body: JSON.stringify({ include_cross_pair_scan: includeCrossPairScan }),
  })
export const getPositionAgentSupervisor = (address: string) =>
  fetchJson<AgentPositionSupervisor>(`/positions/${encodeURIComponent(address)}/agent/supervisor`)
export const getPositionDiagnostics = (address: string) =>
  fetchJson<PositionDiagnosticsResponse>(`/positions/${encodeURIComponent(address)}/diagnostics`)
export const suggestPositionStrategy = (address: string) =>
  fetchJson<SuggestStrategyLinkResponse>(
    `/positions/${encodeURIComponent(address)}/suggest-strategy`,
  )
export const getPositionStreamPerformance = (address: string) =>
  fetchJson<PositionStreamPerformanceResponse>(
    `/positions/${encodeURIComponent(address)}/stream-performance`,
  )
export const getPositionStreamPnL = (address: string, mode: 'live' | 'settlement_v1' = 'live') =>
  fetchJson<PositionStreamPnLResponse>(
    `/positions/${encodeURIComponent(address)}/stream-pnl?${new URLSearchParams({ mode })}`,
  )
export const getPositionStreamLineage = (
  address: string,
  mode: 'live' | 'settlement_v1' = 'live',
) =>
  // Lineage reconstruction can be slow when DB is disabled and API scans JSONL.
  // Keep this above the default 15s UI timeout to avoid false "no lineage" states.
  fetchJsonWithTimeout<PositionStreamLineageResponse>(
    `/positions/${encodeURIComponent(address)}/stream-lineage?${new URLSearchParams({ mode })}`,
    120_000,
  )
export const getPositionLifecycleSummary = (address: string) =>
  fetchJson<PositionLifecycleSummaryResponse>(
    `/positions/${encodeURIComponent(address)}/lifecycle-summary`,
  )

export const runBacktestFromClosedPosition = (body: BacktestFromClosedPositionRequest) =>
  fetchJsonLong<BacktestJobStatusResponse>('/backtests/from-closed-position', {
    method: 'POST',
    body: JSON.stringify(body),
  })

export const runBacktestFromOpenPosition = (body: BacktestFromOpenPositionRequest) =>
  fetchJsonLong<BacktestJobStatusResponse>('/backtests/from-open-position', {
    method: 'POST',
    body: JSON.stringify(body),
  })

export const getBacktestJob = (id: string) =>
  fetchJson<BacktestJobResponse>(`/backtests/${encodeURIComponent(id)}`)

export const getBacktestStrategyCatalog = () =>
  fetchJson<BacktestStrategyCatalogResponse>('/backtests/strategy-catalog')

export const startBacktestFull = (body: BacktestFullRequest) =>
  fetchJsonLong<BacktestFullJobStatusResponse>('/backtests/full', {
    method: 'POST',
    body: JSON.stringify(body),
  })

export const getBacktestFullJob = (id: string) =>
  fetchJson<BacktestFullJobResponse>(`/backtests/full/${encodeURIComponent(id)}`)

export const getBacktestDataReadiness = (body: BacktestDataReadinessRequest) =>
  fetchJson<BacktestDataReadinessResponse>('/backtests/data-readiness', {
    method: 'POST',
    body: JSON.stringify(body),
  })

export const startBacktestAutoTune = (body: BacktestAutoTuneStartRequest) =>
  fetchJsonLong<BacktestAutoTuneStatusResponse>('/backtests/auto-tune/start', {
    method: 'POST',
    body: JSON.stringify(body),
  })

export const stopBacktestAutoTune = () =>
  fetchJson<BacktestAutoTuneStatusResponse>('/backtests/auto-tune/stop', {
    method: 'POST',
  })

export const getBacktestAutoTuneStatus = () =>
  fetchJson<BacktestAutoTuneStatusResponse>('/backtests/auto-tune/status')

export const applyBacktestAutoTuneToStrategy = (strategyId: string) =>
  fetchJson<BacktestAutoTuneApplyResponse>(
    `/backtests/auto-tune/apply/${encodeURIComponent(strategyId)}`,
    {
      method: 'POST',
    },
  )

export const getPositionExperimentConfig = (address: string) =>
  fetchJson<PositionExperimentConfigResponse>(
    `/positions/${encodeURIComponent(address)}/experiment-config`,
  )
/** Orca swap in the **same pool** (ExactIn) before open — executed by API wallet, then open tx. */
export interface SwapInPoolBeforeOpen {
  specified_mint: string
  amount_in: number
}

/** `POST /positions/swap-before-open` — SWAP-only step (ExactIn in a Whirlpool). */
export interface SwapBeforeOpenRequest {
  pool_address: string
  specified_mint: string
  amount_in: number
  /** Defaults to server-side `default_slippage` when omitted. */
  slippage_tolerance_bps?: number
  /** Groups swap + open rows in `orca_position_lifecycle.jsonl` for per-position cost sums. */
  cost_session_id?: string
}

export interface SwapBeforeOpenResponse {
  message: string
  swap_signature?: string
  cost_session_id?: string
}

export const swapBeforeOpen = (data: SwapBeforeOpenRequest) =>
  fetchJsonLong<SwapBeforeOpenResponse>('/positions/swap-before-open', {
    method: 'POST',
    body: JSON.stringify(data),
  })

/** `POST /positions` success body (open position + optional swap metadata). */
export interface PositionOpenResponse {
  message: string
  position_pda?: string
  swap_signature?: string
  /** Same id on swap + open rows in `orca_position_lifecycle.jsonl` for cost sums. */
  cost_session_id?: string
}

export const openPosition = (data: {
  pool_address: string
  tick_lower: number
  tick_upper: number
  amount_a: number
  amount_b: number
  /** If set, API appends the new position PDA to this strategy's `parameters.position_addresses`. */
  strategy_id?: string
  /** Optional: swap in this Whirlpool first (server-side keypair), then open position. */
  swap_before_open?: SwapInPoolBeforeOpen
  /** Groups swap + open ledger rows for per-position cost accounting. */
  cost_session_id?: string
}) => fetchJsonLong<PositionOpenResponse>('/positions', {
  method: 'POST',
  body: JSON.stringify(data),
})
export const closePosition = (address: string, cost_session_id?: string) => {
  const qs =
    cost_session_id && cost_session_id.trim()
      ? `?${new URLSearchParams({ cost_session_id: cost_session_id.trim() })}`
      : ''
  return fetchJsonLong<{ message: string }>(`/positions/${address}${qs}`, { method: 'DELETE' })
}
export const collectFees = (address: string, cost_session_id?: string) => {
  const qs =
    cost_session_id && cost_session_id.trim()
      ? `?${new URLSearchParams({ cost_session_id: cost_session_id.trim() })}`
      : ''
  return fetchJsonLong<{ message: string }>(`/positions/${address}/collect${qs}`, { method: 'POST' })
}
export type RebalanceInputKind = 'ticks' | 'strategy_range' | 'price_band'

/** POST /positions/:address/rebalance — tick bounds or auto range from strategy / price. */
export type RebalancePayload = {
  input?: RebalanceInputKind
  new_tick_lower?: number
  new_tick_upper?: number
  strategy_id?: string
  /** Decimal string, e.g. `"1.0"` for 1% width */
  range_width_pct?: string
  /** Decimal string, B per A (same convention as pool price) */
  center_price?: string
  slippage_tolerance_bps?: number
}

export const rebalancePosition = (address: string, data: RebalancePayload) =>
  fetchJsonLong<{ message: string }>(`/positions/${address}/rebalance`, {
    method: 'POST',
    body: JSON.stringify({ slippage_tolerance_bps: 50, ...data }),
  })

/** `liquidity_amount`: base units as decimal string (matches API u128 as string). */
export const decreaseLiquidity = (address: string, liquidity_amount: string) =>
  fetchJsonLong<{ message: string }>(`/positions/${address}/decrease`, {
    method: 'POST',
    body: JSON.stringify({ liquidity_amount }),
  })

/** Link, move, or unlink this position from strategies (`parameters.position_addresses`). */
export const linkPositionStrategy = (address: string, body: { strategy_id: string | null }) =>
  fetchJsonLong<{ message: string }>(
    `/positions/${encodeURIComponent(address)}/strategy`,
    {
      method: 'POST',
      body: JSON.stringify(body),
    },
  )

// Strategies
export const getStrategies = () => fetchJson<{ strategies: Strategy[] }>('/strategies')
export const getStrategy = (id: string) => fetchJson<Strategy>(`/strategies/${id}`)
export const createStrategy = (data: CreateStrategyRequest) =>
  fetchJson<Strategy>('/strategies', { method: 'POST', body: JSON.stringify(data) })
export const updateStrategy = (id: string, data: CreateStrategyRequest) =>
  fetchJson<Strategy>(`/strategies/${id}`, { method: 'PUT', body: JSON.stringify(data) })
export const deleteStrategy = (id: string) =>
  fetchJson<{ message: string }>(`/strategies/${id}`, { method: 'DELETE' })
export const startStrategy = (id: string) =>
  fetchJson<{ message: string }>(`/strategies/${id}/start`, { method: 'POST' })
export const stopStrategy = (id: string) =>
  fetchJson<{ message: string }>(`/strategies/${id}/stop`, { method: 'POST' })

/** `enabled: true` = run automation for this position; `false` = add to executor-disabled list. */
export const setStrategyPositionExecutor = (
  strategyId: string,
  positionAddress: string,
  enabled: boolean,
) =>
  fetchJson<{ message: string }>(`/strategies/${strategyId}/position-executor`, {
    method: 'POST',
    body: JSON.stringify({
      position_address: positionAddress,
      enabled,
    }),
  })

// Pools
export const getPools = () => fetchJson<{ pools: Pool[] }>('/pools')
export const getPool = (address: string) => fetchJson<Pool>(`/pools/${address}`)
export const getPoolState = (address: string) => fetchJson<PoolState>(`/pools/${address}/state`)

/** `GET /pools/:address/estimate-swap-cost` — rough network fee from local ledger + default. */
export interface SwapCostEstimateResponse {
  pool_address: string
  historical_median_network_fee_lamports?: number | null
  historical_sample_count: number
  default_network_fee_lamports: number
  estimated_network_fee_lamports: number
  note: string
}

export const getSwapCostEstimate = (poolAddress: string) =>
  fetchJson<SwapCostEstimateResponse>(
    `/pools/${encodeURIComponent(poolAddress.trim())}/estimate-swap-cost`,
  )

/** `POST /pools/:address/quote-open-budget` — caps targeting ~`target_usd` in-range notional. */
export interface QuoteOpenBudgetRequest {
  tick_lower: number
  tick_upper: number
  target_usd: number
}

export interface QuoteOpenBudgetResponse {
  token_max_a: number
  token_max_b: number
  amount_a: number
  amount_b: number
  amount_a_ui: number
  amount_b_ui: number
  estimated_value_usd: number
  liquidity: string
  in_range: boolean
  note?: string
}

export const quoteOpenBudget = (poolAddress: string, body: QuoteOpenBudgetRequest) =>
  fetchJson<QuoteOpenBudgetResponse>(
    `/pools/${encodeURIComponent(poolAddress.trim())}/quote-open-budget`,
    {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    },
  )

export const getOrcaToken = (mint: string) =>
  fetchJson<OrcaTokenResponse>(`/orca/tokens/${encodeURIComponent(mint.trim())}`)

// Analytics
export const getPortfolioAnalytics = () => fetchJson<PortfolioAnalytics>('/analytics/portfolio')
export const runSimulation = (data: SimulationRequest) =>
  fetchJson<SimulationResponse>('/analytics/simulate', { method: 'POST', body: JSON.stringify(data) })

// Wallets (local keypairs directory + on-chain balances)
export interface WalletEntry {
  id: string
  filename: string
  pubkey: string
  present_in_primary?: boolean
  present_in_secondary?: boolean
  replication_status?: 'healthy' | 'degraded' | 'conflict'
  fingerprint?: string | null
}

export interface WalletsListResponse {
  wallets_dir_primary: string
  wallets_dir_secondary?: string | null
  transfer_min_lamports: number
  transfer_max_lamports?: number | null
  wallets: WalletEntry[]
}

export interface CreateWalletRequest {
  wallet_id?: string
  force?: boolean
}

export interface CreateWalletResponse {
  wallet: WalletEntry
  primary_written: boolean
  secondary_written: boolean
  note?: string | null
}

export interface ActiveSignerResponse {
  wallet_id?: string | null
  pubkey?: string | null
  source: string
}

export interface SetActiveSignerRequest {
  wallet_id: string
}

export interface WalletTransferRequest {
  from_wallet_id: string
  to_pubkey: string
  lamports: number
}

export interface WalletTransferResponse {
  from_wallet_id: string
  from_pubkey: string
  to_pubkey: string
  lamports: number
  signature: string
}

export interface WalletTokenBalance {
  mint: string
  ui_amount: string
}

export interface WalletBalancesResponse {
  owner: string
  rpc_url: string
  lamports: number
  sol: string
  tokens: WalletTokenBalance[]
  token_accounts_total?: number
  token_legacy_ok?: boolean
  token_2022_ok?: boolean
  token_legacy_error?: string | null
  token_2022_error?: string | null
}

export type WalletBalanceConfidence = 'verified' | 'projected' | 'degraded'

export interface WalletEffectiveBalancesResponse extends WalletBalancesResponse {
  as_of_utc: string
  is_stale: boolean
  stale_age_ms: number
  confidence: WalletBalanceConfidence
  pending_ops_count: number
  native_onchain_lamports: number
  native_effective_lamports: number
  wsol_onchain_raw: number
  wsol_effective_raw: number
}

export interface ApiSignerWalletResponse {
  configured: boolean
  pubkey?: string | null
  rpc_url: string
  lamports?: number | null
  sol?: string | null
  /** Min SOL for open/rent-heavy ops (default ~0.01 SOL). */
  min_open_lamports: number
  /** Min SOL for swap-only fee buffer (default ~0.0015 SOL; lower than open). */
  min_swap_lamports: number
  note?: string | null
}

export type ConvertSolDirection = 'native_to_wsol' | 'wsol_to_native'

export interface ConvertSolRequest {
  direction: ConvertSolDirection
  amount_raw: number
}

export interface ConvertSolResponse {
  message: string
  signature?: string | null
  wrap_signature?: string | null
  unwrap_signature?: string | null
  rewrap_signature?: string | null
  confirmed: boolean
  partial: boolean
  op_id: string
  reconciliation_status:
    | 'pending_confirmation'
    | 'confirmed_unreconciled'
    | 'reconciled'
    | 'mismatch'
    | 'failed'
  reason_code?: string | null
  attempts: number
  last_verified_at_utc?: string | null
  direction: ConvertSolDirection
  amount_raw: number
  owner_pubkey: string
  post_native_lamports: number
  post_wsol_raw: number
}

export interface WalletConvertOpResponse {
  op_id: string
  owner_pubkey: string
  direction: ConvertSolDirection
  amount_raw: number
  reconciliation_status:
    | 'pending_confirmation'
    | 'confirmed_unreconciled'
    | 'reconciled'
    | 'mismatch'
    | 'failed'
  reason_code?: string | null
  attempts: number
  created_at_utc: string
  updated_at_utc: string
  last_verified_at_utc?: string | null
  last_error?: string | null
  post_native_lamports?: number | null
  post_wsol_raw?: number | null
}

export interface WalletOpsStatsResponse {
  total: number
  reconciled: number
  confirmed_unreconciled: number
  mismatch: number
  failed: number
  pending_confirmation: number
  mismatch_ratio?: number | null
  avg_seconds_to_reconcile?: number | null
}

export interface WalletWsStatusResponse {
  owners_monitored: number
  owners: string[]
  events_total: number
  reconnects_total: number
  refresh_failures_total: number
}

export const getWallets = () => fetchJson<WalletsListResponse>('/wallets')
export const createWallet = (body: CreateWalletRequest) =>
  fetchJson<CreateWalletResponse>('/wallets/create', {
    method: 'POST',
    body: JSON.stringify(body),
  })
export const getWalletBalances = (owner: string) =>
  // This call may require multiple RPC fallbacks; allow longer than the global 15s timeout.
  fetchJsonWithTimeout<WalletBalancesResponse>(
    `/wallets/balances?${new URLSearchParams({ owner: owner.trim() })}`,
    35_000,
  )
export const getWalletEffectiveBalances = (owner: string) =>
  fetchJsonWithTimeout<WalletEffectiveBalancesResponse>(
    `/wallets/effective-balances?${new URLSearchParams({ owner: owner.trim() })}`,
    15_000,
  )

export const getApiSignerWallet = () => fetchJson<ApiSignerWalletResponse>('/wallets/api-signer')
export const getActiveSigner = () => fetchJson<ActiveSignerResponse>('/wallets/active-signer')
export const setActiveSigner = (body: SetActiveSignerRequest) =>
  fetchJson<ActiveSignerResponse>('/wallets/active-signer', {
    method: 'POST',
    body: JSON.stringify(body),
  })
export const convertSol = (body: ConvertSolRequest) =>
  fetchJsonLong<ConvertSolResponse>('/wallets/convert-sol', {
    method: 'POST',
    body: JSON.stringify(body),
  })
export const getWalletConvertOp = (opId: string) =>
  fetchJson<WalletConvertOpResponse>(`/wallets/ops/${encodeURIComponent(opId)}`)
export const getWalletConvertOps = (params?: {
  owner?: string
  status?: string
  reason_code?: string
  updated_after?: string
  limit?: number
}) =>
  fetchJson<WalletConvertOpResponse[]>(
    `/wallets/ops?${new URLSearchParams(
      Object.entries(params ?? {}).reduce<Record<string, string>>((acc, [k, v]) => {
        if (v !== undefined && v !== null) acc[k] = String(v)
        return acc
      }, {}),
    )}`,
  )
export const getWalletOpsStats = () => fetchJson<WalletOpsStatsResponse>('/wallets/ops/stats')
export const getWalletWsStatus = () => fetchJson<WalletWsStatusResponse>('/wallets/ws-status')
export const transferSol = (body: WalletTransferRequest) =>
  fetchJsonLong<WalletTransferResponse>('/wallets/transfer', {
    method: 'POST',
    body: JSON.stringify(body),
  })

export interface WalletTransferLogEntry {
  ts_utc: string
  from_wallet_id: string
  from_pubkey: string
  to_pubkey: string
  lamports: number
  signature: string
  rpc_url?: string | null
}

export interface WalletTransfersListResponse {
  transfers: WalletTransferLogEntry[]
}

export const getWalletTransfers = (limit = 20) =>
  fetchJson<WalletTransfersListResponse>(`/wallets/transfers?${new URLSearchParams({ limit: String(limit) })}`)

// Bot activity (JSONL ledger + registry; Slack digest)
export interface BotActivityJsonlResponse {
  path: string
  file_missing: boolean
  total_matching_lines: number
  rows_returned: number
  rows: Record<string, unknown>[]
}

export type BotRegistryJsonlResponse = BotActivityJsonlResponse

export interface SlackActivitySummaryResponse {
  ok: boolean
  error: string | null
  rows_included: number
  webhook_configured: boolean
}

export interface PendingOpenRecoveryResponse {
  path: string
  file_missing: boolean
  data?: Record<string, unknown> | null
}

export interface StrandedRebalanceItem {
  rebalance_session_id: string
  close_seen: boolean
  open_seen: boolean
  close_ts_utc?: string | null
  open_ts_utc?: string | null
  old_position?: string | null
  new_position?: string | null
  pool_address?: string | null
  token_mint_a?: string | null
  token_mint_b?: string | null
  token_a_label?: string | null
  token_b_label?: string | null
  rebalance_incomplete_logged: boolean
  in_pending_open_queue: boolean
  intended_tick_lower?: number | null
  intended_tick_upper?: number | null
  reason?: string | null
  can_auto_enqueue: boolean
  note?: string | null
}

export interface StrandedRebalancesResponse {
  lifecycle_path: string
  il_ledger_path?: string | null
  pending_open_path: string
  rows_scanned: number
  auto_enqueued: number
  items: StrandedRebalanceItem[]
}

function qsBotActivity(limit: number, filter?: string, offset?: number): string {
  const p = new URLSearchParams()
  p.set('limit', String(limit))
  if (typeof offset === 'number' && Number.isFinite(offset) && offset > 0) {
    p.set('offset', String(Math.floor(offset)))
  }
  if (filter && filter.trim()) p.set('filter', filter.trim())
  return p.toString()
}

export const getBotLedger = (limit = 200, filter?: string, offset?: number) =>
  fetchJson<BotActivityJsonlResponse>(`/bot-activity/ledger?${qsBotActivity(limit, filter, offset)}`)

/** IL / rebalance JSONL (`event: rebalance`); API path from `CLMM_IL_LEDGER_PATH` (same as `orca-bot-run --il-ledger-path`). */
export const getBotIlLedger = (limit = 200, filter?: string) =>
  fetchJson<BotActivityJsonlResponse>(`/bot-activity/il-ledger?${qsBotActivity(limit, filter)}`)

export const getBotRegistry = (limit = 200, filter?: string) =>
  fetchJson<BotRegistryJsonlResponse>(`/bot-activity/registry?${qsBotActivity(limit, filter)}`)

export const getPendingOpenRecovery = () =>
  fetchJson<PendingOpenRecoveryResponse>('/bot-activity/pending-open')

export const getStrandedRebalances = () =>
  fetchJson<StrandedRebalancesResponse>('/bot-activity/stranded-rebalances')

export const reconcileStrandedRebalances = () =>
  fetchJson<StrandedRebalancesResponse>('/bot-activity/stranded-rebalances/reconcile', {
    method: 'POST',
  })

export const dismissStrandedRebalance = (sessionId: string) =>
  fetchJson<StrandedRebalancesResponse>(
    `/bot-activity/stranded-rebalances/${encodeURIComponent(sessionId)}/dismiss`,
    {
      method: 'POST',
    },
  )

export const postSlackActivitySummary = (limit = 40) =>
  fetchJson<SlackActivitySummaryResponse>('/bot-activity/slack-summary', {
    method: 'POST',
    body: JSON.stringify({ limit }),
  })

// Scripts (manifest + runner)
export interface ScriptRunRecord {
  schema_version?: number
  script_id: string
  ts_utc: string
  ok: boolean
  exit_code: number
  duration_ms: number
  stdout_excerpt?: string | null
  stderr_excerpt?: string | null
  error_excerpt?: string | null
  triggered_by?: string | null
}

export interface ScriptCatalogItem {
  id: string
  path: string
  summary: string
  when_to_use?: string | null
  risk?: string | null
  runnable: boolean
  actions: string[]
  last_run?: ScriptRunRecord | null
  /** Wpis zeskanowany z dysku (brak w scripts-manifest.json) */
  auto_discovered?: boolean
}

export interface ScriptsListResponse {
  repo_root: string
  manifest_path: string
  manifest_missing: boolean
  script_runs_path: string
  script_runs_missing: boolean
  runner_configured: boolean
  scripts: ScriptCatalogItem[]
}

export const getScripts = () => fetchJson<ScriptsListResponse>('/scripts')

export const runScript = (id: string, triggeredBy?: string) =>
  fetchJson<ScriptRunRecord>(`/scripts/${encodeURIComponent(id)}/run`, {
    method: 'POST',
    body: JSON.stringify({
      triggered_by: triggeredBy ?? 'web',
    }),
  })
