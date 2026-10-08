import { describe, expect, it } from 'vitest'
import {
  BALANCE_EPS,
  estimateSwapInputRawExactIn,
  estimateSwapInputRawFromPoolPrice,
  isInsufficientBalance,
} from '@/lib/openPositionSwapEstimates'
import { USDC_MINT, WSOL_MINT } from '@/lib/sessionCapital'

describe('openPositionSwapEstimates', () => {
  it('flags insufficient balance past the UI epsilon', () => {
    expect(isInsufficientBalance(1, 1)).toBe(false)
    expect(isInsufficientBalance(1, 1 + BALANCE_EPS)).toBe(false)
    expect(isInsufficientBalance(1, 1 - 2 * BALANCE_EPS)).toBe(true)
  })

  it('sizes ExactIn raw with a 5% USD buffer', () => {
    const prices = { [USDC_MINT]: 1, [WSOL_MINT]: 100 }
    // Need 2 USDC → $2 → $2.10 of SOL at $100 → 0.021 SOL → 21_000_000 lamports
    expect(estimateSwapInputRawExactIn(WSOL_MINT, 9, USDC_MINT, 2, prices)).toBe(21_000_000)
    expect(estimateSwapInputRawExactIn(WSOL_MINT, 9, USDC_MINT, 2, undefined)).toBeNull()
    expect(estimateSwapInputRawExactIn(WSOL_MINT, 9, USDC_MINT, 0, prices)).toBeNull()
  })

  it('falls back to pool UI price (B per A) with the same 5% buffer', () => {
    // UI B/A = 100 USDC per SOL (raw 1 * 10^(9-6) = 1000 would be different; pass already-UI via helper)
    // poolPriceRaw such that uiPriceFromRawPriceRatio(raw, 9, 6) = 100
    // ui = raw * 10^(9-6) = raw * 1000 → raw = 0.1
    const fromB = estimateSwapInputRawFromPoolPrice(0, 10, true, 9, 6, 0.1)
    // fund A to cover 10 USDC: (10 / 100) * 1.05 = 0.105 SOL → 105_000_000 lamports
    expect(fromB).toBe(105_000_000)

    const fromA = estimateSwapInputRawFromPoolPrice(0.1, 0, false, 9, 6, 0.1)
    // fund B to cover 0.1 SOL: 0.1 * 100 * 1.05 = 10.5 USDC → 10_500_000 raw
    expect(fromA).toBe(10_500_000)

    expect(estimateSwapInputRawFromPoolPrice(1, 1, true, 9, 6, 0)).toBeNull()
  })
})
