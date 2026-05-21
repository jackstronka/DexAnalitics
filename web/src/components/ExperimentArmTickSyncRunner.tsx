import { useArmPool } from '@/hooks/useArmPool'
import { useExperimentArmTickSync } from '@/hooks/useExperimentArmTickSync'
import type { ExperimentArm } from '@/lib/experimentArm'

type Props = {
  arm: ExperimentArm
  onChange: (arm: ExperimentArm) => void
}

/** Invisible — keeps tick auto-sync running for every arm in the roster (not only the selected one). */
export default function ExperimentArmTickSyncRunner({ arm, onChange }: Props) {
  const poolMeta = useArmPool(arm.poolAddress)
  useExperimentArmTickSync({
    arm,
    poolAddress: arm.poolAddress,
    pool: poolMeta.pool,
    poolCurrentTick: poolMeta.poolCurrentTick,
    tickSpacing: poolMeta.tickSpacing,
    onChange,
  })
  return null
}
