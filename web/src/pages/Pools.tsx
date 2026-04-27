import { useQuery } from '@tanstack/react-query'
import { useState } from 'react'
import { Link } from 'react-router-dom'
import { RefreshCw } from 'lucide-react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { getPools } from '@/lib/api'
import { formatUSD, formatPercent, shortenAddress } from '@/lib/utils'
import { useI18n } from '@/lib/i18n'

export default function Pools() {
  const { locale } = useI18n()
  const [volumeWindow, setVolumeWindow] = useState<'5m' | '1h' | '24h' | '7d'>('24h')
  const { data, isLoading, isError, error, refetch } = useQuery({
    queryKey: ['pools'],
    queryFn: getPools,
  })

  const raw = data?.pools
  const pools = Array.isArray(raw) ? raw : []

  return (
    <div className="space-y-6 text-foreground">
      <p className="text-xs text-muted-foreground">
        {locale === 'pl'
          ? 'Pule ładowane są z publicznego API Orca przez backend — przy błędzie proxy/portu lista będzie pusta.'
          : 'Pools are loaded from public Orca API through backend — proxy/port errors can return an empty list.'}
      </p>

      <div className="flex items-center justify-between">
        <h1 className="text-3xl font-bold">{locale === 'pl' ? 'Pule' : 'Pools'}</h1>
        <div className="flex items-center gap-2">
          <select
            className="rounded border bg-background px-2 py-1 text-sm"
            value={volumeWindow}
            onChange={(e) => setVolumeWindow(e.target.value as '5m' | '1h' | '24h' | '7d')}
            title={locale === 'pl' ? 'Okno czasowe dla kolumny Volume' : 'Time window for Volume column'}
          >
            <option value="5m">{locale === 'pl' ? 'Wolumen 5m' : 'Volume 5m'}</option>
            <option value="1h">{locale === 'pl' ? 'Wolumen 1h' : 'Volume 1h'}</option>
            <option value="24h">{locale === 'pl' ? 'Wolumen 24h' : 'Volume 24h'}</option>
            <option value="7d">{locale === 'pl' ? 'Wolumen 7d' : 'Volume 7d'}</option>
          </select>
          <Button variant="outline" size="sm" onClick={() => refetch()}>
            <RefreshCw className="h-4 w-4 mr-2" />
            {locale === 'pl' ? 'Odśwież' : 'Refresh'}
          </Button>
        </div>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>{locale === 'pl' ? 'Dostępne pule' : 'Available Pools'}</CardTitle>
        </CardHeader>
        <CardContent>
          {isLoading ? (
            <div className="text-center py-8 text-muted-foreground">{locale === 'pl' ? 'Ładowanie...' : 'Loading...'}</div>
          ) : isError ? (
            <div className="rounded-md border border-destructive/40 bg-destructive/10 px-4 py-3 text-sm">
              <p className="font-medium text-destructive">Nie udało się pobrać pul</p>
              <p className="text-muted-foreground text-xs mt-1">
                {(error as Error)?.message ?? (locale === 'pl' ? 'Nieznany błąd' : 'Unknown error')} —{' '}
                {locale === 'pl' ? 'sprawdź, czy API działa i czy Vite proxy ma' : 'check API status and Vite proxy'}{' '}
                <code className="text-[11px]">API_UPSTREAM</code> na właściwy port.
              </p>
            </div>
          ) : pools.length === 0 ? (
            <div className="text-center py-8 text-muted-foreground text-sm space-y-2 max-w-lg mx-auto">
              <p>{locale === 'pl' ? 'Brak pul z API Orca.' : 'No pools from Orca API.'}</p>
              <p className="text-xs">
                {locale === 'pl'
                  ? <>Sprawdź, czy backend odpowiada (proxy Vite → ten sam port co <code className="text-[11px]">API_PORT</code>) oraz czy host ma dostęp do publicznego API Orca.</>
                  : <>Check backend availability (Vite proxy → same port as <code className="text-[11px]">API_PORT</code>) and host access to public Orca API.</>}
              </p>
            </div>
          ) : (
            <div className="overflow-x-auto">
              <table className="w-full">
                <thead>
                  <tr className="border-b text-left text-sm text-muted-foreground">
                    <th className="pb-3 font-medium">{locale === 'pl' ? 'Pula' : 'Pool'}</th>
                    <th className="pb-3 font-medium">{locale === 'pl' ? 'Para' : 'Pair'}</th>
                    <th className="pb-3 font-medium">{locale === 'pl' ? 'Protokół' : 'Protocol'}</th>
                    <th className="pb-3 font-medium text-right">TVL</th>
                    <th className="pb-3 font-medium text-right">{locale === 'pl' ? 'Wolumen' : 'Volume'} ({volumeWindow})</th>
                    <th className="pb-3 font-medium text-right">Fee APY</th>
                  </tr>
                </thead>
                <tbody>
                  {pools.map((pool) => {
                    const pairLabel = `${shortenAddress(pool.token_mint_a, 4)}/${shortenAddress(pool.token_mint_b, 4)}`
                    const feePct = (pool.fee_rate_bps ?? 0) / 100
                    const addr = pool.address ?? 'unknown'
                    const tvl = pool.tvl_usd != null ? String(pool.tvl_usd) : '0'
                    const volRaw =
                      volumeWindow === '5m'
                        ? pool.volume_5m_usd
                        : volumeWindow === '1h'
                          ? pool.volume_1h_usd
                          : volumeWindow === '7d'
                            ? pool.volume_7d_usd
                            : pool.volume_24h_usd
                    const vol = volRaw != null ? String(volRaw) : '0'
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
