import { useMemo } from 'react'
import { keepPreviousData, useQuery } from '@tanstack/react-query'
import { RefreshCw } from 'lucide-react'
import { Link } from 'react-router-dom'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { ErrorBanner } from '@/components/ui/error-banner'
import { formatRawAmount } from '@/components/SessionBalancesPanel'
import {
  getWalletChainPortfolio,
  type WalletChainCollectedFeesSummary,
  type WalletChainPortfolioLedgerEvent,
  type WalletSessionBalanceUsdLeg,
} from '@/lib/api'
import { useI18n } from '@/lib/i18n'
import { shortenAddress } from '@/lib/utils'

const WSOL = 'So11111111111111111111111111111111111111112'
const USDC = 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v'

function formatUsdMetric(v: string | null | undefined): string {
  if (v == null || v.trim() === '') return '—'
  const n = parseFloat(v)
  if (!Number.isFinite(n)) return '—'
  const abs = Math.abs(n)
  const maxFrac = abs > 0 && abs < 0.01 ? 6 : 2
  return `$${n.toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: maxFrac })}`
}

function defaultDecimals(mint: string): number {
  if (mint === WSOL) return 9
  if (mint === USDC) return 6
  return 9
}

function mintSymbol(mint: string): string {
  if (mint === WSOL) return 'SOL'
  if (mint === USDC) return 'USDC'
  return shortenAddress(mint, 4)
}

function shortMint(mint: string): string {
  const t = mint.trim()
  if (t.length <= 12) return t
  return `${t.slice(0, 4)}…${t.slice(-4)}`
}

function ledgerKindKey(kind: string): string {
  return `chainPortfolio.kind.${kind}`
}

function nonZeroChainLegs(
  legs: WalletSessionBalanceUsdLeg[] | undefined,
): WalletSessionBalanceUsdLeg[] {
  return (legs ?? []).filter((leg) => {
    try {
      return BigInt(leg.amount_raw.trim()) !== 0n
    } catch {
      return leg.amount_raw.trim() !== '0'
    }
  })
}

function LedgerEventRow({ evt }: { evt: WalletChainPortfolioLedgerEvent }) {
  const { t } = useI18n()
  const kindLabel = (() => {
    const key = ledgerKindKey(evt.kind)
    const translated = t(key)
    return translated === key ? evt.kind : translated
  })()
  return (
    <tr className="border-t border-border/40 align-top">
      <td className="p-2 whitespace-nowrap text-muted-foreground text-xs">
        {evt.ts_utc ? new Date(evt.ts_utc).toLocaleString() : '—'}
      </td>
      <td className="p-2 text-xs">
        <div className="font-medium">{kindLabel}</div>
        {evt.position_pubkey ? (
          <div className="font-mono text-[10px] text-muted-foreground truncate max-w-[8rem]" title={evt.position_pubkey}>
            {shortMint(evt.position_pubkey)}
          </div>
        ) : null}
      </td>
      <td className="p-2 text-xs space-y-0.5">
        {evt.legs.map((leg, i) => (
          <div key={`${leg.mint}-${leg.direction}-${i}`} className="font-mono tabular-nums">
            <span className="text-muted-foreground">
              {leg.direction === 'in' ? t('chainPortfolio.legIn') : t('chainPortfolio.legOut')}
            </span>{' '}
            <span title={leg.mint}>{shortMint(leg.mint)}</span>{' '}
            <span>{leg.amount_raw}</span>
            {leg.value_usd ? (
              <span className="text-muted-foreground"> ({formatUsdMetric(leg.value_usd)})</span>
            ) : null}
          </div>
        ))}
      </td>
      <td className="p-2 text-xs tabular-nums text-right whitespace-nowrap">
        {formatUsdMetric(evt.total_usd)}
      </td>
    </tr>
  )
}

