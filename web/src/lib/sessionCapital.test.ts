import { describe, expect, it } from 'vitest'
import {
  USDC_MINT,
  WSOL_MINT,
  defaultTokenDecimals,
  formatRawAmount,
  mintSymbol,
  rawToUiNumber,
  sessionSpendCapUi,
} from '@/lib/sessionCapital'

describe('sessionCapital', () => {
  it('maps well-known mints to decimals and symbols', () => {
    expect(defaultTokenDecimals(WSOL_MINT)).toBe(9)
    expect(defaultTokenDecimals(USDC_MINT)).toBe(6)
    expect(mintSymbol(WSOL_MINT)).toBe('SOL')
    expect(mintSymbol(USDC_MINT)).toBe('USDC')
  })

  it('formats raw amounts without float rounding (USDC 6 dp)', () => {
    expect(formatRawAmount('1500000', 6)).toBe('1.5')
    expect(formatRawAmount('1', 6)).toBe('0.000001')
    expect(formatRawAmount('-1500000', 6)).toBe('-1.5')
    expect(rawToUiNumber('1500000', 6)).toBe(1.5)
  })

  it('spend cap is non-negative inventory', () => {
    expect(sessionSpendCapUi('1500000', 6)).toBe(1.5)
    expect(sessionSpendCapUi('-1500000', 6)).toBe(0)
    expect(sessionSpendCapUi('not-a-number', 6)).toBe(0)
  })
})
