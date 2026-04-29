import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { History, RefreshCw, Send } from 'lucide-react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { ErrorBanner } from '@/components/ui/error-banner'
import {
  getBotLedger,
  getBotIlLedger,
  getBotRegistry,
  postSlackActivitySummary,
  type BotActivityJsonlResponse,
  type BotRegistryJsonlResponse,
} from '@/lib/api'
import { useI18n } from '@/lib/i18n'

const LIFECYCLE_LEDGER_KEYS = [
  'ts_utc',
  'source',
  'event',
  'signature',
  'tx_fee_lamports',
  'position_pda',
  'position_pubkey',
  'pool_address',
  'rebalance_session_id',
  'fee_payer_net_lamports_delta',
]

const IL_LEDGER_KEYS = [
  'timestamp',
  'event',
  'position',
  'old_position',
  'pool',
  'reason',
  'tx_cost_lamports',
  'rebalance_session_id',
]

function rowCell(v: unknown): string {
  if (v === null || v === undefined) return '—'
  if (typeof v === 'string' || typeof v === 'number' || typeof v === 'boolean') {
    return String(v)
  }
  return JSON.stringify(v)
}

function LedgerTable({
  data,
  columnKeys,
}: {
  data: BotActivityJsonlResponse | BotRegistryJsonlResponse
  columnKeys?: string[]
}) {
  const rows = data.rows
  if (data.file_missing) {
    return (
      <p className="text-sm text-muted-foreground">
        Plik nie istnieje jeszcze — zapis pojawi się po pierwszym tx (CLI/bot). Ścieżka:{' '}
        <code className="text-xs break-all">{data.path}</code>
      </p>
    )
  }
  if (rows.length === 0) {
    return <p className="text-sm text-muted-foreground">Brak wierszy (lub filtr nic nie zwrócił).</p>
  }

  const keys = columnKeys ?? LIFECYCLE_LEDGER_KEYS

  return (
    <div className="overflow-x-auto rounded-md border">
      <table className="w-full text-left text-sm">
        <thead className="bg-muted/50">
          <tr>
            {keys.map((k) => (
              <th key={k} className="px-3 py-2 font-medium whitespace-nowrap">
                {k}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row, i) => (
            <tr key={i} className="border-t border-border/60 hover:bg-muted/30">
              {keys.map((k) => (
                <td key={k} className="px-3 py-2 max-w-[14rem] truncate font-mono text-xs" title={rowCell((row as Record<string, unknown>)[k])}>
                  {rowCell((row as Record<string, unknown>)[k])}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

export default function BotActivity() {
  const { locale } = useI18n()
  const qc = useQueryClient()
  const [filter, setFilter] = useState('')
  const [limit, setLimit] = useState(200)

  const ledgerQ = useQuery({
    queryKey: ['bot-activity-ledger', limit, filter],
    queryFn: () => getBotLedger(limit, filter || undefined),
  })

  const ilLedgerQ = useQuery({
    queryKey: ['bot-activity-il-ledger', limit, filter],
    queryFn: () => getBotIlLedger(limit, filter || undefined),
  })

  const registryQ = useQuery({
    queryKey: ['bot-activity-registry', limit, filter],
    queryFn: () => getBotRegistry(limit, filter || undefined),
  })

  const slackM = useMutation({
    mutationFn: () => postSlackActivitySummary(40),
    onSuccess: (res) => {
      if (!res.webhook_configured || !res.ok) {
        window.alert(
          res.error ??
            (locale === 'pl'
              ? 'Slack: nie wysłano (sprawdź SLACK_WEBHOOK_URL i logi API).'
              : 'Slack: not sent (check SLACK_WEBHOOK_URL and API logs).'),
        )
      } else {
        window.alert(
          locale === 'pl'
            ? `Wysłano digest (${res.rows_included} wierszy) na Slack.`
            : `Digest sent to Slack (${res.rows_included} rows).`,
        )
      }
      void qc.invalidateQueries({ queryKey: ['bot-activity-ledger'] })
      void qc.invalidateQueries({ queryKey: ['bot-activity-il-ledger'] })
    },
    onError: (e: Error) => window.alert(e.message),
  })

  return (
    <div className="space-y-6">
      <div className="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
        <div className="flex items-center gap-3">
          <History className="h-8 w-8 text-primary" />
          <div>
            <h1 className="text-3xl font-bold">{locale === 'pl' ? 'Aktywność bota' : 'Bot activity'}</h1>
            <p className="text-sm text-muted-foreground">
              {locale === 'pl'
                ? <>Historia z plików JSONL: koszty tx (lifecycle), opcjonalnie IL / zdarzenia <code className="text-xs">rebalance</code>, rejestr pozycji — jak timeline bota.</>
                : <>History from JSONL files: tx costs (lifecycle), optional IL / <code className="text-xs">rebalance</code> events, and position registry — bot timeline style.</>}
            </p>
          </div>
        </div>
        <div className="flex flex-wrap gap-2">
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              void qc.invalidateQueries({ queryKey: ['bot-activity-ledger'] })
              void qc.invalidateQueries({ queryKey: ['bot-activity-il-ledger'] })
              void qc.invalidateQueries({ queryKey: ['bot-activity-registry'] })
            }}
          >
            <RefreshCw className="h-4 w-4 mr-2" />
            {locale === 'pl' ? 'Odśwież' : 'Refresh'}
          </Button>
          <Button
            size="sm"
            onClick={() => slackM.mutate()}
            disabled={slackM.isPending}
          >
            <Send className="h-4 w-4 mr-2" />
            {locale === 'pl' ? 'Wyślij skrót na Slack' : 'Send summary to Slack'}
          </Button>
        </div>
      </div>

      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-base">{locale === 'pl' ? 'Filtr' : 'Filter'}</CardTitle>
        </CardHeader>
        <CardContent className="flex flex-col gap-3 sm:flex-row sm:items-end">
          <div className="flex-1 space-y-1">
            <label className="text-xs text-muted-foreground">
              {locale === 'pl'
                ? 'Substring w JSON (np. fragment PDA pozycji)'
                : 'Substring in JSON (e.g. part of position PDA)'}
            </label>
            <input
              className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
              placeholder={locale === 'pl' ? 'opcjonalnie' : 'optional'}
            />
          </div>
          <div className="w-full sm:w-32 space-y-1">
            <label className="text-xs text-muted-foreground">{locale === 'pl' ? 'Limit wierszy' : 'Row limit'}</label>
            <input
              type="number"
              min={1}
              max={2000}
              className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
              value={limit}
              onChange={(e) => setLimit(Number(e.target.value) || 200)}
            />
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>{locale === 'pl' ? 'Lifecycle ledger' : 'Lifecycle ledger'}</CardTitle>
          <p className="text-sm text-muted-foreground">
            {ledgerQ.data?.path ?? '…'} — {locale === 'pl' ? 'dopasowanych wierszy' : 'matching rows'}: {ledgerQ.data?.total_matching_lines ?? '—'}, {locale === 'pl' ? 'zwrócono' : 'returned'}: {ledgerQ.data?.rows_returned ?? '—'}
          </p>
        </CardHeader>
        <CardContent>
          {ledgerQ.isLoading && <p className="text-sm text-muted-foreground">{locale === 'pl' ? 'Ładowanie…' : 'Loading…'}</p>}
          {ledgerQ.isError && (
            <ErrorBanner>{(ledgerQ.error as Error).message}</ErrorBanner>
          )}
          {ledgerQ.data && <LedgerTable data={ledgerQ.data} />}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>{locale === 'pl' ? 'IL / rebalance ledger' : 'IL / rebalance ledger'}</CardTitle>
          <p className="text-sm text-muted-foreground">
            {ilLedgerQ.data?.path ?? '…'} — {locale === 'pl' ? 'dopasowanych' : 'matching'}: {ilLedgerQ.data?.total_matching_lines ?? '—'}, {locale === 'pl' ? 'zwrócono' : 'returned'}:{' '}
            {ilLedgerQ.data?.rows_returned ?? '—'}. {locale === 'pl' ? 'Ustaw na hoście API to samo co bot' : 'Set the same on API host as bot'}:{' '}
            <code className="text-xs">CLMM_IL_LEDGER_PATH</code> (= <code className="text-xs">--il-ledger-path</code>).
          </p>
        </CardHeader>
        <CardContent>
          {ilLedgerQ.isLoading && <p className="text-sm text-muted-foreground">{locale === 'pl' ? 'Ładowanie…' : 'Loading…'}</p>}
          {ilLedgerQ.isError && (
            <ErrorBanner>{(ilLedgerQ.error as Error).message}</ErrorBanner>
          )}
          {ilLedgerQ.data && <LedgerTable data={ilLedgerQ.data} columnKeys={IL_LEDGER_KEYS} />}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>{locale === 'pl' ? 'Rejestr pozycji' : 'Position registry'}</CardTitle>
          <p className="text-sm text-muted-foreground">
            {registryQ.data?.path ?? '…'} — {locale === 'pl' ? 'dopasowanych' : 'matching'}: {registryQ.data?.total_matching_lines ?? '—'}
          </p>
        </CardHeader>
        <CardContent>
          {registryQ.isLoading && <p className="text-sm text-muted-foreground">{locale === 'pl' ? 'Ładowanie…' : 'Loading…'}</p>}
          {registryQ.isError && (
            <ErrorBanner>{(registryQ.error as Error).message}</ErrorBanner>
          )}
          {registryQ.data && <LedgerTable data={registryQ.data} />}
        </CardContent>
      </Card>

      <p className="text-xs text-muted-foreground">
        API musi działać z katalogu repo (względne ścieżki <code>data/ledger/…</code>), a proces musi widzieć te same zmienne co CLI:{' '}
        <code>CLMM_POSITION_LIFECYCLE_LEDGER_PATH</code>, <code>CLMM_IL_LEDGER_PATH</code> (żeby pokazać IL ledger),{' '}
        <code>CLMM_POSITION_REGISTRY_PATH</code>. Slack: <code>SLACK_WEBHOOK_URL</code> w środowisku serwera API. Agregacja:{' '}
        <code className="text-xs">clmm-lp-cli ledger-rebalance-summary</code>.
      </p>
    </div>
  )
}
