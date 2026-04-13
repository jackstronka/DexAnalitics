import { useEffect, useMemo, useState } from 'react'
import { useQuery, useQueries, useMutation, useQueryClient } from '@tanstack/react-query'
import { useParams, Link, useNavigate } from 'react-router-dom'
import * as Tabs from '@radix-ui/react-tabs'
import { ArrowLeft, RefreshCw, X, DollarSign } from 'lucide-react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { PoolPairLabels } from '@/components/PoolPairLabels'
import { PositionLifecycleTimeline } from '@/components/PositionLifecycleTimeline'
import {
  getPosition,
  getPositionDiagnostics,
  getPositionStreamPerformance,
  getPositionStreamPnL,
  getPositionStreamLineage,
  getBacktestJob,
  closePosition,
  collectFees,
  rebalancePosition,
  decreaseLiquidity,
  getBotLedger,
  getBotIlLedger,
  getStrategies,
  setStrategyPositionExecutor,
  getJupiterPricesUsd,
  linkPositionStrategy,
  runBacktestFromOpenPosition,
  suggestPositionStrategy,
} from '@/lib/api'
import type { Strategy } from '@/lib/api'
import {
  FEE_BASE_UNITS_TOOLTIP,
  formatDate,
  formatFeeBaseUnitsClause,
  formatLineageFeesCollectedUsdMain,
  formatPercentFixed,
  formatPrincipalDeltaUsdOrDash,
  formatUsdField,
  formatUsdFixed,
  formatUsdUncollectedFees,
  formatUsdcPriceRange,
  shortenAddress,
} from '@/lib/utils'

/** Wrapped SOL mint — network fees are in native SOL (lamports). */
const WSOL_MINT = 'So11111111111111111111111111111111111111112'

type LedgerRow = Record<string, unknown>

function mergeLifecycleLedgerRows(
  queries: { data?: { rows?: Record<string, unknown>[] } | undefined }[],
): LedgerRow[] {
  const seen = new Set<string>()
  const out: LedgerRow[] = []
  for (const q of queries) {
    for (const row of (q.data?.rows ?? []) as LedgerRow[]) {
      if (!row || typeof row !== 'object') continue
      const sig = typeof row.signature === 'string' ? row.signature : ''
      const ts = typeof row.ts_utc === 'string' ? row.ts_utc : ''
      const ev = typeof row.event === 'string' ? row.event : ''
      const key = `lc|${sig}|${ts}|${ev}`
      if (seen.has(key)) continue
      seen.add(key)
      out.push(row)
    }
  }
  return out
}

function normalizeIlTimelineRow(row: LedgerRow): LedgerRow | null {
  const ts = row.timestamp ?? row.ts_utc
  if (typeof ts !== 'string' || !ts.trim()) return null
  const evRaw = row.event
  const evBase = typeof evRaw === 'string' && evRaw.trim() ? evRaw.trim() : 'rebalance'
  const lam = row.tx_cost_lamports ?? row.tx_fee_lamports
  let txNum = 0
  if (typeof lam === 'number' && Number.isFinite(lam) && lam > 0) txNum = lam
  else if (typeof lam === 'string') {
    const n = parseFloat(lam)
    if (Number.isFinite(n) && n > 0) txNum = n
  }
  return {
    ...row,
    ts_utc: ts,
    event: evBase.startsWith('il:') ? evBase : `il:${evBase}`,
    tx_fee_lamports: txNum,
    source: 'il_ledger',
  }
}

function mergeIlLedgerRows(
  queries: { data?: { rows?: Record<string, unknown>[] } | undefined }[],
): LedgerRow[] {
  const seen = new Set<string>()
  const out: LedgerRow[] = []
  for (const q of queries) {
    for (const row of (q.data?.rows ?? []) as LedgerRow[]) {
      if (!row || typeof row !== 'object') continue
      const norm = normalizeIlTimelineRow(row)
      if (!norm) continue
      const k = `il|${norm.ts_utc}|${String(row.old_position ?? '')}|${String(row.position ?? '')}|${String(row.rebalance_session_id ?? '')}`
      if (seen.has(k)) continue
      seen.add(k)
      out.push(norm)
    }
  }
  return out
}

function groupLedgerBySession(rows: LedgerRow[]): Map<string | null, LedgerRow[]> {
  const m = new Map<string | null, LedgerRow[]>()
  for (const r of rows) {
    // Defensive: API may return non-object rows (e.g. parse failures/nulls) — never crash the page.
    if (!r || typeof r !== 'object') {
      continue
    }
    const raw = (r as Record<string, unknown>).rebalance_session_id
    const sid = typeof raw === 'string' && raw.trim() ? raw.trim() : null
    const key = sid ?? '_no_session'
    if (!m.has(key)) m.set(key, [])
    m.get(key)!.push(r)
  }
  return m
}

function parseLamports(v: unknown): number | null {
  if (typeof v === 'number' && Number.isFinite(v)) return v
  if (typeof v === 'string') {
    const n = parseFloat(v)
    return Number.isFinite(n) ? n : null
  }
  return null
}

/** ~USD for tx fee in lamports, using SOL/USD from Jupiter proxy (`solUsd` per 1 SOL). */
function lamportsToUsdDisplay(lamports: unknown, solUsd: number): string {
  if (solUsd <= 0) return '—'
  const lam = parseLamports(lamports)
  if (lam === null) return '—'
  const usd = (lam / 1e9) * solUsd
  return formatUsdFixed(usd, 3)
}

/** Lamports + USD (~) stacked — for ledger tables. */
function LamportsFeeCell({
  lamportsRaw,
  solUsd,
}: {
  lamportsRaw: unknown
  solUsd: number
}) {
  const lam = parseLamports(lamportsRaw)
  const usd = lamportsToUsdDisplay(lamportsRaw, solUsd)
  if (lam === null) return <span className="text-muted-foreground">—</span>
  return (
    <div className="leading-tight">
      <div className="font-mono tabular-nums">{lam.toLocaleString()} λ</div>
      <div className="text-[11px] text-muted-foreground">{usd}</div>
    </div>
  )
}

function usdOrDash(v: string | number, digits = 3): string {
  // We return "—" instead of $0.000 when backend is explicitly missing the metric.
  // In JSONL-only mode many fields are best-effort; showing zero looks like a real value.
  const n = typeof v === 'number' ? v : parseFloat(String(v))
  if (!Number.isFinite(n)) return '—'
  if (n === 0) return '—'
  return formatUsdFixed(n, digits)
}

function valuationQualityLabel(q?: string | null): string | null {
  const v = (q ?? '').trim().toLowerCase()
  if (!v) return null
  if (v === 'exact') return 'exact'
  if (v === 'fallback') return 'fallback'
  if (v === 'missing_inputs') return 'missing'
  return v
}

function rowEvent(r: LedgerRow): string {
  const e = r.event
  return typeof e === 'string' ? e : '—'
}

function rowSource(r: LedgerRow): string {
  const s = r.source
  return typeof s === 'string' ? s : '—'
}

function rowTs(r: LedgerRow): string {
  const t = r.ts_utc
  return typeof t === 'string' ? t : '—'
}

function rowCollectDetails(r: LedgerRow): string {
  const ev = typeof r.event === 'string' ? r.event : ''
  if (!ev.includes('collect_fees')) return '—'
  const rawA = r.lp_collected_token_a_raw
  const rawB = r.lp_collected_token_b_raw
  const asNum = (v: unknown): number | null => {
    if (typeof v === 'number' && Number.isFinite(v)) return v
    if (typeof v === 'string') {
      const n = Number(v)
      if (Number.isFinite(n)) return n
    }
    return null
  }
  const a = asNum(rawA)
  const b = asNum(rawB)
  if (a === null && b === null) return 'collect tx (brak leg values)'
  const aStr = a === null ? '—' : a.toLocaleString()
  const bStr = b === null ? '—' : b.toLocaleString()
  return `A raw: ${aStr}, B raw: ${bStr}`
}

