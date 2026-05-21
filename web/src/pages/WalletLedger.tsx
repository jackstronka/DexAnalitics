import { useMemo, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Link } from 'react-router-dom'
import { ClipboardList, ExternalLink, RefreshCw } from 'lucide-react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { ErrorBanner } from '@/components/ui/error-banner'
import { SessionBalancesPanel } from '@/components/SessionBalancesPanel'
import { getWalletLedgerEvents, type WalletLedgerStatus } from '@/lib/api'
import { getDevWalletPubkey } from '@/lib/devWallet'
import { solscanAccountUrl, solscanTxUrl } from '@/lib/explorer'
import { useI18n } from '@/lib/i18n'
import { shortenAddress } from '@/lib/utils'

const LEDGER_KINDS = [
  '',
  'swap_before_open',
  'open_position',
  'close_position',
  'collect_fees',
  'decrease_liquidity',
  'rebalance_position',
  'transfer_sol',
  'convert_sol',
] as const

const LEDGER_STATUSES: Array<'' | WalletLedgerStatus> = ['', 'pending', 'confirmed', 'failed']

export default function WalletLedger() {
  const { t } = useI18n()
  const [owner, setOwner] = useState(() => getDevWalletPubkey() ?? '')
  const [kind, setKind] = useState('')
  const [status, setStatus] = useState<'' | WalletLedgerStatus>('')
  const [limit, setLimit] = useState(200)
  const [sessionId, setSessionId] = useState('')

  const q = useQuery({
    queryKey: ['wallet-ledger-events', owner.trim(), kind, status, limit],
    queryFn: () =>
      getWalletLedgerEvents({
        owner: owner.trim() || undefined,
        kind: kind || undefined,
        status: status || undefined,
        limit,
      }),
    staleTime: 10_000,
    refetchInterval: 30_000,
  })

  const kindCounts = useMemo(() => {
    const m = new Map<string, number>()
    for (const ev of q.data?.events ?? []) {
      m.set(ev.kind, (m.get(ev.kind) ?? 0) + 1)
    }
    return m
  }, [q.data?.events])

  const latestSessionFromEvents = useMemo(() => {
    for (const ev of q.data?.events ?? []) {
      const sid = ev.cost_session_id?.trim()
      if (sid) return sid
    }
    return ''
  }, [q.data?.events])

  const statusClass = (s: string) => {
    switch (s) {
      case 'pending':
        return 'bg-amber-100 text-amber-900 border-amber-200'
      case 'confirmed':
        return 'bg-emerald-100 text-emerald-900 border-emerald-200'
      case 'failed':
        return 'bg-rose-100 text-rose-900 border-rose-200'
      default:
        return 'bg-muted text-muted-foreground'
    }
  }

  return (
    <div className="space-y-6">
      <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
        <div>
          <h1 className="text-3xl font-bold flex items-center gap-2">
            <ClipboardList className="h-8 w-8" />
            {t('walletLedger.title')}
          </h1>
          <p className="text-muted-foreground text-sm mt-1 max-w-3xl">{t('walletLedger.subtitle')}</p>
          <p className="text-muted-foreground text-xs mt-2 max-w-3xl">{t('walletLedger.glNote')}</p>
          <p className="mt-2 text-sm">
            <Link to="/wallet" className="text-primary underline-offset-4 hover:underline">
              ← {t('nav.wallet')}
            </Link>
          </p>
        </div>
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={q.isFetching}
          onClick={() => void q.refetch()}
          className="shrink-0 gap-1"
        >
          <RefreshCw className={`h-4 w-4 ${q.isFetching ? 'animate-spin' : ''}`} />
          {t('walletLedger.refresh')}
        </Button>
      </div>

      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-base">{t('walletLedger.filters')}</CardTitle>
        </CardHeader>
        <CardContent className="flex flex-wrap items-end gap-4">
          <label className="flex flex-col gap-1 text-sm min-w-[12rem] flex-1">
            <span className="text-muted-foreground">{t('walletLedger.ownerFilter')}</span>
            <input
              className="flex h-9 w-full max-w-md rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm"
              value={owner}
              onChange={(e) => setOwner(e.target.value)}
              placeholder={t('walletLedger.ownerPlaceholder')}
            />
          </label>
          <label className="flex flex-col gap-1 text-sm min-w-[10rem]">
            <span className="text-muted-foreground">{t('walletLedger.kindFilter')}</span>
            <select
              className="flex h-9 rounded-md border border-input bg-background px-2 text-sm shadow-sm"
              value={kind}
              onChange={(e) => setKind(e.target.value)}
            >
              <option value="">{t('walletLedger.kindAll')}</option>
              {LEDGER_KINDS.filter(Boolean).map((k) => (
                <option key={k} value={k}>
                  {k}
                </option>
              ))}
            </select>
          </label>
          <label className="flex flex-col gap-1 text-sm min-w-[8rem]">
            <span className="text-muted-foreground">{t('walletLedger.statusFilter')}</span>
            <select
              className="flex h-9 rounded-md border border-input bg-background px-2 text-sm shadow-sm"
              value={status}
              onChange={(e) => setStatus(e.target.value as '' | WalletLedgerStatus)}
            >
              <option value="">{t('walletLedger.statusAll')}</option>
              {LEDGER_STATUSES.filter(Boolean).map((s) => (
                <option key={s} value={s}>
                  {s}
                </option>
              ))}
            </select>
          </label>
          <label className="flex flex-col gap-1 text-sm">
            <span className="text-muted-foreground">{t('walletLedger.limit')}</span>
            <select
              className="flex h-9 rounded-md border border-input bg-background px-2 text-sm shadow-sm"
              value={limit}
              onChange={(e) => setLimit(Number(e.target.value))}
            >
              {[50, 200, 500].map((n) => (
                <option key={n} value={n}>
                  {n}
                </option>
              ))}
            </select>
          </label>
          {getDevWalletPubkey() ? (
            <Button
              type="button"
              variant="secondary"
              size="sm"
              className="h-9"
              onClick={() => setOwner(getDevWalletPubkey() ?? '')}
            >
              {t('walletLedger.useDevWallet')}
            </Button>
          ) : null}
          <label className="flex flex-col gap-1 text-sm min-w-[14rem] flex-1">
            <span className="text-muted-foreground">{t('walletLedger.sessionFilter')}</span>
            <input
              className="flex h-9 w-full max-w-md rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm font-mono"
              value={sessionId}
              onChange={(e) => setSessionId(e.target.value)}
              placeholder={t('walletLedger.sessionPlaceholder')}
            />
          </label>
          {latestSessionFromEvents ? (
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="h-9"
              onClick={() => setSessionId(latestSessionFromEvents)}
            >
              {t('walletLedger.pickSessionFromEvents')}
            </Button>
          ) : null}
        </CardContent>
      </Card>

      {sessionId.trim() ? (
        <SessionBalancesPanel sessionId={sessionId.trim()} owner={owner.trim() || undefined} />
      ) : null}

      {q.error ? <ErrorBanner>{(q.error as Error).message}</ErrorBanner> : null}

      <Card>
        <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
          <CardTitle className="text-base">{t('walletLedger.filePath')}</CardTitle>
          {q.data && q.data.events.length > 0 ? (
            <span className="text-xs text-muted-foreground">
              {t('walletLedger.rowCount').replace('{n}', String(q.data.events.length))}
            </span>
          ) : null}
        </CardHeader>
        <CardContent className="space-y-3">
          {q.data ? (
            <div className="space-y-1">
              <p className="text-xs font-mono break-all text-muted-foreground">{q.data.path}</p>
              {q.data.storage ? (
                <p className="text-xs text-muted-foreground">
                  {t('walletLedger.storage')}:{' '}
                  <code className="rounded bg-muted px-1 py-0.5">{q.data.storage}</code>
                </p>
              ) : null}
            </div>
          ) : null}
          {kindCounts.size > 0 ? (
            <p className="text-xs text-muted-foreground">
              {t('walletLedger.kindSummary')}:{' '}
              {[...kindCounts.entries()]
                .map(([k, n]) => `${k} (${n})`)
                .join(' · ')}
            </p>
          ) : null}
          {q.isLoading ? (
            <p className="text-sm text-muted-foreground">…</p>
          ) : !q.data?.events.length ? (
            <p className="text-sm text-muted-foreground">{t('walletLedger.empty')}</p>
          ) : (
            <div className="overflow-x-auto rounded-md border">
              <table className="w-full text-left text-sm">
                <thead className="bg-muted/50">
                  <tr>
                    <th className="px-3 py-2 font-medium whitespace-nowrap">{t('walletLedger.colTime')}</th>
                    <th className="px-3 py-2 font-medium whitespace-nowrap">{t('walletLedger.colStatus')}</th>
                    <th className="px-3 py-2 font-medium whitespace-nowrap">{t('walletLedger.colKind')}</th>
                    <th className="px-3 py-2 font-medium whitespace-nowrap">{t('walletLedger.colOwner')}</th>
                    <th className="px-3 py-2 font-medium whitespace-nowrap">{t('walletLedger.colCorr')}</th>
                    <th className="px-3 py-2 font-medium whitespace-nowrap">{t('walletLedger.colSig')}</th>
                    <th className="px-3 py-2 font-medium whitespace-nowrap">{t('walletLedger.colDeltas')}</th>
                    <th className="px-3 py-2 font-medium whitespace-nowrap">{t('walletLedger.colErr')}</th>
                  </tr>
                </thead>
                <tbody>
                  {q.data.events.map((ev) => (
                    <tr key={ev.event_id} className="border-t border-border/60 hover:bg-muted/30">
                      <td className="px-3 py-2 whitespace-nowrap text-xs font-mono">{ev.ts_utc}</td>
                      <td className="px-3 py-2">
                        <span
                          className={`rounded border px-2 py-0.5 text-xs font-medium ${statusClass(ev.status)}`}
                        >
                          {ev.status}
                        </span>
                      </td>
                      <td className="px-3 py-2 text-xs">{ev.kind}</td>
                      <td className="px-3 py-2 text-xs font-mono" title={ev.owner ?? ''}>
                        {ev.owner ? (
                          <a
                            href={solscanAccountUrl(ev.owner)}
                            target="_blank"
                            rel="noopener noreferrer"
                            className="inline-flex items-center gap-0.5 text-primary hover:underline"
                          >
                            {ev.owner.length > 12 ? shortenAddress(ev.owner, 4) : ev.owner}
                            <ExternalLink className="h-3 w-3 shrink-0 opacity-70" />
                          </a>
                        ) : (
                          '—'
                        )}
                      </td>
                      <td
                        className="px-3 py-2 max-w-[8rem] truncate text-xs font-mono"
                        title={ev.correlation_id}
                      >
                        {ev.correlation_id.length > 12 ? `${ev.correlation_id.slice(0, 8)}…` : ev.correlation_id}
                      </td>
                      <td
                        className="px-3 py-2 max-w-[10rem] truncate text-xs font-mono"
                        title={ev.signature ?? ''}
                      >
                        {ev.signature && ev.signature.length > 12 ? (
                          <a
                            href={solscanTxUrl(ev.signature)}
                            target="_blank"
                            rel="noopener noreferrer"
                            className="inline-flex items-center gap-0.5 text-primary hover:underline"
                          >
                            {shortenAddress(ev.signature, 4)}
                            <ExternalLink className="h-3 w-3 shrink-0 opacity-70" />
                          </a>
                        ) : (
                          ev.signature || '—'
                        )}
                      </td>
                      <td
                        className="px-3 py-2 max-w-[18rem] truncate text-xs font-mono"
                        title={JSON.stringify(ev.deltas)}
                      >
                        {ev.deltas.length > 0
                          ? ev.deltas
                              .map((d) =>
                                d.mint.length > 10
                                  ? `${shortenAddress(d.mint, 4)}:${d.raw_delta_i128}`
                                  : `${d.mint}:${d.raw_delta_i128}`,
                              )
                              .join('; ')
                          : ev.native_lamports_delta
                            ? `lamports:${ev.native_lamports_delta}`
                            : '—'}
                      </td>
                      <td className="px-3 py-2 max-w-[12rem] truncate text-xs text-rose-700" title={ev.error ?? ''}>
                        {ev.error ?? '—'}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  )
}
