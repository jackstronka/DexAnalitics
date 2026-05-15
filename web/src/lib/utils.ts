import { type ClassValue, clsx } from "clsx"
import { twMerge } from "tailwind-merge"

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs))
}

export function formatUSD(value: number | string): string {
  const num = typeof value === 'string' ? parseFloat(value) : value
  return new Intl.NumberFormat('en-US', {
    style: 'currency',
    currency: 'USD',
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  }).format(num)
}

/** USD with a fixed number of fraction digits (e.g. small on-chain fees in lamports). */
export function formatUsdFixed(value: number | string, fractionDigits: number): string {
  const num = typeof value === 'string' ? parseFloat(value) : value
  if (!Number.isFinite(num)) return '—'
  return new Intl.NumberFormat('en-US', {
    style: 'currency',
    currency: 'USD',
    minimumFractionDigits: fractionDigits,
    maximumFractionDigits: fractionDigits,
  }).format(num)
}

/** Qualities where a numeric `0` means “unknown”, not a literal zero-dollar mark. */
const LINEAGE_USD_MISSING_QUALITIES = new Set(['missing_inputs', 'missing_price'])

/**
 * Lineage node USD from API: show `—` only when non-finite / empty, or when value is exactly 0
 * and quality indicates missing valuation (backend uses 0 as sentinel).
 */
export function formatLineageStoredValueUsd(
  usd: string | number | null | undefined,
  valuationQuality: string | null | undefined,
  fractionDigits: number,
): string {
  if (usd == null) return '—'
  const s = String(usd).trim()
  if (s === '') return '—'
  const num = typeof usd === 'number' ? usd : parseFloat(s)
  if (!Number.isFinite(num)) return '—'
  const q = (valuationQuality ?? '').trim().toLowerCase()
  if (num === 0 && LINEAGE_USD_MISSING_QUALITIES.has(q)) return '—'
  return formatUsdFixed(num, fractionDigits)
}

/** True when principal Δ should be hidden (unknown leg). */
export function isLineageStoredUsdMissing(
  usd: string | number | null | undefined,
  valuationQuality: string | null | undefined,
): boolean {
  if (usd == null) return true
  const s = String(usd).trim()
  if (s === '') return true
  const num = typeof usd === 'number' ? usd : parseFloat(s)
  if (!Number.isFinite(num)) return true
  const q = (valuationQuality ?? '').trim().toLowerCase()
  if (num === 0 && LINEAGE_USD_MISSING_QUALITIES.has(q)) return true
  return false
}

export type LineageOpeningUsdExtra = {
  /** When the lineage table has a single node, `totals.baseline_value_usd` may still carry the chain start mark. */
  singleNodeTotalsBaselineUsd?: string | number | null
}

/** Start column: API baseline when known; else lifecycle ledger open-quote USD; else single-node `totals.baseline`. */
export function formatLineageOpeningUsdDisplay(
  n: { position_address: string; baseline_value_usd: string; baseline_valuation_quality?: string | null },
  ledgerOpenQuoteUsd: ReadonlyMap<string, number> | undefined,
  fractionDigits: number,
  extra?: LineageOpeningUsdExtra,
): { text: string; source: 'api' | 'ledger' | 'totals' | 'none' } {
  if (!isLineageStoredUsdMissing(n.baseline_value_usd, n.baseline_valuation_quality)) {
    return {
      text: formatLineageStoredValueUsd(n.baseline_value_usd, n.baseline_valuation_quality, fractionDigits),
      source: 'api',
    }
  }
  const pk = n.position_address.trim()
  const v = ledgerOpenQuoteUsd?.get(pk)
  if (v != null && Number.isFinite(v) && v > 0) {
    return { text: formatUsdFixed(v, fractionDigits), source: 'ledger' }
  }
  const tb = extra?.singleNodeTotalsBaselineUsd
  if (tb != null && String(tb).trim() !== '') {
    const num = typeof tb === 'number' ? tb : parseFloat(String(tb).trim())
    if (Number.isFinite(num) && num > 0) {
      return { text: formatUsdFixed(num, fractionDigits), source: 'totals' }
    }
  }
  return { text: '—', source: 'none' }
}

