export type MetricsMode = 'live' | 'settlement_v1'

const SETTINGS_KEY = 'clmm-settings'

function readSettingsObject(): Record<string, unknown> {
  if (typeof window === 'undefined') return {}
  try {
    const raw = window.localStorage.getItem(SETTINGS_KEY)
    if (!raw) return {}
    const parsed = JSON.parse(raw)
    if (!parsed || typeof parsed !== 'object') return {}
    return parsed as Record<string, unknown>
  } catch {
    return {}
  }
}

export function getMetricsMode(): MetricsMode {
  const settings = readSettingsObject()
  return settings.pnl_mode === 'settlement_v1' ? 'settlement_v1' : 'live'
}

export function saveMetricsMode(mode: MetricsMode): void {
  if (typeof window === 'undefined') return
  const settings = readSettingsObject()
  settings.pnl_mode = mode
  window.localStorage.setItem(SETTINGS_KEY, JSON.stringify(settings))
}

