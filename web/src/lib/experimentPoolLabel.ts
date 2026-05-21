import { CURATED_ORCA_POOLS } from '@/lib/curatedPools'
import { shortenAddress } from '@/lib/utils'

export function poolLabelForAddress(address: string): string {
  const trimmed = address.trim()
  if (!trimmed) return '—'
  const curated = CURATED_ORCA_POOLS.find((p) => p.address === trimmed)
  if (curated) {
    const pair = curated.label.split('(')[0]?.trim()
    return pair || curated.label
  }
  return shortenAddress(trimmed, 4)
}
