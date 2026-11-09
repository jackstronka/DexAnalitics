import { describe, expect, it } from 'vitest'
import {
  dipLiveArmed,
  formatLadderDefaultsLine,
  secretPresence,
} from '@/lib/dipRuntimeDisplay'

describe('dipRuntimeDisplay', () => {
  it('maps secret presence', () => {
    expect(secretPresence(true)).toBe('set')
    expect(secretPresence(false)).toBe('missing')
  })

  it('live armed only when mode=live and JWT set', () => {
    expect(dipLiveArmed('live', true)).toBe(true)
    expect(dipLiveArmed('live', false)).toBe(false)
    expect(dipLiveArmed('propose-only', true)).toBe(false)
  })

  it('formats ladder defaults summary line', () => {
    const line = formatLadderDefaultsLine({
      base_order_usd: 50,
      safety_order_usd: 50,
      deviation_pct: 1,
      deviation_step_scale: 1,
      volume_scale: 1.5,
      max_safety_orders: 5,
      take_profit_pct: 1.5,
      min_order_usd: 10,
    })
    expect(line).toContain('BO $50')
    expect(line).toContain('vol×1.5')
    expect(line).toContain('min $10')
  })
})
