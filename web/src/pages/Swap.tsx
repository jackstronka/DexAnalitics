import { useEffect, useMemo, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Link } from 'react-router-dom'
import { ArrowLeftRight, ChevronDown, Copy, ExternalLink, RefreshCcw } from 'lucide-react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import {
  convertSol,
  type ConvertSolDirection,
  getApiSignerWallet,
  getJupiterPricesUsd,
  getPool,
  getSwapCostEstimate,
  getWalletBalances,
  getWallets,
  getOrcaToken,
  swapBeforeOpen,
} from '@/lib/api'
import { getDevWalletPubkey } from '@/lib/devWallet'
import { shortenAddress } from '@/lib/utils'

const LS_SELECTED_WALLET_ID = 'clmm.selected_wallet_id'
const WSOL_MINT = 'So11111111111111111111111111111111111111112'
const USDC_MINT = 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v'

type SwapProvider = 'jupiter' | 'orca'
type AmountMode = 'input' | 'usd'

type TokenOption = {
  mint: string
  ui: number
}

function buildJupiterSwapUrl(inputMint: string, outputMint: string, amountRaw?: number | null): string {
  const u = new URL('https://jup.ag/swap')
  u.searchParams.set('inputMint', inputMint)
  u.searchParams.set('outputMint', outputMint)
  if (amountRaw != null && amountRaw > 0) {
    u.searchParams.set('amount', String(Math.floor(amountRaw)))
  }
  return u.toString()
}

function copyText(text: string) {
  void navigator.clipboard.writeText(text)
}

function formatUi(n: number, maxFrac = 6): string {
  if (!Number.isFinite(n)) return '—'
  return n.toLocaleString(undefined, { maximumFractionDigits: maxFrac })
}

