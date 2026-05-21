import type { QuoteOpenBudgetResponse } from '@/lib/api'
import type { ExperimentArm } from '@/lib/experimentArm'
import { parametersFromArm } from '@/lib/experimentArm'
import type { ExperimentArmLaunchSpec } from '@/lib/experimentLaunch'

export function makeCostSessionId(): string {
  if (typeof crypto !== 'undefined' && 'randomUUID' in crypto) {
    return crypto.randomUUID()
  }
  return `${Date.now()}-${Math.random().toString(16).slice(2)}`
}

export function makeExperimentBatchId(): string {
  return makeCostSessionId()
}

export function experimentStrategyName(arm: ExperimentArm, batchId: string): string {
  const prefix = batchId.slice(0, 8)
  const label = arm.label.trim() || arm.id.slice(0, 8)
  return `exp-${prefix}-${label}`
}

export function buildExperimentArmLaunchSpecs(
  arms: ExperimentArm[],
  _budgetByArmId: Map<string, number>,
  quotesByArmId: Map<string, QuoteOpenBudgetResponse>,
  batchId: string,
): ExperimentArmLaunchSpec[] {
  return arms
    .filter((a) => a.enabled)
    .map((arm) => {
      const quote = quotesByArmId.get(arm.id)
      if (!quote || arm.tickLower === '' || arm.tickUpper === '') {
        throw new Error(`Missing quote or ticks for arm "${arm.label}"`)
      }

      const spec: ExperimentArmLaunchSpec = {
        armId: arm.id,
        enabled: true,
        tick_lower: Number(arm.tickLower),
        tick_upper: Number(arm.tickUpper),
        amount_a: quote.token_max_a,
        amount_b: quote.token_max_b,
        cost_session_id: makeCostSessionId(),
      }

      if (arm.source === 'reuse_strategy' && arm.reuseStrategyId) {
        spec.strategyId = arm.reuseStrategyId
      } else {
        spec.createStrategy = {
          name: experimentStrategyName(arm, batchId),
          strategy_type: arm.form.strategyType,
          parameters: parametersFromArm(arm) as Record<string, unknown>,
          auto_execute: arm.form.autoStart,
          dry_run: false,
        }
      }

      return spec
    })
}

export const LS_EXPERIMENT_BATCHES = 'clmm.experiment_batches'

export type StoredExperimentBatch = {
  batchId: string
  poolAddress: string
  createdAt: string
  sharedSwapSignature?: string
  arms: Array<{
    armId: string
    label: string
    status: string
    positionPda?: string
    strategyId?: string
    costSessionId?: string
    error?: string
  }>
}

export function persistExperimentBatch(batch: StoredExperimentBatch): void {
  if (typeof window === 'undefined') return
  try {
    const raw = window.localStorage.getItem(LS_EXPERIMENT_BATCHES)
    const prev: StoredExperimentBatch[] = raw ? JSON.parse(raw) : []
    const next = [batch, ...prev.filter((b) => b.batchId !== batch.batchId)].slice(0, 20)
    window.localStorage.setItem(LS_EXPERIMENT_BATCHES, JSON.stringify(next))
  } catch {
    // ignore quota / parse errors
  }
}
