import { shortenAddress } from '@/lib/utils'

export const WSOL_MINT = 'So11111111111111111111111111111111111111112'
export const USDC_MINT = 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v'

export function defaultTokenDecimals(mint: string): number {
  if (mint === WSOL_MINT) return 9
  if (mint === USDC_MINT) return 6
  return 9
}

export function formatRawAmount(raw: string, decimals?: number | null): string {
  const dec = decimals ?? 9
  try {
    const n = BigInt(raw.trim())
    const neg = n < 0n
    const abs = neg ? -n : n
    const base = 10n ** BigInt(dec)
    const whole = abs / base
    const frac = abs % base
    const fracStr = frac.toString().padStart(dec, '0').replace(/0+$/, '')
    const ui = fracStr ? `${whole}.${fracStr}` : whole.toString()
    return neg ? `-${ui}` : ui
  } catch {
    return raw
  }
}

export function rawToUiNumber(raw: string, decimals?: number | null): number | null {
  const s = formatRawAmount(raw, decimals)
  const v = parseFloat(s)
  return Number.isFinite(v) ? v : null
}

/** Spend cap for reopen: positive inventory only (matches executor SESSION caps). */
export function sessionSpendCapUi(raw: string, decimals?: number | null): number {
  const ui = rawToUiNumber(raw, decimals)
  if (ui === null) return 0
  return Math.max(0, ui)
}

export function mintSymbol(mint: string): string {
  if (mint === WSOL_MINT) return 'SOL'
  if (mint === USDC_MINT) return 'USDC'
  return shortenAddress(mint, 4)
}

export function humanSessionSource(source: string, locale: 'pl' | 'en'): string {
  if (source === 'gl_session_shadow') {
    return locale === 'pl' ? 'księga GL' : 'GL ledger'
  }
  if (source === 'gl_session_shadow_pslr_fallback') {
    return locale === 'pl' ? 'suma lifecycle (GL pusta)' : 'lifecycle sum (GL empty)'
  }
  return source
}
