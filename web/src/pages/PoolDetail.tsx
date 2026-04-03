import { useQuery } from '@tanstack/react-query'
import { useParams, Link } from 'react-router-dom'
import { ArrowLeft } from 'lucide-react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { getPool, getPoolState, getOrcaToken } from '@/lib/api'
import { formatUSD, formatPercent, shortenAddress } from '@/lib/utils'

export default function PoolDetail() {
  const { address } = useParams<{ address: string }>()

  const {
    data: pool,
    isLoading: poolLoading,
    isError: poolError,
    error: poolErr,
  } = useQuery({
    queryKey: ['pool', address],
    queryFn: () => getPool(address!),
    enabled: !!address,
  })

  const mintA = pool?.token_mint_a
  const mintB = pool?.token_mint_b

  const { data: orcaA } = useQuery({
    queryKey: ['orca-token', mintA],
    queryFn: () => getOrcaToken(mintA!),
    enabled: !!mintA,
    staleTime: 60 * 60 * 1000,
  })

  const { data: orcaB } = useQuery({
    queryKey: ['orca-token', mintB],
    queryFn: () => getOrcaToken(mintB!),
    enabled: !!mintB,
    staleTime: 60 * 60 * 1000,
  })

  const { data: state, isError: stateError } = useQuery({
    queryKey: ['pool-state', address],
    queryFn: () => getPoolState(address!),
    enabled: !!address,
    refetchInterval: 10000,
  })

  if (!address) {
    return (
      <div className="text-center py-8 text-foreground">
        Brak adresu puli w URL.
      </div>
    )
  }

  if (poolLoading) {
    return (
      <div className="text-center py-8 text-muted-foreground">
        Ładowanie puli…
      </div>
    )
  }

  if (poolError) {
    return (
      <div className="space-y-4 text-foreground max-w-xl">
        <Link to="/pools">
          <Button variant="ghost" size="sm" className="gap-2">
            <ArrowLeft className="h-4 w-4" />
            Wróć do listy
          </Button>
        </Link>
        <div className="rounded-md border border-destructive/40 bg-destructive/10 px-4 py-3 text-sm">
          <p className="font-medium text-destructive">Nie udało się wczytać puli</p>
          <p className="text-muted-foreground text-xs mt-1 font-mono break-all">
            {(poolErr as Error)?.message ?? 'Unknown error'}
          </p>
          <p className="text-muted-foreground text-xs mt-2">
            Sprawdź, czy API działa i czy ten adres istnieje w backendzie (Orca REST / proxy).
          </p>
        </div>
      </div>
    )
  }

  if (!pool) {
    return (
      <div className="text-center py-8 text-foreground">
        <p className="font-medium">Nie znaleziono puli</p>
        <p className="text-sm text-muted-foreground mt-1 font-mono break-all">{address}</p>
        <Link to="/pools" className="inline-block mt-4">
          <Button variant="outline" size="sm">
            Wróć do Pools
          </Button>
        </Link>
      </div>
    )
  }

  const symA = orcaA?.symbol ?? shortenAddress(pool.token_mint_a, 4)
  const symB = orcaB?.symbol ?? shortenAddress(pool.token_mint_b, 4)
  const feePct = (pool.fee_rate_bps ?? 0) / 100
  const poolAddr = pool.address ?? address
  const tvl = pool.tvl_usd != null ? String(pool.tvl_usd) : '0'
  const vol = pool.volume_24h_usd != null ? String(pool.volume_24h_usd) : '0'
  const apy = pool.apy_estimate != null ? String(pool.apy_estimate) : '0'

  let stateTimeLabel = '—'
  if (state?.timestamp) {
    try {
      stateTimeLabel = new Date(state.timestamp).toLocaleString()
    } catch {
      stateTimeLabel = state.timestamp
    }
  }

  return (
    <div className="space-y-6 text-foreground">
      <div className="flex flex-wrap items-center gap-4">
        <Link to="/pools">
          <Button variant="ghost" size="icon" aria-label="Wróć">
            <ArrowLeft className="h-4 w-4" />
          </Button>
        </Link>
        <h1 className="text-3xl font-bold">
          {symA}/{symB}
        </h1>
        <span className="text-muted-foreground">{feePct.toFixed(2)}% fee</span>
      </div>

      <div className="grid gap-6 md:grid-cols-2 lg:grid-cols-3">
        <Card>
          <CardHeader>
            <CardTitle>Pool Info</CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="flex justify-between gap-2">
              <span className="text-muted-foreground">Address</span>
              <span className="font-mono text-sm text-right break-all">{shortenAddress(poolAddr, 8)}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-muted-foreground">Protocol</span>
              <span className="capitalize">{pool.protocol ?? '—'}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-muted-foreground">Tick Spacing</span>
              <span>{pool.tick_spacing ?? '—'}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-muted-foreground">Current tick</span>
              <span>{pool.current_tick}</span>
            </div>
            <div className="flex justify-between gap-2">
              <span className="text-muted-foreground">Price</span>
              <span className="font-mono text-sm text-right break-all">{pool.price}</span>
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Metrics</CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="flex justify-between">
              <span className="text-muted-foreground">TVL</span>
              <span className="font-bold">{formatUSD(tvl)}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-muted-foreground">24h Volume</span>
              <span>{formatUSD(vol)}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-muted-foreground">APY (est.)</span>
              <span className="text-green-500">{formatPercent(apy)}</span>
            </div>
          </CardContent>
        </Card>

        {state && !stateError && (
          <Card>
            <CardHeader>
              <CardTitle>Current State</CardTitle>
            </CardHeader>
            <CardContent className="space-y-4">
              <div className="flex justify-between">
                <span className="text-muted-foreground">Current Tick</span>
                <span>{state.current_tick}</span>
              </div>
              <div className="flex justify-between gap-2">
                <span className="text-muted-foreground">Sqrt price (X64)</span>
                <span className="font-mono text-xs break-all text-right">{state.sqrt_price}</span>
              </div>
              <div className="flex justify-between gap-2">
                <span className="text-muted-foreground">Price</span>
                <span className="font-mono text-sm break-all text-right">{state.price}</span>
              </div>
              <div className="flex justify-between">
                <span className="text-muted-foreground">Liquidity</span>
                <span className="font-mono text-sm break-all">{state.liquidity}</span>
              </div>
              <div className="flex justify-between gap-2">
                <span className="text-muted-foreground">Fee growth A</span>
                <span className="font-mono text-xs break-all text-right">{state.fee_growth_global_a}</span>
              </div>
              <div className="flex justify-between gap-2">
                <span className="text-muted-foreground">Fee growth B</span>
                <span className="font-mono text-xs break-all text-right">{state.fee_growth_global_b}</span>
              </div>
              <div className="flex justify-between gap-2">
                <span className="text-muted-foreground">Updated</span>
                <span className="text-sm text-right">{stateTimeLabel}</span>
              </div>
            </CardContent>
          </Card>
        )}
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Tokens</CardTitle>
        </CardHeader>
        <CardContent>
          <div className="grid gap-4 md:grid-cols-2">
            <div className="p-4 rounded-lg border">
              <div className="font-medium">{symA}</div>
              <div className="text-sm text-muted-foreground font-mono mt-1 break-all">
                {pool.token_mint_a}
              </div>
              <div className="text-xs text-muted-foreground mt-1">
                {orcaA?.decimals != null ? `${orcaA.decimals} decimals` : 'decimals (Orca) —'}
              </div>
            </div>
            <div className="p-4 rounded-lg border">
              <div className="font-medium">{symB}</div>
              <div className="text-sm text-muted-foreground font-mono mt-1 break-all">
                {pool.token_mint_b}
              </div>
              <div className="text-xs text-muted-foreground mt-1">
                {orcaB?.decimals != null ? `${orcaB.decimals} decimals` : 'decimals (Orca) —'}
              </div>
            </div>
          </div>
        </CardContent>
      </Card>
    </div>
  )
}
