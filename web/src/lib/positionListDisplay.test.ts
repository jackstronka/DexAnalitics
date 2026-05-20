import { describe, expect, it } from 'vitest'
import type { Position } from '@/lib/api'
import {
  computeUncollectedFeesUsd,
  feeSourceLabel,
  formatUncollectedFeesCell,
  legUsdPrice,
  positionValueDisplayReady,
  uncollectedFeesUsdDisplayReady,
} from '@/lib/positionListDisplay'

function samplePosition(overrides: Partial<Position> = {}): Position {
  return {
    address: 'HTtpWVsnoctjiZqrYjkhan2RcYEnpxW3ueqns3PFJJQK',
    pool_address: 'Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE',
    owner: '11111111111111111111111111111111',
    tick_lower: -24544,
    tick_upper: -24340,
    liquidity: '1',
    in_range: true,
    value_usd: '0',
    valuation_source: 'list_light',
    pnl: {
      unrealized_pnl_usd: '0',
      unrealized_pnl_pct: '0',
      fees_earned_a: 0,
      fees_earned_b: 0,
      fees_earned_usd: '0',
      il_pct: '0',
      net_pnl_usd: '0',
      net_pnl_pct: '0',
    },
    status: 'active',
    created_at: null,
    ...overrides,
  }
}

describe('positionListDisplay', () => {
  it('value column ready when list_light returns non-zero value_usd', () => {
    const p = samplePosition({ value_usd: '7.7279819220057', valuation_source: 'list_light' })
    expect(positionValueDisplayReady(p)).toBe(true)
  })

  it('value column not ready for monitor-zero placeholder', () => {
    expect(positionValueDisplayReady(samplePosition({ value_usd: '0' }))).toBe(false)
    expect(positionValueDisplayReady(samplePosition({ value_usd: '0.00' }))).toBe(false)
  })

  it('fee USD uses both legs when mint prices present (list_light API shape)', () => {
    const p = samplePosition({
      token_a_label: 'SOL',
      token_b_label: 'USDC',
      token_price_a_usd: 86.37,
      token_price_b_usd: 1,
      uncollected_fees: {
        token_a_label: 'SOL',
        token_b_label: 'USDC',
        amount_a: '0.000414388',
        amount_b: '0.035265',
      },
    })
    const usd = computeUncollectedFeesUsd(p)
    expect(usd).not.toBeNull()
    expect(usd!).toBeGreaterThan(0.03)
    expect(uncollectedFeesUsdDisplayReady(p)).toBe(true)
    expect(formatUncollectedFeesCell(p)).not.toBe('—')
  })

  it('fee USD works with only USDC price when SOL mint price omitted (regression)', () => {
    const p = samplePosition({
      token_a_label: 'SOL',
      token_b_label: 'USDC',
      token_price_b_usd: 1,
      range_lower_usdc: '86.5',
      uncollected_fees: {
        token_a_label: 'SOL',
        token_b_label: 'USDC',
        amount_a: '0.000414388',
        amount_b: '0.035265',
      },
    })
    expect(legUsdPrice(p, 'a')).toBeCloseTo(86.5, 2)
    expect(computeUncollectedFeesUsd(p)).not.toBeNull()
    expect(formatUncollectedFeesCell(p)).not.toBe('—')
  })

  it('fee column stays dash when uncollected_fees missing', () => {
    const p = samplePosition({ uncollected_fees: undefined })
    expect(computeUncollectedFeesUsd(p)).toBeNull()
    expect(formatUncollectedFeesCell(p)).toBe('—')
  })

  it('fee source label for list_light is not unknown', () => {
    expect(feeSourceLabel('list_light', 'pl')).toBe('lista API (szybka)')
    expect(feeSourceLabel('list_light', 'en')).toBe('API list (fast)')
  })
})
