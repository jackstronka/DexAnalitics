import { useQuery } from '@tanstack/react-query'
import { Link } from 'react-router-dom'
import { 
  TrendingUp, 
  TrendingDown, 
  DollarSign, 
  Activity,
  AlertTriangle,
  ArrowRight
} from 'lucide-react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import ApiDataHint from '@/components/ApiDataHint'
import { getPortfolioAnalytics, getPositions, getHealth } from '@/lib/api'
import { formatUSD, formatPercent, formatUsdcPriceRange } from '@/lib/utils'

export default function Dashboard() {
  const { data: analytics, isLoading: analyticsLoading } = useQuery({
    queryKey: ['portfolio-analytics'],
    queryFn: getPortfolioAnalytics,
  })

  const { data: positionsData, isLoading: positionsLoading } = useQuery({
    queryKey: ['positions'],
    queryFn: getPositions,
  })

  const { data: health } = useQuery({
    queryKey: ['health'],
    queryFn: getHealth,
    refetchInterval: 30000,
  })

  const positions = positionsData?.positions || []
  const activePositions = positions.filter(p => p.status === 'active')

  return (
    <div className="space-y-6">
      <ApiDataHint />

      <div className="flex items-center justify-between">
        <h1 className="text-3xl font-bold">Dashboard</h1>
        <div className="flex items-center gap-2">
          <div className={`h-2 w-2 rounded-full ${health?.status === 'healthy' ? 'bg-green-500' : 'bg-yellow-500'}`} />
          <span className="text-sm text-muted-foreground">
            {health?.status || 'Checking...'}
          </span>
        </div>
      </div>

      {/* Stats Grid */}
      <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
        <Card>
          <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
            <CardTitle className="text-sm font-medium">Total Value</CardTitle>
            <DollarSign className="h-4 w-4 text-muted-foreground" />
          </CardHeader>
          <CardContent>
            <div className="text-2xl font-bold">
              {analyticsLoading ? '...' : formatUSD(analytics?.total_value_usd || '0')}
            </div>
            <p className="text-xs text-muted-foreground">
              {analytics?.active_positions || 0} active positions
            </p>
          </CardContent>
        </Card>

        <Card>
          <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
            <CardTitle className="text-sm font-medium">Total PnL</CardTitle>
            {parseFloat(analytics?.total_pnl_pct || '0') >= 0 ? (
              <TrendingUp className="h-4 w-4 text-green-500" />
            ) : (
              <TrendingDown className="h-4 w-4 text-red-500" />
            )}
          </CardHeader>
          <CardContent>
            <div className={`text-2xl font-bold ${
              parseFloat(analytics?.total_pnl_pct || '0') >= 0 ? 'text-green-500' : 'text-red-500'
            }`}>
              {analyticsLoading ? '...' : formatPercent(analytics?.total_pnl_pct || '0')}
            </div>
            <p className="text-xs text-muted-foreground">
              {formatUSD(analytics?.total_pnl_usd || '0')}
            </p>
          </CardContent>
        </Card>

        <Card>
          <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
            <CardTitle className="text-sm font-medium">Fees Earned</CardTitle>
            <Activity className="h-4 w-4 text-muted-foreground" />
          </CardHeader>
          <CardContent>
            <div className="text-2xl font-bold text-green-500">
              {analyticsLoading ? '...' : formatUSD(analytics?.total_fees_usd || '0')}
            </div>
            <p className="text-xs text-muted-foreground">
              Lifetime earnings
            </p>
          </CardContent>
        </Card>

        <Card>
          <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
            <CardTitle className="text-sm font-medium">IL (avg %)</CardTitle>
            <AlertTriangle className="h-4 w-4 text-yellow-500" />
          </CardHeader>
          <CardContent>
            <div className="text-2xl font-bold text-yellow-500">
              {analyticsLoading ? '...' : formatPercent(analytics?.total_il_pct || '0')}
            </div>
            <p className="text-xs text-muted-foreground">
              Across monitored positions
            </p>
          </CardContent>
        </Card>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Portfolio over time</CardTitle>
        </CardHeader>
        <CardContent className="text-sm text-muted-foreground">
          Historical time series is not wired to a dedicated endpoint yet. Use{' '}
          <Link to="/wallet" className="text-primary underline">
            Wallet
          </Link>{' '}
          for current totals and open positions.
        </CardContent>
      </Card>

      {/* Active Positions */}
      <Card>
        <CardHeader className="flex flex-row items-center justify-between">
          <CardTitle>Active Positions</CardTitle>
          <Link to="/positions">
            <Button variant="ghost" size="sm">
              View All <ArrowRight className="ml-2 h-4 w-4" />
            </Button>
          </Link>
        </CardHeader>
        <CardContent>
          {positionsLoading ? (
            <div className="text-center py-8 text-muted-foreground">Loading...</div>
          ) : activePositions.length === 0 ? (
            <div className="text-center py-8 text-muted-foreground">
              No active positions
            </div>
          ) : (
            <div className="space-y-4">
              {activePositions.slice(0, 5).map((position) => (
                <Link 
                  key={position.address}
                  to={`/positions/${position.address}`}
                  className="flex items-center justify-between p-4 rounded-lg border hover:bg-accent transition-colors"
                >
                  <div>
                    <div className="font-medium">
                      {position.pool_address.slice(0, 8)}...
                    </div>
                    <div className="text-sm text-muted-foreground">
                      {formatUsdcPriceRange(
                        position.range_lower_usdc ?? undefined,
                        position.range_upper_usdc ?? undefined,
                        position.range_usdc_quote ?? undefined,
                      ) ?? `Tick: ${position.tick_lower} → ${position.tick_upper}`}
                    </div>
                  </div>
                  <div className="text-right">
                    <div className="font-medium">
                      {formatUSD(position.value_usd)}
                    </div>
                    <div className={`text-sm ${
                      parseFloat(position.pnl.net_pnl_pct) >= 0 ? 'text-green-500' : 'text-red-500'
                    }`}>
                      {formatPercent(position.pnl.net_pnl_pct)}
                    </div>
                  </div>
                  <div className={`h-2 w-2 rounded-full ${position.in_range ? 'bg-green-500' : 'bg-yellow-500'}`} />
                </Link>
              ))}
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  )
}
