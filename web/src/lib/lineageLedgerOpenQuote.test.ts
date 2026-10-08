import { describe, expect, it } from 'vitest'
import { extractLifecycleOpenQuoteUsdByPosition } from '@/lib/lineageLedgerOpenQuote'

describe('lineageLedgerOpenQuote', () => {
  it('takes the latest open-quote USD per PDA (key order, JSON details)', () => {
    const rows = [
      {
        position_pubkey: 'pda-a',
        event: 'bot_open_position',
        ts_utc: '2026-04-01T00:00:00Z',
        details: { open_quote_estimated_value_usd: '9.5' },
      },
      {
        position_pubkey: 'pda-a',
        event: 'bot_open_position',
        ts_utc: '2026-04-02T00:00:00Z',
        details: JSON.stringify({ open_target_usd: '11' }),
      },
      {
        position_pubkey: 'pda-a',
        event: 'bot_close_position',
        ts_utc: '2026-04-03T00:00:00Z',
        details: { open_quote_estimated_value_usd: '99' },
      },
      {
        position: 'pda-b',
        event: 'position_open',
        details: { open_quote_value_usd: 4.25 },
      },
    ]
    const got = extractLifecycleOpenQuoteUsdByPosition(rows)
    expect(got.get('pda-a')).toBe(11)
    expect(got.get('pda-b')).toBe(4.25)
  })

  it('prefers the first positive key in the Rust open-quote order', () => {
    const got = extractLifecycleOpenQuoteUsdByPosition([
      {
        position_pubkey: 'pda',
        event: 'bot_open_position_full_range',
        details: {
          open_quote_estimated_value_usd: '0',
          open_target_usd: '7.1',
          open_prev_end_value_usd: '3',
        },
      },
    ])
    expect(got.get('pda')).toBe(7.1)
  })
})
