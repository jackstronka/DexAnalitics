/** Pure display helpers for Dip Agent snapshot UI (U1). */

export type DipOrdersEmptyKind = 'no_jwt' | 'no_api_key' | 'error' | 'skipped' | 'empty'

/**
 * When there are no open Buy-the-Dip rows, classify why for CTA / empty copy.
 * Returns `null` when the table should show rows.
 */
export function dipOrdersEmptyKind(
  ordersSource: string,
  orderCount: number,
): DipOrdersEmptyKind | null {
  if (orderCount > 0) return null
  switch (ordersSource.trim()) {
    case 'no_jwt':
      return 'no_jwt'
    case 'no_api_key':
      return 'no_api_key'
    case 'error':
      return 'error'
    case 'skipped':
      return 'skipped'
    default:
      return 'empty'
  }
}

/** Normalize zone label from API (`A`|`B`|`C`|`none`). */
export function normalizeDipZone(zone: string): string {
  const z = zone.trim().toUpperCase()
  if (z === 'A' || z === 'B' || z === 'C') return z
  if (z === 'NONE' || z === '') return 'none'
  return zone.trim() || 'none'
}

export function formatDipUsd(n: number | null | undefined, digits = 2): string {
  if (n == null || !Number.isFinite(n)) return '—'
  return `$${n.toFixed(digits)}`
}

export function formatDipZoneBand(min: number, max: number): string {
  return `${formatDipUsd(min)} – ${formatDipUsd(max)}`
}
