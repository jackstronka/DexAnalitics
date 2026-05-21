import type { Strategy, StrategyParameters, StrategyType } from '@/lib/api'
import { MAX_EXPERIMENT_ARMS } from '@/lib/experimentCapital'
import {
  GRID_PRESETS,
  medianCsvNumber,
  parseFirstCsvNumber,
  type GridPresetName,
} from '@/lib/gridPresets'
import { buildParameters, isRangeWidthSatisfied } from '@/lib/strategyFormShared'

export type ExperimentArmSource = 'custom' | 'reuse_strategy'

export type ExperimentArmFormState = {
  strategyType: StrategyType
  rangeWidthPct: number | ''
  maxIlPct: number | ''
  rebalanceThresholdPct: number | ''
  minRebalanceIntervalMinutes: number | ''
  retouchOffsetPct: number | ''
  candleMinutes: number | ''
  bollingerWindow: number | ''
  bollingerK: number | ''
  periodicRequiresOutOfRange: boolean
  rebalanceOnRangeExitImmediately: boolean
  autoStart: boolean
}

export type ExperimentArm = {
  id: string
  enabled: boolean
  label: string
  /** Whirlpool address for this strategy arm. */
  poolAddress: string
  source: ExperimentArmSource
  presetName: GridPresetName | null
  reuseStrategyId: string | null
  form: ExperimentArmFormState
  tickLower: number | ''
  tickUpper: number | ''
  tickAutoSync: boolean
  /** Target USD for this arm (quote-open-budget). */
  budgetUsd: number | ''
}

export function defaultExperimentArmForm(
  strategyType: StrategyType = 'threshold',
  presetName: GridPresetName = 'Balanced',
): ExperimentArmFormState {
  const form: ExperimentArmFormState = {
    strategyType,
    rangeWidthPct: 10,
    maxIlPct: '',
    rebalanceThresholdPct: '',
    minRebalanceIntervalMinutes: '',
    retouchOffsetPct: '',
    candleMinutes: '',
    bollingerWindow: 20,
    bollingerK: 2,
    periodicRequiresOutOfRange: false,
    rebalanceOnRangeExitImmediately: true,
    autoStart: true,
  }
  return applyGridPresetToForm(form, presetName)
}

export function applyGridPresetToForm(
  form: ExperimentArmFormState,
  presetName: GridPresetName,
): ExperimentArmFormState {
  const preset = GRID_PRESETS.find((p) => p.name === presetName)
  if (!preset) return form

  const next: ExperimentArmFormState = { ...form }

  switch (form.strategyType) {
    case 'threshold':
    case 'oor_recenter':
    case 'retouch_shift':
      next.rebalanceThresholdPct = parseFirstCsvNumber(preset.thresholdGridPct)
      next.minRebalanceIntervalMinutes = hoursCsvToMinutes(preset.periodicGridHours)
      break
    case 'periodic':
      next.minRebalanceIntervalMinutes = hoursCsvToMinutes(preset.periodicGridHours)
      break
    case 'bollinger':
      next.bollingerWindow = Math.round(medianCsvNumber(preset.bollingerWindowGrid) || 20)
      next.bollingerK = medianCsvNumber(preset.bollingerKGrid) || 2
      next.minRebalanceIntervalMinutes = hoursCsvToMinutes(preset.bollingerRebalanceHoursGrid)
      break
    case 'last_candle':
    case 'last_candle_periodic': {
      const candleSec = parseFirstCsvNumber(preset.lastCandleSecondsGrid)
      next.candleMinutes = candleSec === '' ? '' : Math.max(1, Math.round(Number(candleSec) / 60))
      next.minRebalanceIntervalMinutes = secondsCsvToMinutes(preset.lastCandleRebalanceSecondsGrid)
      break
    }
    case 'il_limit':
      next.rebalanceThresholdPct = parseFirstCsvNumber(preset.thresholdGridPct)
      next.maxIlPct = 2
      next.minRebalanceIntervalMinutes = hoursCsvToMinutes(preset.periodicGridHours)
      break
    default:
      break
  }

  return next
}

function hoursCsvToMinutes(raw: string): number | '' {
  const h = parseFirstCsvNumber(raw)
  if (h === '') return ''
  return Math.round(Number(h) * 60)
}

function secondsCsvToMinutes(raw: string): number | '' {
  const sec = parseFirstCsvNumber(raw)
  if (sec === '') return ''
  return Math.max(1, Math.round(Number(sec) / 60))
}

export function formStateFromStrategy(strategy: Strategy): ExperimentArmFormState {
  const p = strategy.parameters ?? {}
  const minutes =
    typeof p.min_rebalance_interval_minutes === 'number'
      ? p.min_rebalance_interval_minutes
      : typeof p.min_rebalance_interval_hours === 'number'
        ? p.min_rebalance_interval_hours * 60
        : ''

  return {
    strategyType: strategy.strategy_type,
    rangeWidthPct: numOrEmpty(p.range_width_pct),
    maxIlPct: numOrEmpty(p.max_il_pct),
    rebalanceThresholdPct: numOrEmpty(p.rebalance_threshold_pct),
    minRebalanceIntervalMinutes: minutes === '' ? '' : minutes,
    retouchOffsetPct: numOrEmpty(p.retouch_offset_pct),
    candleMinutes:
      typeof p.candle_seconds === 'number' && p.candle_seconds > 0
        ? Math.round(p.candle_seconds / 60)
        : '',
    bollingerWindow: numOrEmpty(p.bollinger_window),
    bollingerK: numOrEmpty(p.bollinger_k),
    periodicRequiresOutOfRange: p.periodic_requires_out_of_range === true,
    rebalanceOnRangeExitImmediately: p.rebalance_on_range_exit_immediately !== false,
    autoStart: p.auto_start !== false,
  }
}

