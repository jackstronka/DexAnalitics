import { useQuery } from '@tanstack/react-query'
import { Link } from 'react-router-dom'
import { AlertTriangle, CheckCircle2 } from 'lucide-react'
import { getWalletChainPortfolio } from '@/lib/api'
import { chainInventoryUi } from '@/lib/chainCapital'
import { mintSymbol } from '@/lib/sessionCapital'
import { useI18n } from '@/lib/i18n'

export type ChainCapitalPreflightProps = {
  chainSessionId: string
  anchorPosition?: string
  owner?: string
  tokenAMint: string
  tokenASymbol: string
  tokenADecimals: number
  tokenBMint: string
  tokenBSymbol: string
  tokenBDecimals: number
  needA: number
  needB: number
}

export function useChainCapitalCheck(props: ChainCapitalPreflightProps | null) {
  const cid = props?.chainSessionId.trim() ?? ''
  const q = useQuery({
    queryKey: ['chain-portfolio-preflight', cid, props?.anchorPosition?.trim() ?? '', props?.owner?.trim() ?? ''],
    queryFn: () =>
      getWalletChainPortfolio({
        chain_session_id: cid,
        anchor_position: props?.anchorPosition?.trim() || undefined,
        owner: props?.owner?.trim() || undefined,
      }),
    enabled: !!props && cid.length > 0,
    staleTime: 15_000,
  })

  if (!props || cid.length === 0) {
    return { ready: false as const, blocked: false, q }
  }

  const chainCapA = chainInventoryUi(q.data, props.tokenAMint, props.tokenADecimals)
  const chainCapB = chainInventoryUi(q.data, props.tokenBMint, props.tokenBDecimals)
  const shortA = props.needA > chainCapA + 1e-8
  const shortB = props.needB > chainCapB + 1e-8
  const emptyChain = q.isSuccess && chainCapA <= 0 && chainCapB <= 0

  return {
    ready: q.isSuccess,
    blocked: shortA || shortB || emptyChain,
    shortA,
    shortB,
    chainCapA,
    chainCapB,
    emptyChain,
    q,
  }
}

export function ChainCapitalPreflight(props: ChainCapitalPreflightProps) {
  const { t } = useI18n()
  const check = useChainCapitalCheck(props)
  const { q } = check
  const cid = props.chainSessionId.trim()

  if (!cid) return null

  return (
    <div className="rounded-md border border-dashed border-primary/40 bg-primary/5 px-3 py-3 text-sm space-y-2">
      <div className="flex items-start gap-2">
        {check.ready && !check.blocked ? (
          <CheckCircle2 className="h-4 w-4 text-emerald-600 shrink-0 mt-0.5" aria-hidden />
        ) : (
          <AlertTriangle className="h-4 w-4 text-amber-600 shrink-0 mt-0.5" aria-hidden />
        )}
        <div className="space-y-1 min-w-0">
          <p className="font-medium">{t('positionCreate.chainCapitalTitle')}</p>
          <p className="text-xs text-muted-foreground">{t('positionCreate.chainCapitalExplain')}</p>
          <p className="text-[11px] font-mono text-muted-foreground break-all" title={cid}>
            CHAIN:{cid.length > 36 ? `${cid.slice(0, 8)}…${cid.slice(-8)}` : cid}
          </p>
        </div>
      </div>

      {q.isLoading ? <p className="text-xs text-muted-foreground">{t('positionCreate.chainCapitalLoading')}</p> : null}
      {q.error ? <p className="text-xs text-destructive">{(q.error as Error).message}</p> : null}

      {check.ready ? (
        <>
          {check.emptyChain ? (
            <p className="text-xs text-amber-700 dark:text-amber-200">{t('positionCreate.chainCapitalEmpty')}</p>
          ) : null}
          <div className="overflow-x-auto rounded border text-xs">
            <table className="w-full">
              <thead className="bg-muted/50">
                <tr>
                  <th className="px-2 py-1 text-left font-medium">{t('positionCreate.sessionColToken')}</th>
                  <th className="px-2 py-1 text-left font-medium">{t('positionCreate.chainColInventory')}</th>
                  <th className="px-2 py-1 text-left font-medium">{t('positionCreate.sessionColNeed')}</th>
                </tr>
              </thead>
              <tbody>
                <ChainRow
                  symbol={props.tokenASymbol}
                  mint={props.tokenAMint}
                  chainCap={check.chainCapA}
                  need={props.needA}
                  short={check.shortA}
                />
                <ChainRow
                  symbol={props.tokenBSymbol}
                  mint={props.tokenBMint}
                  chainCap={check.chainCapB}
                  need={props.needB}
                  short={check.shortB}
                />
              </tbody>
            </table>
          </div>
          {check.blocked && !check.emptyChain ? (
            <p className="text-xs text-amber-800 dark:text-amber-100">
              {t('positionCreate.chainCapitalBlocked').replace(
                '{tokens}',
                [check.shortA ? props.tokenASymbol : null, check.shortB ? props.tokenBSymbol : null]
                  .filter(Boolean)
                  .join(', ') || '—',
              )}
            </p>
          ) : null}
          {q.data ? (
            <p className="text-[11px] text-muted-foreground">
              {t('positionCreate.chainCapitalSource')}: {q.data.source}
              {' · '}
              <Link to="/wallet-ledger" className="text-primary hover:underline">
                {t('positionCreate.sessionCapitalLedgerLink')}
              </Link>
            </p>
          ) : null}
        </>
      ) : null}
    </div>
  )
}

function ChainRow({
  symbol,
  mint,
  chainCap,
  need,
  short,
}: {
  symbol: string
  mint: string
  chainCap: number
  need: number
  short: boolean
}) {
  return (
    <tr className="border-t border-border/50">
      <td className="px-2 py-1">
        <span className="font-medium">{symbol}</span>
        <span className="block text-[10px] text-muted-foreground">{mintSymbol(mint)}</span>
      </td>
      <td className={`px-2 py-1 tabular-nums ${short ? 'text-amber-600 font-medium' : ''}`}>
        {chainCap.toLocaleString(undefined, { maximumFractionDigits: 8 })}
      </td>
      <td className="px-2 py-1 tabular-nums">{need.toLocaleString(undefined, { maximumFractionDigits: 8 })}</td>
    </tr>
  )
}
