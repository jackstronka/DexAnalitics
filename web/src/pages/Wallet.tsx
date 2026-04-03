import { useQuery, useQueries } from '@tanstack/react-query'
import { useState, useMemo } from 'react'
import { Link } from 'react-router-dom'
import { DollarSign, TrendingDown, TrendingUp, Wallet as WalletIcon, ArrowRight, Copy } from 'lucide-react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import ApiDataHint from '@/components/ApiDataHint'
import {
  getOrcaPositionsByOwner,
  getPortfolioAnalytics,
  getPositions,
  getJupiterPricesUsd,
  getWalletBalances,
  getWallets,
  getOrcaToken,
} from '@/lib/api'
import { getDevWalletPubkey } from '@/lib/devWallet'
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
  const devPk = getDevWalletPubkey()
  const [selectedId, setSelectedId] = useState<string>(() => {
    if (typeof window === 'undefined') return ''
    return window.localStorage.getItem(LS_SELECTED_WALLET_ID) || ''
  })
  const [showZeroTokens, setShowZeroTokens] = useState(false)

  const { data: wallets } = useQuery({
    queryKey: ['wallets'],
    queryFn: getWallets,
    staleTime: 30_000,
  })

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
    queryFn: () => getWalletBalances(ownerPk!),
    enabled: !!ownerPk,
    staleTime: 20_000,
  })
  const balances = balancesQuery.data
  const bLoad = balancesQuery.isLoading
  const bErr = balancesQuery.isError
  const bError = balancesQuery.error

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
          Wallet
        </h1>
        <p className="text-muted-foreground text-sm mt-1">
          Dwie warstwy danych: (1) on-chain saldo dla wybranego portfela (read-only RPC), (2) agregaty USD z{' '}
          <code className="text-xs">/analytics/portfolio</code> (monitor pozycji) — mogą być 0, jeśli monitor nie ma pozycji.
        </p>
      </div>

      <ApiDataHint />

      <Card>
        <CardHeader>
          <CardTitle>Portfele (pliki keypair na hoście API)</CardTitle>
        </CardHeader>
        <CardContent className="space-y-3">
          <p className="text-xs text-muted-foreground">
            Lista jest czytana z katalogu <code className="text-[11px]">CLMM_WALLETS_DIR</code> na hoście API. Jeden plik
            JSON = jeden wpis w UI. Jeśli katalog jest pusty, fallback to{' '}
            <code className="text-[11px]">VITE_DEV_WALLET_PUBKEY</code>.
          </p>
          {(wallets?.wallets ?? []).length === 0 ? (
            <div className="rounded-md border bg-muted/20 px-3 py-2 text-xs text-muted-foreground">
              Brak portfeli w katalogu <code className="text-[11px]">{wallets?.wallets_dir ?? 'wallets/'}</code>. Dodaj
              pliki <code className="text-[11px]">*.json</code> (keypair), ustaw{' '}
              <code className="text-[11px]">CLMM_WALLETS_DIR</code> w root <code className="text-[11px]">.env</code> i
              zrestartuj API.
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
                </Button>
              ))}
            </div>
          )}
          {ownerPk ? (
            <div className="rounded-md border px-3 py-2 text-xs">
              <div className="text-muted-foreground">Aktualny portfel</div>
              <div className="mt-1 flex flex-wrap items-center gap-2">
                <span className="font-mono break-all">{ownerPk}</span>
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  className="h-7 px-2"
                  onClick={() => copyText(ownerPk)}
                  title="Kopiuj pubkey"
                >
                  <Copy className="h-3.5 w-3.5 mr-1" />
                  Kopiuj
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
            <CardTitle>Saldo on-chain (read-only)</CardTitle>
          </CardHeader>
          <CardContent className="space-y-3">
            {bLoad && <div className="text-muted-foreground text-sm">Ładowanie salda…</div>}
            {bErr && (
              <div className="rounded-md border border-destructive/50 bg-destructive/5 px-3 py-2 text-xs text-destructive">
                Nie udało się pobrać salda z RPC: {(bError as Error)?.message ?? 'unknown error'}
              </div>
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
                    Tokeny SPL: <strong className="text-foreground">{balances.tokens.length}</strong>
                  </div>
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    className="h-8"
                    onClick={() => setShowZeroTokens((v) => !v)}
                    title="Pokaż/ukryj tokeny z zerowym balansem"
                  >
                    {showZeroTokens ? 'Ukryj zera' : 'Pokaż zera'}
                  </Button>
                </div>

                {balances.tokens.length === 0 ? (
                  <div className="text-muted-foreground text-xs">Brak tokenów SPL (lub brak dostępu RPC).</div>
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

      <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
        <Card>
          <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
            <CardTitle className="text-sm font-medium">Total value</CardTitle>
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
            <CardTitle className="text-sm font-medium">Net PnL</CardTitle>
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
            <CardTitle className="text-sm font-medium">Fees (USD)</CardTitle>
          </CardHeader>
          <CardContent>
            <div className="text-2xl font-bold text-green-600">
              {aLoad ? '…' : formatUSD(analytics?.total_fees_usd || '0')}
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
            <CardTitle className="text-sm font-medium">IL (avg %)</CardTitle>
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
          <CardTitle>Open positions</CardTitle>
          <Link to="/positions">
            <Button variant="ghost" size="sm">
              All positions <ArrowRight className="ml-2 h-4 w-4" />
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
