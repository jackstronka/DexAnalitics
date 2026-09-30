import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useMemo, useState } from 'react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { ErrorBanner } from '@/components/ui/error-banner'
import { AlertTriangle, CheckCircle2, HelpCircle } from 'lucide-react'
import {
  BalancesTable,
  formatRawAmount,
} from '@/components/SessionBalancesPanel'
import {
  getWalletGlBalances,
  getWalletReconcileWalletGl,
  postWalletGlOpeningImport,
  type WalletGlRpcReconcileResponse,
} from '@/lib/api'
import { useI18n } from '@/lib/i18n'
import { shortenAddress } from '@/lib/utils'

function WalletGlSourceBanner({
  kind,
  quality,
  needsReconcile,
}: {
  kind: WalletSourceKind
  quality?: string
  needsReconcile?: boolean
}) {
  const { t } = useI18n()
  const msg = (() => {
    switch (kind) {
      case 'gl':
        return t('walletGl.sourceGl')
      case 'empty':
        return t('walletGl.sourceEmpty')
      case 'disabled':
        return t('walletGl.sourceDisabled')
      case 'no_db':
        return t('walletGl.sourceNoDb')
      default:
        return null
    }
  })()
  const qLabel = (() => {
    switch (quality) {
      case 'exact':
        return t('sessionBalances.qualityExact')
      case 'empty':
        return t('sessionBalances.qualityEmpty')
      case 'disabled':
        return t('sessionBalances.qualityDisabled')
      case 'no_db':
        return t('sessionBalances.qualityNoDb')
      default:
        return quality ? `${t('sessionBalances.qualityUnknown')}: ${quality}` : null
    }
  })()
  if (!msg && !needsReconcile && !qLabel) return null
  const tone =
    kind === 'gl' && !needsReconcile
      ? 'border-emerald-500/40 bg-emerald-500/10 text-emerald-800 dark:text-emerald-200'
      : needsReconcile
        ? 'border-amber-500/40 bg-amber-500/10 text-amber-900 dark:text-amber-100'
        : 'border-border bg-muted/30 text-muted-foreground'
  const Icon = kind === 'gl' && !needsReconcile ? CheckCircle2 : needsReconcile ? AlertTriangle : HelpCircle
  return (
    <div className={`rounded-md border px-3 py-2 text-sm flex gap-2 ${tone}`}>
      <Icon className="h-4 w-4 shrink-0 mt-0.5" aria-hidden />
      <div className="space-y-1">
        {msg ? <span>{msg}</span> : null}
        {qLabel ? <p className="text-xs opacity-90">{qLabel}</p> : null}
        {needsReconcile ? <p className="text-xs font-medium">{t('walletGl.needsReconcileBanner')}</p> : null}
      </div>
    </div>
  )
}

type WalletSourceKind = 'gl' | 'empty' | 'disabled' | 'no_db' | 'unknown'

function parseWalletSource(source: string): WalletSourceKind {
  if (source === 'gl_wallet_shadow') return 'gl'
  if (source === 'gl_wallet_shadow_empty') return 'empty'
  if (source === 'gl_wallet_shadow_disabled') return 'disabled'
  if (source === 'gl_wallet_shadow_no_db') return 'no_db'
  return 'unknown'
}

const WSOL = 'So11111111111111111111111111111111111111112'
const USDC = 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v'

function mintSymbol(mint: string): string {
  if (mint === WSOL) return 'SOL/WSOL'
  if (mint === USDC) return 'USDC'
  return shortenAddress(mint, 4)
}

function WalletGlRpcReconcilePanel({ data }: { data: WalletGlRpcReconcileResponse }) {
  const { t } = useI18n()
  const rows = useMemo(() => {
    const mints = new Set<string>()
    for (const g of data.gaps) mints.add(g.mint)
    if (mints.size === 0) {
      for (const b of data.gl_balances) mints.add(b.mint)
      for (const b of data.rpc_balances) mints.add(b.mint)
    }
    return [...mints].map((mint) => {
      const gap = data.gaps.find((g) => g.mint === mint)
      const gl = gap?.gl_amount_raw ?? data.gl_balances.find((b) => b.mint === mint)?.amount_raw
      const rpc = gap?.rpc_amount_raw ?? data.rpc_balances.find((b) => b.mint === mint)?.amount_raw
      const match = gl != null && rpc != null && gl === rpc
      return { mint, gl, rpc, delta: gap?.delta_raw, match }
    })
  }, [data])

  return (
    <div className="rounded-md border px-3 py-3 text-sm space-y-3">
      <div>
        <p className="font-medium">
          {t('walletGl.rpcReconcileTitle')}:{' '}
          <span className={data.gl_matches_rpc ? 'text-emerald-600' : 'text-amber-600'}>
            {data.gl_matches_rpc ? t('walletGl.rpcReconcileOk') : t('walletGl.rpcReconcileGap')}
          </span>
        </p>
        <p className="text-xs text-muted-foreground mt-1">{data.note}</p>
        <p className="text-[11px] text-muted-foreground mt-1">
          RPC: {data.rpc_confidence}
          {data.rpc_is_stale ? ` · ${t('walletGl.rpcStale')}` : ''}
          {data.rpc_as_of_utc ? ` · ${new Date(data.rpc_as_of_utc).toLocaleString()}` : ''}
        </p>
      </div>
      <div className="overflow-x-auto rounded-md border">
        <table className="w-full text-left text-xs">
          <thead className="bg-muted/50">
            <tr>
              <th className="px-2 py-1.5 font-medium">{t('sessionBalances.colToken')}</th>
              <th className="px-2 py-1.5 font-medium">{t('walletGl.colGl')}</th>
              <th className="px-2 py-1.5 font-medium">{t('walletGl.colRpc')}</th>
              <th className="px-2 py-1.5 font-medium">{t('walletGl.colDelta')}</th>
              <th className="px-2 py-1.5 font-medium">{t('sessionBalances.colMatch')}</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((r) => (
              <tr key={r.mint} className="border-t border-border/60">
                <td className="px-2 py-1.5 font-medium">{mintSymbol(r.mint)}</td>
                <td className="px-2 py-1.5 tabular-nums font-mono text-[11px]">
                  {r.gl != null && r.gl !== '' ? formatRawAmount(r.gl) : '—'}
                </td>
                <td className="px-2 py-1.5 tabular-nums font-mono text-[11px]">
                  {r.rpc != null && r.rpc !== '' ? formatRawAmount(r.rpc) : '—'}
                </td>
                <td className="px-2 py-1.5 tabular-nums font-mono text-[11px] text-muted-foreground">
                  {r.delta != null && r.delta !== '' ? formatRawAmount(r.delta) : '—'}
                </td>
                <td className="px-2 py-1.5">
                  {r.match ? (
                    <span className="text-emerald-600">{t('sessionBalances.matchOk')}</span>
                  ) : (
                    <span className="text-amber-600">{t('sessionBalances.matchGap')}</span>
                  )}
                </td>
            </tr>
            ))}
          </tbody>
        </table>
      </div>
      <p className="text-[11px] text-muted-foreground">{t('walletGl.rpcReconcileFootnote')}</p>
    </div>
  )
}

