import type { PositionStreamLineageNode, PositionStreamLineageResponse, PositionStreamPnLResponse } from '@/lib/api'

export type ChainEconomicQuality = 'exact' | 'mixed' | 'estimated' | 'degraded'

function positiveUsdString(raw: string | null | undefined): string | null {
  if (raw == null || raw.trim() === '') return null
  const n = parseFloat(raw)
  if (!Number.isFinite(n) || n <= 0) return null
  return raw.trim()
}

/** Match Rust `lineage_node_end_nav_usd` (live → materialized → lifecycle close → estimate). */
export function lineageNodeEndNavUsd(node: PositionStreamLineageNode): string | null {
  const live = positiveUsdString(node.current_value_usd)
  if (live) return live
  const matEnd = positiveUsdString(node.chain_history_end_value_usd)
  if (matEnd) return matEnd
  const matCurrent = positiveUsdString(node.chain_history_current_value_usd)
  if (matCurrent) return matCurrent
  const lifeClose = positiveUsdString(node.lifecycle_close_nav_usd)
  if (lifeClose) return lifeClose
  if (node.closed_ts_utc) {
    const baseline = parseFloat(node.baseline_value_usd)
    const fees = parseFloat(node.fees_collected_usd)
    const cash = parseFloat(node.realized_cashflow_usd)
    const tx = parseFloat(node.tx_fees_usd)
    if ([baseline, fees, cash, tx].every(Number.isFinite) && baseline > 0) {
      const est = baseline + fees + cash - tx
      if (est > 0) return est.toFixed(8)
    }
  }
  return null
}

/** Head LP NAV for chain portfolio footer — not stream PnL headline net. */
export function chainHeadlineEndNavUsd(
  lineage: PositionStreamLineageResponse | null | undefined,
  options?: {
    /** Live RPC mark when UI is on the chain head PDA (`position.value_usd`). */
    liveHeadValueUsd?: string | null
    /** Current page PDA; live mark used only when it equals chain head. */
    pagePositionAddress?: string | null
  },
): string | null {
  const nodes = lineage?.nodes ?? []
  const chain = lineage?.chain ?? []
  const headAddr =
    chain.length > 0
      ? chain[chain.length - 1]?.trim()
      : nodes.length > 0
        ? nodes[nodes.length - 1]?.position_address?.trim()
        : null

  const page = options?.pagePositionAddress?.trim()
  const live = positiveUsdString(options?.liveHeadValueUsd)
  if (live && headAddr && page && page === headAddr) {
    return live
  }

  if (headAddr) {
    const headNode = nodes.find((n) => n.position_address?.trim() === headAddr)
    if (headNode) {
      const nav = lineageNodeEndNavUsd(headNode)
      if (nav) return nav
    }
  }

  for (let i = nodes.length - 1; i >= 0; i--) {
    const nav = lineageNodeEndNavUsd(nodes[i])
    if (nav) return nav
  }

  return positiveUsdString(lineage?.totals?.current_value_usd)
}

export function parseChainEconomicQuality(
  raw: string | null | undefined,
): ChainEconomicQuality | null {
  if (!raw) return null
  const t = raw.trim().toLowerCase()
  if (t === 'exact' || t === 'mixed' || t === 'estimated' || t === 'degraded') {
    return t
  }
  return null
}

/** Show amber banner when net PnL may mislead (estimate or degraded). */
export function needsChainEconomicQualityBanner(
  totals: Pick<PositionStreamPnLResponse, 'economic_quality'> | null | undefined,
): boolean {
  const q = parseChainEconomicQuality(totals?.economic_quality)
  return q === 'estimated' || q === 'degraded'
}

export function chainEconomicQualityLabel(
  quality: ChainEconomicQuality,
  locale: 'pl' | 'en',
): string {
  if (locale === 'pl') {
    switch (quality) {
      case 'exact':
        return 'exact'
      case 'mixed':
        return 'mieszana'
      case 'estimated':
        return 'szacunek'
      case 'degraded':
        return 'zdegradowana'
    }
  }
  return quality
}
