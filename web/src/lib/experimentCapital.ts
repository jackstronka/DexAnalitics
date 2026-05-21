/** Soft minimum USD per experiment arm (UI warning). */
export const DEFAULT_MIN_ARM_BUDGET_USD = 5

/** Maximum arms in one experiment (v1). */
export const MAX_EXPERIMENT_ARMS = 8

const BUDGET_SUM_EPS = 1e-6
const PERCENT_SUM_EPS = 0.01

export type AllocationMode = 'equal' | 'percent' | 'fixed_usd'

/**
 * Split `totalUsd` equally across `armCount` arms.
 * Remainder from floating division is added to the last arm.
 */
export function splitBudgetEqual(totalUsd: number, armCount: number): number[] {
  if (!Number.isFinite(totalUsd) || totalUsd <= 0) {
    throw new Error('totalUsd must be a positive finite number')
  }
  if (!Number.isInteger(armCount) || armCount < 0) {
    throw new Error('armCount must be a non-negative integer')
  }
  if (armCount === 0) return []

  const totalCents = Math.round(totalUsd * 100)
  const baseCents = Math.floor(totalCents / armCount)
  const remainderCents = totalCents - baseCents * armCount

  return Array.from({ length: armCount }, (_, i) => {
    const cents = baseCents + (i === armCount - 1 ? remainderCents : 0)
    return cents / 100
  })
}

/** Split `totalUsd` by percents that must sum to ~100. */
export function splitBudgetByPercent(totalUsd: number, percents: number[]): number[] {
  if (!Number.isFinite(totalUsd) || totalUsd <= 0) {
    throw new Error('totalUsd must be a positive finite number')
  }
  if (percents.length === 0) {
    throw new Error('percents must not be empty')
  }
  const sumPct = percents.reduce((a, b) => a + b, 0)
  if (Math.abs(sumPct - 100) > PERCENT_SUM_EPS) {
    throw new Error('percents must sum to 100')
  }
  return percents.map((p) => (totalUsd * p) / 100)
}

export function sumArmBudgets(budgets: number[]): number {
  return budgets.reduce((acc, v) => acc + v, 0)
}

export function validateArmBudgets(
  totalBudgetUsd: number,
  armBudgetsUsd: number[],
): { valid: boolean; sum: number; exceedsTotal: boolean } {
  const sum = sumArmBudgets(armBudgetsUsd)
  const exceedsTotal = sum > totalBudgetUsd + BUDGET_SUM_EPS
  return {
    valid: !exceedsTotal,
    sum,
    exceedsTotal,
  }
}

export function isBelowMinArmBudget(
  armBudgetUsd: number,
  minUsd: number = DEFAULT_MIN_ARM_BUDGET_USD,
): boolean {
  return armBudgetUsd < minUsd - BUDGET_SUM_EPS
}

export function rawToUi(raw: number, decimals: number): number {
  return raw / 10 ** decimals
}

export function uiToRaw(ui: number, decimals: number): number {
  return Math.round(ui * 10 ** decimals)
}
