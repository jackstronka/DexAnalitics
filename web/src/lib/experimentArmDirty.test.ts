import { describe, expect, it } from 'vitest'
import { createExperimentArm, formStateFromStrategy } from '@/lib/experimentArm'
import { buildStrategyUpdateFromArm, isArmDirty, isArmParamsDirty } from '@/lib/experimentArmDirty'
import type { Strategy } from '@/lib/api'

function mockStrategy(overrides: Partial<Strategy> = {}): Strategy {
  return {
    id: 's1',
    name: 'Test strategy',
    strategy_type: 'threshold',
    pool_address: 'Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE',
    running: false,
    dry_run: true,
    auto_execute: false,
    parameters: {
      range_width_pct: 10,
      rebalance_threshold_pct: 5,
      auto_start: true,
    },
    created_at: '',
    updated_at: '',
    ...overrides,
  }
}

describe('experimentArmDirty', () => {
  it('detects parameter edits', () => {
    const strategy = mockStrategy()
    const arm = createExperimentArm(1)
    arm.reuseStrategyId = strategy.id
    arm.poolAddress = strategy.pool_address!
    arm.label = strategy.name
    arm.form = formStateFromStrategy(strategy)
    expect(isArmDirty(arm, strategy)).toBe(false)
    arm.form.rebalanceThresholdPct = 7
    expect(isArmDirty(arm, strategy)).toBe(true)
    expect(isArmParamsDirty(arm, strategy)).toBe(true)
  })

  it('pool-only change is dirty for save badge but not launch params', () => {
    const strategy = mockStrategy({ pool_address: undefined })
    const arm = createExperimentArm(1)
    arm.reuseStrategyId = strategy.id
    arm.poolAddress = 'Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE'
    arm.label = strategy.name
    arm.form = formStateFromStrategy(strategy)
    expect(isArmDirty(arm, strategy)).toBe(true)
    expect(isArmParamsDirty(arm, strategy)).toBe(false)
  })

  it('builds update payload from arm', () => {
    const strategy = mockStrategy()
    const arm = createExperimentArm(1)
    arm.reuseStrategyId = strategy.id
    arm.poolAddress = strategy.pool_address!
    arm.label = strategy.name
    arm.form = formStateFromStrategy(strategy)
    const payload = buildStrategyUpdateFromArm(arm, strategy)
    expect(payload.name).toBe('Test strategy')
    expect(payload.strategy_type).toBe('threshold')
    expect(payload.pool_address).toBe(strategy.pool_address)
    expect(payload.dry_run).toBe(true)
  })
})
