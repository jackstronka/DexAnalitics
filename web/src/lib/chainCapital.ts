import type { QuoteOpenBudgetRequest, WalletChainPortfolioResponse } from '@/lib/api'

export function resolveChainSessionIdForQuote(opts: {
  chainSessionId?: string | null
  costSessionId?: string | null
}): string | undefined {
  const chain = opts.chainSessionId?.trim()
  if (chain) return chain
  const cost = opts.costSessionId?.trim()
  return cost || undefined
}

export function quoteOpenBudgetBody(
  tick_lower: number,
  tick_upper: number,
  target_usd: number,
  chainSessionId?: string | null,
): QuoteOpenBudgetRequest {
  const cid = chainSessionId?.trim()
  return {
    tick_lower,
    tick_upper,
    target_usd,
    ...(cid ? { chain_session_id: cid } : {}),
  }
}

/** CHAIN inventory (UI units) for one mint from chain-portfolio response. */
export function chainInventoryUi(
  portfolio: WalletChainPortfolioResponse | undefined,
  mint: string,
  defaultDecimals: number,
): number {
  if (!portfolio) return 0
  const row = portfolio.balances?.find((b) => b.mint === mint)
  if (row) {
    const dec = row.decimals ?? defaultDecimals
    const raw = BigInt(row.amount_raw.trim() || '0')
    return Number(raw) / 10 ** dec
  }
  const leg = portfolio.chain_balance_usd_legs?.find((l) => l.mint === mint)
  if (leg) {
    const raw = BigInt(leg.amount_raw.trim() || '0')
    return Number(raw) / 10 ** defaultDecimals
  }
  return 0
}

export function chainPortfolioNotionalUsd(portfolio: WalletChainPortfolioResponse | undefined): number | null {
  const raw =
    portfolio?.portfolio_balance_usd?.trim() ||
    portfolio?.metrics?.current_value_usd?.trim() ||
    undefined
  if (!raw) return null
  const n = parseFloat(raw)
  return Number.isFinite(n) ? n : null
}
