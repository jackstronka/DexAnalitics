import { ChevronDown, ChevronUp, Minus } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { InlineError } from '@/components/ui/inline-error'
import { TooltipProvider } from '@/components/ui/tooltip'
import {
  applyGridPresetToForm,
  applyStrategyToArm,
  experimentArmSummary,
  type ExperimentArm,
  type ExperimentArmFormState,
} from '@/lib/experimentArm'
import { GRID_PRESETS, type GridPresetName } from '@/lib/gridPresets'
import {
  FIELD_ENABLED,
  FieldLabel,
  isRangeWidthSatisfied,
  STRATEGY_COPY,
  TOOLTIPS,
} from '@/lib/strategyFormShared'
import { type Pool, type Strategy, type StrategyType } from '@/lib/api'
import { useI18n } from '@/lib/i18n'

function readOptionalNumber(raw: string): number | '' {
  if (raw.trim() === '') return ''
  const n = Number(raw)
  return Number.isFinite(n) ? n : ''
}

type Props = {
  arm: ExperimentArm
  index: number
  /** Accordion (legacy) or always-open panel for stack layout. */
  variant?: 'accordion' | 'panel'
  expanded?: boolean
  onToggleExpand?: () => void
  poolAddress: string
  pool: Pool | undefined
  poolCurrentTick: number | undefined
  tickSpacing: number | undefined
  strategies: Strategy[]
  /** Parent runs tick sync (card hook); editor only shows loading state. */
  bollingerLoading?: boolean
  onChange: (arm: ExperimentArm) => void
  onRemove: () => void
  canRemove: boolean
}

