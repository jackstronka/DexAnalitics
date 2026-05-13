import { useMutation, useQuery, useQueries, useQueryClient } from '@tanstack/react-query'
import { useState, useMemo, useEffect } from 'react'
import { Link } from 'react-router-dom'
import {
  DollarSign,
  TrendingDown,
  TrendingUp,
  Wallet as WalletIcon,
  ArrowRight,
  Copy,
  HelpCircle,
} from 'lucide-react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { ErrorBanner } from '@/components/ui/error-banner'
import { InlineError } from '@/components/ui/inline-error'
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip'
import ApiDataHint from '@/components/ApiDataHint'
import {
  getOrcaPositionsByOwner,
  getPortfolioAnalytics,
  getPositions,
  getJupiterPricesUsd,
  getWalletEffectiveBalances,
  getWallets,
  getActiveSigner,
  createWallet,
  setActiveSigner,
  transferSol,
  getWalletTransfers,
  getWalletConvertOps,
  getWalletWsStatus,
  getOrcaToken,
} from '@/lib/api'
import { getDevWalletPubkey } from '@/lib/devWallet'
import { useI18n } from '@/lib/i18n'
import { formatUSD, formatPercent, shortenAddress, formatUsdcPriceRange } from '@/lib/utils'

const LS_SELECTED_WALLET_ID = 'clmm.selected_wallet_id'

/** Round to whole cents so table rows + SOL line always add up to the footer total. */
function usdToCents(n: number): number {
  if (!Number.isFinite(n)) return 0
  return Math.round(n * 100)
}

function copyText(text: string) {
  void navigator.clipboard.writeText(text)
}

