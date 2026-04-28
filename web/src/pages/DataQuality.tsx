import { useMemo, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { getBacktestDataReadiness, type BacktestDataReadinessResponse } from '@/lib/api'
import { useI18n } from '@/lib/i18n'

export default function DataQuality() {
  const { locale, t } = useI18n()
  const toLocalInput = (d: Date) => {
    const y = d.getFullYear()
    const m = `${d.getMonth() + 1}`.padStart(2, '0')
    const day = `${d.getDate()}`.padStart(2, '0')
    const h = `${d.getHours()}`.padStart(2, '0')
    const min = `${d.getMinutes()}`.padStart(2, '0')
    return `${y}-${m}-${day}T${h}:${min}`
  }
  const defaultRange = () => {
    const end = new Date()
    const start = new Date(end.getTime() - 72 * 60 * 60 * 1000)
    return { start: toLocalInput(start), end: toLocalInput(end) }
  }
  const [snapshotVariants, setSnapshotVariants] = useState<string[]>(['10m', '5m'])
  const [rangeStartLocal, setRangeStartLocal] = useState<string>(() => defaultRange().start)
  const [rangeEndLocal, setRangeEndLocal] = useState<string>(() => defaultRange().end)
  const rangeStartDate = rangeStartLocal ? new Date(rangeStartLocal) : null
  const rangeEndDate = rangeEndLocal ? new Date(rangeEndLocal) : null
  const hasValidRange =
    !!rangeStartDate &&
    !!rangeEndDate &&
    !Number.isNaN(rangeStartDate.getTime()) &&
    !Number.isNaN(rangeEndDate.getTime()) &&
    rangeStartDate <= rangeEndDate
  const q = useQuery<BacktestDataReadinessResponse>({
    queryKey: ['data-quality-readiness', snapshotVariants, rangeStartLocal, rangeEndLocal],
    queryFn: () =>
      getBacktestDataReadiness({
        snapshot_variants: snapshotVariants,
        range_start_utc: rangeStartDate?.toISOString(),
        range_end_utc: rangeEndDate?.toISOString(),
      }),
    enabled: snapshotVariants.length > 0 && hasValidRange,
    refetchInterval: 60_000,
  })

  const rows = q.data?.rows ?? []
  const statusBadgeClass = (status: string) => {
    switch (status) {
      case 'ok':
        return 'bg-emerald-100 text-emerald-800 border-emerald-200'
      case 'recovering':
        return 'bg-sky-100 text-sky-800 border-sky-200'
      case 'degraded':
        return 'bg-amber-100 text-amber-800 border-amber-200'
      default:
        return 'bg-rose-100 text-rose-800 border-rose-200'
    }
  }
  const statusLabel = (status: string) =>
    t(`dataQuality.status.${status}`, status)
  const sourceBadgeClass =
    q.data?.aggregate.source === 'db'
      ? 'bg-emerald-100 text-emerald-800 border-emerald-200'
      : 'bg-amber-100 text-amber-800 border-amber-200'
  const staleBadgeClass =
    (q.data?.aggregate.db_stale_rows ?? 0) > 0
      ? 'bg-amber-100 text-amber-800 border-amber-200'
      : 'bg-emerald-100 text-emerald-800 border-emerald-200'
  const grouped = useMemo(() => {
    return [...rows].sort((a, b) => {
      const p = a.pool_label.localeCompare(b.pool_label)
      if (p !== 0) return p
      return a.snapshot_variant.localeCompare(b.snapshot_variant)
    })
  }, [rows])

  const toggle = (v: string) =>
    setSnapshotVariants((prev) =>
      prev.includes(v) ? prev.filter((x) => x !== v) : [...prev, v],
    )
  const fmtTs = (ts?: string | null) =>
    ts ? new Date(ts).toLocaleString(locale === 'pl' ? 'pl-PL' : 'en-US') : '—'
  const fmtLookback = (hours: number) => `${hours}h (~${(hours / 24).toFixed(1)}d)`

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader>
          <CardTitle>{t('dataQuality.title')}</CardTitle>
          <CardDescription>{t('dataQuality.subtitle')}</CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <div className="rounded border p-3 space-y-2">
            <div className="text-xs font-semibold">{t('dataQuality.rangeTitle')}</div>
            <div className="flex flex-wrap items-end gap-3 text-sm">
              <label className="flex flex-col gap-1">
                <span className="text-xs text-muted-foreground">{t('dataQuality.rangeStart')}</span>
                <input
                  type="datetime-local"
                  value={rangeStartLocal}
                  onChange={(e) => setRangeStartLocal(e.target.value)}
                  className="rounded border bg-background px-2 py-1"
                />
              </label>
              <label className="flex flex-col gap-1">
                <span className="text-xs text-muted-foreground">{t('dataQuality.rangeEnd')}</span>
                <input
                  type="datetime-local"
                  value={rangeEndLocal}
                  onChange={(e) => setRangeEndLocal(e.target.value)}
                  className="rounded border bg-background px-2 py-1"
                />
              </label>
              <button
                type="button"
                className="rounded border px-2 py-1 text-xs"
                onClick={() => {
                  const d = defaultRange()
                  setRangeStartLocal(d.start)
                  setRangeEndLocal(d.end)
                }}
              >
                {t('dataQuality.rangeReset72h')}
              </button>
            </div>
            <div className="text-[11px] text-muted-foreground">{t('dataQuality.rangeHint')}</div>
            {!hasValidRange && (
              <div className="text-[11px] text-red-500">{t('dataQuality.rangeInvalid')}</div>
            )}
          </div>
          <div className="flex gap-4 text-sm">
            {['10m', '5m'].map((v) => (
              <label key={v} className="flex items-center gap-2">
                <input
                  type="checkbox"
                  checked={snapshotVariants.includes(v)}
                  onChange={() => toggle(v)}
                />
                {v}
              </label>
            ))}
          </div>
          <div className="text-xs text-muted-foreground">
            {t('dataQuality.aggregateMinimum')}{' '}
            {q.data
              ? `${t('dataQuality.safe')} ${fmtLookback(q.data.aggregate.max_backtest_hours_recommended)} | ${t('dataQuality.maximum')} ${fmtLookback(q.data.aggregate.max_backtest_hours_hard)}`
              : '—'}
          </div>
          {q.data && (
            <div className="flex flex-wrap items-center gap-2 text-xs">
              <span className="text-muted-foreground">{t('dataQuality.overallStatus')}</span>
              <span
                className={`rounded border px-2 py-0.5 font-medium ${statusBadgeClass(q.data.aggregate.status)}`}
              >
                {statusLabel(q.data.aggregate.status)}
              </span>
              <span className="text-muted-foreground">
                {t('dataQuality.statusCounts', '')}
                {' '}
                ok={q.data.aggregate.status_ok_count}
                {' | '}
                degraded={q.data.aggregate.status_degraded_count}
                {' | '}
                recovering={q.data.aggregate.status_recovering_count}
                {' | '}
                missing={q.data.aggregate.status_missing_count}
              </span>
              <span className="text-muted-foreground">{t('dataQuality.source')}</span>
              <span className={`rounded border px-2 py-0.5 font-medium ${sourceBadgeClass}`}>
                {q.data.aggregate.source === 'db'
                  ? t('dataQuality.sourceDbFresh')
                  : t('dataQuality.sourceFallback')}
              </span>
              <span className="text-muted-foreground">{t('dataQuality.staleDbRows')}</span>
              <span className={`rounded border px-2 py-0.5 font-medium ${staleBadgeClass}`}>
                {q.data.aggregate.db_stale_rows}
              </span>
            </div>
          )}
          {q.data?.thresholds && (
            <div className="rounded border p-3 text-xs space-y-1">
              <div className="font-semibold">{t('dataQuality.thresholdsTitle')}</div>
              <div>
                cache_ttl_secs={q.data.thresholds.cache_ttl_secs} | hard_gap_multiplier=
                {q.data.thresholds.hard_gap_multiplier}
              </div>
              <div>db_max_age_secs={q.data.thresholds.db_max_age_secs}</div>
              <div>
                recommended_coverage_pct={q.data.thresholds.recommended_coverage_pct} |
                recommended_gap_multiplier={q.data.thresholds.recommended_gap_multiplier}
              </div>
              <div>
                recommended_fallback_ratio={q.data.thresholds.recommended_fallback_ratio}
              </div>
            </div>
          )}
          <div className="text-[11px] text-muted-foreground">{t('dataQuality.maxGapNote')}</div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>{t('dataQuality.detailsTitle')}</CardTitle>
        </CardHeader>
        <CardContent>
          {q.isPending && <div className="text-sm text-muted-foreground">{t('dataQuality.loading')}</div>}
          {q.isError && <div className="text-sm text-red-500">{t('dataQuality.loadError')}</div>}
          {!q.isPending && !q.isError && (
            <div className="overflow-auto">
              <table className="w-full text-sm">
                <thead>
                  <tr className="border-b text-left">
                    <th className="p-2" title={t('dataQuality.tipPool')}>
                      {t('dataQuality.colPool')}
                    </th>
                    <th className="p-2" title={t('dataQuality.tipVariant')}>
                      {t('dataQuality.colVariant')}
                    </th>
                    <th className="p-2" title={t('dataQuality.tipStatus')}>
                      {t('dataQuality.colStatus')}
                    </th>
                    <th className="p-2" title={t('dataQuality.tipCoverage')}>
                      {t('dataQuality.colCoverage')}
                    </th>
                    <th className="p-2" title={t('dataQuality.tipMaxGap')}>
                      {t('dataQuality.colMaxGap')}
                    </th>
                    <th className="p-2" title={t('dataQuality.tipSafeLookback')}>
                      {t('dataQuality.colSafeLookback')}
                    </th>
                    <th className="p-2" title={t('dataQuality.tipMaxLookback')}>
                      {t('dataQuality.colMaxLookback')}
                    </th>
                    <th className="p-2" title={t('dataQuality.tipContinuousFrom')}>
                      {t('dataQuality.colContinuousFrom')}
                    </th>
                    <th className="p-2" title={t('dataQuality.tipLatestPoint')}>
                      {t('dataQuality.colLatestPoint')}
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {grouped.map((r) => (
                    <tr key={`${r.pool_id}-${r.snapshot_variant}`} className="border-b">
                      <td className="p-2">{r.pool_label}</td>
                      <td className="p-2">{r.snapshot_variant}</td>
                      <td className="p-2">
                        <span className={`rounded border px-2 py-0.5 text-xs font-medium ${statusBadgeClass(r.status)}`}>
                          {statusLabel(r.status)}
                        </span>
                      </td>
                      <td className="p-2">{r.coverage_pct == null ? '—' : r.coverage_pct.toFixed(1)}</td>
                      <td className="p-2">{r.max_gap_minutes == null ? '—' : r.max_gap_minutes.toFixed(1)}</td>
                      <td className="p-2">{fmtLookback(r.max_backtest_hours_recommended)}</td>
                      <td className="p-2">{fmtLookback(r.max_backtest_hours_hard)}</td>
                      <td className="p-2">{fmtTs(r.oldest_continuous_ts_utc)}</td>
                      <td className="p-2">{fmtTs(r.latest_ts_utc)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  )
}

