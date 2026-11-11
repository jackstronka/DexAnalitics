import { describe, expect, it } from 'vitest'
import {
  botDryRunMismatch,
  strategyExecBadge,
  summarizeStrategyFleet,
} from './botControlDisplay'

describe('botControlDisplay', () => {
  it('detects dry-run mismatches', () => {
    expect(botDryRunMismatch(true, true)).toBe('none')
    expect(botDryRunMismatch(false, true)).toBe('local_live_api_dry')
    expect(botDryRunMismatch(true, false)).toBe('local_dry_api_live')
    expect(botDryRunMismatch(false, null)).toBe('none')
  })

  it('flags live-armed strategies', () => {
    expect(strategyExecBadge({ running: true, dry_run: false, auto_execute: true }).liveArmed).toBe(
      true,
    )
    expect(strategyExecBadge({ running: true, dry_run: true, auto_execute: true }).liveArmed).toBe(
      false,
    )
  })

  it('summarizes fleet', () => {
    const s = summarizeStrategyFleet([
      { running: true, dry_run: true, auto_execute: true },
      { running: true, dry_run: false, auto_execute: true },
      { running: false, dry_run: false, auto_execute: true },
    ])
    expect(s.total).toBe(3)
    expect(s.running).toBe(2)
    expect(s.liveArmedRunning).toBe(1)
  })
})