export default function Wallet() {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const devPk = getDevWalletPubkey()
  const [selectedId, setSelectedId] = useState<string>(() => {
    if (typeof window === 'undefined') return ''
    return window.localStorage.getItem(LS_SELECTED_WALLET_ID) || ''
  })
  const [showZeroTokens, setShowZeroTokens] = useState(false)
  const [walletAutoRetryCount, setWalletAutoRetryCount] = useState(0)
  const [newWalletId, setNewWalletId] = useState('')
  const [transferTo, setTransferTo] = useState('')
  const [transferLamports, setTransferLamports] = useState('1000000')
  const [transferSolText, setTransferSolText] = useState('0.001')
  const [transferAmountLastEdited, setTransferAmountLastEdited] = useState<'lamports' | 'sol'>('lamports')
  /** Sender for SOL transfer (independent of `selectedId` used for balance / Orca view). */
  const [transferFromWalletId, setTransferFromWalletId] = useState('')
  /** `__custom` = paste pubkey; otherwise a `wallet id` from the list. */
  const [transferDestChoice, setTransferDestChoice] = useState('__custom')

  const { data: wallets } = useQuery({
    queryKey: ['wallets'],
    queryFn: getWallets,
    staleTime: 30_000,
  })
  const { data: activeSigner } = useQuery({
    queryKey: ['active-signer'],
    queryFn: getActiveSigner,
    staleTime: 10_000,
  })

  const createWalletM = useMutation({
    mutationFn: createWallet,
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['wallets'] })
    },
  })

  const setActiveSignerM = useMutation({
    mutationFn: setActiveSigner,
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['active-signer'] })
      void queryClient.invalidateQueries({ queryKey: ['api-signer-wallet'] })
    },
  })

  const transferSolM = useMutation({
    mutationFn: transferSol,
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ['wallet-transfers'] })
      // balance may change for current selected wallet as well
      await queryClient.invalidateQueries({ queryKey: ['wallet-balances'] })
    },
  })

  const transfersQuery = useQuery({
    queryKey: ['wallet-transfers'],
    queryFn: () => getWalletTransfers(20),
    staleTime: 10_000,
  })

  useEffect(() => {
    const list = wallets?.wallets ?? []
    if (list.length === 0) return
    setTransferFromWalletId((prev) =>
      prev && list.some((w) => w.id === prev) ? prev : list[0].id,
    )
  }, [wallets?.wallets])

  useEffect(() => {
    if (transferDestChoice !== '__custom' && transferDestChoice === transferFromWalletId) {
      setTransferDestChoice('__custom')
    }
  }, [transferFromWalletId, transferDestChoice])

  const transferRecipientPubkey = useMemo(() => {
    if (transferDestChoice === '__custom') return transferTo.trim()
    const row = wallets?.wallets?.find((x) => x.id === transferDestChoice)
    return row?.pubkey?.trim() ?? ''
  }, [transferDestChoice, transferTo, wallets?.wallets])

  const transferDestOptions = useMemo(() => {
    const list = wallets?.wallets ?? []
    return list.filter((w) => w.id !== transferFromWalletId)
  }, [wallets?.wallets, transferFromWalletId])

  const transferLamportsN = Number.parseInt(transferLamports, 10)
  const transferLamportsOk = Number.isFinite(transferLamportsN) && transferLamportsN > 0
  const transferMin = wallets?.transfer_min_lamports ?? 1_000_000
  const transferMax = wallets?.transfer_max_lamports ?? null
  const transferWithinMin = transferLamportsOk && transferLamportsN >= transferMin
  const transferWithinMax = !transferLamportsOk ? false : transferMax == null ? true : transferLamportsN <= transferMax
  const transferWithinLimits = transferWithinMin && transferWithinMax
  const transferSolUi =
    transferLamportsOk ? (transferLamportsN / 1_000_000_000).toFixed(9).replace(/0+$/, '').replace(/\.$/, '') : ''

  function setLamportsFromSol(sol: number) {
    const lamports = Math.round(sol * 1_000_000_000)
    setTransferAmountLastEdited('lamports')
    setTransferLamports(String(lamports))
    setTransferSolText(sol.toString())
  }

  useEffect(() => {
    if (transferAmountLastEdited === 'sol') return
    setTransferSolText(transferSolUi || '')
  }, [transferLamports, transferSolUi, transferAmountLastEdited])

  const selectedWallet = wallets?.wallets.find((w) => w.id === selectedId) ?? null
  const ownerPk = selectedWallet?.pubkey ?? devPk ?? null

  const { data: analytics, isLoading: aLoad } = useQuery({
    queryKey: ['portfolio-analytics'],
    queryFn: getPortfolioAnalytics,
  })

  const { data: positionsData, isLoading: pLoad } = useQuery({
    queryKey: ['positions'],
    queryFn: getPositions,
  })

  const { data: onChain } = useQuery({
    queryKey: ['orca-positions-by-owner', ownerPk ?? ''],
    queryFn: () => getOrcaPositionsByOwner(ownerPk!),
    enabled: !!ownerPk,
    staleTime: 60_000,
  })

  const balancesQuery = useQuery({
    queryKey: ['wallet-balances', ownerPk ?? ''],
    queryFn: () => getWalletEffectiveBalances(ownerPk!),
    enabled: !!ownerPk,
    staleTime: 20_000,
    refetchInterval: 10_000,
  })
  const recentConvertOpsQ = useQuery({
    queryKey: ['wallet-convert-ops', ownerPk ?? ''],
    queryFn: () => getWalletConvertOps({ owner: ownerPk ?? undefined, limit: 8 }),
    enabled: !!ownerPk,
    staleTime: 5_000,
    refetchInterval: 10_000,
  })
  const walletWsStatusQ = useQuery({
    queryKey: ['wallet-ws-status'],
    queryFn: getWalletWsStatus,
    staleTime: 5_000,
    refetchInterval: 10_000,
  })
  const wsDiag = walletWsStatusQ.data
  const wsHealth: 'healthy' | 'degraded' | 'critical' = !wsDiag
    ? 'degraded'
    : wsDiag.refresh_failures_total > 0
      ? 'critical'
      : wsDiag.reconnects_total > 0
        ? 'degraded'
        : 'healthy'
  const wsHealthClass =
    wsHealth === 'healthy'
      ? 'border-green-500/40 bg-green-500/10 text-green-200'
      : wsHealth === 'degraded'
        ? 'border-amber-500/40 bg-amber-500/10 text-amber-200'
        : 'border-red-500/40 bg-red-500/10 text-red-200'
  // React Query may keep previous data briefly during owner switches; only render balances/warnings
  // when the payload matches the currently selected owner.
  const balancesRaw = balancesQuery.data
  const balances =
    balancesRaw && ownerPk && balancesRaw.owner?.trim() === ownerPk.trim() ? balancesRaw : null
  const bLoad = balancesQuery.isLoading
  const bErr = balancesQuery.isError
  const bError = balancesQuery.error
  const tokenLegacyOk = balances?.token_legacy_ok
  const token2022Ok = balances?.token_2022_ok
  const tokenReadErrors = [
    balances?.token_legacy_error ? `SPL legacy: ${balances.token_legacy_error}` : null,
    balances?.token_2022_error ? `Token-2022: ${balances.token_2022_error}` : null,
  ].filter(Boolean) as string[]
  const MAX_WALLET_AUTO_RETRIES = 4
  const shouldAutoRetryTokens =
    !!ownerPk &&
    !bLoad &&
    !bErr &&
    !!balances &&
    balances.tokens.length === 0 &&
    walletAutoRetryCount < MAX_WALLET_AUTO_RETRIES &&
    // avoid hammering RPC when both token reads are known to fail (403/429 etc.)
    !(tokenLegacyOk === false && token2022Ok === false && tokenReadErrors.length > 0)
  const hasPartialTokenData =
    !!balances &&
    ((tokenLegacyOk === false && token2022Ok === true) ||
      (tokenLegacyOk === true && token2022Ok === false))
  const tokenReadsBothFailed = tokenLegacyOk === false && token2022Ok === false

  useEffect(() => {
    setWalletAutoRetryCount(0)
  }, [ownerPk])

  useEffect(() => {
    if (!shouldAutoRetryTokens) return
    const id = window.setTimeout(() => {
      setWalletAutoRetryCount((c) => c + 1)
      void balancesQuery.refetch()
    }, 4500)
    return () => window.clearTimeout(id)
  }, [shouldAutoRetryTokens, balancesQuery])

  const WSOL_MINT = 'So11111111111111111111111111111111111111112'

  const priceMints = balances
    ? [WSOL_MINT, ...balances.tokens.map((t) => t.mint)]
    : [WSOL_MINT]

  const pricesQuery = useQuery({
    queryKey: ['jup-prices', ownerPk ?? '', ...(balances?.tokens ?? []).map((t) => t.mint)],
    queryFn: () => getJupiterPricesUsd(priceMints),
    enabled: !!balances,
    staleTime: 60_000,
  })

  const prices = pricesQuery.data ?? {}
  const solUsd = prices[WSOL_MINT] ?? 0
  const solUi = balances ? parseFloat(balances.sol) || 0 : 0
  const solValueUsd = solUsd > 0 ? solUi * solUsd : 0
  const solValueCents = solUsd > 0 ? usdToCents(solValueUsd) : 0

  const tokensTotalCents =
    balances?.tokens.reduce((acc, t) => {
      const p = prices[t.mint]
      if (p == null) return acc
      return acc + usdToCents((parseFloat(t.ui_amount) || 0) * p)
    }, 0) ?? 0

  const onChainTotalCents = solValueCents + tokensTotalCents

  const tokenRows = useMemo(() => {
    if (!balances) return []
    const list = showZeroTokens
      ? balances.tokens
      : balances.tokens.filter((t) => t.ui_amount !== '0' && t.ui_amount !== '0.0')
    return list.slice(0, 50)
  }, [balances, showZeroTokens])

  const orcaTokenQueries = useQueries({
    queries: tokenRows.map((t) => ({
      queryKey: ['orca-token', t.mint] as const,
      queryFn: () => getOrcaToken(t.mint),
      enabled: tokenRows.length > 0,
      staleTime: 60 * 60 * 1000,
    })),
  })

  const positions = positionsData?.positions ?? []
  const active = positions.filter((p) => p.status === 'active')

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-3xl font-bold flex items-center gap-2">
          <WalletIcon className="h-8 w-8" />
          {t('wallet.title')}
        </h1>
        <p className="text-muted-foreground text-sm mt-1">
          {t('wallet.subtitle')}
        </p>
      </div>

      <ApiDataHint />

      <Card>
        <CardHeader>
          <CardTitle>{t('wallet.walletsTitle')}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-3">
          <p className="text-xs text-muted-foreground">{t('wallet.walletsHint')}</p>
          {(wallets?.wallets ?? []).length === 0 ? (
            <div className="rounded-md border bg-muted/20 px-3 py-2 text-xs text-muted-foreground">
              {t('wallet.noWalletsHint')}
            </div>
          ) : (
            <div className="flex flex-wrap gap-2">
              {(wallets?.wallets ?? []).map((w) => (
                <Button
                  key={w.id}
                  type="button"
                  variant={selectedId === w.id ? 'default' : 'outline'}
                  size="sm"
                  onClick={() => {
                    window.localStorage.setItem(LS_SELECTED_WALLET_ID, w.id)
                    setSelectedId(w.id)
                  }}
                  title={`${w.filename}\n${w.pubkey}`}
                >
                  {w.id}
                  {w.replication_status && (
                    <span
                      className={`ml-2 rounded px-1 text-[10px] ${
                        w.replication_status === 'healthy'
                          ? 'bg-green-500/20 text-green-300'
                          : w.replication_status === 'conflict'
                            ? 'bg-red-500/20 text-red-300'
                            : 'bg-amber-500/20 text-amber-300'
                      }`}
                    >
                      {w.replication_status}
                    </span>
                  )}
                </Button>
              ))}
            </div>
          )}
          <div className="rounded-md border px-3 py-2 space-y-2">
            <div className="text-xs font-medium">Create wallet</div>
            <div className="flex flex-wrap items-center gap-2">
              <input
                value={newWalletId}
                onChange={(e) => setNewWalletId(e.target.value)}
                placeholder="wallet_ops_01"
                className="h-8 min-w-[14rem] rounded border bg-background px-2 text-xs"
              />
              <Button
                type="button"
                size="sm"
                onClick={() =>
                  createWalletM.mutate({
                    wallet_id: newWalletId.trim() || undefined,
                    force: false,
                  })
                }
                disabled={createWalletM.isPending}
              >
                {createWalletM.isPending ? 'Creating…' : 'Create'}
              </Button>
            </div>
            {createWalletM.isError && <InlineError>{(createWalletM.error as Error).message}</InlineError>}
            {createWalletM.data && (
              <div className="text-xs text-muted-foreground">
                Created: <strong className="text-foreground">{createWalletM.data.wallet.id}</strong> (
                {createWalletM.data.wallet.pubkey})
              </div>
            )}
          </div>
          <div className="rounded-md border px-3 py-2 space-y-2">
            <div className="text-xs font-medium">Active signer</div>
            <div className="text-xs text-muted-foreground">
              Current: <strong className="text-foreground">{activeSigner?.wallet_id ?? 'env fallback'}</strong>
              {activeSigner?.pubkey ? ` (${activeSigner.pubkey})` : ''}
            </div>
            <div className="flex flex-wrap gap-2">
              {(wallets?.wallets ?? []).map((w) => (
                <Button
                  key={`signer-${w.id}`}
                  type="button"
                  variant={activeSigner?.wallet_id === w.id ? 'default' : 'outline'}
                  size="sm"
                  onClick={() => setActiveSignerM.mutate({ wallet_id: w.id })}
                  disabled={setActiveSignerM.isPending}
                >
                  use {w.id}
                </Button>
              ))}
            </div>
            {setActiveSignerM.isError && <InlineError>{(setActiveSignerM.error as Error).message}</InlineError>}
          </div>
          <TooltipProvider delayDuration={200}>
            <div className="rounded-md border px-3 py-2 space-y-3">
            <div className="text-xs font-medium">Transfer SOL</div>
            <div className="grid gap-3 sm:grid-cols-2">
              <label className="flex flex-col gap-1 text-xs">
                <span className="text-muted-foreground">{t('wallet.transferFrom')}</span>
                <select
                  className="h-8 rounded border bg-background px-2 text-xs"
                  value={transferFromWalletId}
                  onChange={(e) => setTransferFromWalletId(e.target.value)}
                >
                  {(wallets?.wallets ?? []).map((w) => (
                    <option key={`src-${w.id}`} value={w.id}>
                      {w.id} ({shortenAddress(w.pubkey, 6)})
                    </option>
                  ))}
                </select>
              </label>
              <div className="flex flex-col gap-2">
                <label className="flex flex-col gap-1 text-xs">
                  <span className="text-muted-foreground">{t('wallet.transferTo')}</span>
                  <select
                    className="h-8 rounded border bg-background px-2 text-xs"
                    value={transferDestChoice}
                    onChange={(e) => setTransferDestChoice(e.target.value)}
                  >
                    <option value="__custom">{t('wallet.transferToCustom')}</option>
                    {transferDestOptions.map((w) => (
                      <option key={`dst-${w.id}`} value={w.id}>
                        {w.id} ({shortenAddress(w.pubkey, 6)})
                      </option>
                    ))}
                  </select>
                </label>
                {transferDestChoice === '__custom' && (
                  <input
                    value={transferTo}
                    onChange={(e) => setTransferTo(e.target.value)}
                    placeholder={t('wallet.recipientPubkey')}
                    className="h-8 w-full rounded border bg-background px-2 text-xs font-mono"
                  />
                )}
              </div>
            </div>
            <div className="flex flex-wrap items-end gap-2">
              <label className="flex flex-col gap-1 text-xs">
                <span className="flex items-center gap-1 text-muted-foreground">
                  {t('wallet.lamportsLabel')}
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <button
                        type="button"
                        className="inline-flex rounded p-0.5 text-muted-foreground hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                        aria-label={t('wallet.lamportsTooltip')}
                      >
                        <HelpCircle className="h-3.5 w-3.5 shrink-0" />
                      </button>
                    </TooltipTrigger>
                    <TooltipContent side="top" className="max-w-xs leading-snug">
                      {t('wallet.lamportsTooltip')}
                    </TooltipContent>
                  </Tooltip>
                </span>
                <input
                  value={transferLamports}
                  onChange={(e) => {
                    setTransferAmountLastEdited('lamports')
                    setTransferLamports(e.target.value)
                  }}
                  placeholder="lamports"
                  className="h-8 w-36 rounded border bg-background px-2 text-xs"
                />
              </label>
              <div className="flex flex-col gap-1 text-xs">
                <span className="text-muted-foreground">{t('wallet.solPreview')}</span>
                <input
                  value={transferSolText}
                  onChange={(e) => {
                    const next = e.target.value
                    setTransferAmountLastEdited('sol')
                    setTransferSolText(next)
                    const sol = Number.parseFloat(next)
                    if (Number.isFinite(sol)) {
                      const lamports = Math.round(sol * 1_000_000_000)
                      if (Number.isFinite(lamports)) setTransferLamports(String(lamports))
                    }
                  }}
                  placeholder="SOL"
                  inputMode="decimal"
                  className="h-8 w-36 rounded border bg-background px-2 font-mono text-xs"
                />
              </div>
              <div className="flex flex-col gap-1 text-xs">
                <span className="text-muted-foreground">{t('wallet.quickAmounts')}</span>
                <div className="flex gap-2">
                  <Button type="button" variant="outline" size="sm" className="h-8" onClick={() => setLamportsFromSol(0.01)}>
                    0.01
                  </Button>
                  <Button type="button" variant="outline" size="sm" className="h-8" onClick={() => setLamportsFromSol(0.1)}>
                    0.1
                  </Button>
                  <Button type="button" variant="outline" size="sm" className="h-8" onClick={() => setLamportsFromSol(1)}>
                    1
                  </Button>
                </div>
              </div>
              <Button
                type="button"
                size="sm"
                disabled={
                  transferSolM.isPending ||
                  !transferFromWalletId ||
                  !transferRecipientPubkey ||
                  !transferLamportsOk ||
                  !transferWithinLimits
                }
                onClick={() =>
                  transferSolM.mutate({
                    from_wallet_id: transferFromWalletId,
                    to_pubkey: transferRecipientPubkey,
                    lamports: transferLamportsN || 0,
                  })
                }
              >
                {transferSolM.isPending ? 'Sending…' : 'Send'}
              </Button>
            </div>
            {!transferLamportsOk && <InlineError>Podaj dodatnią liczbę lamportów.</InlineError>}
            {transferLamportsOk && !transferWithinMin && (
              <InlineError>
                Minimum: <span className="font-mono">{transferMin}</span> lamports (
                <span className="font-mono">{(transferMin / 1_000_000_000).toFixed(9).replace(/0+$/, '').replace(/\.$/, '')}</span> SOL)
              </InlineError>
            )}
            {transferLamportsOk && transferMax != null && !transferWithinMax && (
              <InlineError>
                Maximum: <span className="font-mono">{transferMax}</span> lamports (
                <span className="font-mono">{(transferMax / 1_000_000_000).toFixed(9).replace(/0+$/, '').replace(/\.$/, '')}</span> SOL)
              </InlineError>
            )}
            {transferSolM.isError && <InlineError>{(transferSolM.error as Error).message}</InlineError>}
            {transferSolM.data && (
              <div className="text-xs text-muted-foreground break-all">
                Signature: <span className="font-mono">{transferSolM.data.signature}</span>
              </div>
            )}
            <div className="rounded-md border bg-muted/10 px-3 py-2 text-xs space-y-2">
              <div className="font-medium">{t('wallet.transferHistory')}</div>
              {transfersQuery.isLoading ? (
                <div className="text-muted-foreground">loading…</div>
              ) : transfersQuery.isError ? (
                <div className="text-muted-foreground">—</div>
              ) : (transfersQuery.data?.transfers?.length ?? 0) === 0 ? (
                <div className="text-muted-foreground">—</div>
              ) : (
                <div className="space-y-1">
                  {transfersQuery.data!.transfers.slice(0, 8).map((tr) => (
                    <div key={tr.signature} className="flex flex-wrap items-center justify-between gap-2">
                      <div className="text-muted-foreground">
                        <span className="font-mono">{tr.ts_utc.replace('T', ' ').replace('Z', '')}</span>{' '}
                        · <strong className="text-foreground">{tr.from_wallet_id}</strong> →{' '}
                        <span className="font-mono">{shortenAddress(tr.to_pubkey, 6)}</span>
                      </div>
                      <div className="font-mono">
                        {(tr.lamports / 1_000_000_000).toFixed(6)} SOL
                      </div>
                    </div>
                  ))}
                </div>
              )}
            </div>
            </div>
          </TooltipProvider>
          {ownerPk ? (
            <div className="rounded-md border px-3 py-2 text-xs">
              <div className="text-muted-foreground">{t('wallet.currentWallet')}</div>
              <div className="mt-1 flex flex-wrap items-center gap-2">
                <span className="font-mono break-all">{ownerPk}</span>
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  className="h-7 px-2"
                  onClick={() => copyText(ownerPk)}
                  title={t('wallet.copy')}
                >
                  <Copy className="h-3.5 w-3.5 mr-1" />
                  {t('wallet.copy')}
                </Button>
              </div>
              {onChain != null && (
                <div className="text-muted-foreground mt-2">
                  Whirlpool LP (scan RPC): <strong className="text-foreground">{onChain.total}</strong>
                </div>
              )}
            </div>
          ) : (
            <div className="text-xs text-muted-foreground">Brak wybranego portfela.</div>
          )}
        </CardContent>
      </Card>

      {(balances || bLoad || bErr) && (
        <Card>
          <CardHeader>
            <CardTitle>{t('wallet.onChainTitle')}</CardTitle>
          </CardHeader>
          <CardContent className="space-y-3">
            {bLoad && <div className="text-muted-foreground text-sm">{t('wallet.loading')}</div>}
            {bErr && (
              <ErrorBanner className="text-xs">
                Nie udało się pobrać salda z RPC: {(bError as Error)?.message ?? 'unknown error'}
              </ErrorBanner>
            )}
            {!balances && !bLoad && !bErr && (
              <div className="text-muted-foreground text-sm">Brak danych salda.</div>
            )}
            {balances && (
              <>
                <div className="grid gap-2 md:grid-cols-2">
                  <div className="rounded-md border bg-muted/20 px-3 py-2">
                    <div className="text-xs text-muted-foreground">SOL</div>
                    <div className="font-mono text-lg">{balances.sol}</div>
                    <div className="text-[11px] text-muted-foreground">lamports: {balances.lamports}</div>
                    <div className="text-[11px] text-muted-foreground">
                      USD (estimate):{' '}
                      {solUsd > 0 ? formatUSD(solValueCents / 100) : '—'}
                    </div>
                  </div>
                  <div className="rounded-md border bg-muted/20 px-3 py-2">
                    <div className="text-xs text-muted-foreground">RPC</div>
                    <div className="font-mono text-xs break-all">{balances.rpc_url}</div>
                    <div className="text-[11px] text-muted-foreground">
                      Ceny:{' '}
                      {pricesQuery.isLoading
                        ? 'loading…'
                        : pricesQuery.isError
                          ? 'error'
                          : 'API (Jupiter / fallback)'}
                    </div>
                  </div>
                </div>

                <div className="flex items-center justify-between gap-3">
                  <div className="text-xs text-muted-foreground">
                    {t('wallet.tokenCount')}: <strong className="text-foreground">{balances.tokens.length}</strong>
                    {typeof balances.token_accounts_total === 'number' && (
                      <span className="ml-2">
                        (konta tokenowe: <strong className="text-foreground">{balances.token_accounts_total}</strong>)
                      </span>
                    )}
                  </div>
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    className="h-8"
                    onClick={() => setShowZeroTokens((v) => !v)}
                    title="Pokaż/ukryj tokeny z zerowym balansem"
                  >
                    {showZeroTokens ? t('wallet.hideZeros') : t('wallet.showZeros')}
                  </Button>
                </div>
                {(hasPartialTokenData || tokenReadErrors.length > 0) && (
                  <div
                    className={
                      tokenReadsBothFailed
                        ? 'rounded-md border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-xs text-amber-200 space-y-1'
                        : 'rounded-md border border-blue-500/40 bg-blue-500/10 px-3 py-2 text-xs text-blue-100 space-y-1'
                    }
                  >
                    <div>
                      {tokenReadsBothFailed
                        ? 'Lista tokenów może być niepełna: oba odczyty RPC dla programów tokenów nie powiodły się.'
                        : 'Częściowy status odczytu tokenów: lista legacy jest dostępna, a odczyt token-2022 nie powiódł się.'}
                    </div>
                    <div className="text-[11px] opacity-90">
                      Status: legacy={String(tokenLegacyOk)} | token-2022={String(token2022Ok)}
                    </div>
                    {tokenReadErrors.length > 0 && (
                      <div className="text-[11px] opacity-90 break-all">
                        {tokenReadErrors.join(' | ')}
                      </div>
                    )}
                  </div>
                )}
                {balances?.confidence && (
                  <div className="text-[11px] text-muted-foreground">
                    confidence: <span className="font-mono">{balances.confidence}</span>
                    {typeof balances.pending_ops_count === 'number' ? ` · pending ops: ${balances.pending_ops_count}` : ''}
                    {balances.is_stale ? ` · stale: ${(balances.stale_age_ms / 1000).toFixed(1)}s` : ''}
                  </div>
                )}
                {balances?.is_stale && (
                  <div className="rounded-md border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-xs text-amber-200">
                    Ostatni znany stan (stale). Trwa odświeżanie danych w tle.
                  </div>
                )}

                {balances.tokens.length === 0 ? (
                  <div className="text-muted-foreground text-xs space-y-1">
                    <div>{t('wallet.noTokens')}</div>
                    {shouldAutoRetryTokens && (
                      <div>
                        Ponawiam pobieranie tokenów SPL automatycznie… próba {walletAutoRetryCount + 1}/{MAX_WALLET_AUTO_RETRIES}.
                      </div>
                    )}
                  </div>
                ) : (
                  <div className="overflow-x-auto">
                    <table className="w-full text-xs">
                      <thead>
                        <tr className="border-b text-left text-muted-foreground">
                          <th className="py-2 pr-3">Token</th>
                          <th className="py-2 pr-3 text-right">UI amount</th>
                          <th className="py-2 pr-3 text-right">Price USD</th>
                          <th className="py-2 pr-3 text-right">Value USD</th>
                          <th className="py-2">Akcje</th>
                        </tr>
                      </thead>
                      <tbody>
                        {tokenRows.map((t, rowIdx) => {
                            const orca = orcaTokenQueries[rowIdx]?.data
                            const orcaPending = orcaTokenQueries[rowIdx]?.isPending
                            const primary =
                              orca?.symbol?.trim() ||
                              orca?.name?.trim() ||
                              (orcaPending ? '…' : shortenAddress(t.mint, 8))
                            const p = prices[t.mint]
                            const lineCents =
                              p != null
                                ? usdToCents((parseFloat(t.ui_amount) || 0) * p)
                                : null
                            return (
                            <tr key={t.mint} className="border-b border-border/60">
                              <td className="py-2 pr-3 align-top" title={t.mint}>
                                <div className="font-medium text-foreground leading-tight">{primary}</div>
                                <div className="text-[10px] text-muted-foreground font-mono mt-0.5 break-all">
                                  {t.mint}
                                </div>
                              </td>
                              <td className="py-2 pr-3 text-right font-mono">{t.ui_amount}</td>
                              <td className="py-2 pr-3 text-right font-mono">
                                {p != null ? p.toFixed(4) : '—'}
                              </td>
                              <td className="py-2 pr-3 text-right font-mono">
                                {lineCents != null ? formatUSD(lineCents / 100) : '—'}
                              </td>
                              <td className="py-2">
                                <Button
                                  type="button"
                                  variant="ghost"
                                  size="sm"
                                  className="h-7 px-2"
                                  onClick={() => copyText(t.mint)}
                                  title="Kopiuj mint"
                                >
                                  <Copy className="h-3.5 w-3.5 mr-1" />
                                  Kopiuj
                                </Button>
                              </td>
                            </tr>
                            )
                          })}
                      </tbody>
                    </table>
                    {balances.tokens.length > 50 && (
                      <div className="text-xs text-muted-foreground mt-2">
                        Pokazano 50 pierwszych. Reszta: {balances.tokens.length - 50}.
                      </div>
                    )}
                    <div className="text-xs text-muted-foreground mt-2 space-y-1">
                      <div>
                        Suma on-chain USD (estimate):{' '}
                        <strong className="text-foreground">
                          {formatUSD(onChainTotalCents / 100)}
                        </strong>
                      </div>
                      <div className="text-[11px] pl-2 border-l border-border/60 space-y-0.5">
                        <div>
                          — SOL:{' '}
                          <span className="text-foreground font-medium">
                            {solUsd > 0 ? formatUSD(solValueCents / 100) : '—'}
                          </span>
                        </div>
                        <div>
                          — Tokeny SPL (wszystkie):{' '}
                          <span className="text-foreground font-medium">
                            {formatUSD(tokensTotalCents / 100)}
                          </span>
                        </div>
                      </div>
                      <span className="block text-[10px] mt-1 opacity-80 leading-relaxed">
                        Razem = SOL (pole powyżej) + wszystkie tokeny z listy RPC. Kolumna „Value USD” w tabeli to
                        tylko tokeny; nie zawiera SOL. Ukryte wiersze / limit 50 wierszy nie zmieniają sumy.
                      </span>
                    </div>
                  </div>
                )}
              </>
            )}
          </CardContent>
        </Card>
      )}

      <Card>
        <CardHeader>
          <CardTitle>{t('wallet.title')}: Ostatnie operacje konwersji</CardTitle>
        </CardHeader>
        <CardContent>
          {recentConvertOpsQ.isLoading ? (
            <div className="text-sm text-muted-foreground">Ładowanie…</div>
          ) : recentConvertOpsQ.isError ? (
            <InlineError as="div">Nie udało się pobrać operacji konwersji.</InlineError>
          ) : (recentConvertOpsQ.data?.length ?? 0) === 0 ? (
            <div className="text-sm text-muted-foreground">Brak operacji.</div>
          ) : (
            <div className="space-y-2">
              {recentConvertOpsQ.data!.map((op) => (
                <div key={op.op_id} className="rounded-md border border-border/70 p-2 text-xs">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="font-mono">{op.op_id.slice(0, 12)}…</span>
                    <span className="text-muted-foreground">{op.direction}</span>
                    <span className="text-muted-foreground">
                      {op.amount_raw / 1e9} {op.direction === 'native_to_wsol' ? 'SOL' : 'WSOL'}
                    </span>
                    <span className="rounded border border-border px-1.5 py-0.5 font-mono">
                      {op.reconciliation_status}
                    </span>
                  </div>
                  {op.reason_code ? (
                    <div className="mt-1 text-muted-foreground">
                      reason: <span className="font-mono">{op.reason_code}</span> · attempts: {op.attempts}
                    </div>
                  ) : null}
                </div>
              ))}
            </div>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            {t('wallet.title')}: Diagnostyka WS
            <span className={`rounded border px-2 py-0.5 text-[11px] uppercase tracking-wide ${wsHealthClass}`}>
              {wsHealth}
            </span>
          </CardTitle>
        </CardHeader>
        <CardContent>
          {walletWsStatusQ.isLoading ? (
            <div className="text-sm text-muted-foreground">Ładowanie…</div>
          ) : walletWsStatusQ.isError ? (
            <InlineError as="div">Nie udało się pobrać statusu subskrypcji WS.</InlineError>
          ) : (
            <div className="space-y-3 text-xs">
              <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-4">
                <div className="rounded-md border border-border/70 p-2">
                  <div className="text-muted-foreground">Owners monitored</div>
                  <div className="font-mono text-sm">
                    {walletWsStatusQ.data?.owners_monitored ?? 0}
                  </div>
                </div>
                <div className="rounded-md border border-border/70 p-2">
                  <div className="text-muted-foreground">Effective cache owners</div>
                  <div className="font-mono text-sm">{walletWsStatusQ.data?.effective_cache_owners ?? 0}</div>
                </div>
                <div
                  className={`rounded-md border p-2 ${
                    (walletWsStatusQ.data?.events_total ?? 0) > 0
                      ? 'border-green-500/40 bg-green-500/10'
                      : 'border-amber-500/40 bg-amber-500/10'
                  }`}
                >
                  <div className="text-muted-foreground">Events total</div>
                  <div className="font-mono text-sm">{walletWsStatusQ.data?.events_total ?? 0}</div>
                </div>
                <div
                  className={`rounded-md border p-2 ${
                    (walletWsStatusQ.data?.reconnects_total ?? 0) === 0
                      ? 'border-green-500/40 bg-green-500/10'
                      : 'border-amber-500/40 bg-amber-500/10'
                  }`}
                >
                  <div className="text-muted-foreground">Reconnects total</div>
                  <div className="font-mono text-sm">{walletWsStatusQ.data?.reconnects_total ?? 0}</div>
                </div>
                <div
                  className={`rounded-md border p-2 ${
                    (walletWsStatusQ.data?.refresh_failures_total ?? 0) === 0
                      ? 'border-green-500/40 bg-green-500/10'
                      : 'border-red-500/40 bg-red-500/10'
                  }`}
                >
                  <div className="text-muted-foreground">Refresh failures</div>
                  <div className="font-mono text-sm">{walletWsStatusQ.data?.refresh_failures_total ?? 0}</div>
                </div>
              </div>
              <div className="rounded-md border border-border/60 px-2 py-1">
                <div className="text-muted-foreground">Persistent wallet snapshot</div>
                <div className="font-mono break-all">
                  {walletWsStatusQ.data?.effective_cache_path ?? 'data/wallet-effective-cache.json'}
                </div>
                <div className="text-muted-foreground">
                  Last write: {walletWsStatusQ.data?.effective_cache_updated_at_utc ?? '—'}
                </div>
              </div>
              <div>
                <div className="mb-1 text-muted-foreground">Tracked owners</div>
                {(walletWsStatusQ.data?.owners?.length ?? 0) === 0 ? (
                  <div className="text-muted-foreground">Brak aktywnych ownerów.</div>
                ) : (
                  <div className="space-y-1">
                    {walletWsStatusQ.data?.owners.map((owner) => (
                      <div key={owner} className="font-mono break-all rounded border border-border/60 px-2 py-1">
                        {owner}
                      </div>
                    ))}
                  </div>
                )}
              </div>
            </div>
          )}
        </CardContent>
      </Card>

      <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
        <Card>
          <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
            <CardTitle className="text-sm font-medium">{t('wallet.totalValue')}</CardTitle>
            <DollarSign className="h-4 w-4 text-muted-foreground" />
          </CardHeader>
          <CardContent>
            <div className="text-2xl font-bold">
              {aLoad ? '…' : formatUSD(analytics?.total_value_usd || '0')}
            </div>
            <p className="text-xs text-muted-foreground">{analytics?.active_positions ?? 0} active positions</p>
          </CardContent>
        </Card>

        <Card>
          <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
            <CardTitle className="text-sm font-medium">{t('wallet.netPnl')}</CardTitle>
            {parseFloat(analytics?.total_pnl_pct || '0') >= 0 ? (
              <TrendingUp className="h-4 w-4 text-green-500" />
            ) : (
              <TrendingDown className="h-4 w-4 text-red-500" />
            )}
          </CardHeader>
          <CardContent>
            <div
              className={`text-2xl font-bold ${
                parseFloat(analytics?.total_pnl_pct || '0') >= 0 ? 'text-green-500' : 'text-red-500'
              }`}
            >
              {aLoad ? '…' : formatPercent(analytics?.total_pnl_pct || '0')}
            </div>
            <p className="text-xs text-muted-foreground">{formatUSD(analytics?.total_pnl_usd || '0')}</p>
          </CardContent>
        </Card>

        <Card>
          <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
            <CardTitle className="text-sm font-medium">{t('wallet.feesUsd')}</CardTitle>
          </CardHeader>
          <CardContent>
            <div className="text-2xl font-bold text-green-600">
              {aLoad ? '…' : formatUSD(analytics?.total_fees_usd || '0')}
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
            <CardTitle className="text-sm font-medium">{t('wallet.ilAvg')}</CardTitle>
          </CardHeader>
          <CardContent>
            <div className="text-2xl font-bold text-yellow-600">
              {aLoad ? '…' : formatPercent(analytics?.total_il_pct || '0')}
            </div>
          </CardContent>
        </Card>
      </div>

      <Card>
        <CardHeader className="flex flex-row items-center justify-between">
          <CardTitle>{t('wallet.openPositions')}</CardTitle>
          <Link to="/positions">
            <Button variant="ghost" size="sm">
              {t('wallet.allPositions')} <ArrowRight className="ml-2 h-4 w-4" />
            </Button>
          </Link>
        </CardHeader>
        <CardContent>
          {pLoad ? (
            <div className="text-muted-foreground">Loading…</div>
          ) : active.length === 0 ? (
            <div className="text-muted-foreground">No active positions.</div>
          ) : (
            <div className="space-y-3">
              {active.map((p) => (
                <Link
                  key={p.address}
                  to={`/positions/${p.address}`}
                  className="flex items-center justify-between p-4 rounded-lg border hover:bg-accent transition-colors"
                >
                  <div>
                    <div className="font-mono text-sm">{shortenAddress(p.pool_address, 6)}</div>
                    <div className="text-xs text-muted-foreground">
                      {formatUsdcPriceRange(
                        p.range_lower_usdc ?? undefined,
                        p.range_upper_usdc ?? undefined,
                        p.range_usdc_quote ?? undefined,
                      ) ?? `Ticks ${p.tick_lower} → ${p.tick_upper}`}{' '}
                      · {p.in_range ? 'in range' : 'out of range'}
                    </div>
                  </div>
                  <div className="text-right">
                    <div className="font-medium">{formatUSD(p.value_usd)}</div>
                    <div className={parseFloat(p.pnl.net_pnl_pct) >= 0 ? 'text-green-500' : 'text-red-500'}>
                      {formatPercent(p.pnl.net_pnl_pct)}
                    </div>
                  </div>
                </Link>
              ))}
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  )
}
