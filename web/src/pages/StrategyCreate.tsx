import { useState } from 'react'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { useNavigate, Link } from 'react-router-dom'
import { ArrowLeft } from 'lucide-react'
import { Card, CardHeader, CardTitle, CardContent } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { createStrategy, StrategyType, CreateStrategyRequest } from '@/lib/api'

export default function StrategyCreate() {
  const navigate = useNavigate()
  const queryClient = useQueryClient()

  const [name, setName] = useState('')
  const [description, setDescription] = useState('')
  const [strategyType, setStrategyType] = useState<StrategyType>('static_range')
  const [poolAddress, setPoolAddress] = useState('')
  const [rebalanceThresholdPct, setRebalanceThresholdPct] = useState<number | ''>('')
  const [maxIlPct, setMaxIlPct] = useState<number | ''>('')
  const [minRebalanceIntervalHours, setMinRebalanceIntervalHours] = useState<number | ''>('')
  const [rangeWidthPct, setRangeWidthPct] = useState<number | ''>('')

  const mutation = useMutation({
    mutationFn: (data: CreateStrategyRequest) => createStrategy(data),
    onSuccess: (strategy) => {
      queryClient.invalidateQueries({ queryKey: ['strategies'] })
      navigate(`/strategies/${strategy.id}`)
    },
  })

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()
    if (!name.trim() || !poolAddress.trim()) {
      return
    }

    const payload: CreateStrategyRequest = {
      name: name.trim(),
      pool_address: poolAddress.trim(),
      strategy_type: strategyType,
      parameters: {
        ...(rebalanceThresholdPct !== '' && { rebalance_threshold_pct: Number(rebalanceThresholdPct) }),
        ...(maxIlPct !== '' && { max_il_pct: Number(maxIlPct) }),
        ...(minRebalanceIntervalHours !== '' && {
          min_rebalance_interval_hours: Number(minRebalanceIntervalHours),
        }),
        ...(rangeWidthPct !== '' && { range_width_pct: Number(rangeWidthPct) }),
      },
      auto_execute: false,
      dry_run: true,
    }

    mutation.mutate(payload)
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center gap-4">
        <Link to="/strategies">
          <Button variant="ghost" size="icon">
            <ArrowLeft className="h-4 w-4" />
          </Button>
        </Link>
        <h1 className="text-3xl font-bold">Create Strategy</h1>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Configuration</CardTitle>
        </CardHeader>
        <CardContent>
          <form className="space-y-4" onSubmit={handleSubmit}>
            <div>
              <label className="block text-sm font-medium mb-1">Name</label>
              <input
                className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                value={name}
                onChange={(e) => setName(e.target.value)}
                required
              />
            </div>

            <div>
              <label className="block text-sm font-medium mb-1">Description (optional)</label>
              <textarea
                className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm min-h-[80px]"
                value={description}
                onChange={(e) => setDescription(e.target.value)}
              />
            </div>

            <div className="grid gap-4 md:grid-cols-2">
              <div>
                <label className="block text-sm font-medium mb-1">Strategy Type</label>
                <select
                  className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                  value={strategyType}
                  onChange={(e) => setStrategyType(e.target.value as StrategyType)}
                >
                  <option value="static_range">Static</option>
                  <option value="periodic">Periodic</option>
                  <option value="threshold">Threshold</option>
                  <option value="il_limit">IL Limit</option>
                </select>
              </div>

              <div>
                <label className="block text-sm font-medium mb-1">Pool Address</label>
                <input
                  className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm font-mono"
                  value={poolAddress}
                  onChange={(e) => setPoolAddress(e.target.value)}
                  placeholder="Whirlpool pool address"
                  required
                />
              </div>
            </div>

            <div className="grid gap-4 md:grid-cols-2">
              <div>
                <label className="block text-sm font-medium mb-1">Range Width % (optional)</label>
                <input
                  type="number"
                  step="0.1"
                  className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                  value={rangeWidthPct}
                  onChange={(e) => setRangeWidthPct(e.target.value === '' ? '' : Number(e.target.value))}
                  placeholder="np. 4.0"
                />
              </div>
              <div>
                <label className="block text-sm font-medium mb-1">Max IL % (optional)</label>
                <input
                  type="number"
                  step="0.1"
                  className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                  value={maxIlPct}
                  onChange={(e) => setMaxIlPct(e.target.value === '' ? '' : Number(e.target.value))}
                  placeholder="np. 2.0"
                />
              </div>
            </div>

            <div className="grid gap-4 md:grid-cols-2">
              <div>
                <label className="block text-sm font-medium mb-1">Rebalance Threshold % (optional)</label>
                <input
                  type="number"
                  step="0.1"
                  className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                  value={rebalanceThresholdPct}
                  onChange={(e) =>
                    setRebalanceThresholdPct(e.target.value === '' ? '' : Number(e.target.value))
                  }
                  placeholder="np. 5.0"
                />
              </div>
              <div>
                <label className="block text-sm font-medium mb-1">Min Rebalance Interval (h, optional)</label>
                <input
                  type="number"
                  step="1"
                  className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                  value={minRebalanceIntervalHours}
                  onChange={(e) =>
                    setMinRebalanceIntervalHours(
                      e.target.value === '' ? '' : Number(e.target.value),
                    )
                  }
                  placeholder="np. 24"
                />
              </div>
            </div>

            <div className="flex justify-end gap-2 pt-2">
              <Link to="/strategies">
                <Button variant="outline" type="button">
                  Cancel
                </Button>
              </Link>
              <Button type="submit" disabled={mutation.isPending}>
                {mutation.isPending ? 'Creating...' : 'Create Strategy'}
              </Button>
            </div>
          </form>
        </CardContent>
      </Card>
    </div>
  )
}

