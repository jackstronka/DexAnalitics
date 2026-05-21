import { useState } from 'react'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import {
  ChevronDown,
  ChevronUp,
  Copy,
  GripVertical,
  RefreshCw,
  Save,
  Trash2,
} from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader } from '@/components/ui/card'
import { InlineError } from '@/components/ui/inline-error'
import ExperimentArmEditor from '@/components/ExperimentArmEditor'
import type { ExperimentArm } from '@/lib/experimentArm'
import { applyStrategyToArm } from '@/lib/experimentArm'
import { experimentArmStatRows } from '@/lib/experimentArmStats'
import { describeArmValidation } from '@/lib/experimentArmValidation'
import {
  buildStrategyUpdateFromArm,
  isArmDirty,
} from '@/lib/experimentArmDirty'
import { useArmPool } from '@/hooks/useArmPool'
import {
  deriveArmTickSyncStatus,
  describeArmTickSyncStatus,
} from '@/hooks/useExperimentArmTickSync'
import { CURATED_ORCA_POOLS } from '@/lib/curatedPools'
import { updateStrategy, type Strategy } from '@/lib/api'
import type { AllocationMode } from '@/lib/experimentCapital'
import { formatUSD } from '@/lib/utils'
import { useI18n } from '@/lib/i18n'

function readBudgetInput(raw: string): number | '' {
  if (raw.trim() === '') return ''
  const n = Number(raw)
  return Number.isFinite(n) && n >= 0 ? n : ''
}

type Props = {
  arm: ExperimentArm
  index: number
  strategies: Strategy[]
  /** Roster detail panel — no timeline chrome. */
  variant?: 'detail' | 'stack'
  defaultEditOpen?: boolean
  onChange: (arm: ExperimentArm) => void
  onRemove: () => void
  onDuplicate: () => void
  onChangeStrategy: () => void
  canRemove: boolean
  allocationMode?: AllocationMode
  onArmBudgetChange?: (budgetUsd: number | '') => void
  resolvedBudgetUsd?: number
}

