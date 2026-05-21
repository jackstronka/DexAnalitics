import { describe, expect, it } from 'vitest'
import { createExperimentArm } from '@/lib/experimentArm'
import {
  buildExperimentArmLaunchSpecs,
  experimentStrategyName,
} from '@/lib/experimentLaunchSpecs'

describe('experimentLaunchSpecs', () => {
  it('builds launch specs with createStrategy for custom arms', () => {
    const arm = createExperimentArm(1)
    arm.tickLower = -100
    arm.tickUpper = 100
    const quotes = new Map([
      [
        arm.id,
        {
          token_max_a: 1_000_000,
          token_max_b: 2_000_000,
          amount_a: 900_000,
          amount_b: 1_800_000,
          amount_a_ui: 0.001,
          amount_b_ui: 2,
          estimated_value_usd: 10,
          liquidity: '123',
          in_range: true,
        },
      ],
    ])
    const budgets = new Map([[arm.id, 10]])
    const specs = buildExperimentArmLaunchSpecs([arm], budgets, quotes, 'batch-abc-123')
    expect(specs).toHaveLength(1)
    expect(specs[0].amount_a).toBe(1_000_000)
    expect(specs[0].createStrategy?.name).toBe(experimentStrategyName(arm, 'batch-abc-123'))
    expect(specs[0].createStrategy?.strategy_type).toBe('threshold')
  })

  it('reuses strategy id when arm source is reuse_strategy', () => {
    const arm = createExperimentArm(1)
    arm.source = 'reuse_strategy'
    arm.reuseStrategyId = 'strat-existing'
    arm.tickLower = 0
    arm.tickUpper = 10
    const quotes = new Map([
      [
        arm.id,
        {
          token_max_a: 100,
          token_max_b: 200,
          amount_a: 90,
          amount_b: 180,
          amount_a_ui: 0.0001,
          amount_b_ui: 0.18,
          estimated_value_usd: 10,
          liquidity: '1',
          in_range: true,
        },
      ],
    ])
    const specs = buildExperimentArmLaunchSpecs([arm], new Map([[arm.id, 10]]), quotes, 'batch-1')
    expect(specs[0].strategyId).toBe('strat-existing')
    expect(specs[0].createStrategy).toBeUndefined()
  })
})