export default function Swap() {
  const devPk = getDevWalletPubkey()
  const queryClient = useQueryClient()

  const walletsQ = useQuery({
    queryKey: ['wallets'],
    queryFn: getWallets,
    staleTime: 30_000,
  })

  const ownerPk = useMemo(() => {
    if (typeof window === 'undefined') return null
    const id = window.localStorage.getItem(LS_SELECTED_WALLET_ID) || ''
    const picked = walletsQ.data?.wallets.find((w) => w.id === id)
    return picked?.pubkey ?? devPk ?? walletsQ.data?.wallets[0]?.pubkey ?? null
  }, [walletsQ.data?.wallets, devPk])

  const balancesQ = useQuery({
    queryKey: ['wallet-balances', ownerPk ?? ''],
    queryFn: () => getWalletBalances(ownerPk!),
    enabled: !!ownerPk,
    staleTime: 20_000,
  })

  const tokenOptions = useMemo(() => {
    const b = balancesQ.data
    const out: TokenOption[] = []
    const solUi = b ? parseFloat(b.sol) || 0 : 0
    out.push({ mint: WSOL_MINT, ui: solUi })
    for (const t of b?.tokens ?? []) {
      out.push({ mint: t.mint, ui: parseFloat(t.ui_amount) || 0 })
    }
    // Unique by mint
    const seen = new Set<string>()
    return out.filter((x) => {
      const k = x.mint
      if (seen.has(k)) return false
      seen.add(k)
      return true
    })
  }, [balancesQ.data])

  const [provider, setProvider] = useState<SwapProvider>('orca')
  const [poolAddress, setPoolAddress] = useState('Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE') // SOL/USDC 0.04%
  const [inputMint, setInputMint] = useState(WSOL_MINT)
  const [outputMint, setOutputMint] = useState(USDC_MINT)
  const [amountMode, setAmountMode] = useState<AmountMode>('usd')
  const [amountUi, setAmountUi] = useState<number | ''>('')
  const [amountUsd, setAmountUsd] = useState<number | ''>('')
  const [percent, setPercent] = useState<number>(0)
  const [swapSig, setSwapSig] = useState<string | null>(null)
  const [swapErr, setSwapErr] = useState<string | null>(null)
  const [convertDirection, setConvertDirection] = useState<ConvertSolDirection>('wsol_to_native')
  const [convertAmountUi, setConvertAmountUi] = useState<number | ''>('')
  const [convertSig, setConvertSig] = useState<string | null>(null)
  const [convertErr, setConvertErr] = useState<string | null>(null)

  const inputMetaQ = useQuery({
    queryKey: ['orca-token', inputMint],
    queryFn: () => getOrcaToken(inputMint),
    enabled: !!inputMint,
    staleTime: 60 * 60 * 1000,
  })

  const outputMetaQ = useQuery({
    queryKey: ['orca-token', outputMint],
    queryFn: () => getOrcaToken(outputMint),
    enabled: !!outputMint,
    staleTime: 60 * 60 * 1000,
  })

  const pricesQ = useQuery({
    queryKey: ['jupiter-prices', inputMint, outputMint],
    queryFn: () => getJupiterPricesUsd([inputMint, outputMint]),
    enabled: !!inputMint && !!outputMint,
    staleTime: 60_000,
  })

  const inputPriceUsd = pricesQ.data?.[inputMint] ?? 0

  const inputLabel = useMemo(() => {
    if (inputMint === WSOL_MINT) return 'SOL'
    const s = inputMetaQ.data?.symbol
    return s && s.trim() ? s : shortenAddress(inputMint, 4)
  }, [inputMetaQ.data?.symbol, inputMint])

  const outputLabel = useMemo(() => {
    if (outputMint === WSOL_MINT) return 'SOL'
    const s = outputMetaQ.data?.symbol
    return s && s.trim() ? s : shortenAddress(outputMint, 4)
  }, [outputMetaQ.data?.symbol, outputMint])

  const inputBalanceUi = useMemo(() => {
    const row = tokenOptions.find((t) => t.mint === inputMint)
    return row?.ui ?? 0
  }, [tokenOptions, inputMint])

  // Percent slider drives input amount (not USD) by balance.
  useEffect(() => {
    if (!Number.isFinite(percent) || percent <= 0) return
    const v = (inputBalanceUi * percent) / 100
    if (Number.isFinite(v)) {
      setAmountMode('input')
      setAmountUi(Number(v.toFixed(8)))
    }
  }, [percent, inputBalanceUi])

  // USD mode drives input amount based on Jupiter price.
  useEffect(() => {
    if (amountMode !== 'usd') return
    if (amountUsd === '' || !Number.isFinite(Number(amountUsd)) || Number(amountUsd) <= 0) return
    if (!(inputPriceUsd > 0)) return
    const ui = Number(amountUsd) / inputPriceUsd
    if (Number.isFinite(ui) && ui > 0) {
      setAmountUi(Number(ui.toFixed(8)))
    }
  }, [amountMode, amountUsd, inputPriceUsd])

  const amountRaw = useMemo(() => {
    if (amountUi === '' || !Number.isFinite(Number(amountUi)) || Number(amountUi) <= 0) return null
    const dec = inputMetaQ.data?.decimals ?? 9
    const mul = 10 ** dec
    const raw = Math.round(Number(amountUi) * mul)
    if (!Number.isFinite(raw) || raw <= 0 || raw > Number.MAX_SAFE_INTEGER) return null
    return raw
  }, [amountUi, inputMetaQ.data?.decimals])

  const jupUrl = useMemo(
    () => buildJupiterSwapUrl(inputMint, outputMint, provider === 'jupiter' ? amountRaw : amountRaw),
    [inputMint, outputMint, amountRaw, provider],
  )

  const canOpen = inputMint.trim() && outputMint.trim() && inputMint !== outputMint

  const curatedPools = useMemo(
    () => [
      { label: 'SOL/USDC (0.04%)', address: 'Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE' },
      { label: 'whETH/SOL (0.05%)', address: 'HktfL7iwGKT5QHjywQkcDnZXScoh811k7akrMZJkCcEF' },
      { label: 'cbBTC/USDC (0.04%)', address: 'HxA6SKW5qA4o12fjVgTpXdq2YnZ5Zv1s7SB4FFomsyLM' },
    ],
    [],
  )

  const poolQ = useQuery({
    queryKey: ['pool', poolAddress.trim()],
    queryFn: () => getPool(poolAddress.trim()),
    enabled: provider === 'orca' && poolAddress.trim().length > 0,
    staleTime: 30_000,
  })

  // In Orca mode, swap pair is dictated by the selected pool; keep mints in sync.
  useEffect(() => {
    if (provider !== 'orca') return
    const p = poolQ.data
    if (!p) return
    const a = p.token_mint_a
    const b = p.token_mint_b
    // If current input is one of the pool mints, keep direction; otherwise default to A->B.
    if (inputMint === a && outputMint === b) return
    if (inputMint === b && outputMint === a) return
    if (inputMint === a) {
      setOutputMint(b)
      return
    }
    if (inputMint === b) {
      setOutputMint(a)
      return
    }
    setInputMint(a)
    setOutputMint(b)
    setPercent(0)
    // keep user-entered amount if possible
  }, [provider, poolQ.data?.token_mint_a, poolQ.data?.token_mint_b])

  const swapCostQ = useQuery({
    queryKey: ['swap-cost-estimate', poolAddress.trim()],
    queryFn: () => getSwapCostEstimate(poolAddress.trim()),
    enabled: provider === 'orca' && poolAddress.trim().length > 0,
    staleTime: 60_000,
  })

  const apiSignerQ = useQuery({
    queryKey: ['api-signer-wallet'],
    queryFn: getApiSignerWallet,
    enabled: provider === 'orca',
    staleTime: 10_000,
    refetchInterval: 15_000,
  })

  const apiSignerBalancesQ = useQuery({
    queryKey: ['wallet-balances', apiSignerQ.data?.pubkey ?? ''],
    queryFn: () => getWalletBalances(apiSignerQ.data!.pubkey!),
    enabled: provider === 'orca' && !!apiSignerQ.data?.pubkey,
    staleTime: 10_000,
    refetchInterval: 15_000,
  })

  const swapMutation = useMutation({
    mutationFn: swapBeforeOpen,
    onSuccess: (data) => {
      setSwapErr(null)
      setSwapSig(data.swap_signature ?? null)
      queryClient.invalidateQueries({ queryKey: ['wallet-balances', ownerPk ?? ''] })
    },
    onError: (err) => {
      const msg = err instanceof Error ? err.message : String(err)
      setSwapErr(msg)
    },
  })

  const convertMutation = useMutation({
    mutationFn: convertSol,
    onSuccess: (data) => {
      setConvertErr(null)
      setConvertSig(data.signature ?? null)
      queryClient.invalidateQueries({ queryKey: ['wallet-balances', apiSignerQ.data?.pubkey ?? ''] })
      queryClient.invalidateQueries({ queryKey: ['api-signer-wallet'] })
    },
    onError: (err) => {
      const msg = err instanceof Error ? err.message : String(err)
      setConvertErr(msg)
    },
  })

  const tokenLabel = (mint: string) => {
    if (mint === WSOL_MINT) return 'SOL'
    if (mint === USDC_MINT) return 'USDC'
    // best-effort: try to use cached meta from current selections
    if (mint === inputMint) return inputLabel
    if (mint === outputMint) return outputLabel
    return shortenAddress(mint, 4)
  }

  const openInProviderUrl = useMemo(() => {
    if (!canOpen) return null
    if (provider === 'jupiter') return jupUrl
    return 'https://www.orca.so/'
  }, [canOpen, provider, jupUrl])

  const onClickMax = () => {
    setPercent(0)
    setAmountMode('input')
    setAmountUi(Number(inputBalanceUi.toFixed(8)))
  }
  const onClickHalf = () => {
    setPercent(0)
    setAmountMode('input')
    setAmountUi(Number((inputBalanceUi / 2).toFixed(8)))
  }

  const apiSignerNativeUi = useMemo(
    () => parseFloat(apiSignerBalancesQ.data?.sol ?? '0') || 0,
    [apiSignerBalancesQ.data?.sol],
  )
  const apiSignerWsolUi = useMemo(() => {
    const row = apiSignerBalancesQ.data?.tokens.find((t) => t.mint === WSOL_MINT)
    return parseFloat(row?.ui_amount ?? '0') || 0
  }, [apiSignerBalancesQ.data?.tokens])
  const convertSourceBalanceUi =
    convertDirection === 'native_to_wsol' ? apiSignerNativeUi : apiSignerWsolUi
  const convertAmountRaw = useMemo(() => {
    if (convertAmountUi === '' || !Number.isFinite(Number(convertAmountUi)) || Number(convertAmountUi) <= 0) {
      return null
    }
    const raw = Math.round(Number(convertAmountUi) * 1e9)
    if (!Number.isFinite(raw) || raw <= 0 || raw > Number.MAX_SAFE_INTEGER) return null
    return raw
  }, [convertAmountUi])
  const canConvert =
    provider === 'orca' &&
    !!apiSignerQ.data?.configured &&
    !!apiSignerQ.data?.pubkey &&
    convertAmountRaw != null &&
    Number(convertAmountUi) <= convertSourceBalanceUi &&
    !convertMutation.isPending

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between gap-4">
        <div className="flex items-center gap-3 min-w-0">
          <Link to="/wallet" className="shrink-0">
            <Button variant="ghost" size="icon">
              <ArrowLeftRight className="h-4 w-4" />
            </Button>
          </Link>
          <h1 className="text-3xl font-bold truncate">Swap</h1>
        </div>
        <div />
      </div>

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center justify-between gap-3">
            <span>Swap</span>
            <div className="flex items-center gap-2">
              <Button
                type="button"
                variant={provider === 'orca' ? 'default' : 'outline'}
                size="sm"
                onClick={() => setProvider('orca')}
              >
                Orca
              </Button>
              <Button
                type="button"
                variant={provider === 'jupiter' ? 'default' : 'outline'}
                size="sm"
                onClick={() => setProvider('jupiter')}
              >
                Jupiter
              </Button>
            </div>
          </CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="text-xs text-muted-foreground">
            {provider === 'orca'
              ? 'Tryb Orca: wykonuje swap ExactIn w wybranej puli Whirlpool przez backend (ten sam mechanizm co „swap-before-open”).'
              : 'Jupiter deep-link ma prefill mintów i kwoty (ExactIn).'}
          </div>

          {provider === 'orca' ? (
            <div className="rounded-lg border border-border bg-muted/10 p-3 space-y-2">
              <div className="flex flex-wrap items-center justify-between gap-2">
                <div className="text-sm font-medium">Pool</div>
                {swapCostQ.data ? (
                  <div className="text-xs text-muted-foreground">
                    Est. network fee ~{(swapCostQ.data.estimated_network_fee_lamports / 1e9).toFixed(6)} SOL
                  </div>
                ) : null}
              </div>
              <div className="grid gap-2 md:grid-cols-2">
                <select
                  className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                  value={poolAddress}
                  onChange={(e) => setPoolAddress(e.target.value)}
                >
                  {curatedPools.map((p) => (
                    <option key={p.address} value={p.address}>
                      {p.label}
                    </option>
                  ))}
                  <option value={poolAddress}>Custom…</option>
                </select>
                <input
                  className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm font-mono"
                  value={poolAddress}
                  onChange={(e) => setPoolAddress(e.target.value)}
                  placeholder="Whirlpool pool address"
                />
              </div>
              {poolQ.isLoading ? (
                <div className="text-[11px] text-muted-foreground">Loading pool meta…</div>
              ) : poolQ.data ? (
                <div className="text-[11px] text-muted-foreground">
                  Pair: {tokenLabel(poolQ.data.token_mint_a)} / {tokenLabel(poolQ.data.token_mint_b)} · tick_spacing{' '}
                  {poolQ.data.tick_spacing}
                </div>
              ) : poolQ.isError ? (
                <div className="text-[11px] text-destructive">Pool meta error: {(poolQ.error as Error).message}</div>
              ) : null}
              <div className="text-[11px] text-muted-foreground">
                Uwaga: backend swap wymaga skonfigurowanego walleta API (`KEYPAIR_PATH`) i SOL na opłaty/rent.
              </div>
            </div>
          ) : null}

          {provider === 'orca' ? (
            <div className="rounded-lg border border-border bg-muted/10 p-3 space-y-2">
              <div className="flex flex-wrap items-center justify-between gap-2">
                <div className="text-sm font-medium">API signer wallet</div>
                <div className="text-xs text-muted-foreground text-right">
                  min for swap: {(apiSignerQ.data?.min_swap_lamports ?? 0) / 1e9} SOL · open/rent:{' '}
                  {(apiSignerQ.data?.min_open_lamports ?? 0) / 1e9} SOL
                </div>
              </div>
              {apiSignerQ.isLoading ? (
                <div className="text-xs text-muted-foreground">Loading…</div>
              ) : apiSignerQ.data ? (
                <div className="space-y-1 text-xs">
                  <div className="text-muted-foreground">
                    configured:{' '}
                    <span className={apiSignerQ.data.configured ? 'text-foreground' : 'text-destructive'}>
                      {apiSignerQ.data.configured ? 'yes' : 'no'}
                    </span>
                  </div>
                  {apiSignerQ.data.pubkey ? (
                    <div className="text-muted-foreground">
                      pubkey: <span className="font-mono">{shortenAddress(apiSignerQ.data.pubkey, 6)}</span>{' '}
                      <button className="underline underline-offset-2" type="button" onClick={() => copyText(apiSignerQ.data.pubkey!)}>
                        copy
                      </button>
                    </div>
                  ) : null}
                  {apiSignerQ.data.sol ? (
                    <div className="text-muted-foreground">
                      SOL: <span className="font-mono tabular-nums">{apiSignerQ.data.sol}</span>
                    </div>
                  ) : null}
                  {apiSignerQ.data.note ? (
                    <div className="text-muted-foreground">{apiSignerQ.data.note}</div>
                  ) : null}
                  {apiSignerQ.data.configured &&
                  apiSignerQ.data.lamports != null &&
                  apiSignerQ.data.lamports < apiSignerQ.data.min_swap_lamports ? (
                    <div className="text-destructive">
                      Too little SOL for swap fees. Top up this wallet.
                    </div>
                  ) : null}
                </div>
              ) : (
                <div className="text-xs text-muted-foreground">—</div>
              )}
            </div>
          ) : null}

          {provider === 'orca' ? (
            <div className="rounded-lg border border-border bg-muted/10 p-3 space-y-3">
              <div className="flex flex-wrap items-center justify-between gap-2">
                <div className="text-sm font-medium">Convert WSOL &lt;-&gt; SOL</div>
                <div className="text-xs text-muted-foreground">
                  Konwersja techniczna 1:1 (bez swapu rynkowego).
                </div>
              </div>
              <div className="grid gap-2 md:grid-cols-[auto_auto_1fr_auto] items-end">
                <Button
                  type="button"
                  variant={convertDirection === 'wsol_to_native' ? 'default' : 'outline'}
                  size="sm"
                  onClick={() => setConvertDirection('wsol_to_native')}
                >
                  WSOL -&gt; SOL
                </Button>
                <Button
                  type="button"
                  variant={convertDirection === 'native_to_wsol' ? 'default' : 'outline'}
                  size="sm"
                  onClick={() => setConvertDirection('native_to_wsol')}
                >
                  SOL -&gt; WSOL
                </Button>
                <input
                  type="number"
                  step="0.000001"
                  className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                  value={convertAmountUi}
                  onChange={(e) => setConvertAmountUi(e.target.value === '' ? '' : Number(e.target.value))}
                  placeholder="Amount (SOL)"
                />
                <Button
                  type="button"
                  variant="secondary"
                  size="sm"
                  onClick={() => setConvertAmountUi(Number(convertSourceBalanceUi.toFixed(9)))}
                >
                  Max
                </Button>
              </div>
              <div className="text-xs text-muted-foreground">
                Source balance:{' '}
                <span className="font-mono tabular-nums">{formatUi(convertSourceBalanceUi, 9)}</span>{' '}
                {convertDirection === 'wsol_to_native' ? 'WSOL' : 'SOL'}
              </div>
              <div className="flex flex-wrap items-center gap-2">
                <Button
                  type="button"
                  disabled={!canConvert}
                  onClick={() => {
                    setConvertErr(null)
                    setConvertSig(null)
                    convertMutation.mutate({
                      direction: convertDirection,
                      amount_raw: convertAmountRaw ?? 0,
                    })
                  }}
                >
                  {convertMutation.isPending ? 'Converting…' : 'Convert now'}
                </Button>
                {convertDirection === 'wsol_to_native' ? (
                  <span className="text-[11px] text-muted-foreground">
                    Safe mode: aktualnie WSOL-&gt;SOL wspiera pełny unwrap (użyj Max).
                  </span>
                ) : null}
              </div>
              {(convertSig || convertErr) ? (
                <div className="rounded-lg border border-border bg-background/60 p-2 text-xs space-y-1">
                  {convertSig ? (
                    <div>
                      <span className="font-medium">Convert submitted:</span>{' '}
                      <span className="font-mono break-all">{convertSig}</span>
                    </div>
                  ) : null}
                  {convertErr ? (
                    <div className="text-destructive break-words">
                      <span className="font-medium">Convert failed:</span> {convertErr}
                    </div>
                  ) : null}
                </div>
              ) : null}
            </div>
          ) : null}

          <div className="grid gap-3 md:grid-cols-[1fr_auto_1fr] items-stretch">
            <div className="rounded-lg border border-border bg-muted/10 p-3 space-y-2">
              <div className="flex items-center justify-between gap-2">
                <div className="text-xs text-muted-foreground">You pay</div>
                <div className="text-xs text-muted-foreground">
                  Balance:{' '}
                  <button
                    type="button"
                    className="font-mono tabular-nums underline underline-offset-2 hover:opacity-90"
                    onClick={() => {
                      setAmountMode('input')
                      setPercent(0)
                      setAmountUi(Number(inputBalanceUi.toFixed(8)))
                    }}
                  >
                    {formatUi(inputBalanceUi, 8)}
                  </button>
                </div>
              </div>

              <div className="flex items-center justify-between gap-2">
                <div className="text-lg font-semibold">{tokenLabel(inputMint)}</div>
                <div className="relative">
                  <select
                    className="appearance-none rounded-md border border-input bg-background pl-3 pr-8 py-2 text-sm"
                    value={inputMint}
                    onChange={(e) => {
                      setPercent(0)
                      const next = e.target.value
                      if (provider === 'orca' && poolQ.data) {
                        const a = poolQ.data.token_mint_a
                        const b = poolQ.data.token_mint_b
                        if (next === a) {
                          setInputMint(a)
                          setOutputMint(b)
                          return
                        }
                        if (next === b) {
                          setInputMint(b)
                          setOutputMint(a)
                          return
                        }
                        return
                      }
                      setInputMint(next)
                    }}
                  >
                    {(provider === 'orca' && poolQ.data
                      ? tokenOptions.filter(
                          (t) => t.mint === poolQ.data.token_mint_a || t.mint === poolQ.data.token_mint_b,
                        )
                      : tokenOptions
                    ).map((t) => (
                      <option key={t.mint} value={t.mint}>
                        {tokenLabel(t.mint)} · {formatUi(t.ui, 8)}
                      </option>
                    ))}
                  </select>
                  <ChevronDown className="pointer-events-none absolute right-2 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground" />
                </div>
              </div>

              <div className="grid gap-2">
                <div className="flex items-center gap-2">
                  <Button
                    type="button"
                    variant={amountMode === 'input' ? 'default' : 'outline'}
                    size="sm"
                    onClick={() => setAmountMode('input')}
                  >
                    Amount
                  </Button>
                  <Button
                    type="button"
                    variant={amountMode === 'usd' ? 'default' : 'outline'}
                    size="sm"
                    onClick={() => setAmountMode('usd')}
                  >
                    Target USD
                  </Button>
                </div>

                {amountMode === 'usd' ? (
                  <div className="grid gap-2">
                    <input
                      type="number"
                      step="0.01"
                      className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                      value={amountUsd}
                      onChange={(e) => setAmountUsd(e.target.value === '' ? '' : Number(e.target.value))}
                      placeholder="np. 10"
                    />
                    <div className="text-[11px] text-muted-foreground">
                      {inputPriceUsd > 0
                        ? `Input price: ~$${inputPriceUsd.toFixed(4)} → input amount auto`
                        : 'Brak ceny USD dla input token (Jupiter price) — nie da się przeliczyć target USD.'}
                    </div>
                  </div>
                ) : (
                  <div className="grid gap-2">
                    <input
                      type="number"
                      step="0.000001"
                      className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                      value={amountUi}
                      onChange={(e) => {
                        setPercent(0)
                        setAmountUi(e.target.value === '' ? '' : Number(e.target.value))
                      }}
                      placeholder="0.0"
                    />
                    <div className="flex flex-wrap gap-2">
                      <Button type="button" variant="secondary" size="sm" onClick={onClickHalf}>
                        Half
                      </Button>
                      <Button type="button" variant="secondary" size="sm" onClick={onClickMax}>
                        Max
                      </Button>
                      <Button
                        type="button"
                        variant="outline"
                        size="sm"
                        onClick={() => copyText(String(inputBalanceUi))}
                      >
                        <Copy className="h-4 w-4 mr-2" />
                        Copy bal
                      </Button>
                    </div>

                    <div className="grid gap-1.5">
                      <div className="flex items-center justify-between text-[11px] text-muted-foreground">
                        <span>Use % of balance</span>
                        <span className="font-mono tabular-nums">{percent}%</span>
                      </div>
                      <input
                        type="range"
                        min={0}
                        max={100}
                        step={1}
                        value={percent}
                        onChange={(e) => setPercent(Number(e.target.value))}
                      />
                      <div className="flex flex-wrap gap-2">
                        {[10, 25, 50, 75, 100].map((p) => (
                          <Button
                            key={p}
                            type="button"
                            variant="outline"
                            size="sm"
                            onClick={() => setPercent(p)}
                          >
                            {p}%
                          </Button>
                        ))}
                        <Button type="button" variant="ghost" size="sm" onClick={() => setPercent(0)}>
                          Reset
                        </Button>
                      </div>
                    </div>
                  </div>
                )}
              </div>

              <div className="text-[11px] text-muted-foreground">
                {amountUi !== '' && inputPriceUsd > 0 && Number.isFinite(Number(amountUi))
                  ? `≈ $${(Number(amountUi) * inputPriceUsd).toFixed(4)}`
                  : ' '}
              </div>
            </div>

            <div className="flex items-center justify-center">
              <Button
                type="button"
                variant="outline"
                size="icon"
                disabled={!canOpen}
                onClick={() => {
                  setPercent(0)
                  if (provider === 'orca' && poolQ.data) {
                    // In Orca mode, just flip between the two pool mints.
                    const a = poolQ.data.token_mint_a
                    const b = poolQ.data.token_mint_b
                    if (inputMint === a) {
                      setInputMint(b)
                      setOutputMint(a)
                    } else {
                      setInputMint(a)
                      setOutputMint(b)
                    }
                    return
                  }
                  const tmp = inputMint
                  setInputMint(outputMint)
                  setOutputMint(tmp)
                }}
              >
                <RefreshCcw className="h-4 w-4" />
              </Button>
            </div>

            <div className="rounded-lg border border-border bg-muted/10 p-3 space-y-2">
              <div className="flex items-center justify-between gap-2">
                <div className="text-xs text-muted-foreground">You receive</div>
                <div className="text-xs text-muted-foreground">Output token</div>
              </div>

              <div className="flex items-center justify-between gap-2">
                <div className="text-lg font-semibold">{tokenLabel(outputMint)}</div>
                <div className="relative">
                  <select
                    className="appearance-none rounded-md border border-input bg-background pl-3 pr-8 py-2 text-sm"
                    value={outputMint}
                    onChange={(e) => {
                      const next = e.target.value
                      if (provider === 'orca' && poolQ.data) {
                        const a = poolQ.data.token_mint_a
                        const b = poolQ.data.token_mint_b
                        // Output is always the opposite of input in Orca pool mode.
                        if (next === a && inputMint !== a) {
                          setOutputMint(a)
                          setInputMint(b)
                          return
                        }
                        if (next === b && inputMint !== b) {
                          setOutputMint(b)
                          setInputMint(a)
                          return
                        }
                        return
                      }
                      setOutputMint(next)
                    }}
                  >
                    {(provider === 'orca' && poolQ.data
                      ? tokenOptions.filter(
                          (t) => t.mint === poolQ.data.token_mint_a || t.mint === poolQ.data.token_mint_b,
                        )
                      : tokenOptions
                    ).map((t) => (
                      <option key={t.mint} value={t.mint}>
                        {tokenLabel(t.mint)}
                      </option>
                    ))}
                  </select>
                  <ChevronDown className="pointer-events-none absolute right-2 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground" />
                </div>
              </div>

              <div className="text-xs text-muted-foreground">
                Mint: <span className="font-mono">{shortenAddress(outputMint, 6)}</span>{' '}
                <button type="button" className="underline underline-offset-2" onClick={() => copyText(outputMint)}>
                  copy
                </button>
              </div>
            </div>
          </div>

          <div className="flex flex-wrap items-center justify-between gap-2 pt-1">
            <div className="text-xs text-muted-foreground">
              Input mint:{' '}
              <span className="font-mono">{shortenAddress(inputMint, 6)}</span>{' '}
              <button type="button" className="underline underline-offset-2" onClick={() => copyText(inputMint)}>
                copy
              </button>
            </div>

            <div className="flex flex-wrap gap-2">
              {provider === 'orca' ? (
                <>
                  <Button
                    type="button"
                    variant="secondary"
                    disabled={!canOpen}
                    onClick={() => copyText(`inputMint=${inputMint}\noutputMint=${outputMint}\namountUi=${amountUi === '' ? '' : String(amountUi)}`)}
                  >
                    <Copy className="h-4 w-4 mr-2" />
                    Copy params
                  </Button>
                </>
              ) : null}

              {provider === 'orca' ? (
                <Button
                  type="button"
                  disabled={
                    !canOpen ||
                    swapMutation.isPending ||
                    amountRaw == null ||
                    poolAddress.trim().length === 0 ||
                    !apiSignerQ.data?.configured ||
                    (apiSignerQ.data.lamports != null &&
                      apiSignerQ.data.lamports < apiSignerQ.data.min_swap_lamports)
                  }
                  onClick={() => {
                    setSwapErr(null)
                    setSwapSig(null)
                    swapMutation.mutate({
                      pool_address: poolAddress.trim(),
                      specified_mint: inputMint,
                      amount_in: amountRaw ?? 0,
                      cost_session_id: `${Date.now()}-${Math.random().toString(16).slice(2)}`,
                    })
                  }}
                >
                  {swapMutation.isPending ? 'Swapping…' : 'Swap now (Orca pool)'}
                </Button>
              ) : (
                <a
                  href={openInProviderUrl ?? undefined}
                  target="_blank"
                  rel="noopener noreferrer"
                  className={`inline-flex items-center gap-2 rounded-md px-3 py-2 text-sm font-medium ${
                    canOpen ? 'bg-primary text-primary-foreground hover:opacity-90' : 'bg-muted text-muted-foreground pointer-events-none'
                  }`}
                >
                  Open in Jupiter <ExternalLink className="h-4 w-4" />
                </a>
              )}
            </div>
          </div>

          {(swapSig || swapErr) && provider === 'orca' ? (
            <div className="rounded-lg border border-border bg-muted/10 p-3 text-xs space-y-1">
              {swapSig ? (
                <div>
                  <span className="font-medium">Swap submitted:</span>{' '}
                  <span className="font-mono break-all">{swapSig}</span>
                </div>
              ) : null}
              {swapErr ? (
                <div className="text-destructive break-words">
                  <span className="font-medium">Swap failed:</span> {swapErr}
                </div>
              ) : null}
            </div>
          ) : null}
        </CardContent>
      </Card>
    </div>
  )
}

