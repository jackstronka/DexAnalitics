import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useNavigate, Link } from 'react-router-dom'
import { ArrowLeft } from 'lucide-react'
import { Card, CardHeader, CardTitle, CardContent } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { ErrorBanner } from '@/components/ui/error-banner'
import { InlineError } from '@/components/ui/inline-error'
import {
  getMintPricesUsd,
  getOrcaToken,
  getPool,
  getPoolState,
  getSwapCostEstimate,
  getStrategies,
  getApiSignerWallet,
  getWalletEffectiveBalances,
  getWallets,
  openPosition,
  quoteOpenBudget,
  swapBeforeOpen as swapBeforeOpenTx,
  type QuoteOpenBudgetResponse,
  type WalletBalancesResponse,
} from '@/lib/api'
import {
  alignPriceRatioToTicks,
  calculateTickRangeFromWidthPct,
  priceRatioToInputString,
  tickToPriceRatio,
  uiPriceFromRawPriceRatio,
  rawPriceRatioFromUiPrice,
  formatPriceRatio,
} from '@/lib/whirlpoolTicks'
import { getDevWalletPubkey } from '@/lib/devWallet'
import { formatUSD, shortenAddress } from '@/lib/utils'
import { useI18n } from '@/lib/i18n'

const LS_SELECTED_WALLET_ID = 'clmm.selected_wallet_id'
const WSOL_MINT = 'So11111111111111111111111111111111111111112'
const WSOL_ATA_RENT_LAMPORTS_EST = 2_039_280

/** Porównanie UI: mała tolerancja na błąd zaokrągleń float. */
const BALANCE_EPS = 1e-8

/**
 * Saldo tokenowe w jednostkach UI dla mintu (bez native SOL).
 */
function getAvailableUiAmount(
  mint: string,
  balances: WalletBalancesResponse | undefined,
): number | null {
  if (!balances) return null
  const row = balances.tokens.find((t) => t.mint === mint)
  if (!row) return 0
  const v = parseFloat(row.ui_amount)
  return Number.isFinite(v) ? v : 0
}

function isInsufficientBalance(needUi: number, haveUi: number): boolean {
  return needUi > haveUi + BALANCE_EPS
}

function hasTokenAccount(mint: string, balances: WalletBalancesResponse | undefined): boolean {
  if (!balances) return false
  return balances.tokens.some((t) => t.mint === mint)
}

/**
 * Szacunek kwoty wejściowej (raw, atomic) przy swapie z tokena `fund` do pokrycia niedoboru `deficitUi` tokena `short`.
 * Używa cen USD z Jupitera; +5% bufor na slippage.
 */
function estimateSwapInputRawExactIn(
  fundMint: string,
  fundDecimals: number,
  shortMint: string,
  deficitUi: number,
  pricesUsd: Record<string, number> | undefined,
): number | null {
  if (!pricesUsd || deficitUi <= 0) return null
  const pShort = pricesUsd[shortMint]
  const pFund = pricesUsd[fundMint]
  if (!pShort || !pFund || pShort <= 0 || pFund <= 0) return null
  const usdNeed = deficitUi * pShort
  const fundUi = (usdNeed / pFund) * 1.05
  if (!Number.isFinite(fundUi) || fundUi <= 0) return null
  const raw = Math.round(fundUi * 10 ** fundDecimals)
  if (raw <= 0 || raw > Number.MAX_SAFE_INTEGER) return null
  return raw
}

/**
 * Fallback estimator from Whirlpool spot price (UI B-per-A) when USD feed is noisy.
 * Returns ExactIn raw amount of funding leg with +5% buffer.
 */
function estimateSwapInputRawFromPoolPrice(
  deficitAUi: number,
  deficitBUi: number,
  fundIsTokenA: boolean,
  tokenADecimals: number,
  tokenBDecimals: number,
  poolPriceRaw: number,
): number | null {
  if (!Number.isFinite(poolPriceRaw) || poolPriceRaw <= 0) return null
  const bPerAUi = uiPriceFromRawPriceRatio(poolPriceRaw, tokenADecimals, tokenBDecimals)
  if (!Number.isFinite(bPerAUi) || (bPerAUi ?? 0) <= 0) return null
  const p = bPerAUi as number
  const buffer = 1.05

  // Funding with token A to cover token B deficit.
  if (fundIsTokenA && deficitBUi > 0) {
    const fundAUi = (deficitBUi / p) * buffer
    const raw = Math.round(fundAUi * 10 ** tokenADecimals)
    if (!Number.isFinite(raw) || raw <= 0 || raw > Number.MAX_SAFE_INTEGER) return null
    return raw
  }
  // Funding with token B to cover token A deficit.
  if (!fundIsTokenA && deficitAUi > 0) {
    const fundBUi = deficitAUi * p * buffer
    const raw = Math.round(fundBUi * 10 ** tokenBDecimals)
    if (!Number.isFinite(raw) || raw <= 0 || raw > Number.MAX_SAFE_INTEGER) return null
    return raw
  }
  return null
}

function legUiFromQuote(q: QuoteOpenBudgetResponse, leg: 'a' | 'b'): number {
  return leg === 'a' ? q.amount_a_ui : q.amount_b_ui
}

/**
 * Dopasowuje `target_usd` tak, żeby `quoteOpenBudget` zwrócił na wybranej nodze (`leg`)
 * ilość UI bliską `targetLegUi` (wyszukiwanie binarne po monotonicznym skalowaniu depozytu).
 */
