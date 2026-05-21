import {
  alignPriceRatioToTicks,
  calculateTickRangeFromWidthPct,
  expandAlignedTickRangeToIncludeCurrent,
} from '@/lib/whirlpoolTicks'
import type { ExperimentArmFormState } from '@/lib/experimentArm'

export type TickPair = { tickLower: number; tickUpper: number }

export function computeBollingerTicksFromPrices(
  prices: number[],
  windowPoints: number,
  bollingerK: number,
  tickSpacing: number,
  liveTick: number | null,
): TickPair | null {
  if (prices.length < windowPoints) return null
  if (!Number.isFinite(bollingerK) || bollingerK <= 0) return null

  const values = prices.slice(-windowPoints)
  const mean = values.reduce((acc, v) => acc + v, 0) / values.length
  if (!Number.isFinite(mean) || mean <= 0) return null
  const variance = values.reduce((acc, v) => {
    const d = v - mean
    return acc + d * d
  }, 0) / values.length
  const sigma = Math.sqrt(variance)

  let loPrice = mean - bollingerK * sigma
  let hiPrice = mean + bollingerK * sigma
  if (!Number.isFinite(loPrice) || !Number.isFinite(hiPrice)) return null
  if (loPrice <= 0) loPrice = Math.max(mean, 1e-12) * 0.999
  if (hiPrice <= loPrice) hiPrice = loPrice * 1.001

  const aligned = alignPriceRatioToTicks(loPrice, hiPrice, tickSpacing)
  if (!aligned) return null
  if (liveTick == null) return aligned
  return expandAlignedTickRangeToIncludeCurrent(
    aligned.tickLower,
    aligned.tickUpper,
    liveTick,
    tickSpacing,
  )
}

export function computeArmTicksFromForm(
  form: ExperimentArmFormState,
  tickCurrent: number,
  tickSpacing: number,
  bollingerPrices?: number[],
): TickPair | null {
  if (form.strategyType === 'bollinger') {
    const windowPoints = Math.max(2, Math.round(Number(form.bollingerWindow || 20)))
    const k = Number(form.bollingerK || 2)
    if (!bollingerPrices) return null
    return computeBollingerTicksFromPrices(
      bollingerPrices,
      windowPoints,
      k,
      tickSpacing,
      tickCurrent,
    )
  }

  const width = form.rangeWidthPct
  if (width === '' || Number(width) <= 0) return null
  return calculateTickRangeFromWidthPct(tickCurrent, Number(width), tickSpacing)
}
