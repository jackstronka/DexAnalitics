import { describe, expect, it } from 'vitest'
import {
  alignPriceRatioToTicks,
  calculateTickRangeFromWidthPct,
  expandAlignedTickRangeToIncludeCurrent,
  formatPriceRatio,
  rawPriceRatioFromUiPrice,
  tickToPriceRatio,
  uiPriceFromRawPriceRatio,
} from '@/lib/whirlpoolTicks'

describe('whirlpoolTicks', () => {
  it('tick 0 is a 1:1 raw price ratio', () => {
    expect(tickToPriceRatio(0)).toBe(1)
  })

  it('converts raw Orca price to UI ratio with SPL decimals (SOL 9 / USDC 6)', () => {
    expect(uiPriceFromRawPriceRatio(1, 9, 6)).toBe(1000)
    expect(rawPriceRatioFromUiPrice(1000, 9, 6)).toBe(1)
    expect(uiPriceFromRawPriceRatio(0, 9, 6)).toBeNull()
    expect(uiPriceFromRawPriceRatio(1, 9, 9 + 80)).toBeNull()
  })

  it('aligns a price band to spacing and covers both edges', () => {
    const aligned = alignPriceRatioToTicks(0.99, 1.01, 64)
    expect(aligned).not.toBeNull()
    expect(aligned!.tickLower % 64).toBeCloseTo(0)
    expect(aligned!.tickUpper % 64).toBeCloseTo(0)
    expect(aligned!.tickLower).toBeLessThan(aligned!.tickUpper)
    expect(tickToPriceRatio(aligned!.tickLower)).toBeLessThanOrEqual(0.99 + 1e-8)
    expect(tickToPriceRatio(aligned!.tickUpper)).toBeGreaterThanOrEqual(1.01 - 1e-8)
  })

  it('swaps inverted price edges before aligning', () => {
    expect(alignPriceRatioToTicks(1.01, 0.99, 64)).toEqual(alignPriceRatioToTicks(0.99, 1.01, 64))
  })

  it('width-pct range is spacing-aligned and contains a positive width', () => {
    const { tickLower, tickUpper } = calculateTickRangeFromWidthPct(0, 1, 64)
    expect(tickLower % 64).toBeCloseTo(0)
    expect(tickUpper % 64).toBeCloseTo(0)
    expect(tickUpper).toBeGreaterThan(tickLower)
  })

  it('expands an aligned range until current tick is in [lower, upper)', () => {
    const expanded = expandAlignedTickRangeToIncludeCurrent(0, 64, 200, 64)
    expect(expanded.tickLower).toBeLessThanOrEqual(200)
    expect(expanded.tickUpper).toBeGreaterThan(200)
    expect(expanded.tickLower % 64).toBeCloseTo(0)
    expect(expanded.tickUpper % 64).toBeCloseTo(0)
  })

  it('formats invalid ratios as an em dash', () => {
    expect(formatPriceRatio(0)).toBe('—')
    expect(formatPriceRatio(Number.NaN)).toBe('—')
  })
})