async function solveTargetUsdForLegAmount(
  poolAddress: string,
  tickLower: number,
  tickUpper: number,
  leg: 'a' | 'b',
  targetLegUi: number,
  signal?: AbortSignal,
): Promise<{ target_usd: number; quote: QuoteOpenBudgetResponse } | null> {
  if (!Number.isFinite(targetLegUi) || targetLegUi <= 0) return null

  const fetchQ = async (target_usd: number) => {
    if (signal?.aborted) throw new DOMException('aborted', 'AbortError')
    return quoteOpenBudget(poolAddress, {
      tick_lower: tickLower,
      tick_upper: tickUpper,
      target_usd,
    })
  }

  const g = (q: QuoteOpenBudgetResponse) => legUiFromQuote(q, leg)

  // Znajdź hi: g(quote(hi)) >= targetLegUi
  let hi = 0.01
  let qHi = await fetchQ(hi)
  let gHi = g(qHi)
  let guard = 0
  while (gHi < targetLegUi && hi < 1e9 && guard < 36) {
    hi *= 2
    qHi = await fetchQ(hi)
    gHi = g(qHi)
    guard++
    if (signal?.aborted) return null
  }
  if (gHi < targetLegUi) {
    return { target_usd: hi, quote: qHi }
  }

  let lo = 1e-8

  const tolLeg = Math.max(1e-12, targetLegUi * 1e-7)
  const tolUsd = 1e-9
  for (let i = 0; i < 48; i++) {
    const mid = (lo + hi) / 2
    const qm = await fetchQ(mid)
    if (signal?.aborted) return null
    const gm = g(qm)
    if (Math.abs(gm - targetLegUi) <= tolLeg) {
      return { target_usd: mid, quote: qm }
    }
    if (gm < targetLegUi) lo = mid
    else hi = mid
    if (hi - lo <= tolUsd) {
      const midF = (lo + hi) / 2
      const qf = await fetchQ(midF)
      if (signal?.aborted) return null
      return { target_usd: midF, quote: qf }
    }
  }
  const midF = (lo + hi) / 2
  const qf = await fetchQ(midF)
  if (signal?.aborted) return null
  return { target_usd: midF, quote: qf }
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

/** SPL token balance for mint (without native SOL fallback). */
function formatBalanceLine(
  mint: string,
  balances: WalletBalancesResponse | undefined,
): { amount: string; note?: string } {
  if (!balances) {
    return { amount: '—' }
  }
  const row = balances.tokens.find((t) => t.mint === mint)
  if (row) {
    const v = parseFloat(row.ui_amount)
    if (Number.isFinite(v)) {
      return { amount: v.toLocaleString(undefined, { maximumFractionDigits: 8 }) }
    }
    return { amount: row.ui_amount || '0' }
  }
  return { amount: '0' }
}

export default function PositionCreate() {
  const { locale } = useI18n()
  const L = (pl: string, en: string) => (locale === 'pl' ? pl : en)
  const navigate = useNavigate()
  const queryClient = useQueryClient()

  const curatedPools = useMemo(
    () => [
      {
        label: 'SOL/USDC (0.04%)',
        address: 'Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE',
      },
      {
        label: 'whETH/SOL (0.05%)',
        address: 'HktfL7iwGKT5QHjywQkcDnZXScoh811k7akrMZJkCcEF',
      },
      {
        label: 'cbBTC/USDC (0.04%)',
        address: 'HxA6SKW5qA4o12fjVgTpXdq2YnZ5Zv1s7SB4FFomsyLM',
      },
      {
        label: 'WBTC/cbBTC (0.01%)',
        address: '4v8ufj8Hj7UvFgtofQJAtzUud5xomwZfEqfCTHZ4wM72',
      },
    ],
    [],
  )

  const [poolAddress, setPoolAddress] = useState('')
  const [strategyId, setStrategyId] = useState('')
  const [tickLower, setTickLower] = useState<number | ''>('')
  const [tickUpper, setTickUpper] = useState<number | ''>('')
  /** When true, tick lower/upper follow pool price + strategy Range Width % (refetch ~10s). */
  const [tickAutoSync, setTickAutoSync] = useState(true)

  // Human units in the UI
  const [amountAUi, setAmountAUi] = useState<number | ''>('')
  const [amountBUi, setAmountBUi] = useState<number | ''>('')

  // Budget split mode (USD)
  const [mode, setMode] = useState<'tokens' | 'budget'>('tokens')
  const [totalUsd, setTotalUsd] = useState<number | ''>('')
  /** API-side Orca swap in pool before open (requires server KEYPAIR / executor). */
  const [swapBeforeOpen, setSwapBeforeOpen] = useState(false)

  // 2-step SWAP then OPEN state
  const [swapCostSessionId, setSwapCostSessionId] = useState<string | null>(null)
  const [swapSignature, setSwapSignature] = useState<string | null>(null)
  const [swapStepInfo, setSwapStepInfo] = useState<string | null>(null)
  const [swapStepError, setSwapStepError] = useState<string | null>(null)
  const [openStepError, setOpenStepError] = useState<string | null>(null)

  /** Tryb „wspólna kwota USD”: edycja Amount A/B → przelicza docelowe USD i drugą nogę (debounce + binary search po API). */
  const budgetLegDebounceRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const budgetLegAbortRef = useRef<AbortController | null>(null)
  const [budgetLegSyncing, setBudgetLegSyncing] = useState(false)
  const [budgetLegSyncError, setBudgetLegSyncError] = useState<string | null>(null)

  /** Editable price range (mint B per 1 mint A); when `syncPriceInputsFromTicks`, fields mirror ticks. */
  const [priceRangeLo, setPriceRangeLo] = useState('')
  const [priceRangeHi, setPriceRangeHi] = useState('')
  const [syncPriceInputsFromTicks, setSyncPriceInputsFromTicks] = useState(true)
  const [priceRangeError, setPriceRangeError] = useState<string | null>(null)
  /** Advanced UI: show raw tick inputs (internally required by API). */
  const [showAdvancedTicks, setShowAdvancedTicks] = useState(false)

  const poolQ = useQuery({
    queryKey: ['pool', poolAddress],
    queryFn: () => getPool(poolAddress.trim()),
    enabled: poolAddress.trim().length > 0,
    staleTime: 60_000,
  })

  const poolStateQ = useQuery({
    queryKey: ['pool-state', poolAddress],
    queryFn: () => getPoolState(poolAddress.trim()),
    enabled: poolAddress.trim().length > 0,
    staleTime: 0,
    refetchInterval: 10_000,
  })

  const strategiesQ = useQuery({
    queryKey: ['strategies'],
    queryFn: getStrategies,
    staleTime: 30_000,
  })

  const devPk = getDevWalletPubkey()
  const walletsQ = useQuery({
    queryKey: ['wallets'],
    queryFn: getWallets,
    staleTime: 30_000,
  })

  const ownerPk = useMemo(() => {
    if (typeof window === 'undefined') {
      return null
    }
    const id = window.localStorage.getItem(LS_SELECTED_WALLET_ID) || ''
    const picked = walletsQ.data?.wallets.find((w) => w.id === id)
    return picked?.pubkey ?? devPk ?? walletsQ.data?.wallets[0]?.pubkey ?? null
  }, [walletsQ.data?.wallets, devPk])

  const apiSignerQ = useQuery({
    queryKey: ['api-signer-wallet'],
    queryFn: getApiSignerWallet,
    staleTime: 20_000,
  })
  const effectiveOwnerPk = apiSignerQ.data?.pubkey?.trim() || ownerPk
  const usesApiSignerBalances = !!apiSignerQ.data?.configured && !!apiSignerQ.data?.pubkey
  const effectiveBalancesQ = useQuery({
    queryKey: ['wallet-balances', effectiveOwnerPk ?? ''],
    queryFn: () => getWalletEffectiveBalances(effectiveOwnerPk!),
    enabled: !!effectiveOwnerPk,
    staleTime: 20_000,
  })

  const strategyOptions = useMemo(
    () => strategiesQ.data?.strategies ?? [],
    [strategiesQ.data?.strategies],
  )

  const selectedStrategy = useMemo(
    () => strategyOptions.find((s) => s.id === strategyId.trim()),
    [strategyOptions, strategyId],
  )

  const strategyRangeWidthPct = selectedStrategy?.parameters.range_width_pct

  useEffect(() => {
    setTickAutoSync(true)
  }, [strategyId, poolAddress])

  const priceAtTickBounds = useMemo(() => {
    if (tickLower === '' || tickUpper === '') {
      return null
    }
    const tl = Number(tickLower)
    const tu = Number(tickUpper)
    if (!Number.isFinite(tl) || !Number.isFinite(tu)) {
      return null
    }
    return {
      lower: tickToPriceRatio(tl),
      upper: tickToPriceRatio(tu),
    }
  }, [tickLower, tickUpper])

  // Price inputs sync from ticks depends on token decimals; the actual conversion effect is
  // defined below where `tokenA` / `tokenB` are available.
  useEffect(() => {
    if (!syncPriceInputsFromTicks) return
    if (tickLower === '' || tickUpper === '') {
      setPriceRangeLo('')
      setPriceRangeHi('')
    }
  }, [tickLower, tickUpper, syncPriceInputsFromTicks])

  useEffect(() => {
    if (!tickAutoSync) {
      return
    }
    if (!strategyId.trim() || strategyRangeWidthPct == null || strategyRangeWidthPct <= 0 || !poolQ.data) {
      return
    }
    const tickCurrent = poolStateQ.data?.current_tick ?? poolQ.data.current_tick
    const spacing = poolQ.data.tick_spacing
    const { tickLower: tl, tickUpper: tu } = calculateTickRangeFromWidthPct(
      tickCurrent,
      strategyRangeWidthPct,
      spacing,
    )
    setTickLower(tl)
    setTickUpper(tu)
    setSyncPriceInputsFromTicks(true)
  }, [
    tickAutoSync,
    strategyId,
    strategyRangeWidthPct,
    poolStateQ.data?.current_tick,
    poolQ.data?.current_tick,
    poolQ.data?.tick_spacing,
  ])

  useEffect(() => {
    setStrategyId('')
  }, [poolAddress])

  const mintA = poolQ.data?.token_mint_a
  const mintB = poolQ.data?.token_mint_b

  const orcaAQ = useQuery({
    queryKey: ['orca-token', mintA],
    queryFn: () => getOrcaToken(mintA!),
    enabled: !!mintA,
    staleTime: 60 * 60 * 1000,
  })

  const orcaBQ = useQuery({
    queryKey: ['orca-token', mintB],
    queryFn: () => getOrcaToken(mintB!),
    enabled: !!mintB,
    staleTime: 60 * 60 * 1000,
  })

  const tokenA = useMemo(() => {
    if (!mintA) return undefined
    return {
      mint: mintA,
      symbol: orcaAQ.data?.symbol ?? shortenAddress(mintA, 4),
      decimals: orcaAQ.data?.decimals ?? 9,
    }
  }, [mintA, orcaAQ.data])

  const tokenB = useMemo(() => {
    if (!mintB) return undefined
    return {
      mint: mintB,
      symbol: orcaBQ.data?.symbol ?? shortenAddress(mintB, 4),
      decimals: orcaBQ.data?.decimals ?? 9,
    }
  }, [mintB, orcaBQ.data])

  useEffect(() => {
    if (!syncPriceInputsFromTicks) return
    if (!tokenA || !tokenB) return
    if (tickLower === '' || tickUpper === '') return

    const tl = Number(tickLower)
    const tu = Number(tickUpper)
    if (!Number.isFinite(tl) || !Number.isFinite(tu)) return

    const rawLo = tickToPriceRatio(tl)
    const rawHi = tickToPriceRatio(tu)
    const uiLo = uiPriceFromRawPriceRatio(rawLo, tokenA.decimals, tokenB.decimals)
    const uiHi = uiPriceFromRawPriceRatio(rawHi, tokenA.decimals, tokenB.decimals)
    setPriceRangeLo(priceRatioToInputString(uiLo ?? Number.NaN))
    setPriceRangeHi(priceRatioToInputString(uiHi ?? Number.NaN))
  }, [
    tickLower,
    tickUpper,
    syncPriceInputsFromTicks,
    tokenA?.decimals,
    tokenB?.decimals,
  ])

  const walletLineA = useMemo(
    () => (mintA ? formatBalanceLine(mintA, effectiveBalancesQ.data) : null),
    [mintA, effectiveBalancesQ.data],
  )
  const walletLineB = useMemo(
    () => (mintB ? formatBalanceLine(mintB, effectiveBalancesQ.data) : null),
    [mintB, effectiveBalancesQ.data],
  )

  const pricesQ = useQuery({
    queryKey: ['jupiter-prices', tokenA?.mint, tokenB?.mint],
    queryFn: async () => {
      const mints = [tokenA?.mint, tokenB?.mint].filter(Boolean) as string[]
      return await getMintPricesUsd(mints)
    },
    // Ceny także poza trybem „budget” — linki Jupiter z wstępnie szacowaną kwotą swapu.
    enabled: !!tokenA?.mint && !!tokenB?.mint,
    staleTime: 60_000,
  })

  const currentTick = useMemo(() => {
    const t = poolStateQ.data?.current_tick ?? poolQ.data?.current_tick
    return typeof t === 'number' && Number.isFinite(t) ? t : null
  }, [poolStateQ.data?.current_tick, poolQ.data?.current_tick])

  const budgetTickRangeInPrice = useMemo(() => {
    if (currentTick == null) return null
    if (tickLower === '' || tickUpper === '') return null
    const tl = Number(tickLower)
    const tu = Number(tickUpper)
    if (!Number.isFinite(tl) || !Number.isFinite(tu)) return null
    return tl <= currentTick && currentTick < tu
  }, [currentTick, tickLower, tickUpper])

  const budgetQuoteEnabled =
    mode === 'budget' &&
    poolAddress.trim().length > 0 &&
    tickLower !== '' &&
    tickUpper !== '' &&
    totalUsd !== '' &&
    Number.isFinite(Number(totalUsd)) &&
    Number(totalUsd) > 0 &&
    Number.isFinite(Number(tickLower)) &&
    Number.isFinite(Number(tickUpper)) &&
    // Backend quote requires current price to be in-range; otherwise it errors.
    // When we can infer current tick, block the request and show a clear UI hint.
    (budgetTickRangeInPrice !== false)

  const budgetQuoteQ = useQuery({
    queryKey: ['quote-open-budget', poolAddress.trim(), tickLower, tickUpper, totalUsd],
    queryFn: () =>
      quoteOpenBudget(poolAddress.trim(), {
        tick_lower: Number(tickLower),
        tick_upper: Number(tickUpper),
        target_usd: Number(totalUsd),
      }),
    enabled: budgetQuoteEnabled,
    staleTime: 10_000,
  })

  const applyBudgetQuoteFromLeg = useCallback(
    async (leg: 'a' | 'b', num: number) => {
      const pool = poolAddress.trim()
      const tl = Number(tickLower)
      const tu = Number(tickUpper)
      if (!pool || !Number.isFinite(tl) || !Number.isFinite(tu)) {
        setBudgetLegSyncError('Ustaw pulę i ticki.')
        return
      }
      if (budgetTickRangeInPrice === false) {
        setBudgetLegSyncError('Cena poza zakresem ticków — najpierw ustaw in-range.')
        return
      }
      const ac = new AbortController()
      budgetLegAbortRef.current = ac
      setBudgetLegSyncing(true)
      setBudgetLegSyncError(null)
      try {
        const solved = await solveTargetUsdForLegAmount(pool, tl, tu, leg, num, ac.signal)
        if (!solved) {
          // Przerwany request (nowa edycja / unmount) zwraca `null` — nie pokazuj błędu „dopasowania”.
          if (ac.signal.aborted) return
          setBudgetLegSyncError('Nie można dopasować docelowej wartości USD do tej kwoty na nodze.')
          return
        }
        const t = Number(solved.target_usd.toFixed(10))
        queryClient.setQueryData<QuoteOpenBudgetResponse>(
          ['quote-open-budget', pool, tickLower, tickUpper, t],
          solved.quote,
        )
        setTotalUsd(t)
        // Jedna spójna paczka z quote (SOL + USDC + USD), bez czekania na kolejny cykl useEffect.
        if (Number.isFinite(solved.quote.amount_a_ui) && Number.isFinite(solved.quote.amount_b_ui)) {
          setAmountAUi(Number(solved.quote.amount_a_ui.toFixed(8)))
          setAmountBUi(Number(solved.quote.amount_b_ui.toFixed(8)))
        }
        setBudgetSubmitRaw({ a: solved.quote.token_max_a, b: solved.quote.token_max_b })
      } catch (e: unknown) {
        if (e instanceof DOMException && e.name === 'AbortError') return
        setBudgetLegSyncError(e instanceof Error ? e.message : String(e))
      } finally {
        setBudgetLegSyncing(false)
      }
    },
    [poolAddress, tickLower, tickUpper, budgetTickRangeInPrice, queryClient],
  )

  const scheduleBudgetLegSync = useCallback(
    (leg: 'a' | 'b', rawValue: string) => {
      if (budgetLegDebounceRef.current) clearTimeout(budgetLegDebounceRef.current)
      budgetLegAbortRef.current?.abort()
      setBudgetLegSyncError(null)

      if (rawValue.trim() === '') return
      const num = Number(rawValue)
      if (!Number.isFinite(num) || num <= 0) return

      budgetLegDebounceRef.current = setTimeout(() => {
        void applyBudgetQuoteFromLeg(leg, num)
      }, 420)
    },
    [applyBudgetQuoteFromLeg],
  )

  useEffect(() => {
    return () => {
      if (budgetLegDebounceRef.current) clearTimeout(budgetLegDebounceRef.current)
      budgetLegAbortRef.current?.abort()
    }
  }, [])

  useEffect(() => {
    if (mode !== 'budget') {
      if (budgetLegDebounceRef.current) clearTimeout(budgetLegDebounceRef.current)
      budgetLegAbortRef.current?.abort()
      setBudgetLegSyncing(false)
      setBudgetLegSyncError(null)
    }
  }, [mode])

  /** Caps z quote (u64) — submit bez strat float; UI pokazuje amount_*_ui. */
  const [budgetSubmitRaw, setBudgetSubmitRaw] = useState<{ a: number; b: number } | null>(null)

  useEffect(() => {
    if (mode !== 'budget') {
      setBudgetSubmitRaw(null)
      return
    }
    const d = budgetQuoteQ.data
    if (!d) {
      setBudgetSubmitRaw(null)
      return
    }
    if (Number.isFinite(d.amount_a_ui) && Number.isFinite(d.amount_b_ui)) {
      setAmountAUi(Number(d.amount_a_ui.toFixed(8)))
      setAmountBUi(Number(d.amount_b_ui.toFixed(8)))
    }
    setBudgetSubmitRaw({ a: d.token_max_a, b: d.token_max_b })
  }, [mode, budgetQuoteQ.data])

  /** Wymagane kwoty vs saldo + linki Jupiter (prefill mintów i szacunkowa kwota wejścia). */
  const fundingCheck = useMemo(() => {
    const empty = {
      ready: false,
      blocked: false,
      shortA: false,
      shortB: false,
      shortOperationalSol: false,
      mode: mode as 'manual' | 'budget',
      deficitA: 0,
      deficitB: 0,
      deficitOperationalSol: 0,
      haveA: null as number | null,
      haveB: null as number | null,
      nativeSol: null as number | null,
      needA: 0,
      needB: 0,
      needSolLegUi: 0,
      requiredNativeForOpenUi: 0,
      minOpenSolUi: 0,
      jupiterSwapToCoverA: null as string | null,
      jupiterSwapToCoverB: null as string | null,
      jupiterSwapToCoverSol: null as string | null,
      jupiterGeneric: 'https://jup.ag/swap' as string,
    }
    if (!effectiveOwnerPk || !tokenA || !tokenB || !effectiveBalancesQ.data) {
      return empty
    }
    if (
      amountAUi === '' ||
      amountBUi === '' ||
      !Number.isFinite(Number(amountAUi)) ||
      !Number.isFinite(Number(amountBUi))
    ) {
      return empty
    }
    let needA = Number(amountAUi)
    let needB = Number(amountBUi)
    // W trybie USD submit używa capów `token_max_*` z quote, więc walidacja
    // musi opierać się na tych samych wartościach (a nie tylko na amount_*_ui).
    if (mode === 'budget' && budgetSubmitRaw != null) {
      needA = budgetSubmitRaw.a / 10 ** tokenA.decimals
      needB = budgetSubmitRaw.b / 10 ** tokenB.decimals
    }
    const haveA = getAvailableUiAmount(tokenA.mint, effectiveBalancesQ.data)
    const haveB = getAvailableUiAmount(tokenB.mint, effectiveBalancesQ.data)
    if (haveA === null || haveB === null) {
      return empty
    }
    const shortA = isInsufficientBalance(needA, haveA)
    const shortB = isInsufficientBalance(needB, haveB)
    const deficitA = shortA ? Math.max(0, needA - haveA) : 0
    const deficitB = shortB ? Math.max(0, needB - haveB) : 0
    const nativeSol = parseFloat(effectiveBalancesQ.data.sol)
    const nativeSolUi = Number.isFinite(nativeSol) ? nativeSol : 0
    const minOpenSolUi = (apiSignerQ.data?.min_open_lamports ?? 0) / 1e9
    const needSolLegUi =
      tokenA.mint === WSOL_MINT
        ? needA
        : tokenB.mint === WSOL_MINT
          ? needB
          : 0
    const wsolAccountExists = hasTokenAccount(WSOL_MINT, effectiveBalancesQ.data)
    const wsolAtaRentUi = !wsolAccountExists && needSolLegUi > 0 ? WSOL_ATA_RENT_LAMPORTS_EST / 1e9 : 0
    // Keep UI estimate aligned with current backend preflight model:
    // required native SOL = WSOL leg need + open pad (+ ATA rent if missing account).
    const requiredNativeForOpenUi = needSolLegUi + minOpenSolUi + wsolAtaRentUi
    const shortOperationalSol = nativeSolUi + BALANCE_EPS < requiredNativeForOpenUi
    const deficitOperationalSol = shortOperationalSol
      ? Math.max(0, requiredNativeForOpenUi - nativeSolUi)
      : 0
    const px = pricesQ.data?.prices

    let jupiterSwapToCoverA: string | null = null
    let jupiterSwapToCoverB: string | null = null
    let jupiterSwapToCoverSol: string | null = null

    if (shortA && shortB) {
      // Brak obu tokenów — nie da się zbudować sensownego ExactIn bez trzeciej nogi; ogólny link.
      return {
        ready: true,
        blocked: true,
        shortA,
        shortB,
        shortOperationalSol,
        mode,
        deficitA,
        deficitB,
        deficitOperationalSol,
        haveA,
        haveB,
        nativeSol: nativeSolUi,
        needA,
        needB,
        needSolLegUi,
        requiredNativeForOpenUi,
        minOpenSolUi,
        jupiterSwapToCoverA: null,
        jupiterSwapToCoverB: null,
        jupiterSwapToCoverSol: null,
        jupiterGeneric: 'https://jup.ag/swap',
      }
    }
    if (shortA && !shortB) {
      const raw = estimateSwapInputRawExactIn(
        tokenB.mint,
        tokenB.decimals,
        tokenA.mint,
        deficitA,
        px,
      )
      jupiterSwapToCoverA = buildJupiterSwapUrl(tokenB.mint, tokenA.mint, raw)
    }
    if (shortB && !shortA) {
      const raw = estimateSwapInputRawExactIn(
        tokenA.mint,
        tokenA.decimals,
        tokenB.mint,
        deficitB,
        px,
      )
      jupiterSwapToCoverB = buildJupiterSwapUrl(tokenA.mint, tokenB.mint, raw)
    }

    if (shortOperationalSol) {
      const candidateInputMint =
        !shortA && tokenA.mint !== WSOL_MINT
          ? tokenA.mint
          : !shortB && tokenB.mint !== WSOL_MINT
            ? tokenB.mint
            : tokenA.mint !== WSOL_MINT
              ? tokenA.mint
              : tokenB.mint !== WSOL_MINT
                ? tokenB.mint
                : null
      jupiterSwapToCoverSol = candidateInputMint
        ? buildJupiterSwapUrl(candidateInputMint, WSOL_MINT, null)
        : 'https://jup.ag/swap?outputMint=So11111111111111111111111111111111111111112'
    }

    return {
      ready: true,
      blocked: shortA || shortB || shortOperationalSol,
      shortA,
      shortB,
      shortOperationalSol,
      mode,
      deficitA,
      deficitB,
      deficitOperationalSol,
      haveA,
      haveB,
      nativeSol: nativeSolUi,
      needA,
      needB,
      needSolLegUi,
      requiredNativeForOpenUi,
      minOpenSolUi,
      jupiterSwapToCoverA,
      jupiterSwapToCoverB,
      jupiterSwapToCoverSol,
      jupiterGeneric: 'https://jup.ag/swap',
    }
  }, [
    effectiveOwnerPk,
    tokenA,
    tokenB,
    mode,
    budgetSubmitRaw,
    effectiveBalancesQ.data,
    amountAUi,
    amountBUi,
    pricesQ.data,
    apiSignerQ.data?.min_open_lamports,
  ])

  /** Single-sided deficit: ExactIn swap in **this** pool (mint + raw amount) for `swap_before_open`. */
  const swapBeforeOpenPlan = useMemo(() => {
    if (!fundingCheck.ready || !tokenA || !tokenB || !effectiveBalancesQ.data) {
      return null
    }
    if (fundingCheck.shortA && fundingCheck.shortB) {
      return null
    }
    const px = pricesQ.data?.prices
    const capPct = 0.92
    const poolPriceRaw = Number(poolStateQ.data?.price ?? poolQ.data?.price)

    // Operational SOL deficit (native rent+fee buffer) with balanced pool legs:
    // allow single in-pool swap to WSOL before open for WSOL pairs.
    if (!fundingCheck.shortA && !fundingCheck.shortB && fundingCheck.shortOperationalSol) {
      const wsolIsA = tokenA.mint === WSOL_MINT
      const wsolIsB = tokenB.mint === WSOL_MINT
      if (!wsolIsA && !wsolIsB) {
        return null
      }
      const inToken = wsolIsA ? tokenB : tokenA
      const outToken = wsolIsA ? tokenA : tokenB
      const rawEst = estimateSwapInputRawExactIn(
        inToken.mint,
        inToken.decimals,
        outToken.mint,
        // add small headroom above computed deficit to reduce retries
        fundingCheck.deficitOperationalSol * 1.05,
        px,
      )
      if (!rawEst || rawEst <= 0) {
        return null
      }
      const haveIn = getAvailableUiAmount(inToken.mint, effectiveBalancesQ.data)
      if (haveIn == null) {
        return null
      }
      const maxRaw = Math.floor(haveIn * 10 ** inToken.decimals * capPct)
      const amount_in = Math.min(Math.floor(rawEst), maxRaw)
      if (amount_in <= 0) {
        return null
      }
      return {
        specified_mint: inToken.mint,
        amount_in,
        label: `${inToken.symbol} → ${outToken.symbol} (operational SOL, w puli Orca)`,
      }
    }

    if (!fundingCheck.shortA && !fundingCheck.shortB) {
      return null
    }

    if (fundingCheck.shortB && !fundingCheck.shortA) {
      const rawEstUsd = estimateSwapInputRawExactIn(
        tokenA.mint,
        tokenA.decimals,
        tokenB.mint,
        fundingCheck.deficitB,
        px,
      )
      const rawEstPool = estimateSwapInputRawFromPoolPrice(
        fundingCheck.deficitA,
        fundingCheck.deficitB,
        true,
        tokenA.decimals,
        tokenB.decimals,
        poolPriceRaw,
      )
      const rawEst = Math.max(rawEstUsd ?? 0, rawEstPool ?? 0)
      if (rawEst <= 0) {
        return null
      }
      const haveA = getAvailableUiAmount(tokenA.mint, effectiveBalancesQ.data)
      if (haveA == null) {
        return null
      }
      const maxRaw = Math.floor(haveA * 10 ** tokenA.decimals * capPct)
      const amount_in = Math.min(Math.floor(rawEst), maxRaw)
      if (amount_in <= 0) {
        return null
      }
      return {
        specified_mint: tokenA.mint,
        amount_in,
        label: `${tokenA.symbol} → ${tokenB.symbol} (w puli Orca)`,
      }
    }

    if (fundingCheck.shortA && !fundingCheck.shortB) {
      const rawEstUsd = estimateSwapInputRawExactIn(
        tokenB.mint,
        tokenB.decimals,
        tokenA.mint,
        fundingCheck.deficitA,
        px,
      )
      const rawEstPool = estimateSwapInputRawFromPoolPrice(
        fundingCheck.deficitA,
        fundingCheck.deficitB,
        false,
        tokenA.decimals,
        tokenB.decimals,
        poolPriceRaw,
      )
      const rawEst = Math.max(rawEstUsd ?? 0, rawEstPool ?? 0)
      if (rawEst <= 0) {
        return null
      }
      const haveB = getAvailableUiAmount(tokenB.mint, effectiveBalancesQ.data)
      if (haveB == null) {
        return null
      }
      const maxRaw = Math.floor(haveB * 10 ** tokenB.decimals * capPct)
      const amount_in = Math.min(Math.floor(rawEst), maxRaw)
      if (amount_in <= 0) {
        return null
      }
      return {
        specified_mint: tokenB.mint,
        amount_in,
        label: `${tokenB.symbol} → ${tokenA.symbol} (w puli Orca)`,
      }
    }

    return null
  }, [fundingCheck, tokenA, tokenB, pricesQ.data, effectiveBalancesQ.data, poolQ.data?.price, poolStateQ.data?.price])

  const swapBeforeOpenInputMeta = useMemo(() => {
    if (!swapBeforeOpenPlan || !tokenA || !tokenB) return null
    if (swapBeforeOpenPlan.specified_mint === tokenA.mint) {
      return { symbol: tokenA.symbol, decimals: tokenA.decimals }
    }
    if (swapBeforeOpenPlan.specified_mint === tokenB.mint) {
      return { symbol: tokenB.symbol, decimals: tokenB.decimals }
    }
    return null
  }, [swapBeforeOpenPlan, tokenA, tokenB])

  const swapBeforeOpenAmountUi = useMemo(() => {
    if (!swapBeforeOpenPlan || !swapBeforeOpenInputMeta) return null
    const mul = 10 ** swapBeforeOpenInputMeta.decimals
    return swapBeforeOpenPlan.amount_in / mul
  }, [swapBeforeOpenPlan, swapBeforeOpenInputMeta])

  const swapBeforeOpenEstimate = useMemo(() => {
    if (!swapBeforeOpenPlan || !tokenA || !tokenB) return null
    const px = pricesQ.data?.prices
    if (!px) return null

    const inMint = swapBeforeOpenPlan.specified_mint
    const outMint = inMint === tokenA.mint ? tokenB.mint : tokenA.mint
    const inSym = inMint === tokenA.mint ? tokenA.symbol : tokenB.symbol
    const outSym = outMint === tokenA.mint ? tokenA.symbol : tokenB.symbol
    const inDec = inMint === tokenA.mint ? tokenA.decimals : tokenB.decimals

    const pIn = px[inMint]
    const pOut = px[outMint]
    if (!(pIn > 0) || !(pOut > 0)) return null

    const inUi = swapBeforeOpenPlan.amount_in / 10 ** inDec
    const inUsd = inUi * pIn
    const outUi = inUsd / pOut
    const outUsd = outUi * pOut

    const deficitOutUi =
      outMint === tokenA.mint ? (fundingCheck.shortA ? fundingCheck.deficitA : 0) : fundingCheck.shortB ? fundingCheck.deficitB : 0

    return {
      inMint,
      outMint,
      inSym,
      outSym,
      inUi,
      inUsd,
      outUi,
      outUsd,
      deficitOutUi,
      priceSource: pricesQ.data?.source ?? 'prices',
    }
  }, [swapBeforeOpenPlan, tokenA, tokenB, pricesQ.data, fundingCheck])

  const swapCostEstimateQ = useQuery({
    queryKey: ['swap-cost-estimate', poolAddress.trim()],
    queryFn: () => getSwapCostEstimate(poolAddress.trim()),
    enabled: poolAddress.trim().length > 0 && !!swapBeforeOpenPlan,
    staleTime: 60_000,
  })

  useEffect(() => {
    if (!swapBeforeOpenPlan && !swapSignature) {
      setSwapBeforeOpen(false)
      setSwapCostSessionId(null)
      setSwapSignature(null)
    }
  }, [swapBeforeOpenPlan, swapSignature])

  useEffect(() => {
    setSwapCostSessionId(null)
    setSwapSignature(null)
  }, [poolAddress])

  useEffect(() => {
    if (!swapBeforeOpen) {
      setSwapCostSessionId(null)
      setSwapSignature(null)
    }
  }, [swapBeforeOpen])

  const toBaseUnitsU64 = (ui: number, decimals: number): number | null => {
    if (!Number.isFinite(ui) || ui < 0) return null
    const mul = 10 ** decimals
    const raw = Math.round(ui * mul)
    if (!Number.isFinite(raw) || raw < 0) return null
    if (raw > Number.MAX_SAFE_INTEGER) return null
    return raw
  }

  const translateSwapBeforeOpenError = (rawMsg: string): string | null => {
    // Orca preflight (Whirlpool) can return a long raw English message like:
    // "open preflight exact-plan: insufficient native SOL. Runtime simulation requires X lamports; with 1% safety margin require Y. Current native balance Z. Top up SOL or lower Amount."
    // We normalize the operator-facing parts for PL locale.
    const m = rawMsg.match(
      /insufficient native SOL\.\s*Runtime simulation requires (\d+) lamports;\s*with 1% safety margin require (\d+)(?: lamports)?\.\s*Current native balance (\d+)/i,
    )
    if (!m) return null

    const requiredLamports = Number(m[1])
    const requiredSafeLamports = Number(m[2])
    const haveLamports = Number(m[3])
    if (![requiredLamports, requiredSafeLamports, haveLamports].every(Number.isFinite)) return null

    const formatLamports = (v: number) => v.toLocaleString()
    const formatSol = (vLamports: number) =>
      (vLamports / 1e9).toLocaleString(undefined, {
        maximumFractionDigits: 6,
      })

    const haveSol = formatSol(haveLamports)
    const requiredSol = formatSol(requiredLamports)
    const requiredSafeSol = formatSol(requiredSafeLamports)

    if (locale === 'pl') {
      return (
        `Za mało natywnego SOL do kroku swap-before-open (Orca preflight). ` +
        `Symulacja wymaga ~${requiredSol} SOL, a z 1% marginesem ~${requiredSafeSol} SOL. ` +
        `Masz ~${haveSol} SOL (${formatLamports(haveLamports)} lamportów). ` +
        `Doładuj SOL albo zmniejsz Amount.`
      )
    }

    return (
      `Insufficient native SOL for swap-before-open (Orca preflight). ` +
      `Simulation requires ~${requiredSol} SOL, or ~${requiredSafeSol} SOL with 1% safety margin. ` +
      `Current balance is ~${haveSol} SOL (${formatLamports(haveLamports)} lamports). ` +
      `Top up SOL or lower Amount.`
    )
  }

  const makeCostSessionId = () => {
    if (typeof crypto !== 'undefined' && 'randomUUID' in crypto) {
      return crypto.randomUUID()
    }
    return `${Date.now()}-${Math.random().toString(16).slice(2)}`
  }

  const swapMutation = useMutation({
    mutationFn: swapBeforeOpenTx,
    onSuccess: (data) => {
      setSwapSignature(data.swap_signature ?? null)
      setSwapCostSessionId(data.cost_session_id ?? swapCostSessionId)
      setSwapStepInfo(data.message ?? null)
      queryClient.invalidateQueries({ queryKey: ['wallet-balances', effectiveOwnerPk ?? ''] })
    },
    onError: (err) => {
      const msg = err instanceof Error ? err.message : String(err)
      const translated = translateSwapBeforeOpenError(msg)
      if (translated) {
        setSwapStepError(
          locale === 'pl'
            ? `POST /api/v1/positions/swap-before-open: nieudane. ${translated}`
            : `POST /api/v1/positions/swap-before-open: failed. ${translated}`,
        )
        return
      }

      setSwapStepError(`POST /api/v1/positions/swap-before-open failed: ${msg}`)
    },
  })

  const mutation = useMutation({
    mutationFn: openPosition,
    onSuccess: (data) => {
      setOpenStepError(null)
      queryClient.invalidateQueries({ queryKey: ['positions'] })
      queryClient.invalidateQueries({ queryKey: ['strategies'] })
      if (data.position_pda?.trim()) {
        navigate(`/positions/${data.position_pda.trim()}`)
      } else {
        navigate('/positions')
      }
    },
    onError: (err) => {
      const msg = err instanceof Error ? err.message : String(err)
      setOpenStepError(`POST /api/v1/positions failed: ${msg}`)
    },
  })

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()
    setOpenStepError(null)
    const pool = poolQ.data
    if (
      !poolAddress.trim() ||
      tickLower === '' ||
      tickUpper === '' ||
      amountAUi === '' ||
      amountBUi === '' ||
      !pool
    ) {
      setOpenStepError('Uzupełnij wymagane pola (pool, ticki i kwoty tokenów).')
      return
    }

    if (mode === 'budget' && budgetSubmitRaw == null) {
      setOpenStepError('Tryb USD: poczekaj na wyliczenie kwot (quote) zanim wyślesz.')
      return
    }

    const blockByTokenDeficit = fundingCheck.ready && (fundingCheck.shortA || fundingCheck.shortB)
    if (blockByTokenDeficit) {
      if (swapBeforeOpen) {
        // Two-step flow: open is allowed only after swap succeeded.
        if (!swapSignature) {
          setOpenStepError('Najpierw wykonaj swap (krok 1), dopiero potem otwórz pozycję.')
          return
        }
      } else {
        setOpenStepError('Za mało tokenów na portfelu dla zadanych kwot (zrób swap albo zmniejsz Amount).')
        return
      }
    }

    let aRaw: number | null
    let bRaw: number | null
    if (mode === 'budget' && budgetSubmitRaw != null) {
      aRaw = budgetSubmitRaw.a
      bRaw = budgetSubmitRaw.b
      if (
        !Number.isFinite(aRaw) ||
        !Number.isFinite(bRaw) ||
        aRaw < 0 ||
        bRaw < 0 ||
        aRaw > Number.MAX_SAFE_INTEGER ||
        bRaw > Number.MAX_SAFE_INTEGER
      ) {
        setOpenStepError('Nieprawidłowe capy z quote (przekroczony limit liczb w JS).')
        return
      }
    } else {
      aRaw = toBaseUnitsU64(Number(amountAUi), tokenA!.decimals)
      bRaw = toBaseUnitsU64(Number(amountBUi), tokenB!.decimals)
      if (aRaw === null || bRaw === null) {
        setOpenStepError('Kwoty tokenów są nieprawidłowe lub za duże (przekraczają bezpieczny zakres).')
        return
      }
    }

    // Send the same bookkeeping id across SWAP and OPEN even if the swap plan
    // becomes null after balances update.
    const openCostSessionId = swapCostSessionId ?? makeCostSessionId()
    if (!swapCostSessionId) {
      setSwapCostSessionId(openCostSessionId)
    }

    mutation.mutate({
      pool_address: poolAddress.trim(),
      tick_lower: Number(tickLower),
      tick_upper: Number(tickUpper),
      amount_a: aRaw,
      amount_b: bRaw,
      ...(strategyId.trim() ? { strategy_id: strategyId.trim() } : {}),
      cost_session_id: openCostSessionId,
    })
  }

  const handleSwapOnly = () => {
    if (!swapBeforeOpenPlan) return
    if (!poolAddress.trim()) return
    const id = makeCostSessionId()
    setSwapCostSessionId(id)
    setSwapSignature(null)
    setSwapStepInfo(null)
    setSwapStepError(null)

    swapMutation.mutate({
      pool_address: poolAddress.trim(),
      specified_mint: swapBeforeOpenPlan.specified_mint,
      amount_in: swapBeforeOpenPlan.amount_in,
      cost_session_id: id,
    })
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center gap-4">
        <Link to="/positions">
          <Button variant="ghost" size="icon">
            <ArrowLeft className="h-4 w-4" />
          </Button>
        </Link>
        <h1 className="text-3xl font-bold">{L('Otwórz pozycję', 'Open Position')}</h1>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>{L('Konfiguracja pozycji', 'Position Configuration')}</CardTitle>
        </CardHeader>
        <CardContent>
          <form className="space-y-4" onSubmit={handleSubmit}>
            <div>
              <label className="block text-sm font-medium mb-1">{L('Pula (curated)', 'Pool (curated)')}</label>
              <select
                className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                value={poolAddress}
                onChange={(e) => setPoolAddress(e.target.value)}
              >
                <option value="">{L('Wybierz parę…', 'Select pair…')}</option>
                {curatedPools.map((p) => (
                  <option key={p.address} value={p.address}>
                    {p.label}
                  </option>
                ))}
              </select>
              <div className="mt-2">
                <label className="block text-xs text-muted-foreground mb-1">{L('Adres puli', 'Pool Address')}</label>
                <input
                  className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm font-mono"
                  value={poolAddress}
                  onChange={(e) => setPoolAddress(e.target.value)}
                  placeholder={L('Adres puli Whirlpool', 'Whirlpool pool address')}
                  required
                />
              </div>
              {poolQ.isLoading ? (
                <div className="text-xs text-muted-foreground mt-2">{L('Ładuję metadane puli…', 'Loading pool metadata…')}</div>
              ) : poolQ.error ? (
                <InlineError className="mt-2">{(poolQ.error as Error).message}</InlineError>
              ) : poolQ.data ? (
                <div className="text-xs text-muted-foreground mt-2">
                  {poolQ.data.protocol.toUpperCase()} · {tokenA?.symbol ?? '…'}/
                  {tokenB?.symbol ?? '…'} · tick_spacing {poolQ.data.tick_spacing} · fee{' '}
                  {((poolQ.data.fee_rate_bps ?? 0) / 100).toFixed(2)}%
                  {orcaAQ.isLoading || orcaBQ.isLoading ? ' · Orca token meta…' : ''}
                </div>
              ) : null}
            </div>

            <div>
              <label className="block text-sm font-medium mb-1">{L('Strategia (opcjonalnie)', 'Strategy (optional)')}</label>
              <select
                className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                value={strategyId}
                onChange={(e) => setStrategyId(e.target.value)}
                disabled={strategyOptions.length === 0}
              >
                <option value="">
                  {strategyOptions.length === 0
                    ? L('Brak strategii — utwórz ją najpierw w Strategiach', 'No strategies yet — create one under Strategies')
                    : L('Brak (tylko manual)', 'None (manual only)')}
                </option>
                {strategyOptions.map((s) => (
                  <option key={s.id} value={s.id}>
                    {s.name} ({s.strategy_type.replace(/_/g, ' ')})
                  </option>
                ))}
              </select>
              <p className="text-xs text-muted-foreground mt-1">
                Pool is set above; strategy stores automation parameters. After a successful open,
                this position is linked and strategy automation starts by default. You can pause
                automation for this position later on the position detail page.
              </p>
              {strategyId.trim() && strategyRangeWidthPct != null && strategyRangeWidthPct > 0 ? (
                <p className="text-xs text-foreground/90 mt-2 rounded-md border border-border bg-muted/30 px-2 py-1.5">
                  Strategia ma <strong>Range Width {strategyRangeWidthPct}%</strong> — ticki niżej
                  można wyliczyć wokół <strong>bieżącej ceny z puli</strong> (odświeżane ~co 10 s).
                </p>
              ) : null}
            </div>

            <div className="rounded-md border border-border/60 bg-muted/10 px-3 py-2.5">
              <div className="flex flex-wrap items-center justify-between gap-3">
                <p className="text-xs text-muted-foreground">
                  Zakres ustalasz po cenach. Ticki są wyliczane automatycznie z pól „Cena dolna / Cena
                  górna”.
                </p>
                <Button
                  type="button"
                  variant="secondary"
                  size="sm"
                  onClick={() => setShowAdvancedTicks((v) => !v)}
                >
                  {showAdvancedTicks ? L('Ukryj ticki', 'Hide ticks') : L('Pokaż ticki (advanced)', 'Show ticks (advanced)')}
                </Button>
              </div>

              {showAdvancedTicks ? (
                <div className="grid gap-4 md:grid-cols-2 mt-3">
                  <div>
                    <label className="block text-sm font-medium mb-1">{L('Tick dolny', 'Tick Lower')}</label>
                    <input
                      type="number"
                      className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                      value={tickLower}
                      onChange={(e) => {
                        setTickAutoSync(false)
                        setSyncPriceInputsFromTicks(true)
                        setTickLower(e.target.value === '' ? '' : Number(e.target.value))
                      }}
                      required
                    />
                  </div>
                  <div>
                    <label className="block text-sm font-medium mb-1">{L('Tick górny', 'Tick Upper')}</label>
                    <input
                      type="number"
                      className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                      value={tickUpper}
                      onChange={(e) => {
                        setTickAutoSync(false)
                        setSyncPriceInputsFromTicks(true)
                        setTickUpper(e.target.value === '' ? '' : Number(e.target.value))
                      }}
                      required
                    />
                  </div>
                </div>
              ) : null}
            </div>

            {poolQ.data && tokenA && tokenB ? (
              <div className="rounded-md border border-border bg-muted/10 px-3 py-3 space-y-2">
                <p className="text-xs font-medium text-foreground">
                  Zakres według ceny (stosunek {tokenB.symbol} za 1 {tokenA.symbol}, jak pole „price” / tick w puli)
                </p>
                <div className="grid gap-3 md:grid-cols-2">
                  <div>
                    <label className="block text-xs text-muted-foreground mb-1">{L('Cena dolna (granica zakresu)', 'Lower price (range boundary)')}</label>
                    <input
                      type="text"
                      inputMode="decimal"
                      autoComplete="off"
                      className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm font-mono"
                      value={priceRangeLo}
                      onChange={(e) => {
                        setSyncPriceInputsFromTicks(false)
                        setPriceRangeError(null)
                        setPriceRangeLo(e.target.value)
                      }}
                      placeholder={
                        poolStateQ.data?.price != null && tokenA && tokenB
                          ? (() => {
                              const raw = Number(poolStateQ.data!.price)
                              const ui = uiPriceFromRawPriceRatio(raw, tokenA.decimals, tokenB.decimals)
                              return ui != null ? `np. ${String(ui)}` : 'np. —'
                            })()
                          : 'np. 0.0142'
                      }
                    />
                  </div>
                  <div>
                    <label className="block text-xs text-muted-foreground mb-1">{L('Cena górna (granica zakresu)', 'Upper price (range boundary)')}</label>
                    <input
                      type="text"
                      inputMode="decimal"
                      autoComplete="off"
                      className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm font-mono"
                      value={priceRangeHi}
                      onChange={(e) => {
                        setSyncPriceInputsFromTicks(false)
                        setPriceRangeError(null)
                        setPriceRangeHi(e.target.value)
                      }}
                      placeholder="wyższa niż dolna"
                    />
                  </div>
                </div>
                <div className="flex flex-wrap items-center gap-2">
                  <Button
                    type="button"
                    variant="secondary"
                    size="sm"
                    onClick={() => {
                      const parseP = (s: string) => {
                        const t = s.trim().replace(/,/g, '.')
                        if (!t) return null
                        const n = Number(t)
                        return Number.isFinite(n) && n > 0 ? n : null
                      }
                      const pl = parseP(priceRangeLo)
                      const ph = parseP(priceRangeHi)
                      const spacing = poolQ.data?.tick_spacing
                      if (pl == null || ph == null) {
                        setPriceRangeError('Podaj dwie dodatnie liczby (kropka lub przecinek jako separator).')
                        return
                      }
                      if (spacing == null) {
                        setPriceRangeError('Brak tick_spacing puli.')
                        return
                      }
                      const rawPl = rawPriceRatioFromUiPrice(pl, tokenA.decimals, tokenB.decimals)
                      const rawPh = rawPriceRatioFromUiPrice(ph, tokenA.decimals, tokenB.decimals)
                      if (rawPl == null || rawPh == null) {
                        setPriceRangeError('Nie udało się przeliczyć (sprawdź format i wartość cen).')
                        return
                      }
                      const r = alignPriceRatioToTicks(rawPl, rawPh, spacing)
                      if (!r) {
                        setPriceRangeError('Nie udało się przeliczyć — sprawdź wartości.')
                        return
                      }
                      setTickLower(r.tickLower)
                      setTickUpper(r.tickUpper)
                      setTickAutoSync(false)
                      setSyncPriceInputsFromTicks(true)
                      setPriceRangeError(null)
                    }}
                  >
                    Ustaw ticki z tych cen
                  </Button>
                  {priceRangeError ? (
                    <InlineError as="span">{priceRangeError}</InlineError>
                  ) : (
                    <span className="text-xs text-muted-foreground">
                      Ticki są wyrównane do tick spacing ({poolQ.data.tick_spacing}). Możesz też edytować ticki
                      powyżej — pola cenowe zaktualizują się automatycznie.
                    </span>
                  )}
                </div>
              </div>
            ) : null}

            
            {strategyId.trim() && strategyRangeWidthPct != null && strategyRangeWidthPct > 0 && poolQ.data ? (
              <label className="flex items-start gap-2 text-sm cursor-pointer">
                <input
                  type="checkbox"
                  className="mt-1 rounded border-input"
                  checked={tickAutoSync}
                  onChange={(e) => setTickAutoSync(e.target.checked)}
                />
                <span className="text-muted-foreground leading-snug">
                  Automatycznie aktualizuj zakres (ceny / ticki) wg aktualnego ticku puli i szerokości
                  strategii (ok. co 10 s). Wyłącz, jeśli chcesz ustawić zakres ręcznie po cenach.
                </span>
              </label>
            ) : null}

            {!priceAtTickBounds && poolAddress.trim() && poolQ.data ? (
              <p className="text-xs text-muted-foreground font-mono tabular-nums">
                Pula: price ref.{' '}
                {poolStateQ.data?.price != null && tokenA && tokenB
                  ? formatPriceRatio(
                      uiPriceFromRawPriceRatio(
                        Number(poolStateQ.data.price),
                        tokenA.decimals,
                        tokenB.decimals,
                      ) ?? Number.NaN,
                    )
                  : poolStateQ.data?.price ?? poolQ.data.price}{' '}
                · tick{' '}
                {poolStateQ.data?.current_tick ?? poolQ.data.current_tick}
                {poolStateQ.isFetching ? ' · odświeżanie…' : ''}
              </p>
            ) : null}

            <div className="rounded-md border border-border p-3 space-y-3">
              <div className="flex flex-wrap gap-3 items-center">
                <span className="text-sm font-medium">{L('Kwota', 'Amount')}</span>
                <label className="text-sm flex items-center gap-2">
                  <input
                    type="radio"
                    name="mode"
                    value="tokens"
                    checked={mode === 'tokens'}
                    onChange={() => setMode('tokens')}
                  />
                  {L('Token A/B (ręcznie)', 'Token A/B (manual)')}
                </label>
                <label className="text-sm flex items-center gap-2">
                  <input
                    type="radio"
                    name="mode"
                    value="budget"
                    checked={mode === 'budget'}
                    onChange={() => setMode('budget')}
                  />
                  {L('Wspólna kwota USD do rozdziału', 'Shared USD amount to allocate')}
                </label>
              </div>

              {mode === 'budget' && (
                <div className="space-y-3">
                  <div>
                    <label className="block text-sm font-medium mb-1">
                      {L('Docelowa wartość pozycji (USD, w zakresie)', 'Target position value (USD, in-range)')}
                    </label>
                    <input
                      type="number"
                      step="0.01"
                      className="w-full max-w-xs rounded-md border border-input bg-background px-3 py-2 text-sm"
                      value={totalUsd}
                      onChange={(e) => {
                        if (budgetLegDebounceRef.current) clearTimeout(budgetLegDebounceRef.current)
                        budgetLegAbortRef.current?.abort()
                        setBudgetLegSyncing(false)
                        setBudgetLegSyncError(null)
                        setTotalUsd(e.target.value === '' ? '' : Number(e.target.value))
                      }}
                      placeholder={L('np. 3', 'e.g. 3')}
                    />
                    {budgetTickRangeInPrice === false ? (
                      <div className="text-xs text-amber-600/90 mt-1 space-y-1">
                        <div>
                          Cena puli jest poza zakresem ticków — quote USD nie może policzyć kwot A/B. Ustaw zakres tak,
                          żeby obejmował bieżącą cenę (in-range).
                        </div>
                        <div className="flex flex-wrap items-center gap-2">
                          <Button
                            type="button"
                            variant="secondary"
                            size="sm"
                            onClick={() => {
                              const spacing = poolQ.data?.tick_spacing
                              if (spacing == null || currentTick == null) return
                              const width = strategyRangeWidthPct != null && strategyRangeWidthPct > 0 ? strategyRangeWidthPct : 2
                              const { tickLower: tl, tickUpper: tu } = calculateTickRangeFromWidthPct(
                                currentTick,
                                width,
                                spacing,
                              )
                              setTickLower(tl)
                              setTickUpper(tu)
                              setTickAutoSync(false)
                              setSyncPriceInputsFromTicks(true)
                            }}
                            disabled={currentTick == null || poolQ.data?.tick_spacing == null}
                          >
                            {L('Ustaw ticki wokół ceny', 'Set ticks around current price')}
                          </Button>
                          {currentTick != null ? (
                            <span className="text-[11px] text-muted-foreground font-mono tabular-nums">
                              current_tick={currentTick}
                            </span>
                          ) : null}
                        </div>
                      </div>
                    ) : null}
                    {budgetQuoteQ.isFetching ? (
                      <div className="text-xs text-muted-foreground mt-1">{L('Liczę kwoty A/B z krzywej puli…', 'Calculating A/B amounts from pool curve…')}</div>
                    ) : null}
                    {budgetQuoteQ.isError ? (
                      <InlineError className="mt-1">{(budgetQuoteQ.error as Error).message}</InlineError>
                    ) : null}
                    {budgetQuoteQ.data ? (
                      <div className="text-xs text-muted-foreground mt-1 space-y-0.5">
                        <div>
                          Szac. wartość przy tych cenach API:{' '}
                          <span className="font-medium text-foreground tabular-nums">
                            ~{formatUSD(budgetQuoteQ.data.estimated_value_usd)}
                          </span>{' '}
                          (capped ≤ wpisanego USD; dyskretna płynność L).
                        </div>
                        {!budgetQuoteQ.data.in_range ? (
                          <div className="text-amber-600/90">
                            Cena puli nie leży w [tick lower, tick upper) — quote może być nieważny; poszerz zakres lub
                            {L('przełącz na Token A/B.', 'switch to Token A/B.')}
                          </div>
                        ) : null}
                      </div>
                    ) : null}
                  </div>
                  <p className="text-xs text-muted-foreground leading-relaxed">
                    Zamiast dzielić USD 50/50 między tokeny, backend wylicza{' '}
                    <strong>ilości zgodne z Whirlpool</strong> przy aktualnym <code className="text-[11px]">sqrt_price</code> i
                    Twoich tickach, tak żeby <strong>notional w pozycji był blisko</strong> wpisanej kwoty (w granicach
                    zaokrągleń). Pola Amount poniżej uzupełniają się z tego quote; wysyłane są surowe capy z API.
                  </p>
                  {budgetLegSyncing || budgetLegSyncError ? (
                    <div className="text-xs space-y-1 pt-1">
                      {budgetLegSyncing ? (
                        <p className="text-muted-foreground">
                          Edycja Amount → dopasowuję <strong>docelową kwotę USD</strong> i drugą nogę (quote Orca)…
                        </p>
                      ) : null}
                      {budgetLegSyncError ? <InlineError as="p">{budgetLegSyncError}</InlineError> : null}
                    </div>
                  ) : null}
                </div>
              )}

              {poolQ.data && mintA && mintB && !effectiveOwnerPk && (
                <p className="text-xs text-amber-600/90">
                  Brak adresu portfela — salda nie będą widoczne. Ustaw{' '}
                  <code className="text-[11px]">VITE_DEV_WALLET_PUBKEY</code> albo wybierz portfel na stronie
                  Wallet.
                </p>
              )}
              {poolQ.data && mintA && mintB && usesApiSignerBalances ? (
                <p className="text-xs text-muted-foreground">
                  Walidacja sald używa portfela API signer (
                  <code className="text-[11px]">{shortenAddress(effectiveOwnerPk ?? '', 6)}</code>), bo z niego backend
                  wysyła transakcje open/swap.
                </p>
              ) : null}
              {!!effectiveBalancesQ.data?.is_stale && (
                <InlineError as="div" className="text-xs">
                  Uzywany jest ostatni znany stan portfela (stale {Math.max(0, (effectiveBalancesQ.data.stale_age_ms / 1000)).toFixed(1)}s), odswiezanie trwa w tle.
                </InlineError>
              )}

              <div className="grid gap-4 md:grid-cols-2">
                <div>
                  <label className="block text-sm font-medium mb-1">
                    Amount {tokenA?.symbol ?? 'Token A'}
                  </label>
                  {mintA && effectiveOwnerPk && (
                    <div className="text-xs text-muted-foreground mb-1.5">
                      Stan portfela:{' '}
                      {effectiveBalancesQ.isLoading ? (
                        <span>…</span>
                      ) : effectiveBalancesQ.isError ? (
                        <InlineError as="span" className="px-1.5 py-0.5">nie udało się odczytać</InlineError>
                      ) : (
                        <>
                          <span className="font-medium text-foreground tabular-nums">
                            {walletLineA?.amount}
                          </span>{' '}
                          {tokenA?.symbol ?? 'A'}
                          {walletLineA?.note ? (
                            <span className="opacity-80"> ({walletLineA.note})</span>
                          ) : null}
                        </>
                      )}
                    </div>
                  )}
                  <input
                    type="number"
                    step="0.000001"
                    className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                    value={amountAUi}
                    onChange={(e) => {
                      const raw = e.target.value
                      if (raw === '') {
                        setAmountAUi('')
                        if (budgetLegDebounceRef.current) clearTimeout(budgetLegDebounceRef.current)
                        budgetLegAbortRef.current?.abort()
                        return
                      }
                      const v = Number(raw)
                      if (!Number.isFinite(v)) {
                        setAmountAUi('')
                        return
                      }
                      setAmountAUi(v)
                      if (mode === 'budget') {
                        scheduleBudgetLegSync('a', raw)
                      }
                    }}
                    required
                  />
                  {tokenA && (
                    <div className="text-[11px] text-muted-foreground mt-1">
                      {mode === 'budget' &&
                      pricesQ.data?.prices?.[tokenA.mint] != null &&
                      amountAUi !== '' &&
                      Number.isFinite(Number(amountAUi)) ? (
                        <>
                          ≈ {formatUSD(Number(amountAUi) * pricesQ.data.prices[tokenA.mint])} USD (szac. wg {pricesQ.data.source})
                        </>
                      ) : (
                        <>raw u64 = round(amount × 10^{tokenA.decimals})</>
                      )}
                    </div>
                  )}
                </div>
                <div>
                  <label className="block text-sm font-medium mb-1">
                    Amount {tokenB?.symbol ?? 'Token B'}
                  </label>
                  {mintB && effectiveOwnerPk && (
                    <div className="text-xs text-muted-foreground mb-1.5">
                      Stan portfela:{' '}
                      {effectiveBalancesQ.isLoading ? (
                        <span>…</span>
                      ) : effectiveBalancesQ.isError ? (
                        <InlineError as="span" className="px-1.5 py-0.5">nie udało się odczytać</InlineError>
                      ) : (
                        <>
                          <span className="font-medium text-foreground tabular-nums">
                            {walletLineB?.amount}
                          </span>{' '}
                          {tokenB?.symbol ?? 'B'}
                          {walletLineB?.note ? (
                            <span className="opacity-80"> ({walletLineB.note})</span>
                          ) : null}
                        </>
                      )}
                    </div>
                  )}
                  <input
                    type="number"
                    step="0.000001"
                    className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                    value={amountBUi}
                    onChange={(e) => {
                      const raw = e.target.value
                      if (raw === '') {
                        setAmountBUi('')
                        if (budgetLegDebounceRef.current) clearTimeout(budgetLegDebounceRef.current)
                        budgetLegAbortRef.current?.abort()
                        return
                      }
                      const v = Number(raw)
                      if (!Number.isFinite(v)) {
                        setAmountBUi('')
                        return
                      }
                      setAmountBUi(v)
                      if (mode === 'budget') {
                        scheduleBudgetLegSync('b', raw)
                      }
                    }}
                    required
                  />
                  {tokenB && (
                    <div className="text-[11px] text-muted-foreground mt-1">
                      {mode === 'budget' &&
                      pricesQ.data?.prices?.[tokenB.mint] != null &&
                      amountBUi !== '' &&
                      Number.isFinite(Number(amountBUi)) ? (
                        <>
                          ≈ {formatUSD(Number(amountBUi) * pricesQ.data.prices[tokenB.mint])} USD (szac. wg {pricesQ.data.source})
                        </>
                      ) : (
                        <>raw u64 = round(amount × 10^{tokenB.decimals})</>
                      )}
                    </div>
                  )}
                </div>
              </div>

              {(tokenA && amountAUi !== '' && toBaseUnitsU64(Number(amountAUi), tokenA.decimals) === null) ||
              (tokenB && amountBUi !== '' && toBaseUnitsU64(Number(amountBUi), tokenB.decimals) === null) ? (
                <InlineError>
                  Kwota jest nieprawidłowa albo za duża (przekracza limit bezpiecznych liczb JS dla u64).
                </InlineError>
              ) : null}

              {fundingCheck.ready && fundingCheck.blocked && (
                <ErrorBanner className="py-2.5 space-y-2">
                  <p className="font-medium">Za mało tokenów na portfelu względem kwot powyżej</p>
                  {fundingCheck.shortA && fundingCheck.shortB ? (
                    <p className="text-muted-foreground">
                      Brakuje{' '}
                      <span className="font-mono tabular-nums">{fundingCheck.deficitA.toFixed(8)}</span> {tokenA?.symbol}{' '}
                      i{' '}
                      <span className="font-mono tabular-nums">{fundingCheck.deficitB.toFixed(8)}</span> {tokenB?.symbol}.
                      Doładuj portfel albo wykonaj swapy (np. z SOL), potem wróć tutaj.
                    </p>
                  ) : (
                    <>
                      {fundingCheck.shortA ? (
                        <p className="text-muted-foreground">
                          Brakuje ok.{' '}
                          <span className="font-mono tabular-nums">{fundingCheck.deficitA.toFixed(8)}</span> {tokenA?.symbol}{' '}
                          (masz {fundingCheck.haveA?.toLocaleString(undefined, { maximumFractionDigits: 8 })}, potrzeba{' '}
                          {Number(amountAUi).toLocaleString(undefined, { maximumFractionDigits: 8 })}). Zswapuj najpierw z{' '}
                          {tokenB?.symbol} → {tokenA?.symbol}, potem otwórz pozycję.
                        </p>
                      ) : null}
                      {fundingCheck.shortB ? (
                        <p className="text-muted-foreground">
                          Brakuje ok.{' '}
                          <span className="font-mono tabular-nums">{fundingCheck.deficitB.toFixed(8)}</span> {tokenB?.symbol}{' '}
                          (masz {fundingCheck.haveB?.toLocaleString(undefined, { maximumFractionDigits: 8 })}, potrzeba{' '}
                          {Number(amountBUi).toLocaleString(undefined, { maximumFractionDigits: 8 })}). Zswapuj najpierw z{' '}
                          {tokenA?.symbol} → {tokenB?.symbol}, potem otwórz pozycję.
                        </p>
                      ) : null}
                      {fundingCheck.shortOperationalSol ? (
                        <p className="text-muted-foreground">
                          Brakuje operacyjnego SOL na open (rent + fee buffer). Szacowany deficyt:{' '}
                          <span className="font-mono tabular-nums">
                            {fundingCheck.deficitOperationalSol.toFixed(6)}
                          </span>{' '}
                          SOL (native: {fundingCheck.nativeSol?.toFixed(6)} SOL, wymagane minimum po finansowaniu pozycji:{' '}
                          {((apiSignerQ.data?.min_open_lamports ?? 0) / 1e9).toFixed(6)} SOL).
                        </p>
                      ) : null}
                    </>
                  )}
                  <p className="text-xs text-muted-foreground">
                    Link otwiera Jupiter z ustawionymi mintami{pricesQ.data ? ' i szacunkową kwotą wejścia (ExactIn, +5% bufor)' : ''}.
                    Rozszerzenie portfela w tej samej przeglądarce zwykle łączy się z Jupiterem automatycznie — zostaje potwierdzić swap.
                  </p>
                  <div className="flex flex-wrap gap-2 pt-1">
                    {fundingCheck.shortA && fundingCheck.shortB ? (
                      <a
                        href={fundingCheck.jupiterGeneric}
                        target="_blank"
                        rel="noopener noreferrer"
                        className="inline-flex items-center rounded-md bg-primary px-3 py-1.5 text-xs font-medium text-primary-foreground hover:opacity-90"
                      >
                        Jupiter — swap
                      </a>
                    ) : (
                      <>
                        {fundingCheck.jupiterSwapToCoverA ? (
                          <a
                            href={fundingCheck.jupiterSwapToCoverA}
                            target="_blank"
                            rel="noopener noreferrer"
                            className="inline-flex items-center rounded-md bg-primary px-3 py-1.5 text-xs font-medium text-primary-foreground hover:opacity-90"
                          >
                            Jupiter: {tokenB?.symbol} → {tokenA?.symbol}
                          </a>
                        ) : null}
                        {fundingCheck.jupiterSwapToCoverB ? (
                          <a
                            href={fundingCheck.jupiterSwapToCoverB}
                            target="_blank"
                            rel="noopener noreferrer"
                            className="inline-flex items-center rounded-md bg-primary px-3 py-1.5 text-xs font-medium text-primary-foreground hover:opacity-90"
                          >
                            Jupiter: {tokenA?.symbol} → {tokenB?.symbol}
                          </a>
                        ) : null}
                        {fundingCheck.jupiterSwapToCoverSol ? (
                          <a
                            href={fundingCheck.jupiterSwapToCoverSol}
                            target="_blank"
                            rel="noopener noreferrer"
                            className="inline-flex items-center rounded-md bg-primary px-3 py-1.5 text-xs font-medium text-primary-foreground hover:opacity-90"
                          >
                            Jupiter: swap to SOL
                          </a>
                        ) : null}
                      </>
                    )}
                    <a
                      href="https://www.orca.so/"
                      target="_blank"
                      rel="noopener noreferrer"
                      className="inline-flex items-center rounded-md border border-border px-3 py-1.5 text-xs font-medium hover:bg-muted/60"
                    >
                      Orca
                    </a>
                  </div>
                </ErrorBanner>
              )}

              {swapBeforeOpenPlan &&
              !(fundingCheck.shortA && fundingCheck.shortB) &&
              fundingCheck.ready &&
              fundingCheck.blocked ? (
                <div className="rounded-md border border-amber-500/35 bg-amber-500/5 px-3 py-2.5 text-sm space-y-2">
                  <label className="flex items-start gap-2.5 cursor-pointer">
                    <input
                      type="checkbox"
                      className="mt-1 rounded border-input"
                      checked={swapBeforeOpen}
                      onChange={(e) => setSwapBeforeOpen(e.target.checked)}
                    />
                    <span className="text-muted-foreground leading-snug">
                      <span className="font-medium text-foreground">Swap w puli Orca przed dodaniem płynności</span>{' '}
                      ({swapBeforeOpenPlan.label}). Szacowana kwota wejścia:{' '}
                      {swapBeforeOpenAmountUi != null && swapBeforeOpenInputMeta != null ? (
                        <span className="font-medium tabular-nums">
                          ~
                          {swapBeforeOpenAmountUi.toLocaleString(undefined, {
                            maximumFractionDigits: Math.min(8, swapBeforeOpenInputMeta.decimals),
                          })}
                          {' '}
                          {swapBeforeOpenInputMeta.symbol}
                        </span>
                      ) : (
                        <span>—</span>
                      )}{' '}
                      (limit: do ~92% dostępnego salda).
                    </span>
                  </label>
                  {swapBeforeOpenEstimate ? (
                    <div className="text-xs text-foreground/90 rounded-md border border-amber-500/25 bg-amber-500/10 px-2 py-1.5">
                      Plan (szacunek z {swapBeforeOpenEstimate.priceSource}): swap{' '}
                      <span className="font-medium tabular-nums">
                        ~{swapBeforeOpenEstimate.inUi.toLocaleString(undefined, { maximumFractionDigits: 8 })}{' '}
                        {swapBeforeOpenEstimate.inSym}
                      </span>{' '}
                      (≈ <span className="font-medium tabular-nums">{formatUSD(swapBeforeOpenEstimate.inUsd)}</span>) → ~{' '}
                      <span className="font-medium tabular-nums">
                        {swapBeforeOpenEstimate.outUi.toLocaleString(undefined, { maximumFractionDigits: 8 })}{' '}
                        {swapBeforeOpenEstimate.outSym}
                      </span>{' '}
                      (≈ <span className="font-medium tabular-nums">{formatUSD(swapBeforeOpenEstimate.outUsd)}</span>).{' '}
                      {swapBeforeOpenEstimate.deficitOutUi > 0 ? (
                        <>
                          Deficyt: <span className="font-medium tabular-nums">
                            {swapBeforeOpenEstimate.deficitOutUi.toLocaleString(undefined, { maximumFractionDigits: 8 })}{' '}
                            {swapBeforeOpenEstimate.outSym}
                          </span>.
                        </>
                      ) : null}
                    </div>
                  ) : null}
                  <p className="text-xs text-muted-foreground">
                    Ten krok wykona transakcję swapu w tej samej puli Orca na sieci Solana (backend).
                    Dopiero po potwierdzeniu będzie można otworzyć pozycję.
                  </p>
                  {swapCostEstimateQ.isLoading ? (
                    <p className="text-xs text-muted-foreground">Szacowanie opłaty sieciowej swapu…</p>
                  ) : null}
                  {swapCostEstimateQ.data ? (
                    <p className="text-xs text-foreground/90">
                      Szacowany koszt sieciowy swapu (Solana <code className="text-[11px]">meta.fee</code>): ~{' '}
                      {(swapCostEstimateQ.data.estimated_network_fee_lamports / 1e9).toLocaleString(undefined, {
                        maximumFractionDigits: 6,
                      })}{' '}
                      SOL ({swapCostEstimateQ.data.estimated_network_fee_lamports.toLocaleString()} lamportów
                      {swapCostEstimateQ.data.historical_sample_count > 0
                        ? ` — mediana z ${swapCostEstimateQ.data.historical_sample_count} wcześniejszych swapów w tej puli`
                        : ' — brak historii lokalnej, wartość ostrożna domyślna'}
                      ).
                    </p>
                  ) : null}
                  {swapSignature ? (
                    <p className="text-xs text-foreground/90">
                      Swap potwierdzony: <code className="text-[11px] bg-muted px-1 rounded">{shortenAddress(swapSignature, 6)}</code>
                    </p>
                  ) : null}
                </div>
              ) : null}
            </div>

            {(swapSignature || swapStepInfo || swapStepError) && (
              <div className="pt-2 space-y-2">
                {swapSignature ? (
                  <div className="rounded-md border border-emerald-600/40 bg-emerald-950/20 px-3 py-2 text-xs text-emerald-200 break-all">
                    <span className="font-medium">Swap potwierdzony:</span>{' '}
                    <code className="text-[11px] bg-muted/50 px-1 rounded">{swapSignature}</code>
                  </div>
                ) : null}
                {swapStepInfo ? (
                  <div className="rounded-md border border-border/50 bg-muted/30 px-3 py-2 text-xs text-foreground/90 break-words">
                    {swapStepInfo}
                  </div>
                ) : null}
                {swapStepError ? (
                  <ErrorBanner className="text-xs break-words">
                    {swapStepError}
                  </ErrorBanner>
                ) : null}
              </div>
            )}

            {openStepError ? (
              <ErrorBanner className="text-xs">
                <div className="font-medium">{L('Otwarcie nieudane', 'Open failed')}</div>
                <div className="break-words">
                  {openStepError.length > 220 ? `${openStepError.slice(0, 220)}…` : openStepError}
                </div>
                {openStepError.toLowerCase().includes('insufficient sol') ? (
                  <div className="mt-2 text-[11px] text-muted-foreground">
                    Tip: możesz szybko podmienić tokeny na SOL w Jupiterze. Wejdź w menu <strong>Swap</strong> albo użyj Jupitera:
                    {' '}
                    <a
                      className="underline underline-offset-2 hover:opacity-90"
                      href="https://jup.ag/swap?outputMint=So11111111111111111111111111111111111111112"
                      target="_blank"
                      rel="noopener noreferrer"
                    >
                      swap to SOL
                    </a>
                    .
                  </div>
                ) : null}
                {openStepError.length > 220 ? (
                  <details className="mt-2">
                    <summary className="cursor-pointer select-none text-[11px] text-destructive-foreground/90">
                      Show details
                    </summary>
                    <pre className="mt-2 whitespace-pre-wrap break-words rounded bg-muted/40 p-2 text-[11px] text-foreground/80">
                      {openStepError}
                    </pre>
                  </details>
                ) : null}
              </ErrorBanner>
            ) : null}

            <div className="flex justify-end gap-2 pt-2">
              <Link to="/positions">
                <Button variant="outline" type="button">
                  {L('Anuluj', 'Cancel')}
                </Button>
              </Link>
              {swapBeforeOpen && swapBeforeOpenPlan && !swapSignature ? (
                <Button
                  type="button"
                  disabled={swapMutation.isPending}
                  onClick={handleSwapOnly}
                >
                  {swapMutation.isPending ? 'Swapping...' : 'Swap'}
                </Button>
              ) : (
                <Button
                  type="submit"
                  disabled={
                    mutation.isPending ||
                    (fundingCheck.ready &&
                      fundingCheck.blocked &&
                      swapBeforeOpen &&
                      !swapSignature)
                  }
                >
                  {mutation.isPending ? L('Otwieranie...', 'Opening...') : L('Otwórz pozycję', 'Open Position')}
                </Button>
              )}
            </div>
          </form>
        </CardContent>
      </Card>
    </div>
  )
}

