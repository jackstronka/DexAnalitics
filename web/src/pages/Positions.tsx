import { useQuery } from '@tanstack/react-query'
import { useState } from 'react'
import { Link, useNavigate } from 'react-router-dom'
import { Plus, RefreshCw } from 'lucide-react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import ApiDataHint from '@/components/ApiDataHint'
import { getOrcaPositionsByOwner, getPositions } from '@/lib/api'
import { getDevWalletPubkey } from '@/lib/devWallet'
import { formatUSD, formatPercent, shortenAddress, formatUsdcPriceRange } from '@/lib/utils'

function rangeCellClass(inRange: boolean | undefined) {
  if (inRange === true) {
    return 'text-emerald-600 dark:text-emerald-400 border-l-2 border-emerald-500 pl-2'
  }
  if (inRange === false) {
    return 'text-red-600 dark:text-red-400 border-l-2 border-red-500 pl-2'
  }
  return 'text-muted-foreground border-l-2 border-border pl-2'
}

function rangeStatusLabel(inRange: boolean | undefined) {
  if (inRange === true) return 'In range'
  if (inRange === false) return 'Out of range'
  return '—'
}

export default function Positions() {
  const navigate = useNavigate()
  const devPk = getDevWalletPubkey()
  const [ownerInput, setOwnerInput] = useState(() => devPk ?? '')
  const [appliedOwner, setAppliedOwner] = useState(() => devPk ?? '')

  const { data, isLoading, refetch } = useQuery({
    queryKey: ['positions'],
    queryFn: getPositions,
  })

  const chainQ = useQuery({
    queryKey: ['orca-positions-by-owner', appliedOwner],
    queryFn: () => getOrcaPositionsByOwner(appliedOwner),
    enabled: appliedOwner.trim().length > 0,
    staleTime: 60_000,
  })

  const positions = data?.positions || []

  return (
    <div className="space-y-6">
      <ApiDataHint />

      <div className="flex items-center justify-between">
        <h1 className="text-3xl font-bold">Positions</h1>
        <div className="flex gap-2">
          <Button variant="outline" size="sm" onClick={() => refetch()}>
            <RefreshCw className="h-4 w-4 mr-2" />
            Refresh
          </Button>
          <Button size="sm" onClick={() => navigate('/positions/new')}>
            <Plus className="h-4 w-4 mr-2" />
            Open Position
          </Button>
        </div>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Monitored positions (API)</CardTitle>
          <p className="text-sm text-muted-foreground font-normal">
            Z monitora w pamięci procesu — szczegóły i PnL tylko dla tych adresów.
          </p>
        </CardHeader>
        <CardContent>
          {isLoading ? (
            <div className="text-center py-8 text-muted-foreground">Loading...</div>
          ) : positions.length === 0 ? (
            <div className="text-center py-8 text-muted-foreground space-y-2 max-w-xl mx-auto">
              <p>Brak pozycji w monitorze API — to nie jest lista wszystkich NFT Orca na portfelu.</p>
              <p className="text-xs">
                Uruchom strategię z adresami pozycji, dodaj pozycję do monitora, albo sprawdź on-chain:{' '}
                <code className="text-[11px]">orca-positions-list</code> (CLI).
              </p>
            </div>
          ) : (
            <div className="overflow-x-auto">
              <table className="w-full">
                <thead>
                  <tr className="border-b text-left text-sm text-muted-foreground">
                    <th className="pb-3 font-medium">Position</th>
                    <th className="pb-3 font-medium">Pool</th>
                    <th className="pb-3 font-medium">Range (in / out)</th>
                    <th className="pb-3 font-medium text-right">Value</th>
                    <th className="pb-3 font-medium text-right">PnL</th>
                    <th className="pb-3 font-medium text-right">Fees</th>
                    <th className="pb-3 font-medium text-center">Status</th>
                  </tr>
                </thead>
                <tbody>
                  {positions.map((position) => (
                    <tr key={position.address} className="border-b last:border-0">
                      <td className="py-4">
                        <Link 
                          to={`/positions/${position.address}`}
                          className="font-medium hover:text-primary"
                        >
                          {shortenAddress(position.address)}
                        </Link>
                      </td>
                      <td className="py-4 text-muted-foreground">
                        {shortenAddress(position.pool_address)}
                      </td>
                      <td className="py-4">
                        <div className="space-y-0.5">
                          <span className={`text-sm block ${rangeCellClass(position.in_range)}`}>
                            {formatUsdcPriceRange(
                              position.range_lower_usdc ?? undefined,
                              position.range_upper_usdc ?? undefined,
                              position.range_usdc_quote ?? undefined,
                            ) ?? `${position.tick_lower} → ${position.tick_upper}`}
                          </span>
                          <span className="text-[11px] text-muted-foreground">
                            {rangeStatusLabel(position.in_range)}
                          </span>
                        </div>
                      </td>
                      <td className="py-4 text-right font-medium">
                        {formatUSD(position.value_usd)}
                      </td>
                      <td className={`py-4 text-right ${
                        parseFloat(position.pnl.net_pnl_pct) >= 0 ? 'text-green-500' : 'text-red-500'
                      }`}>
                        {formatPercent(position.pnl.net_pnl_pct)}
                      </td>
                      <td className="py-4 text-right text-green-500">
                        {formatUSD(position.pnl.fees_earned_usd)}
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
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>On-chain Orca positions (RPC)</CardTitle>
          <p className="text-sm text-muted-foreground font-normal">
            Skan NFT Whirlpool dla portfela — to samo co <code className="text-[11px]">orca-positions-list</code>. Wymaga
            działającego RPC w API; nie używa monitora strategii.
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
              Load on-chain
            </Button>
            <Button
              type="button"
              variant="outline"
              onClick={() => chainQ.refetch()}
              disabled={!appliedOwner.trim()}
            >
              <RefreshCw className="h-4 w-4 mr-2" />
              Refresh
            </Button>
          </div>
          {chainQ.isLoading ? (
            <div className="text-center py-6 text-muted-foreground">Loading RPC…</div>
          ) : chainQ.error ? (
            <div className="rounded-md border border-destructive/40 bg-destructive/5 px-3 py-2 text-sm text-destructive">
              {(chainQ.error as Error).message}
            </div>
          ) : !appliedOwner.trim() ? (
            <div className="text-muted-foreground text-sm">Podaj owner i kliknij „Load on-chain”.</div>
          ) : (
            <>
              <p className="text-xs text-muted-foreground">
                RPC: <code className="break-all">{chainQ.data?.rpc_url ?? '—'}</code> — znaleziono:{' '}
                <strong>{chainQ.data?.total ?? 0}</strong>
              </p>
              {(chainQ.data?.entries?.length ?? 0) === 0 ? (
                <div className="text-muted-foreground text-sm py-4">Brak pozycji Whirlpool dla tego ownera.</div>
              ) : (
                <div className="overflow-x-auto">
                  <table className="w-full">
                    <thead>
                      <tr className="border-b text-left text-sm text-muted-foreground">
                        <th className="pb-3 font-medium">Kind</th>
                        <th className="pb-3 font-medium">Position</th>
                        <th className="pb-3 font-medium">Pool</th>
                        <th className="pb-3 font-medium">Range (in / out)</th>
                        <th className="pb-3 font-medium text-right">Liquidity (raw)</th>
                      </tr>
                    </thead>
                    <tbody>
                      {chainQ.data!.entries.map((row) => (
                        <tr key={row.position_address} className="border-b last:border-0">
                          <td className="py-3 text-xs">{row.kind}</td>
                          <td className="py-3 font-mono text-xs">
                            <Link
                              to={`/positions/${row.position_address}`}
                              className="hover:text-primary"
                            >
                              {shortenAddress(row.position_address)}
                            </Link>
                            {row.position_bundle_address && (
                              <span className="block text-muted-foreground mt-0.5">
                                bundle {shortenAddress(row.position_bundle_address)}
                              </span>
                            )}
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
                                ) ?? `${row.tick_lower} → ${row.tick_upper}`}
                              </span>
                              <span className="text-[11px] text-muted-foreground">
                                {rangeStatusLabel(row.in_range)}
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
