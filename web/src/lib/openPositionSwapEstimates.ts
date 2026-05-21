import { uiPriceFromRawPriceRatio } from '@/lib/whirlpoolTicks'

/** UI balance comparison tolerance (float rounding). */
export const BALANCE_EPS = 1e-8

export function isInsufficientBalance(needUi: number, haveUi: number): boolean {
  return needUi > haveUi + BALANCE_EPS
}

/**
 * ExactIn raw input to cover `deficitUi` of `shortMint`, funded from `fundMint`.
 * Uses USD prices; +5% slippage buffer.
 */
export function estimateSwapInputRawExactIn(
  fundMint: string,
  fundDecimals: number,
  shortMint: string,
  deficitUi: number,
  pricesUsd: Record<string, number> | undefined,
): number | null {
  if (!pricesUsd || deficitUi <= 0) return null
  const pShort = pricesUsd[shortMint]
  const pFund = pricesUsd[fundMint]
  if (!pShort || !pFund || pShort <= 0 || pFund <= 0) return null
  const usdNeed = deficitUi * pShort
  const fundUi = (usdNeed / pFund) * 1.05
  if (!Number.isFinite(fundUi) || fundUi <= 0) return null
  const raw = Math.round(fundUi * 10 ** fundDecimals)
  if (raw <= 0 || raw > Number.MAX_SAFE_INTEGER) return null
  return raw
}

/**
 * Fallback from Whirlpool spot price (UI B-per-A) when USD feed is noisy.
 */
export function estimateSwapInputRawFromPoolPrice(
  deficitAUi: number,
  deficitBUi: number,
  fundIsTokenA: boolean,
  tokenADecimals: number,
  tokenBDecimals: number,
  poolPriceRaw: number,
): number | null {
  if (!Number.isFinite(poolPriceRaw) || poolPriceRaw <= 0) return null
  const bPerAUi = uiPriceFromRawPriceRatio(poolPriceRaw, tokenADecimals, tokenBDecimals)
  if (!Number.isFinite(bPerAUi) || (bPerAUi ?? 0) <= 0) return null
  const p = bPerAUi as number
  const buffer = 1.05

  if (fundIsTokenA && deficitBUi > 0) {
    const fundAUi = (deficitBUi / p) * buffer
    const raw = Math.round(fundAUi * 10 ** tokenADecimals)
    if (!Number.isFinite(raw) || raw <= 0 || raw > Number.MAX_SAFE_INTEGER) return null
    return raw
  }
  if (!fundIsTokenA && deficitAUi > 0) {
    const fundBUi = deficitAUi * p * buffer
    const raw = Math.round(fundBUi * 10 ** tokenBDecimals)
    if (!Number.isFinite(raw) || raw <= 0 || raw > Number.MAX_SAFE_INTEGER) return null
    return raw
  }
  return null
}
