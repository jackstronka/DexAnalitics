import { useEffect, useMemo, useState } from 'react'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { useNavigate, Link } from 'react-router-dom'
import { ArrowLeft } from 'lucide-react'
import { Card, CardHeader, CardTitle, CardContent } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { TooltipProvider } from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'
import { useToast } from '@/hooks/use-toast'
import {
  STRATEGY_COPY,
  TOOLTIPS,
  FieldLabel,
  buildParameters,
  FIELD_ENABLED,
  isRangeWidthSatisfied,
} from '@/lib/strategyFormShared'
import {
  createStrategy,
  StrategyType,
  CreateStrategyRequest,
} from '@/lib/api'

function readOptionalNumber(raw: string): number | '' {
  if (raw.trim() === '') {
    return ''
  }
  const n = Number(raw)
  return Number.isFinite(n) ? n : ''
}

export default function StrategyCreate() {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const { toast } = useToast()

  const [name, setName] = useState('')
  const [description, setDescription] = useState('')
  const [strategyType, setStrategyType] = useState<StrategyType>('static_range')
  const [rebalanceThresholdPct, setRebalanceThresholdPct] = useState<number | ''>('')
  const [maxIlPct, setMaxIlPct] = useState<number | ''>('')
  const [minRebalanceIntervalHours, setMinRebalanceIntervalHours] = useState<number | ''>('')
  const [rangeWidthPct, setRangeWidthPct] = useState<number | ''>('')
  // Semantics toggles (defaults = old behavior).
  const [periodicRequiresOutOfRange, setPeriodicRequiresOutOfRange] = useState(false)
  const [rebalanceOnRangeExitImmediately, setRebalanceOnRangeExitImmediately] = useState(true)
  const [autoStart, setAutoStart] = useState(true)

  const enabled = FIELD_ENABLED[strategyType]

  useEffect(() => {
    switch (strategyType) {
      case 'static_range':
        setMaxIlPct('')
        setRebalanceThresholdPct('')
        setMinRebalanceIntervalHours('')
        break
      case 'periodic':
        setMaxIlPct('')
        setRebalanceThresholdPct('')
        break
      case 'threshold':
      case 'oor_recenter':
      case 'retouch_shift':
        setMaxIlPct('')
        break
      default:
        break
    }

    // Periodic-only toggle is ignored unless the type is periodic.
    if (strategyType !== 'periodic') {
      setPeriodicRequiresOutOfRange(false)
    }
  }, [strategyType])

  const mutation = useMutation({
    mutationFn: (data: CreateStrategyRequest) => createStrategy(data),
    onSuccess: (strategy) => {
      queryClient.invalidateQueries({ queryKey: ['strategies'] })
      navigate(`/strategies/${strategy.id}`)
    },
    onError: (err: Error) => {
      toast({
        title: 'Could not create strategy',
        description:
          err.message ||
          'Check that the API is running and reachable (e.g. clmm-lp-api on the port Vite proxies to).',
        variant: 'destructive',
      })
    },
  })

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()
    if (!name.trim()) {
      toast({
        title: 'Name required',
        description: 'Enter a strategy name — pool is not required at this step.',
        variant: 'destructive',
      })
      return
    }
    if (!isRangeWidthSatisfied(strategyType, rangeWidthPct)) {
      toast({
        title: 'Range width required',
        description: 'Set Range Width % (greater than 0, at most 100) for this strategy type.',
        variant: 'destructive',
      })
      return
    }

    const payload: CreateStrategyRequest = {
      name: name.trim(),
      strategy_type: strategyType,
      parameters: buildParameters(strategyType, {
        rangeWidthPct,
        maxIlPct,
        rebalanceThresholdPct,
        minRebalanceIntervalHours,
        periodicRequiresOutOfRange,
        rebalanceOnRangeExitImmediately,
        autoStart,
      }),
      // Zawsze wysyłaj pole — starsze API wymagały `pool_address` w body; pusty = brak puli (pool przy Open Position).
      pool_address: '',
      auto_execute: false,
      dry_run: true,
    }

    mutation.mutate(payload)
  }

  const strategyBlurb = STRATEGY_COPY[strategyType]

  const minIntervalLabel = useMemo(() => {
    if (strategyType === 'periodic') {
      return 'Rebalance interval (h)'
    }
    return 'Min. rebalance spacing (h)'
  }, [strategyType])

  const rebalanceThresholdTooltip = useMemo(() => {
    if (strategyType === 'il_limit') {
      return TOOLTIPS.rebalanceThresholdIl
    }
    return TOOLTIPS.rebalanceThresholdThreshold
  }, [strategyType])

  const minIntervalTooltip = useMemo(() => {
    if (strategyType === 'periodic') {
      return TOOLTIPS.minIntervalPeriodic
    }
    return TOOLTIPS.minIntervalOther
  }, [strategyType])

  const inputDisabled = 'disabled:cursor-not-allowed disabled:opacity-60'

  return (
    <TooltipProvider delayDuration={200}>
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
            <form className="space-y-4" onSubmit={handleSubmit} noValidate>
              <div>
                <FieldLabel htmlFor="strategy-name" label="Name" tooltip={TOOLTIPS.name} />
                <input
                  id="strategy-name"
                  className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  required
                />
              </div>

              <div>
                <FieldLabel
                  htmlFor="strategy-desc"
                  label="Description (optional)"
                  tooltip={TOOLTIPS.description}
                />
                <textarea
                  id="strategy-desc"
                  className={cn(
                    'w-full rounded-md border border-input bg-background px-3 py-2 text-sm min-h-[80px]',
                  )}
                  value={description}
                  onChange={(e) => setDescription(e.target.value)}
                />
              </div>

              <p className="text-xs text-muted-foreground rounded-md border border-border bg-muted/20 px-3 py-2">
                <span className="text-foreground font-medium">No pool needed here</span> — pool is
                chosen when you{' '}
                <Link to="/positions/new" className="text-primary underline underline-offset-2">
                  open a position
                </Link>
                ; there you attach this strategy to that position.
              </p>

              <div>
                <FieldLabel
                  htmlFor="strategy-type"
                  label="Strategy Type"
                  tooltip={TOOLTIPS.strategyType}
                />
                <select
                  id="strategy-type"
                  className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                  value={strategyType}
                  onChange={(e) => setStrategyType(e.target.value as StrategyType)}
                >
                  <option value="static_range">Static</option>
                  <option value="periodic">Periodic</option>
                  <option value="threshold">Threshold</option>
                  <option value="il_limit">IL Limit</option>
                  <option value="oor_recenter">OOR recenter</option>
                  <option value="retouch_shift">Retouch shift</option>
                </select>
              </div>

              <div
                className="rounded-md border border-border bg-muted/30 px-3 py-3 text-sm text-muted-foreground"
                role="status"
              >
                <p className="font-medium text-foreground">{strategyBlurb.title}</p>
                <p className="mt-1 leading-relaxed">{strategyBlurb.body}</p>
              </div>

              <div className="grid gap-4 md:grid-cols-2">
                <div>
                  <FieldLabel
                    htmlFor="range-width"
                    label={
                      enabled.rangeWidth ? 'Range Width % (required)' : 'Range Width % (n/a for this type)'
                    }
                    tooltip={TOOLTIPS.rangeWidth}
                  />
                  <input
                    id="range-width"
                    type="number"
                    step="0.1"
                    min={enabled.rangeWidth ? 0.01 : undefined}
                    max={enabled.rangeWidth ? 100 : undefined}
                    disabled={!enabled.rangeWidth}
                    required={enabled.rangeWidth}
                    className={cn(
                      'w-full rounded-md border border-input bg-background px-3 py-2 text-sm',
                      inputDisabled,
                    )}
                    value={rangeWidthPct}
                    onChange={(e) => setRangeWidthPct(readOptionalNumber(e.target.value))}
                    placeholder={enabled.rangeWidth ? 'e.g. 1.0 for ~±0.5% price band' : '—'}
                  />
                </div>
                <div>
                  <FieldLabel
                    htmlFor="max-il"
                    label="Max IL % (optional)"
                    tooltip={TOOLTIPS.maxIl}
                  />
                  <input
                    id="max-il"
                    type="number"
                    step="0.1"
                    disabled={!enabled.maxIl}
                    className={cn(
                      'w-full rounded-md border border-input bg-background px-3 py-2 text-sm',
                      inputDisabled,
                    )}
                    value={maxIlPct}
                    onChange={(e) => setMaxIlPct(readOptionalNumber(e.target.value))}
                    placeholder="e.g. 2.0"
                  />
                </div>
              </div>

              <div className="grid gap-4 md:grid-cols-2">
                <div>
                  <FieldLabel
                    htmlFor="rebalance-threshold"
                    label={
                      strategyType === 'il_limit'
                        ? 'IL rebalance threshold % (optional)'
                        : 'Rebalance threshold % (optional)'
                    }
                    tooltip={rebalanceThresholdTooltip}
                  />
                  <input
                    id="rebalance-threshold"
                    type="number"
                    step="0.1"
                    disabled={!enabled.rebalanceThreshold}
                    className={cn(
                      'w-full rounded-md border border-input bg-background px-3 py-2 text-sm',
                      inputDisabled,
                    )}
                    value={rebalanceThresholdPct}
                    onChange={(e) => setRebalanceThresholdPct(readOptionalNumber(e.target.value))}
                    placeholder="e.g. 5.0"
                  />
                </div>
                <div>
                  <FieldLabel
                    htmlFor="min-interval"
                    label={`${minIntervalLabel} (optional)`}
                    tooltip={minIntervalTooltip}
                  />
                  <input
                    id="min-interval"
                    type="number"
                    step="1"
                    disabled={!enabled.minInterval}
                    className={cn(
                      'w-full rounded-md border border-input bg-background px-3 py-2 text-sm',
                      inputDisabled,
                    )}
                    value={minRebalanceIntervalHours}
                    onChange={(e) =>
                      setMinRebalanceIntervalHours(readOptionalNumber(e.target.value))
                    }
                    placeholder="e.g. 24"
                  />
                </div>
              </div>

              <div className="space-y-2 rounded-md border border-border bg-muted/20 px-3 py-3">
                <p className="text-sm font-medium text-foreground">Semantyka rebalance</p>
                <div className="grid gap-3 md:grid-cols-2">
                  <div className="flex items-start gap-2">
                    <input
                      id="create-rebalance-on-exit"
                      type="checkbox"
                      checked={rebalanceOnRangeExitImmediately}
                      onChange={(e) => setRebalanceOnRangeExitImmediately(e.target.checked)}
                      className="mt-0.5 rounded border-input"
                    />
                    <div className="flex-1">
                      <FieldLabel
                        htmlFor="create-rebalance-on-exit"
                        label="Rebalance immediately on range-exit (OOR)"
                        tooltip={TOOLTIPS.rebalanceOnRangeExitImmediately}
                      />
                      <p className="text-xs text-muted-foreground">
                        Default: on (old behavior).
                      </p>
                    </div>
                  </div>

                  <div className="flex items-start gap-2">
                    <input
                      id="create-periodic-requires-oor"
                      type="checkbox"
                      checked={periodicRequiresOutOfRange}
                      onChange={(e) => setPeriodicRequiresOutOfRange(e.target.checked)}
                      className="mt-0.5 rounded border-input"
                      disabled={strategyType !== 'periodic'}
                    />
                    <div className={cn('flex-1', strategyType !== 'periodic' && 'opacity-60')}>
                      <FieldLabel
                        htmlFor="create-periodic-requires-oor"
                        label="Periodic only when OOR"
                        tooltip={TOOLTIPS.periodicRequiresOor}
                      />
                      <p className="text-xs text-muted-foreground">
                        Enabled only for Periodic strategy type.
                      </p>
                    </div>
                  </div>
                </div>
              </div>

              <div className="space-y-2 rounded-md border border-border bg-muted/20 px-3 py-3">
                <p className="text-sm font-medium text-foreground">Startup</p>
                <div className="flex items-start gap-2">
                  <input
                    id="create-auto-start"
                    type="checkbox"
                    checked={autoStart}
                    onChange={(e) => setAutoStart(e.target.checked)}
                    className="mt-0.5 rounded border-input"
                  />
                  <div className="flex-1">
                    <FieldLabel
                      htmlFor="create-auto-start"
                      label="Auto-start on API boot"
                      tooltip={TOOLTIPS.autoStart}
                    />
                    <p className="text-xs text-muted-foreground">
                      Requires server env <code className="text-[11px]">CLMM_STRATEGY_AUTOSTART_ON_BOOT=1</code>.
                    </p>
                  </div>
                </div>
              </div>

              {mutation.isError && (
                <p className="text-sm text-destructive" role="alert">
                  {(mutation.error as Error)?.message ?? 'Request failed.'}
                </p>
              )}

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
    </TooltipProvider>
  )
}
