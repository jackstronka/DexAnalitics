import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Link, useNavigate } from 'react-router-dom'
import { Plus, RefreshCw, FlaskConical, XCircle } from 'lucide-react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { ErrorBanner } from '@/components/ui/error-banner'
import ApiDataHint from '@/components/ApiDataHint'
import {
  getOrcaPositionsByOwner,
  getPositions,
  getPositionsFast,
  postPositionsListExtras,
  postCloseAllPositions,
  postCloseAllPositionsPreview,
  getCloseAllBatchStatus,
  reconcileStalePositions,
  getStrategies,
  getStrandedRebalances,
  dismissStrandedRebalance,
} from '@/lib/api'
import type {
  CloseAllBatchStatusResponse,
  CloseAllItemStatus,
  CloseAllPositionsPreviewResponse,
  CloseAllPositionsStartResponse,
  Position,
  PositionListExtrasEntry,
  PositionStrategyDiagnostics,
  Strategy,
} from '@/lib/api'
import { getDevWalletPubkey } from '@/lib/devWallet'
import {
  formatUSD,
  formatPercentFixed,
  formatNumber,
  shortenAddress,
  formatUsdcPriceRange,
  formatInvertedTokenPriceRange,
} from '@/lib/utils'
import { PoolPairLabels } from '@/components/PoolPairLabels'
import { SessionBalancesPanel } from '@/components/SessionBalancesPanel'
import { useI18n } from '@/lib/i18n'
import { getMetricsMode } from '@/lib/metricsMode'
import { useThrottledPositionStreamPnl } from '@/hooks/useThrottledPositionStreamPnl'
import {
  feeSourceLabel,
  formatUncollectedFeesCell,
} from '@/lib/positionListDisplay'

function rangeCellClass(inRange: boolean | undefined) {
  if (inRange === true) {
    return 'text-emerald-600 dark:text-emerald-400 border-l-2 border-emerald-500 pl-2'
  }
  if (inRange === false) {
    return 'text-red-600 dark:text-red-400 border-l-2 border-red-500 pl-2'
  }
  return 'text-muted-foreground border-l-2 border-border pl-2'
}

function rangeStatusLabel(inRange: boolean | undefined, locale: 'pl' | 'en') {
  if (inRange === true) return locale === 'pl' ? 'W zakresie' : 'In range'
  if (inRange === false) return locale === 'pl' ? 'Poza zakresem' : 'Out of range'
  return '—'
}

function strategyTypeLabel(v: Strategy['strategy_type']) {
  return v.replace(/_/g, ' ')
}

function strategyParamsSummary(s: Strategy, locale: 'pl' | 'en') {
  const p = s.parameters ?? {}
  const bits: string[] = []
  if (typeof p.rebalance_threshold_pct === 'number' && p.rebalance_threshold_pct > 0) {
    bits.push(`thr ${p.rebalance_threshold_pct}%`)
  }
  if (
    typeof p.min_rebalance_interval_minutes === 'number' &&
    p.min_rebalance_interval_minutes > 0
  ) {
    bits.push(locale === 'pl' ? `co ${p.min_rebalance_interval_minutes}m` : `every ${p.min_rebalance_interval_minutes}m`)
  } else if (
    typeof p.min_rebalance_interval_hours === 'number' &&
    p.min_rebalance_interval_hours > 0
  ) {
    bits.push(locale === 'pl' ? `co ${p.min_rebalance_interval_hours * 60}m` : `every ${p.min_rebalance_interval_hours * 60}m`)
  }
  if (typeof p.range_width_pct === 'number' && p.range_width_pct > 0) {
    bits.push(`width ${p.range_width_pct}%`)
  }
  if (typeof p.max_il_pct === 'number' && p.max_il_pct > 0) {
    bits.push(`max IL ${p.max_il_pct}%`)
  }
  if (typeof p.retouch_offset_pct === 'number' && p.retouch_offset_pct !== 0) {
    bits.push(`retouch off ${p.retouch_offset_pct}%`)
  }
  if (p.periodic_requires_out_of_range === true) {
    bits.push(locale === 'pl' ? 'tylko OOR' : 'only OOR')
  }
  if (p.rebalance_on_range_exit_immediately === true) {
    bits.push(locale === 'pl' ? 'natychmiast po wyjściu z zakresu' : 'instant on range-exit')
  }
  return bits.length ? bits.join(' · ') : locale === 'pl' ? 'brak jawnych przełączników' : 'no explicit toggles'
}

function isCloseAll6018Error(err: string | null | undefined): boolean {
  if (!err) return false
  const s = err.toLowerCase()
  return s.includes('6018') || s.includes('tokenminsubceeded') || s.includes('0x1782')
}

function closeAllItemStatusLabel(
  status: CloseAllItemStatus,
  t: (key: string) => string,
): string {
  switch (status) {
    case 'queued':
      return t('positions.closeAllItemQueued')
    case 'submitted':
      return t('positions.closeAllItemSubmitted')
    case 'pending_on_chain':
      return t('positions.closeAllItemPending')
    case 'confirmed':
      return t('positions.closeAllItemConfirmed')
    case 'failed':
      return t('positions.closeAllItemFailed')
    case 'skipped_unmanaged_signer':
      return t('positions.closeAllItemSkipped')
    case 'already_closed':
      return t('positions.closeAllItemAlreadyClosed')
    default:
      return status
  }
}

function formatElapsedSinceStart(startedIso: string | undefined): string | null {
  if (!startedIso) return null
  const start = Date.parse(startedIso)
  if (!Number.isFinite(start)) return null
  const sec = Math.max(0, Math.floor((Date.now() - start) / 1000))
  if (sec < 60) return `${sec}s`
  const m = Math.floor(sec / 60)
  const r = sec % 60
  return r > 0 ? `${m}m ${r}s` : `${m}m`
}

function parseNum(v: unknown): number | null {
  if (typeof v === 'number' && Number.isFinite(v)) return v
  if (typeof v === 'string') {
    const n = parseFloat(v)
    return Number.isFinite(n) ? n : null
  }
  return null
}

function estimateNowUsdcFromPosition(p: Position): number | null {
  const quote = (p.range_usdc_quote ?? '').toLowerCase()
  const labelA = (p.token_a_label ?? '').toLowerCase()
  const labelB = (p.token_b_label ?? '').toLowerCase()
  if (quote && labelA && quote.includes(labelA) && typeof p.token_price_a_usd === 'number') {
    return p.token_price_a_usd
  }
  if (quote && labelB && quote.includes(labelB) && typeof p.token_price_b_usd === 'number') {
    return p.token_price_b_usd
  }
  if (labelA === 'usdc' && typeof p.token_price_b_usd === 'number') return p.token_price_b_usd
  if (labelB === 'usdc' && typeof p.token_price_a_usd === 'number') return p.token_price_a_usd
  return null
}

