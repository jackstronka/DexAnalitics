import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Link } from 'react-router-dom'
import { CheckCircle2, AlertTriangle, HelpCircle, RefreshCw } from 'lucide-react'
import { useMemo, useState } from 'react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { ErrorBanner } from '@/components/ui/error-banner'
import {
  getWalletSessionBalances,
  postWalletReconcileSessionGl,
  postWalletSessionBalancesBackfill,
  type WalletSessionBalanceRow,
  type WalletSessionGlReconcileResponse,
  type WalletSessionMetrics,
} from '@/lib/api'
import { useI18n } from '@/lib/i18n'
import { shortenAddress } from '@/lib/utils'

const WSOL = 'So11111111111111111111111111111111111111112'
const USDC = 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v'

type SourceKind = 'gl' | 'pslr_fallback' | 'empty' | 'disabled' | 'no_db' | 'unknown'

function parseSource(source: string): SourceKind {
  if (source === 'gl_session_shadow') return 'gl'
  if (source === 'gl_session_shadow_pslr_fallback') return 'pslr_fallback'
  if (source === 'gl_session_shadow_empty') return 'empty'
  if (source === 'gl_session_shadow_disabled') return 'disabled'
  if (source === 'gl_session_shadow_no_db') return 'no_db'
  return 'unknown'
}

function defaultDecimals(mint: string): number {
  if (mint === WSOL) return 9
  if (mint === USDC) return 6
  return 9
}

function mintSymbol(mint: string): string {
  if (mint === WSOL) return 'SOL'
  if (mint === USDC) return 'USDC'
  return shortenAddress(mint, 4)
}

export function formatRawAmount(raw: string, decimals?: number | null): string {
  const dec = decimals ?? 9
  try {
    const n = BigInt(raw.trim())
    const neg = n < 0n
    const abs = neg ? -n : n
    const base = 10n ** BigInt(dec)
    const whole = abs / base
    const frac = abs % base
    const fracStr = frac.toString().padStart(dec, '0').replace(/0+$/, '')
    const ui = fracStr ? `${whole}.${fracStr}` : whole.toString()
    return neg ? `-${ui}` : ui
  } catch {
    return raw
  }
}

function formatBalanceRow(row: WalletSessionBalanceRow): string {
  const sym = mintSymbol(row.mint)
  const ui = formatRawAmount(row.amount_raw, row.decimals ?? defaultDecimals(row.mint))
  return `${ui} ${sym}`
}

function formatUsdMetric(v: string | null | undefined): string {
  if (v == null || v.trim() === '') return '—'
  const n = parseFloat(v)
  if (!Number.isFinite(n)) return '—'
  return `$${n.toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`
}

function formatSignedUsd(v: string | null | undefined): string {
  if (v == null || v.trim() === '') return '—'
  const n = parseFloat(v)
  if (!Number.isFinite(n)) return '—'
  const abs = `$${Math.abs(n).toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`
  if (n > 0) return `+${abs}`
  if (n < 0) return `−${abs}`
  return abs
}

function humanUsdSource(source: string, locale: 'pl' | 'en'): string {
  if (source === 'open_quote_estimated_value_usd') {
    return locale === 'pl' ? 'szacunek quote open' : 'open quote estimate'
  }
  if (source === 'open_target_usd') return locale === 'pl' ? 'target open' : 'open target'
  if (source === 'open_prev_end_value_usd') {
    return locale === 'pl' ? 'prev end (reopen)' : 'prev end (reopen)'
  }
  if (source === 'computed_event_prices') {
    return locale === 'pl' ? 'event_price × kwoty' : 'event_price × amounts'
  }
  return source
}

function CompactBalanceList({ rows, label }: { rows: WalletSessionBalanceRow[]; label: string }) {
  if (!rows.length) return null
  return (
    <div>
      <div className="text-[11px] text-muted-foreground mb-1">{label}</div>
      <ul className="space-y-0.5 text-xs tabular-nums">
        {rows.map((row) => (
          <li key={`${label}-${row.mint}`}>
            <span className="font-medium">{mintSymbol(row.mint)}</span>{' '}
            {formatRawAmount(row.amount_raw, row.decimals ?? defaultDecimals(row.mint))}
          </li>
        ))}
      </ul>
    </div>
  )
}

