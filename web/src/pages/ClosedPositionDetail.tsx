import { useQuery } from '@tanstack/react-query'
import { Link, useParams } from 'react-router-dom'
import { ArrowLeft, RefreshCw } from 'lucide-react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { getBacktestJob, getPositionExperimentConfig, getPositionLifecycleSummary, getPositionStreamLineage, runBacktestFromClosedPosition } from '@/lib/api'
import { formatDate, formatPercentFixed, formatUsdFixed, shortenAddress } from '@/lib/utils'
import { useMutation } from '@tanstack/react-query'
import { useState } from 'react'

function usdOrDash(v: string | number, digits = 3): string {
  const n = typeof v === 'string' ? parseFloat(v) : v
  if (!Number.isFinite(n) || n === 0) return '—'
  return formatUsdFixed(n, digits)
}

function parseUsdNum(v: string | number): number | null {
  const n = typeof v === 'string' ? parseFloat(v) : v
  return Number.isFinite(n) ? n : null
}

export default function ClosedPositionDetail() {
  const { address } = useParams<{ address: string }>()
  const pos = (address ?? '').trim()

  const lifecycleQ = useQuery({
    queryKey: ['position-lifecycle-summary', pos],
    queryFn: () => getPositionLifecycleSummary(pos),
    enabled: pos.length > 0,
    staleTime: 30_000,
    retry: 1,
  })

  const lineageQ = useQuery({
    queryKey: ['position-stream-lineage', pos],
    queryFn: () => getPositionStreamLineage(pos),
    enabled: pos.length > 0,
    staleTime: 30_000,
    retry: 0,
  })

  const data = lifecycleQ.data
  const streamLineage = lineageQ.data
  const totals = streamLineage?.totals ?? null
  const entryNode = streamLineage?.nodes.find((n) => n.position_address === pos)
  const chainCost = streamLineage?.chain_cost_summary

  const cfgQ = useQuery({
    queryKey: ['position-experiment-config', pos],
    queryFn: () => getPositionExperimentConfig(pos),
    enabled: pos.length > 0,
    staleTime: 30_000,
    retry: 0,
  })

  const [backtestJobId, setBacktestJobId] = useState<string | null>(null)

  const runBacktestM = useMutation({
    mutationFn: async () => {
      return await runBacktestFromClosedPosition({
        position_address: pos,
        strategy: 'static',
        fee_source: 'snapshots',
        price_path_source: 'snapshots',
        snapshot_protocol: 'orca',
      })
    },
    onSuccess: (r) => setBacktestJobId(r.id),
  })

  const jobQ = useQuery({
    queryKey: ['backtest-job', backtestJobId],
    queryFn: () => getBacktestJob(backtestJobId!),
    enabled: !!backtestJobId,
    refetchInterval: backtestJobId ? 2000 : false,
    staleTime: 0,
    retry: 0,
  })

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between gap-4 flex-wrap">
        <div className="flex items-center gap-3 min-w-0">
          <Link to="/positions/closed">
            <Button variant="outline" size="sm">
              <ArrowLeft className="h-4 w-4 mr-2" />
              Back
            </Button>
          </Link>
          <div className="min-w-0">
            <h1 className="text-2xl font-bold truncate">Closed position</h1>
            <p className="text-sm text-muted-foreground font-mono truncate">{pos}</p>
          </div>
        </div>
        <Button
          variant="outline"
          size="sm"
          onClick={() => {
            void lifecycleQ.refetch()
            void lineageQ.refetch()
          }}
          disabled={lifecycleQ.isPending}
        >
          <RefreshCw className="h-4 w-4 mr-2" />
          Refresh
        </Button>
      </div>

      {lifecycleQ.isError ? (
        <Card>
          <CardHeader>
            <CardTitle>Error</CardTitle>
          </CardHeader>
          <CardContent className="text-sm text-muted-foreground">
            {lifecycleQ.error instanceof Error ? lifecycleQ.error.message : String(lifecycleQ.error)}
          </CardContent>
        </Card>
      ) : null}

      <div className="grid gap-4 md:grid-cols-2">
        <Card className="border-border/80">
          <CardHeader>
            <CardTitle className="text-base">Koszty i prowizje — ten adres</CardTitle>
            <p className="text-sm text-muted-foreground font-normal">
              Tylko PDA tej strony. <span className="text-foreground/90">Sieć</span> = opłaty Solany za każdą transakcję
              (open, collect, close…). <span className="text-foreground/90">LP zebrane</span> = prowizje puli z evenciów{' '}
              <code className="text-[11px]">bot_collect_fees</code>.
            </p>
          </CardHeader>
          <CardContent className="space-y-3 text-sm">
            {entryNode ? (
              <>
                <div className="rounded-md border bg-muted/20 px-3 py-2">
                  <div className="text-xs text-muted-foreground uppercase tracking-wide">Koszt sieci (tx)</div>
                  <div className="font-mono text-lg mt-0.5">
                    {(entryNode.tx_fee_lamports ?? 0).toLocaleString()} lamports
                    <span className="text-muted-foreground"> · </span>
                    {formatUsdFixed(parseFloat(String(entryNode.tx_fees_usd)), 4)}
                  </div>
                </div>
                <div className="rounded-md border bg-muted/20 px-3 py-2">
                  <div className="text-xs text-muted-foreground uppercase tracking-wide">Prowizje LP zebrane</div>
                  <div className="font-mono text-lg mt-0.5">
                    {formatUsdFixed(parseFloat(String(entryNode.fees_collected_usd ?? '0')), 4)}
                    <span className="text-muted-foreground text-sm font-sans">
                      {' '}
                      · {entryNode.collect_events ?? 0}× collect
                    </span>
                  </div>
                </div>
              </>
            ) : (
              <p className="text-muted-foreground text-sm">
                {lineageQ.isPending ? 'Ładowanie lineage…' : 'Brak węzła lineage dla tego adresu.'}
              </p>
            )}
            {data?.collected_fee_token_a_ui != null || data?.collected_fee_token_b_ui != null ? (
              <p className="text-xs text-muted-foreground">
                Tokeny (lifecycle summary): A {String(data?.collected_fee_token_a_ui ?? '—')} · B{' '}
                {String(data?.collected_fee_token_b_ui ?? '—')}
              </p>
            ) : null}
          </CardContent>
        </Card>

        <Card className="border-border/80">
          <CardHeader>
            <CardTitle className="text-base">Koszty i prowizje — cały łańcuch</CardTitle>
            <p className="text-sm text-muted-foreground font-normal">
              Suma po wszystkich PDA w rotacji (ten sam łańcuch co w tabeli poniżej).
            </p>
          </CardHeader>
          <CardContent className="space-y-3 text-sm">
            {chainCost ? (
              <>
                <div className="rounded-md border bg-muted/20 px-3 py-2">
                  <div className="text-xs text-muted-foreground uppercase tracking-wide">Koszt sieci (tx) — suma</div>
                  <div className="font-mono text-lg mt-0.5">
                    {chainCost.tx_fee_lamports_total.toLocaleString()} lamports
                    <span className="text-muted-foreground"> · </span>
                    {formatUsdFixed(parseFloat(chainCost.tx_fees_usd_total), 4)}
                  </div>
                </div>
                <div className="rounded-md border bg-muted/20 px-3 py-2">
                  <div className="text-xs text-muted-foreground uppercase tracking-wide">Prowizje LP zebrane — suma</div>
                  <div className="font-mono text-lg mt-0.5">
                    {formatUsdFixed(parseFloat(chainCost.fees_collected_usd_total), 4)}
                    <span className="text-muted-foreground text-sm font-sans">
                      {' '}
                      · {chainCost.collect_events_total}× collect (łącznie)
                    </span>
                  </div>
                </div>
              </>
            ) : (
              <p className="text-muted-foreground text-sm">
                {lineageQ.isPending ? 'Ładowanie…' : 'Brak podsumowania łańcucha.'}
              </p>
            )}
          </CardContent>
        </Card>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Wynik ekonomiczny (łańcuch)</CardTitle>
          <p className="text-sm text-muted-foreground font-normal">
            Wartości start/koniec i cashflow z agregacji stream — osobno od tabeli „koszty sieci vs LP” powyżej.
          </p>
        </CardHeader>
        <CardContent className="text-sm text-muted-foreground space-y-2">
          {totals ? (
            <>
              <div className="flex flex-wrap gap-x-6 gap-y-1">
                <div>
                  <span className="text-muted-foreground">baseline</span>{' '}
                  <span className="font-mono">{formatUsdFixed(parseFloat(totals.baseline_value_usd), 3)}</span>
                </div>
                <div>
                  <span className="text-muted-foreground">current/end</span>{' '}
                  <span className="font-mono">{formatUsdFixed(parseFloat(totals.current_value_usd), 3)}</span>
                </div>
                <div>
                  <span className="text-muted-foreground">cashflow (łącznie)</span>{' '}
                  <span className="font-mono">{formatUsdFixed(parseFloat(totals.realized_cashflow_usd), 3)}</span>
                </div>
              </div>
              <div className="pt-1">
                <span className="text-muted-foreground">net PnL</span>{' '}
                <span className="font-mono">{formatUsdFixed(parseFloat(totals.net_pnl_usd), 3)}</span>
                <span className="font-mono">
                  {' '}
                  ({(parseFloat(totals.net_pnl_pct) * 100).toFixed(3)}%)
                </span>
              </div>
              {totals.note ? <div className="text-xs">{totals.note}</div> : null}
            </>
          ) : (
            <div>—</div>
          )}
        </CardContent>
      </Card>

      {lineageQ.isPending ? (
        <Card>
          <CardHeader>
            <CardTitle>Position history (rotations)</CardTitle>
          </CardHeader>
          <CardContent className="text-sm text-muted-foreground">Loading lineage…</CardContent>
        </Card>
      ) : streamLineage ? (
        <Card>
          <CardHeader>
            <CardTitle>Position history (rotations)</CardTitle>
            <p className="text-sm text-muted-foreground font-normal">
              Łańcuch PDA (stara → nowa) z API <code className="text-[11px]">/positions/…/stream-lineage</code>. CLI
              zapisuje <code className="text-[11px]">position_open</code> / <code className="text-[11px]">position_close</code>, bot —{' '}
              <code className="text-[11px]">bot_*</code>; oba są łączone.
            </p>
          </CardHeader>
          <CardContent className="space-y-3">
            {streamLineage.note ? (
              <p className="text-[11px] text-muted-foreground leading-snug">{streamLineage.note}</p>
            ) : null}
            {lineageQ.isError ? (
              <p className="text-sm text-destructive">
                {lineageQ.error instanceof Error ? lineageQ.error.message : String(lineageQ.error)}
              </p>
            ) : null}
            {streamLineage.chain.length > 1 ? (
              <div className="text-xs text-muted-foreground font-mono break-all">
                chain: {streamLineage.chain.map((a) => shortenAddress(a, 6)).join(' → ')}
              </div>
            ) : null}
            {streamLineage.nodes.length > 0 ? (
              <div className="overflow-x-auto rounded-md border">
                <table className="w-full text-xs">
                  <thead className="bg-muted/50">
                    <tr>
                      <th className="px-2 py-1 text-left">#</th>
                      <th className="px-2 py-1 text-left">position</th>
                      <th className="px-2 py-1 text-left">opened</th>
                      <th className="px-2 py-1 text-left">closed</th>
                      <th className="px-2 py-1 text-left">start</th>
                      <th className="px-2 py-1 text-left">end</th>
                      <th className="px-2 py-1 text-left">principal Δ</th>
                      <th className="px-2 py-1 text-left">Sieć (tx)</th>
                      <th className="px-2 py-1 text-left">LP zebrane</th>
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
                        <td className="px-2 py-1 whitespace-nowrap font-mono">{usdOrDash(n.baseline_value_usd, 3)}</td>
                        <td className="px-2 py-1 whitespace-nowrap font-mono">{usdOrDash(n.current_value_usd, 3)}</td>
                        <td className="px-2 py-1 whitespace-nowrap font-mono">
                          {(() => {
                            const a = parseUsdNum(n.baseline_value_usd)
                            const b = parseUsdNum(n.current_value_usd)
                            if (a === null || b === null) return '—'
                            return formatUsdFixed(b - a, 3)
                          })()}
                        </td>
                        <td className="px-2 py-1 whitespace-nowrap font-mono text-[11px] leading-tight">
                          {(n.tx_fee_lamports ?? 0).toLocaleString()} λ
                          <br />
                          <span className="text-muted-foreground">{formatUsdFixed(parseFloat(String(n.tx_fees_usd)), 4)}</span>
                        </td>
                        <td className="px-2 py-1 whitespace-nowrap font-mono text-[11px]">
                          {formatUsdFixed(parseFloat(String(n.fees_collected_usd ?? '0')), 4)}
                          <span className="text-muted-foreground"> · {n.collect_events ?? 0}×</span>
                        </td>
                        <td
                          className={
                            parseFloat(n.net_pnl_pct) >= 0
                              ? 'px-2 py-1 whitespace-nowrap font-mono text-green-500'
                              : 'px-2 py-1 whitespace-nowrap font-mono text-red-500'
                          }
                        >
                          {formatUsdFixed(n.net_pnl_usd, 3)} ({formatPercentFixed(n.net_pnl_pct, 3)})
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            ) : (
              <p className="text-sm text-muted-foreground">
                Jedna pozycja w łańcuchu — brak powiązanych PDA w ledgerze albo brak pasujących zdarzeń open/close.
              </p>
            )}
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
          <CardTitle>Detected config (open snapshot)</CardTitle>
          <p className="text-sm text-muted-foreground font-normal">
            Z <code className="text-[11px]">registry_open.details</code> + sesji otwarcia (best-effort).
          </p>
        </CardHeader>
        <CardContent className="text-sm text-muted-foreground space-y-2">
          {cfgQ.isPending ? (
            <div>Loading...</div>
          ) : cfgQ.isError ? (
            <div>{cfgQ.error instanceof Error ? cfgQ.error.message : String(cfgQ.error)}</div>
          ) : (
            <>
              {cfgQ.data?.note ? <div>{cfgQ.data.note}</div> : null}
              <div>
                <span className="text-foreground font-medium">ticks</span>: {cfgQ.data?.tick_lower ?? '—'} →{' '}
                {cfgQ.data?.tick_upper ?? '—'}
              </div>
              <div>
                <span className="text-foreground font-medium">derived range (A/B)</span>:{' '}
                {typeof cfgQ.data?.derived_lower === 'number' ? cfgQ.data.derived_lower.toFixed(6) : '—'} →{' '}
                {typeof cfgQ.data?.derived_upper === 'number' ? cfgQ.data.derived_upper.toFixed(6) : '—'}
              </div>
              <div>
                <span className="text-foreground font-medium">derived initial capital</span>:{' '}
                {typeof cfgQ.data?.derived_initial_capital_usd === 'number'
                  ? formatUsdFixed(cfgQ.data.derived_initial_capital_usd, 4)
                  : '—'}
              </div>
              <div className="text-xs">
                <span className="text-foreground font-medium">open_session_id</span>:{' '}
                <span className="font-mono">{cfgQ.data?.open_session_id ?? '—'}</span>
              </div>
            </>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Sessions</CardTitle>
          <p className="text-sm text-muted-foreground font-normal">
            Grupowanie po <code className="text-[11px]">rebalance_session_id</code> (oraz <code className="text-[11px]">_no_session</code>).
          </p>
        </CardHeader>
        <CardContent className="space-y-4">
          {lifecycleQ.isPending ? (
            <div className="text-center py-8 text-muted-foreground">Loading...</div>
          ) : !data || data.session_summaries.length === 0 ? (
            <div className="text-center py-8 text-muted-foreground">No lifecycle sessions found.</div>
          ) : (
            data.session_summaries.map((s) => (
              <div key={s.session_id} className="border rounded-lg p-4 space-y-2">
                <div className="flex items-center justify-between gap-4 flex-wrap">
                  <div className="font-medium">
                    Session: <span className="font-mono">{s.session_id}</span>
                  </div>
                  <div className="text-sm text-muted-foreground">
                    fee {s.total_tx_fee_lamports} lamports • events {s.events.length}
                  </div>
                </div>
                <div className="overflow-x-auto">
                  <table className="w-full text-sm">
                    <thead>
                      <tr className="border-b text-left text-xs text-muted-foreground">
                        <th className="py-2 font-medium">ts</th>
                        <th className="py-2 font-medium">event</th>
                        <th className="py-2 font-medium">source</th>
                        <th className="py-2 font-medium">pos</th>
                        <th className="py-2 font-medium">sig</th>
                        <th className="py-2 font-medium text-right">fee</th>
                      </tr>
                    </thead>
                    <tbody>
                      {s.events.slice(-25).map((e, i) => (
                        <tr key={`${s.session_id}-${i}`} className="border-b last:border-0">
                          <td className="py-2 text-muted-foreground font-mono">
                            {(e.ts_utc ?? '—').slice(0, 19)}
                          </td>
                          <td className="py-2">{e.event ?? '—'}</td>
                          <td className="py-2 text-muted-foreground">{e.source ?? '—'}</td>
                          <td className="py-2 text-muted-foreground font-mono">
                            {e.position_pubkey ? shortenAddress(e.position_pubkey) : '—'}
                          </td>
                          <td className="py-2 text-muted-foreground font-mono">
                            {e.signature ? shortenAddress(e.signature) : '—'}
                          </td>
                          <td className="py-2 text-right text-muted-foreground font-mono">
                            {typeof e.tx_fee_lamports === 'number' ? e.tx_fee_lamports : '—'}
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
                {s.events.length > 25 ? (
                  <div className="text-xs text-muted-foreground">
                    Showing last 25 events for this session.
                  </div>
                ) : null}
              </div>
            ))
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="flex flex-row items-center justify-between gap-4">
          <div>
            <CardTitle>Backtest (from closed position)</CardTitle>
            <p className="text-sm text-muted-foreground font-normal">
              Uruchamia <code className="text-[11px]">clmm-lp-cli backtest</code> jako subprocess na hoście API (best-effort).
            </p>
          </div>
          <Button
            size="sm"
            onClick={() => runBacktestM.mutate()}
            disabled={runBacktestM.isPending || pos.length === 0}
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
              Job: <span className="font-mono">{backtestJobId}</span> {jobQ.data ? `(${jobQ.data.status})` : ''}
            </div>
          ) : (
            <div className="text-sm text-muted-foreground">No job yet.</div>
          )}
          {jobQ.data?.stderr ? (
            <pre className="text-xs whitespace-pre-wrap bg-muted p-3 rounded-md overflow-auto max-h-64">
{jobQ.data.stderr}
            </pre>
          ) : null}
          {jobQ.data?.stdout ? (
            <pre className="text-xs whitespace-pre-wrap bg-muted p-3 rounded-md overflow-auto max-h-64">
{jobQ.data.stdout}
            </pre>
          ) : null}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Stream context</CardTitle>
        </CardHeader>
        <CardContent className="text-sm text-muted-foreground space-y-1">
          <div>
            <span className="font-medium text-foreground">positions</span>: {data ? data.positions.length : '—'}
          </div>
          <div className="font-mono break-all">
            {(data?.positions ?? []).slice(0, 8).map((p) => shortenAddress(p)).join(', ')}
            {(data?.positions?.length ?? 0) > 8 ? ' …' : ''}
          </div>
        </CardContent>
      </Card>
    </div>
  )
}