function normalizePendingReopenReason(v: string | null | undefined, locale: 'pl' | 'en') {
  if (!v) return locale === 'pl' ? 'Oczekiwanie na cykl reopen.' : 'Waiting for reopen cycle.'
  if (v.toLowerCase().includes('already queued for pending-open recovery')) {
    return locale === 'pl'
      ? 'W kolejce do auto-reopen (oczekiwanie na kolejny cykl recovery).'
      : 'Queued for auto-reopen (waiting for next recovery cycle).'
  }
  return v
}

export default function Positions() {
  const { t, locale } = useI18n()
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const devPk = getDevWalletPubkey()
  const [ownerInput, setOwnerInput] = useState(() => devPk ?? '')
  const [appliedOwner, setAppliedOwner] = useState(() => devPk ?? '')
  const [reconcileMessage, setReconcileMessage] = useState<string | null>(null)
  const [selectedForClose, setSelectedForClose] = useState<Set<string>>(() => new Set())
  const [showCloseAllConfirm, setShowCloseAllConfirm] = useState(false)
  const [closeAllBatchId, setCloseAllBatchId] = useState<string | null>(null)
  const [closeAllStart, setCloseAllStart] = useState<CloseAllPositionsStartResponse | null>(null)
  const [closeAllBannerDismissed, setCloseAllBannerDismissed] = useState(false)
  const [pageVisible, setPageVisible] = useState(
    () => typeof document === 'undefined' || document.visibilityState === 'visible',
  )
  const [debouncedPreviewAddresses, setDebouncedPreviewAddresses] = useState<string[]>([])
  const metricsMode = getMetricsMode()

  useEffect(() => {
    const onVisibility = () => setPageVisible(document.visibilityState === 'visible')
    document.addEventListener('visibilitychange', onVisibility)
    return () => document.removeEventListener('visibilitychange', onVisibility)
  }, [])

  const positionsFastQ = useQuery({
    queryKey: ['positions', 'fast'],
    queryFn: getPositionsFast,
    staleTime: 20_000,
  })
  const positionsValuedQ = useQuery({
    queryKey: ['positions', 'valued'],
    queryFn: getPositions,
    enabled: !!positionsFastQ.data,
    staleTime: 20_000,
  })
  const data = positionsValuedQ.data ?? positionsFastQ.data
  const isLoading = positionsFastQ.isLoading && !positionsFastQ.data
  const positionsEnriching =
    !!positionsFastQ.data && (positionsValuedQ.isFetching || positionsValuedQ.isLoading)
  const isError = positionsFastQ.isError || positionsValuedQ.isError
  const error = positionsValuedQ.error ?? positionsFastQ.error
  const refetchPositions = useCallback(() => {
    void positionsFastQ.refetch()
    void positionsValuedQ.refetch()
  }, [positionsFastQ, positionsValuedQ])
  const [visibleAddresses, setVisibleAddresses] = useState<Set<string>>(() => new Set())
  const markRowVisible = useCallback((address: string) => {
    const a = address.trim()
    if (!a) return
    setVisibleAddresses((prev) => {
      if (prev.has(a)) return prev
      const next = new Set(prev)
      next.add(a)
      return next
    })
  }, [])
  const strategiesQ = useQuery({
    queryKey: ['strategies'],
    queryFn: getStrategies,
    staleTime: 60_000,
    refetchOnWindowFocus: false,
  })

  const chainQ = useQuery({
    queryKey: ['orca-positions-by-owner', appliedOwner],
    queryFn: () => getOrcaPositionsByOwner(appliedOwner),
    enabled: appliedOwner.trim().length > 0,
    staleTime: 60_000,
  })
  const strandedQ = useQuery({
    queryKey: ['stranded-rebalances'],
    queryFn: getStrandedRebalances,
    staleTime: 10_000,
    refetchInterval: 15_000,
    retry: 1,
  })

  const positions = data?.positions || []
  const positionAddressKey = useMemo(
    () => positions.map((p) => p.address.trim()).join(','),
    [positions],
  )
  const tbodyRef = useRef<HTMLTableSectionElement>(null)

  useEffect(() => {
    if (!positionAddressKey) {
      setVisibleAddresses(new Set())
      return
    }
    setVisibleAddresses((prev) => {
      const next = new Set(prev)
      for (const p of positions.slice(0, 5)) {
        next.add(p.address.trim())
      }
      return next
    })
  }, [positionAddressKey, positions])

  useEffect(() => {
    const tbody = tbodyRef.current
    if (!tbody || positions.length === 0) return
    const obs = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (!entry.isIntersecting) continue
          const addr = (entry.target as HTMLElement).dataset.positionAddress?.trim()
          if (addr) markRowVisible(addr)
        }
      },
      { rootMargin: '120px', threshold: 0.05 },
    )
    tbody.querySelectorAll('tr[data-position-address]').forEach((row) => obs.observe(row))
    return () => obs.disconnect()
  }, [positionAddressKey, markRowVisible, positions.length])
  const monitoredAddressSet = useMemo(
    () => new Set(positions.map((p) => p.address.trim())),
    [positions],
  )
  const selectedCloseAddresses = useMemo(
    () => [...selectedForClose].filter((a) => monitoredAddressSet.has(a)),
    [selectedForClose, monitoredAddressSet],
  )
  const selectedCloseCount = selectedCloseAddresses.length
  const allPositionsSelectedForClose =
    positions.length > 0 && selectedCloseCount === positions.length

  useEffect(() => {
    setSelectedForClose((prev) => {
      const next = new Set([...prev].filter((a) => monitoredAddressSet.has(a)))
      if (next.size === prev.size && [...next].every((a) => prev.has(a))) return prev
      return next
    })
  }, [monitoredAddressSet])

  const visibleAddressKey = useMemo(
    () => [...visibleAddresses].sort().join(','),
    [visibleAddresses],
  )

  const listExtrasQ = useQuery({
    queryKey: ['positions-list-extras', visibleAddressKey],
    queryFn: () => postPositionsListExtras([...visibleAddresses].sort()),
    enabled: visibleAddresses.size > 0,
    staleTime: 30_000,
    refetchOnMount: false,
    refetchOnWindowFocus: false,
    retry: 0,
  })

  const listExtrasByAddress = useMemo(() => {
    const map = new Map<string, PositionListExtrasEntry>()
    for (const item of listExtrasQ.data?.items ?? []) {
      map.set(item.address.trim(), item)
    }
    return map
  }, [listExtrasQ.data])

  const diagnosticsLinkedByPosition = useMemo(() => {
    const map = new Map<string, PositionStrategyDiagnostics[]>()
    for (const [addr, item] of listExtrasByAddress) {
      const linked = (item.linked_strategies ?? []).filter(
        (s): s is PositionStrategyDiagnostics =>
          !!s && typeof s.strategy_id === 'string' && s.strategy_id.trim().length > 0,
      )
      map.set(addr, linked)
    }
    return map
  }, [listExtrasByAddress])

  const streamPnlByAddress = useThrottledPositionStreamPnl(
    positions,
    visibleAddresses,
    metricsMode,
  )
  const poolLabelByAddress = useMemo(() => {
    const m = new Map<string, string>()
    for (const p of positions) {
      if (!p.pool_address) continue
      const a = p.token_a_label?.trim()
      const b = p.token_b_label?.trim()
      if (a && b) m.set(p.pool_address, `${a} / ${b}`)
    }
    for (const row of chainQ.data?.entries ?? []) {
      if (!row.pool_address) continue
      if (m.has(row.pool_address)) continue
      const a = row.token_a_label?.trim()
      const b = row.token_b_label?.trim()
      if (a && b) m.set(row.pool_address, `${a} / ${b}`)
    }
    return m
  }, [positions, chainQ.data])

  const strategiesById = useMemo(() => {
    const map = new Map<string, Strategy>()
    for (const s of strategiesQ.data?.strategies ?? []) {
      map.set(s.id.trim(), s)
    }
    return map
  }, [strategiesQ.data])
  const pendingReopenItems = useMemo(
    () =>
      (strandedQ.data?.items ?? []).filter(
        (it) => it.close_seen === true && it.open_seen === false,
      ),
    [strandedQ.data],
  )
  const [strandedSessionPick, setStrandedSessionPick] = useState('')
  const activeStrandedSession = useMemo(() => {
    const pick = strandedSessionPick.trim()
    if (pick) return pick
    return pendingReopenItems[0]?.rebalance_session_id?.trim() ?? ''
  }, [strandedSessionPick, pendingReopenItems])
  const devWalletOwner = getDevWalletPubkey() ?? ''
  const dismissStrandedM = useMutation({
    mutationFn: (sessionId: string) => dismissStrandedRebalance(sessionId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['stranded-rebalances'] })
    },
  })

  const reconcileMutation = useMutation({
    mutationFn: reconcileStalePositions,
    onSuccess: (report) => {
      queryClient.invalidateQueries({ queryKey: ['positions', 'fast'] })
      queryClient.invalidateQueries({ queryKey: ['positions', 'valued'] })
      setReconcileMessage(
        locale === 'pl'
          ? `Reconcile: sprawdzono ${report.checked}, zamknięto registry ${report.registry_closed.length}, usunięto linki strategii ${report.strategy_links_removed}, nadal on-chain ${report.still_on_chain}, błędy RPC ${report.rpc_errors}.`
          : `Reconcile: checked ${report.checked}, registry closed ${report.registry_closed.length}, strategy links removed ${report.strategy_links_removed}, still on-chain ${report.still_on_chain}, RPC errors ${report.rpc_errors}.`,
      )
    },
    onError: (e: Error) => setReconcileMessage(e.message),
  })

  useEffect(() => {
    if (!showCloseAllConfirm || selectedCloseCount === 0) {
      setDebouncedPreviewAddresses([])
      return
    }
    const id = window.setTimeout(() => {
      setDebouncedPreviewAddresses(selectedCloseAddresses)
    }, 300)
    return () => window.clearTimeout(id)
  }, [showCloseAllConfirm, selectedCloseCount, selectedCloseAddresses])

  const closeSelectedRequest = useMemo(
    () =>
      ({
        scope: 'explicit' as const,
        addresses: selectedCloseAddresses,
        pause_linked_strategies: true,
        options: { skip_pre_collect: true, send_mode: 'send_first', slippage_bps: 200 },
      }) satisfies Parameters<typeof postCloseAllPositions>[0],
    [selectedCloseAddresses],
  )

  const closePreviewRequest = useMemo(
    () =>
      ({
        scope: 'explicit' as const,
        addresses: debouncedPreviewAddresses,
        pause_linked_strategies: true,
        options: { skip_pre_collect: true, send_mode: 'send_first', slippage_bps: 200 },
      }) satisfies Parameters<typeof postCloseAllPositions>[0],
    [debouncedPreviewAddresses],
  )

  const closeAllMutation = useMutation({
    mutationFn: () => postCloseAllPositions(closeSelectedRequest),
    onSuccess: (resp) => {
      setShowCloseAllConfirm(false)
      setSelectedForClose(new Set())
      setCloseAllBatchId(resp.batch_id)
      setCloseAllStart(resp)
      setCloseAllBannerDismissed(false)
      queryClient.invalidateQueries({ queryKey: ['close-all-batch', resp.batch_id] })
    },
  })

  const closeAllPreviewQ = useQuery({
    queryKey: ['close-all-preview', debouncedPreviewAddresses.join(',')],
    queryFn: () => postCloseAllPositionsPreview(closePreviewRequest),
    enabled: showCloseAllConfirm && debouncedPreviewAddresses.length > 0,
    staleTime: 30_000,
    retry: 1,
  })

  const closeAllPreview: CloseAllPositionsPreviewResponse | undefined = closeAllPreviewQ.data
  const closeAllPreviewBusy =
    showCloseAllConfirm &&
    selectedCloseCount > 0 &&
    (debouncedPreviewAddresses.length !== selectedCloseCount ||
      closeAllPreviewQ.isLoading ||
      closeAllPreviewQ.isFetching)

  const closeAllBatchQ = useQuery({
    queryKey: ['close-all-batch', closeAllBatchId],
    queryFn: () => getCloseAllBatchStatus(closeAllBatchId!),
    enabled: !!closeAllBatchId,
    refetchInterval: (q) => {
      if (!pageVisible) return false
      const status = q.state.data?.status
      if (status === 'done' || status === 'failed') return false
      return 8000
    },
  })

  const closeAllBatch: CloseAllBatchStatusResponse | undefined = closeAllBatchQ.data
  const closeAllRunning =
    !!closeAllBatchId &&
    !closeAllBatchQ.isError &&
    closeAllBatchQ.data?.status !== 'done' &&
    closeAllBatchQ.data?.status !== 'failed'

  const [closeAllElapsedTick, setCloseAllElapsedTick] = useState(0)
  useEffect(() => {
    if (!closeAllRunning) return
    const id = window.setInterval(() => setCloseAllElapsedTick((n) => n + 1), 1000)
    return () => window.clearInterval(id)
  }, [closeAllRunning])

  const closeAllElapsedLabel = useMemo(
    () => formatElapsedSinceStart(closeAllBatch?.started_ts_utc),
    [closeAllBatch?.started_ts_utc, closeAllElapsedTick],
  )

  const prevCloseAllDone = closeAllBatch?.status === 'done'
  useEffect(() => {
    if (prevCloseAllDone) {
      queryClient.invalidateQueries({ queryKey: ['positions', 'fast'] })
      queryClient.invalidateQueries({ queryKey: ['positions', 'valued'] })
    }
  }, [prevCloseAllDone, queryClient])

  const showReconcileCta =
    !!data?.meta &&
    ((data.meta.skipped_absent_cached ?? 0) > 0 ||
      (data.meta.skipped_registry_closed ?? 0) > 0 ||
      (data.meta.skipped_chain_error ?? 0) > 0)

  return (
    <div className="space-y-6">
      <ApiDataHint />

      <div className="flex items-center justify-between">
        <h1 className="text-3xl font-bold">{t('positions.title')}</h1>
        <div className="flex gap-2">
          {showReconcileCta ? (
            <Button
              variant="outline"
              size="sm"
              disabled={reconcileMutation.isPending}
              onClick={() => {
                setReconcileMessage(null)
                reconcileMutation.mutate()
              }}
            >
              {locale === 'pl' ? 'Wyczyść martwe registry' : 'Reconcile stale registry'}
            </Button>
          ) : null}
          <Button variant="outline" size="sm" onClick={() => refetchPositions()}>
            <RefreshCw className="h-4 w-4 mr-2" />
            {t('positions.refresh')}
          </Button>
          {selectedCloseCount > 0 ? (
            <Button
              variant="destructive"
              size="sm"
              disabled={closeAllRunning || closeAllMutation.isPending || isLoading}
              onClick={() => setShowCloseAllConfirm(true)}
            >
              <XCircle className="h-4 w-4 mr-2" />
              {t('positions.closeSelectedCount').replace('{n}', String(selectedCloseCount))}
            </Button>
          ) : null}
          <Button variant="outline" size="sm" onClick={() => navigate('/experiments/new')}>
            <FlaskConical className="h-4 w-4 mr-2" />
            {t('positions.newExperiment')}
          </Button>
          <Button size="sm" onClick={() => navigate('/positions/new')}>
            <Plus className="h-4 w-4 mr-2" />
            {t('positions.openPosition')}
          </Button>
        </div>
      </div>

      {showCloseAllConfirm ? (
        <Card className="border-destructive/40">
          <CardHeader>
            <CardTitle className="text-destructive">{t('positions.closeSelectedConfirmTitle')}</CardTitle>
            <p className="text-sm text-muted-foreground font-normal">
              {t('positions.closeSelectedConfirmBody').replace('{n}', String(selectedCloseCount))}
            </p>
            {closeAllPreviewBusy ? (
              <p className="text-sm text-muted-foreground font-normal mt-2">
                {t('positions.closeAllPreviewLoading')}
              </p>
            ) : null}
            {closeAllPreviewQ.isError ? (
              <p className="text-sm text-destructive font-normal mt-2">
                {t('positions.closeAllPreviewError')}{' '}
                {(closeAllPreviewQ.error as Error)?.message ?? ''}
              </p>
            ) : null}
            {closeAllPreview ? (
              <p className="text-sm font-medium mt-2">
                {t('positions.closeAllClosableCount')
                  .replace('{n}', String(closeAllPreview.closable))
                  .replace('{total}', String(closeAllPreview.total))}
              </p>
            ) : null}
          </CardHeader>
          <CardContent className="space-y-3">
            {closeAllPreview && closeAllPreview.groups.length > 0 ? (
              <div className="text-sm">
                <div className="font-medium mb-1">{t('positions.closeAllGroups')}</div>
                <ul className="list-disc pl-5 space-y-0.5 text-muted-foreground">
                  {closeAllPreview.groups.map((g) => (
                    <li key={`${g.wallet_id}-${g.owner_pubkey}`}>
                      {g.wallet_id} · {g.count}{' '}
                      {locale === 'pl' ? 'poz.' : 'pos.'} · {shortenAddress(g.owner_pubkey, 4)}
                    </li>
                  ))}
                </ul>
              </div>
            ) : null}
            {closeAllPreview && (closeAllPreview.skipped_preview?.length ?? 0) > 0 ? (
              <div className="text-sm">
                <div className="font-medium mb-1">{t('positions.closeAllSkipped')}</div>
                <ul className="list-disc pl-5 space-y-0.5 text-muted-foreground">
                  {closeAllPreview.skipped_preview!.map((s) => (
                    <li key={s.address}>
                      {shortenAddress(s.address, 4)}
                      {s.owner_pubkey ? ` (${shortenAddress(s.owner_pubkey, 4)})` : ''} — {s.reason}
                    </li>
                  ))}
                </ul>
              </div>
            ) : null}
            {closeAllPreview && closeAllPreview.closable === 0 ? (
              <p className="text-sm text-amber-600 dark:text-amber-400">
                {t('positions.closeAllNoneClosable')}
              </p>
            ) : null}
            {closeAllPreview && closeAllPreview.closable > 0 ? (
              <p className="text-sm text-muted-foreground">{t('positions.closeAllBulkSlippageHint')}</p>
            ) : null}
            <div className="flex flex-wrap gap-2">
              <Button
                variant="destructive"
                disabled={
                  closeAllMutation.isPending ||
                  closeAllPreviewQ.isLoading ||
                  closeAllPreviewQ.isFetching ||
                  closeAllPreviewQ.isError ||
                  debouncedPreviewAddresses.length !== selectedCloseCount ||
                  !closeAllPreview ||
                  closeAllPreview.closable === 0
                }
                onClick={() => closeAllMutation.mutate()}
              >
                {closeAllMutation.isPending
                  ? t('positions.closeAllStarting')
                  : t('positions.closeSelectedConfirmAction')}
              </Button>
              <Button
                variant="outline"
                disabled={closeAllMutation.isPending}
                onClick={() => setShowCloseAllConfirm(false)}
              >
                {t('positions.closeAllCancel')}
              </Button>
              {closeAllMutation.isError ? (
                <p className="w-full text-sm text-destructive">
                  {(closeAllMutation.error as Error).message}
                </p>
              ) : null}
            </div>
          </CardContent>
        </Card>
      ) : null}

      {closeAllBatchId && !closeAllBannerDismissed ? (
        <Card className="border-amber-500/40">
          <CardHeader className="flex flex-row items-start justify-between gap-4">
            <div>
              <CardTitle>
                {closeAllBatch?.status === 'done'
                  ? t('positions.closeAllDone')
                  : closeAllRunning
                    ? t('positions.closeAllBackgroundTitle')
                    : t('positions.closeAllProgress')}
              </CardTitle>
              <p className="text-sm text-muted-foreground font-normal mt-1">
                batch {closeAllBatchId.slice(0, 8)}…
              </p>
            </div>
            <Button variant="ghost" size="sm" onClick={() => setCloseAllBannerDismissed(true)}>
              {t('positions.closeAllDismiss')}
            </Button>
          </CardHeader>
          <CardContent className="space-y-3 text-sm">
            {closeAllBatchQ.isError ? (
              <ErrorBanner>
                {t('positions.closeAllBatchNotFound')}{' '}
                {(closeAllBatchQ.error as Error)?.message ?? ''}
              </ErrorBanner>
            ) : null}
            {closeAllRunning ? (
              <p className="text-muted-foreground">{t('positions.closeAllSendFirstHint')}</p>
            ) : null}
            {closeAllElapsedLabel && closeAllRunning ? (
              <p className="text-muted-foreground">
                {t('positions.closeAllElapsed').replace('{s}', closeAllElapsedLabel)}
              </p>
            ) : null}
            {closeAllBatch?.summary ? (
              <p>
                {(() => {
                  const submittedN =
                    closeAllBatch.items?.filter((i) => i.status === 'submitted').length ?? 0
                  return t('positions.closeAllSummarySendFirst')
                    .replace('{confirmed}', String(closeAllBatch.summary.closed))
                    .replace('{total}', String(closeAllBatch.summary.total))
                    .replace('{submitted}', String(submittedN))
                    .replace('{failed}', String(closeAllBatch.summary.failed))
                    .replace('{pending}', String(closeAllBatch.summary.pending))
                })()}
              </p>
            ) : closeAllStart ? (
              <p>
                {locale === 'pl'
                  ? `Zakolejkowano ${closeAllStart.total} pozycji.`
                  : `Queued ${closeAllStart.total} positions.`}
              </p>
            ) : null}
            {(closeAllStart?.groups?.length ?? 0) > 0 ? (
              <div>
                <div className="font-medium mb-1">{t('positions.closeAllGroups')}</div>
                <ul className="list-disc pl-5 space-y-0.5 text-muted-foreground">
                  {(closeAllStart?.groups ?? []).map((g) => (
                    <li key={`${g.wallet_id}-${g.owner_pubkey}`}>
                      {g.wallet_id} · {g.count} · {shortenAddress(g.owner_pubkey, 4)}
                    </li>
                  ))}
                </ul>
              </div>
            ) : null}
            {(closeAllStart?.skipped_preview?.length ?? 0) > 0 ? (
              <div>
                <div className="font-medium mb-1">{t('positions.closeAllSkipped')}</div>
                <ul className="list-disc pl-5 space-y-0.5 text-muted-foreground">
                  {closeAllStart!.skipped_preview!.map((s) => (
                    <li key={s.address}>
                      {shortenAddress(s.address, 4)}
                      {s.owner_pubkey ? ` (${shortenAddress(s.owner_pubkey, 4)})` : ''} — {s.reason}
                    </li>
                  ))}
                </ul>
              </div>
            ) : null}
            {(closeAllBatch?.items?.length ?? 0) > 0 ? (
              <div>
                <div className="font-medium mb-1">{t('positions.closeAllItemList')}</div>
                <ul className="space-y-1.5">
                  {closeAllBatch!.items.map((item) => (
                    <li
                      key={item.address}
                      className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5 font-mono text-xs"
                    >
                      <Link
                        to={`/positions/${item.address}`}
                        className="text-primary hover:underline"
                      >
                        {shortenAddress(item.address, 4)}
                      </Link>
                      <span
                        className={
                          item.status === 'confirmed' || item.status === 'already_closed'
                            ? 'text-emerald-600 dark:text-emerald-400'
                            : item.status === 'failed'
                              ? 'text-destructive'
                              : item.status === 'pending_on_chain'
                              ? 'text-amber-600 dark:text-amber-400'
                              : item.status === 'submitted'
                                ? 'text-sky-600 dark:text-sky-400'
                              : 'text-muted-foreground'
                        }
                      >
                        {closeAllItemStatusLabel(item.status, t)}
                      </span>
                      {item.signature ? (
                        <a
                          href={`https://solscan.io/tx/${item.signature}`}
                          target="_blank"
                          rel="noreferrer"
                          className="text-primary hover:underline font-sans"
                        >
                          {shortenAddress(item.signature, 4)}
                        </a>
                      ) : null}
                      {item.error ? (
                        <span className="text-destructive font-sans break-all">{item.error}</span>
                      ) : null}
                      {item.status === 'failed' && isCloseAll6018Error(item.error) ? (
                        <span className="text-amber-700 dark:text-amber-400 font-sans w-full">
                          {t('positions.closeAll6018FailedHint')}
                        </span>
                      ) : null}
                    </li>
                  ))}
                </ul>
              </div>
            ) : null}
            {closeAllBatchQ.isFetching && closeAllRunning ? (
              <p className="text-muted-foreground">{t('positions.loading')}</p>
            ) : null}
          </CardContent>
        </Card>
      ) : null}

      <Card>
        <CardHeader>
          <CardTitle>{t('positions.monitoredTitle')}</CardTitle>
          <p className="text-sm text-muted-foreground font-normal">
            {locale === 'pl'
              ? 'Monitor API + otwarte wpisy registry + adresy ze strategii w stanie running (tylko żywe on-chain).'
              : 'API monitor + registry opens + running strategy position_addresses (on-chain only).'}
          </p>
        </CardHeader>
        <CardContent>
          {isError ? (
            <ErrorBanner className="mb-4">
              {(error as Error).message}
              <div className="mt-2">
                <Button type="button" variant="outline" size="sm" onClick={() => refetchPositions()}>
                  {t('positions.refresh')}
                </Button>
              </div>
            </ErrorBanner>
          ) : null}
          {data?.meta &&
          (data.meta.skipped_absent_cached ||
            data.meta.skipped_chain_error ||
            data.meta.skipped_registry_closed) ? (
            <p className="text-xs text-muted-foreground mb-4">
              {locale === 'pl'
                ? `Część adresów pominięta (martwe registry/strategia lub RPC): cache=${data.meta.skipped_absent_cached ?? 0}, registry zamknięte=${data.meta.skipped_registry_closed ?? 0}, błąd RPC=${data.meta.skipped_chain_error ?? 0}.`
                : `Some addresses skipped (stale registry/strategy or RPC): cached absent=${data.meta.skipped_absent_cached ?? 0}, registry closed=${data.meta.skipped_registry_closed ?? 0}, RPC error=${data.meta.skipped_chain_error ?? 0}.`}
            </p>
          ) : null}
          {reconcileMessage ? (
            <p className="text-xs text-muted-foreground mb-4">{reconcileMessage}</p>
          ) : null}
          {positionsEnriching ? (
            <p className="text-xs text-muted-foreground mb-4">{t('positions.listEnriching')}</p>
          ) : null}
          {isLoading ? (
            <div className="text-center py-8 text-muted-foreground">{t('positions.loading')}</div>
          ) : !isError && positions.length === 0 ? (
            <div className="text-center py-8 text-muted-foreground space-y-2 max-w-xl mx-auto">
              <p>
                {locale === 'pl'
                  ? 'Brak pozycji w monitorze API — to nie jest lista wszystkich NFT Orca na portfelu.'
                  : 'No positions in API monitor — this is not a full list of Orca NFTs for the wallet.'}
              </p>
              <p className="text-xs">
                {locale === 'pl'
                  ? 'Uruchom strategię z adresami pozycji, dodaj pozycję do monitora, albo sprawdź on-chain:'
                  : 'Start a strategy with position addresses, add position to monitor, or check on-chain:'}{' '}
                <code className="text-[11px]">orca-positions-list</code> (CLI).
              </p>
            </div>
          ) : (
            <div className="overflow-x-auto">
              <table className="w-full">
                <thead>
                  <tr className="border-b text-left text-sm text-muted-foreground">
                    <th className="pb-3 w-10 pr-2">
                      <input
                        type="checkbox"
                        className="h-4 w-4 rounded border-input"
                        checked={allPositionsSelectedForClose}
                        aria-label={
                          allPositionsSelectedForClose
                            ? t('positions.closeDeselectAll')
                            : t('positions.closeSelectAll')
                        }
                        onChange={() => {
                          if (allPositionsSelectedForClose) {
                            setSelectedForClose(new Set())
                          } else {
                            setSelectedForClose(
                              new Set(positions.map((p) => p.address.trim())),
                            )
                          }
                        }}
                      />
                    </th>
                    <th className="pb-3 font-medium">{locale === 'pl' ? 'Pozycja' : 'Position'}</th>
                    <th className="pb-3 font-medium">{locale === 'pl' ? 'Strategia' : 'Strategy'}</th>
                    <th className="pb-3 font-medium">{locale === 'pl' ? 'Agent' : 'Agent'}</th>
                    <th className="pb-3 font-medium">{locale === 'pl' ? 'Zakres (in / out)' : 'Range (in / out)'}</th>
                    <th className="pb-3 font-medium text-right">{locale === 'pl' ? 'Wartość' : 'Value'}</th>
                    <th className="pb-3 font-medium text-right">PnL</th>
                    <th className="pb-3 font-medium text-right">{locale === 'pl' ? 'Fee (niezebrane)' : 'Fees (uncollected)'}</th>
                    <th className="pb-3 font-medium text-center">{locale === 'pl' ? 'Status' : 'Status'}</th>
                  </tr>
                </thead>
                <tbody ref={tbodyRef}>
                  {positions.map((position) => {
                    const addr = position.address.trim()
                    const isSelectedForClose = selectedForClose.has(addr)
                    return (
                    <tr
                      key={position.address}
                      className="border-b last:border-0"
                      data-position-address={addr}
                    >
                      <td className="py-4 w-10 pr-2 align-top">
                        <input
                          type="checkbox"
                          className="h-4 w-4 rounded border-input mt-1"
                          checked={isSelectedForClose}
                          aria-label={t('positions.closeSelectRow')}
                          onChange={() => {
                            setSelectedForClose((prev) => {
                              const next = new Set(prev)
                              if (next.has(addr)) next.delete(addr)
                              else next.add(addr)
                              return next
                            })
                          }}
                        />
                      </td>
                      <td className="py-4 max-w-[14rem]">
                        <Link
                          to={`/positions/${position.address}`}
                          className="block hover:text-primary space-y-1"
                        >
                          <PoolPairLabels
                            labelA={position.token_a_label}
                            labelB={position.token_b_label}
                            mintA={position.token_mint_a}
                            mintB={position.token_mint_b}
                            priceA={position.token_price_a_usd}
                            priceB={position.token_price_b_usd}
                          />
                          <div className="font-medium font-mono text-sm">
                            {shortenAddress(position.address)}
                          </div>
                        </Link>
                      </td>
                      <td className="py-4 max-w-[18rem]">
                        {(() => {
                          const backendLinked = diagnosticsLinkedByPosition.get(position.address.trim()) ?? []
                          const diagnosticsPending =
                            listExtrasQ.isLoading || listExtrasQ.isFetching
                          if (diagnosticsPending) {
                            return <span className="text-xs text-muted-foreground">{t('positions.checking')}</span>
                          }
                          if (!backendLinked.length) {
                            return <span className="text-xs text-muted-foreground">{t('positions.notLinked')}</span>
                          }
                          return (
                            <div className="space-y-1.5">
                              {backendLinked.map((diagLinked) => {
                                const strategy = strategiesById.get(diagLinked.strategy_id.trim())
                                return (
                                  <div key={diagLinked.strategy_id} className="text-xs leading-tight">
                                  <div className="font-medium">
                                    {strategy?.name ?? diagLinked.name}{' '}
                                    <span className="text-muted-foreground">
                                      ({strategyTypeLabel(strategy?.strategy_type ?? diagLinked.strategy_type)})
                                    </span>
                                  </div>
                                  {strategy ? (
                                    <div className="text-muted-foreground">{strategyParamsSummary(strategy, locale)}</div>
                                  ) : (
                                    <div className="text-muted-foreground">
                                      {locale === 'pl' ? 'podpięta (szczegóły z diagnostics)' : 'linked (details from diagnostics)'}
                                    </div>
                                  )}
                                </div>
                                )
                              })}
                            </div>
                          )
                        })()}
                      </td>
                      <td className="py-4">
                        {(() => {
                          const extras = listExtrasByAddress.get(position.address.trim())
                          if (listExtrasQ.isLoading || listExtrasQ.isFetching) {
                            return <span className="text-xs text-muted-foreground">{locale === 'pl' ? 'Sprawdzanie…' : 'Checking…'}</span>
                          }
                          const session = extras?.agent_session
                          if (!session) {
                            return (
                              <span className="inline-flex items-center gap-1.5 px-2 py-1 rounded-full text-xs font-medium bg-muted text-muted-foreground">
                                {locale === 'pl' ? 'nieaktywna' : 'inactive'}
                              </span>
                            )
                          }
                          return (
                            <div className="space-y-1">
                              <span className="inline-flex items-center gap-1.5 px-2 py-1 rounded-full text-xs font-medium bg-emerald-500/10 text-emerald-500">
                                <span className="h-1.5 w-1.5 rounded-full bg-emerald-500" />
                                {locale === 'pl' ? 'aktywna' : 'active'}
                              </span>
                              <div className="text-[10px] text-muted-foreground">
                                {locale === 'pl' ? 'następny:' : 'next:'}{' '}
                                {session.next_scan_ts_utc
                                  ? new Date(session.next_scan_ts_utc).toLocaleTimeString()
                                  : '—'}
                              </div>
                            </div>
                          )
                        })()}
                      </td>
                      <td className="py-4">
                        <div className="space-y-1">
                          <span className={`text-sm block ${rangeCellClass(position.in_range)}`}>
                            {formatUsdcPriceRange(
                              position.range_lower_usdc ?? undefined,
                              position.range_upper_usdc ?? undefined,
                              position.range_usdc_quote ?? undefined,
                            ) ??
                              formatInvertedTokenPriceRange(
                                position.range_lower_price ?? undefined,
                                position.range_upper_price ?? undefined,
                                position.range_price_quote ?? undefined,
                              ) ??
                              `${position.tick_lower} → ${position.tick_upper}`}
                          </span>
                          {(() => {
                            const lowerUsdc = parseNum(position.range_lower_usdc)
                            const upperUsdc = parseNum(position.range_upper_usdc)
                            const lowerGeneric = parseNum(position.range_lower_price)
                            const upperGeneric = parseNum(position.range_upper_price)
                            const useUsdc = lowerUsdc !== null && upperUsdc !== null
                            const lower = useUsdc ? lowerUsdc : lowerGeneric
                            const upper = useUsdc ? upperUsdc : upperGeneric
                            const now = useUsdc
                              ? estimateNowUsdcFromPosition(position)
                              : parseNum(position.token_price_a_usd) ??
                                parseNum(position.token_price_b_usd)
                            if (lower === null || upper === null || now === null || upper <= lower) return null
                            const markerPct = Math.max(0, Math.min(100, ((now - lower) / (upper - lower)) * 100))
                            return (
                              <div className="pt-0.5">
                                <div className="relative h-1.5 rounded-full bg-muted">
                                  <span
                                    className={`absolute top-1/2 h-3 w-3 -translate-y-1/2 -translate-x-1/2 rounded-full border border-background ${
                                      position.in_range ? 'bg-emerald-500' : 'bg-red-500'
                                    }`}
                                    style={{ left: `${markerPct}%` }}
                                    aria-label={locale === 'pl' ? 'Bieżąca cena względem zakresu pozycji' : 'Current price inside position range'}
                                  />
                                </div>
                              </div>
                            )
                          })()}
                          <span className="text-[11px] text-muted-foreground">
                            {rangeStatusLabel(position.in_range, locale)}
                          </span>
                        </div>
                      </td>
                      <td className="py-4 text-right font-medium">
                        {formatUSD(position.value_usd)}
                      </td>
                      <td
                        className={`py-4 text-right ${(() => {
                          const pnlQ = streamPnlByAddress.get(position.address.trim())
                          const streamPct = parseNum(pnlQ?.data?.net_pnl_pct)
                          const fallbackPct = parseNum(position.pnl.net_pnl_pct)
                          const pct = streamPct ?? fallbackPct ?? 0
                          return pct >= 0 ? 'text-green-500' : 'text-red-500'
                        })()}`}
                      >
                        {(() => {
                          const pnlQ = streamPnlByAddress.get(position.address.trim())
                          const streamPct = parseNum(pnlQ?.data?.net_pnl_pct)
                          if (streamPct !== null) {
                            return (
                              <div className="space-y-0.5">
                                <div>{formatPercentFixed(streamPct, 3)}</div>
                                <div className="text-[10px] text-muted-foreground">
                                  {locale === 'pl' ? 'źródło: stream' : 'source: stream'}
                                </div>
                              </div>
                            )
                          }
                          const fallbackPct = parseNum(position.pnl.net_pnl_pct)
                          return (
                            <div className="space-y-0.5">
                              <div>{formatPercentFixed(fallbackPct ?? 0, 3)}</div>
                              <div className="text-[10px] text-muted-foreground">
                                {pnlQ?.isFetching
                                  ? locale === 'pl'
                                    ? 'źródło: stream (ładowanie…)'
                                    : 'source: stream (loading…)'
                                  : locale === 'pl'
                                    ? 'źródło: lista API'
                                    : 'source: API list'}
                              </div>
                            </div>
                          )
                        })()}
                      </td>
                      <td className="py-4 text-right text-green-500">
                        <div className="space-y-0.5">
                          <div>{formatUncollectedFeesCell(position)}</div>
                          <div className="text-[10px] text-muted-foreground">
                            {locale === 'pl' ? 'źródło:' : 'source:'}{' '}
                            {feeSourceLabel(position.valuation_source, locale)}
                          </div>
                          {position.uncollected_fees ? (
                            <div className="text-[10px] text-muted-foreground font-mono">
                              {position.uncollected_fees.token_a_label}:{' '}
                              {formatNumber(position.uncollected_fees.amount_a, 6)} ·{' '}
                              {position.uncollected_fees.token_b_label}:{' '}
                              {formatNumber(position.uncollected_fees.amount_b, 6)}
                            </div>
                          ) : null}
                        </div>
                      </td>
                      <td className="py-4 text-center">
                        <span className={`inline-flex items-center gap-1.5 px-2 py-1 rounded-full text-xs font-medium ${
                          position.status === 'active' 
                            ? 'bg-green-500/10 text-green-500' 
                            : position.status === 'pending'
                            ? 'bg-yellow-500/10 text-yellow-500'
                            : 'bg-muted text-muted-foreground'
                        }`}>
                          <span className={`h-1.5 w-1.5 rounded-full ${
                            position.status === 'active' 
                              ? 'bg-green-500' 
                              : position.status === 'pending'
                              ? 'bg-yellow-500'
                              : 'bg-muted-foreground'
                          }`} />
                          {position.status}
                        </span>
                      </td>
                    </tr>
                    )
                  })}
                </tbody>
              </table>
            </div>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>{t('positions.pendingTitle')}</CardTitle>
          <p className="text-sm text-muted-foreground font-normal">
            {locale === 'pl'
              ? 'Sesje rebalance, gdzie bot zamknął starą pozycję, ale nowa nie została jeszcze otwarta. Po udanym reopen wpis znika z tej sekcji.'
              : 'Rebalance sessions where bot closed old position but new one is not opened yet. Entry disappears after successful reopen.'}
          </p>
        </CardHeader>
        <CardContent>
          {strandedQ.isLoading ? (
            <div className="text-center py-6 text-muted-foreground">{t('positions.loading')}</div>
          ) : strandedQ.error ? (
            <ErrorBanner>{(strandedQ.error as Error).message}</ErrorBanner>
          ) : pendingReopenItems.length === 0 ? (
            <div className="text-muted-foreground text-sm">{t('positions.pendingEmpty')}</div>
          ) : (
            <div className="space-y-4">
              <div className="flex flex-wrap items-end gap-3">
                <label className="flex flex-col gap-1 text-sm min-w-[12rem]">
                  <span className="text-muted-foreground">{t('positions.sessionCapitalTitle')}</span>
                  <select
                    className="flex h-9 rounded-md border border-input bg-background px-2 text-sm font-mono shadow-sm"
                    value={activeStrandedSession}
                    onChange={(e) => setStrandedSessionPick(e.target.value)}
                  >
                    {pendingReopenItems.map((it) => (
                      <option key={it.rebalance_session_id} value={it.rebalance_session_id}>
                        {shortenAddress(it.rebalance_session_id)}
                        {it.old_position ? ` · ${shortenAddress(it.old_position)}` : ''}
                      </option>
                    ))}
                  </select>
                </label>
              </div>
              {activeStrandedSession ? (
                <SessionBalancesPanel
                  sessionId={activeStrandedSession}
                  owner={devWalletOwner || undefined}
                />
              ) : null}
            <div className="overflow-x-auto">
              <table className="w-full">
                <thead>
                  <tr className="border-b text-left text-sm text-muted-foreground">
                    <th className="pb-3 font-medium">{locale === 'pl' ? 'Zamknięta pozycja' : 'Closed position'}</th>
                    <th className="pb-3 font-medium">{locale === 'pl' ? 'Pula' : 'Pool'}</th>
                    <th className="pb-3 font-medium">{locale === 'pl' ? 'Zamknięta o' : 'Closed at'}</th>
                    <th className="pb-3 font-medium">{locale === 'pl' ? 'Docelowy zakres' : 'Intended range'}</th>
                    <th className="pb-3 font-medium">{locale === 'pl' ? 'Powód' : 'Reason'}</th>
                    <th className="pb-3 font-medium">{locale === 'pl' ? 'Sesja' : 'Session'}</th>
                    <th className="pb-3 font-medium text-right">{locale === 'pl' ? 'Akcja' : 'Action'}</th>
                  </tr>
                </thead>
                <tbody>
                  {pendingReopenItems.map((it) => (
                    <tr key={it.rebalance_session_id} className="border-b last:border-0">
                      <td className="py-3 text-sm">
                        {it.old_position ? (
                          <Link to={`/positions/closed/${it.old_position}`} className="font-mono hover:text-primary">
                            {shortenAddress(it.old_position)}
                          </Link>
                        ) : (
                          <span className="text-muted-foreground">—</span>
                        )}
                      </td>
                      <td className="py-3 text-sm text-muted-foreground">
                        {it.token_a_label && it.token_b_label
                          ? `${it.token_a_label} / ${it.token_b_label}`
                          : it.pool_address
                            ? (poolLabelByAddress.get(it.pool_address) ?? shortenAddress(it.pool_address))
                            : '—'}
                      </td>
                      <td className="py-3 text-sm text-muted-foreground">{it.close_ts_utc ?? '—'}</td>
                      <td className="py-3 text-sm text-muted-foreground">
                        {it.intended_tick_lower != null && it.intended_tick_upper != null
                          ? `${it.intended_tick_lower} → ${it.intended_tick_upper}`
                          : '—'}
                      </td>
                      <td className="py-3 text-sm text-muted-foreground">
                        {normalizePendingReopenReason(it.reason ?? it.note, locale)}
                      </td>
                      <td className="py-3 text-xs font-mono text-muted-foreground">
                        {shortenAddress(it.rebalance_session_id)}
                      </td>
                      <td className="py-3 text-right">
                        <Button
                          type="button"
                          variant="outline"
                          size="sm"
                          className="h-7 px-2 text-[11px]"
                          disabled={dismissStrandedM.isPending}
                          onClick={() => dismissStrandedM.mutate(it.rebalance_session_id)}
                        >
                          {dismissStrandedM.isPending ? t('positions.removing') : t('positions.remove')}
                        </Button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            </div>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>{t('positions.onchainTitle')}</CardTitle>
          <p className="text-sm text-muted-foreground font-normal">
            {locale === 'pl' ? (
              <>
                Skan NFT Whirlpool dla portfela — to samo co <code className="text-[11px]">orca-positions-list</code>. Wymaga
                działającego RPC w API; nie używa monitora strategii.
              </>
            ) : (
              <>
                Whirlpool NFT scan for wallet — same as <code className="text-[11px]">orca-positions-list</code>. Requires
                working API RPC; does not use strategy monitor.
              </>
            )}
          </p>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex flex-col gap-3 sm:flex-row sm:items-end">
            <div className="flex-1 space-y-1">
              <label className="text-xs text-muted-foreground">Owner (base58)</label>
              <input
                className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm font-mono"
                value={ownerInput}
                onChange={(e) => setOwnerInput(e.target.value)}
                placeholder="Wklej pubkey portfela"
              />
            </div>
            <Button
              type="button"
              variant="secondary"
              onClick={() => setAppliedOwner(ownerInput.trim())}
            >
              {t('positions.loadOnchain')}
            </Button>
            <Button
              type="button"
              variant="outline"
              onClick={() => chainQ.refetch()}
              disabled={!appliedOwner.trim()}
            >
              <RefreshCw className="h-4 w-4 mr-2" />
              {t('positions.refresh')}
            </Button>
          </div>
          {chainQ.isLoading ? (
            <div className="text-center py-6 text-muted-foreground">
              {locale === 'pl' ? 'Ładowanie RPC…' : 'Loading RPC…'}
            </div>
          ) : chainQ.error ? (
            <ErrorBanner>{(chainQ.error as Error).message}</ErrorBanner>
          ) : !appliedOwner.trim() ? (
            <div className="text-muted-foreground text-sm">Podaj owner i kliknij „Load on-chain”.</div>
          ) : (
            <>
              <p className="text-xs text-muted-foreground">
                RPC: <code className="break-all">{chainQ.data?.rpc_url ?? '—'}</code> — {locale === 'pl' ? 'znaleziono' : 'found'}:{' '}
                <strong>{chainQ.data?.total ?? 0}</strong>
              </p>
              {(chainQ.data?.entries?.length ?? 0) === 0 ? (
                <div className="text-muted-foreground text-sm py-4">
                  {locale === 'pl' ? 'Brak pozycji Whirlpool dla tego ownera.' : 'No Whirlpool positions for this owner.'}
                </div>
              ) : (
                <div className="overflow-x-auto">
                  <table className="w-full">
                    <thead>
                      <tr className="border-b text-left text-sm text-muted-foreground">
                        <th className="pb-3 font-medium">Kind</th>
                        <th className="pb-3 font-medium">Pair (mints · USD)</th>
                        <th className="pb-3 font-medium">Whirlpool</th>
                        <th className="pb-3 font-medium">Range (in / out)</th>
                        <th className="pb-3 font-medium text-right">Liquidity (raw)</th>
                      </tr>
                    </thead>
                    <tbody>
                      {chainQ.data!.entries.map((row) => (
                        <tr key={row.position_address} className="border-b last:border-0">
                          <td className="py-3 text-xs">{row.kind}</td>
                          <td className="py-3 text-xs max-w-[14rem]">
                            <Link
                              to={`/positions/${row.position_address}`}
                              className="block hover:text-primary space-y-1"
                            >
                              <PoolPairLabels
                                labelA={row.token_a_label}
                                labelB={row.token_b_label}
                                mintA={row.token_mint_a}
                                mintB={row.token_mint_b}
                                priceA={row.token_price_a_usd}
                                priceB={row.token_price_b_usd}
                              />
                              {row.token_a_label && row.token_b_label ? (
                                <div className="text-[11px] text-muted-foreground font-mono">
                                  PDA {shortenAddress(row.position_address)}
                                </div>
                              ) : (
                                <div className="font-mono font-medium">{shortenAddress(row.position_address)}</div>
                              )}
                              {row.position_bundle_address ? (
                                <span className="block text-muted-foreground mt-0.5 text-[10px]">
                                  bundle {shortenAddress(row.position_bundle_address)}
                                </span>
                              ) : null}
                            </Link>
                          </td>
                          <td className="py-3 text-muted-foreground font-mono text-xs">
                            {shortenAddress(row.pool_address)}
                          </td>
                          <td className="py-3">
                            <div className="space-y-0.5">
                              <span className={`text-sm block ${rangeCellClass(row.in_range)}`}>
                                {formatUsdcPriceRange(
                                  row.range_lower_usdc ?? undefined,
                                  row.range_upper_usdc ?? undefined,
                                  row.range_usdc_quote ?? undefined,
                                ) ??
                                  formatInvertedTokenPriceRange(
                                    row.range_lower_price ?? undefined,
                                    row.range_upper_price ?? undefined,
                                    row.range_price_quote ?? undefined,
                                  ) ??
                                  `${row.tick_lower} → ${row.tick_upper}`}
                              </span>
                              <span className="text-[11px] text-muted-foreground">
                                {rangeStatusLabel(row.in_range, locale)}
                              </span>
                            </div>
                          </td>
                          <td className="py-3 text-right font-mono text-xs">{row.liquidity}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
            </>
          )}
        </CardContent>
      </Card>
    </div>
  )
}