function numOrEmpty(v: unknown): number | '' {
  if (typeof v === 'number' && Number.isFinite(v)) return v
  if (typeof v === 'string' && v.trim() !== '') {
    const n = Number(v)
    if (Number.isFinite(n)) return n
  }
  return ''
}

export function buildArmParameters(form: ExperimentArmFormState) {
  return buildParameters(form.strategyType, form)
}

export function experimentArmSummary(arm: ExperimentArm): string {
  const t = arm.form.strategyType.replace(/_/g, ' ')
  const bits: string[] = [t]
  const p = buildArmParameters(arm.form)
  if (typeof p.rebalance_threshold_pct === 'number') bits.push(`thr ${p.rebalance_threshold_pct}%`)
  if (typeof p.range_width_pct === 'number') bits.push(`width ${p.range_width_pct}%`)
  if (typeof p.bollinger_k === 'number') bits.push(`k=${p.bollinger_k}`)
  if (typeof p.min_rebalance_interval_minutes === 'number') {
    bits.push(`every ${p.min_rebalance_interval_minutes}m`)
  }
  if (arm.presetName) bits.push(arm.presetName)
  if (arm.reuseStrategyId) bits.push('reuse')
  return bits.join(' · ')
}

export type ComparisonArmTemplate = {
  id: string
  label: string
  strategyType: StrategyType
  presetName: GridPresetName
}

export const COMPARISON_ARM_TEMPLATES: ComparisonArmTemplate[] = [
  { id: 'thr', label: 'Threshold 5%', strategyType: 'threshold', presetName: 'Balanced' },
  { id: 'boll', label: 'Bollinger', strategyType: 'bollinger', presetName: 'Balanced' },
  { id: 'lc', label: 'Last candle', strategyType: 'last_candle', presetName: 'Balanced' },
]

export function createExperimentArmFromTemplate(
  template: ComparisonArmTemplate,
  sequence: number,
): ExperimentArm {
  const form = defaultExperimentArmForm(template.strategyType, template.presetName)
  if (template.strategyType === 'threshold') {
    form.rebalanceThresholdPct = 5
  }
  return {
    id: crypto.randomUUID(),
    enabled: true,
    label: template.label || `Arm ${sequence}`,
    poolAddress: '',
    source: 'custom',
    presetName: template.presetName,
    reuseStrategyId: null,
    form,
    tickLower: '',
    tickUpper: '',
    tickAutoSync: true,
    budgetUsd: '',
  }
}

export function createExperimentArm(sequence: number): ExperimentArm {
  return createExperimentArmFromTemplate(
    { id: 'default', label: `Arm ${sequence}`, strategyType: 'threshold', presetName: 'Balanced' },
    sequence,
  )
}

export function createComparisonArmSet(): ExperimentArm[] {
  return COMPARISON_ARM_TEMPLATES.map((tpl, i) => createExperimentArmFromTemplate(tpl, i + 1))
}

export function applyStrategyToArm(arm: ExperimentArm, strategy: Strategy): ExperimentArm {
  const poolFromStrategy = strategy.pool_address?.trim() ?? ''
  return {
    ...arm,
    source: 'reuse_strategy',
    reuseStrategyId: strategy.id,
    presetName: null,
    label: strategy.name,
    poolAddress: poolFromStrategy || arm.poolAddress,
    form: formStateFromStrategy(strategy),
    tickLower: '',
    tickUpper: '',
    tickAutoSync: true,
    budgetUsd: arm.budgetUsd,
  }
}

export function parametersFromArm(arm: ExperimentArm): StrategyParameters {
  return buildArmParameters(arm.form)
}

export function canAddExperimentArm(arms: ExperimentArm[]): boolean {
  return arms.length < MAX_EXPERIMENT_ARMS
}

export function ensureMinOneArm(arms: ExperimentArm[]): ExperimentArm[] {
  if (arms.length > 0) return arms
  return [createExperimentArm(1)]
}

export function isExperimentArmValid(arm: ExperimentArm, poolMetadataReady = false): boolean {
  if (!arm.enabled) return true
  if (!arm.reuseStrategyId) return false
  if (!arm.poolAddress.trim()) return false
  if (!isRangeWidthSatisfied(arm.form.strategyType, arm.form.rangeWidthPct)) {
    return false
  }
  if (poolMetadataReady && (arm.tickLower === '' || arm.tickUpper === '')) {
    return false
  }
  return true
}

export function areExperimentArmsValid(arms: ExperimentArm[], poolMetadataReady = false): boolean {
  const enabled = arms.filter((a) => a.enabled)
  if (enabled.length === 0) return false
  return enabled.every((a) => isExperimentArmValid(a, poolMetadataReady))
}

/** All enabled arms share one non-empty pool, or null if mixed / missing. */
export function resolveExperimentCommonPool(arms: ExperimentArm[]): string | null {
  const enabled = arms.filter((a) => a.enabled)
  if (enabled.length === 0) return null
  const pools = [...new Set(enabled.map((a) => a.poolAddress.trim()).filter(Boolean))]
  if (pools.length !== 1) return null
  return pools[0]!
}
