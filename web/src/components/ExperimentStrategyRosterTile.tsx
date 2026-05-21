import { AlertCircle, Check, CircleOff, Save, X } from 'lucide-react'
import type { ExperimentArm } from '@/lib/experimentArm'
import { describeArmValidation } from '@/lib/experimentArmValidation'
import { isArmDirty } from '@/lib/experimentArmDirty'
import { deriveArmTickSyncStatus, describeArmTickSyncStatus } from '@/hooks/useExperimentArmTickSync'
import { useArmPool } from '@/hooks/useArmPool'
import { poolLabelForAddress } from '@/lib/experimentPoolLabel'
import type { Strategy } from '@/lib/api'
import { useI18n } from '@/lib/i18n'
import { formatUSD } from '@/lib/utils'

type Props = {
  arm: ExperimentArm
  index: number
  selected: boolean
  strategies: Strategy[]
  budgetUsd?: number
  onSelect: () => void
  onRemove: () => void
}

export default function ExperimentStrategyRosterTile({
  arm,
  index,
  selected,
  strategies,
  budgetUsd,
  onSelect,
  onRemove,
}: Props) {
  const { t, locale } = useI18n()
  const poolMeta = useArmPool(arm.poolAddress)
  const linked = strategies.find((s) => s.id === arm.reuseStrategyId)
  const dirty = isArmDirty(arm, linked)
  const validation = describeArmValidation(arm, poolMeta.poolReady, locale)
  const ready = validation.ok && !dirty && arm.enabled
  const tickStatus = deriveArmTickSyncStatus(
    arm,
    arm.poolAddress,
    poolMeta.poolReady,
    poolMeta.poolCurrentTick,
    poolMeta.tickSpacing,
    false,
    arm.form.strategyType === 'bollinger',
  )
  const tickStatusLabel = describeArmTickSyncStatus(tickStatus, locale)

  const typeLabel = arm.form.strategyType.replace(/_/g, ' ')
  const pairLabel = arm.poolAddress.trim()
    ? poolLabelForAddress(arm.poolAddress)
    : locale === 'pl'
      ? 'Brak pary'
      : 'No pair'

  return (
    <div
      className={[
        'group relative flex flex-col rounded-xl border-2 text-left transition-all duration-150',
        'min-h-[7.5rem]',
        selected
          ? 'border-primary bg-primary/10 shadow-md ring-2 ring-primary/30 scale-[1.02] z-10'
          : 'border-border/80 bg-card hover:border-primary/50',
        !arm.enabled ? 'opacity-55 saturate-50' : '',
      ].join(' ')}
    >
      <button
        type="button"
        onClick={(e) => {
          e.stopPropagation()
          onRemove()
        }}
        className={[
          'absolute -top-1.5 -right-1.5 z-20 flex h-5 w-5 items-center justify-center rounded-full',
          'border border-border bg-background shadow-sm',
          'text-muted-foreground hover:text-destructive hover:border-destructive/40 hover:bg-destructive/10',
          'transition-colors',
        ].join(' ')}
        aria-label={t('experiment.removeFromRoster').replace('{name}', arm.label || `#${index + 1}`)}
      >
        <X className="h-3 w-3" strokeWidth={2.5} />
      </button>

      <button
        type="button"
        role="tab"
        aria-selected={selected}
        onClick={onSelect}
        className={[
          'flex flex-1 flex-col text-left p-2.5 sm:p-3 min-h-[7.5rem]',
          'hover:shadow-md hover:-translate-y-0.5 transition-all duration-150 rounded-[10px]',
        ].join(' ')}
      >
      <div className="flex items-start justify-between gap-1 mb-1.5 pr-3">
        <span
          className={[
            'flex h-6 w-6 shrink-0 items-center justify-center rounded-md text-[11px] font-bold tabular-nums',
            selected ? 'bg-primary text-primary-foreground' : 'bg-muted text-muted-foreground',
          ].join(' ')}
        >
          {index + 1}
        </span>
        <div className="flex items-center gap-0.5 shrink-0">
          {!arm.enabled ? (
            <CircleOff className="h-3.5 w-3.5 text-muted-foreground" aria-label="disabled" />
          ) : dirty ? (
            <Save className="h-3.5 w-3.5 text-amber-600 dark:text-amber-400" aria-label="unsaved" />
          ) : ready ? (
            <Check className="h-3.5 w-3.5 text-green-600 dark:text-green-400" aria-label="ready" />
          ) : (
            <AlertCircle className="h-3.5 w-3.5 text-amber-600 dark:text-amber-400" aria-label="needs attention" />
          )}
        </div>
      </div>

      <div className="flex-1 min-h-0 space-y-0.5">
        <div className="font-semibold text-sm leading-tight line-clamp-2" title={arm.label}>
          {arm.label || `#${index + 1}`}
        </div>
        <div className="text-[10px] uppercase tracking-wide text-muted-foreground truncate capitalize">
          {typeLabel}
        </div>
        <div className="text-[11px] text-primary/90 font-medium truncate mt-1">{pairLabel}</div>
      </div>

      <div className="mt-auto pt-2 space-y-0.5">
        <div
          className={[
            'text-xs font-bold tabular-nums',
            budgetUsd != null && budgetUsd > 0 && arm.enabled
              ? 'text-emerald-700 dark:text-emerald-400'
              : 'text-muted-foreground',
          ].join(' ')}
        >
          {budgetUsd != null && budgetUsd > 0 && arm.enabled
            ? formatUSD(budgetUsd)
            : arm.enabled
              ? '—'
              : locale === 'pl'
                ? 'Poza launch'
                : 'Excluded'}
        </div>
        <div className="text-[10px] text-muted-foreground truncate border-t border-border/50 pt-1">
          {arm.tickLower !== '' && arm.tickUpper !== ''
            ? `[${arm.tickLower}, ${arm.tickUpper}]`
            : tickStatusLabel}
        </div>
      </div>
      </button>
    </div>
  )
}
