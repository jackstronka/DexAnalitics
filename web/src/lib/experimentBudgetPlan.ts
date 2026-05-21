import type { QuoteOpenBudgetResponse } from '@/lib/api'
import type { ExperimentArm } from '@/lib/experimentArm'
import { splitBudgetEqual, type AllocationMode } from '@/lib/experimentCapital'
import type { ArmTokenNeed } from '@/lib/experimentFundingPlan'

export type ArmBudgetQuote = {
  arm: ExperimentArm
  budgetUsd: number
  quote: QuoteOpenBudgetResponse | undefined
  quoteError: Error | null
  isLoading: boolean
}

/** Resolve per-arm USD budget from total + mode. */
export function resolveArmBudgetsUsd(
  arms: ExperimentArm[],
  totalBudgetUsd: number | '',
  allocationMode: AllocationMode,
): Map<string, number> {
  const enabled = arms.filter((a) => a.enabled)
  const map = new Map<string, number>()

  if (totalBudgetUsd === '' || !Number.isFinite(Number(totalBudgetUsd)) || Number(totalBudgetUsd) <= 0) {
    return map
  }

  const total = Number(totalBudgetUsd)

  if (allocationMode === 'equal') {
    const splits = splitBudgetEqual(total, enabled.length)
    enabled.forEach((arm, i) => map.set(arm.id, splits[i] ?? 0))
    return map
  }

  if (allocationMode === 'fixed_usd') {
    for (const arm of enabled) {
      const b = arm.budgetUsd
      if (b !== '' && Number(b) > 0) map.set(arm.id, Number(b))
    }
    return map
  }

  // percent mode: derive from arm.budgetUsd as absolute (user sets per arm)
  for (const arm of enabled) {
    const b = arm.budgetUsd
    if (b !== '' && Number(b) > 0) map.set(arm.id, Number(b))
  }
  return map
}

export function armTokenNeedsFromQuotes(rows: ArmBudgetQuote[]): ArmTokenNeed[] {
  return rows
    .filter((r) => r.quote)
    .map((r) => ({
      armId: r.arm.id,
      amount_a_raw: r.quote!.token_max_a,
      amount_b_raw: r.quote!.token_max_b,
    }))
}

export function quotesReady(rows: ArmBudgetQuote[]): boolean {
  const enabled = rows.length
  if (enabled === 0) return false
  return rows.every((r) => !r.isLoading && !r.quoteError && !!r.quote)
}
