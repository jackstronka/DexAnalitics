import { describe, expect, it } from 'vitest'
import type { Strategy } from '@/lib/api'
import {
  applyGridPresetToForm,
  createComparisonArmSet,
  createExperimentArm,
  defaultExperimentArmForm,
  experimentArmSummary,
  formStateFromStrategy,
  isExperimentArmValid,
} from '@/lib/experimentArm'
import { computeArmTicksFromForm } from '@/lib/experimentArmTicks'

describe('experimentArm', () => {
  it('applies Balanced preset to threshold form', () => {
    const form = defaultExperimentArmForm('threshold', 'Balanced')
    expect(form.rebalanceThresholdPct).toBe(3)
    expect(form.minRebalanceIntervalMinutes).toBe(24 * 60)
  })

  it('creates comparison set with three distinct strategy types', () => {
    const arms = createComparisonArmSet()
    expect(arms).toHaveLength(3)
    const types = arms.map((a) => a.form.strategyType)
    expect(types).toContain('threshold')
    expect(types).toContain('bollinger')
    expect(types).toContain('last_candle')
  })

  it('builds human-readable summary', () => {
    const arm = createExperimentArm(1)
    const summary = experimentArmSummary(arm)
    expect(summary).toMatch(/threshold/)
  })

  it('validates range width when pool not ready', () => {
    const arm = createExperimentArm(1)
    arm.reuseStrategyId = 'strategy-1'
    arm.poolAddress = 'Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE'
    arm.form.rangeWidthPct = ''
    expect(isExperimentArmValid(arm, false)).toBe(false)
  })

  it('requires pool address on enabled arm', () => {
    const arm = createExperimentArm(1)
    expect(isExperimentArmValid(arm, false)).toBe(false)
    arm.poolAddress = 'Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE'
    expect(isExperimentArmValid(arm, false)).toBe(false)
    arm.reuseStrategyId = 'strategy-1'
    expect(isExperimentArmValid(arm, false)).toBe(true)
  })

  it('applyGridPresetToForm updates bollinger fields', () => {
    const base = defaultExperimentArmForm('bollinger', 'Balanced')
    const next = applyGridPresetToForm(base, 'Aggressive')
    expect(next.bollingerK).toBeTruthy()
    expect(next.minRebalanceIntervalMinutes).toBeTruthy()
  })

  it('formStateFromStrategy parses string numeric parameters from API', () => {
    const strategy = {
      id: 's1',
      name: 'Retouch shift',
      strategy_type: 'retouch_shift' as const,
      parameters: {
        range_width_pct: '2',
        retouch_offset_pct: '0',
        min_rebalance_interval_minutes: 60,
      },
      running: false,
      dry_run: false,
      auto_execute: false,
      created_at: '',
      updated_at: '',
    } as Strategy

    const form = formStateFromStrategy(strategy)
    expect(form.rangeWidthPct).toBe(2)
    expect(form.retouchOffsetPct).toBe(0)

    const ticks = computeArmTicksFromForm(form, -24443, 4)
    expect(ticks).not.toBeNull()
    expect(ticks!.tickLower).toBeLessThan(-24443)
    expect(ticks!.tickUpper).toBeGreaterThan(-24443)
  })
})
