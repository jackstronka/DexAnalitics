import { Plus } from 'lucide-react'
import { MAX_EXPERIMENT_ARMS } from '@/lib/experimentCapital'
import type { ExperimentArm } from '@/lib/experimentArm'
import type { Strategy } from '@/lib/api'
import type { AllocationMode } from '@/lib/experimentCapital'
import ExperimentSharedCapitalBar from '@/components/ExperimentSharedCapitalBar'
import ExperimentArmTickSyncRunner from '@/components/ExperimentArmTickSyncRunner'
import ExperimentStrategyRosterTile from '@/components/ExperimentStrategyRosterTile'
import { useI18n } from '@/lib/i18n'

type Props = {
  arms: ExperimentArm[]
  strategies: Strategy[]
  selectedArmId: string | null
  totalBudgetUsd: number | ''
  allocationMode: AllocationMode
  budgetByArmId: Map<string, number>
  onTotalBudgetChange: (v: number | '') => void
  onAllocationModeChange: (mode: AllocationMode) => void
  onSelectArm: (armId: string) => void
  onArmChange: (armId: string, arm: ExperimentArm) => void
  onRemoveArm: (armId: string) => void
  onAdd: () => void
  canAddMore: boolean
  addDisabledReason?: string
}

export default function ExperimentStrategyRoster({
  arms,
  strategies,
  selectedArmId,
  totalBudgetUsd,
  allocationMode,
  budgetByArmId,
  onTotalBudgetChange,
  onAllocationModeChange,
  onSelectArm,
  onArmChange,
  onRemoveArm,
  onAdd,
  canAddMore,
  addDisabledReason,
}: Props) {
  const { t } = useI18n()
  const remaining = MAX_EXPERIMENT_ARMS - arms.length

  return (
    <div className="rounded-2xl border border-border/80 bg-muted/5 p-3 sm:p-4 shadow-sm">
      <div className="flex items-center justify-between gap-2 mb-3 px-0.5">
        <span className="text-xs font-medium text-muted-foreground uppercase tracking-wide">
          {t('experiment.rosterTitle')}
        </span>
        <span className="text-xs text-muted-foreground tabular-nums">
          {t('experiment.armsCount')
            .replace('{n}', String(arms.length))
            .replace('{max}', String(MAX_EXPERIMENT_ARMS))
            .replace('{enabled}', String(arms.filter((a) => a.enabled).length))}
        </span>
      </div>

      <ExperimentSharedCapitalBar
        arms={arms}
        totalBudgetUsd={totalBudgetUsd}
        allocationMode={allocationMode}
        budgetByArmId={budgetByArmId}
        onTotalBudgetChange={onTotalBudgetChange}
        onAllocationModeChange={onAllocationModeChange}
      />

      {arms.map((arm) => (
        <ExperimentArmTickSyncRunner
          key={`tick-sync-${arm.id}`}
          arm={arm}
          onChange={(next) => onArmChange(arm.id, next)}
        />
      ))}

      <div
        className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6 gap-2 sm:gap-2.5 mt-3"
        role="tablist"
        aria-label={t('experiment.rosterTitle')}
      >
        {arms.map((arm, idx) => (
          <ExperimentStrategyRosterTile
            key={arm.id}
            arm={arm}
            index={idx}
            selected={arm.id === selectedArmId}
            strategies={strategies}
            budgetUsd={budgetByArmId.get(arm.id)}
            onSelect={() => onSelectArm(arm.id)}
            onRemove={() => onRemoveArm(arm.id)}
          />
        ))}

        <button
          type="button"
          disabled={!canAddMore}
          onClick={onAdd}
          title={!canAddMore ? addDisabledReason : undefined}
          className={[
            'flex flex-col items-center justify-center rounded-xl border-2 border-dashed min-h-[7.5rem] p-2 transition-all',
            canAddMore
              ? 'border-border hover:border-primary hover:bg-primary/5 text-muted-foreground hover:text-primary'
              : 'border-border/50 opacity-50 cursor-not-allowed',
          ].join(' ')}
        >
          <span className="flex h-9 w-9 items-center justify-center rounded-full border-2 border-current mb-1">
            <Plus className="h-4 w-4" strokeWidth={2.5} />
          </span>
          <span className="text-xs font-medium text-center leading-tight">
            {t('experiment.addStrategyInline')}
          </span>
          {canAddMore ? (
            <span className="text-[10px] text-muted-foreground mt-0.5">
              {t('experiment.addStrategyInlineHint').replace('{remaining}', String(remaining))}
            </span>
          ) : null}
        </button>
      </div>
    </div>
  )
}