/** Principal Δ using ledger open-quote USD as baseline when API baseline is still missing. */
export function formatPrincipalDeltaForLineageNode(
  n: {
    position_address: string
    baseline_value_usd: string
    baseline_valuation_quality?: string | null
    current_value_usd: string
    current_valuation_quality?: string | null
  },
  ledgerOpenQuoteUsd: ReadonlyMap<string, number> | undefined,
  fractionDigits = 3,
  extra?: LineageOpeningUsdExtra,
): string {
  let bUsd: string | number = n.baseline_value_usd
  let bQ = n.baseline_valuation_quality
  if (isLineageStoredUsdMissing(bUsd, bQ)) {
    const v = ledgerOpenQuoteUsd?.get(n.position_address.trim())
    if (v != null && Number.isFinite(v) && v > 0) {
      bUsd = v
      bQ = 'exact'
    } else {
      const t = extra?.singleNodeTotalsBaselineUsd
      if (t != null && String(t).trim() !== '') {
        const num = typeof t === 'number' ? t : parseFloat(String(t).trim())
        if (Number.isFinite(num) && num > 0) {
          bUsd = num
          bQ = 'exact'
        }
      }
    }
  }
  return formatPrincipalDeltaUsdOrDash(bUsd, bQ, n.current_value_usd, n.current_valuation_quality, fractionDigits)
}

/** Principal Δ = end − start; "—" if either leg is unknown per lineage quality rules. */
export function formatPrincipalDeltaUsdOrDash(
  baselineUsd: string | number,
  baselineQuality: string | null | undefined,
  currentUsd: string | number,
  currentQuality: string | null | undefined,
  fractionDigits = 3,
): string {
  if (
    isLineageStoredUsdMissing(baselineUsd, baselineQuality) ||
    isLineageStoredUsdMissing(currentUsd, currentQuality)
  ) {
    return '—'
  }
  const a = typeof baselineUsd === 'string' ? parseFloat(baselineUsd) : baselineUsd
  const b = typeof currentUsd === 'string' ? parseFloat(currentUsd) : currentUsd
  return formatUsdFixed(b - a, fractionDigits)
}

/** Decimal/string fields from API — empty or non-numeric → "—", else fixed USD. */
export function formatUsdField(
  value: string | number | null | undefined,
  fractionDigits: number,
): string {
  if (value == null) return '—'
  const s = String(value).trim()
  if (s === '') return '—'
  const num = typeof value === 'number' ? value : parseFloat(s)
  if (!Number.isFinite(num)) return '—'
  return formatUsdFixed(num, fractionDigits)
}

/**
 * Stream-lineage "Fees zebrane" main USD fragment: sub-cent fees use extra decimals; when there were
 * collects but USD is 0 (missing mint prices / rounding), keep "—" for the USD figure — callers add token rows + note.
 */
export function formatLineageFeesCollectedUsdMain(
  feesUsd: string | number | null | undefined,
  collectEvents: number,
): string {
  const c = collectEvents ?? 0
  const raw = feesUsd == null ? '' : String(feesUsd).trim()
  const usd = raw === '' ? NaN : parseFloat(raw)
  if (!Number.isFinite(usd)) return '—'
  if (c === 0) return formatUsdFixed(usd, 4)
  if (usd > 0) return formatUsdUncollectedFees(usd)
  return '—'
}

/** USD spot for one token leg — extra decimals when price is tiny. */
export function formatUsdTokenSpot(value: number | string | null | undefined): string {
  if (value == null) return '—'
  const num = typeof value === 'string' ? parseFloat(value) : value
  if (!Number.isFinite(num)) return '—'
  const abs = Math.abs(num)
  const frac = abs >= 100 ? 2 : abs >= 1 ? 4 : abs >= 0.01 ? 5 : 6
  return formatUsdFixed(num, frac)
}

/**
 * USD for uncollected LP fees: 3 dp for normal amounts; for sub-cent non-zero values use 6 dp so
 * e.g. $0.0007 does not show as $0.000.
 */
export function formatUsdUncollectedFees(value: number | string): string {
  const num = typeof value === 'string' ? parseFloat(value) : value
  if (!Number.isFinite(num)) return '—'
  const abs = Math.abs(num)
  const fractionDigits = abs === 0 ? 3 : abs < 0.01 ? 6 : 3
  return new Intl.NumberFormat('en-US', {
    style: 'currency',
    currency: 'USD',
    minimumFractionDigits: fractionDigits,
    maximumFractionDigits: fractionDigits,
  }).format(num)
}

export function formatPercent(value: number | string): string {
  const num = typeof value === 'string' ? parseFloat(value) : value
  return `${num >= 0 ? '+' : ''}${num.toFixed(2)}%`
}

export function formatPercentFixed(value: number | string, fractionDigits: number): string {
  const num = typeof value === 'string' ? parseFloat(value) : value
  if (!Number.isFinite(num)) return '—'
  const abs = Math.abs(num)
  // If the value is extremely small, increase precision so we don't show "-0.000%".
  // Example: -0.0013% should render as "-0.0013%" (4 dp), not "-0.000%".
  const dynamicDigits = abs > 0 && abs < 0.01 ? Math.max(fractionDigits, 5) : fractionDigits
  return `${num >= 0 ? '+' : ''}${num.toFixed(dynamicDigits)}%`
}