export default function PositionDetail() {
  const { address } = useParams<{ address: string }>()
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const [actionError, setActionError] = useState<string | null>(null)
  const [actionInfo, setActionInfo] = useState<string | null>(null)
  const [backtestJobId, setBacktestJobId] = useState<string | null>(null)

  const { data: position, isLoading, isError, error } = useQuery({
    queryKey: ['position', address],
    queryFn: () => getPosition(address!),
    enabled: !!address,
    retry: 1,
  })

  const { data: diag } = useQuery({
    queryKey: ['position-diagnostics', address],
    queryFn: () => getPositionDiagnostics(address!),
    enabled: !!address,
    retry: 0,
    staleTime: 15_000,
  })

  const suggestQ = useQuery({
    queryKey: ['position-suggest-strategy', address],
    queryFn: () => suggestPositionStrategy(address!),
    enabled: !!address,
    retry: 0,
    staleTime: 30_000,
  })

  const { data: streamPerf } = useQuery({
    queryKey: ['position-stream-performance', address],
    queryFn: () => getPositionStreamPerformance(address!),
    enabled: !!address,
    retry: 0,
    staleTime: 30_000,
  })

  const { data: streamPnl } = useQuery({
    queryKey: ['position-stream-pnl', address],
    queryFn: () => getPositionStreamPnL(address!),
    enabled: !!address,
    retry: 0,
    staleTime: 30_000,
  })

  const lineageQ = useQuery({
    queryKey: ['position-stream-lineage', address],
    queryFn: () => getPositionStreamLineage(address!),
    enabled: !!address,
    retry: 0,
    staleTime: 60_000,
    refetchOnWindowFocus: false,
  })
  const streamLineage = lineageQ.data

  const chainSet = useMemo(() => {
    const addr = address?.trim() ?? ''
    const raw = (streamLineage?.chain ?? [])
      .filter((x): x is string => typeof x === 'string' && x.trim().length > 0)
      .map((x) => x.trim())
    const uniq = [...new Set(raw)]
    if (uniq.length === 0) return addr ? [addr] : []
    if (addr && !uniq.includes(addr)) return [...uniq, addr]
    return uniq
  }, [streamLineage?.chain, address])

  const ledgerQueries = useQueries({
    queries: chainSet.map((pda) => ({
      queryKey: ['bot-ledger', pda, 1500],
      queryFn: () => getBotLedger(1500, pda),
      enabled: !!pda,
      staleTime: 30_000,
    })),
  })

  const ilQueries = useQueries({
    queries: chainSet.map((pda) => ({
      queryKey: ['bot-il-ledger', pda, 800],
      queryFn: () => getBotIlLedger(800, pda),
      enabled: !!pda,
      staleTime: 30_000,
    })),
  })

  const ledgerDigest = ledgerQueries.map((q) => `${q.isFetched}:${q.data?.rows_returned ?? 0}:${q.data?.file_missing}`).join('|')
  const mergedLifecycleRows = useMemo(() => mergeLifecycleLedgerRows(ledgerQueries), [ledgerDigest])

  const ilDigest = ilQueries.map((q) => `${q.isFetched}:${q.data?.rows_returned ?? 0}`).join('|')
  const mergedIlTimelineRows = useMemo(() => mergeIlLedgerRows(ilQueries), [ilDigest])

  const timelineRows = useMemo(
    () => [...mergedLifecycleRows, ...mergedIlTimelineRows],
    [mergedLifecycleRows, mergedIlTimelineRows],
  )

  const sessionLedgerIdx = useMemo(
    () => chainSet.findIndex((p) => p === address?.trim()),
    [chainSet, address],
  )

  const ledgerData =
    sessionLedgerIdx >= 0 ? ledgerQueries[sessionLedgerIdx]?.data : ledgerQueries[0]?.data
  const ledgerRows = (ledgerData?.rows ?? []) as LedgerRow[]

  const ilLedgerData =
    sessionLedgerIdx >= 0 ? ilQueries[sessionLedgerIdx]?.data : ilQueries[0]?.data
  const ilRows = (ilLedgerData?.rows ?? []) as LedgerRow[]

  const ledgerAnyPresent = ledgerQueries.some((q) => q.data && !q.data.file_missing)
  const ilAnyPresent = ilQueries.some((q) => q.data && !q.data.file_missing)

  const { data: strategiesData } = useQuery({
    queryKey: ['strategies'],
    queryFn: getStrategies,
  })

  const { data: solPriceMap } = useQuery({
    queryKey: ['jupiter-prices', WSOL_MINT, 'position-ledger-tx-fee'],
    queryFn: () => getJupiterPricesUsd([WSOL_MINT]),
    enabled: !!address,
    staleTime: 60_000,
  })
  const solUsd = solPriceMap?.[WSOL_MINT] ?? 0

  const bySession = useMemo(() => groupLedgerBySession(ledgerRows), [ledgerRows])

  const streamTotals = streamLineage?.totals ?? streamPnl ?? null
  const streamKnownPdas =
    streamLineage?.chain?.length && streamLineage.chain.length > 0
      ? streamLineage.chain.length
      : streamPerf?.positions?.length ?? 1

  const lastRebalanceIncomplete = useMemo(() => {
    for (const r of ilRows) {
      if (typeof r?.event === 'string' && r.event === 'rebalance_incomplete') return r
    }
    return null
  }, [ilRows])

  const lastRebalanceSession = useMemo(() => {
    // Find newest session with any bot_close/bot_open events; useful even without IL ledger.
    const sessions = Array.from(bySession.entries())
    for (const [sid, rows] of sessions) {
      const hasClose = rows.some((r) => typeof r.event === 'string' && r.event.includes('close'))
      const hasOpen = rows.some((r) => typeof r.event === 'string' && r.event.includes('open'))
      const hasSwap = rows.some((r) => typeof r.event === 'string' && r.event.includes('swap'))
      if (hasClose || hasOpen || hasSwap) {
        return { session: sid, rows, hasClose, hasOpen, hasSwap }
      }
    }
    return null
  }, [bySession])

  const linkedStrategies = useMemo(() => {
    if (!address) {
      return []
    }
    const needle = address.trim()
    const list = strategiesData?.strategies ?? []
    return list.filter((s) =>
      (s.parameters.position_addresses ?? []).some((a) => {
        const x = typeof a === 'string' ? a.trim() : String(a).trim()
        return x.length > 0 && x === needle
      }),
    )
  }, [address, strategiesData?.strategies])

  const allStrategies = strategiesData?.strategies ?? []
  const [strategyPick, setStrategyPick] = useState<string>('')
  useEffect(() => {
    setStrategyPick(linkedStrategies[0]?.id ?? '')
  }, [linkedStrategies])

  useEffect(() => {
    if (linkedStrategies.length > 0) {
      return
    }
    const sid = suggestQ.data?.strategy_id
    if (typeof sid === 'string' && sid.trim().length > 0) {
      setStrategyPick(sid.trim())
    }
  }, [linkedStrategies.length, suggestQ.data?.strategy_id])

  const linkStrategyMutation = useMutation({
    mutationFn: (strategy_id: string | null) => linkPositionStrategy(address!, { strategy_id }),
    onSuccess: (data) => {
      setActionError(null)
      setActionInfo(data?.message ?? 'Strategy link updated.')
      void queryClient.invalidateQueries({ queryKey: ['strategies'] })
    },
    onError: (err) => {
      const msg = err instanceof Error ? err.message : String(err)
      setActionInfo(null)
      setActionError(`Strategy link failed: ${msg}`)
    },
  })

  function isAutomationOnForPosition(s: Strategy): boolean {
    if (!address) {
      return true
    }
    const disabled = s.parameters.executor_disabled_position_addresses ?? []
    const needle = address.trim()
    return !disabled.some((a) => {
      const x = typeof a === 'string' ? a.trim() : String(a).trim()
      return x === needle
    })
  }

  const automationMutation = useMutation({
    mutationFn: ({
      strategyId,
      enabled,
    }: {
      strategyId: string
      enabled: boolean
    }) => setStrategyPositionExecutor(strategyId, address!, enabled),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['strategies'] })
    },
  })

  const closeMutation = useMutation({
    mutationFn: () => closePosition(address!),
    onSuccess: (data) => {
      setActionError(null)
      setActionInfo(data?.message ?? 'Close requested.')
      void queryClient.invalidateQueries({ queryKey: ['position', address] })
      void queryClient.invalidateQueries({ queryKey: ['positions'] })
      void queryClient.invalidateQueries({ queryKey: ['bot-ledger', address] })
      void queryClient.invalidateQueries({ queryKey: ['bot-il-ledger', address] })
      // If it was a real close (not dry-run), go back to list.
      if (!(data?.message ?? '').toLowerCase().includes('dry-run')) {
        navigate('/positions')
      }
    },
    onError: (err) => {
      const msg = err instanceof Error ? err.message : String(err)
      setActionInfo(null)
      setActionError(`Close Position failed: ${msg}`)
    },
  })

  const collectMutation = useMutation({
    mutationFn: () => collectFees(address!),
    onSuccess: (data) => {
      setActionError(null)
      setActionInfo(data?.message ?? 'Collect requested.')
      void queryClient.invalidateQueries({ queryKey: ['position', address] })
      void queryClient.invalidateQueries({ queryKey: ['position-stream-lineage', address] })
      void queryClient.invalidateQueries({ queryKey: ['bot-ledger', address] })
      void queryClient.invalidateQueries({ queryKey: ['bot-il-ledger', address] })
    },
    onError: (err) => {
      const msg = err instanceof Error ? err.message : String(err)
      setActionInfo(null)
      setActionError(`Collect Fees failed: ${msg}`)
    },
  })

  const rebalanceMutation = useMutation({
    mutationFn: async () => {
      const lo = window.prompt('New tick lower')
      const hi = window.prompt('New tick upper')
      if (lo === null || hi === null) throw new Error('Cancelled')
      const lower = parseInt(lo, 10)
      const upper = parseInt(hi, 10)
      if (Number.isNaN(lower) || Number.isNaN(upper)) throw new Error('Invalid ticks')
      return rebalancePosition(address!, {
        new_tick_lower: lower,
        new_tick_upper: upper,
      })
    },
    onSuccess: () => {
      setActionError(null)
      setActionInfo('Rebalance requested.')
      void queryClient.invalidateQueries({ queryKey: ['position', address] })
      void queryClient.invalidateQueries({ queryKey: ['bot-ledger', address] })
      void queryClient.invalidateQueries({ queryKey: ['bot-il-ledger', address] })
    },
    onError: (err) => {
      const msg = err instanceof Error ? err.message : String(err)
      if (msg === 'Cancelled') return
      setActionInfo(null)
      setActionError(`Rebalance failed: ${msg}`)
    },
  })

  const decreaseMutation = useMutation({
    mutationFn: async () => {
      const raw = window.prompt('Liquidity amount to remove (base units, decimal string)')
      if (raw === null) throw new Error('Cancelled')
      const trimmed = raw.trim()
      if (!/^\d+$/.test(trimmed)) throw new Error('Must be a non-negative integer string')
      return decreaseLiquidity(address!, trimmed)
    },
    onSuccess: () => {
      setActionError(null)
      setActionInfo('Decrease requested.')
      void queryClient.invalidateQueries({ queryKey: ['position', address] })
      void queryClient.invalidateQueries({ queryKey: ['positions'] })
      void queryClient.invalidateQueries({ queryKey: ['bot-ledger', address] })
      void queryClient.invalidateQueries({ queryKey: ['bot-il-ledger', address] })
    },
    onError: (err) => {
      const msg = err instanceof Error ? err.message : String(err)
      if (msg === 'Cancelled') return
      setActionInfo(null)
      setActionError(`Decrease liquidity failed: ${msg}`)
    },
  })

  const runBacktestM = useMutation({
    mutationFn: async () => {
      const raw = streamLineage?.nodes.find((n) => n.position_address === address)?.baseline_value_usd
      let capital: number | undefined
      if (raw != null && String(raw).trim() !== '') {
        const n = parseFloat(String(raw))
        if (Number.isFinite(n) && n > 0) capital = n
      }
      return await runBacktestFromOpenPosition({
        position_address: address!,
        strategy: 'static',
        fee_source: 'snapshots',
        price_path_source: 'snapshots',
        snapshot_protocol: 'orca',
        ...(capital != null ? { capital } : {}),
      })
    },
    onSuccess: (r) => setBacktestJobId(r.id),
  })

  const backtestJobQ = useQuery({
    queryKey: ['backtest-job-open', backtestJobId],
    queryFn: () => getBacktestJob(backtestJobId!),
    enabled: !!backtestJobId,
    refetchInterval: backtestJobId ? 2000 : false,
    staleTime: 0,
    retry: 0,
  })

  if (isLoading) {
    return <div className="text-center py-8">Loading...</div>
  }

  if (isError) {
    const msg = error instanceof Error ? error.message : String(error)
    const looksLikeRpcOrUpstream =
      /\b(BAD_GATEWAY|Bad gateway|502|503|504|408|HTTP 408|RPC|timed out|timeout|ECONNREFUSED|fetch failed|Solana RPC error)\b/i.test(
        msg,
      )
    const looksLikeNotFoundOnChain =
      /\b(404|NOT_FOUND|Not found:|account not found|On-chain position account not found)\b/i.test(msg)
    return (
      <div className="text-center py-8 space-y-3 max-w-lg mx-auto px-4">
        <p className="text-destructive font-medium">Nie udało się pobrać pozycji z API</p>
        <p className="text-sm text-muted-foreground break-words font-mono">{msg}</p>
        {looksLikeRpcOrUpstream ? (
          <p className="text-xs text-muted-foreground text-left">
            To wygląda na problem <strong className="text-foreground">RPC / sieci / limitu</strong>, a nie na pewny brak
            pozycji. Sprawdź <code className="text-[11px]">RPC_URL</code> po stronie API, spróbuj ponownie za chwilę albo
            inny endpoint. HTTP <strong className="text-foreground">502 Bad gateway</strong> z tego endpointu oznacza w
            praktyce „błąd po drodze do Solany”, nie „zamknięta pozycja”.
          </p>
        ) : null}
        {looksLikeNotFoundOnChain && !looksLikeRpcOrUpstream ? (
          <p className="text-xs text-muted-foreground text-left">
            Komunikat wskazuje, że dla <strong className="text-foreground">tego klastra RPC</strong> konto pozycji nie
            istnieje (np. pozycja zamknięta, zły adres, mainnet vs devnet) albo wklejono adres{' '}
            <strong className="text-foreground">poolu</strong> zamiast PDA pozycji.
          </p>
        ) : null}
        <p className="text-xs text-muted-foreground">
          Przy HTTP 502 z proxy / pustej odpowiedzi: backend nie działa albo Vite{' '}
          <code className="text-[11px]">API_UPSTREAM</code> nie trafia w port API — to{' '}
          <strong className="text-foreground">nie</strong> znaczy automatycznie, że pozycji nie ma on-chain.
        </p>
        <Link to="/positions">
          <Button variant="outline" size="sm">
            Wróć do listy
          </Button>
        </Link>
      </div>
    )
  }

  if (!position) {
    return <div className="text-center py-8">Position not found</div>
  }

  const rangeUsdcLine = formatUsdcPriceRange(
    position.range_lower_usdc ?? undefined,
    position.range_upper_usdc ?? undefined,
    position.range_usdc_quote ?? undefined,
  )

  return (
    <div className="space-y-6">
      <div className="flex items-start gap-4 min-w-0">
        <Link to="/positions">
          <Button variant="ghost" size="icon">
            <ArrowLeft className="h-4 w-4" />
          </Button>
        </Link>
        <div className="min-w-0 space-y-1 flex-1">
          <h1 className="text-3xl font-bold">Position Details</h1>
          {position.token_a_label && position.token_b_label ? (
            <div className="text-lg font-semibold">
              {position.token_a_label} / {position.token_b_label}
            </div>
          ) : null}
          <div className="max-w-xl">
            <PoolPairLabels
              labelA={position.token_a_label}
              labelB={position.token_b_label}
              mintA={position.token_mint_a}
              mintB={position.token_mint_b}
              priceA={position.token_price_a_usd}
              priceB={position.token_price_b_usd}
            />
          </div>
          <p className="text-xs text-muted-foreground font-mono break-all pt-0.5" title={position.address}>
            PDA {position.address}
          </p>
        </div>
      </div>

      <Tabs.Root defaultValue="overview">
        <Tabs.List className="flex gap-2 border-b border-border pb-2">
          <Tabs.Trigger
            value="overview"
            className="px-3 py-1.5 text-sm rounded-md data-[state=active]:bg-primary data-[state=active]:text-primary-foreground"
          >
            Overview
          </Tabs.Trigger>
          <Tabs.Trigger
            value="ledger"
            className="px-3 py-1.5 text-sm rounded-md data-[state=active]:bg-primary data-[state=active]:text-primary-foreground"
          >
            Logs / rebalances
          </Tabs.Trigger>
        </Tabs.List>

        <Tabs.Content value="overview" className="mt-4 space-y-6">
          {(lastRebalanceIncomplete || lastRebalanceSession) && (
            <Card>
              <CardHeader>
                <CardTitle>Last rebalance diagnostics</CardTitle>
              </CardHeader>
              <CardContent className="text-sm space-y-2">
                {lastRebalanceIncomplete ? (
                  <div className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2">
                    <div className="font-medium text-destructive">
                      Rebalance incomplete — old position closed, new one not opened
                    </div>
                    <div className="text-xs text-muted-foreground mt-1">
                      {typeof lastRebalanceIncomplete.ts_utc === 'string' ? lastRebalanceIncomplete.ts_utc : '—'}
                      {typeof lastRebalanceIncomplete.rebalance_session_id === 'string'
                        ? ` · session ${lastRebalanceIncomplete.rebalance_session_id}`
                        : ''}
                    </div>
                    {typeof lastRebalanceIncomplete.error === 'string' && lastRebalanceIncomplete.error.trim() ? (
                      <div className="text-xs mt-2 break-words">
                        <span className="font-medium">error:</span> {lastRebalanceIncomplete.error}
                      </div>
                    ) : null}
                    {typeof lastRebalanceIncomplete.hint === 'string' && lastRebalanceIncomplete.hint.trim() ? (
                      <div className="text-xs mt-1 break-words">
                        <span className="font-medium">hint:</span> {lastRebalanceIncomplete.hint}
                      </div>
                    ) : null}
                  </div>
                ) : null}

                {!lastRebalanceIncomplete && lastRebalanceSession ? (
                  <div className="rounded-md border border-border bg-muted/10 px-3 py-2">
                    <div className="font-medium">Latest tx session (from lifecycle ledger)</div>
                    <div className="text-xs text-muted-foreground mt-1">
                      session:{' '}
                      <span className="font-mono">
                        {lastRebalanceSession.session === '_no_session'
                          ? '(no rebalance_session_id)'
                          : String(lastRebalanceSession.session)}
                      </span>
                      {lastRebalanceSession.hasClose && !lastRebalanceSession.hasOpen
                        ? ' · close without open (likely incomplete)'
                        : ''}
                    </div>
                    <div className="text-xs text-muted-foreground mt-1">
                      Open the <strong>Logs / rebalances</strong> tab to see raw rows.
                    </div>
                  </div>
                ) : null}

                {(!ledgerAnyPresent || !ilAnyPresent) && (
                  <div className="text-xs text-yellow-500">
                    Bot logs are file-backed on the API host. If IL ledger is missing, set{' '}
                    <code className="text-[11px]">CLMM_IL_LEDGER_PATH</code> (or run CLI with{' '}
                    <code className="text-[11px]">--il-ledger-path</code>) and restart API.
                  </div>
                )}
              </CardContent>
            </Card>
          )}

          <div className="grid gap-6 md:grid-cols-2">
            <Card>
              <CardHeader>
                <CardTitle>Position Info</CardTitle>
              </CardHeader>
              <CardContent className="space-y-4">
                <div className="flex flex-col gap-1 sm:flex-row sm:justify-between sm:items-start border-b border-border/50 pb-3">
                  <span className="text-muted-foreground shrink-0">Token pair</span>
                  <div className="text-right max-w-md">
                    <PoolPairLabels
                      labelA={position.token_a_label}
                      labelB={position.token_b_label}
                      mintA={position.token_mint_a}
                      mintB={position.token_mint_b}
                      priceA={position.token_price_a_usd}
                      priceB={position.token_price_b_usd}
                    />
                  </div>
                </div>
                <div className="flex justify-between gap-2">
                  <span className="text-muted-foreground">Whirlpool</span>
                  <span className="font-mono text-xs text-right break-all max-w-[16rem]" title={position.pool_address}>
                    {shortenAddress(position.pool_address, 6)}
                  </span>
                </div>
                <div className="flex flex-col gap-3 sm:flex-row sm:justify-between sm:items-start">
                  <span className="text-muted-foreground shrink-0 pt-0.5">Strategy</span>
                  <div className="flex flex-col items-stretch sm:items-end gap-3 w-full sm:max-w-md">
                    <div className="text-right sm:text-right w-full">
                      {linkedStrategies.length === 0 ? (
                        <span className="text-muted-foreground text-sm">None linked</span>
                      ) : (
                        <ul className="space-y-1">
                          {linkedStrategies.map((s) => (
                            <li key={s.id}>
                              <Link
                                to={`/strategies/${s.id}`}
                                className="font-medium text-primary hover:underline"
                              >
                                {s.name}
                              </Link>
                              <span className="text-xs text-muted-foreground ml-1">({s.strategy_type})</span>
                            </li>
                          ))}
                        </ul>
                      )}
                    </div>
                    <div className="flex flex-col gap-2 w-full border-t border-border/60 pt-3">
                      <p className="text-xs text-muted-foreground text-left sm:text-right">
                        Link, switch, or remove strategy for this position (updates{' '}
                        <code className="text-[10px]">parameters.position_addresses</code>).
                      </p>
                      {linkedStrategies.length === 0 && suggestQ.data?.reason ? (
                        <p className="text-[11px] text-muted-foreground text-left sm:text-right">
                          Suggestion: {suggestQ.data.reason}
                        </p>
                      ) : null}
                      <div className="flex flex-col sm:flex-row sm:flex-wrap gap-2 items-stretch sm:items-center">
                        <select
                          className="rounded-md border border-input bg-background px-2 py-2 text-sm min-w-0 flex-1 sm:max-w-xs"
                          value={strategyPick}
                          onChange={(e) => setStrategyPick(e.target.value)}
                          disabled={linkStrategyMutation.isPending || allStrategies.length === 0}
                        >
                          <option value="">— None (unlink) —</option>
                          {allStrategies.map((s) => (
                            <option key={s.id} value={s.id}>
                              {s.name} ({s.strategy_type.replace(/_/g, ' ')})
                            </option>
                          ))}
                        </select>
                        <Button
                          type="button"
                          size="sm"
                          disabled={linkStrategyMutation.isPending || !address}
                          onClick={() =>
                            linkStrategyMutation.mutate(strategyPick.trim() ? strategyPick.trim() : null)
                          }
                        >
                          {linkStrategyMutation.isPending ? 'Saving…' : 'Apply'}
                        </Button>
                        <Button
                          type="button"
                          size="sm"
                          variant="outline"
                          disabled={linkStrategyMutation.isPending || !address}
                          onClick={() => {
                            setStrategyPick('')
                            linkStrategyMutation.mutate(null)
                          }}
                        >
                          Remove link
                        </Button>
                      </div>
                      {allStrategies.length === 0 && (
                        <p className="text-xs text-amber-600 text-left sm:text-right">
                          No strategies yet — create one under Strategies first.
                        </p>
                      )}
                    </div>
                  </div>
                </div>
                <div className="flex justify-between gap-4">
                  <span className="text-muted-foreground shrink-0">Range</span>
                  <span className="text-right">
                    {rangeUsdcLine ? (
                      <>
                        <span className="block">{rangeUsdcLine}</span>
                        <span className="block text-xs text-muted-foreground mt-0.5">
                          ticks {position.tick_lower} → {position.tick_upper}
                        </span>
                      </>
                    ) : (
                      <span>
                        {position.tick_lower} → {position.tick_upper}{' '}
                        <span className="text-xs text-muted-foreground">(ticks)</span>
                      </span>
                    )}
                  </span>
                </div>
                <div className="flex justify-between">
                  <span className="text-muted-foreground">Liquidity</span>
                  <span>{position.liquidity}</span>
                </div>
                <div className="flex justify-between">
                  <span className="text-muted-foreground">In Range</span>
                  <span className={position.in_range ? 'text-green-500' : 'text-yellow-500'}>
                    {position.in_range ? 'Yes' : 'No'}
                  </span>
                </div>
                {position.created_at && (
                  <div className="flex justify-between">
                    <span className="text-muted-foreground">Created</span>
                    <span>{formatDate(position.created_at)}</span>
                  </div>
                )}
              </CardContent>
            </Card>

            <Card>
              <CardHeader>
                <CardTitle>Diagnostics</CardTitle>
              </CardHeader>
              <CardContent className="space-y-3 text-sm">
                <div className="flex justify-between">
                  <span className="text-muted-foreground">In monitor</span>
                  <span>{diag?.in_monitor ? 'Yes' : 'No'}</span>
                </div>
                <div className="flex justify-between">
                  <span className="text-muted-foreground">Monitor in range</span>
                  <span>{diag?.monitor_in_range === undefined ? '—' : diag?.monitor_in_range ? 'Yes' : 'No'}</span>
                </div>

                <div className="border-t border-border/60 pt-3 space-y-2">
                  <div className="text-muted-foreground text-xs">Linked strategies</div>
                  {diag?.linked_strategies?.length ? (
                    <ul className="space-y-2">
                      {diag.linked_strategies.map((s) => (
                        <li key={s.strategy_id} className="rounded-md border border-border/60 p-2">
                          <div className="flex flex-wrap items-center justify-between gap-2">
                            <div className="font-medium">
                              <Link to={`/strategies/${s.strategy_id}`} className="text-primary hover:underline">
                                {s.name}
                              </Link>{' '}
                              <span className="text-xs text-muted-foreground">({s.strategy_type})</span>
                            </div>
                            <div className="text-xs text-muted-foreground">
                              running: {s.running ? 'yes' : 'no'} · auto_execute: {s.auto_execute ? 'yes' : 'no'} · dry_run:{' '}
                              {s.dry_run ? 'yes' : 'no'}
                            </div>
                          </div>
                          <div className="text-xs text-muted-foreground mt-1">
                            disabled for this position: {s.automation_disabled_for_position ? 'yes' : 'no'}
                          </div>
                          {s.last_eval ? (
                            <div className="mt-2 text-xs font-mono text-muted-foreground space-y-1">
                              <div>last_eval: {s.last_eval.ts_utc}</div>
                              <div>
                                tick_current: {s.last_eval.pool_tick_current} · in_range: {s.last_eval.in_range ? 'yes' : 'no'} · hours_since_rebalance:{' '}
                                {s.last_eval.hours_since_rebalance ?? '—'}
                              </div>
                              <div>
                                decision: {s.last_eval.decision} · requires_tx: {s.last_eval.requires_transaction ? 'yes' : 'no'} · auto_execute:{' '}
                                {s.last_eval.auto_execute ? 'yes' : 'no'}
                              </div>
                            </div>
                          ) : (
                            <div className="mt-2 text-xs text-muted-foreground">
                              No executor evaluation snapshot yet (strategy not running, or no tick cycle since page load).
                            </div>
                          )}
                        </li>
                      ))}
                    </ul>
                  ) : (
                    <div className="text-muted-foreground">No linked strategies.</div>
                  )}
                </div>
              </CardContent>
            </Card>

            <Card>
              <CardHeader>
                <CardTitle>Performance</CardTitle>
              </CardHeader>
              <CardContent className="space-y-4">
                <div className="flex justify-between">
                  <span className="text-muted-foreground">Value</span>
                  <span className="font-bold">{formatUsdFixed(position.value_usd, 3)}</span>
                </div>
                <div className="flex justify-between">
                  <span className="text-muted-foreground">Net PnL</span>
                  <span
                    className={
                      parseFloat(position.pnl.net_pnl_pct) >= 0 ? 'text-green-500' : 'text-red-500'
                    }
                  >
                    {formatUsdFixed(position.pnl.net_pnl_usd, 3)} (
                    {formatPercentFixed(position.pnl.net_pnl_pct, 3)})
                  </span>
                </div>
                <div className="flex justify-between">
                  <span className="text-muted-foreground">Uncollected fees (USD)</span>
                  <span className="text-green-500">
                    {formatUsdUncollectedFees(position.pnl.fees_earned_usd)}
                  </span>
                </div>
                <p className="text-[11px] text-muted-foreground leading-snug">
                  On-chain <code className="text-[10px]">fee_owed</code> raw:{' '}
                  {position.token_a_label ? `${position.token_a_label} (A)` : 'token A'}{' '}
                  <span className="font-mono tabular-nums">{position.pnl.fees_earned_a}</span> ·{' '}
                  {position.token_b_label ? `${position.token_b_label} (B)` : 'token B'}{' '}
                  <span className="font-mono tabular-nums">{position.pnl.fees_earned_b}</span> (smallest
                  units). If both are 0, nothing has accrued in the position account yet. If non-zero but
                  USD stays $0, the price service did not return a USD rate for a pool mint.
                </p>
                <div className="flex justify-between">
                  <span className="text-muted-foreground">Impermanent Loss</span>
                  <span className="text-yellow-500">{formatPercentFixed(position.pnl.il_pct, 3)}</span>
                </div>
                {(streamTotals || streamPerf || streamLineage) ? (
                  <div className="rounded-md border border-border/60 bg-muted/10 px-3 py-2 space-y-2">
                    <div className="text-xs text-muted-foreground">Stream summary (across rotated PDAs)</div>
                    <div className="flex justify-between text-sm">
                      <span className="text-muted-foreground">Known PDAs</span>
                      <span className="font-mono tabular-nums">{streamKnownPdas}</span>
                    </div>
                    <div className="flex justify-between text-sm gap-4">
                      <span className="text-muted-foreground shrink-0">Tx fees (network)</span>
                      <div className="text-right font-mono tabular-nums text-xs leading-tight">
                        {(() => {
                          const lam =
                            streamLineage?.chain_cost_summary?.tx_fee_lamports_total ??
                            streamPerf?.total_tx_fee_lamports
                          const usdStr = streamTotals
                            ? String(streamTotals.tx_fees_usd)
                            : streamPerf
                              ? streamPerf.total_tx_fee_usd
                              : null
                          if (!usdStr && lam == null) return '—'
                          return (
                            <>
                              {lam != null && lam > 0 ? (
                                <div>{Number(lam).toLocaleString()} λ</div>
                              ) : null}
                              <div className="text-muted-foreground">
                                {usdStr ? formatUsdFixed(parseFloat(usdStr), 4) : '—'}
                              </div>
                            </>
                          )
                        })()}
                      </div>
                    </div>
                    {streamTotals ? (
                      <div className="border-t border-border/60 pt-2 space-y-1">
                        <div className="flex justify-between text-sm">
                          <span className="text-muted-foreground">Baseline → current</span>
                          <span className="font-mono tabular-nums">
                            {usdOrDash(streamTotals.baseline_value_usd)} → {usdOrDash(streamTotals.current_value_usd)}
                          </span>
                        </div>
                        <div className="flex justify-between text-sm">
                          <span className="text-muted-foreground">Realized cashflow</span>
                          <span className="font-mono tabular-nums">{formatUsdFixed(streamTotals.realized_cashflow_usd, 3)}</span>
                        </div>
                        <div className="flex justify-between text-sm">
                          <span className="text-muted-foreground">Stream Net PnL</span>
                          <span
                            className={
                              parseFloat(streamTotals.net_pnl_pct) >= 0
                                ? 'text-green-500 font-mono'
                                : 'text-red-500 font-mono'
                            }
                          >
                            {formatUsdFixed(streamTotals.net_pnl_usd, 3)} ({formatPercentFixed(streamTotals.net_pnl_pct, 3)})
                          </span>
                        </div>
                        {streamTotals.note ? (
                          <div className="text-[11px] text-muted-foreground leading-snug">{streamTotals.note}</div>
                        ) : null}
                      </div>
                    ) : null}
                    {streamLineage?.note ? (
                      <div className="text-[11px] text-muted-foreground leading-snug">{streamLineage.note}</div>
                    ) : null}
                    {!streamTotals && streamPerf?.note ? (
                      <div className="text-[11px] text-muted-foreground leading-snug">{streamPerf.note}</div>
                    ) : null}
                  </div>
                ) : null}
                <p className="text-xs text-muted-foreground border-t border-border/60 pt-3 leading-relaxed">
                  <span className="font-medium text-foreground/90">Why zeros?</span> Net PnL and IL% come from
                  the API process monitor (entry baseline vs current mark). Uncollected fees (USD) are an
                  estimate: on-chain <code className="text-[10px]">fees_owed</code> × token USD prices;
                  sub-cent amounts use 6 decimal places so they are not rounded to $0.000. Values refresh
                  from RPC on each position load. Compare the raw line above with Orca if in doubt.
                </p>
              </CardContent>
            </Card>
          </div>

          {linkedStrategies.length > 0 && (
            <Card>
              <CardHeader>
                <CardTitle>Strategy automation (this position)</CardTitle>
              </CardHeader>
              <CardContent className="space-y-4">
                <p className="text-sm text-muted-foreground">
                  This position is linked to {linkedStrategies.length === 1 ? 'a strategy' : 'strategies'}.
                  Turn automation off to stop this executor from acting on this PDA only (other linked
                  positions are unchanged).
                </p>
                <ul className="space-y-3">
                  {linkedStrategies.map((s) => (
                    <li
                      key={s.id}
                      className="flex flex-col gap-2 rounded-md border border-border p-3 sm:flex-row sm:items-center sm:justify-between"
                    >
                      <div>
                        <Link
                          to={`/strategies/${s.id}`}
                          className="font-medium text-primary hover:underline"
                        >
                          {s.name}
                        </Link>
                        <div className="text-xs text-muted-foreground">
                          Strategy {s.running ? 'running' : 'stopped'} · this position:{' '}
                          {isAutomationOnForPosition(s) ? (
                            <span className="text-foreground">automation on</span>
                          ) : (
                            <span className="text-amber-600">automation paused</span>
                          )}
                        </div>
                      </div>
                      <Button
                        type="button"
                        size="sm"
                        variant={isAutomationOnForPosition(s) ? 'outline' : 'default'}
                        disabled={automationMutation.isPending}
                        onClick={() =>
                          automationMutation.mutate({
                            strategyId: s.id,
                            enabled: !isAutomationOnForPosition(s),
                          })
                        }
                      >
                        {isAutomationOnForPosition(s)
                          ? 'Pause automation for this position'
                          : 'Resume automation for this position'}
                      </Button>
                    </li>
                  ))}
                </ul>
              </CardContent>
            </Card>
          )}

          <Card>
            <CardHeader>
              <CardTitle>Actions</CardTitle>
            </CardHeader>
            <CardContent className="space-y-3">
              {actionError ? (
                <div className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive break-words">
                  {actionError}
                </div>
              ) : null}
              {actionInfo ? (
                <div className="rounded-md border border-emerald-600/40 bg-emerald-950/20 px-3 py-2 text-sm text-emerald-200 break-words">
                  {actionInfo}
                </div>
              ) : null}
              <div className="flex flex-wrap gap-4">
              <Button onClick={() => collectMutation.mutate()} disabled={collectMutation.isPending}>
                <DollarSign className="h-4 w-4 mr-2" />
                {collectMutation.isPending ? 'Collecting...' : 'Collect Fees'}
              </Button>
              <Button
                variant="outline"
                onClick={() => rebalanceMutation.mutate()}
                disabled={rebalanceMutation.isPending}
              >
                <RefreshCw className="h-4 w-4 mr-2" />
                {rebalanceMutation.isPending ? 'Rebalancing...' : 'Rebalance'}
              </Button>
              <Button
                variant="outline"
                onClick={() => decreaseMutation.mutate()}
                disabled={decreaseMutation.isPending}
              >
                {decreaseMutation.isPending ? 'Decreasing...' : 'Decrease liquidity'}
              </Button>
              <Button
                variant="destructive"
                onClick={() => {
                  if (
                    !window.confirm(
                      'Zamknąć tę pozycję? Operacji nie cofniesz z poziomu tego panelu (on-chain tx).',
                    )
                  ) {
                    return
                  }
                  closeMutation.mutate()
                }}
                disabled={closeMutation.isPending}
              >
                <X className="h-4 w-4 mr-2" />
                {closeMutation.isPending ? 'Closing...' : 'Close Position'}
              </Button>
              </div>
            </CardContent>
          </Card>

          <Card>
            <CardHeader className="flex flex-row items-center justify-between gap-4">
              <div>
                <CardTitle>Backtest (from open position)</CardTitle>
                <p className="text-sm text-muted-foreground font-normal">
                  Uruchamia <code className="text-[11px]">clmm-lp-cli backtest</code> dla aktualnie otwartej pozycji
                  (start z registry_open). Jeśli baseline dla tego PDA jest dostępny, API użyje go jako{' '}
                  <code className="text-[11px]">capital</code>.
                </p>
              </div>
              <Button
                size="sm"
                onClick={() => runBacktestM.mutate()}
                disabled={runBacktestM.isPending || !address}
              >
                Run backtest
              </Button>
            </CardHeader>
            <CardContent className="space-y-3">
              {runBacktestM.isError && (runBacktestM.error as Error)?.message !== 'Cancelled' ? (
                <div className="text-sm text-destructive">
                  {(runBacktestM.error as Error)?.message ?? 'Backtest failed'}
                </div>
              ) : null}
              {backtestJobId ? (
                <div className="text-sm text-muted-foreground">
                  Job: <span className="font-mono">{backtestJobId}</span>{' '}
                  {backtestJobQ.data ? `(${backtestJobQ.data.status})` : ''}
                </div>
              ) : (
                <div className="text-sm text-muted-foreground">No job yet.</div>
              )}
              {backtestJobQ.data?.stderr ? (
                <pre className="text-xs whitespace-pre-wrap bg-muted p-3 rounded-md overflow-auto max-h-64">
{backtestJobQ.data.stderr}
                </pre>
              ) : null}
              {backtestJobQ.data?.stdout ? (
                <pre className="text-xs whitespace-pre-wrap bg-muted p-3 rounded-md overflow-auto max-h-64">
{backtestJobQ.data.stdout}
                </pre>
              ) : null}
            </CardContent>
          </Card>
        </Tabs.Content>

        <Tabs.Content value="ledger" className="mt-4 space-y-4">
          {lineageQ.isPending ? (
            <Card>
              <CardHeader>
                <CardTitle>Position history (rotations)</CardTitle>
              </CardHeader>
              <CardContent className="text-sm text-muted-foreground">Loading lineage…</CardContent>
            </Card>
          ) : lineageQ.isError ? (
            <Card>
              <CardHeader>
                <CardTitle>Position history (rotations)</CardTitle>
              </CardHeader>
              <CardContent className="text-sm text-destructive">
                {lineageQ.error instanceof Error ? lineageQ.error.message : String(lineageQ.error)}
              </CardContent>
            </Card>
          ) : streamLineage ? (
            <Card>
              <CardHeader>
                <CardTitle>Position history (rotations)</CardTitle>
              </CardHeader>
              <CardContent className="space-y-3">
                <p className="text-sm text-muted-foreground">
                  Best-effort chain reconstructed from IL edges (old → new). Each row is a distinct PDA created by rebalance/strategy rotation.
                </p>
                {streamLineage.note ? (
                  <p className="text-[11px] text-muted-foreground leading-snug">{streamLineage.note}</p>
                ) : null}
                <p className="text-[11px] text-muted-foreground leading-snug">
                  <span className="font-medium">LP zebrane:</span> przy <code className="text-[10px]">collect_fees</code> Orca
                  przenosi oba tokeny puli, ale <code className="text-[10px]">fee_payer_token_deltas</code> z meta RPC często
                  zawiera tylko jeden mint SPL (np. whETH) — WSOL bywa pominięty. W nawiasie pokazujemy{' '}
                  <span title={FEE_BASE_UNITS_TOOLTIP} className="cursor-help border-b border-dotted border-muted-foreground/40">
                    baz. jedn.
                  </span>{' '}
                  = najmniejsze jednostki on-chain (np. lamporty), nie osobny token „raw”.
                </p>

                {streamLineage.nodes.length > 0 ? (
                  <div className="overflow-x-auto rounded-md border">
                    <table className="w-full text-xs">
                      <thead className="bg-muted/50">
                        <tr>
                          <th className="px-2 py-1 text-left">#</th>
                          <th className="px-2 py-1 text-left">position</th>
                          <th className="px-2 py-1 text-left">opened</th>
                          <th className="px-2 py-1 text-left">closed / last</th>
                          <th className="px-2 py-1 text-left">start value</th>
                          <th className="px-2 py-1 text-left">end value</th>
                          <th className="px-2 py-1 text-left">principal Δ</th>
                          <th className="px-2 py-1 text-left">Sieć (tx)</th>
                          <th className="px-2 py-1 text-left">LP zebrane</th>
                          <th className="px-2 py-1 text-left">cashflow</th>
                          <th className="px-2 py-1 text-left">net PnL</th>
                        </tr>
                      </thead>
                      <tbody>
                        {streamLineage.nodes.map((n, i) => (
                          <tr key={n.position_address} className="border-t border-border/60">
                            <td className="px-2 py-1 font-mono tabular-nums">{i + 1}</td>
                            <td className="px-2 py-1 font-mono whitespace-nowrap">
                              <Link
                                to={
                                  n.closed_ts_utc
                                    ? `/positions/closed/${n.position_address}`
                                    : `/positions/${n.position_address}`
                                }
                                className="text-primary hover:underline"
                                title={n.position_address}
                              >
                                {shortenAddress(n.position_address, 8)}
                              </Link>
                            </td>
                            <td className="px-2 py-1 whitespace-nowrap">
                              {n.opened_ts_utc ? formatDate(n.opened_ts_utc) : '—'}
                            </td>
                            <td className="px-2 py-1 whitespace-nowrap">
                              {n.closed_ts_utc ? formatDate(n.closed_ts_utc) : '—'}
                            </td>
                            <td className="px-2 py-1 whitespace-nowrap font-mono">
                              {usdOrDash(n.baseline_value_usd, 3)}
                              {valuationQualityLabel(n.baseline_valuation_quality) ? (
                                <span className="ml-1 rounded border border-border/60 px-1 py-0 text-[10px] text-muted-foreground">
                                  {valuationQualityLabel(n.baseline_valuation_quality)}
                                </span>
                              ) : null}
                            </td>
                            <td className="px-2 py-1 whitespace-nowrap font-mono">
                              {usdOrDash(n.current_value_usd, 3)}
                              {valuationQualityLabel(n.current_valuation_quality) ? (
                                <span className="ml-1 rounded border border-border/60 px-1 py-0 text-[10px] text-muted-foreground">
                                  {valuationQualityLabel(n.current_valuation_quality)}
                                </span>
                              ) : null}
                            </td>
                            <td className="px-2 py-1 whitespace-nowrap font-mono">
                              {formatPrincipalDeltaUsdOrDash(n.baseline_value_usd, n.current_value_usd, 3)}
                            </td>
                            <td className="px-2 py-1 whitespace-nowrap font-mono text-[11px] leading-tight">
                              {(n.tx_fee_lamports ?? 0).toLocaleString()} λ
                              <br />
                              <span className="text-muted-foreground">{formatUsdFixed(parseFloat(String(n.tx_fees_usd)), 4)}</span>
                            </td>
                            <td className="px-2 py-1 whitespace-nowrap font-mono text-[11px]">
                              {(() => {
                                const collects = n.collect_events ?? 0
                                const usdNum = parseFloat(String(n.fees_collected_usd ?? '').trim() || '0')
                                const hasTokenVals =
                                  n.fees_collected_token_a_ui != null ||
                                  n.fees_collected_token_b_ui != null ||
                                  n.fees_collected_token_a_raw != null ||
                                  n.fees_collected_token_b_raw != null
                                const showLegRows =
                                  collects > 0 && (hasTokenVals || n.token_a_label || n.token_b_label)
                                return (
                                  <>
                                    <span>{formatLineageFeesCollectedUsdMain(n.fees_collected_usd, collects)}</span>
                                    <span className="text-muted-foreground"> · {collects}×</span>
                                    {showLegRows ? (
                                      <div className="text-muted-foreground mt-1 leading-tight">
                                        {n.token_a_label ? (
                                          <div>
                                            {n.token_a_label}: {String(n.fees_collected_token_a_ui ?? '—')}
                                            {formatFeeBaseUnitsClause(n.fees_collected_token_a_raw) ? (
                                              <span title={FEE_BASE_UNITS_TOOLTIP}>
                                                {' '}
                                                {formatFeeBaseUnitsClause(n.fees_collected_token_a_raw)}
                                              </span>
                                            ) : null}
                                          </div>
                                        ) : null}
                                        {n.token_b_label ? (
                                          <div>
                                            {n.token_b_label}: {String(n.fees_collected_token_b_ui ?? '—')}
                                            {formatFeeBaseUnitsClause(n.fees_collected_token_b_raw) ? (
                                              <span title={FEE_BASE_UNITS_TOOLTIP}>
                                                {' '}
                                                {formatFeeBaseUnitsClause(n.fees_collected_token_b_raw)}
                                              </span>
                                            ) : null}
                                          </div>
                                        ) : null}
                                      </div>
                                    ) : null}
                                    {collects > 0 && usdNum === 0 && !hasTokenVals ? (
                                      <div className="text-muted-foreground mt-1 leading-tight text-[10px]">
                                        Brak sumy USD w API (ceny mintów / skala); szczegóły w ledgerze lifecycle.
                                      </div>
                                    ) : null}
                                    {n.collect_zero_diagnostics ? (
                                      <div
                                        className="text-muted-foreground mt-1 leading-tight text-[10px]"
                                        title={n.collect_zero_diagnostics.methodology_note}
                                      >
                                        dlaczego 0: in-range~{n.collect_zero_diagnostics.in_range_time_share_pct_est ?? '—'}%
                                        {' · '}
                                        swapy~{n.collect_zero_diagnostics.swap_events_in_window_est}
                                        {' · '}
                                        udział~{n.collect_zero_diagnostics.position_share_pct_est ?? '—'}%
                                      </div>
                                    ) : null}
                                  </>
                                )
                              })()}
                            </td>
                            <td className="px-2 py-1 whitespace-nowrap font-mono">
                              {formatUsdField(n.realized_cashflow_usd, 3)}
                            </td>
                            <td
                              className={
                                (() => {
                                  const pct = parseFloat(String(n.net_pnl_pct ?? ''))
                                  return Number.isFinite(pct) && pct >= 0
                                    ? 'px-2 py-1 whitespace-nowrap font-mono text-green-500'
                                    : 'px-2 py-1 whitespace-nowrap font-mono text-red-500'
                                })()
                              }
                            >
                              {formatUsdField(n.net_pnl_usd, 3)} (
                              {Number.isFinite(parseFloat(String(n.net_pnl_pct ?? '')))
                                ? formatPercentFixed(n.net_pnl_pct, 3)
                                : '—'}
                              )
                            </td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                ) : (
                  <p className="text-sm text-muted-foreground">No lineage rows yet (missing IL edges / DB snapshots).</p>
                )}

                {streamLineage.totals ? (
                  <div className="rounded-md border border-border/60 bg-muted/10 px-3 py-2">
                    <div className="text-xs text-muted-foreground">Totals (across full chain)</div>
                    <div className="flex flex-wrap gap-x-6 gap-y-1 text-sm mt-1">
                      <div>
                        <span className="text-muted-foreground">baseline</span>{' '}
                        <span className="font-mono">{formatUsdFixed(streamLineage.totals.baseline_value_usd, 3)}</span>
                      </div>
                      <div>
                        <span className="text-muted-foreground">current</span>{' '}
                        <span className="font-mono">{formatUsdFixed(streamLineage.totals.current_value_usd, 3)}</span>
                      </div>
                      <div>
                        <span className="text-muted-foreground">tx fees</span>{' '}
                        <span className="font-mono text-[11px] leading-tight inline-block align-top">
                          {streamLineage.chain_cost_summary != null ? (
                            <>
                              <span className="block">
                                {streamLineage.chain_cost_summary.tx_fee_lamports_total.toLocaleString()} λ
                              </span>
                              <span className="block text-muted-foreground">
                                {formatUsdFixed(
                                  parseFloat(String(streamLineage.chain_cost_summary.tx_fees_usd_total)),
                                  4,
                                )}
                              </span>
                            </>
                          ) : (
                            formatUsdFixed(streamLineage.totals.tx_fees_usd, 3)
                          )}
                        </span>
                      </div>
                      <div>
                        <span className="text-muted-foreground">cashflow</span>{' '}
                        <span className="font-mono">{formatUsdFixed(streamLineage.totals.realized_cashflow_usd, 3)}</span>
                      </div>
                      <div>
                        <span className="text-muted-foreground">net PnL</span>{' '}
                        <span
                          className={
                            parseFloat(streamLineage.totals.net_pnl_pct) >= 0
                              ? 'font-mono text-green-500'
                              : 'font-mono text-red-500'
                          }
                        >
                          {formatUsdFixed(streamLineage.totals.net_pnl_usd, 3)} (
                          {formatPercentFixed(streamLineage.totals.net_pnl_pct, 3)})
                        </span>
                      </div>
                    </div>
                    {streamLineage.totals.note ? (
                      <div className="text-[11px] text-muted-foreground leading-snug mt-2">{streamLineage.totals.note}</div>
                    ) : null}
                  </div>
                ) : null}
              </CardContent>
            </Card>
          ) : (
            <Card>
              <CardHeader>
                <CardTitle>Position history (rotations)</CardTitle>
              </CardHeader>
              <CardContent className="text-sm text-muted-foreground">No lineage response.</CardContent>
            </Card>
          )}

          <Card>
            <CardHeader>
              <CardTitle>Lifecycle ledger (filtered)</CardTitle>
            </CardHeader>
            <CardContent className="text-sm text-muted-foreground space-y-2">
              <p>
                Rows from <code className="text-xs">/bot-activity/ledger</code> filtered to <strong>this</strong>{' '}
                position PDA (sesje poniżej). Oś <strong>Lifecycle timeline</strong> scala też wszystkie PDA z{' '}
                <code className="text-xs">stream-lineage</code> oraz IL ledger. Fee (USD) uses SOL/USD from{' '}
                <code className="text-xs">/prices/jupiter</code> (lamports → SOL → USD).
              </p>
              {!ledgerAnyPresent && (
                <p className="text-yellow-500">
                  Lifecycle ledger file missing on API host ({ledgerData?.path ?? '—'}).
                </p>
              )}
            </CardContent>
          </Card>

          {ledgerAnyPresent ? (
            <PositionLifecycleTimeline
              rows={timelineRows}
              solUsd={solUsd}
              tokenMintA={position.token_mint_a}
              tokenMintB={position.token_mint_b}
              priceA={position.token_price_a_usd}
              priceB={position.token_price_b_usd}
              chainPdaCount={chainSet.length}
            />
          ) : null}

          <Card>
            <CardHeader>
              <CardTitle className="text-base">IL ledger (rebalance events)</CardTitle>
            </CardHeader>
            <CardContent className="text-sm text-muted-foreground space-y-2">
              <p>
                From <code className="text-xs">/bot-activity/il-ledger</code> — rows where JSON contains this address
                (new <code className="text-xs">position</code> or <code className="text-xs">old_position</code>). Requires{' '}
                <code className="text-xs">CLMM_IL_LEDGER_PATH</code> on the API host (same file as{' '}
                <code className="text-xs">orca-bot-run --il-ledger-path</code>).
              </p>
              {!ilAnyPresent && (
                <p className="text-yellow-500">
                  IL ledger not configured or file missing ({ilLedgerData?.path ?? '—'}).
                </p>
              )}
              {ilAnyPresent && ilRows.length > 0 && (
                <div className="overflow-x-auto rounded-md border">
                  <table className="w-full text-xs">
                    <thead className="bg-muted/50">
                      <tr>
                        <th className="px-2 py-1 text-left">timestamp</th>
                        <th className="px-2 py-1 text-left">old → new</th>
                        <th className="px-2 py-1 text-left">reason</th>
                        <th className="px-2 py-1 text-left">Tx fee (λ · ~USD)</th>
                        <th className="px-2 py-1 text-left">session</th>
                      </tr>
                    </thead>
                    <tbody>
                      {ilRows.map((r, i) => (
                        <tr key={i} className="border-t border-border/60">
                          <td className="px-2 py-1 whitespace-nowrap">
                            {typeof r.timestamp === 'string' ? r.timestamp : '—'}
                          </td>
                          <td className="px-2 py-1 font-mono max-w-[12rem] truncate" title={String(r.old_position ?? '')}>
                            {typeof r.old_position === 'string' ? shortenAddress(r.old_position, 5) : '—'} →{' '}
                            {typeof r.position === 'string' ? shortenAddress(r.position, 5) : '—'}
                          </td>
                          <td className="px-2 py-1">{String(r.reason ?? '—')}</td>
                          <td className="px-2 py-1">
                            <LamportsFeeCell lamportsRaw={r.tx_cost_lamports} solUsd={solUsd} />
                          </td>
                          <td className="px-2 py-1 max-w-[8rem] truncate" title={String(r.rebalance_session_id ?? '')}>
                            {String(r.rebalance_session_id ?? '—')}
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
              {ilAnyPresent && ilRows.length === 0 && (
                <p className="text-muted-foreground">No IL rows for this address yet.</p>
              )}
            </CardContent>
          </Card>

          {Array.from(bySession.entries()).map(([session, rows]) => (
            <Card key={session ?? 'null'}>
              <CardHeader>
                <CardTitle className="text-base">
                  Session: {session === '_no_session' ? '(no rebalance_session_id)' : session}
                </CardTitle>
                <p className="text-xs text-muted-foreground">{rows.length} row(s)</p>
              </CardHeader>
              <CardContent className="overflow-x-auto">
                <table className="w-full text-xs">
                  <thead>
                    <tr className="border-b text-left text-muted-foreground">
                      <th className="py-1 pr-2">Time</th>
                      <th className="py-1 pr-2">Source</th>
                      <th className="py-1 pr-2">Event</th>
                      <th className="py-1 pr-2">Collect values</th>
                      <th className="py-1 pr-2">Tx fee (λ · ~USD)</th>
                    </tr>
                  </thead>
                  <tbody>
                    {rows.map((r, i) => (
                      <tr key={i} className="border-b border-border/50">
                        <td className="py-1 pr-2 whitespace-nowrap">{rowTs(r)}</td>
                        <td className="py-1 pr-2">{rowSource(r)}</td>
                        <td className="py-1 pr-2 font-mono">{rowEvent(r)}</td>
                        <td className="py-1 pr-2 font-mono text-[11px]">{rowCollectDetails(r)}</td>
                        <td className="py-1 pr-2">
                          <LamportsFeeCell lamportsRaw={r.tx_fee_lamports} solUsd={solUsd} />
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </CardContent>
            </Card>
          ))}

          {ledgerRows.length === 0 && ledgerAnyPresent && (
            <p className="text-muted-foreground text-sm">No matching lines yet for this address.</p>
          )}
        </Tabs.Content>
      </Tabs.Root>
    </div>
  )
}