function SessionMetricsPanel({ metrics }: { metrics: WalletSessionMetrics }) {
  const { t, locale } = useI18n()
  const open = metrics.open_start
  const hasPreOpen = open.pre_open_balances.length > 0
  const showUntrusted =
    metrics.metrics_trusted === false || open.mint_resolution === 'incomplete'

  return (
    <div className="rounded-md border border-primary/25 bg-primary/5 px-3 py-3 space-y-3 text-sm">
      <div>
        <p className="font-medium">{t('sessionBalances.metricsTitle')}</p>
        <p className="text-xs text-muted-foreground mt-1">{t('sessionBalances.metricsExplain')}</p>
      </div>
      {showUntrusted ? (
        <div className="rounded-md border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-xs text-amber-900 dark:text-amber-100 flex gap-2">
          <AlertTriangle className="h-4 w-4 shrink-0 mt-0.5" aria-hidden />
          <span>{t('sessionBalances.metricsUntrusted')}</span>
        </div>
      ) : null}
      <div className="grid gap-3 sm:grid-cols-2 text-xs">
        <div className="space-y-2">
          <div className="text-muted-foreground">
            {t('sessionBalances.metricsOpenAt')}
            {open.ts_utc ? `: ${open.ts_utc}` : ''}
          </div>
          {open.position_pubkey ? (
            <div className="font-mono text-[10px] text-muted-foreground break-all" title={open.position_pubkey}>
              {shortenAddress(open.position_pubkey, 6)}
            </div>
          ) : null}
          <CompactBalanceList rows={open.deployed_balances} label={t('sessionBalances.metricsDeployed')} />
          {hasPreOpen ? (
            <CompactBalanceList rows={open.pre_open_balances} label={t('sessionBalances.metricsPreOpen')} />
          ) : null}
        </div>
        <div className="space-y-1.5 tabular-nums">
          <div className="flex justify-between gap-2">
            <span className="text-muted-foreground">{t('sessionBalances.metricsDeployed')}</span>
            <span className="font-medium">{formatUsdMetric(open.value_usd)}</span>
          </div>
          {hasPreOpen ? (
            <div className="flex justify-between gap-2">
              <span className="text-muted-foreground">{t('sessionBalances.metricsPreOpen')}</span>
              <span>{formatUsdMetric(open.pre_open_value_usd)}</span>
            </div>
          ) : null}
          <div className="flex justify-between gap-2 border-t border-border/50 pt-1.5">
            <span className="text-muted-foreground">{t('sessionBalances.metricsCurrentUsd')}</span>
            <span className="font-medium">{formatUsdMetric(metrics.current_value_usd)}</span>
          </div>
          {metrics.delta_vs_pre_open_usd != null && metrics.delta_vs_pre_open_usd !== '' ? (
            <div className="flex justify-between gap-2">
              <span className="text-muted-foreground">{t('sessionBalances.metricsDeltaPreOpen')}</span>
              <span>{formatSignedUsd(metrics.delta_vs_pre_open_usd)}</span>
            </div>
          ) : null}
          <div className="text-[10px] text-muted-foreground pt-1">
            {t('sessionBalances.metricsUsdSource')}: {humanUsdSource(open.value_usd_source, locale)}
          </div>
        </div>
      </div>
    </div>
  )
}

