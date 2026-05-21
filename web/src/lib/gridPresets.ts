export type GridPresetName = 'Ultra-safe' | 'Conservative' | 'Balanced' | 'Aggressive' | 'Scalper'

export type GridPreset = {
  name: GridPresetName
  thresholdGridPct: string
  periodicGridHours: string
  bollingerWindowGrid: string
  bollingerKGrid: string
  bollingerRebalanceHoursGrid: string
  /** UI values in minutes; API expects seconds for backtests. */
  lastCandleSecondsGrid: string
  lastCandleRebalanceSecondsGrid: string
}

export const GRID_PRESETS: GridPreset[] = [
  {
    name: 'Ultra-safe',
    thresholdGridPct: '10,15,20',
    periodicGridHours: '72,96,144',
    bollingerWindowGrid: '30,40',
    bollingerKGrid: '2.5,3.0',
    bollingerRebalanceHoursGrid: '8,12',
    lastCandleSecondsGrid: '30,60,120',
    lastCandleRebalanceSecondsGrid: '240,720,1440',
  },
  {
    name: 'Conservative',
    thresholdGridPct: '7,10,15',
    periodicGridHours: '48,72',
    bollingerWindowGrid: '20,30',
    bollingerKGrid: '2.0,2.5',
    bollingerRebalanceHoursGrid: '8',
    lastCandleSecondsGrid: '30,60',
    lastCandleRebalanceSecondsGrid: '60,240,720',
  },
  {
    name: 'Balanced',
    thresholdGridPct: '3,5,7,10',
    periodicGridHours: '24,48,72',
    bollingerWindowGrid: '20',
    bollingerKGrid: '1.5,2.0,2.5',
    bollingerRebalanceHoursGrid: '4,8',
    lastCandleSecondsGrid: '15,30,45,60',
    lastCandleRebalanceSecondsGrid: '30,60,240,720',
  },
  {
    name: 'Aggressive',
    thresholdGridPct: '2,3,5,7',
    periodicGridHours: '12,24,48',
    bollingerWindowGrid: '10,20',
    bollingerKGrid: '1.0,1.5,2.0',
    bollingerRebalanceHoursGrid: '2,4',
    lastCandleSecondsGrid: '15,30,45',
    lastCandleRebalanceSecondsGrid: '15,30,45,60,240',
  },
  {
    name: 'Scalper',
    thresholdGridPct: '1,1.5,2,3',
    periodicGridHours: '6,12,24',
    bollingerWindowGrid: '8,10,14',
    bollingerKGrid: '0.8,1.0,1.5',
    bollingerRebalanceHoursGrid: '1,2,4',
    lastCandleSecondsGrid: '5,10,15',
    lastCandleRebalanceSecondsGrid: '5,10,15,30',
  },
]

export function parseFirstCsvNumber(raw: string): number | '' {
  const part = raw.split(',')[0]?.trim()
  if (!part) return ''
  const n = Number(part)
  return Number.isFinite(n) ? n : ''
}

/** Median of comma-separated numbers (used for balanced defaults). */
export function medianCsvNumber(raw: string): number | '' {
  const nums = raw
    .split(',')
    .map((s) => Number(s.trim()))
    .filter((n) => Number.isFinite(n))
  if (nums.length === 0) return ''
  const sorted = [...nums].sort((a, b) => a - b)
  const mid = Math.floor(sorted.length / 2)
  if (sorted.length % 2 === 0) {
    return (sorted[mid - 1]! + sorted[mid]!) / 2
  }
  return sorted[mid]!
}
