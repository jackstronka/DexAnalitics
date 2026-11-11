import { describe, expect, it } from 'vitest'
import { buildDipLadderPreview } from './dipLadder'
import {
  DIP_PROPOSE_LADDER_KIND,
  buildLadderBuyDipPropose,
  buildLadderProposedOrders,
  buildQueueResolvePropose,
  isLadderBuyDipPropose,
} from './dipProposePayload'

describe('dipProposePayload', () => {
  const entry = 100
  const preview = buildDipLadderPreview(entry, {
    baseOrderUsd: 50,
    safetyOrderUsd: 50,
    deviationPct: 1,
    deviationStepScale: 1,
    volumeScale: 1,
    maxSafetyOrders: 2,
    takeProfitPct: 1.5,
    availableUsd: 500,
  })

  it('builds ladder proposed_orders with buys + TP', () => {
    const po = buildLadderProposedOrders(entry, preview, 1.5)
    expect(po.kind).toBe(DIP_PROPOSE_LADDER_KIND)
    expect(po.levels.some((l) => l.role === 'buy_dip')).toBe(true)
    expect(po.levels.some((l) => l.role === 'take_profit' && l.level === 'TP1')).toBe(true)
    expect(po.required_usd).toBeGreaterThan(0)
  })

  it('returns null buy_dip body when propose disabled', () => {
    const blocked = buildDipLadderPreview(entry, {
      baseOrderUsd: 50,
      safetyOrderUsd: 50,
      deviationPct: 1,
      deviationStepScale: 1,
      volumeScale: 1,
      maxSafetyOrders: 2,
      takeProfitPct: 1.5,
      availableUsd: 10,
    })
    expect(buildLadderBuyDipPropose({ entryPriceUsd: entry, preview: blocked, takeProfitPct: 1.5 })).toBeNull()
  })

  it('builds propose-only buy_dip body when allowed', () => {
    const body = buildLadderBuyDipPropose({
      entryPriceUsd: entry,
      preview,
      takeProfitPct: 1.5,
      zone: 'B',
      solUsd: entry,
    })
    expect(body).not.toBeNull()
    expect(body!.action).toBe('buy_dip')
    expect(body!.mode).toBe('propose-only')
    expect(body!.zone).toBe('B')
    expect((body!.proposed_orders as { kind: string }).kind).toBe(DIP_PROPOSE_LADDER_KIND)
  })

  it('builds confirm/reject resolve bodies', () => {
    const c = buildQueueResolvePropose({ resolve: 'confirm', refTsUtc: '2026-01-01T00:00:00Z' })
    expect(c.action).toBe('hold')
    const r = buildQueueResolvePropose({ resolve: 'reject', refTsUtc: '2026-01-01T00:00:00Z' })
    expect(r.action).toBe('skip')
  })

  it('detects ladder buy_dip propose for queue actions', () => {
    expect(
      isLadderBuyDipPropose({
        kind: 'dip_propose',
        action: 'buy_dip',
        proposed_orders: { kind: DIP_PROPOSE_LADDER_KIND },
      }),
    ).toBe(true)
    expect(isLadderBuyDipPropose({ kind: 'dip_propose', action: 'hold' })).toBe(false)
  })
})
