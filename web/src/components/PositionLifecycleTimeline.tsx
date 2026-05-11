import { useMemo, useState, useCallback } from 'react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { formatUsdFixed, shortenAddress } from '@/lib/utils'

const WSOL = 'So11111111111111111111111111111111111111112'

export type TimelineLedgerRow = Record<string, unknown>

type TimelinePoint = {
  id: string
  tsMs: number
  tsLabel: string
  event: string
  source: string
  txLamports: number
  lpUsdDelta: number
  sigShort: string
  /** Short PDA / old→new for merged chain + IL rows */
  context?: string
}

function parseTsMs(v: unknown): number | null {
  if (typeof v !== 'string' || !v.trim()) return null
  const ms = Date.parse(v)
  return Number.isFinite(ms) ? ms : null
}

function rowTimestampIso(r: TimelineLedgerRow): unknown {
  return r.ts_utc ?? r.timestamp
}

function rowEvent(r: TimelineLedgerRow): string {
  const e = r.event
  return typeof e === 'string' ? e : '—'
}

function rowSource(r: TimelineLedgerRow): string {
  const s = r.source
  return typeof s === 'string' ? s : '—'
}

function rowTsUtc(r: TimelineLedgerRow): string {
  const t = rowTimestampIso(r)
  return typeof t === 'string' ? t : '—'
}

function rowTxLamports(r: TimelineLedgerRow): number {
  const v = r.tx_fee_lamports ?? r.tx_cost_lamports
  if (typeof v === 'number' && Number.isFinite(v) && v > 0) return v
  if (typeof v === 'string') {
    const n = parseFloat(v)
    if (Number.isFinite(n) && n > 0) return n
  }
  return 0
}

function rowContextLine(r: TimelineLedgerRow): string | undefined {
  const pk = r.position_pubkey
  if (typeof pk === 'string' && pk.length > 8) return shortenAddress(pk, 4)
  const pos = r.position
  const old = r.old_position
  if (typeof old === 'string' && typeof pos === 'string' && old.length > 4 && pos.length > 4) {
    return `${shortenAddress(old, 3)}→${shortenAddress(pos, 3)}`
  }
  if (typeof pos === 'string' && pos.length > 4) return shortenAddress(pos, 4)
  return undefined
}

function isCollectLikeEvent(ev: string): boolean {
  const e = ev.toLowerCase()
  return (
    e.includes('collect_fee') ||
    e.includes('collect_fees') ||
    e === 'bot_collect_fees'
  )
}

function parseTokenDeltas(raw: unknown): Record<string, number> {
  if (!raw || typeof raw !== 'object') return {}
  const out: Record<string, number> = {}
  for (const [k, val] of Object.entries(raw as Record<string, unknown>)) {
    if (typeof val === 'number' && Number.isFinite(val)) {
      out[k] = val
    } else if (typeof val === 'string') {
      const n = parseFloat(val)
      if (Number.isFinite(n)) out[k] = n
    }
  }
  return out
}

function lpCollectedUsdDelta(
  r: TimelineLedgerRow,
  mintA: string | null,
  mintB: string | null,
  priceA: number,
  priceB: number,
  solUsd: number,
): number {
  const ev = rowEvent(r)
  if (!isCollectLikeEvent(ev)) return 0
  const deltas = parseTokenDeltas(r.fee_payer_token_deltas)
  let usd = 0
  for (const [mint, dUi] of Object.entries(deltas)) {
    if (dUi <= 0) continue
    let px = 0
    if (mintA && mint === mintA) px = priceA
    else if (mintB && mint === mintB) px = priceB
    else if (mint === WSOL) px = solUsd > 0 ? solUsd : 0
    if (px > 0) usd += dUi * px
  }
  return usd
}

function formatTsShort(iso: string): string {
  if (iso === '—') return iso
  const d = new Date(iso)
  if (!Number.isFinite(d.getTime())) return iso.slice(0, 19)
  return d.toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' })
}

