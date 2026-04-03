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

export function formatPercent(value: number | string): string {
  const num = typeof value === 'string' ? parseFloat(value) : value
  return `${num >= 0 ? '+' : ''}${num.toFixed(2)}%`
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
