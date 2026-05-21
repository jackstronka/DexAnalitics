import { useEffect, useMemo, useRef } from 'react'
import { useQuery } from '@tanstack/react-query'
import type { ExperimentArm } from '@/lib/experimentArm'
import { computeArmTicksFromForm } from '@/lib/experimentArmTicks'
import { getDataSnapshots, type Pool } from '@/lib/api'

export type ArmTickSyncStatus =
  | 'waiting_pool'
  | 'manual'
  | 'syncing_bollinger'
  | 'computing'
  | 'ready'

export function deriveArmTickSyncStatus(
  arm: ExperimentArm,
  poolAddress: string,
  poolReady: boolean,
  poolCurrentTick: number | undefined,
  tickSpacing: number | undefined,
  bollingerLoading: boolean,
  isBollinger: boolean,
): ArmTickSyncStatus {
  if (!arm.tickAutoSync) return 'manual'
  if (!poolAddress.trim() || !poolReady || poolCurrentTick == null || tickSpacing == null) {
    return 'waiting_pool'
  }
  if (isBollinger && bollingerLoading) return 'syncing_bollinger'
  if (arm.tickLower !== '' && arm.tickUpper !== '') return 'ready'
  return 'computing'
}

export function describeArmTickSyncStatus(
  status: ArmTickSyncStatus,
  locale: 'pl' | 'en',
): string {
  const pl = locale === 'pl'
  switch (status) {
    case 'waiting_pool':
      return pl ? 'Najpierw wybierz parę' : 'Select pair first'
    case 'manual':
      return pl ? 'Zakres ręczny' : 'Manual range'
    case 'syncing_bollinger':
      return pl ? 'Liczenie zakresu (Bollinger)…' : 'Computing range (Bollinger)…'
    case 'computing':
      return pl ? 'Liczenie zakresu…' : 'Computing range…'
    case 'ready':
      return pl ? 'Zakres gotowy' : 'Range ready'
  }
}

type Args = {
  arm: ExperimentArm
  poolAddress: string
  pool: Pool | undefined
  poolCurrentTick: number | undefined
  tickSpacing: number | undefined
  onChange: (arm: ExperimentArm) => void
}

export function useExperimentArmTickSync({
  arm,
  poolAddress,
  pool,
  poolCurrentTick,
  tickSpacing,
  onChange,
}: Args) {
  const isBollinger = arm.form.strategyType === 'bollinger'
  const bollingerWindowPoints = Math.max(2, Math.round(Number(arm.form.bollingerWindow || 20)))

  const bollingerSnapshotsQ = useQuery({
    queryKey: ['data-snapshots-bollinger-arm', poolAddress, arm.id, bollingerWindowPoints],
    queryFn: () =>
      getDataSnapshots({
        protocol: 'orca',
        pool: poolAddress,
        limit: Math.max(120, bollingerWindowPoints * 4),
      }),
    enabled: isBollinger && poolAddress.trim().length > 0 && arm.tickAutoSync,
    staleTime: 20_000,
  })

  const bollingerPrices = useMemo(() => {
    return (bollingerSnapshotsQ.data?.rows ?? [])
      .map((r) => Number(r.price_ab))
      .filter((v) => Number.isFinite(v) && v > 0)
  }, [bollingerSnapshotsQ.data])

  const armRef = useRef(arm)
  armRef.current = arm
  const onChangeRef = useRef(onChange)
  onChangeRef.current = onChange

  useEffect(() => {
    const current = armRef.current
    if (!current.tickAutoSync || !pool || tickSpacing == null || poolCurrentTick == null) {
      return
    }
    const ticks = computeArmTicksFromForm(
      current.form,
      poolCurrentTick,
      tickSpacing,
      isBollinger ? bollingerPrices : undefined,
    )
    if (!ticks) return
    if (current.tickLower === ticks.tickLower && current.tickUpper === ticks.tickUpper) return
    onChangeRef.current({
      ...current,
      tickLower: ticks.tickLower,
      tickUpper: ticks.tickUpper,
    })
    // eslint-disable-next-line react-hooks/exhaustive-deps -- sync ticks from pool/form
  }, [
    arm.tickAutoSync,
    arm.id,
    arm.tickLower,
    arm.tickUpper,
    arm.form,
    pool,
    tickSpacing,
    poolCurrentTick,
    isBollinger,
    bollingerPrices,
  ])

  const ticksOk = arm.tickLower !== '' && arm.tickUpper !== ''

  const status: ArmTickSyncStatus = deriveArmTickSyncStatus(
    arm,
    poolAddress,
    Boolean(pool && poolAddress.trim()),
    poolCurrentTick,
    tickSpacing,
    isBollinger && bollingerSnapshotsQ.isLoading,
    isBollinger,
  )

  return {
    status,
    ticksOk,
    bollingerLoading: isBollinger && bollingerSnapshotsQ.isLoading,
    bollingerPrices,
  }
}
