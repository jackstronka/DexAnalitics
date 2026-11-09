import { useMemo, useState, type ChangeEvent } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Link } from 'react-router-dom'
import { RefreshCw, TrendingDown } from 'lucide-react'
import { DipLadderPreviewTable } from '@/components/dip/DipLadderPreviewTable'
import { DipLadderSummary } from '@/components/dip/DipLadderSummary'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { ErrorBanner } from '@/components/ui/error-banner'
import { getDipSnapshot } from '@/lib/api'
import { buildDipLadderPreview } from '@/lib/dipLadder'
import {
  dipOrdersEmptyKind,
  formatDipUsd,
  formatDipZoneBand,
  normalizeDipZone,
} from '@/lib/dipSnapshotDisplay'
import { useI18n } from '@/lib/i18n'

function zoneBadgeClass(zone: string): string {
  switch (normalizeDipZone(zone)) {
    case 'A':
      return 'bg-emerald-100 text-emerald-800 border-emerald-200'
    case 'B':
      return 'bg-amber-100 text-amber-800 border-amber-200'
    case 'C':
      return 'bg-rose-100 text-rose-800 border-rose-200'
    default:
      return 'bg-muted text-muted-foreground border-border'
  }
}

type LadderForm = {
  baseOrderUsd: string
  safetyOrderUsd: string
  deviationPct: string
  deviationStepScale: string
  volumeScale: string
  maxSafetyOrders: string
  takeProfitPct: string
  availableUsd: string
}

const DEFAULT_LADDER: LadderForm = {
  baseOrderUsd: '50',
  safetyOrderUsd: '50',
  deviationPct: '1',
  deviationStepScale: '1',
  volumeScale: '1.5',
  maxSafetyOrders: '5',
  takeProfitPct: '1.5',
  availableUsd: '',
}

function num(s: string, fallback = 0): number {
  const n = Number(s)
  return Number.isFinite(n) ? n : fallback
}