export function formatNumber(value: number | string, decimals = 2): string {
  const num = typeof value === 'string' ? parseFloat(value) : value
  return new Intl.NumberFormat('en-US', {
    minimumFractionDigits: decimals,
    maximumFractionDigits: decimals,
  }).format(num)
}

/** Position price range when API sends `range_*_usdc` (USDC per 1 of the other token). */
export function formatUsdcPriceRange(
  lo: number | string | null | undefined,
  hi: number | string | null | undefined,
  quote?: string | null,
): string | null {
  if (lo == null || hi == null) return null
  const a = typeof lo === 'string' ? parseFloat(lo) : lo
  const b = typeof hi === 'string' ? parseFloat(hi) : hi
  if (!Number.isFinite(a) || !Number.isFinite(b)) return null
  const fmt = (n: number) =>
    new Intl.NumberFormat('en-US', {
      style: 'currency',
      currency: 'USD',
      minimumFractionDigits: 3,
      maximumFractionDigits: 3,
    }).format(n)
  const core = `${fmt(a)} – ${fmt(b)} USDC`
  return quote ? `${core} ${quote}` : core
}

/** Token-price range when API sends `range_*_price` (token B per 1 token A in UI units). */
export function formatTokenPriceRange(
  lo: number | string | null | undefined,
  hi: number | string | null | undefined,
  quote?: string | null,
): string | null {
  if (lo == null || hi == null) return null
  const a = typeof lo === 'string' ? parseFloat(lo) : lo
  const b = typeof hi === 'string' ? parseFloat(hi) : hi
  if (!Number.isFinite(a) || !Number.isFinite(b)) return null
  const abs = Math.max(Math.abs(a), Math.abs(b))
  const frac = abs >= 100 ? 3 : abs >= 1 ? 6 : abs >= 0.01 ? 8 : 10
  const fmt = (n: number) =>
    new Intl.NumberFormat('en-US', {
      minimumFractionDigits: frac,
      maximumFractionDigits: frac,
    }).format(n)
  const core = `${fmt(a)} – ${fmt(b)}`
  return quote ? `${core} ${quote}` : core
}

/** Invert a token-price range: [lo, hi] (B per 1 A) -> [1/hi, 1/lo] (A per 1 B). */
export function formatInvertedTokenPriceRange(
  lo: number | string | null | undefined,
  hi: number | string | null | undefined,
  quote?: string | null,
): string | null {
  if (lo == null || hi == null) return null
  const a = typeof lo === 'string' ? parseFloat(lo) : lo
  const b = typeof hi === 'string' ? parseFloat(hi) : hi
  if (!Number.isFinite(a) || !Number.isFinite(b) || a <= 0 || b <= 0) return null
  const invLo = 1 / b
  const invHi = 1 / a
  if (!Number.isFinite(invLo) || !Number.isFinite(invHi) || invLo <= 0 || invHi <= 0) return null

  let invQuote: string | null | undefined = quote
  if (quote && quote.includes(' per 1 ')) {
    const [bLabel, aLabel] = quote.split(' per 1 ')
    if (bLabel && aLabel) {
      invQuote = `${aLabel} per 1 ${bLabel}`
    }
  }
  return formatTokenPriceRange(invLo, invHi, invQuote)
}

export function shortenAddress(address: string, chars = 4): string {
  return `${address.slice(0, chars)}...${address.slice(-chars)}`
}

export function formatDate(date: string | Date): string {
  const d = typeof date === 'string' ? new Date(date) : date
  return d.toLocaleDateString('en-US', {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  })
}

/** Tooltip for lineage `*_raw` fee fields: integer smallest on-chain units, not a separate “raw” token. */
export const FEE_BASE_UNITS_TOOLTIP =
  'Liczba całkowita w najmniejszych jednostkach on-chain tego tokenu (np. lamporty dla SOL/WSOL). To nie jest osobny „raw SOL” — nadal SOL/WSOL, tylko w skali atomowej (jak w transakcji).'

/** Returns a short clause for UI, or `null` if backend did not send a usable raw counter. */
export function formatFeeBaseUnitsClause(baseUnits: unknown): string | null {
  if (baseUnits === null || baseUnits === undefined) return null
  const s = String(baseUnits).trim()
  if (s === '' || s === 'undefined' || s === 'null') return null
  return `(baz. jedn.: ${s})`
}
