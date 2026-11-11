import { describe, expect, it } from 'vitest'
import {
  agentDecisionAction,
  agentDecisionKind,
  formatAgentDecisionPreview,
  matchesAgentDecisionKindFilter,
} from './agentDecisionsDisplay'

describe('agentDecisionsDisplay', () => {
  it('reads kind and action from dip_propose payload', () => {
    const d = { kind: 'dip_propose', action: 'place_buy_dip', reason: 'x' }
    expect(agentDecisionKind(d)).toBe('dip_propose')
    expect(agentDecisionAction(d)).toBe('place_buy_dip')
  })

  it('filters by kind', () => {
    const dip = { kind: 'dip_propose' }
    const other = { kind: 'rebalance' }
    expect(matchesAgentDecisionKindFilter(dip, 'all')).toBe(true)
    expect(matchesAgentDecisionKindFilter(dip, 'dip_propose')).toBe(true)
    expect(matchesAgentDecisionKindFilter(other, 'dip_propose')).toBe(false)
    expect(matchesAgentDecisionKindFilter(other, 'other')).toBe(true)
    expect(matchesAgentDecisionKindFilter(dip, 'other')).toBe(false)
    expect(matchesAgentDecisionKindFilter({}, 'other')).toBe(true)
  })

  it('truncates preview', () => {
    const long = { a: 'x'.repeat(200) }
    const p = formatAgentDecisionPreview(long, 40)
    expect(p.length).toBeLessThanOrEqual(41)
    expect(p.endsWith('…')).toBe(true)
  })
})
