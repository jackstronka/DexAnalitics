import type { Position } from '@/lib/api'
import { formatUsdUncollectedFees } from '@/lib/utils'

export function parseNum(v: string | number | null | undefined): number | null {
  if (v == null) return null
  const n = typeof v === 'number' ? v : parseFloat(String(v))
  return Number.isFinite(n) ? n : null
}

/** USD price for one pool leg (matches Positions table fee / display helpers). */
export function legUsdPrice(
  position: Position,
  leg: 'a' | 'b',
  fees?: Position['uncollected_fees'],
): number {
  const label = (
    leg === 'a'
      ? (position.token_a_label ?? fees?.token_a_label ?? '')
      : (position.token_b_label ?? fees?.token_b_label ?? '')
  ).toUpperCase()
  const apiPrice = leg === 'a' ? position.token_price_a_usd : position.token_price_b_usd
  if (typeof apiPrice === 'number' && Number.isFinite(apiPrice)) return apiPrice
  if (label.includes('USDC') || label.includes('USDT')) return 1
  if (label.includes('SOL')) {
    const fromRange = parseNum(position.range_lower_usdc)
    if (fromRange !== null && fromRange > 0) return fromRange
  }
  return NaN
}

export function feeSourceLabel(
  valuationSource: string | null | undefined,
  locale: 'pl' | 'en',
): string {
  if (valuationSource === 'live_valuation') {
    return locale === 'pl' ? 'wycena live' : 'live valuation'
  }
  if (valuationSource === 'list_light') {
    return locale === 'pl' ? 'lista API (szybka)' : 'API list (fast)'
  }
  if (valuationSource === 'list_fast') {
    return locale === 'pl' ? 'lista API (szybka, wstępna)' : 'API list (fast, initial)'
  }
  if (valuationSource === 'fallback_monitor') {
    return locale === 'pl' ? 'fallback monitor' : 'fallback monitor'
  }
  return locale === 'pl' ? 'nieznane' : 'unknown'
}

/** Whether API list row has enough fields to render Value column (not monitor zero placeholder). */
export function positionValueDisplayReady(position: Position): boolean {
  const v = parseNum(position.value_usd)
  return v !== null && v > 0
}

/** Whether fee column can show a USD aggregate (not em dash). */
export function uncollectedFeesUsdDisplayReady(position: Position): boolean {
  return computeUncollectedFeesUsd(position) !== null
}

/**
 * Sum uncollected fees in USD for the table cell; `null` when legs cannot be priced.
 */
export function computeUncollectedFeesUsd(position: Position): number | null {
  const f = position.uncollected_fees
  if (!f) return null
  const a = parseFloat(String(f.amount_a))
  const b = parseFloat(String(f.amount_b))
  const priceA = legUsdPrice(position, 'a', f)
  const priceB = legUsdPrice(position, 'b', f)

  let usd = 0
  let ok = false
  if (Number.isFinite(a) && Number.isFinite(priceA)) {
    usd += a * priceA
    ok = true
  }
  if (Number.isFinite(b) && Number.isFinite(priceB)) {
    usd += b * priceB
    ok = true
  }
  return ok ? usd : null
}

export function formatUncollectedFeesCell(position: Position): string {
  const usd = computeUncollectedFeesUsd(position)
  return usd === null ? '—' : formatUsdUncollectedFees(usd)
}
