import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useParams, Link, useNavigate } from 'react-router-dom'
import { ArrowLeft, Pencil, Trash2 } from 'lucide-react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { getStrategy, deleteStrategy } from '@/lib/api'
import { formatDate, shortenAddress } from '@/lib/utils'

export default function StrategyDetail() {
  const { id } = useParams<{ id: string }>()
  const navigate = useNavigate()
  const queryClient = useQueryClient()

  const { data: strategy, isLoading } = useQuery({
    queryKey: ['strategy', id],
    queryFn: () => getStrategy(id!),
    enabled: !!id,
  })

  const deleteMutation = useMutation({
    mutationFn: () => deleteStrategy(id!),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['strategies'] })
      queryClient.removeQueries({ queryKey: ['strategy', id] })
      navigate('/strategies')
    },
  })

  const handleDelete = () => {
    if (
      !id ||
      !window.confirm(
        'Delete this strategy? Linked position addresses are removed from the server. This cannot be undone.',
      )
    ) {
      return
    }
    deleteMutation.mutate()
  }

  if (isLoading) {
    return <div className="text-center py-8">Loading...</div>
  }

  if (!strategy) {
    return <div className="text-center py-8">Strategy not found</div>
  }

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-center gap-4">
        <Link to="/strategies">
          <Button variant="ghost" size="icon">
            <ArrowLeft className="h-4 w-4" />
          </Button>
        </Link>
        <h1 className="text-3xl font-bold">{strategy.name}</h1>
        <span className={`px-2 py-1 rounded-full text-xs font-medium ${
          strategy.running 
            ? 'bg-green-500/10 text-green-500' 
            : 'bg-muted text-muted-foreground'
        }`}>
          {strategy.running ? 'Running' : 'Stopped'}
        </span>
        <div className="ml-auto flex flex-wrap gap-2">
          <Link to={`/strategies/${id}/edit`}>
            <Button variant="outline" size="sm" type="button">
              <Pencil className="h-4 w-4 mr-2" />
              Edit
            </Button>
          </Link>
          <Button
            variant="destructive"
            size="sm"
            type="button"
            onClick={handleDelete}
            disabled={deleteMutation.isPending}
          >
            <Trash2 className="h-4 w-4 mr-2" />
            {deleteMutation.isPending ? 'Deleting...' : 'Delete'}
          </Button>
        </div>
      </div>

      <div className="grid gap-6 md:grid-cols-2">
        <Card>
          <CardHeader>
            <CardTitle>Configuration</CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="flex justify-between">
              <span className="text-muted-foreground">Type</span>
              <span className="capitalize">{strategy.strategy_type.replace('_', ' ')}</span>
            </div>
            <div className="flex justify-between gap-4">
              <span className="text-muted-foreground shrink-0">Pool</span>
              <span className="font-mono text-sm text-right">
                {strategy.pool_address
                  ? shortenAddress(strategy.pool_address, 6)
                  : 'Per position (Open Position)'}
              </span>
            </div>
            <div className="flex justify-between">
              <span className="text-muted-foreground">Created</span>
              <span>{formatDate(strategy.created_at)}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-muted-foreground">Updated</span>
              <span>{formatDate(strategy.updated_at)}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-muted-foreground">Dry run</span>
              <span>{strategy.dry_run ? 'Yes' : 'No'}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-muted-foreground">Auto-execute</span>
              <span>{strategy.auto_execute ? 'Yes' : 'No'}</span>
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Parameters</CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            {strategy.parameters.rebalance_threshold_pct !== undefined && (
              <div className="flex justify-between">
                <span className="text-muted-foreground">Rebalance Threshold</span>
                <span>{strategy.parameters.rebalance_threshold_pct}%</span>
              </div>
            )}
            {strategy.parameters.max_il_pct !== undefined && (
              <div className="flex justify-between">
                <span className="text-muted-foreground">Max IL</span>
                <span>{strategy.parameters.max_il_pct}%</span>
              </div>
            )}
            {(strategy.parameters.min_rebalance_interval_minutes !== undefined ||
              strategy.parameters.min_rebalance_interval_hours !== undefined) && (
              <div className="flex justify-between">
                <span className="text-muted-foreground">Min Rebalance Interval</span>
                <span>
                  {strategy.parameters.min_rebalance_interval_minutes ??
                    (strategy.parameters.min_rebalance_interval_hours ?? 0) * 60}
                  m
                </span>
              </div>
            )}
            {strategy.parameters.range_width_pct !== undefined && (
              <div className="flex justify-between">
                <span className="text-muted-foreground">Range Width</span>
                <span>{strategy.parameters.range_width_pct}%</span>
              </div>
            )}
            {strategy.parameters.retouch_offset_pct !== undefined && (
              <div className="flex justify-between">
                <span className="text-muted-foreground">Retouch Offset</span>
                <span>{strategy.parameters.retouch_offset_pct}%</span>
              </div>
            )}
            {strategy.parameters.candle_seconds !== undefined && (
              <div className="flex justify-between">
                <span className="text-muted-foreground">Candle seconds</span>
                <span>{strategy.parameters.candle_seconds}s</span>
              </div>
            )}
            {strategy.parameters.position_addresses &&
              strategy.parameters.position_addresses.length > 0 && (
                <div className="pt-2 border-t border-border">
                  <p className="text-xs text-muted-foreground mb-2">Linked positions</p>
                  <ul className="text-xs font-mono space-y-1">
                    {strategy.parameters.position_addresses.map((a) => (
                      <li key={a}>{shortenAddress(a, 8)}</li>
                    ))}
                  </ul>
                </div>
              )}
          </CardContent>
        </Card>
      </div>

      {strategy.description && (
        <Card>
          <CardHeader>
            <CardTitle>Description</CardTitle>
          </CardHeader>
          <CardContent>
            <p className="text-muted-foreground">{strategy.description}</p>
          </CardContent>
        </Card>
      )}
    </div>
  )
}