function SourceBanner({ kind }: { kind: SourceKind }) {
  const { t } = useI18n()
  const msg = (() => {
    switch (kind) {
      case 'gl':
        return t('sessionBalances.sourceGl')
      case 'pslr_fallback':
        return t('sessionBalances.sourcePslrFallback')
      case 'empty':
        return t('sessionBalances.sourceEmpty')
      case 'disabled':
        return t('sessionBalances.sourceDisabled')
      case 'no_db':
        return t('sessionBalances.sourceNoDb')
      default:
        return null
    }
  })()
  if (!msg) return null
  const tone =
    kind === 'gl'
      ? 'border-emerald-500/40 bg-emerald-500/10 text-emerald-800 dark:text-emerald-200'
      : kind === 'pslr_fallback'
        ? 'border-amber-500/40 bg-amber-500/10 text-amber-900 dark:text-amber-100'
        : 'border-border bg-muted/30 text-muted-foreground'
  const Icon = kind === 'gl' ? CheckCircle2 : kind === 'pslr_fallback' ? AlertTriangle : HelpCircle
  return (
    <div className={`rounded-md border px-3 py-2 text-sm flex gap-2 ${tone}`}>
      <Icon className="h-4 w-4 shrink-0 mt-0.5" aria-hidden />
      <span>{msg}</span>
    </div>
  )
}