function ChainWalletTokensBlock({
  legs,
  totalUsd,
  excludedMintCount,
  glMatchesPslr,
  quality,
}: {
  legs: WalletSessionBalanceUsdLeg[]
  totalUsd?: string
  excludedMintCount?: number
  glMatchesPslr?: boolean
  quality?: string
}) {
  const { t } = useI18n()
  const showQualityNote =
    (excludedMintCount ?? 0) > 0 ||
    glMatchesPslr === false ||
    quality === 'pslr_corrected' ||
    quality === 'pslr_fallback'
  return (
    <div className="space-y-2">
      <div>
        <p className="text-xs font-medium text-foreground">{t('chainPortfolio.chainTokensHeading')}</p>
        <p className="text-[11px] text-muted-foreground mt-0.5">{t('chainPortfolio.chainTokensHint')}</p>
        {showQualityNote ? (
          <p className="text-[11px] text-amber-800 dark:text-amber-200 mt-1">
            {(excludedMintCount ?? 0) > 0
              ? t('chainPortfolio.chainWalletFilteredNote').replace('{n}', String(excludedMintCount))
              : t('chainPortfolio.chainWalletGlMismatchNote')}
          </p>
        ) : null}
      </div>
      <TokenLegsTable
        legs={legs}
        totalUsd={totalUsd}
        emptyLabel={t('chainPortfolio.chainWalletEmpty')}
      />
    </div>
  )
}

function TokenLegsTable({
  legs,
  totalUsd,
  emptyLabel,
}: {
  legs: WalletSessionBalanceUsdLeg[]
  totalUsd?: string
  emptyLabel: string
}) {
  const { t } = useI18n()
  if (legs.length === 0) {
    return <p className="text-sm text-muted-foreground">{emptyLabel}</p>
  }
  return (
    <div className="overflow-x-auto rounded border border-border/50">
      <table className="w-full text-xs">
        <thead className="bg-muted/30">
          <tr>
            <th className="text-left p-2 font-medium">{t('chainPortfolio.chainWalletColToken')}</th>
            <th className="text-right p-2 font-medium">{t('chainPortfolio.chainWalletColAmount')}</th>
            <th className="text-right p-2 font-medium">{t('chainPortfolio.chainWalletColUsd')}</th>
          </tr>
        </thead>
        <tbody>
          {legs.map((leg) => (
            <tr key={leg.mint} className="border-t border-border/40">
              <td className="p-2 font-mono" title={leg.mint}>
                {mintSymbol(leg.mint)}
              </td>
              <td className="p-2 text-right tabular-nums font-mono">
                {formatRawAmount(leg.amount_raw, defaultDecimals(leg.mint))}
              </td>
              <td className="p-2 text-right tabular-nums">{formatUsdMetric(leg.value_usd)}</td>
            </tr>
          ))}
        </tbody>
        {totalUsd ? (
          <tfoot>
            <tr className="border-t border-border/60 bg-muted/20">
              <td colSpan={2} className="p-2 text-right text-muted-foreground">
                {t('chainPortfolio.chainWalletUsdSum')}
              </td>
              <td className="p-2 text-right tabular-nums font-semibold">{formatUsdMetric(totalUsd)}</td>
            </tr>
          </tfoot>
        ) : null}
      </table>
    </div>
  )
}

function ChainCollectedFeesBlock({ summary }: { summary: WalletChainCollectedFeesSummary | undefined }) {
  const { t } = useI18n()
  const legs = nonZeroChainLegs(summary?.legs)
  const collectEvents = summary?.collect_events ?? 0
  return (
    <div className="space-y-2 border-t border-border/50 pt-3">
      <div>
        <p className="text-xs font-medium text-foreground">{t('chainPortfolio.collectedFeesHeading')}</p>
        <p className="text-[11px] text-muted-foreground mt-0.5">{t('chainPortfolio.collectedFeesHint')}</p>
        {collectEvents > 0 ? (
          <p className="text-[11px] text-muted-foreground mt-0.5">
            {t('chainPortfolio.collectedFeesCollectCount').replace('{n}', String(collectEvents))}
          </p>
        ) : null}
      </div>
      <TokenLegsTable
        legs={legs}
        totalUsd={summary?.total_usd?.trim() || undefined}
        emptyLabel={t('chainPortfolio.collectedFeesEmpty')}
      />
    </div>
  )
}

