import type { DipLadderSummaryMetrics } from '@/lib/dipLadder'
import { formatDipUsd } from '@/lib/dipSnapshotDisplay'
import { useI18n } from '@/lib/i18n'
import { Button } from '@/components/ui/button'

function fmtPct(n: number): string {
  if (!Number.isFinite(n)) return '—'
  return `${n.toFixed(2)}%`
}

export function DipLadderSummary({
  summary,
  onPropose,
  proposePending,
}: {
  summary: DipLadderSummaryMetrics
  /** When set, enables Propose (U4) unless summary.proposeDisabled. */
  onPropose?: () => void
  proposePending?: boolean
}) {
  const { t } = useI18n()
  const reasonKey =
    summary.proposeDisabledReason === 'required_gt_available'
      ? 'dip.ladderBlockRequired'
      : summary.proposeDisabledReason === 'below_min'
        ? 'dip.ladderBlockMin'
        : summary.proposeDisabledReason === 'invalid_entry'
          ? 'dip.ladderBlockEntry'
          : null

  const canPropose = Boolean(onPropose) && !summary.proposeDisabled

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
            {summary.availableUsd == null
              ? t('dip.ladderAvailableUnknown')
              : formatDipUsd(summary.availableUsd)}
          </dd>
        </div>
      </dl>
      <div className="space-y-1 pt-1">
        <Button
          type="button"
          size="sm"
          disabled={!canPropose || proposePending}
          onClick={() => onPropose?.()}
          title={t('dip.ladderProposeHint')}
        >
          {proposePending
            ? t('dip.ladderProposePending')
            : canPropose
              ? t('dip.ladderPropose')
              : t('dip.ladderProposeDisabled')}
        </Button>
        {summary.proposeDisabled && reasonKey ? (
          <p className="text-xs text-amber-700 dark:text-amber-400">{t(reasonKey)}</p>
        ) : (
          <p className="text-xs text-muted-foreground">{t('dip.ladderProposeHint')}</p>
        )}
      </div>
    </div>
  )
}
