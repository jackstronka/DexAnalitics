import { useMutation, useQueries, useQuery, useQueryClient } from '@tanstack/react-query'
import { useMemo, useState } from 'react'
import { Link, useNavigate } from 'react-router-dom'
import { Plus, RefreshCw } from 'lucide-react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import ApiDataHint from '@/components/ApiDataHint'
import {
  getOrcaPositionsByOwner,
  getPoolState,
  getPositions,
  getStrategies,
  getStrandedRebalances,
  dismissStrandedRebalance,
} from '@/lib/api'
import type { Position, Strategy } from '@/lib/api'
import { getDevWalletPubkey } from '@/lib/devWallet'
import {
  formatUSD,
  formatPercentFixed,
  formatNumber,
  shortenAddress,
  formatUsdcPriceRange,
  formatInvertedTokenPriceRange,
  formatUsdUncollectedFees,
} from '@/lib/utils'
import { PoolPairLabels } from '@/components/PoolPairLabels'

function rangeCellClass(inRange: boolean | undefined) {
  if (inRange === true) {
    return 'text-emerald-600 dark:text-emerald-400 border-l-2 border-emerald-500 pl-2'
  }
  if (inRange === false) {
    return 'text-red-600 dark:text-red-400 border-l-2 border-red-500 pl-2'
  }
  return 'text-muted-foreground border-l-2 border-border pl-2'
}

function rangeStatusLabel(inRange: boolean | undefined) {
  if (inRange === true) return 'In range'
  if (inRange === false) return 'Out of range'
  return '—'
}

function strategyTypeLabel(v: Strategy['strategy_type']) {
  return v.replace(/_/g, ' ')
}

function strategyParamsSummary(s: Strategy) {
  const p = s.parameters ?? {}
  const bits: string[] = []
  if (typeof p.rebalance_threshold_pct === 'number' && p.rebalance_threshold_pct > 0) {
    bits.push(`thr ${p.rebalance_threshold_pct}%`)
  }
  if (typeof p.min_rebalance_interval_hours === 'number' && p.min_rebalance_interval_hours > 0) {
    bits.push(`every ${p.min_rebalance_interval_hours}h`)
  }
  if (typeof p.range_width_pct === 'number' && p.range_width_pct > 0) {
    bits.push(`width ${p.range_width_pct}%`)
  }
  if (typeof p.max_il_pct === 'number' && p.max_il_pct > 0) {
    bits.push(`max IL ${p.max_il_pct}%`)
  }
  if (p.periodic_requires_out_of_range === true) {
    bits.push('only OOR')
  }
  if (p.rebalance_on_range_exit_immediately === true) {
    bits.push('instant on range-exit')
  }
  return bits.length ? bits.join(' · ') : 'no explicit toggles'
}

function parseNum(v: unknown): number | null {
  if (typeof v === 'number' && Number.isFinite(v)) return v
  if (typeof v === 'string') {
    const n = parseFloat(v)
    return Number.isFinite(n) ? n : null
  }
  return null
}

function estimateNowUsdcFromPosition(p: Position): number | null {
  const quote = (p.range_usdc_quote ?? '').toLowerCase()
  const labelA = (p.token_a_label ?? '').toLowerCase()
  const labelB = (p.token_b_label ?? '').toLowerCase()
  if (quote && labelA && quote.includes(labelA) && typeof p.token_price_a_usd === 'number') {
    return p.token_price_a_usd
  }
  if (quote && labelB && quote.includes(labelB) && typeof p.token_price_b_usd === 'number') {
    return p.token_price_b_usd
  }
  if (labelA === 'usdc' && typeof p.token_price_b_usd === 'number') return p.token_price_b_usd
  if (labelB === 'usdc' && typeof p.token_price_a_usd === 'number') return p.token_price_a_usd
  return null
}

function normalizePendingReopenReason(v?: string | null) {
  if (!v) return 'Waiting for reopen cycle.'
  if (v.toLowerCase().includes('already queued for pending-open recovery')) {
    return 'Queued for auto-reopen (waiting for next recovery cycle).'
  }
  return v
}

