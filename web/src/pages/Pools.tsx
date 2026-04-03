import { useQuery } from '@tanstack/react-query'
import { Link } from 'react-router-dom'
import { RefreshCw } from 'lucide-react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { getPools } from '@/lib/api'
import { formatUSD, formatPercent, shortenAddress } from '@/lib/utils'

export default function Pools() {
  const { data, isLoading, isError, error, refetch } = useQuery({
    queryKey: ['pools'],
    queryFn: getPools,
  })

  const raw = data?.pools
  const pools = Array.isArray(raw) ? raw : []

  return (
    <div className="space-y-6 text-foreground">
      <p className="text-xs text-muted-foreground">
        Pule ładowane są z publicznego API Orca przez backend — przy błędzie proxy/portu lista będzie pusta.
      </p>

      <div className="flex items-center justify-between">
        <h1 className="text-3xl font-bold">Pools</h1>
        <Button variant="outline" size="sm" onClick={() => refetch()}>
          <RefreshCw className="h-4 w-4 mr-2" />
          Refresh
        </Button>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Available Pools</CardTitle>
        </CardHeader>
        <CardContent>
          {isLoading ? (
            <div className="text-center py-8 text-muted-foreground">Loading...</div>
          ) : isError ? (
            <div className="rounded-md border border-destructive/40 bg-destructive/10 px-4 py-3 text-sm">
              <p className="font-medium text-destructive">Nie udało się pobrać pul</p>
              <p className="text-muted-foreground text-xs mt-1">
                {(error as Error)?.message ?? 'Unknown error'} — sprawdź, czy API działa i czy Vite proxy ma{' '}
                <code className="text-[11px]">API_UPSTREAM</code> na właściwy port.
              </p>
            </div>
          ) : pools.length === 0 ? (
            <div className="text-center py-8 text-muted-foreground text-sm space-y-2 max-w-lg mx-auto">
              <p>Brak pul z API Orca.</p>
              <p className="text-xs">
                Sprawdź, czy backend odpowiada (proxy Vite → ten sam port co <code className="text-[11px]">API_PORT</code>) oraz czy host ma dostęp do publicznego API Orca.
              </p>
            </div>
          ) : (
            <div className="overflow-x-auto">
              <table className="w-full">
                <thead>
                  <tr className="border-b text-left text-sm text-muted-foreground">
                    <th className="pb-3 font-medium">Pool</th>
                    <th className="pb-3 font-medium">Pair</th>
                    <th className="pb-3 font-medium">Protocol</th>
                    <th className="pb-3 font-medium text-right">TVL</th>
                    <th className="pb-3 font-medium text-right">24h Volume</th>
                    <th className="pb-3 font-medium text-right">Fee APY</th>
                  </tr>
                </thead>
                <tbody>
                  {pools.map((pool) => {
                    const pairLabel = `${shortenAddress(pool.token_mint_a, 4)}/${shortenAddress(pool.token_mint_b, 4)}`
                    const feePct = (pool.fee_rate_bps ?? 0) / 100
                    const addr = pool.address ?? 'unknown'
                    const tvl = pool.tvl_usd != null ? String(pool.tvl_usd) : '0'
                    const vol = pool.volume_24h_usd != null ? String(pool.volume_24h_usd) : '0'
                    const apy = pool.apy_estimate != null ? String(pool.apy_estimate) : '0'
                    return (
                    <tr key={addr} className="border-b last:border-0">
                      <td className="py-4">
                        <Link 
                          to={`/pools/${addr}`}
                          className="font-mono text-sm hover:text-primary"
                        >
                          {shortenAddress(addr)}
                        </Link>
                      </td>
                      <td className="py-4">
                        <span className="font-medium">
                          {pairLabel}
                        </span>
                        <span className="ml-2 text-xs text-muted-foreground">
                          {feePct.toFixed(2)}%
                        </span>
                      </td>
                      <td className="py-4 capitalize">{pool.protocol ?? '—'}</td>
                      <td className="py-4 text-right">{formatUSD(tvl)}</td>
                      <td className="py-4 text-right">{formatUSD(vol)}</td>
                      <td className="py-4 text-right text-green-500">
                        {formatPercent(apy)}
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
    </div>
  )
}
