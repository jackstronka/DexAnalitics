import { useQuery } from '@tanstack/react-query'
import { Link, useParams } from 'react-router-dom'
import { ArrowLeft, RefreshCw } from 'lucide-react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { ErrorBanner } from '@/components/ui/error-banner'
import { PoolPairLabels } from '@/components/PoolPairLabels'
import {
  getBacktestJob,
  getJupiterPricesUsd,
  getPositionExperimentConfig,
  getPositionLifecycleSummary,
  getPositionLineagePreferMaterialized,
  runBacktestFromClosedPosition,
} from '@/lib/api'
import {
  FEE_BASE_UNITS_TOOLTIP,
  formatDate,
  formatFeeBaseUnitsClause,
  formatLineageFeesCollectedUsdMain,
  formatPercentFixed,
  formatPrincipalDeltaUsdOrDash,
  formatLineageStoredValueUsd,
  formatUsdField,
  formatUsdFixed,
  shortenAddress,
} from '@/lib/utils'
import { useMutation } from '@tanstack/react-query'
import { useMemo, useState } from 'react'
import { getMetricsMode } from '@/lib/metricsMode'
import { useI18n } from '@/lib/i18n'
import { isLineageFromPostgresMaterialized } from '@/lib/lineageReadSource'

function valuationQualityLabel(q?: string | null): string | null {
  const v = (q ?? '').trim().toLowerCase()
  if (!v) return null
  if (v === 'exact') return 'exact'
  if (v === 'fallback') return 'fallback'
  if (v === 'missing_inputs') return 'missing'
  return v
}

