/**
 * Dip ladder preview math (3Commas-like safety-order model A).
 * Pure / hermetic — used by `/dip` UI before any Trigger create.
 */

export const DIP_LADDER_MIN_ORDER_USD_DEFAULT = 10

export type DipLadderParams = {
  baseOrderUsd: number
  safetyOrderUsd: number
  /** SO1 distance from entry (%). */
  deviationPct: number
  /** Geometric step scale for cumulative deviation (1 = linear). */
  deviationStepScale: number
  /** Multiplier for SO size: SO_n = safety * volumeScale^(n-1). */
  volumeScale: number
  maxSafetyOrders: number
  takeProfitPct: number
  /** TP from weighted avg (default) or from base/entry. */
  tpBasis?: 'avg' | 'base'
  minOrderUsd?: number
  /** Wallet/vault quote available; null/undefined = unknown. */
  availableUsd?: number | null
}

export type DipLadderRow = {
  level: string
  kind: 'bo' | 'so' | 'tp'
  /** Cumulative % below entry for buys; TP % for take-profit row. */
  deviationPct: number
  orderSizeQuote: number
  orderSizeBase: number
  estPrice: number
  avgEntryAfter: number
  tpPriceAfter: number
  cumulativeQuote: number
}

export type DipLadderProposeBlockReason = 'required_gt_available' | 'below_min' | 'invalid_entry'

export type DipLadderSummaryMetrics = {
  maxPriceDropPct: number
  maxAvgEntryDeviationPct: number
  requiredUsd: number
  availableUsd: number | null
  proposeDisabled: boolean
  proposeDisabledReason: DipLadderProposeBlockReason | null
}

export type DipLadderPreview = {
  rows: DipLadderRow[]
  summary: DipLadderSummaryMetrics
}

/** Cumulative deviation % for safety order index `n` (1-based), 3Commas-style. */
export function cumulativeSafetyDeviationPct(
  n: number,
  deviationPct: number,
  stepScale: number,
): number {
  if (n <= 0) return 0
  if (!Number.isFinite(deviationPct) || !Number.isFinite(stepScale)) return NaN
  if (Math.abs(stepScale - 1) < 1e-12) return deviationPct * n
  return (deviationPct * (Math.pow(stepScale, n) - 1)) / (stepScale - 1)
}

function tpPriceFrom(avgEntry: number, entry: number, tpPct: number, basis: 'avg' | 'base'): number {
  const base = basis === 'base' ? entry : avgEntry
  return base * (1 + tpPct / 100)
}

/**
 * Build BO + SO1…SOn (+ trailing TP preview row) from entry USD and ladder params.
 */
export function buildDipLadderPreview(
  entryPriceUsd: number,
  params: DipLadderParams,
): DipLadderPreview {
  const minOrder = params.minOrderUsd ?? DIP_LADDER_MIN_ORDER_USD_DEFAULT
  const tpBasis = params.tpBasis ?? 'avg'
  const available =
    params.availableUsd == null || !Number.isFinite(params.availableUsd)
      ? null
      : params.availableUsd

  const empty = (reason: DipLadderProposeBlockReason | null): DipLadderPreview => ({
    rows: [],
    summary: {
      maxPriceDropPct: 0,
      maxAvgEntryDeviationPct: 0,
      requiredUsd: 0,
      availableUsd: available,
      proposeDisabled: true,
      proposeDisabledReason: reason,
    },
  })

  if (!Number.isFinite(entryPriceUsd) || entryPriceUsd <= 0) {
    return empty('invalid_entry')
  }

  const maxSo = Math.max(0, Math.floor(params.maxSafetyOrders))
  const rows: DipLadderRow[] = []
  let totalQuote = 0
  let totalBase = 0
  let belowMin = false

  const pushBuy = (
    level: string,
    kind: 'bo' | 'so',
    deviationPct: number,
    quote: number,
    price: number,
  ) => {
    if (!(quote > 0) || !(price > 0)) return
    if (quote + 1e-9 < minOrder) belowMin = true
    const baseAmt = quote / price
    totalQuote += quote
    totalBase += baseAmt
    const avg = totalQuote / totalBase
    rows.push({
      level,
      kind,
      deviationPct,
      orderSizeQuote: quote,
      orderSizeBase: baseAmt,
      estPrice: price,
      avgEntryAfter: avg,
      tpPriceAfter: tpPriceFrom(avg, entryPriceUsd, params.takeProfitPct, tpBasis),
      cumulativeQuote: totalQuote,
    })
  }

  pushBuy('BO', 'bo', 0, params.baseOrderUsd, entryPriceUsd)

  for (let n = 1; n <= maxSo; n++) {
    const cumDev = cumulativeSafetyDeviationPct(
      n,
      params.deviationPct,
      params.deviationStepScale,
    )
    const price = entryPriceUsd * (1 - cumDev / 100)
    if (!(price > 0)) break
    const quote = params.safetyOrderUsd * Math.pow(params.volumeScale, n - 1)
    pushBuy(`SO${n}`, 'so', cumDev, quote, price)
  }

  const finalAvg = totalBase > 0 ? totalQuote / totalBase : entryPriceUsd
  const maxDrop = rows.length > 0 ? Math.max(...rows.map((r) => r.deviationPct)) : 0
  const maxAvgDev =
    entryPriceUsd > 0 ? ((entryPriceUsd - finalAvg) / entryPriceUsd) * 100 : 0

  if (rows.length > 0 && Number.isFinite(params.takeProfitPct)) {
    const tpPx = tpPriceFrom(finalAvg, entryPriceUsd, params.takeProfitPct, tpBasis)
    rows.push({
      level: 'TP1',
      kind: 'tp',
      deviationPct: params.takeProfitPct,
      orderSizeQuote: 0,
      orderSizeBase: totalBase,
      estPrice: tpPx,
      avgEntryAfter: finalAvg,
      tpPriceAfter: tpPx,
      cumulativeQuote: totalQuote,
    })
  }

  let reason: DipLadderProposeBlockReason | null = null
  if (belowMin) reason = 'below_min'
  else if (available != null && totalQuote > available + 1e-9) reason = 'required_gt_available'

  return {
    rows,
    summary: {
      maxPriceDropPct: maxDrop,
      maxAvgEntryDeviationPct: maxAvgDev,
      requiredUsd: totalQuote,
      availableUsd: available,
      proposeDisabled: reason != null,
      proposeDisabledReason: reason,
    },
  }
}
