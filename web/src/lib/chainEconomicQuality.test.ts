import { describe, expect, it } from 'vitest'
import type { PositionStreamLineageNode, PositionStreamLineageResponse } from '@/lib/api'
import {
  chainEconomicQualityLabel,
  chainHeadlineEndNavUsd,
  lineageNodeEndNavUsd,
  needsChainEconomicQualityBanner,
  parseChainEconomicQuality,
} from '@/lib/chainEconomicQuality'

function node(overrides: Partial<PositionStreamLineageNode> = {}): PositionStreamLineageNode {
  return {
    position_address: 'head',
    baseline_value_usd: '10',
    current_value_usd: '0',
    tx_fee_lamports: 0,
    tx_fees_usd: '0.01',
    fees_collected_usd: '0.5',
    collect_events: 0,
    realized_cashflow_usd: '0.2',
    net_pnl_usd: '0',
    net_pnl_pct: '0',
    ...overrides,
  }
}

function lineage(
  overrides: Partial<PositionStreamLineageResponse> = {},
): PositionStreamLineageResponse {
  return {
    position_address: 'head',
    chain: ['head'],
    nodes: [node()],
    ...overrides,
  }
}

describe('chainEconomicQuality', () => {
  it('resolves node end NAV live → materialized → lifecycle close → close estimate', () => {
    expect(lineageNodeEndNavUsd(node({ current_value_usd: '4.75' }))).toBe('4.75')
    expect(
      lineageNodeEndNavUsd(
        node({ current_value_usd: '0', chain_history_end_value_usd: '3.2' }),
      ),
    ).toBe('3.2')
    expect(
      lineageNodeEndNavUsd(
        node({
          current_value_usd: '0',
          lifecycle_close_nav_usd: '2.1',
        }),
      ),
    ).toBe('2.1')
    expect(
      lineageNodeEndNavUsd(
        node({
          current_value_usd: '0',
          closed_ts_utc: '2026-04-01T00:00:00Z',
          baseline_value_usd: '10',
          fees_collected_usd: '0.5',
          realized_cashflow_usd: '0.2',
          tx_fees_usd: '0.01',
        }),
      ),
    ).toBe((10 + 0.5 + 0.2 - 0.01).toFixed(8))
  })

  it('uses live page mark only when the page PDA is the chain head', () => {
    const tree = lineage({
      chain: ['old', 'head'],
      nodes: [node({ position_address: 'old', current_value_usd: '1' }), node({ current_value_usd: '2' })],
    })
    expect(
      chainHeadlineEndNavUsd(tree, {
        liveHeadValueUsd: '9.99',
        pagePositionAddress: 'head',
      }),
    ).toBe('9.99')
    expect(
      chainHeadlineEndNavUsd(tree, {
        liveHeadValueUsd: '9.99',
        pagePositionAddress: 'old',
      }),
    ).toBe('2')
  })

  it('parses quality and shows the banner only for estimated/degraded', () => {
    expect(parseChainEconomicQuality(' Exact ')).toBe('exact')
    expect(parseChainEconomicQuality('nope')).toBeNull()
    expect(needsChainEconomicQualityBanner({ economic_quality: 'estimated' })).toBe(true)
    expect(needsChainEconomicQualityBanner({ economic_quality: 'degraded' })).toBe(true)
    expect(needsChainEconomicQualityBanner({ economic_quality: 'exact' })).toBe(false)
    expect(chainEconomicQualityLabel('degraded', 'pl')).toBe('zdegradowana')
    expect(chainEconomicQualityLabel('degraded', 'en')).toBe('degraded')
  })
})