export default function ClosedPositionDetail() {
  const { locale, t } = useI18n()
  const L = (pl: string, en: string) => (locale === 'pl' ? pl : en)
  const { address } = useParams<{ address: string }>()
  const pos = (address ?? '').trim()
  const metricsMode = getMetricsMode()
  const isSettlementMode = metricsMode === 'settlement_v1'

  const lifecycleQ = useQuery({
    queryKey: ['position-lifecycle-summary', pos],
    queryFn: () => getPositionLifecycleSummary(pos),
    enabled: pos.length > 0,
    staleTime: 30_000,
    retry: 1,
  })

  const lineageQ = useQuery({
    queryKey: ['position-lineage', pos, metricsMode],
    queryFn: () => getPositionLineagePreferMaterialized(pos, metricsMode),
    enabled: pos.length > 0,
    staleTime: 30_000,
    retry: 0,
  })

  const data = lifecycleQ.data
  const streamLineage = lineageQ.data
  const lineageReadFromPostgres = useMemo(
    () => isLineageFromPostgresMaterialized(streamLineage),
    [streamLineage?.note],
  )
  const totals = streamLineage?.totals ?? null
  const totalsSourceBadge = (() => {
    const note = (totals?.note ?? '').toLowerCase()
    if (isSettlementMode || note.includes('settlement v1') || note.includes('self-seed disabled')) {
      return {
        label: 'source: persisted settlement',
        className: 'border-emerald-600/40 bg-emerald-500/10 text-emerald-300',
      }
    }
    if (note.includes('self-seed')) {
      return {
        label: 'source: live seeded',
        className: 'border-amber-600/40 bg-amber-500/10 text-amber-300',
      }
    }
    return {
      label: 'source: live snapshots',
      className: 'border-border/70 bg-background/70 text-muted-foreground',
    }
  })()
  const entryNode = streamLineage?.nodes.find((n) => n.position_address === pos)
  const chainCost = streamLineage?.chain_cost_summary
  const chainPriceMints = [
    entryNode?.token_mint_a?.trim(),
    entryNode?.token_mint_b?.trim(),
  ].filter((m): m is string => Boolean(m))

  const chainPricesQ = useQuery({
    queryKey: ['closed-chain-fee-prices', ...chainPriceMints],
    queryFn: () => getJupiterPricesUsd(chainPriceMints),
    enabled: chainPriceMints.length > 0,
    staleTime: 30_000,
    retry: 1,
  })
  const chainPrices = chainPricesQ.data ?? {}

  const formatLegUsd = (ui: string | number | null | undefined, mint?: string | null) => {
    const amount = ui == null ? NaN : parseFloat(String(ui))
    if (!Number.isFinite(amount)) return '—'
    const px = mint ? chainPrices[mint] : undefined
    if (!Number.isFinite(px)) return '—'
    return formatUsdFixed(amount * Number(px), 6)
  }

  const formatUsdCollectedOrDash = (usd: string | null | undefined, collects: number | null | undefined) => {
    const v = parseFloat(String(usd ?? '0'))
    const c = collects ?? 0
    if (!Number.isFinite(v)) return '—'
    // In JSONL-only mode we can have token deltas but no reliable USD valuation; avoid misleading "$0.0000".
    if (c > 0 && v === 0) return '—'
    return formatUsdFixed(v, 4)
  }

  const cfgQ = useQuery({
    queryKey: ['position-experiment-config', pos],
    queryFn: () => getPositionExperimentConfig(pos),
    enabled: pos.length > 0,
    staleTime: 30_000,
    retry: 0,
  })

  const [backtestJobId, setBacktestJobId] = useState<string | null>(null)
  const [showOnlyNonZeroBreakdown, setShowOnlyNonZeroBreakdown] = useState(true)

  const runBacktestM = useMutation({
    mutationFn: async () => {
      const raw = entryNode?.baseline_value_usd
      let capital: number | undefined
      if (raw != null && String(raw).trim() !== '') {
        const n = parseFloat(String(raw))
        if (Number.isFinite(n) && n > 0) capital = n
      }
      return await runBacktestFromClosedPosition({
        position_address: pos,
        strategy: 'static',
        fee_source: 'snapshots',
        price_path_source: 'snapshots',
        snapshot_protocol: 'orca',
        ...(capital != null ? { capital } : {}),
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
              {L('Wstecz', 'Back')}
            </Button>
          </Link>
          <div className="min-w-0">
            <h1 className="text-2xl font-bold truncate">{L('Zamknięta pozycja', 'Closed position')}</h1>
            <div className="text-xs text-muted-foreground">
              {L('Tryb metryk', 'Metrics mode')}:{' '}
              <span className="font-medium text-foreground">
                {isSettlementMode ? 'Settlement v1' : 'Live stream'}
              </span>
            </div>
            {entryNode?.token_a_label || entryNode?.token_b_label || entryNode?.token_mint_a || entryNode?.token_mint_b ? (
              <div className="mt-1">
                <PoolPairLabels
                  labelA={entryNode?.token_a_label}
                  labelB={entryNode?.token_b_label}
                  mintA={entryNode?.token_mint_a}
                  mintB={entryNode?.token_mint_b}
                  priceA={null}
                  priceB={null}
                />
              </div>
            ) : null}
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
          {L('Odśwież', 'Refresh')}
        </Button>
      </div>

      {lifecycleQ.isError ? (
        <Card>
          <CardHeader>
            <CardTitle>{L('Błąd', 'Error')}</CardTitle>
          </CardHeader>
          <CardContent className="text-sm text-muted-foreground">
            {lifecycleQ.error instanceof Error ? lifecycleQ.error.message : String(lifecycleQ.error)}
          </CardContent>
        </Card>
      ) : null}

      <div className="grid gap-4 md:grid-cols-2">
        <Card className="border-border/80">
          <CardHeader>
            <CardTitle className="text-base">{L('Koszty i prowizje — ten adres', 'Costs and fees — this address')}</CardTitle>
            <p className="text-sm text-muted-foreground font-normal">
              Tylko PDA tej strony. <span className="text-foreground/90">Sieć</span> = opłaty Solany za każdą transakcję
              (open, collect, close…). <span className="text-foreground/90">Fees zebrane</span> = prowizje puli
              best-effort z eventów collect + close.
            </p>
          </CardHeader>
          <CardContent className="space-y-3 text-sm">
            {entryNode ? (
              <>
                <div className="rounded-md border bg-muted/20 px-3 py-2">
                  <div className="text-xs text-muted-foreground uppercase tracking-wide">{L('Koszt sieci (tx)', 'Network cost (tx)')}</div>
                  <div className="font-mono text-lg mt-0.5">
                    {(entryNode.tx_fee_lamports ?? 0).toLocaleString()} lamports
                    <span className="text-muted-foreground"> · </span>
                    {formatUsdFixed(parseFloat(String(entryNode.tx_fees_usd)), 4)}
                  </div>
                </div>
                <div className="rounded-md border bg-muted/20 px-3 py-2">
                  <div className="text-xs text-muted-foreground uppercase tracking-wide">{L('Fees zebrane', 'Fees collected')}</div>
                  <div className="font-mono text-lg mt-0.5">
                    {formatUsdCollectedOrDash(entryNode.fees_collected_usd, entryNode.collect_events)}
                    <span className="text-muted-foreground text-sm font-sans">
                      {' '}
                      · {entryNode.collect_events ?? 0}× events
                    </span>
                  </div>
                  {(entryNode.fees_collected_token_a_ui != null ||
                    entryNode.fees_collected_token_b_ui != null ||
                    entryNode.fees_collected_token_a_raw != null ||
                    entryNode.fees_collected_token_b_raw != null) &&
                  (entryNode.token_a_label || entryNode.token_b_label) ? (
                    <div className="text-xs text-muted-foreground mt-1">
                      {entryNode.token_a_label ? (
                        <div>
                          {entryNode.token_a_label}:{' '}
                          <span className="font-mono text-foreground/90">
                            {String(entryNode.fees_collected_token_a_ui ?? '—')}
                          </span>
                          {formatFeeBaseUnitsClause(entryNode.fees_collected_token_a_raw) ? (
                            <span className="font-mono text-muted-foreground" title={FEE_BASE_UNITS_TOOLTIP}>
                              {' '}
                              {formatFeeBaseUnitsClause(entryNode.fees_collected_token_a_raw)}
                            </span>
                          ) : null}
                        </div>
                      ) : null}
                      {entryNode.token_b_label ? (
                        <div>
                          {entryNode.token_b_label}:{' '}
                          <span className="font-mono text-foreground/90">
                            {String(entryNode.fees_collected_token_b_ui ?? '—')}
                          </span>
                          {formatFeeBaseUnitsClause(entryNode.fees_collected_token_b_raw) ? (
                            <span className="font-mono text-muted-foreground" title={FEE_BASE_UNITS_TOOLTIP}>
                              {' '}
                              {formatFeeBaseUnitsClause(entryNode.fees_collected_token_b_raw)}
                            </span>
                          ) : null}
                        </div>
                      ) : null}
                    </div>
                  ) : null}
                </div>
              </>
            ) : (
              <p className="text-muted-foreground text-sm">
                {lineageQ.isPending ? 'Ładowanie lineage…' : 'Brak węzła lineage dla tego adresu.'}
              </p>
            )}
            {data?.collected_fee_token_a_ui != null ||
            data?.collected_fee_token_b_ui != null ||
            data?.collected_fee_token_a_raw != null ||
            data?.collected_fee_token_b_raw != null ? (
              <p className="text-xs text-muted-foreground">
                Tokeny (lifecycle summary): A {String(data?.collected_fee_token_a_ui ?? '—')}
                {formatFeeBaseUnitsClause(data?.collected_fee_token_a_raw) ? (
                  <span title={FEE_BASE_UNITS_TOOLTIP}>
                    {' '}
                    {formatFeeBaseUnitsClause(data?.collected_fee_token_a_raw)}
                  </span>
                ) : null}{' '}
                · B {String(data?.collected_fee_token_b_ui ?? '—')}
                {formatFeeBaseUnitsClause(data?.collected_fee_token_b_raw) ? (
                  <span title={FEE_BASE_UNITS_TOOLTIP}>
                    {' '}
                    {formatFeeBaseUnitsClause(data?.collected_fee_token_b_raw)}
                  </span>
                ) : null}
              </p>
            ) : null}
          </CardContent>
        </Card>

        <Card className="border-border/80">
          <CardHeader>
            <CardTitle className="text-base">{L('Koszty i prowizje — cały łańcuch', 'Costs and fees — full chain')}</CardTitle>
            <p className="text-sm text-muted-foreground font-normal">
              Suma po wszystkich PDA w rotacji (ten sam łańcuch co w tabeli poniżej).
            </p>
          </CardHeader>
          <CardContent className="space-y-3 text-sm">
            {chainCost ? (
              <>
                <div className="flex justify-end">
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => setShowOnlyNonZeroBreakdown((v) => !v)}
                  >
                    {showOnlyNonZeroBreakdown ? L('Pokaż wszystkie pozycje', 'Show all positions') : L('Pokaż tylko niezerowe', 'Show non-zero only')}
                  </Button>
                </div>
                <div className="rounded-md border bg-muted/20 px-3 py-2">
                  <div className="text-xs text-muted-foreground uppercase tracking-wide">{L('Koszt sieci (tx) — suma', 'Network cost (tx) — total')}</div>
                  <div className="font-mono text-lg mt-0.5">
                    {chainCost.tx_fee_lamports_total.toLocaleString()} lamports
                    <span className="text-muted-foreground"> · </span>
                    {formatUsdFixed(parseFloat(chainCost.tx_fees_usd_total), 4)}
                  </div>
                  {streamLineage?.nodes?.length ? (
                    <div className="mt-2 space-y-1 text-xs text-muted-foreground">
                      {streamLineage.nodes.map((n) => {
                        const lam = n.tx_fee_lamports ?? 0
                        if (showOnlyNonZeroBreakdown && lam <= 0) return null
                        return (
                          <div key={`tx-${n.position_address}`} className="font-mono">
                            {shortenAddress(n.position_address, 6)}: {lam.toLocaleString()} lamports ·{' '}
                            {formatUsdFixed(parseFloat(String(n.tx_fees_usd ?? '0')), 4)}
                          </div>
                        )
                      })}
                    </div>
                  ) : null}
                </div>
                <div className="rounded-md border bg-muted/20 px-3 py-2">
                  <div className="text-xs text-muted-foreground uppercase tracking-wide">{L('Fees zebrane — suma', 'Fees collected — total')}</div>
                  <div className="font-mono text-lg mt-0.5">
                    {formatUsdCollectedOrDash(chainCost.fees_collected_usd_total, chainCost.collect_events_total)}
                    <span className="text-muted-foreground text-sm font-sans">
                      {' '}
                      · {chainCost.collect_events_total}× events (łącznie)
                    </span>
                  </div>
                  {(chainCost.fees_collected_token_a_ui_total != null ||
                    chainCost.fees_collected_token_b_ui_total != null ||
                    chainCost.fees_collected_token_a_raw_total != null ||
                    chainCost.fees_collected_token_b_raw_total != null) &&
                  (entryNode?.token_a_label || entryNode?.token_b_label) ? (
                    <div className="text-xs text-muted-foreground mt-1">
                      {entryNode?.token_a_label ? (
                        <div>
                          {entryNode.token_a_label}:{' '}
                          <span className="font-mono text-foreground/90">
                            {String(chainCost.fees_collected_token_a_ui_total ?? '—')}
                          </span>
                          <span className="font-mono text-muted-foreground">
                            {' '}
                            (≈{' '}
                            {formatLegUsd(
                              chainCost.fees_collected_token_a_ui_total,
                              entryNode.token_mint_a,
                            )}
                            )
                          </span>
                          {formatFeeBaseUnitsClause(chainCost.fees_collected_token_a_raw_total) ? (
                            <span className="font-mono text-muted-foreground" title={FEE_BASE_UNITS_TOOLTIP}>
                              {' '}
                              {formatFeeBaseUnitsClause(chainCost.fees_collected_token_a_raw_total)}
                            </span>
                          ) : null}
                        </div>
                      ) : null}
                      {entryNode?.token_b_label ? (
                        <div>
                          {entryNode.token_b_label}:{' '}
                          <span className="font-mono text-foreground/90">
                            {String(chainCost.fees_collected_token_b_ui_total ?? '—')}
                          </span>
                          <span className="font-mono text-muted-foreground">
                            {' '}
                            (≈{' '}
                            {formatLegUsd(
                              chainCost.fees_collected_token_b_ui_total,
                              entryNode.token_mint_b,
                            )}
                            )
                          </span>
                          {formatFeeBaseUnitsClause(chainCost.fees_collected_token_b_raw_total) ? (
                            <span className="font-mono text-muted-foreground" title={FEE_BASE_UNITS_TOOLTIP}>
                              {' '}
                              {formatFeeBaseUnitsClause(chainCost.fees_collected_token_b_raw_total)}
                            </span>
                          ) : null}
                        </div>
                      ) : null}
                    </div>
                  ) : null}
                  {streamLineage?.nodes?.length ? (
                    <div className="mt-2 space-y-1 text-xs text-muted-foreground">
                      {streamLineage.nodes.map((n) => {
                        const collects = n.collect_events ?? 0
                        const usdMain = formatLineageFeesCollectedUsdMain(n.fees_collected_usd, collects)
                        const hasA =
                          n.fees_collected_token_a_ui != null || n.fees_collected_token_a_raw != null
                        const hasB =
                          n.fees_collected_token_b_ui != null || n.fees_collected_token_b_raw != null
                        if (showOnlyNonZeroBreakdown && collects <= 0 && !hasA && !hasB) return null
                        return (
                          <div key={`fee-${n.position_address}`} className="space-y-0.5">
                            <div className="font-mono">
                              {shortenAddress(n.position_address, 6)}: {usdMain} · {collects}x collect
                            </div>
                            {(n.token_a_label || n.token_b_label) && (hasA || hasB) ? (
                              <div className="pl-3 font-mono">
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
                          </div>
                        )
                      })}
                    </div>
                  ) : null}
                </div>
              </>
            ) : (
              <p className="text-muted-foreground text-sm">
                {lineageQ.isPending ? L('Ładowanie…', 'Loading…') : L('Brak podsumowania łańcucha.', 'No chain summary.')}
              </p>
            )}
          </CardContent>
        </Card>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>
            {isSettlementMode
              ? L('Settlement v1 — dwie definicje (nie mylić)', 'Settlement v1 — two definitions (do not mix)')
              : L('Stream — dwie definicje (nie mylić)', 'Stream — two definitions (do not mix)')}
          </CardTitle>
          <p className="text-sm text-muted-foreground font-normal">
            Wynik ekonomiczny vs benchmark IL/HODL — osobno od karty „koszty sieci vs LP fee” powyżej.
          </p>
        </CardHeader>
        <CardContent className="text-sm text-muted-foreground space-y-4">
          {totals ? (
            <>
              <div className="rounded-md border border-border/60 bg-muted/15 px-3 py-2 space-y-2">
                <div className="text-xs font-medium text-foreground">1) Wynik ekonomiczny łańcucha (net PnL)</div>
                <div
                  className={`inline-flex w-fit rounded-full border px-2 py-0.5 text-[10px] ${totalsSourceBadge.className}`}
                >
                  {totalsSourceBadge.label}
                </div>
                {totals.interpretation?.economic_net_pnl_caption_pl ? (
                  <p className="text-[11px] leading-snug">{totals.interpretation.economic_net_pnl_caption_pl}</p>
                ) : null}
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
                  <div>
                    <span className="text-muted-foreground">tx fees (stream)</span>{' '}
                    <span className="font-mono">{formatUsdFixed(parseFloat(totals.tx_fees_usd), 4)}</span>
                  </div>
                </div>
                <div className="pt-0.5">
                  <span className="text-muted-foreground">net PnL</span>{' '}
                  <span className="font-mono">{formatUsdFixed(parseFloat(totals.net_pnl_usd), 3)}</span>
                  <span className="font-mono">
                    {' '}
                    ({(parseFloat(totals.net_pnl_pct) * 100).toFixed(3)}%)
                  </span>
                </div>
              </div>

              <div className="rounded-md border border-border/60 bg-muted/15 px-3 py-2 space-y-2">
                <div className="text-xs font-medium text-foreground">2) Benchmark IL vs HODL (start łańcucha)</div>
                {totals.interpretation?.il_vs_initial_hodl_caption_pl ? (
                  <p className="text-[11px] leading-snug">{totals.interpretation.il_vs_initial_hodl_caption_pl}</p>
                ) : null}
                <div className="flex flex-wrap gap-x-6 gap-y-1">
                  <div>
                    <span className="text-muted-foreground">HODL (koszyk startowy × ceny)</span>{' '}
                    <span className="font-mono">{formatUsdFixed(parseFloat(totals.hodl_value_usd), 3)}</span>
                  </div>
                  <div>
                    <span className="text-muted-foreground">IL USD</span>{' '}
                    <span className="font-mono">{formatUsdFixed(parseFloat(totals.il_usd), 3)}</span>
                  </div>
                  <div>
                    <span className="text-muted-foreground">IL %</span>{' '}
                    <span className="font-mono">{(parseFloat(totals.il_pct) * 100).toFixed(3)}%</span>
                  </div>
                </div>
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
            <CardTitle>{t('positionDetail.positionHistory')}</CardTitle>
          </CardHeader>
          <CardContent className="text-sm text-muted-foreground">{L('Ładowanie lineage…', 'Loading lineage…')}</CardContent>
        </Card>
      ) : streamLineage ? (
        <Card>
          <CardHeader className="space-y-2">
            <div className="flex flex-wrap items-start justify-between gap-2">
              <CardTitle className="mb-0">{t('positionDetail.positionHistory')}</CardTitle>
              <span
                className={
                  lineageReadFromPostgres
                    ? 'inline-flex shrink-0 rounded-full border border-sky-600/35 bg-sky-500/10 px-2 py-0.5 text-[10px] text-sky-200'
                    : 'inline-flex shrink-0 rounded-full border border-amber-600/35 bg-amber-500/10 px-2 py-0.5 text-[10px] text-amber-200'
                }
                title={t('positionDetail.lineageHistoryApiIntro')}
              >
                {lineageReadFromPostgres
                  ? t('positionDetail.lineageReadBadgePostgres')
                  : t('positionDetail.lineageReadBadgeCompute')}
              </span>
            </div>
            <p className="text-[11px] text-muted-foreground font-normal leading-snug">{t('positionDetail.lineageHistoryApiIntro')}</p>
            <p className="text-sm text-muted-foreground font-normal">
              {locale === 'pl' ? (
                <>
                  Łańcuch PDA (stara → nowa): semantyka jak w stream-lineage (CLI:{' '}
                  <code className="text-[11px]">position_open</code> / <code className="text-[11px]">position_close</code>; bot:{' '}
                  <code className="text-[11px]">bot_*</code>; oba źródła są łączone).
                </>
              ) : (
                <>
                  PDA chain (old → new): same semantics as stream-lineage (CLI:{' '}
                  <code className="text-[11px]">position_open</code> / <code className="text-[11px]">position_close</code>; bot:{' '}
                  <code className="text-[11px]">bot_*</code>; both sources merged).
                </>
              )}
            </p>
            <p className="text-[11px] text-muted-foreground font-normal leading-snug">
              {locale === 'pl' ? (
                <>
                  Kolumna <span className="font-medium">Fees zebrane</span> to best-effort suma prowizji z eventów collect + close.
                  Dla części transakcji Orca/RPC w <code className="text-[10px]">fee_payer_token_deltas</code> bywa widoczna tylko jedna
                  noga mintu, więc breakdown tokenowy może być niepełny.
                </>
              ) : (
                <>
                  <span className="font-medium">Fees collected</span> is a best-effort sum from collect + close events. For some Orca/RPC
                  txs in <code className="text-[10px]">fee_payer_token_deltas</code> only one mint leg may appear, so token breakdown can be incomplete.
                </>
              )}
            </p>
          </CardHeader>
          <CardContent className="space-y-3">
            {streamLineage.note ? (
              <p className="text-[11px] text-muted-foreground leading-snug">{streamLineage.note}</p>
            ) : null}
            {lineageQ.isError ? (
              <ErrorBanner>
                {lineageQ.error instanceof Error ? lineageQ.error.message : String(lineageQ.error)}
              </ErrorBanner>
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
                      <th className="px-2 py-1 text-left">start value</th>
                      <th className="px-2 py-1 text-left">end value</th>
                      <th className="px-2 py-1 text-left">principal Δ</th>
                      <th className="px-2 py-1 text-left">Sieć (tx)</th>
                      <th className="px-2 py-1 text-left">Fees zebrane</th>
                      <th className="px-2 py-1 text-left">net PnL</th>
                    </tr>
                  </thead>
                  <tbody>
                    {streamLineage.nodes.map((n, i) => (
                      <tr key={n.position_address} className="border-t border-border/60">
                        <td className="px-2 py-1 font-mono tabular-nums">{i + 1}</td>
                        <td className="px-2 py-1 font-mono text-[11px] align-top break-all min-w-[12rem] max-w-[28rem]">
                          <Link
                            to={
                              n.closed_ts_utc
                                ? `/positions/closed/${n.position_address}`
                                : `/positions/${n.position_address}`
                            }
                            className="text-primary hover:underline break-all"
                          >
                            {n.position_address}
                          </Link>
                        </td>
                        <td className="px-2 py-1 whitespace-nowrap">
                          {n.opened_ts_utc ? formatDate(n.opened_ts_utc) : '—'}
                        </td>
                        <td className="px-2 py-1 whitespace-nowrap">
                          {n.closed_ts_utc ? formatDate(n.closed_ts_utc) : '—'}
                        </td>
                        <td className="px-2 py-1 whitespace-nowrap font-mono">
                          {formatLineageStoredValueUsd(n.baseline_value_usd, n.baseline_valuation_quality, 3)}
                          {valuationQualityLabel(n.baseline_valuation_quality) ? (
                            <span className="ml-1 rounded border border-border/60 px-1 py-0 text-[10px] text-muted-foreground">
                              {valuationQualityLabel(n.baseline_valuation_quality)}
                            </span>
                          ) : null}
                        </td>
                        <td className="px-2 py-1 whitespace-nowrap font-mono">
                          {formatLineageStoredValueUsd(n.current_value_usd, n.current_valuation_quality, 3)}
                          {valuationQualityLabel(n.current_valuation_quality) ? (
                            <span className="ml-1 rounded border border-border/60 px-1 py-0 text-[10px] text-muted-foreground">
                              {valuationQualityLabel(n.current_valuation_quality)}
                            </span>
                          ) : null}
                        </td>
                        <td className="px-2 py-1 whitespace-nowrap font-mono">
                          {formatPrincipalDeltaUsdOrDash(
                            n.baseline_value_usd,
                            n.baseline_valuation_quality,
                            n.current_value_usd,
                            n.current_valuation_quality,
                            3,
                          )}
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
                                <span className="text-muted-foreground"> · {collects}× events</span>
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
                              </>
                            )
                          })()}
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
              <p className="text-sm text-muted-foreground">
                Jedna pozycja w łańcuchu — brak powiązanych PDA w ledgerze albo brak pasujących zdarzeń open/close.
              </p>
            )}
          </CardContent>
        </Card>
      ) : (
        <Card>
          <CardHeader>
            <CardTitle>{t('positionDetail.positionHistory')}</CardTitle>
          </CardHeader>
          <CardContent className="text-sm text-muted-foreground">
            {lineageQ.isError ? (
              <ErrorBanner as="span" className="inline-block px-2 py-1 text-xs">
                {lineageQ.error instanceof Error ? lineageQ.error.message : String(lineageQ.error)}
              </ErrorBanner>
            ) : (
              'No lineage response.'
            )}
          </CardContent>
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
            <div>{L('Ładowanie...', 'Loading...')}</div>
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
            <div className="text-center py-8 text-muted-foreground">{L('Ładowanie...', 'Loading...')}</div>
          ) : !data || data.session_summaries.length === 0 ? (
            <div className="text-center py-8 text-muted-foreground">{L('Brak sesji lifecycle.', 'No lifecycle sessions found.')}</div>
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
            <CardTitle>{L('Backtest (z zamkniętej pozycji)', 'Backtest (from closed position)')}</CardTitle>
            <p className="text-sm text-muted-foreground font-normal">
              Uruchamia <code className="text-[11px]">clmm-lp-cli backtest</code> jako subprocess na hoście API (best-effort).
              Gdy stream-lineage ma <span className="text-foreground/90">baseline USD</span> dla tego PDA, jest wysyłany jako{' '}
              <code className="text-[11px]">capital</code> — inaczej API próbuje ledgera / snapshotu DB. Na hoście musi być dostępny{' '}
              <code className="text-[11px]">clmm-lp-cli</code> (PATH,{' '}
              <code className="text-[11px]">CLMM_LP_CLI_PATH</code>, ten sam <code className="text-[11px]">target/</code> co API lub{' '}
              <code className="text-[11px]">CLMM_REPO_ROOT</code> + <code className="text-[11px]">CLMM_API_TARGET_DIR</code>).
            </p>
          </div>
          <Button
            size="sm"
            onClick={() => runBacktestM.mutate()}
            disabled={runBacktestM.isPending || pos.length === 0}
          >
            {L('Uruchom backtest', 'Run backtest')}
          </Button>
        </CardHeader>
        <CardContent className="space-y-3">
          {runBacktestM.isError && (runBacktestM.error as Error)?.message !== 'Cancelled' ? (
            <ErrorBanner>
              {(runBacktestM.error as Error)?.message ?? L('Backtest nieudany', 'Backtest failed')}
            </ErrorBanner>
          ) : null}
          {backtestJobId ? (
            <div className="text-sm text-muted-foreground">
              Job: <span className="font-mono">{backtestJobId}</span> {jobQ.data ? `(${jobQ.data.status})` : ''}
            </div>
          ) : (
            <div className="text-sm text-muted-foreground">{L('Brak joba.', 'No job yet.')}</div>
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
          <CardTitle>{L('Kontekst streamu', 'Stream context')}</CardTitle>
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

