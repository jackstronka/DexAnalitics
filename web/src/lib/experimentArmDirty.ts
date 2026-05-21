import type { CreateStrategyRequest, Strategy } from '@/lib/api'
import {
  formStateFromStrategy,
  parametersFromArm,
  type ExperimentArm,
  type ExperimentArmFormState,
} from '@/lib/experimentArm'

function formsEqual(a: ExperimentArmFormState, b: ExperimentArmFormState): boolean {
  return JSON.stringify(a) === JSON.stringify(b)
}

/** True when arm edits differ from the linked saved strategy (params, pool, or name). */
export function isArmDirty(arm: ExperimentArm, strategy: Strategy | undefined): boolean {
  if (!arm.reuseStrategyId || !strategy || strategy.id !== arm.reuseStrategyId) {
    return false
  }
  if (arm.poolAddress.trim() !== (strategy.pool_address?.trim() ?? '')) return true
  if (arm.label.trim() !== strategy.name.trim()) return true
  return isArmParamsDirty(arm, strategy)
}

/** Form parameters only — used to block launch (pool/label are experiment-local until saved). */
export function isArmParamsDirty(arm: ExperimentArm, strategy: Strategy | undefined): boolean {
  if (!arm.reuseStrategyId || !strategy || strategy.id !== arm.reuseStrategyId) {
    return false
  }
  return !formsEqual(arm.form, formStateFromStrategy(strategy))
}

export function buildStrategyUpdateFromArm(
  arm: ExperimentArm,
  strategy: Strategy,
): CreateStrategyRequest {
  return {
    name: arm.label.trim() || strategy.name,
    strategy_type: arm.form.strategyType,
    parameters: parametersFromArm(arm),
    pool_address: arm.poolAddress.trim() || null,
    auto_execute: arm.form.autoStart,
    dry_run: strategy.dry_run ?? false,
  }
}
