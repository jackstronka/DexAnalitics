import type { DipLadderSummaryMetrics } from '@/lib/dipLadder'
import { formatDipUsd } from '@/lib/dipSnapshotDisplay'
import { useI18n } from '@/lib/i18n'

function fmtPct(n: number): string {
  if (!Number.isFinite(n)) return '—'
  return `${n.toFixed(2)}%`
}

export function DipLadderSummary({ summary }: { summary: DipLadderSummaryMetrics }) {
  const { t } = useI18n()
  const reasonKey =
    summary.proposeDisabledReason === 'required_gt_available'
      ? 'dip.ladderBlockRequired'
      : summary.proposeDisabledReason === 'below_min'
        ? 'dip.ladderBlockMin'
        : summary.proposeDisabledReason === 'invalid_entry'
          ? 'dip.ladderBlockEntry'
          : null

  return (
    <div className="rounded-md border p-3 space-y-2 text-sm">
      <div className="font-semibold">{t('dip.ladderSummaryTitle')}</div>
      <dl className="grid gap-1 sm:grid-cols-2">
        <div className="flex justify-between gap-2 sm:block">
          <dt className="text-muted-foreground">{t('dip.ladderMaxDrop')}</dt>
          <dd className="font-medium">{fmtPct(summary.maxPriceDropPct)}</dd>
        </div>
        <div className="flex justify-between gap-2 sm:block">
          <dt className="text-muted-foreground">{t('dip.ladderMaxAvgDev')}</dt>
          <dd className="font-medium">{fmtPct(summary.maxAvgEntryDeviationPct)}</dd>
        </div>
        <div className="flex justify-between gap-2 sm:block">
          <dt className="text-muted-foreground">{t('dip.ladderRequired')}</dt>
          <dd className="font-medium">{formatDipUsd(summary.requiredUsd)}</dd>
        </div>
        <div className="flex justify-between gap-2 sm:block">
          <dt className="text-muted-foreground">{t('dip.ladderAvailable')}</dt>
          <dd className="font-medium">
            {summary.availableUsd == null ? t('dip.ladderAvailableUnknown') : formatDipUsd(summary.availableUsd)}
          </dd>
        </div>
      </dl>
      <ButtonLikeDisabled proposeDisabled={summary.proposeDisabled} reasonKey={reasonKey} />
    </div>
  )
}

function ButtonLikeDisabled({
  proposeDisabled,
  reasonKey,
}: {
  proposeDisabled: boolean
  reasonKey: string | null
}) {
  const { t } = useI18n()
  return (
    <div className="space-y-1 pt-1">
      <button
        type="button"
        disabled
        className="inline-flex h-9 items-center rounded-md border px-3 text-sm opacity-60 cursor-not-allowed"
        title={t('dip.ladderProposeHint')}
      >
        {t('dip.ladderProposeDisabled')}
      </button>
      {proposeDisabled && reasonKey ? (
        <p className="text-xs text-amber-700 dark:text-amber-400">{t(reasonKey)}</p>
      ) : (
        <p className="text-xs text-muted-foreground">{t('dip.ladderProposeHint')}</p>
      )}
    </div>
  )
}
