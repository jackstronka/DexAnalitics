import type { ExperimentArm } from '@/lib/experimentArm'
import { isExperimentArmValid } from '@/lib/experimentArm'
import { isRangeWidthSatisfied } from '@/lib/strategyFormShared'

export type ArmValidation = {
  ok: boolean
  message: string | null
}

export function describeArmValidation(
  arm: ExperimentArm,
  poolReady: boolean,
  locale: 'pl' | 'en',
): ArmValidation {
  const pl = locale === 'pl'
  if (!arm.enabled) {
    return { ok: true, message: pl ? 'Wyłączona z launchu' : 'Excluded from launch' }
  }
  if (!arm.poolAddress.trim()) {
    return {
      ok: false,
      message: pl ? 'Wybierz parę tokenów' : 'Select token pair',
    }
  }
  if (!arm.reuseStrategyId) {
    return {
      ok: false,
      message: pl ? 'Wybierz zapisaną strategię' : 'Pick a saved strategy',
    }
  }
  if (!isRangeWidthSatisfied(arm.form.strategyType, arm.form.rangeWidthPct)) {
    return {
      ok: false,
      message: pl ? 'Ustaw szerokość zakresu (Range Width %)' : 'Set range width %',
    }
  }
  if (poolReady && (arm.tickLower === '' || arm.tickUpper === '')) {
    return {
      ok: false,
      message: pl ? 'Oczekiwanie na ticki puli…' : 'Waiting for pool ticks…',
    }
  }
  if (!isExperimentArmValid(arm, poolReady)) {
    return { ok: false, message: pl ? 'Uzupełnij parametry' : 'Complete parameters' }
  }
  return { ok: true, message: pl ? 'Gotowa do launchu' : 'Ready to launch' }
}
