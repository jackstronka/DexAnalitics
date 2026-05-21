import { describe, expect, it } from 'vitest'
import {
  DEFAULT_MIN_ARM_BUDGET_USD,
  MAX_EXPERIMENT_ARMS,
  isBelowMinArmBudget,
  splitBudgetByPercent,
  splitBudgetEqual,
  sumArmBudgets,
  validateArmBudgets,
} from '@/lib/experimentCapital'

describe('experimentCapital', () => {
  describe('splitBudgetEqual', () => {
    it('splits 30 USD across 3 arms equally', () => {
      expect(splitBudgetEqual(30, 3)).toEqual([10, 10, 10])
    })

    it('assigns remainder cents to the last arm when not evenly divisible', () => {
      const parts = splitBudgetEqual(30, 7)
      expect(parts).toHaveLength(7)
      expect(sumArmBudgets(parts)).toBeCloseTo(30, 6)
      expect(parts.slice(0, 6).every((p) => p === 4.28)).toBe(true)
      expect(parts[6]).toBe(4.32)
    })

    it('returns empty array for zero arms', () => {
      expect(splitBudgetEqual(30, 0)).toEqual([])
    })

    it('returns full total for a single arm', () => {
      expect(splitBudgetEqual(30, 1)).toEqual([30])
    })

    it('rejects non-positive total or arm count', () => {
      expect(() => splitBudgetEqual(0, 3)).toThrow()
      expect(() => splitBudgetEqual(-1, 3)).toThrow()
      expect(() => splitBudgetEqual(30, -1)).toThrow()
    })
  })

  describe('splitBudgetByPercent', () => {
    it('splits by explicit percents that sum to 100', () => {
      expect(splitBudgetByPercent(30, [50, 30, 20])).toEqual([15, 9, 6])
    })

    it('rejects percents that do not sum to ~100', () => {
      expect(() => splitBudgetByPercent(30, [50, 30])).toThrow()
    })
  })

  describe('validateArmBudgets', () => {
    it('accepts when sum equals total', () => {
      const r = validateArmBudgets(30, [10, 10, 10])
      expect(r.valid).toBe(true)
      expect(r.exceedsTotal).toBe(false)
    })

    it('rejects when sum exceeds total', () => {
      const r = validateArmBudgets(30, [15, 15, 15])
      expect(r.valid).toBe(false)
      expect(r.exceedsTotal).toBe(true)
    })
  })

  describe('isBelowMinArmBudget', () => {
    it('warns below default floor', () => {
      expect(isBelowMinArmBudget(4.99)).toBe(true)
      expect(isBelowMinArmBudget(5)).toBe(false)
    })

    it('respects custom minimum', () => {
      expect(isBelowMinArmBudget(9, 10)).toBe(true)
    })
  })

  describe('constants', () => {
    it('exports documented limits', () => {
      expect(DEFAULT_MIN_ARM_BUDGET_USD).toBe(5)
      expect(MAX_EXPERIMENT_ARMS).toBe(8)
    })
  })
})