export default function Positions() {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const devPk = getDevWalletPubkey()
  const [ownerInput, setOwnerInput] = useState(() => devPk ?? '')
  const [appliedOwner, setAppliedOwner] = useState(() => devPk ?? '')

  const { data, isLoading, refetch } = useQuery({
    queryKey: ['positions'],
    queryFn: getPositions,
  })
  const strategiesQ = useQuery({
    queryKey: ['strategies'],
    queryFn: getStrategies,
    staleTime: 30_000,
  })

  const chainQ = useQuery({
    queryKey: ['orca-positions-by-owner', appliedOwner],
    queryFn: () => getOrcaPositionsByOwner(appliedOwner),
    enabled: appliedOwner.trim().length > 0,
    staleTime: 60_000,
  })
  const strandedQ = useQuery({
    queryKey: ['stranded-rebalances'],
    queryFn: getStrandedRebalances,
    staleTime: 10_000,
    refetchInterval: 15_000,
    retry: 1,
  })

  const positions = data?.positions || []
  const monitoredPools = useMemo(
    () => Array.from(new Set(positions.map((p) => p.pool_address).filter((v) => !!v))),
    [positions],
  )
  const poolStateQueries = useQueries({
    queries: monitoredPools.map((poolAddress) => ({
      queryKey: ['pool-state', poolAddress],
      queryFn: () => getPoolState(poolAddress),
      staleTime: 30_000,
      retry: 1,
    })),
  })
  const poolSpotByAddress = useMemo(() => {
    const m = new Map<string, number>()
    monitoredPools.forEach((poolAddress, idx) => {
      const n = parseNum(poolStateQueries[idx]?.data?.price)
      if (n !== null) m.set(poolAddress, n)
    })
    return m
  }, [monitoredPools, poolStateQueries])
  const poolLabelByAddress = useMemo(() => {
    const m = new Map<string, string>()
    for (const p of positions) {
      if (!p.pool_address) continue
      const a = p.token_a_label?.trim()
      const b = p.token_b_label?.trim()
      if (a && b) m.set(p.pool_address, `${a} / ${b}`)
    }
    for (const row of chainQ.data?.entries ?? []) {
      if (!row.pool_address) continue
      if (m.has(row.pool_address)) continue
      const a = row.token_a_label?.trim()
      const b = row.token_b_label?.trim()
      if (a && b) m.set(row.pool_address, `${a} / ${b}`)
    }
    return m
  }, [positions, chainQ.data])

  const strategiesByPosition = useMemo(() => {
    const map = new Map<string, Strategy[]>()
    for (const s of strategiesQ.data?.strategies ?? []) {
      for (const addr of s.parameters?.position_addresses ?? []) {
        const key = addr.trim()
        if (!key) continue
        if (!map.has(key)) map.set(key, [])
        map.get(key)!.push(s)
      }
    }
    return map
  }, [strategiesQ.data])
  const pendingReopenItems = useMemo(
    () =>
      (strandedQ.data?.items ?? []).filter(
        (it) => it.close_seen === true && it.open_seen === false,
      ),
    [strandedQ.data],
  )
  const dismissStrandedM = useMutation({
    mutationFn: (sessionId: string) => dismissStrandedRebalance(sessionId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['stranded-rebalances'] })
    },
  })

  return (
    <div className="space-y-6">
      <ApiDataHint />

      <div className="flex items-center justify-between">
        <h1 className="text-3xl font-bold">Positions</h1>
        <div className="flex gap-2">
          <Button variant="outline" size="sm" onClick={() => refetch()}>
            <RefreshCw className="h-4 w-4 mr-2" />
            Refresh
          </Button>
          <Button size="sm" onClick={() => navigate('/positions/new')}>
            <Plus className="h-4 w-4 mr-2" />
            Open Position
          </Button>
        </div>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Monitored positions (API)</CardTitle>
          <p className="text-sm text-muted-foreground font-normal">
            Z monitora w pamięci procesu — szczegóły i PnL tylko dla tych adresów.
          </p>
        </CardHeader>
        <CardContent>
          {isLoading ? (
            <div className="text-center py-8 text-muted-foreground">Loading...</div>
          ) : positions.length === 0 ? (
            <div className="text-center py-8 text-muted-foreground space-y-2 max-w-xl mx-auto">
              <p>Brak pozycji w monitorze API — to nie jest lista wszystkich NFT Orca na portfelu.</p>
              <p className="text-xs">
                Uruchom strategię z adresami pozycji, dodaj pozycję do monitora, albo sprawdź on-chain:{' '}
                <code className="text-[11px]">orca-positions-list</code> (CLI).
              </p>
            </div>
          ) : (
            <div className="overflow-x-auto">
              <table className="w-full">
                <thead>
                  <tr className="border-b text-left text-sm text-muted-foreground">
                    <th className="pb-3 font-medium">Position</th>
                    <th className="pb-3 font-medium">Strategy</th>
                    <th className="pb-3 font-medium">Range (in / out)</th>
                    <th className="pb-3 font-medium text-right">Value</th>
                    <th className="pb-3 font-medium text-right">PnL</th>
                    <th className="pb-3 font-medium text-right">Fees (uncollected)</th>
                    <th className="pb-3 font-medium text-center">Status</th>
                  </tr>
                </thead>
                <tbody>
                  {positions.map((position) => (
                    <tr key={position.address} className="border-b last:border-0">
                      <td className="py-4 max-w-[14rem]">
                        <Link
                          to={`/positions/${position.address}`}
                          className="block hover:text-primary space-y-1"
                        >
                          <PoolPairLabels
                            labelA={position.token_a_label}
                            labelB={position.token_b_label}
                            mintA={position.token_mint_a}
                            mintB={position.token_mint_b}
                            priceA={position.token_price_a_usd}
                            priceB={position.token_price_b_usd}
                          />
                          <div className="font-medium font-mono text-sm">
                            {shortenAddress(position.address)}
                          </div>
                        </Link>
                      </td>
                      <td className="py-4 max-w-[18rem]">
                        {(() => {
                          const linked = strategiesByPosition.get(position.address) ?? []
                          if (!linked.length) {
                            return <span className="text-xs text-muted-foreground">Not linked</span>
                          }
                          return (
                            <div className="space-y-1.5">
                              {linked.map((s) => (
                                <div key={s.id} className="text-xs leading-tight">
                                  <div className="font-medium">
                                    {s.name}{' '}
                                    <span className="text-muted-foreground">({strategyTypeLabel(s.strategy_type)})</span>
                                  </div>
                                  <div className="text-muted-foreground">{strategyParamsSummary(s)}</div>
                                </div>
                              ))}
                            </div>
                          )
                        })()}
                      </td>
                      <td className="py-4">
                        <div className="space-y-1">
                          <span className={`text-sm block ${rangeCellClass(position.in_range)}`}>
                            {formatUsdcPriceRange(
                              position.range_lower_usdc ?? undefined,
                              position.range_upper_usdc ?? undefined,
                              position.range_usdc_quote ?? undefined,
                            ) ??
                              formatInvertedTokenPriceRange(
                                position.range_lower_price ?? undefined,
                                position.range_upper_price ?? undefined,
                                position.range_price_quote ?? undefined,
                              ) ??
                              `${position.tick_lower} → ${position.tick_upper}`}
                          </span>
                          {(() => {
                            const lowerUsdc = parseNum(position.range_lower_usdc)
                            const upperUsdc = parseNum(position.range_upper_usdc)
                            const lowerGeneric = parseNum(position.range_lower_price)
                            const upperGeneric = parseNum(position.range_upper_price)
                            const useUsdc = lowerUsdc !== null && upperUsdc !== null
                            const lower = useUsdc ? lowerUsdc : lowerGeneric
                            const upper = useUsdc ? upperUsdc : upperGeneric
                            const now = useUsdc
                              ? estimateNowUsdcFromPosition(position)
                              : (poolSpotByAddress.get(position.pool_address) ?? null)
                            if (lower === null || upper === null || now === null || upper <= lower) return null
                            const markerPct = Math.max(0, Math.min(100, ((now - lower) / (upper - lower)) * 100))
                            return (
                              <div className="pt-0.5">
                                <div className="relative h-1.5 rounded-full bg-muted">
                                  <span
                                    className={`absolute top-1/2 h-3 w-3 -translate-y-1/2 -translate-x-1/2 rounded-full border border-background ${
                                      position.in_range ? 'bg-emerald-500' : 'bg-red-500'
                                    }`}
                                    style={{ left: `${markerPct}%` }}
                                    aria-label="Current price inside position range"
                                  />
                                </div>
                              </div>
                            )
                          })()}
                          <span className="text-[11px] text-muted-foreground">
                            {rangeStatusLabel(position.in_range)}
                          </span>
                        </div>
                      </td>
                      <td className="py-4 text-right font-medium">
                        {formatUSD(position.value_usd)}
                      </td>
                      <td className={`py-4 text-right ${
                        parseFloat(position.pnl.net_pnl_pct) >= 0 ? 'text-green-500' : 'text-red-500'
                      }`}>
                        {formatPercentFixed(position.pnl.net_pnl_pct, 3)}
                      </td>
                      <td className="py-4 text-right text-green-500">
                        <div className="space-y-0.5">
                          <div>{formatUsdUncollectedFees(position.pnl.fees_earned_usd)}</div>
                          {position.uncollected_fees ? (
                            <div className="text-[10px] text-muted-foreground font-mono">
                              {position.uncollected_fees.token_a_label}:{' '}
                              {formatNumber(position.uncollected_fees.amount_a, 6)} ·{' '}
                              {position.uncollected_fees.token_b_label}:{' '}
                              {formatNumber(position.uncollected_fees.amount_b, 6)}
                              <div className="mt-0.5">
                                raw A {position.pnl.fees_earned_a.toLocaleString()} · raw B{' '}
                                {position.pnl.fees_earned_b.toLocaleString()}
                              </div>
                            </div>
                          ) : null}
                        </div>
                      </td>
                      <td className="py-4 text-center">
                        <span className={`inline-flex items-center gap-1.5 px-2 py-1 rounded-full text-xs font-medium ${
                          position.status === 'active' 
                            ? 'bg-green-500/10 text-green-500' 
                            : position.status === 'pending'
                            ? 'bg-yellow-500/10 text-yellow-500'
                            : 'bg-muted text-muted-foreground'
                        }`}>
                          <span className={`h-1.5 w-1.5 rounded-full ${
                            position.status === 'active' 
                              ? 'bg-green-500' 
                              : position.status === 'pending'
                              ? 'bg-yellow-500'
                              : 'bg-muted-foreground'
                          }`} />
                          {position.status}
                        </span>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Closed by bot, waiting for reopen</CardTitle>
          <p className="text-sm text-muted-foreground font-normal">
            Sesje rebalance, gdzie bot zamknął starą pozycję, ale nowa nie została jeszcze otwarta.
            Po udanym reopen wpis znika z tej sekcji.
          </p>
        </CardHeader>
        <CardContent>
          {strandedQ.isLoading ? (
            <div className="text-center py-6 text-muted-foreground">Loading stranded rebalances...</div>
          ) : strandedQ.error ? (
            <div className="rounded-md border border-destructive/40 bg-destructive/5 px-3 py-2 text-sm text-destructive">
              {(strandedQ.error as Error).message}
            </div>
          ) : pendingReopenItems.length === 0 ? (
            <div className="text-muted-foreground text-sm">Brak oczekujących close-&gt;open.</div>
          ) : (
            <div className="overflow-x-auto">
              <table className="w-full">
                <thead>
                  <tr className="border-b text-left text-sm text-muted-foreground">
                    <th className="pb-3 font-medium">Closed position</th>
                    <th className="pb-3 font-medium">Pool</th>
                    <th className="pb-3 font-medium">Closed at</th>
                    <th className="pb-3 font-medium">Intended range</th>
                    <th className="pb-3 font-medium">Reason</th>
                    <th className="pb-3 font-medium">Session</th>
                    <th className="pb-3 font-medium text-right">Action</th>
                  </tr>
                </thead>
                <tbody>
                  {pendingReopenItems.map((it) => (
                    <tr key={it.rebalance_session_id} className="border-b last:border-0">
                      <td className="py-3 text-sm">
                        {it.old_position ? (
                          <Link to={`/positions/${it.old_position}`} className="font-mono hover:text-primary">
                            {shortenAddress(it.old_position)}
                          </Link>
                        ) : (
                          <span className="text-muted-foreground">—</span>
                        )}
                      </td>
                      <td className="py-3 text-sm text-muted-foreground">
                        {it.token_a_label && it.token_b_label
                          ? `${it.token_a_label} / ${it.token_b_label}`
                          : it.pool_address
                            ? (poolLabelByAddress.get(it.pool_address) ?? shortenAddress(it.pool_address))
                            : '—'}
                      </td>
                      <td className="py-3 text-sm text-muted-foreground">{it.close_ts_utc ?? '—'}</td>
                      <td className="py-3 text-sm text-muted-foreground">
                        {it.intended_tick_lower != null && it.intended_tick_upper != null
                          ? `${it.intended_tick_lower} → ${it.intended_tick_upper}`
                          : '—'}
                      </td>
                      <td className="py-3 text-sm text-muted-foreground">
                        {normalizePendingReopenReason(it.reason ?? it.note)}
                      </td>
                      <td className="py-3 text-xs font-mono text-muted-foreground">
                        {shortenAddress(it.rebalance_session_id)}
                      </td>
                      <td className="py-3 text-right">
                        <Button
                          type="button"
                          variant="outline"
                          size="sm"
                          className="h-7 px-2 text-[11px]"
                          disabled={dismissStrandedM.isPending}
                          onClick={() => dismissStrandedM.mutate(it.rebalance_session_id)}
                        >
                          {dismissStrandedM.isPending ? 'Removing...' : 'Remove'}
                        </Button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>On-chain Orca positions (RPC)</CardTitle>
          <p className="text-sm text-muted-foreground font-normal">
            Skan NFT Whirlpool dla portfela — to samo co <code className="text-[11px]">orca-positions-list</code>. Wymaga
            działającego RPC w API; nie używa monitora strategii.
          </p>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex flex-col gap-3 sm:flex-row sm:items-end">
            <div className="flex-1 space-y-1">
              <label className="text-xs text-muted-foreground">Owner (base58)</label>
              <input
                className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm font-mono"
                value={ownerInput}
                onChange={(e) => setOwnerInput(e.target.value)}
                placeholder="Wklej pubkey portfela"
              />
            </div>
            <Button
              type="button"
              variant="secondary"
              onClick={() => setAppliedOwner(ownerInput.trim())}
            >
              Load on-chain
            </Button>
            <Button
              type="button"
              variant="outline"
              onClick={() => chainQ.refetch()}
              disabled={!appliedOwner.trim()}
            >
              <RefreshCw className="h-4 w-4 mr-2" />
              Refresh
            </Button>
          </div>
          {chainQ.isLoading ? (
            <div className="text-center py-6 text-muted-foreground">Loading RPC…</div>
          ) : chainQ.error ? (
            <div className="rounded-md border border-destructive/40 bg-destructive/5 px-3 py-2 text-sm text-destructive">
              {(chainQ.error as Error).message}
            </div>
          ) : !appliedOwner.trim() ? (
            <div className="text-muted-foreground text-sm">Podaj owner i kliknij „Load on-chain”.</div>
          ) : (
            <>
              <p className="text-xs text-muted-foreground">
                RPC: <code className="break-all">{chainQ.data?.rpc_url ?? '—'}</code> — znaleziono:{' '}
                <strong>{chainQ.data?.total ?? 0}</strong>
              </p>
              {(chainQ.data?.entries?.length ?? 0) === 0 ? (
                <div className="text-muted-foreground text-sm py-4">Brak pozycji Whirlpool dla tego ownera.</div>
              ) : (
                <div className="overflow-x-auto">
                  <table className="w-full">
                    <thead>
                      <tr className="border-b text-left text-sm text-muted-foreground">
                        <th className="pb-3 font-medium">Kind</th>
                        <th className="pb-3 font-medium">Pair (mints · USD)</th>
                        <th className="pb-3 font-medium">Whirlpool</th>
                        <th className="pb-3 font-medium">Range (in / out)</th>
                        <th className="pb-3 font-medium text-right">Liquidity (raw)</th>
                      </tr>
                    </thead>
                    <tbody>
                      {chainQ.data!.entries.map((row) => (
                        <tr key={row.position_address} className="border-b last:border-0">
                          <td className="py-3 text-xs">{row.kind}</td>
                          <td className="py-3 text-xs max-w-[14rem]">
                            <Link
                              to={`/positions/${row.position_address}`}
                              className="block hover:text-primary space-y-1"
                            >
                              <PoolPairLabels
                                labelA={row.token_a_label}
                                labelB={row.token_b_label}
                                mintA={row.token_mint_a}
                                mintB={row.token_mint_b}
                                priceA={row.token_price_a_usd}
                                priceB={row.token_price_b_usd}
                              />
                              {row.token_a_label && row.token_b_label ? (
                                <div className="text-[11px] text-muted-foreground font-mono">
                                  PDA {shortenAddress(row.position_address)}
                                </div>
                              ) : (
                                <div className="font-mono font-medium">{shortenAddress(row.position_address)}</div>
                              )}
                              {row.position_bundle_address ? (
                                <span className="block text-muted-foreground mt-0.5 text-[10px]">
                                  bundle {shortenAddress(row.position_bundle_address)}
                                </span>
                              ) : null}
                            </Link>
                          </td>
                          <td className="py-3 text-muted-foreground font-mono text-xs">
                            {shortenAddress(row.pool_address)}
                          </td>
                          <td className="py-3">
                            <div className="space-y-0.5">
                              <span className={`text-sm block ${rangeCellClass(row.in_range)}`}>
                                {formatUsdcPriceRange(
                                  row.range_lower_usdc ?? undefined,
                                  row.range_upper_usdc ?? undefined,
                                  row.range_usdc_quote ?? undefined,
                                ) ??
                                  formatInvertedTokenPriceRange(
                                    row.range_lower_price ?? undefined,
                                    row.range_upper_price ?? undefined,
                                    row.range_price_quote ?? undefined,
                                  ) ??
                                  `${row.tick_lower} → ${row.tick_upper}`}
                              </span>
                              <span className="text-[11px] text-muted-foreground">
                                {rangeStatusLabel(row.in_range)}
                              </span>
                            </div>
                          </td>
                          <td className="py-3 text-right font-mono text-xs">{row.liquidity}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
            </>
          )}
        </CardContent>
      </Card>
    </div>
  )
}
