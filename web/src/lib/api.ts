// API client for Bociarz LP backend

const API_BASE = '/api/v1'

export interface Position {
  address: string
  pool_address: string
  owner: string
  tick_lower: number
  tick_upper: number
  liquidity: string
  in_range: boolean
  value_usd: string
  pnl: PnL
  status: 'active' | 'closed' | 'pending'
  created_at: string | null
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
  | 'il_limit'
  | 'oor_recenter'
  | 'retouch_shift'

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
  max_il_pct?: number
  min_rebalance_interval_hours?: number
  range_width_pct?: number
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
  const statusLine = `HTTP ${status}`
  const trimmed = text.trim()
  if (!trimmed) {
    return `${statusLine} (empty body)`
  }
  try {
    const j = JSON.parse(trimmed) as Record<string, unknown>
    const m = j.message ?? j.error ?? j.detail ?? j.title
    if (typeof m === 'string' && m.length > 0) {
      return m
    }
  } catch {
    /* not JSON */
  }
  if (trimmed.startsWith('<!') || trimmed.startsWith('<html')) {
    return `${statusLine} — odpowiedź to HTML (zły URL API albo proxy Vite nie trafia na backend).`
  }
  return trimmed.length > 400 ? `${trimmed.slice(0, 400)}…` : trimmed
}

async function fetchJsonWithTimeout<T>(
  url: string,
  timeoutMs: number,
  options?: RequestInit,
): Promise<T> {
  const ctrl = new AbortController()
  const t = setTimeout(() => ctrl.abort(), timeoutMs)
  const response = await fetch(`${API_BASE}${url}`, {
    ...options,
    signal: options?.signal ?? ctrl.signal,
    headers: {
      'Content-Type': 'application/json',
      Accept: 'application/json',
      ...options?.headers,
    },
  }).finally(() => clearTimeout(t))

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
  return fetchJsonWithTimeout(url, 90_000, options)
}

// ============================================================================
// External (free) price sources
// ============================================================================

export interface JupiterPriceResponse {
  data: Record<
    string,
    {
      id: string
      price: number
    }
  >
}

export async function getJupiterPricesUsd(mints: string[]): Promise<Record<string, number>> {
  const ids = [...new Set(mints.map((m) => m.trim()).filter(Boolean))]
  if (ids.length === 0) return {}
  // Prefer server-side proxy to avoid browser CORS/adblock issues.
  try {
    const r = await fetchJson<{
      source: string
      requested: number
      returned: number
      prices: Record<string, number>
    }>(`/prices/jupiter?${new URLSearchParams({ ids: ids.join(',') })}`)
    return r.prices ?? {}
  } catch {
    // Fallback to direct Jupiter fetch (best effort).
    const qs = new URLSearchParams({ ids: ids.join(',') }).toString()
    const resp = await fetch(`https://price.jup.ag/v4/price?${qs}`)
    if (!resp.ok) {
      throw new Error(`Jupiter price HTTP ${resp.status}`)
    }
    const j = (await resp.json()) as JupiterPriceResponse
    const out: Record<string, number> = {}
    for (const [mint, row] of Object.entries(j.data ?? {})) {
      if (typeof row?.price === 'number' && Number.isFinite(row.price)) out[mint] = row.price
    }
    return out
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
  liquidity: string
  position_mint?: string | null
  position_bundle_address?: string | null
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

// Positions
export const getPositions = () => fetchJson<{ positions: Position[] }>('/positions')
export const getPosition = (address: string) => fetchJson<Position>(`/positions/${address}`)
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
export const closePosition = (address: string) => 
  fetchJsonLong<{ message: string }>(`/positions/${address}`, { method: 'DELETE' })
export const collectFees = (address: string) => 
  fetchJsonLong<{ message: string }>(`/positions/${address}/collect`, { method: 'POST' })
export const rebalancePosition = (address: string, data: { new_tick_lower: number; new_tick_upper: number }) =>
  fetchJsonLong<{ message: string }>(`/positions/${address}/rebalance`, {
    method: 'POST',
    body: JSON.stringify(data),
  })

/** `liquidity_amount`: base units as decimal string (matches API u128 as string). */
export const decreaseLiquidity = (address: string, liquidity_amount: string) =>
  fetchJsonLong<{ message: string }>(`/positions/${address}/decrease`, {
    method: 'POST',
    body: JSON.stringify({ liquidity_amount }),
  })

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
}

export interface WalletsListResponse {
  wallets_dir: string
  wallets: WalletEntry[]
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
}

export const getWallets = () => fetchJson<WalletsListResponse>('/wallets')
export const getWalletBalances = (owner: string) =>
  // This call may require multiple RPC fallbacks; allow longer than the global 15s timeout.
  fetchJsonWithTimeout<WalletBalancesResponse>(
    `/wallets/balances?${new URLSearchParams({ owner: owner.trim() })}`,
    35_000,
  )

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

function qsBotActivity(limit: number, filter?: string): string {
  const p = new URLSearchParams()
  p.set('limit', String(limit))
  if (filter && filter.trim()) p.set('filter', filter.trim())
  return p.toString()
}

export const getBotLedger = (limit = 200, filter?: string) =>
  fetchJson<BotActivityJsonlResponse>(`/bot-activity/ledger?${qsBotActivity(limit, filter)}`)

/** IL / rebalance JSONL (`event: rebalance`); API path from `CLMM_IL_LEDGER_PATH` (same as `orca-bot-run --il-ledger-path`). */
export const getBotIlLedger = (limit = 200, filter?: string) =>
  fetchJson<BotActivityJsonlResponse>(`/bot-activity/il-ledger?${qsBotActivity(limit, filter)}`)

export const getBotRegistry = (limit = 200, filter?: string) =>
  fetchJson<BotRegistryJsonlResponse>(`/bot-activity/registry?${qsBotActivity(limit, filter)}`)

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
