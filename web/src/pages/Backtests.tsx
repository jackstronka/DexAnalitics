import { useMemo, useState } from 'react'
import { useMutation, useQuery } from '@tanstack/react-query'
import {
  getBacktestAutoTuneStatus,
  getBacktestFullJob,
  getPools,
  getBacktestStrategyCatalog,
  startBacktestAutoTune,
  startBacktestFull,
  stopBacktestAutoTune,
  type BacktestFullWindowResult,
  type Pool,
} from '@/lib/api'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'

const WINDOWS = [24, 48, 72, 96]
const POOLS = [
  { id: 'ORCA_SOL_USDC', label: 'Orca SOL/USDC' },
  { id: 'ORCA_WHETH_SOL', label: 'Orca whETH/SOL' },
  { id: 'ORCA_CBBTC_USDC', label: 'Orca cbBTC/USDC' },
  { id: 'ORCA_CBBTC_WBTC', label: 'Orca cbBTC/WBTC' },
  { id: 'RAYDIUM_SOL_USDT', label: 'Raydium SOL/USDT' },
  { id: 'METEORA_SOL_USDC_S1', label: 'Meteora SOL/USDC Step1' },
  { id: 'METEORA_SOL_USDC_S4', label: 'Meteora SOL/USDC Step4' },
  { id: 'METEORA_SOL_USDC_S10', label: 'Meteora SOL/USDC Step10' },
]
const POOL_ADDRESS_BY_ID: Record<string, string> = {
  ORCA_SOL_USDC: 'Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE',
  ORCA_WHETH_SOL: 'HktfL7iwGKT5QHjywQkcDnZXScoh811k7akrMZJkCcEF',
  ORCA_CBBTC_USDC: 'HxA6SKW5qA4o12fjVgTpXdq2YnZ5Zv1s7SB4FFomsyLM',
  ORCA_CBBTC_WBTC: '4v8ufj8Hj7UvFgtofQJAtzUud5xomwZfEqfCTHZ4wM72',
  RAYDIUM_SOL_USDT: '3nMFwZXwY1s1M5s8vYAHqd4wGs4iSxXE4LRoUMMYqEgF',
  METEORA_SOL_USDC_S1: 'HTvjzsfX3yU6BUodCjZ5vZkUrAxMDTrBs3CJaq43ashR',
  METEORA_SOL_USDC_S4: '5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6',
  METEORA_SOL_USDC_S10: 'BGm1tav58oGcsQJehL9WXBFXF7D27vZsKefj4xJKD5Y',
}

const OBJECTIVES = [
  {
    id: 'vs-hodl',
    label: 'vs-hodl (domyslny)',
    description:
      'Maksymalizuje przewage LP nad HODL. Najlepszy do wyboru strategii, gdy chcesz pobic pasywne trzymanie.',
    useCase:
      'Uzyj jako baseline do porownania, czy LP ma sens vs zwykle trzymanie tokenow.',
  },
  {
    id: 'fees',
    label: 'fees',
    description:
      'Maksymalizuje same fee, bez bezposredniego celu na PnL/vs-HODL.',
    useCase:
      'Uzyj, gdy priorytetem jest capture fee i chcesz testowac bardziej fee-seeking konfiguracje.',
  },
  {
    id: 'pnl',
    label: 'pnl',
    description:
      'Maksymalizuje bezwzgledny PnL LP (finalna wartosc minus kapital poczatkowy).',
    useCase:
      'Uzyj, gdy oceniasz strategię po nominalnym wyniku USD, niezaleznie od benchmarku HODL.',
  },
  {
    id: 'composite',
    label: 'composite',
    description:
      'Score mieszany: fee pomniejszone o kare za IL i koszty rebalansow (zaleznie od alpha).',
    useCase:
      'Uzyj, gdy chcesz kompromisu: zarabianie fee, ale z kontrola kosztow i dragu IL.',
  },
  {
    id: 'risk-adj',
    label: 'risk-adj',
    description:
      'Ranking risk-adjusted: preferuje stabilniejsze profile wzgledem obsuniecia (drawdown).',
    useCase:
      'Uzyj przy bardziej konserwatywnym profilu i porownaniu strategii o podobnym zysku.',
  },
] as const

function fmt(n: number | undefined | null, d = 2): string {
  if (n == null || Number.isNaN(n)) return '—'
  return n.toFixed(d)
}

function pnlPct(pnlUsd: number, initialCapitalUsd: number | null): number | null {
  if (initialCapitalUsd == null || !Number.isFinite(initialCapitalUsd) || initialCapitalUsd <= 0) {
    return null
  }
  return (pnlUsd / initialCapitalUsd) * 100
}

/** USD z separatorem tysięcy — szybsze skanowanie niż same cyfry. */
function fmtMoneyUsd(n: number | undefined | null): string {
  if (n == null || Number.isNaN(n)) return '—'
  return new Intl.NumberFormat('pl-PL', {
    style: 'currency',
    currency: 'USD',
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  }).format(n)
}

function pnlToneClass(n: number): string {
  if (n > 0) return 'text-emerald-600 dark:text-emerald-400'
  if (n < 0) return 'text-red-600 dark:text-red-400'
  return 'text-muted-foreground'
}

function fmtSignedPct(n: number | null, d = 2): string {
  if (n == null || Number.isNaN(n)) return '—'
  const abs = Math.abs(n).toFixed(d)
  if (n > 0) return `+${abs}%`
  if (n < 0) return `-${abs}%`
  return `${Number(n).toFixed(d)}%`
}

function parseOptionalNumber(raw: string): number | undefined {
  if (raw.trim() === '') return undefined
  const v = Number(raw)
  if (!Number.isFinite(v)) return undefined
  return v
}

/** Values for API `Vec<f64>` fields (threshold %, bollinger k). */
function parseCsvFloats(raw: string): number[] | undefined {
  const arr = raw
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean)
    .map((s) => Number(s))
    .filter((n) => Number.isFinite(n))
  return arr.length > 0 ? arr : undefined
}

/**
 * Values for API `Vec<u64>` grid fields. Non-integers are skipped
 * so JSON never sends floats into integer slots (Axum returns 422).
 */
function parseCsvUInt64s(raw: string): number[] | undefined {
  const arr: number[] = []
  for (const part of raw.split(',')) {
    const s = part.trim()
    if (!s) continue
    const n = Number(s)
    if (!Number.isFinite(n) || !Number.isInteger(n) || n < 0) continue
    if (n > Number.MAX_SAFE_INTEGER) continue
    arr.push(n)
  }
  return arr.length > 0 ? arr : undefined
}

function strategyTooltip(strategy: string): string | undefined {
  const boll = strategy.match(/^bollinger_w(\d+)_k([0-9.]+)_r(\d+)$/i)
  if (boll) {
    const [, w, k, r] = boll
    return `Bollinger: w=${w} (okno), k=${k} (szerokosc pasma), r=${r} (kroki między rebalance).`
  }
  const last = strategy.match(/^last[_-]?candle[_-]?c(\d+)[_-]?r(\d+)$/i)
  if (last) {
    const [, c, r] = last
    return `Last candle: c=${c} (kroki swiecy), r=${r} (kroki między rebalance).`
  }
  const thr = strategy.match(/^threshold[_-]?([0-9.]+)%?$/i)
  if (thr) {
    const [, t] = thr
    return `Threshold: próg ruchu ${t}% do rebalansu.`
  }
  const periodicLegacy = strategy.match(/^periodic_steps_(\d+)$/i)
  if (periodicLegacy) {
    const [, n] = periodicLegacy
    return `Periodic (legacy): rebalance co ${n} kroków danych (step-based), bez zegara wall-clock.`
  }
  const periodicWall = strategy.match(/^periodic_(\d+)h?$/i)
  if (periodicWall) {
    const [, h] = periodicWall
    return `Periodic (domyślny): rebalance co ${h}h elapsed wall-clock (na timestampach), zgodnie z logiką bota.`
  }
  return undefined
}

function periodicModeBadge(strategy: string): { label: string; title: string } | null {
  if (/^periodic_steps_\d+$/i.test(strategy)) {
    return {
      label: 'legacy',
      title: 'Legacy periodic step-based: trigger co N kroków danych.',
    }
  }
  if (/^periodic_\d+h?$/i.test(strategy)) {
    return {
      label: 'wall-clock',
      title: 'Domyślny periodic: trigger po elapsed czasie (godziny) z timestampów.',
    }
  }
  return null
}

