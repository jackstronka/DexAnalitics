import { describe, expect, it } from 'vitest'
import { computeSolFirstFundingBalances, WSOL_MINT } from '@/lib/solFirstFunding'
import type { WalletEffectiveBalancesResponse } from '@/lib/api'

const USDC = 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v'

function mockBalances(overrides: Partial<WalletEffectiveBalancesResponse> = {}): WalletEffectiveBalancesResponse {
  return {
    owner: 'owner',
    sol: '0',
    tokens: [],
    is_stale: false,
    ...overrides,
  }
}

describe('solFirstFunding', () => {
  it('uses native SOL for WSOL leg when SPL WSOL is zero', () => {
    const balances = mockBalances({
      sol: '1.5',
      tokens: [{ mint: WSOL_MINT, ui_amount: '0', amount_raw: '0', decimals: 9 }],
    })
    const r = computeSolFirstFundingBalances({
      balances,
      tokenAMint: WSOL_MINT,
      tokenBMint: USDC,
      minOpenLamports: 50_000_000,
    })
    expect(r.haveA).toBe(0)
    expect(r.walletDisplayA).toBe(1.5)
    expect(r.effectiveHaveA).toBeGreaterThan(0)
    expect(r.effectiveHaveA).toBeLessThan(1.5)
  })

  it('uses SPL balance for non-WSOL leg', () => {
    const balances = mockBalances({
      sol: '0.1',
      tokens: [{ mint: USDC, ui_amount: '25.5', amount_raw: '25500000', decimals: 6 }],
    })
    const r = computeSolFirstFundingBalances({
      balances,
      tokenAMint: WSOL_MINT,
      tokenBMint: USDC,
    })
    expect(r.effectiveHaveB).toBe(25.5)
  })
})