export function PositionLifecycleTimeline(props: {
  rows: TimelineLedgerRow[]
  solUsd: number
  tokenMintA?: string | null
  tokenMintB?: string | null
  priceA?: number | null
  priceB?: number | null
  /** Unique PDAs merged into this timeline (stream-lineage chain). */
  chainPdaCount?: number
}) {
  const { rows, solUsd, tokenMintA, tokenMintB, priceA, priceB, chainPdaCount } = props

  const pa = priceA != null && Number.isFinite(priceA) ? priceA : 0
  const pb = priceB != null && Number.isFinite(priceB) ? priceB : 0
  const ma = tokenMintA?.trim() || null
  const mb = tokenMintB?.trim() || null

  const points = useMemo(() => {
    const list: TimelinePoint[] = []
    let i = 0
    for (const r of rows) {
      if (!r || typeof r !== 'object') continue
      const tsMs = parseTsMs(rowTimestampIso(r))
      if (tsMs === null) continue
      const ev = rowEvent(r)
      const src = rowSource(r)
      const txL = rowTxLamports(r)
      const lp = lpCollectedUsdDelta(r, ma, mb, pa, pb, solUsd)
      const sig = typeof r.signature === 'string' ? r.signature : ''
      list.push({
        id: `${tsMs}-${src}-${i++}`,
        tsMs,
        tsLabel: rowTsUtc(r),
        event: ev,
        source: src,
        txLamports: txL,
        lpUsdDelta: lp,
        sigShort: sig.length > 12 ? shortenAddress(sig, 4) : sig || '—',
        context: rowContextLine(r),
      })
    }
    list.sort((a, b) => a.tsMs - b.tsMs || a.id.localeCompare(b.id))
    return list
  }, [rows, ma, mb, pa, pb, solUsd])

  const [hoverIdx, setHoverIdx] = useState<number | null>(null)
  const [lockedIdx, setLockedIdx] = useState<number | null>(null)

  /** Marker + sums: locked point if set; otherwise follow hover; default = full history (last event). */
  const activeIdx = useMemo(() => {
    if (points.length === 0) return null
    if (lockedIdx !== null) return Math.max(0, Math.min(lockedIdx, points.length - 1))
    if (hoverIdx !== null) return Math.max(0, Math.min(hoverIdx, points.length - 1))
    return points.length - 1
  }, [points.length, hoverIdx, lockedIdx])

  const prefix = useMemo(() => {
    const txLam = points.map((p) => p.txLamports)
    const lpUsd = points.map((p) => p.lpUsdDelta)
    const preTx: number[] = []
    const preLp: number[] = []
    let s0 = 0
    let s1 = 0
    for (let i = 0; i < points.length; i++) {
      s0 += txLam[i] ?? 0
      s1 += lpUsd[i] ?? 0
      preTx.push(s0)
      preLp.push(s1)
    }
    return { preTx, preLp }
  }, [points])

  const tMin = points.length ? points[0].tsMs : 0
  const tMax = points.length ? points[points.length - 1].tsMs : 0
  const span = Math.max(1, tMax - tMin)

  const activeTotals = useMemo(() => {
    if (activeIdx === null || points.length === 0) {
      return { txLamports: 0, txUsd: '—' as string, lpUsd: 0 }
    }
    const lam = prefix.preTx[activeIdx] ?? 0
    const lp = prefix.preLp[activeIdx] ?? 0
    const usd =
      solUsd > 0 && lam > 0 ? formatUsdFixed((lam / 1e9) * solUsd, 4) : '—'
    return { txLamports: lam, txUsd: usd, lpUsd: lp }
  }, [activeIdx, points.length, prefix.preTx, prefix.preLp, solUsd])

  const pctForIdx = useCallback(
    (idx: number) => {
      if (!points.length) return 0
      const ts = points[idx].tsMs
      return ((ts - tMin) / span) * 100
    },
    [points, tMin, span],
  )

  const onDotClick = (idx: number) => {
    setLockedIdx((prev) => (prev === idx ? null : idx))
  }

  if (points.length === 0) {
    return (
      <Card>
        <CardHeader>
          <CardTitle className="text-base">Lifecycle timeline</CardTitle>
          <p className="text-xs text-muted-foreground font-normal">
            Brak wierszy z czasem (<code className="text-[10px]">ts_utc</code> /{' '}
            <code className="text-[10px]">timestamp</code>) — rozszerz limit zapytań lub sprawdź JSONL.
          </p>
        </CardHeader>
      </Card>
    )
  }

  const markerPct = activeIdx !== null ? pctForIdx(activeIdx) : 100

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">Lifecycle timeline</CardTitle>
        <p className="text-xs text-muted-foreground font-normal leading-snug">
          {chainPdaCount != null && chainPdaCount > 1 ? (
            <span className="block mb-1 text-foreground/90">
              Łańcuch rotacji: <strong>{chainPdaCount}</strong> PDA (lifecycle JSONL per PDA) + zdarzenia{' '}
              <code className="text-[10px]">il_ledger</code> (rebalance).
            </span>
          ) : null}
          Oś czasu — scalone zdarzenia chronologicznie. Najedź (podgląd) lub kliknij, żeby zablokować; sumy = wszystko{' '}
          <strong>do tego momentu włącznie</strong>. Koszty sieci: <code className="text-[10px]">tx_fee_lamports</code> /{' '}
          <code className="text-[10px]">tx_cost_lamports</code> (IL); LP: collect × ceny mintów (best-effort). Kropki:{' '}
          <span className="text-emerald-600 dark:text-emerald-400">zielony</span> = collect,{' '}
          <span className="text-rose-600 dark:text-rose-400">czerwony</span> = tx,{' '}
          <span className="text-violet-500 dark:text-violet-400">fiolet</span> = IL/rebalance,{' '}
          <span className="text-amber-600 dark:text-amber-400">bursztyn</span> = collect+tx.
        </p>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="rounded-md border border-border/70 bg-muted/15 px-3 py-3">
          <div className="text-xs text-muted-foreground mb-1">Skumulowanie do zaznaczenia</div>
          <div className="flex flex-wrap gap-x-6 gap-y-2 text-sm">
            <div>
              <span className="text-muted-foreground">Koszt sieci (tx)</span>{' '}
              <span className="font-mono tabular-nums block">
                {activeTotals.txLamports.toLocaleString()} λ
                {activeTotals.txUsd !== '—' ? (
                  <span className="block text-xs text-muted-foreground">~{activeTotals.txUsd}</span>
                ) : null}
              </span>
            </div>
            <div>
              <span className="text-muted-foreground">Fees zebrane (~USD)</span>{' '}
              <span className="font-mono tabular-nums block text-emerald-600 dark:text-emerald-400">
                {formatUsdFixed(activeTotals.lpUsd, 4)}
              </span>
            </div>
            {activeIdx !== null && points[activeIdx] ? (
              <div className="text-xs text-muted-foreground max-w-md">
                <span className="font-medium text-foreground">Zdarzenie: </span>
                <code className="text-[10px]">{points[activeIdx].event}</code> · {formatTsShort(points[activeIdx].tsLabel)}
                {points[activeIdx].context ? (
                  <span className="text-foreground/80"> · {points[activeIdx].context}</span>
                ) : null}{' '}
                · {points[activeIdx].sigShort}
              </div>
            ) : null}
          </div>
          {lockedIdx !== null ? (
            <p className="text-[11px] text-muted-foreground mt-2">
              Wybór zablokowany — linia i sumy nie podążają za kursorem. Kliknij ten sam punkt, aby odblokować.
            </p>
          ) : null}
        </div>

        <div className="relative select-none pt-2 pb-6 px-1">
          {/* axis */}
          <div className="relative h-12 rounded-sm bg-muted/30 border border-border/50">
            <div
              className="absolute top-0 bottom-0 w-px bg-primary/90 z-20 pointer-events-none"
              style={{ left: `calc(${markerPct}% - 0.5px)` }}
              aria-hidden
            />
            {points.map((p, idx) => {
              const leftPct = pctForIdx(idx)
              const isCost = p.txLamports > 0
              const isLp = p.lpUsdDelta > 0
              const isIl = p.event.startsWith('il:') || p.source === 'il_ledger'
              const isHover = hoverIdx === idx
              const isLocked = lockedIdx === idx
              const activeDot = isHover || isLocked
              let bg = 'bg-muted-foreground/50'
              if (isLp && isCost) bg = 'bg-amber-500'
              else if (isLp) bg = 'bg-emerald-500'
              else if (isCost && isIl) bg = 'bg-violet-600'
              else if (isCost) bg = 'bg-rose-500/80'

              return (
                <button
                  key={p.id}
                  type="button"
                  className={`absolute top-1/2 -translate-y-1/2 -translate-x-1/2 z-10 rounded-full border-2 border-background transition-all outline-none focus-visible:ring-2 focus-visible:ring-primary ${bg} ${
                    activeDot ? 'w-3.5 h-3.5 ring-2 ring-primary scale-110' : 'w-2.5 h-2.5 hover:scale-125'
                  }`}
                  style={{ left: `${leftPct}%` }}
                  title={`${p.event} @ ${p.tsLabel}`}
                  aria-label={`${p.event} at ${p.tsLabel}`}
                  onMouseEnter={() => setHoverIdx(idx)}
                  onMouseLeave={() => setHoverIdx(null)}
                  onClick={() => onDotClick(idx)}
                  onFocus={() => setHoverIdx(idx)}
                  onBlur={() => setHoverIdx(null)}
                />
              )
            })}
          </div>
          <div className="flex justify-between text-[10px] text-muted-foreground font-mono mt-1 px-0.5">
            <span>{formatTsShort(points[0].tsLabel)}</span>
            <span>{formatTsShort(points[points.length - 1].tsLabel)}</span>
          </div>
        </div>
      </CardContent>
    </Card>
  )
}
