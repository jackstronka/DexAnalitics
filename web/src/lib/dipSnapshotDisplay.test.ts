import { describe, expect, it } from 'vitest'
import {
  dipOrdersEmptyKind,
  formatDipUsd,
  formatDipZoneBand,
  normalizeDipZone,
} from '@/lib/dipSnapshotDisplay'

describe('dipSnapshotDisplay', () => {
  it('empty kind is null when orders exist', () => {
    expect(dipOrdersEmptyKind('no_jwt', 1)).toBeNull()
    expect(dipOrdersEmptyKind('trigger_v2_orders_history', 2)).toBeNull()
  })

  it('maps orders_source to empty CTA kinds without JWT / API key', () => {
    expect(dipOrdersEmptyKind('no_jwt', 0)).toBe('no_jwt')
    expect(dipOrdersEmptyKind('no_api_key', 0)).toBe('no_api_key')
    expect(dipOrdersEmptyKind('error', 0)).toBe('error')
    expect(dipOrdersEmptyKind('skipped', 0)).toBe('skipped')
    expect(dipOrdersEmptyKind('trigger_v2_orders_history', 0)).toBe('empty')
  })

  it('normalizes zone labels', () => {
    expect(normalizeDipZone('a')).toBe('A')
    expect(normalizeDipZone('B')).toBe('B')
    expect(normalizeDipZone('none')).toBe('none')
    expect(normalizeDipZone('')).toBe('none')
  })

  it('formats USD and zone bands', () => {
    expect(formatDipUsd(123.456, 2)).toBe('$123.46')
    expect(formatDipUsd(null)).toBe('—')
    expect(formatDipZoneBand(80, 100)).toBe('$80.00 – $100.00')
  })
})
