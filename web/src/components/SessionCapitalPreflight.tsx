import { useQuery } from '@tanstack/react-query'
import { Link } from 'react-router-dom'
import { AlertTriangle, CheckCircle2 } from 'lucide-react'
import { getWalletSessionBalances } from '@/lib/api'
import { humanSessionSource, mintSymbol, sessionSpendCapUi } from '@/lib/sessionCapital'
import { useI18n } from '@/lib/i18n'

export type SessionCapitalPreflightProps = {
  sessionId: string
  owner?: string
  tokenAMint: string
  tokenASymbol: string
  tokenADecimals: number
  tokenBMint: string
  tokenBSymbol: string
  tokenBDecimals: number
  needA: number
  needB: number
  walletHaveA: number
  walletHaveB: number
}

export function useSessionCapitalCheck(props: SessionCapitalPreflightProps | null) {
  const sid = props?.sessionId.trim() ?? ''
  const q = useQuery({
    queryKey: ['wallet-session-balances', sid, props?.owner?.trim() ?? ''],
    queryFn: () =>
      getWalletSessionBalances({
        session_id: sid,
        owner: props?.owner?.trim() || undefined,
      }),
    enabled: !!props && sid.length > 0,
    staleTime: 15_000,
  })

  if (!props || sid.length === 0) {
    return { ready: false as const, blocked: false, q }
  }

  const rowA = q.data?.balances.find((b) => b.mint === props.tokenAMint)
  const rowB = q.data?.balances.find((b) => b.mint === props.tokenBMint)
  const sessionCapA = rowA
    ? sessionSpendCapUi(rowA.amount_raw, rowA.decimals ?? props.tokenADecimals)
    : 0
  const sessionCapB = rowB
    ? sessionSpendCapUi(rowB.amount_raw, rowB.decimals ?? props.tokenBDecimals)
    : 0
  const effectiveHaveA = Math.min(props.walletHaveA, sessionCapA)
  const effectiveHaveB = Math.min(props.walletHaveB, sessionCapB)
  const shortA = props.needA > effectiveHaveA + 1e-8
  const shortB = props.needB > effectiveHaveB + 1e-8
  const deficitA = shortA ? Math.max(0, props.needA - effectiveHaveA) : 0
  const deficitB = shortB ? Math.max(0, props.needB - effectiveHaveB) : 0
  const emptySession = q.isSuccess && (q.data?.balances.length ?? 0) === 0

  return {
    ready: q.isSuccess,
    blocked: shortA || shortB || emptySession,
    shortA,
    shortB,
    deficitA,
    deficitB,
    sessionCapA,
    sessionCapB,
    effectiveHaveA,
    effectiveHaveB,
    emptySession,
    q,
  }
}

export function SessionCapitalPreflight(props: SessionCapitalPreflightProps) {
  const { locale, t } = useI18n()
  const check = useSessionCapitalCheck(props)
  const { q } = check
  const sid = props.sessionId.trim()

  if (!sid) return null

  return (
    <div className="rounded-md border border-dashed border-border/80 bg-muted/20 px-3 py-3 text-sm space-y-2">
      <div className="flex items-start gap-2">
        {check.ready && !check.blocked ? (
          <CheckCircle2 className="h-4 w-4 text-emerald-600 shrink-0 mt-0.5" aria-hidden />
        ) : (
          <AlertTriangle className="h-4 w-4 text-amber-600 shrink-0 mt-0.5" aria-hidden />
        )}
        <div className="space-y-1 min-w-0">
          <p className="font-medium">{t('positionCreate.sessionCapitalTitle')}</p>
          <p className="text-xs text-muted-foreground">{t('positionCreate.sessionCapitalExplain')}</p>
          <p className="text-[11px] font-mono text-muted-foreground break-all" title={sid}>
            {sid.length > 36 ? `${sid.slice(0, 8)}…${sid.slice(-8)}` : sid}
          </p>
        </div>
      </div>

      {q.isLoading ? (
        <p className="text-xs text-muted-foreground">{t('positionCreate.sessionCapitalLoading')}</p>
      ) : null}
      {q.error ? (
        <p className="text-xs text-destructive">{(q.error as Error).message}</p>
      ) : null}

      {check.ready ? (
        <>
          {check.emptySession ? (
            <p className="text-xs text-amber-700 dark:text-amber-200">{t('positionCreate.sessionCapitalEmpty')}</p>
          ) : null}
          <div className="overflow-x-auto rounded border text-xs">
            <table className="w-full">
              <thead className="bg-muted/50">
                <tr>
                  <th className="px-2 py-1 text-left font-medium">{t('positionCreate.sessionColToken')}</th>
                  <th className="px-2 py-1 text-left font-medium">{t('positionCreate.sessionColWallet')}</th>
                  <th className="px-2 py-1 text-left font-medium">{t('positionCreate.sessionColSession')}</th>
                  <th className="px-2 py-1 text-left font-medium">{t('positionCreate.sessionColEffective')}</th>
                  <th className="px-2 py-1 text-left font-medium">{t('positionCreate.sessionColNeed')}</th>
                </tr>
              </thead>
              <tbody>
                <SessionRow
                  symbol={props.tokenASymbol}
                  mint={props.tokenAMint}
                  walletHave={props.walletHaveA}
                  sessionCap={check.sessionCapA}
                  effectiveHave={check.effectiveHaveA}
                  need={props.needA}
                  short={check.shortA}
                />
                <SessionRow
                  symbol={props.tokenBSymbol}
                  mint={props.tokenBMint}
                  walletHave={props.walletHaveB}
                  sessionCap={check.sessionCapB}
                  effectiveHave={check.effectiveHaveB}
                  need={props.needB}
                  short={check.shortB}
                />
              </tbody>
            </table>
          </div>
          {check.blocked && !check.emptySession ? (
            <p className="text-xs text-amber-800 dark:text-amber-100">
              {t('positionCreate.sessionCapitalBlocked').replace(
                '{tokens}',
                [check.shortA ? props.tokenASymbol : null, check.shortB ? props.tokenBSymbol : null]
                  .filter(Boolean)
                  .join(', ') || '—',
              )}
            </p>
          ) : null}
          {q.data ? (
            <p className="text-[11px] text-muted-foreground">
              {t('positionCreate.sessionCapitalSource')}: {humanSessionSource(q.data.source, locale)}
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

function SessionRow({
  symbol,
  mint,
  walletHave,
  sessionCap,
  effectiveHave,
  need,
  short,
}: {
  symbol: string
  mint: string
  walletHave: number
  sessionCap: number
  effectiveHave: number
  need: number
  short: boolean
}) {
  return (
    <tr className="border-t border-border/50">
      <td className="px-2 py-1">
        <span className="font-medium">{symbol}</span>
        <span className="block text-[10px] text-muted-foreground">{mintSymbol(mint)}</span>
      </td>
      <td className="px-2 py-1 tabular-nums">{walletHave.toLocaleString(undefined, { maximumFractionDigits: 8 })}</td>
      <td className="px-2 py-1 tabular-nums">{sessionCap.toLocaleString(undefined, { maximumFractionDigits: 8 })}</td>
      <td className={`px-2 py-1 tabular-nums ${short ? 'text-amber-600 font-medium' : ''}`}>
        {effectiveHave.toLocaleString(undefined, { maximumFractionDigits: 8 })}
      </td>
      <td className="px-2 py-1 tabular-nums">{need.toLocaleString(undefined, { maximumFractionDigits: 8 })}</td>
    </tr>
  )
}
