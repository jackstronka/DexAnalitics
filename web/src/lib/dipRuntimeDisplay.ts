/** Pure helpers for Dip runtime / Settings (U2). */

export type DipSecretPresence = 'set' | 'missing'

export function secretPresence(set: boolean): DipSecretPresence {
  return set ? 'set' : 'missing'
}

/** Propose/Create live controls require mode=live AND JWT. */
export function dipLiveArmed(mode: string, triggerJwtSet: boolean): boolean {
  return mode.trim().toLowerCase() === 'live' && triggerJwtSet
}

export function formatLadderDefaultsLine(d: {
  base_order_usd: number
  safety_order_usd: number
  deviation_pct: number
  deviation_step_scale: number
  volume_scale: number
  max_safety_orders: number
  take_profit_pct: number
  min_order_usd: number
}): string {
  return [
    `BO $${d.base_order_usd}`,
    `SO $${d.safety_order_usd}`,
    `dev ${d.deviation_pct}%`,
    `step×${d.deviation_step_scale}`,
    `vol×${d.volume_scale}`,
    `maxSO ${d.max_safety_orders}`,
    `TP ${d.take_profit_pct}%`,
    `min $${d.min_order_usd}`,
  ].join(' · ')
}
