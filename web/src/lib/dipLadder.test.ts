import { describe, expect, it } from 'vitest'
import {
  buildDipLadderPreview,
  cumulativeSafetyDeviationPct,
  DIP_LADDER_MIN_ORDER_USD_DEFAULT,
} from '@/lib/dipLadder'

describe('cumulativeSafetyDeviationPct', () => {
  it('is linear when step scale = 1', () => {
    expect(cumulativeSafetyDeviationPct(1, 1, 1)).toBeCloseTo(1, 10)
    expect(cumulativeSafetyDeviationPct(3, 1, 1)).toBeCloseTo(3, 10)
  })

  it('uses geometric series when step scale ≠ 1', () => {
    // 1 * (1.5^2 - 1) / (1.5 - 1) = (2.25 - 1) / 0.5 = 2.5
    expect(cumulativeSafetyDeviationPct(2, 1, 1.5)).toBeCloseTo(2.5, 10)
    // 1 * (1.5^3 - 1) / 0.5 = (3.375 - 1) / 0.5 = 4.75
    expect(cumulativeSafetyDeviationPct(3, 1, 1.5)).toBeCloseTo(4.75, 10)
  })
})

describe('buildDipLadderPreview', () => {
  const entry = 100

  it('scale=1: deviations, required, and avg entry', () => {
    const { rows, summary } = buildDipLadderPreview(entry, {
      baseOrderUsd: 100,
      safetyOrderUsd: 50,
      deviationPct: 1,
      deviationStepScale: 1,
      volumeScale: 1,
      maxSafetyOrders: 2,
      takeProfitPct: 2,
    })
    const buys = rows.filter((r) => r.kind !== 'tp')
    expect(buys.map((r) => r.level)).toEqual(['BO', 'SO1', 'SO2'])
    expect(buys[0].estPrice).toBeCloseTo(100, 8)
    expect(buys[1].deviationPct).toBeCloseTo(1, 8)
    expect(buys[1].estPrice).toBeCloseTo(99, 8)
    expect(buys[2].deviationPct).toBeCloseTo(2, 8)
    expect(buys[2].estPrice).toBeCloseTo(98, 8)
    expect(summary.requiredUsd).toBeCloseTo(200, 8)
    expect(summary.maxPriceDropPct).toBeCloseTo(2, 8)
    // Quotes 100@100 + 50@99 + 50@98 → base 1 + 50/99 + 50/98
    const base = 1 + 50 / 99 + 50 / 98
    const avg = 200 / base
    expect(buys[2].avgEntryAfter).toBeCloseTo(avg, 8)
    expect(summary.maxAvgEntryDeviationPct).toBeCloseTo(((100 - avg) / 100) * 100, 8)
    const tp = rows.find((r) => r.kind === 'tp')
    expect(tp?.estPrice).toBeCloseTo(avg * 1.02, 8)
  })

  it('scale≠1: volume and deviation scales compound', () => {
    const { rows, summary } = buildDipLadderPreview(entry, {
      baseOrderUsd: 40,
      safetyOrderUsd: 10,
      deviationPct: 1,
      deviationStepScale: 1.5,
      volumeScale: 2,
      maxSafetyOrders: 2,
      takeProfitPct: 1,
    })
    const so1 = rows.find((r) => r.level === 'SO1')!
    const so2 = rows.find((r) => r.level === 'SO2')!
    expect(so1.orderSizeQuote).toBeCloseTo(10, 8)
    expect(so2.orderSizeQuote).toBeCloseTo(20, 8)
    expect(so1.deviationPct).toBeCloseTo(1, 8)
    expect(so2.deviationPct).toBeCloseTo(2.5, 8)
    expect(summary.requiredUsd).toBeCloseTo(70, 8)
  })

  it('disables propose when Required > Available', () => {
    const { summary } = buildDipLadderPreview(entry, {
      baseOrderUsd: 50,
      safetyOrderUsd: 50,
      deviationPct: 1,
      deviationStepScale: 1,
      volumeScale: 1,
      maxSafetyOrders: 1,
      takeProfitPct: 1,
      availableUsd: 80,
    })
    expect(summary.requiredUsd).toBeCloseTo(100, 8)
    expect(summary.proposeDisabled).toBe(true)
    expect(summary.proposeDisabledReason).toBe('required_gt_available')
  })

  it('disables propose when a level is below min Jupiter size', () => {
    const { summary } = buildDipLadderPreview(entry, {
      baseOrderUsd: 50,
      safetyOrderUsd: 5,
      deviationPct: 1,
      deviationStepScale: 1,
      volumeScale: 1,
      maxSafetyOrders: 1,
      takeProfitPct: 1,
      minOrderUsd: DIP_LADDER_MIN_ORDER_USD_DEFAULT,
      availableUsd: 1000,
    })
    expect(summary.proposeDisabled).toBe(true)
    expect(summary.proposeDisabledReason).toBe('below_min')
  })

  it('keeps propose enabled when Available unknown and sizes ok', () => {
    const { summary } = buildDipLadderPreview(entry, {
      baseOrderUsd: 50,
      safetyOrderUsd: 50,
      deviationPct: 1,
      deviationStepScale: 1,
      volumeScale: 1,
      maxSafetyOrders: 0,
      takeProfitPct: 1,
      availableUsd: null,
    })
    expect(summary.proposeDisabled).toBe(false)
    expect(summary.proposeDisabledReason).toBeNull()
  })
})
