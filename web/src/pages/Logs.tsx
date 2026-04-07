import { useMemo, useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { ScrollText, RefreshCw } from 'lucide-react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import {
  getBotIlLedger,
  getBotLedger,
  getBotRegistry,
  getPendingOpenRecovery,
  type BotActivityJsonlResponse,
  type BotRegistryJsonlResponse,
  type PendingOpenRecoveryResponse,
} from '@/lib/api'

function rowCell(v: unknown): string {
  if (v === null || v === undefined) return '—'
  if (typeof v === 'string' || typeof v === 'number' || typeof v === 'boolean') return String(v)
  return JSON.stringify(v)
}

function JsonlTable({
  data,
  columnKeys,
}: {
  data: BotActivityJsonlResponse | BotRegistryJsonlResponse
  columnKeys: string[]
}) {
  if (data.file_missing) {
    return (
      <p className="text-sm text-muted-foreground">
        Plik nie istnieje jeszcze. Ścieżka: <code className="text-xs break-all">{data.path}</code>
      </p>
    )
  }
  if (!data.rows?.length) {
    return <p className="text-sm text-muted-foreground">Brak wierszy (albo filtr nic nie zwrócił).</p>
  }
  return (
    <div className="overflow-x-auto rounded-md border">
      <table className="w-full text-left text-sm">
        <thead className="bg-muted/50">
          <tr>
            {columnKeys.map((k) => (
              <th key={k} className="px-3 py-2 font-medium whitespace-nowrap">
                {k}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {data.rows.map((row, i) => (
            <tr key={i} className="border-t border-border/60 hover:bg-muted/30">
              {columnKeys.map((k) => (
                <td
                  key={k}
                  className="px-3 py-2 max-w-[18rem] truncate font-mono text-xs"
                  title={rowCell((row as Record<string, unknown>)[k])}
                >
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

const LIFECYCLE_KEYS = [
  'ts_utc',
  'source',
  'event',
  'signature',
  'position_pda',
  'pool_address',
  'rebalance_session_id',
  'tx_fee_lamports',
  'details',
]

const IL_KEYS = ['timestamp', 'event', 'old_position', 'position', 'pool', 'reason', 'tx_cost_lamports', 'hint']

const REGISTRY_KEYS = ['ts_utc', 'event', 'position', 'pool', 'owner', 'signature']

export default function Logs() {
  const qc = useQueryClient()
  const [filter, setFilter] = useState('')
  const [limit, setLimit] = useState(300)

  const ledgerQ = useQuery({
    queryKey: ['logs-ledger', limit, filter],
    queryFn: () => getBotLedger(limit, filter || undefined),
  })
  const ilQ = useQuery({
    queryKey: ['logs-il', limit, filter],
    queryFn: () => getBotIlLedger(limit, filter || undefined),
  })
  const registryQ = useQuery({
    queryKey: ['logs-registry', limit, filter],
    queryFn: () => getBotRegistry(limit, filter || undefined),
  })
  const pendingQ = useQuery({
    queryKey: ['logs-pending-open'],
    queryFn: getPendingOpenRecovery,
    staleTime: 5_000,
  })

  const lastIncomplete = useMemo(() => {
    const rows = (ilQ.data?.rows ?? []) as Record<string, unknown>[]
    for (const r of rows) {
      if (r?.event === 'rebalance_incomplete') return r
    }
    return null
  }, [ilQ.data?.rows])

  const closeWithoutOpen = useMemo(() => {
    const rows = (ledgerQ.data?.rows ?? []) as Record<string, unknown>[]
    const bySid = new Map<string, { close: number; open: number; lastTs: string | null }>()
    for (const r of rows) {
      if (r?.source !== 'orca_bot') continue
      const sidRaw = r.rebalance_session_id
      const sid = typeof sidRaw === 'string' && sidRaw.trim() ? sidRaw.trim() : null
      if (!sid) continue
      const e = typeof r.event === 'string' ? r.event : ''
      const ts = typeof r.ts_utc === 'string' ? r.ts_utc : null
      const cur = bySid.get(sid) ?? { close: 0, open: 0, lastTs: null }
      if (e === 'bot_close_position') cur.close += 1
      if (e === 'bot_open_position' || e === 'bot_open_position_full_range') cur.open += 1
      cur.lastTs = ts ?? cur.lastTs
      bySid.set(sid, cur)
    }
    const out: { sid: string; lastTs: string | null }[] = []
    for (const [sid, v] of bySid.entries()) {
      if (v.close > 0 && v.open === 0) out.push({ sid, lastTs: v.lastTs })
    }
    return out.slice(0, 10)
  }, [ledgerQ.data?.rows])

  return (
    <div className="space-y-6">
      <div className="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
        <div className="flex items-center gap-3">
          <ScrollText className="h-8 w-8 text-primary" />
          <div>
            <h1 className="text-3xl font-bold">Logs</h1>
            <p className="text-sm text-muted-foreground">
              Najnowsze zdarzenia bota niezależnie od tego, czy pozycja nadal istnieje. Szukaj{' '}
              <code className="text-xs">rebalance_incomplete</code>.
            </p>
          </div>
        </div>
        <Button
          variant="outline"
          size="sm"
          onClick={() => {
            void qc.invalidateQueries({ queryKey: ['logs-ledger'] })
            void qc.invalidateQueries({ queryKey: ['logs-il'] })
            void qc.invalidateQueries({ queryKey: ['logs-registry'] })
            void qc.invalidateQueries({ queryKey: ['logs-pending-open'] })
          }}
        >
          <RefreshCw className="h-4 w-4 mr-2" />
          Odśwież
        </Button>
      </div>

      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-base">Filtr (substring w JSON)</CardTitle>
        </CardHeader>
        <CardContent className="flex flex-col gap-3 sm:flex-row sm:items-end">
          <div className="flex-1 space-y-1">
            <label className="text-xs text-muted-foreground">np. fragment PDA, pool, signature, session id</label>
            <input
              className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
              placeholder="opcjonalnie"
            />
          </div>
          <div className="w-full sm:w-32 space-y-1">
            <label className="text-xs text-muted-foreground">Limit</label>
            <input
              type="number"
              min={1}
              max={2000}
              className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
              value={limit}
              onChange={(e) => setLimit(Number(e.target.value) || 300)}
            />
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Last rebalance incomplete</CardTitle>
          <p className="text-sm text-muted-foreground">
            Źródło: <code className="text-xs">/bot-activity/il-ledger</code> (wymaga{' '}
            <code className="text-xs">CLMM_IL_LEDGER_PATH</code>).
          </p>
        </CardHeader>
        <CardContent className="text-sm">
          {ilQ.data?.file_missing ? (
            <p className="text-muted-foreground">
              IL ledger nie jest skonfigurowany: <code className="text-xs break-all">{ilQ.data.path}</code>
            </p>
          ) : lastIncomplete ? (
            <div className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs space-y-1">
              <div>
                <span className="font-medium">timestamp:</span> {String(lastIncomplete.timestamp ?? '—')}
              </div>
              <div className="break-words">
                <span className="font-medium">error:</span> {String(lastIncomplete.error ?? '—')}
              </div>
              <div className="break-words">
                <span className="font-medium">hint:</span> {String(lastIncomplete.hint ?? '—')}
              </div>
              <div className="break-words">
                <span className="font-medium">old → new:</span> {String(lastIncomplete.old_position ?? '—')} →{' '}
                {String(lastIncomplete.position ?? '—')}
              </div>
              <div>
                <span className="font-medium">session:</span> {String(lastIncomplete.rebalance_session_id ?? '—')}
              </div>
            </div>
          ) : (
            <p className="text-muted-foreground">Brak `rebalance_incomplete` w ostatnich wierszach (albo za niski limit).</p>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Pending open recovery</CardTitle>
          <p className="text-sm text-muted-foreground">
            Źródło: <code className="text-xs">CLMM_PENDING_OPEN_RECOVERY_PATH</code> (domyślnie{' '}
            <code className="text-xs">data/pending-open-recovery.json</code>).
          </p>
        </CardHeader>
        <CardContent className="text-sm">
          {pendingQ.isLoading ? (
            <p className="text-muted-foreground">Ładowanie…</p>
          ) : pendingQ.isError ? (
            <p className="text-destructive">{(pendingQ.error as Error).message}</p>
          ) : pendingQ.data ? (
            <PendingOpenBox data={pendingQ.data} />
          ) : null}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Lifecycle ledger</CardTitle>
          <p className="text-sm text-muted-foreground">
            {ledgerQ.data?.path ?? '…'} — dopasowanych: {ledgerQ.data?.total_matching_lines ?? '—'}, zwrócono:{' '}
            {ledgerQ.data?.rows_returned ?? '—'}
          </p>
        </CardHeader>
        <CardContent>
          {closeWithoutOpen.length > 0 && (
            <div className="mb-3 rounded-md border border-amber-500/35 bg-amber-500/5 px-3 py-2 text-sm">
              <div className="font-medium">Sessions with close but no open (recent)</div>
              <div className="text-xs text-muted-foreground mt-1">
                Kliknij, żeby wypełnić filtr <code className="text-[11px]">rebalance_session_id</code>.
              </div>
              <div className="flex flex-wrap gap-2 mt-2">
                {closeWithoutOpen.map((x) => (
                  <Button
                    key={x.sid}
                    type="button"
                    size="sm"
                    variant="secondary"
                    onClick={() => setFilter(x.sid)}
                    title={x.lastTs ?? undefined}
                  >
                    {x.sid}
                  </Button>
                ))}
              </div>
            </div>
          )}
          {ledgerQ.isLoading && <p className="text-sm text-muted-foreground">Ładowanie…</p>}
          {ledgerQ.isError && <p className="text-sm text-destructive">{(ledgerQ.error as Error).message}</p>}
          {ledgerQ.data && <JsonlTable data={ledgerQ.data} columnKeys={LIFECYCLE_KEYS} />}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>IL / rebalance ledger</CardTitle>
          <p className="text-sm text-muted-foreground">
            {ilQ.data?.path ?? '…'} — dopasowanych: {ilQ.data?.total_matching_lines ?? '—'}, zwrócono: {ilQ.data?.rows_returned ?? '—'}
          </p>
        </CardHeader>
        <CardContent>
          {ilQ.isLoading && <p className="text-sm text-muted-foreground">Ładowanie…</p>}
          {ilQ.isError && <p className="text-sm text-destructive">{(ilQ.error as Error).message}</p>}
          {ilQ.data && <JsonlTable data={ilQ.data} columnKeys={IL_KEYS} />}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Position registry</CardTitle>
          <p className="text-sm text-muted-foreground">
            {registryQ.data?.path ?? '…'} — dopasowanych: {registryQ.data?.total_matching_lines ?? '—'}
          </p>
        </CardHeader>
        <CardContent>
          {registryQ.isLoading && <p className="text-sm text-muted-foreground">Ładowanie…</p>}
          {registryQ.isError && <p className="text-sm text-destructive">{(registryQ.error as Error).message}</p>}
          {registryQ.data && <JsonlTable data={registryQ.data} columnKeys={REGISTRY_KEYS} />}
        </CardContent>
      </Card>
    </div>
  )
}

function PendingOpenBox({ data }: { data: PendingOpenRecoveryResponse }) {
  if (data.file_missing) {
    return (
      <p className="text-sm text-muted-foreground">
        Brak pliku: <code className="text-xs break-all">{data.path}</code>
      </p>
    )
  }
  return (
    <div className="rounded-md border bg-muted/10 p-3 text-xs space-y-2">
      <div className="text-muted-foreground">
        path: <code className="text-xs break-all">{data.path}</code>
      </div>
      <pre className="whitespace-pre-wrap break-words rounded bg-muted/40 p-2 text-[11px] text-foreground/80">
        {JSON.stringify(data.data ?? {}, null, 2)}
      </pre>
    </div>
  )
}