export type ChainPortfolioPanelProps = {
  anchorPosition: string
  owner?: string
  poolAddress?: string
  /** NAV bieżącej głowy łańcucha (z stream-lineage) — linia „w puli teraz”. */
  lpNavUsd?: string | null
  /** Gdy chain-portfolio jeszcze się ładuje / timeout — start z lineage (pierwszy PDA łańcucha). */
  fallbackStartUsd?: string | null
  embedded?: boolean
  className?: string
}

export function ChainPortfolioPanel({
  anchorPosition,
  owner,
  poolAddress,
  lpNavUsd,
  embedded,
  className,
}: ChainPortfolioPanelProps) {
  const { t } = useI18n()
  const anchor = anchorPosition.trim()

  const q = useQuery({
    // Stable key: do not include lpNavUsd (loads async from lineage and would reset this slow query).
    queryKey: ['wallet-chain-portfolio', anchor, owner?.trim() ?? ''],
    queryFn: () =>
      getWalletChainPortfolio({
        anchor_position: anchor,
        owner: owner?.trim() || undefined,
      }),
    enabled: anchor.length > 0,
    staleTime: 60_000,
    gcTime: 5 * 60_000,
    placeholderData: keepPreviousData,
    refetchOnWindowFocus: false,
  })

  const chainId = q.data?.chain_session_id?.trim() ?? ''
  const navUsd = lpNavUsd?.trim() || q.data?.lp_nav_usd?.trim() || undefined
  const ledgerEvents = q.data?.ledger_events ?? []
  const initialLoad = q.isPending && !q.data

  const chainLegs = useMemo(() => {
    const apiLegs = nonZeroChainLegs(q.data?.chain_balance_usd_legs)
    if (apiLegs.length > 0) return apiLegs
    return nonZeroChainLegs(q.data?.metrics?.current_balance_usd_legs)
  }, [q.data?.chain_balance_usd_legs, q.data?.metrics?.current_balance_usd_legs])

  const chainTotalUsd =
    q.data?.portfolio_balance_usd?.trim() ||
    q.data?.metrics?.current_value_usd?.trim() ||
    undefined

  const openFromChainHref =
    chainId && anchor
      ? `/positions/new?${new URLSearchParams({
          chain_session_id: chainId,
          anchor_position: anchor,
          ...(poolAddress?.trim() ? { pool: poolAddress.trim() } : {}),
        }).toString()}`
      : null

  const inner = (
    <div className="space-y-3">
      {q.data?.meta || chainId ? (
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
          {q.data?.meta ? (
            <span
              className={`rounded-full px-2 py-0.5 font-medium ${
                q.data.meta.status === 'closed'
                  ? 'bg-muted text-muted-foreground'
                  : 'bg-emerald-500/15 text-emerald-800 dark:text-emerald-200'
              }`}
            >
              {q.data.meta.status === 'closed' ? t('chainPortfolio.statusClosed') : t('chainPortfolio.statusActive')}
            </span>
          ) : null}
          {chainId ? (
            <span className="font-mono break-all" title={chainId}>
              {t('chainPortfolio.chainIdLabel')}: {chainId}
            </span>
          ) : null}
          {q.data?.meta?.chain_pda_count != null ? (
            <span>{t('chainPortfolio.pdaCount').replace('{n}', String(q.data.meta.chain_pda_count))}</span>
          ) : null}
          {q.data?.meta?.closed_at ? (
            <span>
              {t('chainPortfolio.closedAt')}: {new Date(q.data.meta.closed_at).toLocaleString()}
            </span>
          ) : null}
        </div>
      ) : null}
      {q.error ? <ErrorBanner>{(q.error as Error).message}</ErrorBanner> : null}
      {initialLoad ? (
        <p className="text-sm text-muted-foreground">{t('chainPortfolio.loading')}</p>
      ) : null}
      {!initialLoad && ledgerEvents.length === 0 && !q.error ? (
        <p className="text-sm text-muted-foreground">{t('chainPortfolio.ledgerEmpty')}</p>
      ) : null}
      {!initialLoad && ledgerEvents.length > 0 ? (
        <div className="rounded-md border border-border/60 overflow-x-auto max-h-[28rem] overflow-y-auto">
          <table className="w-full text-sm">
            <thead className="bg-muted/40 sticky top-0 z-10">
              <tr>
                <th className="text-left p-2 font-medium text-xs">{t('chainPortfolio.ledgerColTime')}</th>
                <th className="text-left p-2 font-medium text-xs">{t('chainPortfolio.ledgerColEvent')}</th>
                <th className="text-left p-2 font-medium text-xs">{t('chainPortfolio.ledgerColLegs')}</th>
                <th className="text-right p-2 font-medium text-xs">{t('chainPortfolio.ledgerColTotal')}</th>
              </tr>
            </thead>
            <tbody>
              {ledgerEvents.map((evt, i) => (
                <LedgerEventRow key={`${evt.kind}-${evt.signature ?? evt.event}-${i}`} evt={evt} />
              ))}
            </tbody>
          </table>
        </div>
      ) : null}
      {!initialLoad && q.data ? (
        <div className="space-y-0 rounded-md border border-border/70 bg-muted/20 px-4 py-3">
          {q.isFetching ? (
            <p className="text-[10px] text-muted-foreground mb-2">{t('chainPortfolio.refreshing')}</p>
          ) : null}
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
            <ChainWalletTokensBlock
              legs={chainLegs}
              totalUsd={chainTotalUsd}
              excludedMintCount={q.data.chain_wallet_excluded_mint_count}
              glMatchesPslr={q.data.gl_matches_pslr}
              quality={q.data.quality}
            />
            <div className="sm:border-l sm:border-border/50 sm:pl-3">
              <p className="text-xs text-muted-foreground">{t('chainPortfolio.footerNav')}</p>
              <p className="text-lg font-semibold tabular-nums mt-1">{formatUsdMetric(navUsd)}</p>
            </div>
          </div>
          <ChainCollectedFeesBlock summary={q.data.collected_fees} />
          {embedded && openFromChainHref ? (
            <div className="border-t border-border/50 pt-3 mt-3">
              <Button asChild size="sm" variant="outline">
                <Link to={openFromChainHref}>{t('chainPortfolio.openFromChain')}</Link>
              </Button>
              <p className="text-[11px] text-muted-foreground mt-1.5">{t('chainPortfolio.openFromChainHint')}</p>
            </div>
          ) : null}
        </div>
      ) : null}
    </div>
  )

  if (embedded) {
    return <div className={className}>{inner}</div>
  }

  return (
    <Card className={className}>
      <CardHeader className="flex flex-row items-start justify-between space-y-0 pb-2">
        <div>
          <CardTitle className="text-base">{t('chainPortfolio.title')}</CardTitle>
          {chainId ? (
            <p className="text-xs text-muted-foreground mt-1 font-mono break-all">{chainId}</p>
          ) : (
            <p className="text-xs text-muted-foreground mt-1">{t('chainPortfolio.noChainIdYet')}</p>
          )}
          <p className="text-xs text-muted-foreground mt-1 max-w-2xl">{t('chainPortfolio.retentionNote')}</p>
        </div>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          disabled={q.isFetching}
          onClick={() => void q.refetch()}
          className="shrink-0"
          title={t('chainPortfolio.refresh')}
        >
          <RefreshCw className={`h-4 w-4 ${q.isFetching ? 'animate-spin' : ''}`} />
        </Button>
      </CardHeader>
      <CardContent>{inner}</CardContent>
    </Card>
  )
}