export default function Dip() {
  const { locale, t } = useI18n()
  const [ladderForm, setLadderForm] = useState<LadderForm>(DEFAULT_LADDER)
  const q = useQuery({
    queryKey: ['dip-snapshot'],
    queryFn: getDipSnapshot,
    refetchInterval: 60_000,
    retry: 1,
  })

  const data = q.data
  const zone = data ? normalizeDipZone(data.zone) : '—'
  const emptyKind = data
    ? dipOrdersEmptyKind(data.orders_source, data.buy_dip_orders.length)
    : null
  const fmtTs = (ts?: string) =>
    ts ? new Date(ts).toLocaleString(locale === 'pl' ? 'pl-PL' : 'en-US') : '—'

  const entryPrice = data?.price_usd ?? null
  const ladderPreview = useMemo(() => {
    const availableRaw = ladderForm.availableUsd.trim()
    const availableUsd = availableRaw === '' ? null : num(availableRaw, NaN)
    return buildDipLadderPreview(entryPrice ?? NaN, {
      baseOrderUsd: num(ladderForm.baseOrderUsd),
      safetyOrderUsd: num(ladderForm.safetyOrderUsd),
      deviationPct: num(ladderForm.deviationPct),
      deviationStepScale: num(ladderForm.deviationStepScale, 1),
      volumeScale: num(ladderForm.volumeScale, 1),
      maxSafetyOrders: Math.floor(num(ladderForm.maxSafetyOrders)),
      takeProfitPct: num(ladderForm.takeProfitPct),
      availableUsd: availableUsd != null && Number.isFinite(availableUsd) ? availableUsd : null,
    })
  }, [entryPrice, ladderForm])

  const setField = (key: keyof LadderForm) => (e: ChangeEvent<HTMLInputElement>) => {
    setLadderForm((prev) => ({ ...prev, [key]: e.target.value }))
  }

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight flex items-center gap-2">
            <TrendingDown className="h-6 w-6 text-primary" />
            {t('dip.title')}
          </h1>
          <p className="text-sm text-muted-foreground mt-1 max-w-2xl">{t('dip.subtitle')}</p>
        </div>
        <Button
          variant="outline"
          size="sm"
          onClick={() => void q.refetch()}
          disabled={q.isFetching}
        >
          <RefreshCw className={`h-4 w-4 mr-2 ${q.isFetching ? 'animate-spin' : ''}`} />
          {t('dip.refresh')}
        </Button>
      </div>

      {q.isError ? (
        <ErrorBanner>{q.error instanceof Error ? q.error.message : String(q.error)}</ErrorBanner>
      ) : null}

      <div className="grid gap-4 md:grid-cols-2">
        <Card>
          <CardHeader>
            <CardTitle>{t('dip.zoneTitle')}</CardTitle>
            <CardDescription>{t('dip.zoneHint')}</CardDescription>
          </CardHeader>
          <CardContent className="space-y-3 text-sm">
            {q.isLoading && !data ? (
              <p className="text-muted-foreground">{t('dip.loading')}</p>
            ) : data ? (
              <>
                <div className="flex flex-wrap items-center gap-3">
                  <span
                    className={`inline-flex items-center rounded border px-2.5 py-1 text-sm font-semibold ${zoneBadgeClass(zone)}`}
                  >
                    {t('dip.zoneLabel')}: {zone}
                  </span>
                  <span className="font-medium">
                    {data.symbol} {formatDipUsd(data.price_usd ?? null)}
                  </span>
                </div>
                <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-muted-foreground">
                  <dt>A</dt>
                  <dd>{formatDipZoneBand(data.zones.a.min, data.zones.a.max)}</dd>
                  <dt>B</dt>
                  <dd>{formatDipZoneBand(data.zones.b.min, data.zones.b.max)}</dd>
                  <dt>C</dt>
                  <dd>{formatDipZoneBand(data.zones.c.min, data.zones.c.max)}</dd>
                </dl>
                <p className="text-xs text-muted-foreground">
                  {t('dip.priceSource')}: {data.price_source} · {t('dip.fetchedAt')}:{' '}
                  {fmtTs(data.fetched_at)}
                </p>
              </>
            ) : null}
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>{t('dip.metaTitle')}</CardTitle>
            <CardDescription>{t('dip.metaHint')}</CardDescription>
          </CardHeader>
          <CardContent className="space-y-2 text-sm">
            {data ? (
              <>
                <p>
                  <span className="text-muted-foreground">{t('dip.ordersSource')}: </span>
                  <code className="text-xs">{data.orders_source}</code>
                </p>
                <p>
                  <span className="text-muted-foreground">{t('dip.mint')}: </span>
                  <code className="text-xs break-all">{data.mint}</code>
                </p>
                {data.notes.length > 0 && (
                  <ul className="list-disc pl-5 text-muted-foreground space-y-1">
                    {data.notes.map((n) => (
                      <li key={n}>{n}</li>
                    ))}
                  </ul>
                )}
              </>
            ) : (
              <p className="text-muted-foreground">{t('dip.loading')}</p>
            )}
          </CardContent>
        </Card>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>{t('dip.ordersTitle')}</CardTitle>
          <CardDescription>{t('dip.ordersHint')}</CardDescription>
        </CardHeader>
        <CardContent>
          {q.isLoading && !data ? (
            <p className="text-sm text-muted-foreground">{t('dip.loading')}</p>
          ) : emptyKind === 'no_jwt' ? (
            <div className="space-y-2 text-sm">
              <p>{t('dip.emptyNoJwt')}</p>
              <Button asChild variant="secondary" size="sm">
                <Link to="/settings">{t('dip.openSettings')}</Link>
              </Button>
            </div>
          ) : emptyKind === 'no_api_key' ? (
            <p className="text-sm text-muted-foreground">{t('dip.emptyNoApiKey')}</p>
          ) : emptyKind === 'error' ? (
            <p className="text-sm text-muted-foreground">{t('dip.emptyError')}</p>
          ) : emptyKind === 'skipped' ? (
            <p className="text-sm text-muted-foreground">{t('dip.emptySkipped')}</p>
          ) : emptyKind === 'empty' ? (
            <p className="text-sm text-muted-foreground">{t('dip.emptyOrders')}</p>
          ) : data && data.buy_dip_orders.length > 0 ? (
            <div className="overflow-x-auto rounded-md border">
              <table className="w-full text-left text-sm">
                <thead className="bg-muted/50">
                  <tr>
                    <th className="px-3 py-2 font-medium whitespace-nowrap">{t('dip.colId')}</th>
                    <th className="px-3 py-2 font-medium whitespace-nowrap">{t('dip.colState')}</th>
                    <th className="px-3 py-2 font-medium whitespace-nowrap">
                      {t('dip.colTrigger')}
                    </th>
                    <th className="px-3 py-2 font-medium whitespace-nowrap">
                      {t('dip.colRemaining')}
                    </th>
                    <th className="px-3 py-2 font-medium whitespace-nowrap">
                      {t('dip.colExpires')}
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {data.buy_dip_orders.map((o) => (
                    <tr key={o.id} className="border-t">
                      <td className="px-3 py-2 font-mono text-xs break-all">{o.id}</td>
                      <td className="px-3 py-2">{o.order_state ?? '—'}</td>
                      <td className="px-3 py-2">{formatDipUsd(o.trigger_price_usd ?? null)}</td>
                      <td className="px-3 py-2 font-mono text-xs">
                        {o.remaining_input_amount ?? '—'}
                      </td>
                      <td className="px-3 py-2 text-xs text-muted-foreground">
                        {o.expires_at != null
                          ? new Date(o.expires_at * 1000).toLocaleString(
                              locale === 'pl' ? 'pl-PL' : 'en-US',
                            )
                          : '—'}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          ) : null}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>{t('dip.ladderTitle')}</CardTitle>
          <CardDescription>{t('dip.ladderHint')}</CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <p className="text-xs text-muted-foreground">
            {t('dip.ladderEntry')}: {formatDipUsd(entryPrice)} ({t('dip.ladderEntrySource')})
          </p>
          <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4 text-sm">
            {(
              [
                ['baseOrderUsd', 'dip.ladderBase'],
                ['safetyOrderUsd', 'dip.ladderSafety'],
                ['deviationPct', 'dip.ladderDeviation'],
                ['deviationStepScale', 'dip.ladderDevScale'],
                ['volumeScale', 'dip.ladderVolScale'],
                ['maxSafetyOrders', 'dip.ladderMaxSo'],
                ['takeProfitPct', 'dip.ladderTp'],
                ['availableUsd', 'dip.ladderAvailableInput'],
              ] as const
            ).map(([key, labelKey]) => (
              <label key={key} className="flex flex-col gap-1">
                <span className="text-xs text-muted-foreground">{t(labelKey)}</span>
                <input
                  className="rounded-md border bg-background px-2 py-1.5"
                  type="number"
                  step="any"
                  value={ladderForm[key]}
                  onChange={setField(key)}
                />
              </label>
            ))}
          </div>
          <DipLadderSummary summary={ladderPreview.summary} />
          <DipLadderPreviewTable rows={ladderPreview.rows} />
        </CardContent>
      </Card>
    </div>
  )
}
