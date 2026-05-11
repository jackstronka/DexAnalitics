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

/** Backend often uses 0 for “unknown”; lineage tables show that as "—" via `usdOrDash`. */
export function isUsdValueMissingForLineage(v: string | number): boolean {
  const n = typeof v === 'string' ? parseFloat(v) : v
  return !Number.isFinite(n) || n === 0
}

/** Principal Δ = end − start; "—" if either leg is unknown (matches start/end column rules). */
export function formatPrincipalDeltaUsdOrDash(
  baselineUsd: string | number,
  currentUsd: string | number,
  fractionDigits = 3,
): string {
  if (isUsdValueMissingForLineage(baselineUsd) || isUsdValueMissingForLineage(currentUsd)) {
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
