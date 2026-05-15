import { Link } from 'react-router-dom'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import type { PositionStreamLineageResponse } from '@/lib/api'
import {
  FEE_BASE_UNITS_TOOLTIP,
  formatDate,
  formatNumber,
  formatFeeBaseUnitsClause,
  formatInvertedTokenPriceRange,
  formatLineageFeesCollectedUsdMain,
  formatPercentFixed,
  formatPrincipalDeltaForLineageNode,
  formatLineageStoredValueUsd,
  formatLineageOpeningUsdDisplay,
  formatUsdField,
  formatUsdFixed,
  formatTokenPriceRange,
  isLineageStoredUsdMissing,
  shortenAddress,
} from '@/lib/utils'
import { tickToPriceRatio, uiPriceFromRawPriceRatio } from '@/lib/whirlpoolTicks'

type TickRange = {
  lower: number
  upper: number
}

type PositionOpenCloseRanges = {
  open?: TickRange
}

function localizeLineageNote(note: string, locale: 'pl' | 'en'): string {
  if (!note?.trim()) return note
  let out = note
  if (locale === 'pl') {
    out = out.replace(
      'Best-effort. LP mark vs HODL uses first→last position in rotation lineage; IL/HODL: baseline basket (open amounts at chain start) × current mint USD prices when pool mints are known (stable+dexpaprika). tx fees in USD use SOL/USD (dexpaprika). realized_cashflow uses lifecycle fee_payer_token_deltas × mint USD prices (stable+dexpaprika). cost/cashflow scope=chain positions fallback.',
      'Best-effort. LP mark vs HODL używa pierwszej→ostatniej pozycji w rotacyjnym lineage; IL/HODL: koszyk bazowy (kwoty open na starcie łańcucha) × bieżące ceny USD mintów, gdy minty puli są znane (stable+dexpaprika). Opłaty tx w USD używają SOL/USD (dexpaprika). realized_cashflow używa lifecycle fee_payer_token_deltas × ceny USD mintów (stable+dexpaprika). Zakres koszt/cashflow = fallback pozycji łańcucha.',
    )
    out = out.replace(
      'Lineage chain is best-effort and assumes a mostly linear old→new rotation path (common for strategies). If edges are missing, the chain may be incomplete. Cross-PDA stream stitching was suppressed for this mint (operator open: CLI `position_open` / `source:cli` / API `open_origin=operator_api`, unanchored bot open, or non-rotation lifecycle); the history table lists this position only.',
      'Łańcuch lineage jest best-effort i zakłada głównie liniową rotację old→new (typową dla strategii). Jeśli brakuje krawędzi, łańcuch może być niepełny. Stitching streamu cross-PDA został wyłączony dla tego minta (operator open: CLI `position_open` / `source:cli` / API `open_origin=operator_api`, niezakotwiczony bot open lub lifecycle bez rotacji); tabela historii pokazuje tylko tę pozycję.',
    )
  }
  return out
}

function formatClosePriceAtEvent(
  price: number | undefined,
  tokenALabel?: string | null,
  tokenBLabel?: string | null,
): string {
  if (typeof price !== 'number' || !Number.isFinite(price) || price <= 0) return '—'
  const base = tokenALabel?.trim() || 'token A'
  const quote = tokenBLabel?.trim() || 'USD'
  return `${formatNumber(price, 6)} ${quote} per 1 ${base}`
}

function parsePositiveUsdSpot(s: string | null | undefined): number | undefined {
  if (s == null) return undefined
  const n = parseFloat(String(s).trim())
  return Number.isFinite(n) && n > 0 ? n : undefined
}

function formatRangeFromTicks(
  range: TickRange | undefined,
  tokenALabel?: string | null,
  tokenBLabel?: string | null,
  decimalsA?: number | null,
  decimalsB?: number | null,
  invertQuote = false,
): string {
  if (!range) return '—'
  const quote =
    tokenALabel && tokenBLabel ? `${tokenBLabel} per 1 ${tokenALabel}` : 'token B per 1 token A'
  const invQuote =
    tokenALabel && tokenBLabel ? `${tokenALabel} per 1 ${tokenBLabel}` : 'token A per 1 token B'
  const lowerRaw = tickToPriceRatio(range.lower)
  const upperRaw = tickToPriceRatio(range.upper)
  const lower =
    decimalsA != null && decimalsB != null
      ? uiPriceFromRawPriceRatio(lowerRaw, decimalsA, decimalsB)
      : null
  const upper =
    decimalsA != null && decimalsB != null
      ? uiPriceFromRawPriceRatio(upperRaw, decimalsA, decimalsB)
      : null
  if (lower == null || upper == null) return `${range.lower} -> ${range.upper} ticks`
  if (invertQuote) {
    return (
      formatInvertedTokenPriceRange(lower, upper, invQuote) ??
      `${range.lower} -> ${range.upper} ticks`
    )
  }
  return formatTokenPriceRange(lower, upper, quote) ?? `${range.lower} -> ${range.upper} ticks`
}

