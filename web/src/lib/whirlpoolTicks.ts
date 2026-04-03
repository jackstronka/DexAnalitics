/**
 * Tick math aligned with `clmm_lp_protocols::orca::pool_reader::calculate_tick_range`
 * (Orca Whirlpool: price ∝ 1.0001^tick).
 */

const LN_10001 = Math.log(1.0001)

/** Jak `tick_to_price` w Orca: stosunek ceny = 1.0001^tick (mint B za 1 mint A w kolejności puli). */
export function tickToPriceRatio(tick: number): number {
  return Math.pow(1.0001, tick)
}

/**
 * Orca's `price` derived from tick is a ratio of **raw** token amounts.
 * Convert it to a UI ratio (token B per 1 token A), adjusting for SPL decimals:
 * `P_raw = B_raw / A_raw` and `P_ui = B_ui / A_ui = P_raw * 10^(decA - decB)`.
 */
export function uiPriceFromRawPriceRatio(
  rawPriceRatio: number,
  decimalsA: number,
  decimalsB: number,
): number | null {
  if (!Number.isFinite(rawPriceRatio) || rawPriceRatio <= 0) return null
  const diff = decimalsA - decimalsB
  // Avoid exponent blow-ups for unexpected decimals.
  if (!Number.isFinite(diff) || Math.abs(diff) > 64) return null
  return rawPriceRatio * 10 ** diff
}

/**
 * Inverse of [`uiPriceFromRawPriceRatio`].
 * `P_raw = P_ui * 10^(decB - decA)`.
 */
export function rawPriceRatioFromUiPrice(
  uiPrice: number,
  decimalsA: number,
  decimalsB: number,
): number | null {
  if (!Number.isFinite(uiPrice) || uiPrice <= 0) return null
  const diff = decimalsB - decimalsA
  if (!Number.isFinite(diff) || Math.abs(diff) > 64) return null
  return uiPrice * 10 ** diff
}

/** Wyświetlanie stosunku ceny z ticka (unika śmieci przy bardzo małych/wielkich wartościach). */
export function formatPriceRatio(p: number): string {
  if (!Number.isFinite(p) || p <= 0) {
    return '—'
  }
  const abs = Math.abs(p)
  if (abs >= 1e10 || abs < 1e-10) {
    return p.toExponential(6)
  }
  return p.toLocaleString(undefined, { maximumFractionDigits: 8 })
}

/**
 * @param currentTick Pool `tick_current`
 * @param rangeWidthPct Total price width in **percent** (e.g. `1` = 1%), same as strategy `range_width_pct`
 * @param tickSpacing From pool metadata
 */
export function calculateTickRangeFromWidthPct(
  currentTick: number,
  rangeWidthPct: number,
  tickSpacing: number,
): { tickLower: number; tickUpper: number } {
  const widthFraction = rangeWidthPct / 100
  const halfWidth = widthFraction / 2
  const tickDelta = Math.trunc(Math.abs(halfWidth / LN_10001))
  const spacing = tickSpacing
  const lower = Math.trunc((currentTick - tickDelta) / spacing) * spacing
  const upper = (Math.trunc((currentTick + tickDelta) / spacing) + 1) * spacing
  return { tickLower: lower, tickUpper: upper }
}

/**
 * Raw tick index from price ratio `P = 1.0001^tick` (mint B per 1 mint A), before spacing alignment.
 */
export function priceRatioToTickFloat(price: number): number {
  if (!Number.isFinite(price) || price <= 0) {
    return Number.NaN
  }
  return Math.log(price) / LN_10001
}

/**
 * Aligns a price ratio to Whirlpool tick indexes (multiples of `tickSpacing`).
 * Lower edge uses floor, upper edge uses ceil so the **numeric** range [priceLower, priceUpper] is covered.
 * If `priceLower > priceUpper`, values are swapped.
 */
export function alignPriceRatioToTicks(
  priceLower: number,
  priceUpper: number,
  tickSpacing: number,
): { tickLower: number; tickUpper: number } | null {
  if (!Number.isFinite(priceLower) || !Number.isFinite(priceUpper) || priceLower <= 0 || priceUpper <= 0) {
    return null
  }
  let lo = priceLower
  let hi = priceUpper
  if (lo > hi) {
    const t = lo
    lo = hi
    hi = t
  }
  const rawLo = priceRatioToTickFloat(lo)
  const rawHi = priceRatioToTickFloat(hi)
  if (!Number.isFinite(rawLo) || !Number.isFinite(rawHi)) {
    return null
  }
  const s = tickSpacing
  let tickLower = Math.floor(rawLo / s) * s
  let tickUpper = Math.ceil(rawHi / s) * s
  if (tickLower >= tickUpper) {
    tickUpper = tickLower + s
  }
  return { tickLower, tickUpper }
}

/** String for numeric inputs (avoids overly long `toFixed` chains for tiny ratios). */
export function priceRatioToInputString(p: number): string {
  if (!Number.isFinite(p) || p <= 0) {
    return ''
  }
  const abs = Math.abs(p)
  if (abs >= 1e-12 && abs < 1e15) {
    return String(Number(p.toPrecision(14)))
  }
  return p.toExponential(12)
}