export type WalletGlBalancesPanelProps = {
  owner: string
  className?: string
}

export function WalletGlBalancesPanel({ owner, className }: WalletGlBalancesPanelProps) {
  const { t } = useI18n()
  const qc = useQueryClient()
  const [showRaw, setShowRaw] = useState(false)
  const pk = owner.trim()

  const q = useQuery({
    queryKey: ['wallet-gl-balances', pk],
    queryFn: () => getWalletGlBalances({ owner: pk }),
    enabled: pk.length > 0,
    staleTime: 15_000,
    refetchInterval: 30_000,
  })

  const openingM = useMutation({
    mutationFn: () => postWalletGlOpeningImport({ owner: pk }),
    onSuccess: async () => {
      await qc.invalidateQueries({ queryKey: ['wallet-gl-balances', pk] })
    },
  })

  const rpcReconcileM = useMutation({
    mutationFn: () => getWalletReconcileWalletGl({ owner: pk }),
  })

  const sourceKind = q.data ? parseWalletSource(q.data.source) : 'unknown'
  const mappedKind =
    sourceKind === 'gl'
      ? 'gl'
      : sourceKind === 'empty'
        ? 'empty'
        : sourceKind === 'disabled'
          ? 'disabled'
          : sourceKind === 'no_db'
            ? 'no_db'
            : 'unknown'

  const openingStatus = openingM.data ? (
    <span className="text-xs text-emerald-700 dark:text-emerald-300 self-center">
      {t('walletGl.openingImportDone').replace('{n}', String(openingM.data.mints_posted))}
    </span>
  ) : null

  return (
    <Card className={className}>
      <CardHeader className="pb-2">
        <CardTitle className="text-base">{t('walletGl.title')}</CardTitle>
        <p className="text-xs text-muted-foreground mt-1 max-w-2xl">{t('walletGl.whatIsThis')}</p>
      </CardHeader>
      <CardContent className="space-y-3">
        {q.data ? (
          <WalletGlSourceBanner
            kind={mappedKind}
            quality={q.data.quality}
            needsReconcile={q.data.needs_reconcile}
          />
        ) : null}
        {q.data?.opening_import_applied ? (
          <p className="text-xs text-muted-foreground">{t('walletGl.openingApplied')}</p>
        ) : null}
        <div className="flex flex-wrap items-center gap-2">
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={!pk || openingM.isPending}
            onClick={() => openingM.mutate()}
            title={t('walletGl.openingImportTitle')}
          >
            {openingM.isPending ? t('walletGl.openingImportPending') : t('walletGl.openingImport')}
          </Button>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={!pk || rpcReconcileM.isPending}
            onClick={() => rpcReconcileM.mutate()}
            title={t('walletGl.rpcReconcileTitle')}
          >
            {rpcReconcileM.isPending ? t('walletGl.rpcReconcilePending') : t('walletGl.rpcReconcile')}
          </Button>
          {openingStatus}
        </div>
        {openingM.error ? <ErrorBanner>{(openingM.error as Error).message}</ErrorBanner> : null}
        {rpcReconcileM.error ? <ErrorBanner>{(rpcReconcileM.error as Error).message}</ErrorBanner> : null}
        {rpcReconcileM.data ? <WalletGlRpcReconcilePanel data={rpcReconcileM.data} /> : null}
        {q.error ? <ErrorBanner>{(q.error as Error).message}</ErrorBanner> : null}
        {q.isLoading ? (
          <p className="text-sm text-muted-foreground">{t('walletGl.loading')}</p>
        ) : !q.data?.balances.length ? (
          <p className="text-sm text-muted-foreground">{t('walletGl.empty')}</p>
        ) : (
          <>
            <div className="flex justify-end">
              <Button type="button" variant="ghost" size="sm" className="h-7 text-xs" onClick={() => setShowRaw((v) => !v)}>
                {showRaw ? t('sessionBalances.hideRaw') : t('sessionBalances.showRaw')}
              </Button>
            </div>
            <BalancesTable balances={q.data.balances} showRaw={showRaw} />
          </>
        )}
      </CardContent>
    </Card>
  )
}