type TopSortBy = 'vs_hodl' | 'score' | 'pnl' | 'fees'

type StrategyHelp = {
  what: string
  trigger: string
  whenToUse: string
  risk: string
}

type GridPreset = {
  name: 'Ultra-safe' | 'Conservative' | 'Balanced' | 'Aggressive' | 'Scalper'
  thresholdGridPct: string
  periodicGridSteps: string
  bollingerWindowGrid: string
  bollingerKGrid: string
  bollingerRebalanceStepsGrid: string
  lastCandleSecondsGrid: string
  lastCandleRebalanceSecondsGrid: string
}

function median(values: number[]): number | null {
  if (values.length === 0) return null
  const sorted = [...values].sort((a, b) => a - b)
  const mid = Math.floor(sorted.length / 2)
  if (sorted.length % 2 === 0) return (sorted[mid - 1] + sorted[mid]) / 2
  return sorted[mid]
}

function rangeLabel(lowerUsd: number, upperUsd: number): string {
  return `$${fmt(lowerUsd)} - $${fmt(upperUsd)}`
}

const STRATEGY_HELP: Record<string, StrategyHelp> = {
  static: {
    what: 'Staly zakres, bez aktywnego rebalansowania.',
    trigger: 'Brak triggera; pozycja pozostaje na starcie.',
    whenToUse: 'Benchmark i spokojny rynek.',
    risk: 'Dlugie wyjscie poza range ogranicza fee.',
  },
  oor_recenter: {
    what:
      'W symulacji FULL: pelne recentrowanie pasma wokol biezacej ceny (±width_pct), na kazdym kroku gdy cena jest poza pasmem — moze byc wiele rebalansow z rzedu, jesli cena „ucieka” dalej po kazdym centrowaniu.',
    trigger: 'Kazdy krok snapshotu z pozycja out-of-range (float A/B).',
    whenToUse: 'Trendy / silne wybicia — chcesz doganiac rynek pelnym pasmem.',
    risk: 'Wiecej tx niz retouch przy dlugim OOR; koszty rebalansu rosna.',
  },
  threshold: {
    what: 'Rebalance przy przekroczeniu progu odchylenia.',
    trigger: 'Ruch ceny > threshold_pct.',
    whenToUse: 'Rynek z czytelnymi impulsami.',
    risk: 'W chopie moze robic zbyt wiele rebalansow.',
  },
  periodic: {
    what: 'Rebalance co zadany interwal godzinowy (wall-clock, jak bot).',
    trigger: 'Uplyw period_hours (na timestampach snapshotow).',
    whenToUse: 'Stala, przewidywalna automatyzacja zgodna z live botem.',
    risk: 'Moze handlowac w niekorzystnym momencie; przy bardzo nieregularnych danych trigger zalezy od czasu, nie liczby rekordow.',
  },
  il_limit: {
    what: 'Ogranicza drawdown IL przez limity i domkniecie.',
    trigger: 'IL przekracza max_il_pct / close_il_pct.',
    whenToUse: 'Konserwatywne podejscie i ochrona kapitalu.',
    risk: 'Czeste resety moga obciac fee edge.',
  },
  retouch_shift: {
    what:
      'W symulacji FULL: przesuwa tylko krawedz „wyjscia”, zachowujac szerokosc pasma w jednostkach A/B; maks. jeden retouch na epizod OOR (po retouchu cena trafia na krawedz — zwykle wraca in-range), kolejny dopiero po ponownym wejsciu w pasmo i kolejnym wyjsciu.',
    trigger:
      'OOR gdy `retouch_armed` (bron po powrocie in-range). Dodatkowo `retouch_offset_pct` przesuwa caly nowy zakres wzgledem ceny OOR.',
    whenToUse: 'Mean-reversion; inna geometria niz pelne centrowanie oor_recenter.',
    risk:
      'Przy dlugim trendzie bez powrotu in-range moze zostac OOR bez kolejnych retouchy (do nastepnego cyklu armed). Przy krotkim pojedynczym OOR wyniki moga byc zblizone do oor — zobacz kolumne Rebals i szerokosc pasma.',
  },
  bollinger: {
    what: 'Zakres oparty o SMA ± k*sigma (zmiennosc).',
    trigger: 'Rebalance co rebalance_steps, pasmo zalezne od window i k.',
    whenToUse: 'Zmiennosc zmienna w czasie.',
    risk: 'W silnym trendzie potrafi przegrywac z HODL.',
  },
  last_candle: {
    what: 'Zakres z high/low ostatniej swiecy.',
    trigger: 'Aktualizacja po nowej swiecy + rebalance interval.',
    whenToUse: 'Rynek z lokalnymi swingami i czytelnym rytmem.',
    risk: 'Wybicie poza swiece moze szybko zestarzec range.',
  },
}

const PARAM_GLOSSARY: Array<{ key: string; meaning: string }> = [
  { key: 'width_pct', meaning: 'Szerokosc zakresu; wyzsza = mniej triggerow, nizsza koncentracja.' },
  { key: 'threshold_pct', meaning: 'Prog odchylenia ceny wyzwalajacy rebalance.' },
  { key: 'period_hours', meaning: 'Interwal czasu (godziny) miedzy rebalance w strategii periodic.' },
  { key: 'max_il_pct', meaning: 'Poziom IL, przy ktorym strategia zaczyna reagowac.' },
  { key: 'close_il_pct', meaning: 'Silniejszy prog IL wymuszajacy domkniecie/reset.' },
  { key: 'grace_steps', meaning: 'Okres karencji po starcie przed aktywacja limitu IL.' },
  { key: 'window', meaning: 'Dlugosc okna statystycznego (np. Bollinger).' },
  { key: 'k', meaning: 'Mnoznik odchylenia standardowego w Bollinger.' },
  { key: 'rebalance_steps', meaning: 'Co ile krokow odswiezac decyzje/przebudowe.' },
  { key: 'candle_steps / candle_seconds', meaning: 'Rozmiar swiecy dla strategii last_candle.' },
  {
    key: 'IL (classical, ex-fees)',
    meaning:
      'Klasyczny IL: porownanie wartosci LP (bez fee) do HODL tych samych tokenow przy biezacej cenie.',
  },
  {
    key: 'IL-like (backtest accounting)',
    meaning:
      'Metryka ksiegowa backtestu: under/over-performance LP vs HODL (ex-fees), uzywana do rankingu i porownan strategii.',
  },
]

const GRID_PRESETS: GridPreset[] = [
  {
    name: 'Ultra-safe',
    thresholdGridPct: '10,15,20',
    periodicGridSteps: '72,96,144',
    bollingerWindowGrid: '30,40',
    bollingerKGrid: '2.5,3.0',
    bollingerRebalanceStepsGrid: '48,72',
    lastCandleSecondsGrid: '1800,3600,7200',
    lastCandleRebalanceSecondsGrid: '14400,43200,86400',
  },
  {
    name: 'Conservative',
    thresholdGridPct: '7,10,15',
    periodicGridSteps: '48,72',
    bollingerWindowGrid: '20,30',
    bollingerKGrid: '2.0,2.5',
    bollingerRebalanceStepsGrid: '48',
    lastCandleSecondsGrid: '1800,3600',
    lastCandleRebalanceSecondsGrid: '3600,14400,43200',
  },
  {
    name: 'Balanced',
    thresholdGridPct: '3,5,7,10',
    periodicGridSteps: '24,48,72',
    bollingerWindowGrid: '20',
    bollingerKGrid: '1.5,2.0,2.5',
    bollingerRebalanceStepsGrid: '24,48',
    lastCandleSecondsGrid: '900,1800,2700,3600',
    lastCandleRebalanceSecondsGrid: '1800,3600,14400,43200',
  },
  {
    name: 'Aggressive',
    thresholdGridPct: '2,3,5,7',
    periodicGridSteps: '12,24,48',
    bollingerWindowGrid: '10,20',
    bollingerKGrid: '1.0,1.5,2.0',
    bollingerRebalanceStepsGrid: '12,24',
    lastCandleSecondsGrid: '900,1800,2700',
    lastCandleRebalanceSecondsGrid: '900,1800,2700,3600,14400',
  },
  {
    name: 'Scalper',
    thresholdGridPct: '1,1.5,2,3',
    periodicGridSteps: '6,12,24',
    bollingerWindowGrid: '8,10,14',
    bollingerKGrid: '0.8,1.0,1.5',
    bollingerRebalanceStepsGrid: '6,12,24',
    lastCandleSecondsGrid: '300,600,900',
    lastCandleRebalanceSecondsGrid: '300,600,900,1800',
  },
]

