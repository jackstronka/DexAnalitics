import { describe, expect, it } from 'vitest'
import { computeSolFirstFundingBalances, WSOL_MINT } from '@/lib/solFirstFunding'
import type { WalletEffectiveBalancesResponse } from '@/lib/api'

const USDC = 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v'

function mockBalances(overrides: Partial<WalletEffectiveBalancesResponse> = {}): WalletEffectiveBalancesResponse {
  return {
    owner: 'owner',
    rpc_url: 'http://localhost',
    lamports: 0,
    sol: '0',
    tokens: [],
    as_of_utc: '',
    is_stale: false,
    stale_age_ms: 0,
    confidence: 'verified',
    pending_ops_count: 0,
    native_onchain_lamports: 0,
    native_effective_lamports: 0,
    wsol_onchain_raw: 0,
    wsol_effective_raw: 0,
    ...overrides,
  }
}

describe('solFirstFunding', () => {
  it('uses native SOL for WSOL leg when SPL WSOL is zero', () => {
    const balances = mockBalances({
      sol: '1.5',
      tokens: [{ mint: WSOL_MINT, ui_amount: '0' }],
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
      tokens: [{ mint: USDC, ui_amount: '25.5' }],
    })
    const r = computeSolFirstFundingBalances({
      balances,
      tokenAMint: WSOL_MINT,
      tokenBMint: USDC,
    })
    expect(r.effectiveHaveB).toBe(25.5)
  })
})