function BalancesTable({
  balances,
  showRaw,
}: {
  balances: WalletSessionBalanceRow[]
  showRaw: boolean
}) {
  const { t } = useI18n()
  return (
    <div className="overflow-x-auto rounded-md border">
      <table className="w-full text-left text-sm">
        <thead className="bg-muted/50">
          <tr>
            <th className="px-3 py-2 font-medium">{t('sessionBalances.colToken')}</th>
            <th className="px-3 py-2 font-medium">{t('sessionBalances.colAmount')}</th>
            {showRaw ? (
              <th className="px-3 py-2 font-medium font-mono text-xs">{t('sessionBalances.colRaw')}</th>
            ) : null}
          </tr>
        </thead>
        <tbody>
          {balances.map((row) => (
            <tr key={row.mint} className="border-t border-border/60">
              <td className="px-3 py-2">
                <span className="font-medium">{mintSymbol(row.mint)}</span>
                <span className="block font-mono text-[10px] text-muted-foreground" title={row.mint}>
                  {row.mint.length > 14 ? shortenAddress(row.mint, 6) : row.mint}
                </span>
              </td>
              <td className="px-3 py-2 tabular-nums">
                {formatRawAmount(row.amount_raw, row.decimals ?? defaultDecimals(row.mint))}
              </td>
              {showRaw ? (
                <td className="px-3 py-2 font-mono text-xs text-muted-foreground">{row.amount_raw}</td>
              ) : null}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

type CompareRow = {
  mint: string
  gl?: string | null
  pslr?: string | null
  lastClose?: string | null
  match: boolean
}

function buildCompareRows(data: WalletSessionGlReconcileResponse): CompareRow[] {
  const mints = new Set<string>()
  for (const b of data.gl_balances) mints.add(b.mint)
  for (const b of data.pslr_balances) mints.add(b.mint)
  for (const g of data.gaps) mints.add(g.mint)
  return [...mints].sort().map((mint) => {
    const gap = data.gaps.find((g) => g.mint === mint)
    const gl = gap?.gl_amount_raw ?? data.gl_balances.find((b) => b.mint === mint)?.amount_raw
    const pslr = gap?.pslr_amount_raw ?? data.pslr_balances.find((b) => b.mint === mint)?.amount_raw
    const lastClose = gap?.last_close_returned_raw
    const match = gl != null && pslr != null && gl === pslr
    return { mint, gl, pslr, lastClose, match }
  })
}

function ReconcilePanel({ data }: { data: WalletSessionGlReconcileResponse }) {
  const { t } = useI18n()
  const rows = useMemo(() => buildCompareRows(data), [data])
  const hasLastClose = rows.some((r) => r.lastClose != null && r.lastClose !== '')

  return (
    <div className="mt-3 rounded-md border px-3 py-3 text-sm space-y-3">
      <div>
        <p className="font-medium">
          {t('sessionBalances.reconcileTitle')}:{' '}
          <span className={data.gl_matches_pslr ? 'text-emerald-600' : 'text-amber-600'}>
            {data.gl_matches_pslr ? t('sessionBalances.reconcileOk') : t('sessionBalances.reconcileGap')}
          </span>
        </p>
        <p className="text-xs text-muted-foreground mt-1">{t('sessionBalances.reconcileExplain')}</p>
      </div>
      <div className="overflow-x-auto rounded-md border">
        <table className="w-full text-left text-xs">
          <thead className="bg-muted/50">
            <tr>
              <th className="px-2 py-1.5 font-medium">{t('sessionBalances.colToken')}</th>
              <th className="px-2 py-1.5 font-medium">{t('sessionBalances.colGl')}</th>
              <th className="px-2 py-1.5 font-medium">{t('sessionBalances.colPslr')}</th>
              {hasLastClose ? (
                <th className="px-2 py-1.5 font-medium text-muted-foreground">
                  {t('sessionBalances.colLastClose')}
                </th>
              ) : null}
              <th className="px-2 py-1.5 font-medium">{t('sessionBalances.colMatch')}</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((r) => (
              <tr key={r.mint} className="border-t border-border/60">
                <td className="px-2 py-1.5 font-medium">{mintSymbol(r.mint)}</td>
                <td className="px-2 py-1.5 tabular-nums">
                  {r.gl != null && r.gl !== ''
                    ? formatBalanceRow({ mint: r.mint, amount_raw: r.gl })
                    : `— (${t('sessionBalances.missingInGl')})`}
                </td>
                <td className="px-2 py-1.5 tabular-nums">
                  {r.pslr != null && r.pslr !== ''
                    ? formatBalanceRow({ mint: r.mint, amount_raw: r.pslr })
                    : '—'}
                </td>
                {hasLastClose ? (
                  <td className="px-2 py-1.5 tabular-nums text-muted-foreground">
                    {r.lastClose != null && r.lastClose !== ''
                      ? formatBalanceRow({ mint: r.mint, amount_raw: r.lastClose })
                      : '—'}
                  </td>
                ) : null}
                <td className="px-2 py-1.5">
                  {r.match ? (
                    <span className="text-emerald-600">{t('sessionBalances.matchOk')}</span>
                  ) : (
                    <span className="text-amber-600">{t('sessionBalances.matchGap')}</span>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {hasLastClose ? (
        <p className="text-[11px] text-muted-foreground">{t('sessionBalances.lastCloseFootnote')}</p>
      ) : null}
    </div>
  )
}

export type SessionBalancesPanelProps = {
  sessionId: string
  owner?: string
  /** Hide outer Card wrapper (embed in parent Card). */
  embedded?: boolean
  className?: string
}

export function SessionBalancesPanel({ sessionId, owner, embedded, className }: SessionBalancesPanelProps) {
  const { t } = useI18n()
  const qc = useQueryClient()
  const [showRaw, setShowRaw] = useState(false)
  const sid = sessionId.trim()
  const q = useQuery({
    queryKey: ['wallet-session-balances', sid, owner?.trim() ?? ''],
    queryFn: () =>
      getWalletSessionBalances({
        session_id: sid,
        owner: owner?.trim() || undefined,
      }),
    enabled: sid.length > 0,
    staleTime: 15_000,
    refetchInterval: 30_000,
  })

  const sourceKind = q.data ? parseSource(q.data.source) : 'unknown'

  const reconcileM = useMutation({
    mutationFn: () =>
      postWalletReconcileSessionGl({
        session_id: sid,
        owner: owner?.trim() || undefined,
      }),
  })

  const backfillM = useMutation({
    mutationFn: () =>
      postWalletSessionBalancesBackfill({
        session_id: sid,
        limit: 1,
      }),
    onSuccess: async (report) => {
      await qc.invalidateQueries({ queryKey: ['wallet-session-balances', sid] })
      if (report.postings_applied > 0) {
        reconcileM.mutate()
      }
    },
  })

  const backfillStatus = backfillM.data ? (
    backfillM.data.postings_applied > 0 ? (
      <span className="text-xs text-emerald-700 dark:text-emerald-300 self-center">
        {t('sessionBalances.backfillDone')
          .replace('{applied}', String(backfillM.data.postings_applied))
          .replace('{skipped}', String(backfillM.data.rows_skipped_already))}
      </span>
    ) : (
      <span className="text-xs text-muted-foreground self-center">{t('sessionBalances.backfillNone')}</span>
    )
  ) : null

  const actionRow = (
    <div className="flex flex-wrap items-center gap-2 mb-3">
      <Button
        type="button"
        variant="outline"
        size="sm"
        disabled={!sid || backfillM.isPending}
        onClick={() => backfillM.mutate()}
        title={t('sessionBalances.backfillTitle')}
      >
        {backfillM.isPending ? t('sessionBalances.backfilling') : t('sessionBalances.backfill')}
      </Button>
      <Button
        type="button"
        variant="outline"
        size="sm"
        disabled={!sid || reconcileM.isPending}
        onClick={() => reconcileM.mutate()}
      >
        {reconcileM.isPending ? t('sessionBalances.reconciling') : t('sessionBalances.reconcile')}
      </Button>
      {backfillStatus}
    </div>
  )

  const inner = (
    <div className="space-y-3">
      <p className="text-xs text-muted-foreground leading-relaxed">{t('sessionBalances.whatIsThis')}</p>
      {q.data ? <SourceBanner kind={sourceKind} /> : null}
      {actionRow}
      {backfillM.error ? <ErrorBanner>{(backfillM.error as Error).message}</ErrorBanner> : null}
      {reconcileM.error ? <ErrorBanner>{(reconcileM.error as Error).message}</ErrorBanner> : null}
      {reconcileM.data ? <ReconcilePanel data={reconcileM.data} /> : null}
      {q.error ? <ErrorBanner>{(q.error as Error).message}</ErrorBanner> : null}
      {q.isLoading ? (
        <p className="text-sm text-muted-foreground">{t('sessionBalances.loading')}</p>
      ) : q.data?.metrics ? (
        <SessionMetricsPanel metrics={q.data.metrics} />
      ) : q.data && !q.isLoading && sid.length > 0 ? (
        <p className="text-xs text-muted-foreground">{t('sessionBalances.metricsNoOpen')}</p>
      ) : null}
      {q.isLoading ? null : !q.data?.balances.length ? (
        <p className="text-sm text-muted-foreground">{t('sessionBalances.empty')}</p>
      ) : (
        <>
          <div className="flex justify-end">
            <Button type="button" variant="ghost" size="sm" className="h-7 text-xs" onClick={() => setShowRaw((v) => !v)}>
              {showRaw ? t('sessionBalances.hideRaw') : t('sessionBalances.showRaw')}
            </Button>
          </div>
          <BalancesTable balances={q.data.balances} showRaw={showRaw} />
        </>
      )}
      <p className="text-xs text-muted-foreground">
        <Link to="/logs" className="text-primary hover:underline">
          {t('sessionBalances.lifecycleLink')}
        </Link>
      </p>
    </div>
  )

  if (embedded) {
    return <div className={className}>{inner}</div>
  }

  return (
    <Card className={className}>
      <CardHeader className="flex flex-row items-start justify-between space-y-0 pb-2">
        <div>
          <CardTitle className="text-base">{t('sessionBalances.title')}</CardTitle>
          <p className="text-xs text-muted-foreground mt-1 font-mono break-all">{sid}</p>
          <p className="text-xs text-muted-foreground mt-1 max-w-2xl">{t('sessionBalances.retentionNote')}</p>
        </div>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          disabled={q.isFetching}
          onClick={() => void q.refetch()}
          className="shrink-0"
          title={t('sessionBalances.refresh')}
        >
          <RefreshCw className={`h-4 w-4 ${q.isFetching ? 'animate-spin' : ''}`} />
        </Button>
      </CardHeader>
      <CardContent>{inner}</CardContent>
    </Card>
  )
}
