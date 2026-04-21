import { useMemo, useState } from 'react'
import { useMutation, useQuery } from '@tanstack/react-query'
import {
  getBacktestFullJob,
  getPools,
  getBacktestStrategyCatalog,
  startBacktestFull,
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

function parseOptionalNumber(raw: string): number | undefined {
  if (raw.trim() === '') return undefined
  const v = Number(raw)
  if (!Number.isFinite(v)) return undefined
  return v
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
  return undefined
}

type TopSortBy = 'vs_hodl' | 'score' | 'pnl' | 'fees'

function rangeLabel(lowerUsd: number, upperUsd: number): string {
  return `$${fmt(lowerUsd)} - $${fmt(upperUsd)}`
}

export default function Backtests() {
  const [selectedWindows, setSelectedWindows] = useState<number[]>([...WINDOWS])
  const [selectedPools, setSelectedPools] = useState<string[]>(POOLS.map((p) => p.id))
  const [selectedStrategies, setSelectedStrategies] = useState<string[]>([])
  const [includeIndicators, setIncludeIndicators] = useState(true)
  const [objective, setObjective] = useState('vs-hodl')
  const [lpShare, setLpShare] = useState('')
  const [capitalUsd, setCapitalUsd] = useState('7000')
  const [targetVsHodlUsd, setTargetVsHodlUsd] = useState('')
  const [topSortBy, setTopSortBy] = useState<TopSortBy>('vs_hodl')
  const [jobId, setJobId] = useState<string | null>(null)
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
      const ratio1h24h = v24 > 0 ? (v1 * 24) / v24 : 0
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
        items: [...r.metrics]
          .sort((a, b) => {
            if (topSortBy === 'score') return b.score - a.score
            if (topSortBy === 'pnl') return b.pnl - a.pnl
            if (topSortBy === 'fees') return b.fees - a.fees
            return b.vs_hodl - a.vs_hodl
          })
          .reduce<typeof r.metrics>((acc, row) => {
            if (acc.some((x) => x.strategy === row.strategy)) return acc
            acc.push(row)
            return acc
          }, [])
          .slice(0, 3),
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

  const toggleSelection = <T extends string | number>(
    list: T[],
    value: T,
    setter: (v: T[]) => void,
  ) => {
    if (list.includes(value)) setter(list.filter((x) => x !== value))
    else setter([...list, value])
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
                  </div>
                </label>
              ))}
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

          <Button
            disabled={
              fullRunMut.isPending || selectedWindows.length === 0 || selectedPools.length === 0
            }
            onClick={() =>
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
              })
            }
          >
            {fullRunMut.isPending ? 'Uruchamiam...' : 'Uruchom FULL porownanie'}
          </Button>

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
              Szybki przeglad: TOP 3 strategie (wg vs HODL) dla kazdej pary i okna.
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
            </div>
            <div className="grid gap-4 md:grid-cols-2">
              {qualifyingTop3.map((g) => (
                <div key={g.key} className="rounded border p-3">
                  <div className="mb-2 text-sm font-medium">
                    {g.poolLabel} - {g.windowHours}h
                  </div>
                  {g.items.length === 0 ? (
                    <div className="text-sm text-muted-foreground">
                      Brak strategii spelniajacych warunek.
                    </div>
                  ) : (
                    <ol className="space-y-1 text-sm">
                      {g.items.map((m, idx) => (
                        <li key={`${g.key}-${m.strategy}-${idx}`} className="flex items-center justify-between gap-2">
                          <span className="font-mono">
                            <span title={strategyTooltip(m.strategy)}>
                              {idx + 1}. {m.strategy}
                            </span>
                          </span>
                          <span className="text-muted-foreground">
                            {topSortBy === 'score' && <>Score: {fmt(m.score)}</>}
                            {topSortBy === 'pnl' && <>PnL: {fmt(m.pnl)} USD</>}
                            {topSortBy === 'fees' && <>Fees: {fmt(m.fees)} USD</>}
                            {topSortBy === 'vs_hodl' && <>vs HODL: {fmt(m.vs_hodl)} USD</>}
                            {' | '}
                            <span title="Zakres ceny (USD), na ktorym liczono ten wariant strategii.">
                              range: {rangeLabel(m.lower_usd, m.upper_usd)}
                            </span>
                          </span>
                        </li>
                      ))}
                    </ol>
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
                    <th className="p-2" title="(volume_1h*24)/volume_24h; >1 = ostatnia godzina bardziej aktywna.">
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
                      <td className="p-2">{fmt(x.ratio1h24h)}</td>
                      <td className="p-2">{x.label}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
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
            <div className="mb-3 text-xs text-muted-foreground">
              Wystapienia = liczba wariantow (strategia + inny range) policzonych w calym runie.
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
                      <td className="p-2">{fmt(g.avgVsHodl)}</td>
                      <td className="p-2">{fmt(g.avgScore)}</td>
                      <td className="p-2">{fmt(g.avgPnl)}</td>
                      <td className="p-2">{fmt(g.avgFees)}</td>
                      <td className="p-2">{fmt(g.bestVsHodl)}</td>
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
                    <th className="p-2" title="PnL LP (USD).">PnL</th>
                    <th className="p-2" title="Przewaga LP nad HODL (USD).">vs HODL</th>
                    <th className="p-2" title="Fee LP (USD).">Fees</th>
                    <th className="p-2" title="Time in range (% czasu).">TIR%</th>
                    <th className="p-2" title="IL-like (bez fee) jako % kapitalu.">IL-like%</th>
                    <th className="p-2" title="Liczba rebalansow w tym wariancie.">Rebalances</th>
                  </tr>
                </thead>
                <tbody>
                  {r.metrics.slice(0, 20).map((m) => (
                    <tr key={`${r.pool_id}-${r.window_hours}-${m.rank}-${m.strategy}`} className="border-b">
                      <td className="p-2">{m.rank}</td>
                      <td className="p-2 font-mono">
                        <span title={strategyTooltip(m.strategy)}>{m.strategy}</span>
                      </td>
                      <td className="p-2">{rangeLabel(m.lower_usd, m.upper_usd)}</td>
                      <td className="p-2">{fmt(m.width_pct)}</td>
                      <td className="p-2">{fmt(m.score)}</td>
                      <td className="p-2">{fmt(m.pnl)}</td>
                      <td className="p-2">{fmt(m.vs_hodl)}</td>
                      <td className="p-2">{fmt(m.fees)}</td>
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