export default function ExperimentStrategyCard({
  arm,
  index,
  strategies,
  variant = 'detail',
  defaultEditOpen = false,
  onChange,
  onRemove,
  onDuplicate,
  onChangeStrategy,
  canRemove,
  allocationMode = 'equal',
  onArmBudgetChange,
  resolvedBudgetUsd,
}: Props) {
  const { t, locale } = useI18n()
  const queryClient = useQueryClient()
  const [editOpen, setEditOpen] = useState(defaultEditOpen)
  const [saveError, setSaveError] = useState<string | null>(null)
  const linkedStrategy = strategies.find((s) => s.id === arm.reuseStrategyId)
  const dirty = isArmDirty(arm, linkedStrategy)
  const poolMeta = useArmPool(arm.poolAddress)
  const isBollinger = arm.form.strategyType === 'bollinger'
  const tickStatus = deriveArmTickSyncStatus(
    arm,
    arm.poolAddress,
    poolMeta.poolReady,
    poolMeta.poolCurrentTick,
    poolMeta.tickSpacing,
    false,
    isBollinger,
  )
  const stats = experimentArmStatRows(arm, locale)
  const validation = describeArmValidation(arm, poolMeta.poolReady, locale)
  const tickLabel = describeArmTickSyncStatus(tickStatus, locale)
  const tickSync = { ticksOk: arm.tickLower !== '' && arm.tickUpper !== '', bollingerLoading: false }

  const saveMutation = useMutation({
    mutationFn: async () => {
      if (!arm.reuseStrategyId || !linkedStrategy) {
        throw new Error(t('experiment.saveStrategyMissing'))
      }
      return updateStrategy(
        arm.reuseStrategyId,
        buildStrategyUpdateFromArm(arm, linkedStrategy),
      )
    },
    onSuccess: (updated) => {
      setSaveError(null)
      onChange(applyStrategyToArm(arm, updated))
      void queryClient.invalidateQueries({ queryKey: ['strategies'] })
      void queryClient.invalidateQueries({ queryKey: ['strategy', updated.id] })
    },
    onError: (e: Error) => setSaveError(e.message),
  })

  function onPoolChange(nextAddress: string) {
    onChange({
      ...arm,
      poolAddress: nextAddress,
      tickLower: '',
      tickUpper: '',
      tickAutoSync: true,
    })
  }

  return (
    <div className={variant === 'stack' ? 'relative pl-0 sm:pl-8' : ''}>
      {variant === 'stack' ? (
        <>
          <div
            className="hidden sm:flex absolute left-3 top-0 bottom-0 w-px bg-border"
            aria-hidden
          />
          <div
            className="hidden sm:flex absolute left-1.5 top-6 h-3 w-3 rounded-full border-2 border-primary bg-background"
            aria-hidden
          />
        </>
      ) : null}

      <Card
        className={[
          'overflow-hidden shadow-sm transition-shadow',
          validation.ok ? 'border-border/80' : 'border-amber-500/40 ring-1 ring-amber-500/20',
        ].join(' ')}
      >
        <CardHeader className="pb-3 space-y-3 bg-muted/10">
          <div className="flex flex-wrap items-start gap-2 sm:gap-3">
            <div className="flex items-center gap-2 text-muted-foreground pt-1">
              {variant === 'stack' ? (
                <GripVertical className="h-4 w-4 hidden sm:block opacity-40" aria-hidden />
              ) : null}
              <span className="flex h-7 w-7 items-center justify-center rounded-full bg-primary/15 text-primary text-xs font-bold tabular-nums">
                {index + 1}
              </span>
            </div>

            <div className="flex-1 min-w-[10rem] space-y-3">
              <div className="space-y-1">
                <label className="text-xs text-muted-foreground">{t('experiment.armPoolLabel')}</label>
                <select
                  className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                  value={arm.poolAddress}
                  onChange={(e) => onPoolChange(e.target.value)}
                >
                  <option value="">{t('experiment.poolSelectPlaceholder')}</option>
                  {CURATED_ORCA_POOLS.map((p) => (
                    <option key={p.address} value={p.address}>
                      {p.label}
                    </option>
                  ))}
                </select>
                {poolMeta.poolLoading ? (
                  <p className="text-[11px] text-muted-foreground">{t('experiment.poolLoading')}</p>
                ) : null}
                {poolMeta.poolError ? (
                  <InlineError>{(poolMeta.poolError as Error).message}</InlineError>
                ) : null}
                {poolMeta.pairLabel ? (
                  <div className="inline-flex items-center rounded-full bg-primary/10 text-primary px-2.5 py-0.5 text-[11px] font-medium">
                    {poolMeta.pairLabel}
                  </div>
                ) : null}
              </div>

              <div className="space-y-1">
                <label className="text-xs text-muted-foreground">{t('experiment.strategyLabel')}</label>
                <input
                  className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm font-medium"
                  value={arm.label}
                  onChange={(e) => onChange({ ...arm, label: e.target.value })}
                />
              </div>

              {allocationMode === 'fixed_usd' && onArmBudgetChange ? (
                <div className="space-y-1">
                  <label className="text-xs text-muted-foreground">{t('experiment.armBudgetUsd')}</label>
                  <input
                    type="number"
                    min={0}
                    step="0.01"
                    className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm tabular-nums"
                    value={arm.budgetUsd}
                    onChange={(e) => onArmBudgetChange(readBudgetInput(e.target.value))}
                    placeholder="10"
                  />
                </div>
              ) : resolvedBudgetUsd != null && resolvedBudgetUsd > 0 ? (
                <div className="rounded-lg bg-emerald-500/10 border border-emerald-500/20 px-3 py-2">
                  <div className="text-[10px] uppercase text-muted-foreground">{t('experiment.armBudgetShare')}</div>
                  <div className="text-lg font-bold tabular-nums text-emerald-700 dark:text-emerald-400">
                    {formatUSD(resolvedBudgetUsd)}
                  </div>
                </div>
              ) : null}
            </div>

            <div className="flex flex-wrap items-center gap-2 pt-5 sm:pt-6">
              <span
                className={[
                  'text-[11px] font-medium rounded-full px-2 py-0.5',
                  tickSync.ticksOk
                    ? 'bg-blue-500/10 text-blue-700 dark:text-blue-300'
                    : 'bg-muted text-muted-foreground',
                ].join(' ')}
                title={t('experiment.ticksAutoHint')}
              >
                {tickLabel}
                {tickSync.ticksOk ? ` · [${arm.tickLower}, ${arm.tickUpper}]` : ''}
              </span>
              <span
                className={[
                  'text-[11px] font-medium rounded-full px-2 py-0.5',
                  validation.ok
                    ? 'bg-green-500/10 text-green-700 dark:text-green-400'
                    : 'bg-amber-500/10 text-amber-800 dark:text-amber-300',
                ].join(' ')}
              >
                {validation.message}
              </span>
              {dirty ? (
                <span className="text-[11px] font-medium rounded-full px-2 py-0.5 bg-amber-500/15 text-amber-800 dark:text-amber-300">
                  {t('experiment.unsavedChanges')}
                </span>
              ) : null}
              <label className="flex items-center gap-1.5 text-xs text-muted-foreground cursor-pointer">
                <input
                  type="checkbox"
                  checked={arm.enabled}
                  onChange={() => onChange({ ...arm, enabled: !arm.enabled })}
                />
                {t('experiment.armEnabled')}
              </label>
            </div>
          </div>

          <div className="grid grid-cols-2 sm:grid-cols-3 gap-2">
            {stats.slice(0, 6).map((row) => (
              <div key={row.key} className="rounded-lg bg-background/80 border border-border/50 px-3 py-2">
                <div className="text-[10px] uppercase tracking-wide text-muted-foreground">{row.label}</div>
                <div className="text-sm font-semibold mt-0.5 truncate font-mono" title={row.value}>
                  {row.value}
                </div>
              </div>
            ))}
          </div>

          <div className="flex flex-wrap gap-2">
            {dirty ? (
              <Button
                type="button"
                size="sm"
                disabled={saveMutation.isPending}
                onClick={() => saveMutation.mutate()}
                className="gap-1.5"
              >
                <Save className="h-3.5 w-3.5" />
                {saveMutation.isPending ? t('experiment.savingStrategy') : t('experiment.saveStrategyChanges')}
              </Button>
            ) : null}
            <Button type="button" variant="outline" size="sm" onClick={onChangeStrategy}>
              <RefreshCw className="h-3.5 w-3.5 mr-1.5" />
              {t('experiment.changeStrategy')}
            </Button>
            <Button type="button" variant="ghost" size="sm" onClick={() => setEditOpen((o) => !o)}>
              {editOpen ? (
                <ChevronUp className="h-3.5 w-3.5 mr-1.5" />
              ) : (
                <ChevronDown className="h-3.5 w-3.5 mr-1.5" />
              )}
              {editOpen ? t('experiment.hideParameters') : t('experiment.editParameters')}
            </Button>
            <Button type="button" variant="ghost" size="sm" onClick={onDuplicate}>
              <Copy className="h-3.5 w-3.5 mr-1.5" />
              {t('experiment.duplicateArm')}
            </Button>
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="text-muted-foreground hover:text-destructive"
              disabled={!canRemove}
              onClick={onRemove}
            >
              <Trash2 className="h-3.5 w-3.5 mr-1.5" />
              {t('experiment.removeArmShort')}
            </Button>
          </div>
          {saveError ? <InlineError>{saveError}</InlineError> : null}
          {dirty ? (
            <p className="text-xs text-muted-foreground">{t('experiment.unsavedChangesHint')}</p>
          ) : null}
        </CardHeader>

        {editOpen ? (
          <CardContent className="pt-0 border-t border-border/60 bg-background">
            <div className="pt-4">
              <ExperimentArmEditor
                arm={arm}
                index={index}
                variant="panel"
                poolAddress={arm.poolAddress}
                pool={poolMeta.pool}
                poolCurrentTick={poolMeta.poolCurrentTick}
                tickSpacing={poolMeta.tickSpacing}
                strategies={strategies}
                bollingerLoading={tickSync.bollingerLoading}
                onChange={onChange}
                onRemove={onRemove}
                canRemove={canRemove}
              />
            </div>
          </CardContent>
        ) : null}
      </Card>
    </div>
  )
}
