import { useEffect, useMemo, useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { useParams, Link, useNavigate } from 'react-router-dom'
import * as Tabs from '@radix-ui/react-tabs'
import { ArrowLeft, RefreshCw, X, DollarSign } from 'lucide-react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import {
  getPosition,
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
} from '@/lib/api'
import type { Strategy } from '@/lib/api'
import {
  formatUsdFixed,
  formatUsdUncollectedFees,
  formatPercentFixed,
  shortenAddress,
  formatDate,
  formatUsdcPriceRange,
} from '@/lib/utils'

/** Wrapped SOL mint — network fees are in native SOL (lamports). */
const WSOL_MINT = 'So11111111111111111111111111111111111111112'

type LedgerRow = Record<string, unknown>

function groupLedgerBySession(rows: LedgerRow[]): Map<string | null, LedgerRow[]> {
  const m = new Map<string | null, LedgerRow[]>()
  for (const r of rows) {
    const raw = r.rebalance_session_id
    const sid = typeof raw === 'string' && raw.trim() ? raw.trim() : null
    const key = sid ?? '_no_session'
    if (!m.has(key)) m.set(key, [])
    m.get(key)!.push(r)
  }
  return m
}

function rowFee(r: LedgerRow): string {
  const v = r.tx_fee_lamports
  if (typeof v === 'number') return `${v}`
  if (typeof v === 'string') return v
  return '—'
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

export default function PositionDetail() {
  const { address } = useParams<{ address: string }>()
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const [actionError, setActionError] = useState<string | null>(null)
  const [actionInfo, setActionInfo] = useState<string | null>(null)

  const { data: position, isLoading, isError, error } = useQuery({
    queryKey: ['position', address],
    queryFn: () => getPosition(address!),
    enabled: !!address,
    retry: 1,
  })

  const { data: ledgerData } = useQuery({
    queryKey: ['bot-ledger', address],
    queryFn: () => getBotLedger(1500, address),
    enabled: !!address,
  })

  const { data: ilLedgerData } = useQuery({
    queryKey: ['bot-il-ledger', address],
    queryFn: () => getBotIlLedger(200, address),
    enabled: !!address,
  })

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
    onSuccess: () => {
      setActionError(null)
      setActionInfo('Collect requested.')
      void queryClient.invalidateQueries({ queryKey: ['position', address] })
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

  if (isLoading) {
    return <div className="text-center py-8">Loading...</div>
  }

  if (isError) {
    const msg = error instanceof Error ? error.message : String(error)
    return (
      <div className="text-center py-8 space-y-3 max-w-lg mx-auto px-4">
        <p className="text-destructive font-medium">Nie udało się pobrać pozycji z API</p>
        <p className="text-sm text-muted-foreground break-words font-mono">{msg}</p>
        <p className="text-xs text-muted-foreground">
          Przy HTTP 502 / braku odpowiedzi backend nie działa albo Vite proxy (`API_UPSTREAM`) nie trafia w port API —
          to <strong className="text-foreground">nie</strong> znaczy, że pozycji nie ma on-chain.
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

  const ledgerRows = (ledgerData?.rows ?? []) as LedgerRow[]
  const ilRows = (ilLedgerData?.rows ?? []) as LedgerRow[]
  const bySession = groupLedgerBySession(ledgerRows)

  const rangeUsdcLine = formatUsdcPriceRange(
    position.range_lower_usdc ?? undefined,
    position.range_upper_usdc ?? undefined,
    position.range_usdc_quote ?? undefined,
  )

  return (
    <div className="space-y-6">
      <div className="flex items-center gap-4">
        <Link to="/positions">
          <Button variant="ghost" size="icon">
            <ArrowLeft className="h-4 w-4" />
          </Button>
        </Link>
        <h1 className="text-3xl font-bold">Position Details</h1>
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
            Ledger / rebalances
          </Tabs.Trigger>
        </Tabs.List>

        <Tabs.Content value="overview" className="mt-4 space-y-6">
          <div className="grid gap-6 md:grid-cols-2">
            <Card>
              <CardHeader>
                <CardTitle>Position Info</CardTitle>
              </CardHeader>
              <CardContent className="space-y-4">
                <div className="flex justify-between">
                  <span className="text-muted-foreground">Address</span>
                  <span className="font-mono">{shortenAddress(position.address, 8)}</span>
                </div>
                <div className="flex justify-between">
                  <span className="text-muted-foreground">Pool</span>
                  <span className="font-mono">{shortenAddress(position.pool_address, 8)}</span>
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
                  On-chain <code className="text-[10px]">fee_owed</code> raw: token A{' '}
                  <span className="font-mono tabular-nums">{position.pnl.fees_earned_a}</span> · token B{' '}
                  <span className="font-mono tabular-nums">{position.pnl.fees_earned_b}</span> (smallest
                  units). If both are 0, nothing has accrued in the position account yet. If non-zero but
                  USD stays $0, the price service did not return a USD rate for a pool mint.
                </p>
                <div className="flex justify-between">
                  <span className="text-muted-foreground">Impermanent Loss</span>
                  <span className="text-yellow-500">{formatPercentFixed(position.pnl.il_pct, 3)}</span>
                </div>
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
        </Tabs.Content>

        <Tabs.Content value="ledger" className="mt-4 space-y-4">
          <Card>
            <CardHeader>
              <CardTitle>Lifecycle ledger (filtered)</CardTitle>
            </CardHeader>
            <CardContent className="text-sm text-muted-foreground space-y-2">
              <p>
                Rows from <code className="text-xs">/bot-activity/ledger</code> whose JSON contains this position
                address. Grouped by <code className="text-xs">rebalance_session_id</code> when present (swap + bot +
                open/close in one session). Fee (USD) uses SOL/USD from{' '}
                <code className="text-xs">/prices/jupiter</code> (lamports → SOL → USD).
              </p>
              {ledgerData?.file_missing && (
                <p className="text-yellow-500">Ledger file missing on API host ({ledgerData.path}).</p>
              )}
            </CardContent>
          </Card>

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
              {ilLedgerData?.file_missing && (
                <p className="text-yellow-500">
                  IL ledger not configured or file missing ({ilLedgerData.path}).
                </p>
              )}
              {!ilLedgerData?.file_missing && ilRows.length > 0 && (
                <div className="overflow-x-auto rounded-md border">
                  <table className="w-full text-xs">
                    <thead className="bg-muted/50">
                      <tr>
                        <th className="px-2 py-1 text-left">timestamp</th>
                        <th className="px-2 py-1 text-left">old → new</th>
                        <th className="px-2 py-1 text-left">reason</th>
                        <th className="px-2 py-1 text-left">tx_cost_lamports</th>
                        <th className="px-2 py-1 text-left">tx_cost (USD)</th>
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
                            {String(r.old_position ?? '—')} → {String(r.position ?? '—')}
                          </td>
                          <td className="px-2 py-1">{String(r.reason ?? '—')}</td>
                          <td className="px-2 py-1">{String(r.tx_cost_lamports ?? '—')}</td>
                          <td className="px-2 py-1 whitespace-nowrap">
                            {lamportsToUsdDisplay(r.tx_cost_lamports, solUsd)}
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
              {!ilLedgerData?.file_missing && ilRows.length === 0 && (
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
                      <th className="py-1 pr-2">Fee (lamports)</th>
                      <th className="py-1 pr-2">Fee (USD)</th>
                    </tr>
                  </thead>
                  <tbody>
                    {rows.map((r, i) => (
                      <tr key={i} className="border-b border-border/50">
                        <td className="py-1 pr-2 whitespace-nowrap">{rowTs(r)}</td>
                        <td className="py-1 pr-2">{rowSource(r)}</td>
                        <td className="py-1 pr-2 font-mono">{rowEvent(r)}</td>
                        <td className="py-1 pr-2">{rowFee(r)}</td>
                        <td className="py-1 pr-2 whitespace-nowrap">
                          {lamportsToUsdDisplay(r.tx_fee_lamports, solUsd)}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </CardContent>
            </Card>
          ))}

          {ledgerRows.length === 0 && !ledgerData?.file_missing && (
            <p className="text-muted-foreground text-sm">No matching lines yet for this address.</p>
          )}
        </Tabs.Content>
      </Tabs.Root>
    </div>
  )
}
