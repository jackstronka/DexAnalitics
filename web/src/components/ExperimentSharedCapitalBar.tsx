import { useMemo } from 'react'
import { InlineError } from '@/components/ui/inline-error'
import type { ExperimentArm } from '@/lib/experimentArm'
import {
  splitBudgetEqual,
  validateArmBudgets,
  type AllocationMode,
} from '@/lib/experimentCapital'
import { formatUSD } from '@/lib/utils'
import { useI18n } from '@/lib/i18n'

function readBudgetInput(raw: string): number | '' {
  if (raw.trim() === '') return ''
  const n = Number(raw)
  return Number.isFinite(n) && n >= 0 ? n : ''
}

type Props = {
  arms: ExperimentArm[]
  totalBudgetUsd: number | ''
  allocationMode: AllocationMode
  budgetByArmId: Map<string, number>
  onTotalBudgetChange: (v: number | '') => void
  onAllocationModeChange: (mode: AllocationMode) => void
}

export default function ExperimentSharedCapitalBar({
  arms,
  totalBudgetUsd,
  allocationMode,
  budgetByArmId,
  onTotalBudgetChange,
  onAllocationModeChange,
}: Props) {
  const { t } = useI18n()
  const enabledArms = useMemo(() => arms.filter((a) => a.enabled), [arms])

  const equalPreview = useMemo(() => {
    if (totalBudgetUsd === '' || Number(totalBudgetUsd) <= 0 || enabledArms.length === 0) {
      return []
    }
    return splitBudgetEqual(Number(totalBudgetUsd), enabledArms.length)
  }, [totalBudgetUsd, enabledArms.length])

  const budgetValidation = useMemo(() => {
    if (totalBudgetUsd === '' || Number(totalBudgetUsd) <= 0) {
      return { valid: false, sum: 0, exceedsTotal: false }
    }
    const budgets = enabledArms.map((a) => budgetByArmId.get(a.id) ?? 0)
    return validateArmBudgets(Number(totalBudgetUsd), budgets)
  }, [totalBudgetUsd, enabledArms, budgetByArmId])

  const liveSplitLabel = useMemo(() => {
    if (enabledArms.length === 0) return null
    if (totalBudgetUsd === '' || Number(totalBudgetUsd) <= 0) return null
    if (allocationMode === 'equal' && equalPreview.length > 0) {
      const per = equalPreview[0]
      if (per == null) return null
      return t('experiment.liveSplitEqual')
        .replace('{n}', String(enabledArms.length))
        .replace('{each}', formatUSD(per))
        .replace('{total}', formatUSD(Number(totalBudgetUsd)))
    }
    const parts = enabledArms
      .map((a) => {
        const b = budgetByArmId.get(a.id)
        return b != null && b > 0 ? `${a.label}: ${formatUSD(b)}` : null
      })
      .filter(Boolean)
    if (parts.length === 0) return null
    return parts.join(' · ')
  }, [enabledArms, totalBudgetUsd, allocationMode, equalPreview, budgetByArmId, t])

  return (
    <div className="rounded-xl border border-border/80 bg-background/80 px-3 py-3 sm:px-4 space-y-2">
      <div className="flex flex-wrap items-end gap-3">
        <div className="flex-1 min-w-[10rem]">
          <label className="block text-xs font-medium text-muted-foreground mb-1">
            {t('experiment.totalBudgetUsd')}
          </label>
          <input
            type="number"
            min={0}
            step="0.01"
            className="w-full rounded-lg border border-input bg-background px-3 py-2 text-sm font-semibold tabular-nums"
            value={totalBudgetUsd}
            onChange={(e) => onTotalBudgetChange(readBudgetInput(e.target.value))}
            placeholder="30"
          />
        </div>
        <div className="flex-1 min-w-[10rem]">
          <label className="block text-xs font-medium text-muted-foreground mb-1">
            {t('experiment.allocationMode')}
          </label>
          <select
            className="w-full rounded-lg border border-input bg-background px-3 py-2 text-sm"
            value={allocationMode}
            onChange={(e) => onAllocationModeChange(e.target.value as AllocationMode)}
          >
            <option value="equal">{t('experiment.allocationEqual')}</option>
            <option value="fixed_usd">{t('experiment.allocationManual')}</option>
          </select>
        </div>
      </div>

      {liveSplitLabel ? (
        <p className="text-xs text-muted-foreground leading-relaxed">{liveSplitLabel}</p>
      ) : enabledArms.length > 0 ? (
        <p className="text-xs text-muted-foreground">{t('experiment.liveSplitHint')}</p>
      ) : null}

      {allocationMode === 'fixed_usd' ? (
        <p className="text-[11px] text-muted-foreground">{t('experiment.manualBudgetOnDetail')}</p>
      ) : null}

      {!budgetValidation.valid && totalBudgetUsd !== '' ? (
        <InlineError>{t('experiment.budgetExceedsTotal')}</InlineError>
      ) : null}
    </div>
  )
}