function rangeAdjustmentBadge(reason: string | null): { text: string; className: string } {
  if (!reason) {
    return {
      text: 'as planned',
      className: 'border-emerald-600/40 bg-emerald-500/10 text-emerald-300',
    }
  }
  return {
    text: reason.startsWith('recover_plan_') ? 'replanned' : 'adapted',
    className: 'border-amber-600/40 bg-amber-500/10 text-amber-300',
  }
}

function buildTotalsSourceBadge(
  lineage: PositionStreamLineageResponse | undefined,
  isSettlementMode: boolean,
): { label: string; className: string } {
  const note = (lineage?.totals?.note ?? '').toLowerCase()
  if (isSettlementMode || note.includes('settlement v1') || note.includes('self-seed disabled')) {
    return {
      label: 'source: persisted settlement',
      className: 'border-emerald-600/40 bg-emerald-500/10 text-emerald-300',
    }
  }
  if (note.includes('self-seed')) {
    return {
      label: 'source: live seeded',
      className: 'border-amber-600/40 bg-amber-500/10 text-amber-300',
    }
  }
  return {
    label: 'source: live snapshots',
    className: 'border-border/70 bg-background/70 text-muted-foreground',
  }
}

export type PositionLineageHistoryPanelProps = {
  lineage: PositionStreamLineageResponse
  /** Stream = compute-on-read API; Postgres = GET …/chain-history materialized rows. */
  badgeMode: 'stream' | 'postgres'
  cardTitle: string
  apiIntro: string
  chainReconstructHelp: string
  locale: 'pl' | 'en'
  isSettlementMode: boolean
  nodeOpenCloseRanges: Map<string, PositionOpenCloseRanges>
  /** USD spot token A from lifecycle **close** row (`details.event_price_a_usd`), latest per PDA. */
  closePriceByPosition: Map<string, number>
  /** USD spot token A from lifecycle **open** row (`details.event_price_a_usd`), earliest per PDA. */
  openEventPriceByPosition: Map<string, number>
  rangeAdjustmentReasonByPosition: Map<string, string | null>
  invertRangeQuote: boolean
  onInvertRangeQuote: (next: boolean) => void
  showOnlyNonZeroBreakdown: boolean
  onToggleShowOnlyNonZeroBreakdown: () => void
  tokenDecimalsA: number | null
  tokenDecimalsB: number | null
  readBadgePostgres: string
  readBadgeStream: string
  /** Open-quote USD per PDA from merged lifecycle ledger (fallback when API baseline missing). */
  ledgerOpenQuoteUsdByPosition?: ReadonlyMap<string, number>
}