function strategyFamily(strategy: string): string {
  if (strategy.startsWith('bollinger_')) return 'bollinger'
  if (strategy.startsWith('last_candle_')) return 'last_candle'
  if (strategy.startsWith('threshold_')) return 'threshold'
  if (strategy.startsWith('periodic_')) return 'periodic'
  if (strategy.startsWith('il_limit_')) return 'il_limit'
  if (strategy === 'static') return 'static'
  if (strategy === 'oor_recenter') return 'oor_recenter'
  if (strategy === 'retouch_shift') return 'retouch_shift'
  return 'other'
}

export default function Backtests() {
  const [selectedWindows, setSelectedWindows] = useState<number[]>([...WINDOWS])
  const [selectedPools, setSelectedPools] = useState<string[]>(POOLS.map((p) => p.id))
  const [selectedStrategies, setSelectedStrategies] = useState<string[]>([])
  const [includeIndicators, setIncludeIndicators] = useState(true)
  const [objective, setObjective] = useState('vs-hodl')
  const [lpShare, setLpShare] = useState('')
  const [capitalUsd, setCapitalUsd] = useState('8000')
  const [targetVsHodlUsd, setTargetVsHodlUsd] = useState('')
  const [autoTuneIntervalMinutes, setAutoTuneIntervalMinutes] = useState('30')
  const [staticDeviationPct, setStaticDeviationPct] = useState('')
  const [staticManualLower, setStaticManualLower] = useState('')
  const [staticManualUpper, setStaticManualUpper] = useState('')
  const [oorRecenterDeviationPct, setOorRecenterDeviationPct] = useState('')
  const [topSortBy, setTopSortBy] = useState<TopSortBy>('vs_hodl')
  const [thresholdGridPct, setThresholdGridPct] = useState('1,2,3,4')
  const [thresholdMinRebalanceIntervalHours, setThresholdMinRebalanceIntervalHours] = useState('0')
  const [thresholdRebalanceOnRangeExitImmediately, setThresholdRebalanceOnRangeExitImmediately] =
    useState(true)
  /** CLI/back-end: periodic hours grid (u64). */
  const [periodicGridSteps, setPeriodicGridSteps] = useState('12,24,48,72')
  const [retouchOffsetPct, setRetouchOffsetPct] = useState('0')
  const [bollingerWindowGrid, setBollingerWindowGrid] = useState('20')
  const [bollingerKGrid, setBollingerKGrid] = useState('1,1.5,2,0.2,5')
  const [bollingerRebalanceStepsGrid, setBollingerRebalanceStepsGrid] = useState('24,48')
  const [lastCandleSecondsGrid, setLastCandleSecondsGrid] = useState('900,1800,2700,3600')
  const [lastCandleRebalanceSecondsGrid, setLastCandleRebalanceSecondsGrid] = useState(
    '900,1800,2700,3600,14400,43200',
  )
  const [jobId, setJobId] = useState<string | null>(null)
  const [lastRunCapitalUsd, setLastRunCapitalUsd] = useState<number | null>(8000)
  const poolsQ = useQuery({
    queryKey: ['pools'],
    queryFn: getPools,
  })

  const catalogQ = useQuery({
    queryKey: ['backtests-strategy-catalog'],
    queryFn: getBacktestStrategyCatalog,
  })

  const fullRunMut = useMutation({
    mutationFn: startBacktestFull,
    onSuccess: (r) => setJobId(r.id),
  })

  const autoTuneStatusQ = useQuery({
    queryKey: ['backtests-auto-tune-status'],
    queryFn: getBacktestAutoTuneStatus,
    refetchInterval: 10_000,
  })

  const startAutoTuneMut = useMutation({
    mutationFn: startBacktestAutoTune,
    onSuccess: () => {
      void autoTuneStatusQ.refetch()
    },
  })
  const stopAutoTuneMut = useMutation({
    mutationFn: stopBacktestAutoTune,
    onSuccess: () => {
      void autoTuneStatusQ.refetch()
    },
  })

  const jobQ = useQuery({
    queryKey: ['backtests-full-job', jobId],
    queryFn: () => getBacktestFullJob(jobId || ''),
    enabled: !!jobId,
    refetchInterval: (q) => {
      const s = q.state.data?.status
      return s === 'running' ? 3000 : false
    },
  })

  const strategies = catalogQ.data?.strategies ?? []
  const selectedStrategySet = useMemo(
    () => new Set(selectedStrategies),
    [selectedStrategies],
  )
  const selectedPoolsIncludeMeteora = useMemo(
    () => selectedPools.some((id) => id.startsWith('METEORA_')),
    [selectedPools],
  )
  const singlePoolSelected = selectedPools.length === 1
  const staticManualActive = singlePoolSelected
  const staticManualLowerNum = parseOptionalNumber(staticManualLower)
  const staticManualUpperNum = parseOptionalNumber(staticManualUpper)
  const staticManualReady =
    staticManualActive &&
    staticManualLowerNum !== undefined &&
    staticManualUpperNum !== undefined &&
    staticManualLowerNum > 0 &&
    staticManualUpperNum > staticManualLowerNum
  const selectedObjective = useMemo(
    () => OBJECTIVES.find((o) => o.id === objective) ?? OBJECTIVES[0],
    [objective],
  )

  const sortedResults = useMemo(() => {
    const items = (jobQ.data?.results ?? []) as BacktestFullWindowResult[]
    return [...items].sort((a, b) => {
      const byPool = a.pool_label.localeCompare(b.pool_label)
      if (byPool !== 0) return byPool
      return a.window_hours - b.window_hours
    })
  }, [jobQ.data?.results])

  const poolByAddress = useMemo(() => {
    const map = new Map<string, Pool>()
    for (const p of poolsQ.data?.pools ?? []) {
      map.set((p.address ?? '').trim(), p)
    }
    return map
  }, [poolsQ.data?.pools])

  const liquidityRegime = useMemo(() => {
    return sortedResults.map((r) => {
      const addr = POOL_ADDRESS_BY_ID[r.pool_id] ?? r.pool_address
      const p = poolByAddress.get(addr)
      const v24 = Number(p?.volume_24h_usd ?? 0)
      const v1 = Number(p?.volume_1h_usd ?? 0)
      const approxWindowVol = Number.isFinite(v24) ? (v24 * r.window_hours) / 24 : 0
      const ratio1h24h = v24 > 0 ? v1 / (v24 / 24) : 0
      const label = approxWindowVol < 100_000 ? 'niska' : approxWindowVol < 1_000_000 ? 'srednia' : 'wysoka'
      return {
        key: `${r.pool_id}-${r.window_hours}`,
        poolLabel: r.pool_label,
        windowHours: r.window_hours,
        approxWindowVol,
        ratio1h24h,
        label,
      }
    })
  }, [sortedResults, poolByAddress])

  const qualifyingTop3 = useMemo(
    () =>
      sortedResults.map((r) => ({
        key: `${r.pool_id}-${r.window_hours}`,
        poolLabel: r.pool_label,
        windowHours: r.window_hours,
        families: (() => {
          const grouped = new Map<string, typeof r.metrics>()
          for (const m of r.metrics) {
            const fam = strategyFamily(m.strategy)
            const cur = grouped.get(fam) ?? []
            cur.push(m)
            grouped.set(fam, cur)
          }
          return [...grouped.entries()]
            .sort((a, b) => a[0].localeCompare(b[0]))
            .map(([family, items]) => ({
              family,
              items: [...items]
                .sort((a, b) => {
                  if (topSortBy === 'score') return b.score - a.score
                  if (topSortBy === 'pnl') return b.pnl - a.pnl
                  if (topSortBy === 'fees') return b.fees - a.fees
                  return b.vs_hodl - a.vs_hodl
                })
                // Keep only the best variant per strategy label in a family.
                .reduce<typeof items>((acc, cur) => {
                  if (!acc.some((m) => m.strategy === cur.strategy)) acc.push(cur)
                  return acc
                }, [])
                .slice(0, 3),
            }))
        })(),
      })),
    [sortedResults, topSortBy],
  )

  const globalTop = useMemo(() => {
    type Agg = {
      strategy: string
      appearances: number
      wins: number
      sumVsHodl: number
      sumScore: number
      sumPnl: number
      sumFees: number
      bestVsHodl: number
    }
    const map = new Map<string, Agg>()
    for (const r of sortedResults) {
      for (const m of r.metrics) {
        const prev = map.get(m.strategy) ?? {
          strategy: m.strategy,
          appearances: 0,
          wins: 0,
          sumVsHodl: 0,
          sumScore: 0,
          sumPnl: 0,
          sumFees: 0,
          bestVsHodl: Number.NEGATIVE_INFINITY,
        }
        prev.appearances += 1
        if (m.rank === 1) prev.wins += 1
        prev.sumVsHodl += m.vs_hodl
        prev.sumScore += m.score
        prev.sumPnl += m.pnl
        prev.sumFees += m.fees
        prev.bestVsHodl = Math.max(prev.bestVsHodl, m.vs_hodl)
        map.set(m.strategy, prev)
      }
    }
    const arr = [...map.values()].map((a) => ({
      strategy: a.strategy,
      appearances: a.appearances,
      wins: a.wins,
      avgVsHodl: a.appearances > 0 ? a.sumVsHodl / a.appearances : 0,
      avgScore: a.appearances > 0 ? a.sumScore / a.appearances : 0,
      avgPnl: a.appearances > 0 ? a.sumPnl / a.appearances : 0,
      avgFees: a.appearances > 0 ? a.sumFees / a.appearances : 0,
      bestVsHodl: a.bestVsHodl,
    }))
    arr.sort((a, b) => {
      if (topSortBy === 'score') return b.avgScore - a.avgScore
      if (topSortBy === 'pnl') return b.avgPnl - a.avgPnl
      if (topSortBy === 'fees') return b.avgFees - a.avgFees
      return b.avgVsHodl - a.avgVsHodl
    })
    return arr.slice(0, 10)
  }, [sortedResults, topSortBy])

  const windowStats = useMemo(() => {
    const byWindow = new Map<number, number[]>()
    for (const r of sortedResults) {
      const cur = byWindow.get(r.window_hours) ?? []
      for (const m of r.metrics) cur.push(m.vs_hodl)
      byWindow.set(r.window_hours, cur)
    }
    return [...byWindow.entries()]
      .map(([windowHours, vsHodlValues]) => {
        const target = parseOptionalNumber(targetVsHodlUsd) ?? Number.NEGATIVE_INFINITY
        const qualifiedCount = vsHodlValues.filter((v) => v >= target).length
        return {
          windowHours,
          totalCount: vsHodlValues.length,
          qualifiedCount,
          medianVsHodl: median(vsHodlValues),
        }
      })
      .sort((a, b) => a.windowHours - b.windowHours)
  }, [sortedResults, targetVsHodlUsd])

  const toggleSelection = <T extends string | number>(
    list: T[],
    value: T,
    setter: (v: T[]) => void,
  ) => {
    if (list.includes(value)) setter(list.filter((x) => x !== value))
    else setter([...list, value])
  }

  const applyPreset = (preset: GridPreset) => {
    setThresholdGridPct(preset.thresholdGridPct)
    setPeriodicGridSteps(preset.periodicGridSteps)
    setBollingerWindowGrid(preset.bollingerWindowGrid)
    setBollingerKGrid(preset.bollingerKGrid)
    setBollingerRebalanceStepsGrid(preset.bollingerRebalanceStepsGrid)
    setLastCandleSecondsGrid(preset.lastCandleSecondsGrid)
    setLastCandleRebalanceSecondsGrid(preset.lastCandleRebalanceSecondsGrid)
  }

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader>
          <CardTitle>Backtests</CardTitle>
          <CardDescription>
            FULL porownanie strategii i parametrow dla okien 24/48/72/96h.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-6">
          <div className="space-y-2">
            <div className="text-base font-medium">Okna czasowe (h)</div>
            <div className="flex flex-wrap gap-4">
              {WINDOWS.map((h) => (
                <label key={h} className="flex items-center gap-2 text-sm">
                  <input
                    type="checkbox"
                    checked={selectedWindows.includes(h)}
                    onChange={() =>
                      toggleSelection(selectedWindows, h, setSelectedWindows)
                    }
                  />
                  {h}h
                </label>
              ))}
            </div>
          </div>

          <div className="space-y-2">
            <div className="text-base font-medium">Pary</div>
            <div className="grid gap-2 md:grid-cols-2">
              {POOLS.map((p) => (
                <label key={p.id} className="flex items-center gap-2 text-sm">
                  <input
                    type="checkbox"
                    checked={selectedPools.includes(p.id)}
                    onChange={() =>
                      toggleSelection(selectedPools, p.id, setSelectedPools)
                    }
                  />
                  {p.label}
                </label>
              ))}
            </div>
          </div>

          <div className="space-y-2">
            <div className="text-base font-medium">Strategie (rodziny + parametry)</div>
            <div className="grid gap-2 md:grid-cols-2">
              {strategies.map((s) => (
                <label key={s.id} className="flex items-start gap-2 text-sm">
                  <input
                    type="checkbox"
                    checked={selectedStrategySet.has(s.id)}
                    onChange={() =>
                      toggleSelection(selectedStrategies, s.id, setSelectedStrategies)
                    }
                  />
                  <div>
                    <div className="font-medium">{s.label}</div>
                    <div className="text-muted-foreground">
                      parametry: {s.parameters.join(', ')}
                    </div>
                    {STRATEGY_HELP[s.id] && (
                      <div className="mt-1 space-y-0.5 text-xs text-muted-foreground">
                        <div>
                          <span className="font-medium text-foreground">Co robi:</span>{' '}
                          {STRATEGY_HELP[s.id].what}
                        </div>
                        <div>
                          <span className="font-medium text-foreground">Trigger:</span>{' '}
                          {STRATEGY_HELP[s.id].trigger}
                        </div>
                        <div>
                          <span className="font-medium text-foreground">Kiedy uzywac:</span>{' '}
                          {STRATEGY_HELP[s.id].whenToUse}
                        </div>
                        <div>
                          <span className="font-medium text-foreground">Ryzyko:</span>{' '}
                          {STRATEGY_HELP[s.id].risk}
                        </div>
                      </div>
                    )}
                  </div>
                </label>
              ))}
            </div>
            <div className="mt-2 rounded border p-3 text-xs">
              <div className="mb-1 font-semibold">Jak czytac parametry</div>
              <div className="grid gap-1 md:grid-cols-2">
                {PARAM_GLOSSARY.map((g) => (
                  <div key={g.key}>
                    <span className="font-medium">{g.key}:</span> {g.meaning}
                  </div>
                ))}
              </div>
            </div>
          </div>

          <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
            <div className="space-y-2">
              <div className="text-sm font-medium">Objective</div>
              <select
                className="w-full rounded border bg-background px-3 py-2 text-sm"
                value={objective}
                onChange={(e) => setObjective(e.target.value)}
              >
                {OBJECTIVES.map((o) => (
                  <option key={o.id} value={o.id}>
                    {o.label}
                  </option>
                ))}
              </select>
              <div className="text-xs text-muted-foreground">
                {selectedObjective.description}
              </div>
              <div className="text-xs text-muted-foreground">
                Przyklad uzycia: {selectedObjective.useCase}
              </div>
            </div>
            {selectedPoolsIncludeMeteora && (
              <div className="space-y-2">
                <div className="text-sm font-medium">LP share (Meteora, opcjonalnie)</div>
                <input
                  className="w-full rounded border bg-background px-3 py-2 text-sm"
                  value={lpShare}
                  onChange={(e) => setLpShare(e.target.value)}
                />
              </div>
            )}
            <div className="space-y-2">
              <div className="text-sm font-medium">Kwota symulacji (USD)</div>
              <input
                className="w-full rounded border bg-background px-3 py-2 text-sm"
                value={capitalUsd}
                onChange={(e) => setCapitalUsd(e.target.value)}
              />
            </div>
            <div className="space-y-2">
              <div className="text-sm font-medium">Cel vs HODL (USD)</div>
              <input
                className="w-full rounded border bg-background px-3 py-2 text-sm"
                value={targetVsHodlUsd}
                onChange={(e) => setTargetVsHodlUsd(e.target.value)}
                  placeholder="np. 50 (zostaw puste = bez filtra)"
              />
              <div className="text-xs text-muted-foreground">
                Pokazujemy tylko strategie spelniajace warunek: vs_hodl &gt;= target.
              </div>
            </div>
            <div className="space-y-1 pt-8">
              <label className="flex items-center gap-2 text-sm">
                <input
                  type="checkbox"
                  checked={includeIndicators}
                  onChange={(e) => setIncludeIndicators(e.target.checked)}
                />
                Dodaj strategie wskaznikowe (Bollinger + Last Candle)
              </label>
              <div className="text-xs text-muted-foreground">
                Dodaje do siatki optimize: Bollinger (6 presetow: k = 1.5/2.0/2.5 x rebalance
                co 24/48 krokow) oraz Last Candle (14 presetow roznych okien swiecy i
                czestotliwosci rebalansu). To zwieksza liczbe testowanych konfiguracji i czas
                liczenia.
              </div>
            </div>
          </div>

          <div className="rounded border p-3 text-xs space-y-3">
            <div className="text-sm font-semibold">Konfiguracja parametrow strategii (grid)</div>
            <div className="flex flex-wrap gap-2">
              {GRID_PRESETS.map((preset) => (
                <button
                  key={preset.name}
                  type="button"
                  className="rounded border px-2 py-1 text-xs hover:bg-accent"
                  onClick={() => applyPreset(preset)}
                  title={`Ustaw preset ${preset.name}`}
                >
                  {preset.name}
                </button>
              ))}
            </div>
            <div className="grid gap-3 md:grid-cols-2">
              <div>
                <div className="mb-1 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
                  Static
                </div>
                <div className="font-medium">static_deviation_pct</div>
                <div className="text-muted-foreground">
                  Opcjonalny staly zakres dla `static`: X oznacza `entry * (1±X%)` (np. 10).
                  Gdy ustawione, siatka width jest spinana do jednego wariantu. Uzywane dla wielu par.
                </div>
                <input
                  className="mt-1 w-full rounded border bg-background px-2 py-1"
                  value={staticDeviationPct}
                  onChange={(e) => setStaticDeviationPct(e.target.value)}
                  placeholder="np. 10"
                  disabled={staticManualReady}
                />
                <div className="mt-2 font-medium">
                  static_manual_lower / static_manual_upper
                  <span
                    className="ml-1 cursor-help text-muted-foreground"
                    title="Manualny zakres static działa tylko przy jednej wybranej parze. Gdy podasz poprawny lower/upper, ma priorytet nad static_deviation_pct."
                  >
                    ⓘ
                  </span>
                </div>
                <div className="text-muted-foreground">
                  Reczny zakres ceny dla `static` (dwa inputy). Jest uzyty tylko gdy wybrana jest jedna para.
                </div>
                <div className="mt-1 grid grid-cols-2 gap-2">
                  <input
                    className="w-full rounded border bg-background px-2 py-1"
                    value={staticManualLower}
                    onChange={(e) => setStaticManualLower(e.target.value)}
                    placeholder="lower"
                  />
                  <input
                    className="w-full rounded border bg-background px-2 py-1"
                    value={staticManualUpper}
                    onChange={(e) => setStaticManualUpper(e.target.value)}
                    placeholder="upper"
                  />
                </div>
                {!singlePoolSelected && (staticManualLower.trim() !== '' || staticManualUpper.trim() !== '') && (
                  <div className="mt-1 text-muted-foreground">
                    Wybrano wiele par: reczny `lower/upper` nie bedzie uzyty. Dla tego runu dziala `static_deviation_pct`.
                  </div>
                )}
              </div>
              <div>
                <div className="mb-1 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
                  Out-of-range recenter
                </div>
                <div className="font-medium">oor_recenter_deviation_pct</div>
                <div className="text-muted-foreground">
                  Osobny staly zakres dla `oor_recenter`: X oznacza `entry * (1±X%)`.
                  Uzyj przy porownaniu `static` vs `oor_recenter` na tej samej szerokosci.
                </div>
                <input
                  className="mt-1 w-full rounded border bg-background px-2 py-1"
                  value={oorRecenterDeviationPct}
                  onChange={(e) => setOorRecenterDeviationPct(e.target.value)}
                  placeholder="np. 10"
                />
              </div>
              <div className="md:col-span-2">
                <div className="text-muted-foreground">
                  Uwaga UX: ustaw tylko jedno z pol `static_deviation_pct` lub `oor_recenter_deviation_pct` w danym runie. Gdy aktywny jest poprawny manual `static lower/upper` (1 para), ma on priorytet nad `static_deviation_pct`.
                </div>
              </div>
              <div>
                <div className="mb-1 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
                  Threshold
                </div>
                <div className="font-medium">threshold_grid_pct</div>
                <div className="text-muted-foreground">Progi (%) dla strategii threshold; nizsze = czestsze triggerowanie.</div>
                <input className="mt-1 w-full rounded border bg-background px-2 py-1" value={thresholdGridPct} onChange={(e) => setThresholdGridPct(e.target.value)} />
                <div className="mt-2 font-medium">threshold_min_rebalance_interval_hours</div>
                <div className="text-muted-foreground">
                  Dodatkowa bramka czasu dla OOR, gdy immediate OOR jest wyłączony.
                </div>
                <input
                  className="mt-1 w-full rounded border bg-background px-2 py-1"
                  value={thresholdMinRebalanceIntervalHours}
                  onChange={(e) => setThresholdMinRebalanceIntervalHours(e.target.value)}
                />
                <label className="mt-2 flex items-center gap-2">
                  <input
                    type="checkbox"
                    checked={thresholdRebalanceOnRangeExitImmediately}
                    onChange={(e) =>
                      setThresholdRebalanceOnRangeExitImmediately(e.target.checked)
                    }
                  />
                  <span>
                    threshold_rebalance_on_range_exit_immediately (bot parity default: on)
                  </span>
                </label>
              </div>
              <div>
                <div className="mb-1 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
                  Retouch shift
                </div>
                <div className="font-medium">
                  retouch_offset_pct
                  <span
                    className="ml-1 cursor-help text-muted-foreground"
                    title="Offset procentowy wzgledem ceny OOR (w % ceny, nie w jednostkach absolutnych). 0 = nowy zakres dotyka ceny; +0.1 przesuwa zakres o +0.1% (w prawo), -0.1 o -0.1% (w lewo)."
                  >
                    ⓘ
                  </span>
                </div>
                <div className="text-muted-foreground">
                  Przesuniecie nowego pasma po retouch wzgledem ceny OOR (w % ceny, nie wartosc absolutna).
                  <br />
                  `0` = krawedz dotyka ceny OOR, dodatni przesuwa caly zakres w prawo, ujemny w lewo.
                </div>
                <input
                  className="mt-1 w-full rounded border bg-background px-2 py-1"
                  value={retouchOffsetPct}
                  onChange={(e) => setRetouchOffsetPct(e.target.value)}
                  placeholder="np. 0.1 lub -0.1"
                  title="Przyklad: zakres startowy 98-100, cena OOR=101, width bez zmian; offset 0 => zakres dotyka 101. Offset +0.1 => caly zakres przesuniety o +0.1% ceny."
                />
              </div>
              <div>
                <div className="font-medium">periodic_grid_hours</div>
                <div className="text-muted-foreground">
                  Interwaly periodic w godzinach (wall-clock); nizsze = czestsze rebalance.
                  Legacy periodic krokowy jest ukryty.
                </div>
                <input className="mt-1 w-full rounded border bg-background px-2 py-1" value={periodicGridSteps} onChange={(e) => setPeriodicGridSteps(e.target.value)} />
              </div>
              <div>
                <div className="font-medium">bollinger_window_grid</div>
                <div className="text-muted-foreground">Dlugosc okna SMA/odchylenia; wyzsze = gladsze pasma.</div>
                <input className="mt-1 w-full rounded border bg-background px-2 py-1" value={bollingerWindowGrid} onChange={(e) => setBollingerWindowGrid(e.target.value)} />
              </div>
              <div>
                <div className="font-medium">bollinger_k_grid</div>
                <div className="text-muted-foreground">Szerokosc pasma sigma; wyzsze = szerszy range, mniej triggerow.</div>
                <input className="mt-1 w-full rounded border bg-background px-2 py-1" value={bollingerKGrid} onChange={(e) => setBollingerKGrid(e.target.value)} />
              </div>
              <div>
                <div className="font-medium">bollinger_rebalance_steps_grid</div>
                <div className="text-muted-foreground">Czestotliwosc przebudowy Bollinger (kroki).</div>
                <input className="mt-1 w-full rounded border bg-background px-2 py-1" value={bollingerRebalanceStepsGrid} onChange={(e) => setBollingerRebalanceStepsGrid(e.target.value)} />
              </div>
              <div>
                <div className="font-medium">last_candle_seconds_grid</div>
                <div className="text-muted-foreground">Rozmiary swiec dla Last Candle (sekundy, snapshot mode).</div>
                <input className="mt-1 w-full rounded border bg-background px-2 py-1" value={lastCandleSecondsGrid} onChange={(e) => setLastCandleSecondsGrid(e.target.value)} />
              </div>
              <div>
                <div className="font-medium">last_candle_rebalance_seconds_grid</div>
                <div className="text-muted-foreground">Interwaly rebalansu Last Candle (sekundy, snapshot mode).</div>
                <input className="mt-1 w-full rounded border bg-background px-2 py-1" value={lastCandleRebalanceSecondsGrid} onChange={(e) => setLastCandleRebalanceSecondsGrid(e.target.value)} />
              </div>
            </div>
          </div>

          <Button
            disabled={
              fullRunMut.isPending || selectedWindows.length === 0 || selectedPools.length === 0
            }
            onClick={() => {
              const runCapitalUsd = parseOptionalNumber(capitalUsd) ?? 8000
              setLastRunCapitalUsd(runCapitalUsd)
              fullRunMut.mutate({
                windows_hours: selectedWindows,
                include_strategy_ids:
                  selectedStrategies.length > 0 ? selectedStrategies : undefined,
                include_indicator_strategies: includeIndicators,
                objective,
                pool_ids: selectedPools,
                lp_share: selectedPoolsIncludeMeteora
                  ? parseOptionalNumber(lpShare)
                  : undefined,
                capital_usd: parseOptionalNumber(capitalUsd),
                target_vs_hodl_usd: parseOptionalNumber(targetVsHodlUsd),
                static_deviation_pct: staticManualReady
                  ? undefined
                  : parseOptionalNumber(staticDeviationPct),
                static_manual_lower: staticManualReady ? staticManualLowerNum : undefined,
                static_manual_upper: staticManualReady ? staticManualUpperNum : undefined,
                oor_recenter_deviation_pct: parseOptionalNumber(oorRecenterDeviationPct),
                threshold_grid_pct: parseCsvFloats(thresholdGridPct),
                threshold_min_rebalance_interval_hours: parseOptionalNumber(
                  thresholdMinRebalanceIntervalHours,
                ),
                threshold_rebalance_on_range_exit_immediately:
                  thresholdRebalanceOnRangeExitImmediately,
                periodic_grid_steps: parseCsvUInt64s(periodicGridSteps),
                retouch_offset_pct: parseOptionalNumber(retouchOffsetPct),
                bollinger_window_grid: parseCsvUInt64s(bollingerWindowGrid),
                bollinger_k_grid: parseCsvFloats(bollingerKGrid),
                bollinger_rebalance_steps_grid: parseCsvUInt64s(bollingerRebalanceStepsGrid),
                last_candle_seconds_grid: parseCsvUInt64s(lastCandleSecondsGrid),
                last_candle_rebalance_seconds_grid: parseCsvUInt64s(
                  lastCandleRebalanceSecondsGrid,
                ),
              })
            }}
          >
            {fullRunMut.isPending ? 'Uruchamiam...' : 'Uruchom FULL porownanie'}
          </Button>

          <div className="rounded border p-3 space-y-2">
            <div className="text-sm font-semibold">Auto-Tune (background)</div>
            <div className="text-xs text-muted-foreground">
              Cyklicznie odpala FULL optimize i aktualizuje najlepszego winnera. Mozesz potem
              zastosowac winnera w sekcji Strategies.
            </div>
            <div className="flex flex-wrap items-end gap-2">
              <div className="space-y-1">
                <div className="text-xs font-medium">Interval (min)</div>
                <input
                  className="w-28 rounded border bg-background px-2 py-1 text-sm"
                  value={autoTuneIntervalMinutes}
                  onChange={(e) => setAutoTuneIntervalMinutes(e.target.value)}
                />
              </div>
              <Button
                variant="secondary"
                disabled={startAutoTuneMut.isPending}
                onClick={() =>
                  startAutoTuneMut.mutate({
                    interval_minutes: parseOptionalNumber(autoTuneIntervalMinutes),
                    full_request: {
                      windows_hours: selectedWindows,
                      include_strategy_ids:
                        selectedStrategies.length > 0 ? selectedStrategies : undefined,
                      include_indicator_strategies: includeIndicators,
                      objective,
                      pool_ids: selectedPools,
                      lp_share: selectedPoolsIncludeMeteora
                        ? parseOptionalNumber(lpShare)
                        : undefined,
                      capital_usd: parseOptionalNumber(capitalUsd),
                      target_vs_hodl_usd: parseOptionalNumber(targetVsHodlUsd),
                      static_deviation_pct: staticManualReady
                        ? undefined
                        : parseOptionalNumber(staticDeviationPct),
                      static_manual_lower: staticManualReady ? staticManualLowerNum : undefined,
                      static_manual_upper: staticManualReady ? staticManualUpperNum : undefined,
                      oor_recenter_deviation_pct: parseOptionalNumber(oorRecenterDeviationPct),
                      threshold_grid_pct: parseCsvFloats(thresholdGridPct),
                      threshold_min_rebalance_interval_hours: parseOptionalNumber(
                        thresholdMinRebalanceIntervalHours,
                      ),
                      threshold_rebalance_on_range_exit_immediately:
                        thresholdRebalanceOnRangeExitImmediately,
                      periodic_grid_steps: parseCsvUInt64s(periodicGridSteps),
                      retouch_offset_pct: parseOptionalNumber(retouchOffsetPct),
                      bollinger_window_grid: parseCsvUInt64s(bollingerWindowGrid),
                      bollinger_k_grid: parseCsvFloats(bollingerKGrid),
                      bollinger_rebalance_steps_grid: parseCsvUInt64s(
                        bollingerRebalanceStepsGrid,
                      ),
                      last_candle_seconds_grid: parseCsvUInt64s(lastCandleSecondsGrid),
                      last_candle_rebalance_seconds_grid: parseCsvUInt64s(
                        lastCandleRebalanceSecondsGrid,
                      ),
                    },
                  })
                }
              >
                Start Auto-Tune
              </Button>
              <Button
                variant="outline"
                disabled={stopAutoTuneMut.isPending}
                onClick={() => stopAutoTuneMut.mutate()}
              >
                Stop Auto-Tune
              </Button>
            </div>
            <div className="text-xs text-muted-foreground">
              Status: {autoTuneStatusQ.data?.running ? 'running' : 'stopped'} | note:{' '}
              {autoTuneStatusQ.data?.note ?? '—'}
            </div>
            {autoTuneStatusQ.data?.latest_winner && (
              <div className="text-xs">
                Latest winner: <span className="font-mono">{autoTuneStatusQ.data.latest_winner.strategy}</span>{' '}
                ({fmt(autoTuneStatusQ.data.latest_winner.score, 3)} score,{' '}
                {autoTuneStatusQ.data.latest_winner.pool_label},{' '}
                {autoTuneStatusQ.data.latest_winner.window_hours}h; PnL{' '}
                {fmt(autoTuneStatusQ.data.latest_winner.pnl)} USD (
                {fmt(
                  pnlPct(
                    autoTuneStatusQ.data.latest_winner.pnl,
                    parseOptionalNumber(capitalUsd) ?? lastRunCapitalUsd,
                  ),
                )}
                %); end {fmt((parseOptionalNumber(capitalUsd) ?? lastRunCapitalUsd ?? 0) + autoTuneStatusQ.data.latest_winner.pnl)}{' '}
                USD)
              </div>
            )}
          </div>

          {jobId && (
            <div className="text-sm text-muted-foreground">
              Job: <span className="font-mono">{jobId}</span> | status:{' '}
              <span className="font-medium">{jobQ.data?.status ?? 'running'}</span>
            </div>
          )}
          {jobQ.data?.stderr && (
            <pre className="max-h-36 overflow-auto rounded bg-muted p-3 text-xs">
              {jobQ.data.stderr}
            </pre>
          )}
        </CardContent>
      </Card>

      {qualifyingTop3.length > 0 && (
        <Card>
          <CardHeader>
            <CardTitle>Strategie spelniajace target</CardTitle>
            <CardDescription>
              Szybki przeglad: TOP 3 warianty z kazdej rodziny strategii dla kazdej pary i okna. Wynik finansowy
              (PnL, vs HODL) w kolorze: zieleń = na plusie, czerwień = na minusie; kwoty w USD z separatorem
              tysięcy.
            </CardDescription>
          </CardHeader>
          <CardContent>
            <div className="mb-4 flex flex-wrap items-center gap-2 text-sm">
              <span className="font-medium">Sortuj TOP wg:</span>
              <select
                className="rounded border bg-background px-2 py-1"
                value={topSortBy}
                onChange={(e) =>
                  setTopSortBy(e.target.value as 'vs_hodl' | 'score' | 'pnl' | 'fees')
                }
              >
                <option value="vs_hodl">vs HODL</option>
                <option value="score">Score</option>
                <option value="pnl">PnL</option>
                <option value="fees">Fees</option>
              </select>
              <span className="text-muted-foreground">
                Kapitał startowy (do end / PnL%): {fmt(lastRunCapitalUsd)} USD
              </span>
            </div>
            <div className="grid gap-4 md:grid-cols-2">
              {qualifyingTop3.map((g) => (
                <div key={g.key} className="rounded border p-3">
                  <div className="mb-2 text-sm font-medium">
                    {g.poolLabel} - {g.windowHours}h
                  </div>
                  {g.families.length === 0 ? (
                    <div className="text-sm text-muted-foreground">
                      Brak strategii spelniajacych warunek.
                    </div>
                  ) : (
                    <div className="space-y-3">
                      {g.families.map((fam) => (
                        <div key={`${g.key}-${fam.family}`}>
                          <div className="mb-1 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                            {fam.family}
                          </div>
                          <ul className="list-none space-y-2.5 p-0">
                            {fam.items.map((m, idx) => {
                              const endUsd = (lastRunCapitalUsd ?? 0) + m.pnl
                              const pct = pnlPct(m.pnl, lastRunCapitalUsd)
                              const badge = periodicModeBadge(m.strategy)
                              const statShell = (active: boolean) =>
                                `rounded-lg px-2.5 py-2 ${
                                  active
                                    ? 'bg-primary/10 ring-2 ring-primary/45'
                                    : 'bg-background/90 ring-1 ring-border/70'
                                }`
                              return (
                                <li
                                  key={`${g.key}-${fam.family}-${m.strategy}-${idx}`}
                                  className="rounded-xl border border-border/80 bg-muted/25 p-3 shadow-sm"
                                >
                                  <div className="flex gap-2.5">
                                    <div
                                      className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-primary/15 text-xs font-bold text-primary"
                                      aria-hidden
                                    >
                                      {idx + 1}
                                    </div>
                                    <div className="min-w-0 flex-1 space-y-2">
                                      <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
                                        <div
                                          className="font-mono text-[13px] font-medium leading-snug text-foreground break-words"
                                          title={strategyTooltip(m.strategy)}
                                        >
                                          {m.strategy}
                                        </div>
                                        {badge && (
                                          <span
                                            className="shrink-0 rounded border border-border bg-background px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground"
                                            title={badge.title}
                                          >
                                            {badge.label}
                                          </span>
                                        )}
                                      </div>
                                      <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
                                        <div
                                          className={statShell(false)}
                                          title="Wartość portfela LP po oknie (kapitał startowy + PnL)."
                                        >
                                          <div className="text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
                                            Koniec
                                          </div>
                                          <div className="mt-0.5 text-sm font-semibold tabular-nums tracking-tight text-foreground">
                                            {fmtMoneyUsd(endUsd)}
                                          </div>
                                        </div>
                                        <div
                                          className={statShell(topSortBy === 'pnl')}
                                          title="Zysk / strata vs kapitał startowy (USD)."
                                        >
                                          <div className="text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
                                            PnL
                                          </div>
                                          <div
                                            className={`mt-0.5 text-sm font-semibold tabular-nums tracking-tight ${pnlToneClass(m.pnl)}`}
                                          >
                                            {fmtMoneyUsd(m.pnl)}
                                          </div>
                                        </div>
                                        <div
                                          className={statShell(false)}
                                          title="PnL w procentach kapitału startowego."
                                        >
                                          <div className="text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
                                            PnL %
                                          </div>
                                          <div
                                            className={`mt-0.5 text-sm font-semibold tabular-nums tracking-tight ${pnlToneClass(m.pnl)}`}
                                          >
                                            {fmtSignedPct(pct)}
                                          </div>
                                        </div>
                                        <div
                                          className={statShell(topSortBy === 'vs_hodl')}
                                          title="Przewaga nad benchmarkiem HODL (USD)."
                                        >
                                          <div className="text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
                                            vs HODL
                                          </div>
                                          <div
                                            className={`mt-0.5 text-sm font-semibold tabular-nums tracking-tight ${pnlToneClass(m.vs_hodl)}`}
                                          >
                                            {fmtMoneyUsd(m.vs_hodl)}
                                          </div>
                                        </div>
                                      </div>
                                      <div className="flex flex-wrap gap-2 border-t border-border/50 pt-2 text-xs">
                                        <div
                                          className={
                                            topSortBy === 'score'
                                              ? 'rounded-md bg-primary/10 px-2 py-1 ring-1 ring-primary/40'
                                              : 'text-muted-foreground'
                                          }
                                        >
                                          <span className="text-muted-foreground">Score </span>
                                          <span className="font-medium tabular-nums text-foreground">
                                            {fmt(m.score)}
                                          </span>
                                        </div>
                                        <div
                                          className={
                                            topSortBy === 'fees'
                                              ? 'rounded-md bg-primary/10 px-2 py-1 ring-1 ring-primary/40'
                                              : 'text-muted-foreground'
                                          }
                                        >
                                          <span className="text-muted-foreground">Fees </span>
                                          <span className="font-medium tabular-nums text-foreground">
                                            {fmtMoneyUsd(m.fees)}
                                          </span>
                                        </div>
                                      </div>
                                      <div className="text-xs">
                                        <div className="font-semibold uppercase tracking-wide text-muted-foreground">
                                          Zakres ceny (USD)
                                        </div>
                                        <div
                                          className="mt-0.5 font-mono text-[12px] tabular-nums text-foreground/90"
                                          title="Zakres ceny (USD), na ktorym liczono ten wariant strategii."
                                        >
                                          {rangeLabel(m.lower_usd, m.upper_usd)}
                                        </div>
                                      </div>
                                    </div>
                                  </div>
                                </li>
                              )
                            })}
                          </ul>
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              ))}
            </div>
          </CardContent>
        </Card>
      )}

      {liquidityRegime.length > 0 && (
        <Card>
          <CardHeader>
            <CardTitle>Rezim plynnosci (orientacyjnie)</CardTitle>
            <CardDescription>
              Kontekst wolumenu z biezacego API Orca dla pooli użytych w rankingu.
            </CardDescription>
          </CardHeader>
          <CardContent>
            <div className="overflow-auto">
              <table className="w-full text-sm">
                <thead>
                  <tr className="border-b text-left">
                    <th className="p-2">Pair / okno</th>
                    <th className="p-2" title="Przyblizony wolumen dla okna (v24h * okno/24).">
                      Approx volume (okno)
                    </th>
                    <th
                      className="p-2"
                      title="Intensity = volume_1h / (volume_24h / 24). Wartosc > 1 oznacza, ze ostatnia godzina byla bardziej aktywna niz srednia godzina z 24h."
                    >
                      1h/24h intensity
                    </th>
                    <th className="p-2">Rezim</th>
                  </tr>
                </thead>
                <tbody>
                  {liquidityRegime.map((x) => (
                    <tr key={x.key} className="border-b">
                      <td className="p-2">
                        {x.poolLabel} - {x.windowHours}h
                      </td>
                      <td className="p-2">{fmt(x.approxWindowVol)}</td>
                      <td className="p-2">{fmt(x.ratio1h24h, 3)}</td>
                      <td className="p-2">{x.label}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </CardContent>
        </Card>
      )}

      {windowStats.length > 0 && (
        <Card>
          <CardHeader>
            <CardTitle>Target per okno</CardTitle>
            <CardDescription>
              Szybki podglad: ile strategii przechodzi target i jaka jest mediana vs HODL.
            </CardDescription>
          </CardHeader>
          <CardContent>
            <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
              {windowStats.map((s) => {
                const pct = s.totalCount > 0 ? (s.qualifiedCount / s.totalCount) * 100 : 0
                return (
                  <div key={s.windowHours} className="rounded border p-3">
                    <div className="text-sm font-medium">{s.windowHours}h</div>
                    <div className="mt-1 text-sm text-muted-foreground">
                      target pass: {s.qualifiedCount}/{s.totalCount}
                    </div>
                    <div className="mt-1 text-sm">
                      mediana vs HODL: {fmt(s.medianVsHodl)} USD
                    </div>
                    <div className="mt-2 h-2 w-full overflow-hidden rounded bg-muted">
                      <div
                        className="h-full bg-primary"
                        style={{ width: `${Math.max(0, Math.min(100, pct))}%` }}
                      />
                    </div>
                    <div className="mt-1 text-xs text-muted-foreground">{fmt(pct, 1)}%</div>
                  </div>
                )
              })}
            </div>
          </CardContent>
        </Card>
      )}

      {globalTop.length > 0 && (
        <Card>
          <CardHeader>
            <CardTitle>Globalny ranking TOP (caly run)</CardTitle>
            <CardDescription>
              Agregacja przez wszystkie pary i okna czasowe.
            </CardDescription>
          </CardHeader>
          <CardContent>
            <div className="mb-3 space-y-1 text-xs text-muted-foreground">
              <div>
                Wystapienia = liczba wariantow (strategia + inny range) policzonych w calym runie.
              </div>
              <div>
                Kapitał startowy (do średniego end / Avg PnL%): {fmt(lastRunCapitalUsd)} USD
              </div>
            </div>
            <div className="overflow-auto">
              <table className="w-full text-sm">
                <thead>
                  <tr className="border-b text-left">
                    <th className="p-2">#</th>
                    <th className="p-2">Strategy</th>
                    <th
                      className="p-2"
                      title="Ile wariantow tej strategii pojawilo sie lacznie (pool x okno x range)."
                    >
                      Wystapienia
                    </th>
                    <th className="p-2" title="Ile razy wariant tej strategii mial rank=1 w swoim oknie.">
                      Wygrane (rank=1)
                    </th>
                    <th className="p-2" title="Srednia przewaga LP nad HODL (USD).">Avg vs HODL</th>
                    <th className="p-2" title="Sredni score wg wybranego objective.">Avg Score</th>
                    <th className="p-2" title="Sredni PnL LP (USD).">Avg PnL</th>
                    <th
                      className="p-2"
                      title="Sredni kapital koncowy: start + sredni PnL (ten sam start dla calego runu)."
                    >
                      Avg capital end
                    </th>
                    <th className="p-2" title="Sredni PnL % wzgledem kapitalu startowego.">
                      Avg PnL %
                    </th>
                    <th className="p-2" title="Srednie fee LP (USD).">Avg Fees</th>
                    <th className="p-2" title="Najlepszy pojedynczy wynik vs HODL (USD).">Best vs HODL</th>
                  </tr>
                </thead>
                <tbody>
                  {globalTop.map((g, idx) => (
                    <tr key={`${g.strategy}-${idx}`} className="border-b">
                      <td className="p-2">{idx + 1}</td>
                      <td className="p-2 font-mono">
                        <span title={strategyTooltip(g.strategy)}>{g.strategy}</span>
                      </td>
                      <td className="p-2">{g.appearances}</td>
                      <td className="p-2">{g.wins}</td>
                      <td className={`p-2 tabular-nums ${pnlToneClass(g.avgVsHodl)}`}>
                        {fmtMoneyUsd(g.avgVsHodl)}
                      </td>
                      <td className="p-2 tabular-nums">{fmt(g.avgScore)}</td>
                      <td className={`p-2 tabular-nums font-semibold ${pnlToneClass(g.avgPnl)}`}>
                        {fmtMoneyUsd(g.avgPnl)}
                      </td>
                      <td className="p-2 whitespace-nowrap tabular-nums font-medium">
                        {fmtMoneyUsd((lastRunCapitalUsd ?? 0) + g.avgPnl)}
                      </td>
                      <td className={`p-2 tabular-nums font-semibold ${pnlToneClass(g.avgPnl)}`}>
                        {fmtSignedPct(pnlPct(g.avgPnl, lastRunCapitalUsd))}
                      </td>
                      <td className="p-2 whitespace-nowrap tabular-nums">{fmtMoneyUsd(g.avgFees)}</td>
                      <td className={`p-2 tabular-nums ${pnlToneClass(g.bestVsHodl)}`}>
                        {fmtMoneyUsd(g.bestVsHodl)}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </CardContent>
        </Card>
      )}

      {sortedResults.map((r) => (
        <Card key={`${r.pool_id}-${r.window_hours}`}>
          <CardHeader>
            <CardTitle>
              {r.pool_label} - {r.window_hours}h
            </CardTitle>
            <CardDescription>{r.protocol.toUpperCase()}</CardDescription>
          </CardHeader>
          <CardContent>
            <div className="mb-2 text-xs text-muted-foreground">
              Kapital startowy dla tej tabeli: {fmt(lastRunCapitalUsd)} USD
            </div>
            <div className="overflow-auto">
              <table className="w-full text-sm">
                <thead>
                  <tr className="border-b text-left">
                    <th className="p-2">Rank</th>
                    <th className="p-2">Strategy</th>
                    <th className="p-2" title="Zakres ceny (USD), dla ktorego liczono ten wariant.">
                      Range (USD)
                    </th>
                    <th className="p-2" title="Szerokosc zakresu ceny tego wariantu.">Width%</th>
                    <th className="p-2" title="Wynik objective dla wariantu.">Score</th>
                    <th className="p-2" title="Kapital po oknie czasowym: capital_start + PnL.">
                      Capital end (USD)
                    </th>
                    <th className="p-2" title="PnL LP (USD).">PnL</th>
                    <th className="p-2" title="PnL procentowo wzgledem kapitalu startowego.">
                      PnL %
                    </th>
                    <th className="p-2" title="Przewaga LP nad HODL (USD).">vs HODL</th>
                    <th className="p-2" title="Fee LP (USD).">Fees</th>
                    <th className="p-2" title="Time in range (% czasu).">TIR%</th>
                    <th
                      className="p-2"
                      title="IL-like (backtest accounting, ex-fees): under/over-performance LP vs HODL wedlug ksiegowosci backtestu. To nie jest czysty wzor klasycznego IL v2/v3."
                    >
                      IL-like% (acct)
                    </th>
                    <th className="p-2" title="Liczba rebalansow w tym wariancie.">Rebalances</th>
                  </tr>
                </thead>
                <tbody>
                  {r.metrics.slice(0, 20).map((m) => (
                    <tr key={`${r.pool_id}-${r.window_hours}-${m.rank}-${m.strategy}`} className="border-b">
                      <td className="p-2">{m.rank}</td>
                      <td className="p-2 font-mono">
                        <span title={strategyTooltip(m.strategy)}>{m.strategy}</span>
                        {(() => {
                          const badge = periodicModeBadge(m.strategy)
                          if (!badge) return null
                          return (
                            <span
                              className="ml-2 rounded border px-1 py-0.5 text-[10px] uppercase tracking-wide text-muted-foreground"
                              title={badge.title}
                            >
                              {badge.label}
                            </span>
                          )
                        })()}
                      </td>
                      <td className="p-2 font-mono text-xs tabular-nums text-muted-foreground">
                        {rangeLabel(m.lower_usd, m.upper_usd)}
                      </td>
                      <td className="p-2 tabular-nums">{fmt(m.width_pct)}</td>
                      <td className="p-2 tabular-nums">{fmt(m.score)}</td>
                      <td className="p-2 whitespace-nowrap tabular-nums font-medium">
                        {fmtMoneyUsd((lastRunCapitalUsd ?? 0) + m.pnl)}
                      </td>
                      <td
                        className={`p-2 whitespace-nowrap tabular-nums font-semibold ${pnlToneClass(m.pnl)}`}
                      >
                        {fmtMoneyUsd(m.pnl)}
                      </td>
                      <td
                        className={`p-2 whitespace-nowrap tabular-nums font-semibold ${pnlToneClass(m.pnl)}`}
                      >
                        {fmtSignedPct(pnlPct(m.pnl, lastRunCapitalUsd))}
                      </td>
                      <td
                        className={`p-2 whitespace-nowrap tabular-nums font-medium ${pnlToneClass(m.vs_hodl)}`}
                      >
                        {fmtMoneyUsd(m.vs_hodl)}
                      </td>
                      <td className="p-2 whitespace-nowrap tabular-nums">{fmtMoneyUsd(m.fees)}</td>
                      <td className="p-2">{fmt(m.tir_pct)}</td>
                      <td className="p-2">{fmt(m.il_like_pct)}</td>
                      <td className="p-2">{m.rebalances}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </CardContent>
        </Card>
      ))}
    </div>
  )
}

