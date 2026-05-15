/** Same event set as Rust `is_lifecycle_open_event` in `position_stream_lineage.rs`. */
const LIFECYCLE_OPEN_EVENTS = new Set([
  'bot_open_position',
  'bot_open_position_full_range',
  'position_open',
])

/** Matches Rust `open_quote_usd_from_open_details` plus legacy `open_quote_value_usd`. */
const OPEN_QUOTE_USD_KEYS = [
  'open_quote_estimated_value_usd',
  'open_target_usd',
  'open_prev_end_value_usd',
  'open_quote_value_usd',
] as const

function parseLedgerDetails(details: unknown): Record<string, unknown> | null {
  if (details && typeof details === 'object' && !Array.isArray(details)) {
    return details as Record<string, unknown>
  }
  if (typeof details === 'string') {
    try {
      const p = JSON.parse(details) as unknown
      if (p && typeof p === 'object' && !Array.isArray(p)) return p as Record<string, unknown>
    } catch {
      return null
    }
  }
  return null
}

function openQuoteUsdFromDetails(d: Record<string, unknown>): number | null {
  for (const key of OPEN_QUOTE_USD_KEYS) {
    const raw = d[key]
    if (raw == null) continue
    let n: number
    if (typeof raw === 'number') n = raw
    else if (typeof raw === 'string') n = parseFloat(raw.trim())
    else continue
    if (Number.isFinite(n) && n > 0) return n
  }
  return null
}

function rowPositionPubkey(r: Record<string, unknown>): string {
  const pk = r.position_pubkey
  if (typeof pk === 'string' && pk.trim()) return pk.trim()
  const p = r.position
  if (typeof p === 'string' && p.trim()) return p.trim()
  return ''
}

/** RFC3339 / epoch seconds / epoch ms — best-effort for JSONL rows. */
function parseRowTsMs(r: Record<string, unknown>): number | null {
  const tsRaw = r.ts_utc ?? r.timestamp
  if (tsRaw == null) return null
  if (typeof tsRaw === 'number' && Number.isFinite(tsRaw)) {
    return tsRaw < 1e12 ? Math.round(tsRaw * 1000) : Math.round(tsRaw)
  }
  if (typeof tsRaw === 'string') {
    const p = Date.parse(tsRaw)
    if (Number.isFinite(p)) return p
  }
  return null
}

type Best = { t: number | null; usd: number }

/** Prefer row with a parseable timestamp; among those, latest wins (Rust `merge_open_quote_usd_from_lifecycle_rows`). */
function shouldReplace(cur: Best | undefined, newTs: number | null, _newUsd: number): boolean {
  if (!cur) return true
  if (cur.t == null && newTs != null) return true
  if (cur.t != null && newTs == null) return false
  if (cur.t == null && newTs == null) return true
  return newTs! >= cur.t!
}

/**
 * Per-position PDA: open-quote USD from merged lifecycle ledger rows when API lineage baseline is empty.
 * Aligns with backend: exact open events, `open_quote_usd_from_open_details` key order, JSON-string `details`,
 * **latest** matching open row by timestamp (same as `merge_open_quote_usd_from_lifecycle_rows`).
 */
export function extractLifecycleOpenQuoteUsdByPosition(
  rows: ReadonlyArray<Record<string, unknown>>,
): Map<string, number> {
  const best = new Map<string, Best>()
  for (const r of rows) {
    if (!r || typeof r !== 'object') continue
    const pk = rowPositionPubkey(r)
    if (!pk) continue
    const ev = typeof r.event === 'string' ? r.event.trim() : ''
    if (!LIFECYCLE_OPEN_EVENTS.has(ev)) continue
    const d = parseLedgerDetails(r.details)
    if (!d) continue
    const usd = openQuoteUsdFromDetails(d)
    if (usd == null) continue
    const ts = parseRowTsMs(r)
    const cur = best.get(pk)
    if (shouldReplace(cur, ts, usd)) best.set(pk, { t: ts, usd })
  }
  return new Map([...best.entries()].map(([k, v]) => [k, v.usd]))
}

/** @deprecated Prefer `extractLifecycleOpenQuoteUsdByPosition` (name reflected “earliest” but logic matches backend: latest open row). */
export const extractEarliestOpenQuoteUsdByPosition = extractLifecycleOpenQuoteUsdByPosition
