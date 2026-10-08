import { describe, expect, it } from 'vitest'
import type { WalletChainPortfolioResponse } from '@/lib/api'
import {
  chainInventoryUi,
  chainPortfolioNotionalUsd,
  quoteOpenBudgetBody,
  resolveChainSessionIdForQuote,
} from '@/lib/chainCapital'
import { USDC_MINT, WSOL_MINT } from '@/lib/sessionCapital'

function portfolio(
  overrides: Partial<WalletChainPortfolioResponse> = {},
): WalletChainPortfolioResponse {
  return {
    chain_session_id: 'chain-1',
    source: 'test',
    quality: 'exact',
    gl_matches_pslr: true,
    needs_reconcile: false,
    balances: [],
    ...overrides,
  }
}

describe('chainCapital', () => {
  it('prefers chain_session_id over cost_session_id', () => {
    expect(
      resolveChainSessionIdForQuote({
        chainSessionId: '  chain-a  ',
        costSessionId: 'cost-b',
      }),
    ).toBe('chain-a')
    expect(resolveChainSessionIdForQuote({ costSessionId: 'cost-b' })).toBe('cost-b')
    expect(resolveChainSessionIdForQuote({})).toBeUndefined()
  })

  it('omits empty chain_session_id from the quote body', () => {
    expect(quoteOpenBudgetBody(-64, 64, 10)).toEqual({
      tick_lower: -64,
      tick_upper: 64,
      target_usd: 10,
    })
    expect(quoteOpenBudgetBody(-64, 64, 10, ' sid ')).toEqual({
      tick_lower: -64,
      tick_upper: 64,
      target_usd: 10,
      chain_session_id: 'sid',
    })
  })

  it('converts CHAIN inventory from raw balances, then USD legs', () => {
    const fromBal = portfolio({
      balances: [{ mint: USDC_MINT, amount_raw: '2500000', decimals: 6 }],
    })
    expect(chainInventoryUi(fromBal, USDC_MINT, 6)).toBe(2.5)

    const fromLeg = portfolio({
      balances: [],
      chain_balance_usd_legs: [{ mint: WSOL_MINT, amount_raw: '1500000000' }],
    })
    expect(chainInventoryUi(fromLeg, WSOL_MINT, 9)).toBe(1.5)
    expect(chainInventoryUi(undefined, USDC_MINT, 6)).toBe(0)
  })

  it('parses portfolio notional USD strings', () => {
    expect(chainPortfolioNotionalUsd(portfolio({ portfolio_balance_usd: '12.5' }))).toBe(12.5)
    expect(
      chainPortfolioNotionalUsd(
        portfolio({
          portfolio_balance_usd: undefined,
          metrics: {
            open_start: {
              signature: 'sig',
              event: 'open',
              deployed_balances: [],
              value_usd_source: 'test',
              pre_open_balances: [],
            },
            current_value_usd: '8.25',
          },
        }),
      ),
    ).toBe(8.25)
    expect(chainPortfolioNotionalUsd(portfolio({ portfolio_balance_usd: 'nope' }))).toBeNull()
  })
})