export function PositionLineageHistoryPanel({
  lineage,
  badgeMode,
  cardTitle,
  apiIntro,
  chainReconstructHelp,
  locale,
  isSettlementMode,
  nodeOpenCloseRanges,
  closePriceByPosition,
  openEventPriceByPosition,
  rangeAdjustmentReasonByPosition,
  invertRangeQuote,
  onInvertRangeQuote,
  showOnlyNonZeroBreakdown,
  onToggleShowOnlyNonZeroBreakdown,
  tokenDecimalsA,
  tokenDecimalsB,
  readBadgePostgres,
  readBadgeStream,
  ledgerOpenQuoteUsdByPosition,
}: PositionLineageHistoryPanelProps) {
  const totalsSourceBadge = buildTotalsSourceBadge(lineage, isSettlementMode)
  const fromPostgres = badgeMode === 'postgres'
  const singleNodeOpeningExtra =
    lineage.nodes.length === 1
      ? { singleNodeTotalsBaselineUsd: lineage.totals?.baseline_value_usd ?? null }
      : undefined

  return (
    <Card>
      <CardHeader className="space-y-2">
        <div className="flex flex-wrap items-start justify-between gap-2">
          <CardTitle className="mb-0">{cardTitle}</CardTitle>
          <span
            className={
              fromPostgres
                ? 'inline-flex shrink-0 rounded-full border border-sky-600/35 bg-sky-500/10 px-2 py-0.5 text-[10px] text-sky-200'
                : 'inline-flex shrink-0 rounded-full border border-amber-600/35 bg-amber-500/10 px-2 py-0.5 text-[10px] text-amber-200'
            }
            title={apiIntro}
          >
            {fromPostgres ? readBadgePostgres : readBadgeStream}
          </span>
        </div>
        <p className="text-[11px] text-muted-foreground font-normal leading-snug">{apiIntro}</p>
      </CardHeader>
      <CardContent className="space-y-3">
        <p className="text-sm text-muted-foreground">{chainReconstructHelp}</p>
        {lineage.note ? (
          <p className="text-[11px] text-muted-foreground leading-snug">{localizeLineageNote(lineage.note, locale)}</p>
        ) : null}
        <p className="text-[11px] text-muted-foreground leading-snug">
          <span className="font-medium">{locale === 'pl' ? 'Fees zebrane:' : 'Fees collected:'}</span>{' '}
          {locale === 'pl'
            ? 'to prowizje puli (LP) przypisane do pozycji. Preferujemy on-chain snapshot `fee_owed_a/b` (collect/close) gdy dostępny; bez niego liczby są best-effort.'
            : 'pool (LP) fees attributed to this position. We prefer on-chain `fee_owed_a/b` snapshots (collect/close) when available; otherwise numbers are best-effort.'}{' '}
          <span title={FEE_BASE_UNITS_TOOLTIP} className="cursor-help border-b border-dotted border-muted-foreground/40">
            {locale === 'pl' ? 'baz. jedn.' : 'base units'}
          </span>{' '}
          = {locale === 'pl' ? 'najmniejsze jednostki on-chain (np. lamporty).' : 'smallest on-chain units (e.g. lamports).'}
        </p>

        {lineage.nodes.length > 0 ? (
          <div className="space-y-2">
            <div className="flex justify-end">
              <label className="inline-flex cursor-pointer items-center gap-2 rounded-md border border-border/70 bg-muted/25 px-2.5 py-1 text-[11px] text-muted-foreground">
                <input
                  type="checkbox"
                  checked={invertRangeQuote}
                  onChange={(e) => onInvertRangeQuote(e.target.checked)}
                />
                {locale === 'pl'
                  ? 'Pokazuj zakres jako A per 1 B (zamiast B per 1 A)'
                  : 'Show range as A per 1 B (instead of B per 1 A)'}
              </label>
            </div>
            <div className="overflow-x-auto rounded-md border">
              <table className="w-full text-xs">
                <thead className="bg-muted/50">
                  <tr>
                    <th className="px-2 py-1 text-left">#</th>
                    <th className="px-2 py-1 text-left">{locale === 'pl' ? 'pozycja' : 'position'}</th>
                    <th className="px-2 py-1 text-left">{locale === 'pl' ? 'otwarta' : 'opened'}</th>
                    <th className="px-2 py-1 text-left">{locale === 'pl' ? 'zamknięta / ostatnia' : 'closed / last'}</th>
                    <th className="px-2 py-1 text-left">{locale === 'pl' ? 'zakres @ open' : 'range @ open'}</th>
                    <th
                      className="px-2 py-1 text-left"
                      title={
                        locale === 'pl'
                          ? 'Dla pozycji otwartej: spot z wiersza open w lifecycle (`event_price_a_usd`). Dla zamkniętej: z wiersza close.'
                          : 'Open row: spot from lifecycle open (`event_price_a_usd`). Closed row: from close event.'
                      }
                    >
                      {locale === 'pl' ? 'cena @ open / zamknięciu' : 'Price @ open / close'}
                    </th>
                    <th className="px-2 py-1 text-left">{locale === 'pl' ? 'wartość start' : 'start value'}</th>
                    <th className="px-2 py-1 text-left">{locale === 'pl' ? 'wartość end' : 'end value'}</th>
                    <th className="px-2 py-1 text-left">{locale === 'pl' ? 'kapitał Δ' : 'principal Δ'}</th>
                    <th className="px-2 py-1 text-left">{locale === 'pl' ? 'Sieć (tx)' : 'Network (tx)'}</th>
                    <th className="px-2 py-1 text-left">{locale === 'pl' ? 'Fees zebrane' : 'Fees collected'}</th>
                    <th className="px-2 py-1 text-left">cashflow</th>
                    <th className="px-2 py-1 text-left">net PnL</th>
                  </tr>
                </thead>
                <tbody>
                  {lineage.nodes.map((n, i) => {
                    const tickOpenFromPg =
                      typeof n.chain_history_tick_lower_open === 'number' &&
                      typeof n.chain_history_tick_upper_open === 'number' &&
                      n.chain_history_tick_lower_open < n.chain_history_tick_upper_open
                        ? {
                            lower: n.chain_history_tick_lower_open,
                            upper: n.chain_history_tick_upper_open,
                          }
                        : undefined
                    const tickOpen = tickOpenFromPg ?? nodeOpenCloseRanges.get(n.position_address)?.open

                    const spotOpenPg = parsePositiveUsdSpot(n.chain_history_event_spot_token_a_usd_open)
                    const spotClosePg = parsePositiveUsdSpot(n.chain_history_event_spot_token_a_usd_close)
                    const spotForCell = n.closed_ts_utc
                      ? spotClosePg ?? closePriceByPosition.get(n.position_address)
                      : spotOpenPg ?? openEventPriceByPosition.get(n.position_address)

                    const pgStartRaw =
                      fromPostgres && n.chain_history_start_value_usd != null
                        ? String(n.chain_history_start_value_usd).trim()
                        : ''
                    const pgStartNum = pgStartRaw !== '' ? parseFloat(pgStartRaw) : NaN
                    const pgStart =
                      fromPostgres && pgStartRaw !== '' && Number.isFinite(pgStartNum) && pgStartNum > 0
                        ? pgStartRaw
                        : null

                    // Without this, missing `chain_history_start_value_usd` (stale cache / older API)
                    // fell through to **ledger open_quote** (~$9.66x) even though `baseline_value_usd` on the
                    // same node already matches materialized Postgres (~$9.67x).
                    const pgBaselineFallbackRaw =
                      fromPostgres &&
                      pgStart == null &&
                      !isLineageStoredUsdMissing(n.baseline_value_usd, n.baseline_valuation_quality)
                        ? String(n.baseline_value_usd).trim()
                        : ''
                    const pgBaselineFallbackNum =
                      pgBaselineFallbackRaw !== '' ? parseFloat(pgBaselineFallbackRaw) : NaN
                    const pgBaselineFallback =
                      fromPostgres &&
                      pgBaselineFallbackRaw !== '' &&
                      Number.isFinite(pgBaselineFallbackNum) &&
                      pgBaselineFallbackNum > 0
                        ? pgBaselineFallbackRaw
                        : null

                    const postgresStartFrom: 'column' | 'baseline' | null =
                      pgStart != null ? 'column' : pgBaselineFallback != null ? 'baseline' : null

                    const startDisp =
                      postgresStartFrom === 'column'
                        ? ({ text: formatUsdField(pgStart!, 3), source: 'postgres' } as const)
                        : postgresStartFrom === 'baseline'
                          ? ({
                              text: formatUsdField(pgBaselineFallback!, 3),
                              source: 'postgres',
                            } as const)
                          : formatLineageOpeningUsdDisplay(
                              n,
                              ledgerOpenQuoteUsdByPosition,
                              3,
                              singleNodeOpeningExtra,
                            )

                    const pgEndRaw =
                      fromPostgres && n.chain_history_end_value_usd != null
                        ? String(n.chain_history_end_value_usd).trim()
                        : ''
                    const pgEndNum = pgEndRaw !== '' ? parseFloat(pgEndRaw) : NaN
                    const pgEnd =
                      fromPostgres && pgEndRaw !== '' && Number.isFinite(pgEndNum) && pgEndNum > 0 ? pgEndRaw : null

                    return (
                    <tr key={n.position_address} className="border-t border-border/60">
                      <td className="px-2 py-1 font-mono tabular-nums">{i + 1}</td>
                      <td className="px-2 py-1 font-mono text-[11px] align-top break-all min-w-[12rem] max-w-[28rem]">
                        <Link
                          to={
                            n.closed_ts_utc
                              ? `/positions/closed/${n.position_address}`
                              : `/positions/${n.position_address}`
                          }
                          className="text-primary hover:underline break-all"
                        >
                          {n.position_address}
                        </Link>
                      </td>
                      <td className="px-2 py-1 whitespace-nowrap">{n.opened_ts_utc ? formatDate(n.opened_ts_utc) : '—'}</td>
                      <td className="px-2 py-1 whitespace-nowrap">{n.closed_ts_utc ? formatDate(n.closed_ts_utc) : '—'}</td>
                      <td className="px-2 py-1 whitespace-nowrap font-mono text-[11px]" title="Open-event price range.">
                        {formatRangeFromTicks(
                          tickOpen,
                          n.token_a_label,
                          n.token_b_label,
                          tokenDecimalsA,
                          tokenDecimalsB,
                          invertRangeQuote,
                        )}
                      </td>
                      <td className="px-2 py-1 whitespace-nowrap font-mono text-[11px]">
                        <div className="space-y-1">
                          <div>
                            {formatClosePriceAtEvent(
                              spotForCell,
                              n.token_a_label,
                              n.token_b_label,
                            )}
                          </div>
                          {(() => {
                            const reason = rangeAdjustmentReasonByPosition.get(n.position_address) ?? null
                            const badge = rangeAdjustmentBadge(reason)
                            return (
                              <span
                                className={`inline-flex rounded-full border px-1.5 py-0.5 text-[10px] ${badge.className}`}
                                title={reason ? `range_adjustment_reason: ${reason}` : 'No range adjustment recorded.'}
                              >
                                {badge.text}
                              </span>
                            )
                          })()}
                        </div>
                      </td>
                      <td
                        className="px-2 py-1 whitespace-nowrap font-mono"
                        title={
                          startDisp.source === 'postgres'
                            ? postgresStartFrom === 'baseline'
                              ? locale === 'pl'
                                ? 'Wartość z `baseline_value_usd` węzła (materializacja Postgres), gdy pole `chain_history_start_value_usd` w odpowiedzi jest puste lub brak — zamiast ledger open_quote.'
                                : 'From node `baseline_value_usd` (Postgres materialization) when `chain_history_start_value_usd` is missing or empty in the response — avoids ledger open_quote drift.'
                              : locale === 'pl'
                                ? 'Wartość z kolumny Postgres `position_chain_history_nodes.start_value_usd` (bez heurystyk JSON lineage).'
                                : 'From Postgres column `position_chain_history_nodes.start_value_usd` (bypasses JSON lineage heuristics).'
                            : startDisp.source === 'ledger'
                              ? locale === 'pl'
                                ? 'Wartość z lifecycle ledger (open: open_quote_estimated_value_usd / open_target_usd / open_prev_end_value_usd w details) — gdy API lineage nie ma baseline.'
                                : 'From lifecycle ledger (open row: open_quote_estimated_value_usd / open_target_usd / open_prev_end_value_usd in details) when API lineage baseline is missing.'
                              : startDisp.source === 'totals'
                                ? locale === 'pl'
                                  ? 'Wartość z sum lineage (baseline) przy jednym węźle — gdy wiersz węzła nie ma baseline, a totals łańcucha tak.'
                                  : 'From lineage totals baseline when this table has a single node and the node row has no baseline yet.'
                                : undefined
                        }
                      >
                        {startDisp.text}
                      </td>
                      <td className="px-2 py-1 whitespace-nowrap font-mono">
                        {n.closed_ts_utc
                          ? pgEnd
                            ? formatUsdField(pgEnd, 3)
                            : formatLineageStoredValueUsd(n.current_value_usd, n.current_valuation_quality, 3)
                          : '—'}
                      </td>
                      <td className="px-2 py-1 whitespace-nowrap font-mono">
                        {formatPrincipalDeltaForLineageNode(
                          n,
                          ledgerOpenQuoteUsdByPosition,
                          3,
                          singleNodeOpeningExtra,
                        )}
                      </td>
                      <td className="px-2 py-1 whitespace-nowrap font-mono text-[11px] leading-tight">
                        {(n.tx_fee_lamports ?? 0).toLocaleString()} λ
                        <br />
                        <span className="text-muted-foreground">{formatUsdFixed(parseFloat(String(n.tx_fees_usd)), 4)}</span>
                      </td>
                      <td className="px-2 py-1 whitespace-nowrap font-mono text-[11px]">
                        {(() => {
                          const events = n.collect_events ?? 0
                          const usdNum = parseFloat(String(n.fees_collected_usd ?? '').trim() || '0')
                          const hasTokenVals =
                            n.fees_collected_token_a_ui != null ||
                            n.fees_collected_token_b_ui != null ||
                            n.fees_collected_token_a_raw != null ||
                            n.fees_collected_token_b_raw != null
                          const showLegRows = events > 0 && (hasTokenVals || n.token_a_label || n.token_b_label)
                          return (
                            <>
                              <span>{formatLineageFeesCollectedUsdMain(n.fees_collected_usd, events)}</span>
                              <span className="text-muted-foreground"> · {events}×</span>
                              {showLegRows ? (
                                <div className="text-muted-foreground mt-1 leading-tight">
                                  {n.token_a_label ? (
                                    <div>
                                      {n.token_a_label}: {String(n.fees_collected_token_a_ui ?? '—')}
                                      {formatFeeBaseUnitsClause(n.fees_collected_token_a_raw) ? (
                                        <span title={FEE_BASE_UNITS_TOOLTIP}>
                                          {' '}
                                          {formatFeeBaseUnitsClause(n.fees_collected_token_a_raw)}
                                        </span>
                                      ) : null}
                                    </div>
                                  ) : null}
                                  {n.token_b_label ? (
                                    <div>
                                      {n.token_b_label}: {String(n.fees_collected_token_b_ui ?? '—')}
                                      {formatFeeBaseUnitsClause(n.fees_collected_token_b_raw) ? (
                                        <span title={FEE_BASE_UNITS_TOOLTIP}>
                                          {' '}
                                          {formatFeeBaseUnitsClause(n.fees_collected_token_b_raw)}
                                        </span>
                                      ) : null}
                                    </div>
                                  ) : null}
                                </div>
                              ) : null}
                              {events > 0 && usdNum === 0 && !hasTokenVals ? (
                                <div className="text-muted-foreground mt-1 leading-tight text-[10px]">
                                  {locale === 'pl'
                                    ? 'Brak sumy USD w API (ceny mintów / skala); szczegóły w ledgerze lifecycle.'
                                    : 'Missing USD total in API (mint pricing / scale); see lifecycle ledger for details.'}
                                </div>
                              ) : null}
                              {n.collect_zero_diagnostics ? (
                                <div
                                  className="text-muted-foreground mt-1 leading-tight text-[10px]"
                                  title={n.collect_zero_diagnostics.methodology_note}
                                >
                                  {locale === 'pl' ? 'dlaczego 0' : 'why 0'}: in-range~
                                  {n.collect_zero_diagnostics.in_range_time_share_pct_est ?? '—'}% ·{' '}
                                  {locale === 'pl' ? 'swapy' : 'swaps'}~{n.collect_zero_diagnostics.swap_events_in_window_est} ·{' '}
                                  {locale === 'pl' ? 'udział' : 'share'}~{n.collect_zero_diagnostics.position_share_pct_est ?? '—'}%
                                </div>
                              ) : null}
                            </>
                          )
                        })()}
                      </td>
                      <td className="px-2 py-1 whitespace-nowrap font-mono">{formatUsdField(n.realized_cashflow_usd, 3)}</td>
                      <td
                        className={
                          (() => {
                            const pct = parseFloat(String(n.net_pnl_pct ?? ''))
                            return Number.isFinite(pct) && pct >= 0
                              ? 'px-2 py-1 whitespace-nowrap font-mono text-green-500'
                              : 'px-2 py-1 whitespace-nowrap font-mono text-red-500'
                          })()
                        }
                      >
                        {formatUsdField(n.net_pnl_usd, 3)} (
                        {Number.isFinite(parseFloat(String(n.net_pnl_pct ?? '')))
                          ? formatPercentFixed(n.net_pnl_pct, 3)
                          : '—'}
                        )
                      </td>
                    </tr>
                  )})}
                </tbody>
              </table>
            </div>
          </div>
        ) : (
          <p className="text-sm text-muted-foreground">
            {locale === 'pl'
              ? 'Brak wierszy lineage (brak IL edges / snapshotów DB albo brak zapisu materializacji).'
              : 'No lineage rows yet (missing IL edges / DB snapshots, or no materialized chain-history rows).'}
          </p>
        )}

        {lineage.totals ? (
          <div className="space-y-3">
            <div className="flex justify-end">
              <Button variant="outline" size="sm" onClick={onToggleShowOnlyNonZeroBreakdown}>
                {showOnlyNonZeroBreakdown
                  ? locale === 'pl'
                    ? 'Pokaż wszystkie pozycje'
                    : 'Show all positions'
                  : locale === 'pl'
                    ? 'Pokaż tylko niezerowe'
                    : 'Show non-zero only'}
              </Button>
            </div>
            <div className="rounded-md border border-border/60 bg-muted/10 px-3 py-2 space-y-2">
              <div className="text-xs font-medium text-foreground">
                {isSettlementMode
                  ? locale === 'pl'
                    ? 'Settlement v1 — wynik ekonomiczny łańcucha (net PnL)'
                    : 'Settlement v1 — chain economic result (net PnL)'
                  : locale === 'pl'
                    ? 'Wynik ekonomiczny łańcucha (net PnL)'
                    : 'Chain economic result (net PnL)'}
              </div>
              <div className={`inline-flex w-fit rounded-full border px-2 py-0.5 text-[10px] ${totalsSourceBadge.className}`}>
                {totalsSourceBadge.label}
              </div>
              <p className="text-[10px] text-muted-foreground leading-snug">
                {locale === 'pl'
                  ? 'End NAV + cashflow z ledgera − baseline − opłaty sieci SOL (USD). To inna metryka niż IL vs HODL.'
                  : 'End NAV + ledger cashflow − baseline − SOL network fees (USD). This metric is different from IL vs HODL.'}
              </p>
              <div className="flex flex-wrap gap-x-6 gap-y-1 text-sm">
                <div>
                  <span className="text-muted-foreground">baseline</span>{' '}
                  <span className="font-mono">{formatUsdFixed(lineage.totals.baseline_value_usd, 3)}</span>
                </div>
                <div>
                  <span className="text-muted-foreground">current</span>{' '}
                  <span className="font-mono">{formatUsdFixed(lineage.totals.current_value_usd, 3)}</span>
                </div>
                <div>
                  <span className="text-muted-foreground">tx fees</span>{' '}
                  <span className="font-mono text-[11px] leading-tight inline-block align-top">
                    {lineage.chain_cost_summary != null ? (
                      <>
                        <span className="block">{lineage.chain_cost_summary.tx_fee_lamports_total.toLocaleString()} λ</span>
                        <span className="block text-muted-foreground">
                          {formatUsdFixed(parseFloat(String(lineage.chain_cost_summary.tx_fees_usd_total)), 4)}
                        </span>
                      </>
                    ) : (
                      formatUsdFixed(lineage.totals.tx_fees_usd, 3)
                    )}
                  </span>
                </div>
                <div>
                  <span className="text-muted-foreground">cashflow</span>{' '}
                  <span className="font-mono">{formatUsdFixed(lineage.totals.realized_cashflow_usd, 3)}</span>
                </div>
                <div>
                  <span className="text-muted-foreground">
                    {locale === 'pl' ? 'Realized LP fees (sum)' : 'Realized LP fees (sum)'}
                  </span>{' '}
                  <span className="font-mono text-[11px] leading-tight inline-block align-top">
                    {lineage.chain_cost_summary != null ? (
                      <>
                        <span className="block">
                          {formatLineageFeesCollectedUsdMain(
                            lineage.chain_cost_summary.fees_collected_usd_total,
                            lineage.chain_cost_summary.collect_events_total,
                          )}
                        </span>
                        <span className="block text-muted-foreground">
                          {lineage.chain_cost_summary.collect_events_total}x collect
                        </span>
                      </>
                    ) : (
                      '—'
                    )}
                  </span>
                </div>
                <div>
                  <span className="text-muted-foreground">net PnL</span>{' '}
                  <span
                    className={
                      parseFloat(lineage.totals.net_pnl_pct) >= 0 ? 'font-mono text-green-500' : 'font-mono text-red-500'
                    }
                  >
                    {formatUsdFixed(lineage.totals.net_pnl_usd, 3)} ({formatPercentFixed(lineage.totals.net_pnl_pct, 3)})
                  </span>
                </div>
              </div>
              {lineage.nodes?.length ? (
                <div className="mt-1 space-y-1 text-xs text-muted-foreground">
                  {lineage.nodes.map((n) => {
                    const lam = n.tx_fee_lamports ?? 0
                    if (showOnlyNonZeroBreakdown && lam <= 0) return null
                    return (
                      <div key={`tx-breakdown-${n.position_address}`} className="font-mono">
                        {shortenAddress(n.position_address, 6)}: {lam.toLocaleString()} λ · {formatUsdField(n.tx_fees_usd, 4)}
                      </div>
                    )
                  })}
                </div>
              ) : null}
            </div>
            <div className="rounded-md border border-border/60 bg-muted/10 px-3 py-2 space-y-2">
              <div className="text-xs font-medium text-foreground">
                {isSettlementMode
                  ? locale === 'pl'
                    ? 'Settlement v1 — IL vs koszyk początkowy (benchmark)'
                    : 'Settlement v1 — IL vs initial basket (benchmark)'
                  : locale === 'pl'
                    ? 'IL vs koszyk początkowy (benchmark)'
                    : 'IL vs initial basket (benchmark)'}
              </div>
              <p className="text-[10px] text-muted-foreground leading-snug">
                {locale === 'pl'
                  ? 'Wartość LP vs hipotetyczny HODL tokenów depozytu na starcie łańcucha. Dla zamkniętego łańcucha preferuje ceny USD z eventu close; dla aktywnego używa live/fallback.'
                  : 'LP value vs hypothetical HODL of initial deposit tokens. Closed chains prefer close-event USD prices; active chains use live/fallback prices.'}
              </p>
              <div className="flex flex-wrap gap-x-6 gap-y-1 text-sm">
                <div>
                  <span className="text-muted-foreground">HODL USD</span>{' '}
                  <span className="font-mono">{formatUsdFixed(lineage.totals.hodl_value_usd, 3)}</span>
                </div>
                <div>
                  <span className="text-muted-foreground">Clean IL USD</span>{' '}
                  <span className="font-mono">{formatUsdFixed(lineage.totals.clean_il_usd ?? lineage.totals.il_usd, 3)}</span>
                </div>
                <div>
                  <span className="text-muted-foreground">Clean IL %</span>{' '}
                  <span className="font-mono">{formatPercentFixed(lineage.totals.clean_il_pct ?? lineage.totals.il_pct, 3)}</span>
                </div>
                <div>
                  <span className="text-muted-foreground">LP fees</span>{' '}
                  <span className="font-mono">{formatUsdFixed(lineage.totals.lp_fees_total_usd, 3)}</span>
                </div>
                <div>
                  <span className="text-muted-foreground">LP vs HODL incl. fees</span>{' '}
                  <span className="font-mono">
                    {formatUsdFixed(lineage.totals.lp_vs_hodl_with_fees_usd, 3)} (
                    {formatPercentFixed(lineage.totals.lp_vs_hodl_with_fees_pct, 3)})
                  </span>
                </div>
              </div>
              {lineage.totals.price_basis_note ? (
                <div className="text-[10px] text-muted-foreground leading-snug">
                  <span className="font-medium">{lineage.totals.valuation_price_time_kind}</span>: {lineage.totals.price_basis_note}
                </div>
              ) : null}
            </div>
            <div className="rounded-md border border-border/60 bg-muted/10 px-3 py-2 space-y-2">
              <div className="text-xs font-medium text-foreground">
                {locale === 'pl' ? 'Rozbicie Fees zebrane (per PDA)' : 'Fees collected breakdown (per PDA)'}
              </div>
              <div className="text-[10px] text-muted-foreground leading-snug">
                {locale === 'pl'
                  ? 'Składowe budujące łączną wartość `Fees zebrane` dla całego łańcucha.'
                  : 'Components building total `Fees collected` value for the whole chain.'}
              </div>
              <div className="space-y-1 text-xs text-muted-foreground">
                {lineage.nodes.map((n) => {
                  const collects = n.collect_events ?? 0
                  const hasA = n.fees_collected_token_a_ui != null || n.fees_collected_token_a_raw != null
                  const hasB = n.fees_collected_token_b_ui != null || n.fees_collected_token_b_raw != null
                  if (showOnlyNonZeroBreakdown && collects <= 0 && !hasA && !hasB) return null
                  return (
                    <div key={`fee-breakdown-${n.position_address}`} className="space-y-0.5">
                      <div className="font-mono">
                        {shortenAddress(n.position_address, 6)}: {formatLineageFeesCollectedUsdMain(n.fees_collected_usd, collects)} ·{' '}
                        {collects}×
                      </div>
                      {(n.token_a_label || n.token_b_label) && (hasA || hasB) ? (
                        <div className="pl-3 font-mono">
                          {n.token_a_label ? (
                            <div>
                              {n.token_a_label}: {String(n.fees_collected_token_a_ui ?? '—')}
                              {formatFeeBaseUnitsClause(n.fees_collected_token_a_raw) ? (
                                <span title={FEE_BASE_UNITS_TOOLTIP}>
                                  {' '}
                                  {formatFeeBaseUnitsClause(n.fees_collected_token_a_raw)}
                                </span>
                              ) : null}
                            </div>
                          ) : null}
                          {n.token_b_label ? (
                            <div>
                              {n.token_b_label}: {String(n.fees_collected_token_b_ui ?? '—')}
                              {formatFeeBaseUnitsClause(n.fees_collected_token_b_raw) ? (
                                <span title={FEE_BASE_UNITS_TOOLTIP}>
                                  {' '}
                                  {formatFeeBaseUnitsClause(n.fees_collected_token_b_raw)}
                                </span>
                              ) : null}
                            </div>
                          ) : null}
                        </div>
                      ) : null}
                    </div>
                  )
                })}
              </div>
            </div>
            {lineage.totals.note ? (
              <div className="text-[11px] text-muted-foreground leading-snug">{localizeLineageNote(lineage.totals.note, locale)}</div>
            ) : null}
          </div>
        ) : null}
      </CardContent>
    </Card>
  )
}
