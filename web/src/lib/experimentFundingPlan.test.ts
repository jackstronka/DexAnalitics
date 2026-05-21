import { describe, expect, it } from 'vitest'
import {
  aggregateTokenNeeds,
  buildExperimentFundingPlan,
  computeFundingDeficits,
  planSharedSwap,
  type ArmTokenNeed,
} from '@/lib/experimentFundingPlan'

const SOL_MINT = 'So11111111111111111111111111111111111111112'
const USDC_MINT = 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v'

function sampleArms(): ArmTokenNeed[] {
  return [
    { armId: 'a1', amount_a_raw: 50_000_000, amount_b_raw: 5_000_000 },
    { armId: 'a2', amount_a_raw: 50_000_000, amount_b_raw: 5_000_000 },
    { armId: 'a3', amount_a_raw: 50_000_000, amount_b_raw: 5_000_000 },
  ]
}

describe('experimentFundingPlan', () => {
  describe('aggregateTokenNeeds', () => {
    it('sums raw token needs across arms', () => {
      const agg = aggregateTokenNeeds(sampleArms(), 9, 6)
      expect(agg.total_a_raw).toBe(150_000_000)
      expect(agg.total_b_raw).toBe(15_000_000)
      expect(agg.total_a_ui).toBeCloseTo(0.15, 8)
      expect(agg.total_b_ui).toBeCloseTo(15, 6)
    })

    it('returns zeros for empty arms', () => {
      const agg = aggregateTokenNeeds([], 9, 6)
      expect(agg.total_a_raw).toBe(0)
      expect(agg.total_b_raw).toBe(0)
    })
  })

  describe('computeFundingDeficits', () => {
    it('detects single-leg B deficit', () => {
      const d = computeFundingDeficits(0.15, 15, 1.0, 10)
      expect(d.short_b).toBe(true)
      expect(d.short_a).toBe(false)
      expect(d.short_both).toBe(false)
      expect(d.deficit_b_ui).toBeCloseTo(5, 6)
    })

    it('detects both legs short', () => {
      const d = computeFundingDeficits(1, 10, 0.1, 1)
      expect(d.short_both).toBe(true)
    })

    it('reports no deficit when funded', () => {
      const d = computeFundingDeficits(0.15, 15, 1, 20)
      expect(d.short_a).toBe(false)
      expect(d.short_b).toBe(false)
      expect(d.deficit_a_ui).toBe(0)
      expect(d.deficit_b_ui).toBe(0)
    })
  })

  describe('planSharedSwap', () => {
    it('returns A→B swap when only token B is short', () => {
      const plan = planSharedSwap({
        mintA: SOL_MINT,
        mintB: USDC_MINT,
        symbolA: 'SOL',
        symbolB: 'USDC',
        decimalsA: 9,
        decimalsB: 6,
        totalNeedAUi: 0.15,
        totalNeedBUi: 15,
        haveAUi: 1.0,
        haveBUi: 10,
        pricesUsd: {
          [SOL_MINT]: 100,
          [USDC_MINT]: 1,
        },
      })
      expect(plan).not.toBeNull()
      expect(plan!.direction).toBe('a_to_b')
      expect(plan!.specified_mint).toBe(SOL_MINT)
      expect(plan!.amount_in).toBeGreaterThan(0)
    })

    it('returns null when both legs are short', () => {
      const plan = planSharedSwap({
        mintA: SOL_MINT,
        mintB: USDC_MINT,
        symbolA: 'SOL',
        symbolB: 'USDC',
        decimalsA: 9,
        decimalsB: 6,
        totalNeedAUi: 1,
        totalNeedBUi: 10,
        haveAUi: 0.01,
        haveBUi: 1,
      })
      expect(plan).toBeNull()
    })

    it('returns null when already funded', () => {
      const plan = planSharedSwap({
        mintA: SOL_MINT,
        mintB: USDC_MINT,
        symbolA: 'SOL',
        symbolB: 'USDC',
        decimalsA: 9,
        decimalsB: 6,
        totalNeedAUi: 0.15,
        totalNeedBUi: 15,
        haveAUi: 1,
        haveBUi: 20,
      })
      expect(plan).toBeNull()
    })
  })

  describe('buildExperimentFundingPlan', () => {
    it('combines aggregate, deficits, and recommended swap', () => {
      const plan = buildExperimentFundingPlan({
        arms: sampleArms(),
        decimalsA: 9,
        decimalsB: 6,
        mintA: SOL_MINT,
        mintB: USDC_MINT,
        symbolA: 'SOL',
        symbolB: 'USDC',
        haveAUi: 1,
        haveBUi: 10,
        pricesUsd: { [SOL_MINT]: 100, [USDC_MINT]: 1 },
      })
      expect(plan.aggregate.total_b_ui).toBeCloseTo(15, 6)
      expect(plan.deficits.short_b).toBe(true)
      expect(plan.recommended_swap).not.toBeNull()
    })
  })
})
