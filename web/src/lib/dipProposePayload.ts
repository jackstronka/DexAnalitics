/**
 * Build POST /dip/propose bodies from ladder preview (U4 / AH4).
 * Propose-only — never Trigger create.
 */

import type { DipLadderPreview, DipLadderRow } from './dipLadder'

export const DIP_PROPOSE_DEFAULT_REFS = [
  'config/dip/context/news-digest.md',
  'config/dip/context/assets/SOL/news.md',
] as const

export const DIP_PROPOSE_LADDER_KIND = 'ladder_v1'

export type DipLadderProposedLevel = {
  level: string
  role: 'buy_dip' | 'take_profit'
  trigger_price_usd: number
  size_usd: number
  size_base: number
  deviation_pct: number
}

export type DipLadderProposedOrders = {
  kind: typeof DIP_PROPOSE_LADDER_KIND
  entry_price_usd: number
  take_profit_pct: number
  required_usd: number
  levels: DipLadderProposedLevel[]
}

function rowRole(row: DipLadderRow): 'buy_dip' | 'take_profit' {
  return row.kind === 'tp' ? 'take_profit' : 'buy_dip'
}

/** Map ladder preview rows → free-form `proposed_orders` for audit JSONL. */
export function buildLadderProposedOrders(
  entryPriceUsd: number,
  preview: DipLadderPreview,
  takeProfitPct: number,
): DipLadderProposedOrders {
  const levels: DipLadderProposedLevel[] = preview.rows.map((row) => ({
    level: row.level,
    role: rowRole(row),
    trigger_price_usd: row.estPrice,
    size_usd: row.kind === 'tp' ? 0 : row.orderSizeQuote,
    size_base: row.orderSizeBase,
    deviation_pct: row.deviationPct,
  }))
  return {
    kind: DIP_PROPOSE_LADDER_KIND,
    entry_price_usd: entryPriceUsd,
    take_profit_pct: takeProfitPct,
    required_usd: preview.summary.requiredUsd,
    levels,
  }
}

export type DipProposeBody = {
  action: string
  mode: string
  rationale: string
  refs: string[]
  zone?: string
  sol_usd?: number
  proposed_orders?: unknown
  session_id?: string
}

/** Ladder → buy_dip propose (always propose-only from UI). */
export function buildLadderBuyDipPropose(args: {
  entryPriceUsd: number
  preview: DipLadderPreview
  takeProfitPct: number
  zone?: string
  solUsd?: number
  rationale?: string
}): DipProposeBody | null {
  if (args.preview.summary.proposeDisabled || args.preview.rows.length === 0) return null
  const proposed_orders = buildLadderProposedOrders(
    args.entryPriceUsd,
    args.preview,
    args.takeProfitPct,
  )
  const buyLevels = proposed_orders.levels.filter((l) => l.role === 'buy_dip').length
  const rationale =
    args.rationale?.trim() ||
    `Ladder propose: ${buyLevels} buy level(s) + TP @ ${args.takeProfitPct}%; required ~${proposed_orders.required_usd.toFixed(2)} USD (propose-only, no Trigger create).`
  return {
    action: 'buy_dip',
    mode: 'propose-only',
    rationale,
    refs: [...DIP_PROPOSE_DEFAULT_REFS],
    zone: args.zone,
    sol_usd: args.solUsd,
    proposed_orders,
  }
}

/** Operator confirm/reject of a queued propose (audit-only). */
export function buildQueueResolvePropose(args: {
  resolve: 'confirm' | 'reject'
  refTsUtc: string
  zone?: string
  solUsd?: number
}): DipProposeBody {
  const ref = args.refTsUtc.trim()
  if (args.resolve === 'confirm') {
    return {
      action: 'hold',
      mode: 'propose-only',
      rationale: `Operator confirmed dip propose ${ref} (audit only; no live Trigger).`,
      refs: [...DIP_PROPOSE_DEFAULT_REFS],
      zone: args.zone,
      sol_usd: args.solUsd,
      proposed_orders: { kind: 'queue_resolve_v1', resolve: 'confirm', ref_ts_utc: ref },
      session_id: `confirm:${ref}`,
    }
  }
  return {
    action: 'skip',
    mode: 'propose-only',
    rationale: `Operator rejected dip propose ${ref} (audit only).`,
    refs: [...DIP_PROPOSE_DEFAULT_REFS],
    zone: args.zone,
    sol_usd: args.solUsd,
    proposed_orders: { kind: 'queue_resolve_v1', resolve: 'reject', ref_ts_utc: ref },
    session_id: `reject:${ref}`,
  }
}

/** True when decision is a ladder buy_dip propose (eligible for confirm/reject). */
export function isLadderBuyDipPropose(decision: unknown): boolean {
  if (!decision || typeof decision !== 'object') return false
  const d = decision as { kind?: unknown; action?: unknown; proposed_orders?: unknown }
  if (d.kind !== 'dip_propose') return false
  if (d.action !== 'buy_dip') return false
  const po = d.proposed_orders
  if (!po || typeof po !== 'object') return false
  return (po as { kind?: unknown }).kind === DIP_PROPOSE_LADDER_KIND
}
