import { useMemo, useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { ExternalLink, ScrollText, RefreshCw } from 'lucide-react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import {
  getBotIlLedger,
  getBotLedger,
  getBotRegistry,
  getPendingOpenRecovery,
  getStrandedRebalances,
  reconcileStrandedRebalances,
  type BotActivityJsonlResponse,
  type BotRegistryJsonlResponse,
  type PendingOpenRecoveryResponse,
  type StrandedRebalancesResponse,
} from '@/lib/api'

function rowCell(v: unknown): string {
  if (v === null || v === undefined) return '—'
  if (typeof v === 'string' || typeof v === 'number' || typeof v === 'boolean') return String(v)
  return JSON.stringify(v)
}

function JsonlTable({
  data,
  columnKeys,
  getCellValue,
}: {
  data: BotActivityJsonlResponse | BotRegistryJsonlResponse
  columnKeys: string[]
  /** Optional: map column key → value (np. `position_pubkey` z fallbackiem do `position_pda`). */
  getCellValue?: (row: Record<string, unknown>, key: string) => unknown
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
              {columnKeys.map((k) => {
                const raw = getCellValue
                  ? getCellValue(row as Record<string, unknown>, k)
                  : (row as Record<string, unknown>)[k]
                const shown = rowCell(raw)
                return (
                  <td
                    key={k}
                    className="px-3 py-2 max-w-[18rem] truncate font-mono text-xs"
                    title={shown}
                  >
                    {shown}
                  </td>
                )
              })}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

// `details`: confirmed `bot_swap_exact_in` swap params (mints, amount_in, slippage); diagnostic rows use the same key.
// Ledger JSONL używa `position_pubkey` (orca_bot); starsze/CLI mogą mieć `position_pda` — łączymy w getLifecycleCell.
const LIFECYCLE_KEYS = [
  'ts_utc',
  'source',
  'event',
  'signature',
  'position_pubkey',
  'pool_address',
  'rebalance_session_id',
  'tx_fee_lamports',
  'details',
]

function getLifecycleCell(row: Record<string, unknown>, key: string): unknown {
  if (key === 'position_pubkey') {
    return row.position_pubkey ?? row.position_pda ?? null
  }
  return row[key]
}

type LedgerRow = Record<string, unknown>

function isLikelySolanaPubkeyOrSig(s: string): boolean {
  const t = s.trim()
  return t.length >= 32 && t.length <= 128 && /^[1-9A-HJ-NP-Za-km-z]+$/.test(t)
}

function solscanAccountUrl(addr: string): string {
  return `https://solscan.io/account/${encodeURIComponent(addr.trim())}`
}

function solscanTxUrl(sig: string): string {
  return `https://solscan.io/tx/${encodeURIComponent(sig.trim())}`
}

function shortText(s: string, head = 6, tail = 4): string {
  if (s.length <= head + tail + 3) return s
  return `${s.slice(0, head)}…${s.slice(-tail)}`
}

function ledgerSessionId(row: LedgerRow): string {
  const sidRaw = row.rebalance_session_id
  if (typeof sidRaw === 'string' && sidRaw.trim()) return sidRaw.trim()
  return ''
}

function ledgerPositionKey(row: LedgerRow): string {
  const p = row.position_pubkey ?? row.position_pda
  return typeof p === 'string' ? p.trim() : ''
}

function ledgerEventSummary(rows: LedgerRow[]): string {
  const counts = new Map<string, number>()
  for (const r of rows) {
    const e = typeof r.event === 'string' && r.event.trim() ? r.event.trim() : '—'
    counts.set(e, (counts.get(e) ?? 0) + 1)
  }
  return [...counts.entries()]
    .sort((a, b) => a[0].localeCompare(b[0]))
    .map(([ev, n]) => `${ev}×${n}`)
    .join(' · ')
}

function groupLedgerSessions(rows: LedgerRow[]): { sessionId: string; rows: LedgerRow[] }[] {
  const map = new Map<string, LedgerRow[]>()
  for (const r of rows) {
    const sid = ledgerSessionId(r) || '(brak rebalance_session_id)'
    const arr = map.get(sid) ?? []
    arr.push(r)
    map.set(sid, arr)
  }
  for (const arr of map.values()) {
    arr.sort((a, b) => String(b.ts_utc ?? '').localeCompare(String(a.ts_utc ?? '')))
  }
  const groups = [...map.entries()].map(([sessionId, gr]) => ({ sessionId, rows: gr }))
  groups.sort((a, b) => {
    const ta = String(a.rows[0]?.ts_utc ?? '')
    const tb = String(b.rows[0]?.ts_utc ?? '')
    return tb.localeCompare(ta)
  })
  return groups
}

function LinkSolscan({
  href,
  label,
  title,
}: {
  href: string
  label: string
  title?: string
}) {
  return (
    <a
      href={href}
      target="_blank"
      rel="noopener noreferrer"
      className="inline-flex max-w-[14rem] items-center gap-1 truncate text-primary hover:underline"
      title={title}
    >
      <span className="truncate font-mono text-[11px]">{label}</span>
      <ExternalLink className="h-3 w-3 shrink-0 opacity-70" aria-hidden />
    </a>
  )
}

function DetailsJson({ value }: { value: unknown }) {
  if (value === null || value === undefined || value === '') {
    return <span className="text-muted-foreground">—</span>
  }
  let text: string
  try {
    text = typeof value === 'string' ? value : JSON.stringify(value, null, 2)
  } catch {
    text = String(value)
  }
  if (text.length <= 120 && !text.includes('\n')) {
    return <span className="break-all font-mono text-[11px]">{text}</span>
  }
  return (
    <details className="max-w-[20rem]">
      <summary className="cursor-pointer select-none text-[11px] text-muted-foreground hover:text-foreground">
        Pokaż szczegóły (JSON)
      </summary>
      <pre className="mt-1 max-h-48 overflow-auto whitespace-pre-wrap break-words rounded border bg-muted/30 p-2 text-[10px] leading-snug">
        {text}
      </pre>
    </details>
  )
}

type TickRange = { lower: number; upper: number }

function asObj(v: unknown): Record<string, unknown> | null {
  if (!v || typeof v !== 'object' || Array.isArray(v)) return null
  return v as Record<string, unknown>
}

function numField(obj: Record<string, unknown> | null, key: string): number | null {
  if (!obj) return null
  const v = obj[key]
  if (typeof v === 'number' && Number.isFinite(v)) return v
  if (typeof v === 'string') {
    const n = Number(v)
    if (Number.isFinite(n)) return n
  }
  return null
}

function rangeFrom(obj: Record<string, unknown> | null, lowKey: string, upKey: string): TickRange | null {
  const lo = numField(obj, lowKey)
  const hi = numField(obj, upKey)
  if (lo == null || hi == null) return null
  return { lower: lo, upper: hi }
}

function TickRangeGraphic({
  current,
  previous,
  planned,
}: {
  current?: TickRange | null
  previous?: TickRange | null
  planned?: TickRange | null
}) {
  const ranges = [previous, current, planned].filter((x): x is TickRange => !!x)
  if (!ranges.length) return null
  let min = Number.POSITIVE_INFINITY
  let max = Number.NEGATIVE_INFINITY
  for (const r of ranges) {
    min = Math.min(min, r.lower, r.upper)
    max = Math.max(max, r.lower, r.upper)
  }
  const span = Math.max(1, max - min)
  const pos = (v: number) => ((v - min) / span) * 100
  const bar = (r: TickRange, cls: string, top: number) => (
    <div
      className={`absolute h-2 rounded ${cls}`}
      style={{ left: `${pos(r.lower)}%`, width: `${Math.max(1, pos(r.upper) - pos(r.lower))}%`, top }}
    />
  )
  return (
    <div className="rounded border bg-muted/20 px-2 py-1">
      <div className="relative h-7">
        {previous ? bar(previous, 'bg-amber-400/80', 2) : null}
        {current ? bar(current, 'bg-emerald-400/80', 10) : null}
        {planned ? bar(planned, 'bg-sky-400/80', 18) : null}
      </div>
      <div className="mt-1 flex flex-wrap gap-x-3 gap-y-0.5 text-[10px] text-muted-foreground">
        {previous ? <span>prev: [{previous.lower}, {previous.upper}]</span> : null}
        {current ? <span>current: [{current.lower}, {current.upper}]</span> : null}
        {planned ? <span>planned: [{planned.lower}, {planned.upper}]</span> : null}
      </div>
    </div>
  )
}

function EventRangePanel({ row }: { row: LedgerRow }) {
  const ev = typeof row.event === 'string' ? row.event : ''
  const d = asObj(row.details)
  if (ev === 'bot_close_position') {
    const current = rangeFrom(d, 'old_tick_lower', 'old_tick_upper')
    const planned = rangeFrom(d, 'planned_new_tick_lower', 'planned_new_tick_upper')
    if (!current && !planned) return null
    return (
      <div className="space-y-1">
        <div className="text-[10px] text-muted-foreground">Zakres pozycji przy zamknięciu (i planowany nowy)</div>
        <TickRangeGraphic current={current} planned={planned} />
      </div>
    )
  }
  if (ev === 'bot_open_position' || ev === 'bot_open_position_full_range') {
    const current = rangeFrom(d, 'tick_lower', 'tick_upper') ?? rangeFrom(d, 'new_tick_lower', 'new_tick_upper')
    const previous = rangeFrom(d, 'prev_tick_lower', 'prev_tick_upper')
    if (!current && !previous) return null
    return (
      <div className="space-y-1">
        <div className="text-[10px] text-muted-foreground">Nowa pozycja vs poprzedni zakres</div>
        <TickRangeGraphic current={current} previous={previous} />
      </div>
    )
  }
  return null
}

function LifecycleSessionsView({
  rows,
  onFilterSession,
}: {
  rows: LedgerRow[]
  onFilterSession: (sid: string) => void
}) {
  if (!rows.length) {
    return <p className="text-sm text-muted-foreground">Brak wierszy (albo filtr nic nie zwrócił).</p>
  }
  const groups = groupLedgerSessions(rows)
  return (
    <div className="space-y-3">
      {groups.map((g, gi) => {
        const pos = ledgerPositionKey(g.rows[0] ?? {})
        const pool =
          typeof g.rows[0]?.pool_address === 'string' ? (g.rows[0].pool_address as string).trim() : ''
        const summary = ledgerEventSummary(g.rows)
        const filterable = g.sessionId !== '(brak rebalance_session_id)'
        return (
          <div key={`${g.sessionId}-${gi}`} className="rounded-md border border-border/60 bg-muted/5">
            <div className="flex flex-col gap-2 border-b border-border/50 px-3 py-2 sm:flex-row sm:items-center sm:justify-between">
              <div className="min-w-0 space-y-1">
                <div className="flex flex-wrap items-center gap-2 text-xs">
                  <span className="font-medium text-foreground">Sesja</span>
                  <code className="truncate text-[11px]" title={g.sessionId}>
                    {filterable ? shortText(g.sessionId, 10, 8) : g.sessionId}
                  </code>
                  {filterable ? (
                    <Button size="sm" variant="outline" className="h-7 text-[11px]" onClick={() => onFilterSession(g.sessionId)}>
                      Filtruj
                    </Button>
                  ) : null}
                </div>
                <div className="text-[11px] text-muted-foreground">
                  {g.rows.length} zdarzeń · {summary}
                </div>
                <div className="flex flex-wrap gap-x-3 gap-y-1 text-[11px] text-muted-foreground">
                  {pos && isLikelySolanaPubkeyOrSig(pos) ? (
                    <span>
                      pozycja:{' '}
                      <LinkSolscan href={solscanAccountUrl(pos)} label={shortText(pos, 4, 4)} title={pos} />
                    </span>
                  ) : null}
                  {pool && isLikelySolanaPubkeyOrSig(pool) ? (
                    <span>
                      pool:{' '}
                      <LinkSolscan href={solscanAccountUrl(pool)} label={shortText(pool, 4, 4)} title={pool} />
                    </span>
                  ) : null}
                </div>
              </div>
            </div>
            <ul className="divide-y divide-border/40">
              {g.rows.map((r, ri) => {
                const ts = typeof r.ts_utc === 'string' ? r.ts_utc : '—'
                const ev = typeof r.event === 'string' ? r.event : '—'
                const src = typeof r.source === 'string' ? r.source : '—'
                const sigRaw = r.signature
                const sig = typeof sigRaw === 'string' ? sigRaw.trim() : ''
                return (
                  <li key={`${g.sessionId}-${gi}-${ri}`} className="px-3 py-2 text-xs">
                    <div className="flex flex-col gap-1 sm:flex-row sm:items-start sm:justify-between">
                      <div className="min-w-0 space-y-1">
                        <div className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
                          <span className="whitespace-nowrap font-mono text-[10px] text-muted-foreground">{ts}</span>
                          <code className="rounded bg-muted/50 px-1.5 py-0.5 text-[11px]">{ev}</code>
                          <span className="text-[10px] text-muted-foreground">{src}</span>
                        </div>
                        <div className="flex flex-wrap items-center gap-2">
                          {sig && isLikelySolanaPubkeyOrSig(sig) ? (
                            <LinkSolscan href={solscanTxUrl(sig)} label={`tx ${shortText(sig, 4, 4)}`} title={sig} />
                          ) : (
                            <span className="text-muted-foreground">—</span>
                          )}
                        </div>
                        <EventRangePanel row={r} />
                      </div>
                      <div className="shrink-0 pt-0.5 sm:max-w-[22rem]">
                        <DetailsJson value={r.details} />
                      </div>
                    </div>
                  </li>
                )
              })}
            </ul>
          </div>
        )
      })}
    </div>
  )
}

function LifecycleRawTable({ rows }: { rows: LedgerRow[] }) {
  if (!rows.length) {
    return <p className="text-sm text-muted-foreground">Brak wierszy (albo filtr nic nie zwrócił).</p>
  }
  return (
    <div className="overflow-x-auto rounded-md border">
      <table className="w-full text-left text-sm">
        <thead className="bg-muted/50">
          <tr>
            {LIFECYCLE_KEYS.map((k) => (
              <th key={k} className="px-3 py-2 font-medium whitespace-nowrap">
                {k}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row, i) => (
            <tr key={i} className="border-t border-border/60 hover:bg-muted/30">
              {LIFECYCLE_KEYS.map((k) => {
                const raw = getLifecycleCell(row as Record<string, unknown>, k)
                if (k === 'signature') {
                  const sig = typeof raw === 'string' ? raw.trim() : ''
                  return (
                    <td key={k} className="px-3 py-2 align-top">
                      {sig && isLikelySolanaPubkeyOrSig(sig) ? (
                        <LinkSolscan href={solscanTxUrl(sig)} label={shortText(sig, 6, 6)} title={sig} />
                      ) : (
                        <span className="font-mono text-[11px] text-muted-foreground">—</span>
                      )}
                    </td>
                  )
                }
                if (k === 'position_pubkey') {
                  const pk = typeof raw === 'string' ? raw.trim() : ''
                  return (
                    <td key={k} className="max-w-[12rem] px-3 py-2 align-top">
                      {pk && isLikelySolanaPubkeyOrSig(pk) ? (
                        <LinkSolscan href={solscanAccountUrl(pk)} label={shortText(pk, 4, 4)} title={pk} />
                      ) : (
                        <span className="truncate font-mono text-[11px]" title={rowCell(raw)}>
                          {rowCell(raw)}
                        </span>
                      )}
                    </td>
                  )
                }
                if (k === 'pool_address') {
                  const pk = typeof raw === 'string' ? raw.trim() : ''
                  return (
                    <td key={k} className="max-w-[12rem] px-3 py-2 align-top">
                      {pk && isLikelySolanaPubkeyOrSig(pk) ? (
                        <LinkSolscan href={solscanAccountUrl(pk)} label={shortText(pk, 4, 4)} title={pk} />
                      ) : (
                        <span className="truncate font-mono text-[11px]" title={rowCell(raw)}>
                          {rowCell(raw)}
                        </span>
                      )}
                    </td>
                  )
                }
                if (k === 'details') {
                  return (
                    <td key={k} className="max-w-[18rem] px-3 py-2 align-top">
                      <DetailsJson value={raw} />
                    </td>
                  )
                }
                const shown = rowCell(raw)
                return (
                  <td key={k} className="px-3 py-2 align-top font-mono text-[11px]" title={shown}>
                    <span className="line-clamp-2 break-all">{shown}</span>
                  </td>
                )
              })}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

const IL_KEYS = ['timestamp', 'event', 'old_position', 'position', 'pool', 'reason', 'tx_cost_lamports', 'hint']

const REGISTRY_KEYS = ['ts_utc', 'event', 'position', 'pool', 'owner', 'signature']

export default function Logs() {
  const qc = useQueryClient()
  const [filter, setFilter] = useState('')
  const [limit, setLimit] = useState(300)
  const [ledgerOffset, setLedgerOffset] = useState(0)
  const [reconcileMsg, setReconcileMsg] = useState<string | null>(null)
  const [reconcileErr, setReconcileErr] = useState<string | null>(null)
  const [reconciling, setReconciling] = useState(false)
  const [lifecycleView, setLifecycleView] = useState<'sessions' | 'table'>('sessions')

  const ledgerQ = useQuery({
    queryKey: ['logs-ledger', limit, filter, ledgerOffset],
    queryFn: () => getBotLedger(limit, filter || undefined, ledgerOffset),
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
  const strandedQ = useQuery({
    queryKey: ['logs-stranded-rebalances'],
    queryFn: getStrandedRebalances,
    staleTime: 5_000,
  })

  const lastIncomplete = useMemo(() => {
    const rows = (ilQ.data?.rows ?? []) as Record<string, unknown>[]
    for (const r of rows) {
      if (r?.event === 'rebalance_incomplete') return r
    }
    return null
  }, [ilQ.data?.rows])

  /** W obrębie strony: najnowsze wiersze na górze (API zwraca fragment w kolejności pliku). */
  const ledgerDisplayData = useMemo((): BotActivityJsonlResponse | null => {
    if (!ledgerQ.data) return null
    return {
      ...ledgerQ.data,
      rows: [...ledgerQ.data.rows].reverse(),
    }
  }, [ledgerQ.data])

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
            void qc.invalidateQueries({ queryKey: ['logs-stranded-rebalances'] })
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
              onChange={(e) => {
                setLedgerOffset(0)
                setFilter(e.target.value)
              }}
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
              onChange={(e) => {
                setLedgerOffset(0)
                setLimit(Number(e.target.value) || 300)
              }}
            />
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Urwane pozycje (watchdog)</CardTitle>
          <p className="text-sm text-muted-foreground">
            Sesje z <code className="text-xs">bot_close_position</code> bez odpowiadającego{' '}
            <code className="text-xs">bot_open_position</code>. Ta sekcja ma osobny widok do recovery. Okresowy reconcile w{' '}
            API: ustaw <code className="text-xs">CLMM_STRANDED_RECONCILE_INTERVAL_SECS</code> (np.{' '}
            <code className="text-xs">300</code>) — wymaga <code className="text-xs">CLMM_IL_LEDGER_PATH</code>.
          </p>
        </CardHeader>
        <CardContent className="space-y-3 text-sm">
          <div className="flex flex-wrap items-center gap-2">
            <Button
              size="sm"
              variant="secondary"
              disabled={reconciling}
              onClick={async () => {
                setReconcileMsg(null)
                setReconcileErr(null)
                setReconciling(true)
                try {
                  const res = await reconcileStrandedRebalances()
                  setReconcileMsg(`Auto-enqueued: ${res.auto_enqueued}`)
                  await qc.invalidateQueries({ queryKey: ['logs-stranded-rebalances'] })
                  await qc.invalidateQueries({ queryKey: ['logs-pending-open'] })
                } catch (e) {
                  setReconcileErr((e as Error).message)
                } finally {
                  setReconciling(false)
                }
              }}
            >
              {reconciling ? 'Reconciling…' : 'Run watchdog reconcile'}
            </Button>
            {reconcileMsg && <span className="text-xs text-emerald-700 dark:text-emerald-400">{reconcileMsg}</span>}
            {reconcileErr && <span className="text-xs text-destructive">{reconcileErr}</span>}
          </div>
          {strandedQ.isLoading && <p className="text-muted-foreground">Ładowanie…</p>}
          {strandedQ.isError && <p className="text-destructive">{(strandedQ.error as Error).message}</p>}
          {strandedQ.data && <StrandedRebalancesBox data={strandedQ.data} onFilterSession={(sid) => {
            setLedgerOffset(0)
            setFilter(sid)
          }} />}
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
            {ledgerQ.data?.rows_returned ?? '—'}. To jest <span className="font-medium">dziennik wykonania</span>{' '}
            (otwarcie / zamknięcie / fee), nie analityka strategii — sens mają powiązania{' '}
            <code className="text-[11px]">rebalance_session_id</code> i linki do Solscan. Zamknięcie:{' '}
            <code className="text-xs">bot_close_position</code>.
          </p>
        </CardHeader>
        <CardContent>
          <div className="mb-3 flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
            <div className="text-xs text-muted-foreground">
              offset: <code className="text-[11px]">{ledgerOffset}</code>
              {typeof ledgerQ.data?.total_matching_lines === 'number' ? (
                <>
                  {' '}
                  • strona{' '}
                  <code className="text-[11px]">
                    {Math.floor(ledgerOffset / Math.max(1, limit)) + 1}
                  </code>
                </>
              ) : null}
            </div>
            <div className="flex flex-wrap gap-2">
              <Button
                type="button"
                size="sm"
                variant="outline"
                disabled={ledgerOffset <= 0}
                onClick={() => setLedgerOffset((x) => Math.max(0, x - Math.max(1, limit)))}
              >
                Nowsze
              </Button>
              <Button
                type="button"
                size="sm"
                variant="outline"
                disabled={
                  !ledgerQ.data ||
                  ledgerQ.data.file_missing ||
                  ledgerQ.data.rows_returned === 0 ||
                  ledgerOffset + ledgerQ.data.rows_returned >= ledgerQ.data.total_matching_lines
                }
                onClick={() => setLedgerOffset((x) => x + Math.max(1, limit))}
              >
                Starsze
              </Button>
              <Button
                type="button"
                size="sm"
                variant="secondary"
                disabled={ledgerOffset === 0}
                onClick={() => setLedgerOffset(0)}
              >
                Najnowsze
              </Button>
            </div>
          </div>
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
                    onClick={() => {
                      setLedgerOffset(0)
                      setFilter(x.sid)
                    }}
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
          {ledgerDisplayData && (
            <div className="space-y-3">
              <div className="flex flex-wrap gap-2">
                <Button
                  type="button"
                  size="sm"
                  variant={lifecycleView === 'sessions' ? 'default' : 'outline'}
                  onClick={() => setLifecycleView('sessions')}
                >
                  Widok sesji (czytelny)
                </Button>
                <Button
                  type="button"
                  size="sm"
                  variant={lifecycleView === 'table' ? 'default' : 'outline'}
                  onClick={() => setLifecycleView('table')}
                >
                  Tabela surowa
                </Button>
              </div>
              <p className="text-xs text-muted-foreground">
                Widok sesji grupuje wiersze po <code className="text-[11px]">rebalance_session_id</code> (tylko bieżąca
                strona / limit). Linki otwierają Solscan (mainnet).
              </p>
              {lifecycleView === 'sessions' ? (
                <LifecycleSessionsView
                  rows={ledgerDisplayData.rows as LedgerRow[]}
                  onFilterSession={(sid) => {
                    setLedgerOffset(0)
                    setFilter(sid)
                  }}
                />
              ) : (
                <LifecycleRawTable rows={ledgerDisplayData.rows as LedgerRow[]} />
              )}
            </div>
          )}
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

function StrandedRebalancesBox({
  data,
  onFilterSession,
}: {
  data: StrandedRebalancesResponse
  onFilterSession: (sid: string) => void
}) {
  if (!data.items?.length) {
    return (
      <div className="rounded-md border bg-muted/10 p-3 text-xs text-muted-foreground space-y-1">
        <div>Brak urwanych sesji w ostatnim skanie.</div>
        <div>
          scanned: <code className="text-[11px]">{data.rows_scanned}</code>
        </div>
      </div>
    )
  }
  return (
    <div className="space-y-2">
      <div className="text-xs text-muted-foreground">
        scanned: <code className="text-[11px]">{data.rows_scanned}</code> • found:{' '}
        <code className="text-[11px]">{data.items.length}</code>
      </div>
      <div className="space-y-2">
        {data.items.slice(0, 20).map((it) => (
          <div key={it.rebalance_session_id} className="rounded-md border px-3 py-2 text-xs space-y-1">
            <div className="flex flex-wrap items-center gap-2">
              <code className="text-[11px]">{it.rebalance_session_id}</code>
              <Button size="sm" variant="outline" onClick={() => onFilterSession(it.rebalance_session_id)}>
                Filtruj w lifecycle
              </Button>
            </div>
            <div>
              old: <code className="text-[11px]">{it.old_position ?? '—'}</code> • pool:{' '}
              <code className="text-[11px]">{it.pool_address ?? '—'}</code>
            </div>
            <div>
              pending_queue: {it.in_pending_open_queue ? 'yes' : 'no'} • IL row:{' '}
              {it.rebalance_incomplete_logged ? 'yes' : 'no'} • auto-enqueue:{' '}
              {it.can_auto_enqueue ? 'yes' : 'no'}
            </div>
            <div>
              intended ticks: {it.intended_tick_lower ?? '—'} / {it.intended_tick_upper ?? '—'} • reason:{' '}
              {it.reason ?? '—'}
            </div>
            {it.note ? <div className="text-muted-foreground">{it.note}</div> : null}
          </div>
        ))}
      </div>
    </div>
  )
}

