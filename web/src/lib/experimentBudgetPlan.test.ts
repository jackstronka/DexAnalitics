import { describe, expect, it } from 'vitest'
import type { ExperimentArm } from '@/lib/experimentArm'
import { createExperimentArm } from '@/lib/experimentArm'
import { armTokenNeedsFromQuotes, resolveArmBudgetsUsd } from '@/lib/experimentBudgetPlan'

function armWithBudget(id: string, enabled: boolean, budgetUsd: number | ''): ExperimentArm {
  const base = createExperimentArm(1)
  return { ...base, id, enabled, budgetUsd }
}

describe('experimentBudgetPlan', () => {
  it('splits total budget equally across enabled arms', () => {
    const arms = [
      armWithBudget('a1', true, ''),
      armWithBudget('a2', true, ''),
      armWithBudget('a3', false, ''),
    ]
    const map = resolveArmBudgetsUsd(arms, 30, 'equal')
    expect(map.get('a1')).toBe(15)
    expect(map.get('a2')).toBe(15)
    expect(map.has('a3')).toBe(false)
  })

  it('builds token needs from quote rows', () => {
    const arm = createExperimentArm(1)
    const rows = [
      {
        arm,
        budgetUsd: 10,
        quote: {
          token_max_a: 100,
          token_max_b: 200,
          amount_a: 90,
          amount_b: 180,
          amount_a_ui: 0.1,
          amount_b_ui: 0.2,
          estimated_value_usd: 10,
          liquidity: '1',
          in_range: true,
        },
        quoteError: null,
        isLoading: false,
      },
    ]
    expect(armTokenNeedsFromQuotes(rows)).toEqual([
      { armId: arm.id, amount_a_raw: 100, amount_b_raw: 200 },
    ])
  })
})