export default function ExperimentArmEditor({
  arm,
  index,
  variant = 'accordion',
  expanded = false,
  onToggleExpand,
  poolAddress: _poolAddress,
  pool,
  poolCurrentTick: _poolCurrentTick,
  tickSpacing: _tickSpacing,
  strategies,
  bollingerLoading = false,
  onChange,
  onRemove,
  canRemove,
}: Props) {
  const { t, locale } = useI18n()
  const L = (pl: string, en: string) => (locale === 'pl' ? pl : en)
  const form = arm.form
  const enabled = FIELD_ENABLED[form.strategyType]
  const isBollinger = form.strategyType === 'bollinger'

  const isPanel = variant === 'panel'

  function patchForm(patch: Partial<ExperimentArmFormState>) {
    onChange({
      ...arm,
      form: { ...form, ...patch },
    })
  }

  function onStrategyTypeChange(strategyType: StrategyType) {
    onChange({
      ...arm,
      presetName: arm.presetName,
      form: {
        ...form,
        strategyType,
        rangeWidthPct: strategyType === 'bollinger' ? '' : form.rangeWidthPct || 10,
      },
      tickAutoSync: true,
    })
  }

  function onPresetClick(presetName: GridPresetName) {
    onChange({
      ...arm,
      presetName,
      form: applyGridPresetToForm(form, presetName),
    })
  }

  function onReuseStrategy(strategyId: string) {
    if (!strategyId) {
      onChange({
        ...arm,
        source: 'custom',
        reuseStrategyId: null,
      })
      return
    }
    const strategy = strategies.find((s) => s.id === strategyId)
    if (!strategy) return
    onChange(applyStrategyToArm(arm, strategy))
  }

  const rangeOk = isRangeWidthSatisfied(form.strategyType, form.rangeWidthPct)
  const ticksOk = arm.tickLower !== '' && arm.tickUpper !== ''

  const formBody = (
    <div className={isPanel ? 'space-y-4' : 'px-3 py-3 space-y-4 border-t border-border'}>
            {!isPanel ? (
            <div className="grid gap-3 md:grid-cols-2">
              <div>
                <label className="block text-sm font-medium mb-1">{L('Etykieta', 'Label')}</label>
                <input
                  className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                  value={arm.label}
                  onChange={(e) => onChange({ ...arm, label: e.target.value })}
                />
              </div>
              <div>
                <label className="block text-sm font-medium mb-1">
                  {L('Reuse strategii', 'Reuse strategy')}
                </label>
                <select
                  className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                  value={arm.reuseStrategyId ?? ''}
                  onChange={(e) => onReuseStrategy(e.target.value)}
                >
                  <option value="">{L('— nowa konfiguracja —', '— new config —')}</option>
                  {strategies.map((s) => (
                    <option key={s.id} value={s.id}>
                      {s.name} ({s.strategy_type})
                    </option>
                  ))}
                </select>
              </div>
            </div>
            ) : null}

            {!isPanel || !arm.reuseStrategyId ? (
            <div>
              <span className="text-xs text-muted-foreground block mb-1">{t('experiment.gridPresets')}</span>
              <div className="flex flex-wrap gap-1">
                {GRID_PRESETS.map((preset) => (
                  <button
                    key={preset.name}
                    type="button"
                    className={`rounded border px-2 py-1 text-xs hover:bg-accent ${
                      arm.presetName === preset.name ? 'border-primary bg-accent' : ''
                    }`}
                    onClick={() => onPresetClick(preset.name)}
                  >
                    {preset.name}
                  </button>
                ))}
              </div>
            </div>
            ) : null}

            <div>
              <FieldLabel
                htmlFor={`arm-${arm.id}-type`}
                label={L('Typ strategii', 'Strategy type')}
                tooltip={TOOLTIPS.strategyType}
              />
              <select
                id={`arm-${arm.id}-type`}
                className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                value={form.strategyType}
                onChange={(e) => onStrategyTypeChange(e.target.value as StrategyType)}
                disabled={Boolean(arm.reuseStrategyId)}
              >
                <option value="static_range">Static</option>
                <option value="periodic">Periodic</option>
                <option value="threshold">Threshold</option>
                <option value="bollinger">Bollinger</option>
                <option value="il_limit">IL Limit</option>
                <option value="oor_recenter">OOR recenter</option>
                <option value="retouch_shift">Retouch shift</option>
                <option value="last_candle">Last candle</option>
                <option value="last_candle_periodic">Last candle (periodic)</option>
              </select>
              <p className="mt-2 text-xs text-muted-foreground">{STRATEGY_COPY[form.strategyType].body}</p>
            </div>

            <div className="grid gap-3 md:grid-cols-2">
              {enabled.rangeWidth ? (
                <div>
                  <FieldLabel
                    htmlFor={`arm-${arm.id}-width`}
                    label={L('Szerokość zakresu %', 'Range width %')}
                    tooltip={TOOLTIPS.rangeWidth}
                  />
                  <input
                    id={`arm-${arm.id}-width`}
                    type="number"
                    step="0.1"
                    min={0.01}
                    max={100}
                    className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                    value={form.rangeWidthPct}
                    onChange={(e) => patchForm({ rangeWidthPct: readOptionalNumber(e.target.value) })}
                  />
                </div>
              ) : null}
              {enabled.rebalanceThreshold ? (
                <div>
                  <FieldLabel
                    htmlFor={`arm-${arm.id}-thr`}
                    label={L('Próg rebalance %', 'Rebalance threshold %')}
                    tooltip={TOOLTIPS.rebalanceThresholdThreshold}
                  />
                  <input
                    id={`arm-${arm.id}-thr`}
                    type="number"
                    step="0.1"
                    className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                    value={form.rebalanceThresholdPct}
                    onChange={(e) =>
                      patchForm({ rebalanceThresholdPct: readOptionalNumber(e.target.value) })
                    }
                  />
                </div>
              ) : null}
              {enabled.minInterval ? (
                <div>
                  <FieldLabel
                    htmlFor={`arm-${arm.id}-interval`}
                    label={L('Min. odstęp (min)', 'Min. interval (min)')}
                    tooltip={TOOLTIPS.minIntervalOther}
                  />
                  <input
                    id={`arm-${arm.id}-interval`}
                    type="number"
                    step="1"
                    min={0}
                    className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                    value={form.minRebalanceIntervalMinutes}
                    onChange={(e) =>
                      patchForm({ minRebalanceIntervalMinutes: readOptionalNumber(e.target.value) })
                    }
                  />
                </div>
              ) : null}
              {form.strategyType === 'bollinger' ? (
                <>
                  <div>
                    <FieldLabel
                      htmlFor={`arm-${arm.id}-bb-window`}
                      label="Bollinger window"
                      tooltip={TOOLTIPS.bollingerWindow}
                    />
                    <input
                      id={`arm-${arm.id}-bb-window`}
                      type="number"
                      min={2}
                      className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                      value={form.bollingerWindow}
                      onChange={(e) =>
                        patchForm({ bollingerWindow: readOptionalNumber(e.target.value) })
                      }
                    />
                  </div>
                  <div>
                    <FieldLabel htmlFor={`arm-${arm.id}-bb-k`} label="Bollinger k" tooltip={TOOLTIPS.bollingerK} />
                    <input
                      id={`arm-${arm.id}-bb-k`}
                      type="number"
                      step="0.1"
                      min={0.1}
                      className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                      value={form.bollingerK}
                      onChange={(e) => patchForm({ bollingerK: readOptionalNumber(e.target.value) })}
                    />
                  </div>
                </>
              ) : null}
              {form.strategyType === 'last_candle' || form.strategyType === 'last_candle_periodic' ? (
                <div>
                  <FieldLabel
                    htmlFor={`arm-${arm.id}-candle`}
                    label={L('Świeca (min)', 'Candle (min)')}
                    tooltip={TOOLTIPS.candleSeconds}
                  />
                  <input
                    id={`arm-${arm.id}-candle`}
                    type="number"
                    min={1}
                    className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                    value={form.candleMinutes}
                    onChange={(e) => patchForm({ candleMinutes: readOptionalNumber(e.target.value) })}
                  />
                </div>
              ) : null}
            </div>

            <div className="rounded-md border border-border bg-muted/10 px-3 py-2 space-y-2">
              <div className="flex flex-wrap items-center gap-3">
                <label className="flex items-center gap-2 text-sm">
                  <input
                    type="checkbox"
                    checked={arm.tickAutoSync}
                    onChange={(e) => onChange({ ...arm, tickAutoSync: e.target.checked })}
                  />
                  {t('experiment.tickAutoSyncLabel')}
                </label>
              </div>
              <div className="grid gap-2 md:grid-cols-2">
                <div>
                  <label className="text-xs text-muted-foreground">tick_lower</label>
                  <input
                    type="number"
                    className="w-full rounded-md border border-input bg-background px-2 py-1 text-sm font-mono"
                    value={arm.tickLower}
                    disabled={arm.tickAutoSync}
                    onChange={(e) =>
                      onChange({
                        ...arm,
                        tickLower: e.target.value === '' ? '' : Number(e.target.value),
                        tickAutoSync: false,
                      })
                    }
                  />
                </div>
                <div>
                  <label className="text-xs text-muted-foreground">tick_upper</label>
                  <input
                    type="number"
                    className="w-full rounded-md border border-input bg-background px-2 py-1 text-sm font-mono"
                    value={arm.tickUpper}
                    disabled={arm.tickAutoSync}
                    onChange={(e) =>
                      onChange({
                        ...arm,
                        tickUpper: e.target.value === '' ? '' : Number(e.target.value),
                        tickAutoSync: false,
                      })
                    }
                  />
                </div>
              </div>
              {isBollinger && bollingerLoading ? (
                <p className="text-xs text-muted-foreground">{L('Ładuję snapshoty…', 'Loading snapshots…')}</p>
              ) : null}
              {!rangeOk ? (
                <InlineError>{L('Ustaw Range Width % dla tego typu.', 'Set Range Width % for this type.')}</InlineError>
              ) : null}
              {pool && !ticksOk && arm.tickAutoSync ? (
                <InlineError>
                  {L(
                    'Ticki niedostępne — poczekaj na cenę puli / snapshoty Bollinger.',
                    'Ticks unavailable — wait for pool price / Bollinger snapshots.',
                  )}
                </InlineError>
              ) : null}
            </div>
          </div>
  )

  if (isPanel) {
    return (
      <TooltipProvider delayDuration={200}>
        {formBody}
      </TooltipProvider>
    )
  }

  return (
    <TooltipProvider delayDuration={200}>
      <div className="rounded-md border border-border overflow-hidden">
        <div className="flex flex-wrap items-center gap-2 px-3 py-2 bg-muted/20">
          <input
            type="checkbox"
            checked={arm.enabled}
            onChange={() => onChange({ ...arm, enabled: !arm.enabled })}
            aria-label={t('experiment.toggleArm').replace('{name}', arm.label)}
          />
          <button
            type="button"
            className="flex flex-1 items-center gap-2 text-left min-w-0"
            onClick={onToggleExpand}
          >
            {expanded ? (
              <ChevronUp className="h-4 w-4 shrink-0" />
            ) : (
              <ChevronDown className="h-4 w-4 shrink-0" />
            )}
            <span className="text-sm font-medium truncate">{arm.label}</span>
            <span className="text-xs text-muted-foreground truncate">{experimentArmSummary(arm)}</span>
          </button>
          {pool && ticksOk ? (
            <span className="text-xs font-mono text-muted-foreground">
              [{arm.tickLower}, {arm.tickUpper}]
            </span>
          ) : null}
          <Button
            type="button"
            size="icon"
            variant="ghost"
            disabled={!canRemove}
            onClick={onRemove}
            aria-label={t('experiment.removeArm').replace('{n}', String(index + 1))}
          >
            <Minus className="h-4 w-4" />
          </Button>
        </div>

        {expanded ? formBody : null}
      </div>
    </TooltipProvider>
  )
}
