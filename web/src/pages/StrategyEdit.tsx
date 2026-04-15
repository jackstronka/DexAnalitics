import { useEffect, useMemo, useRef, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useNavigate, Link, useParams } from 'react-router-dom'
import { ArrowLeft } from 'lucide-react'
import { Card, CardHeader, CardTitle, CardContent } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { TooltipProvider } from '@/components/ui/tooltip'
import { useToast } from '@/hooks/use-toast'
import { cn } from '@/lib/utils'
import {
  STRATEGY_COPY,
  TOOLTIPS,
  FieldLabel,
  buildParameters,
  FIELD_ENABLED,
  isRangeWidthSatisfied,
} from '@/lib/strategyFormShared'
import {
  getStrategy,
  updateStrategy,
  StrategyType,
  CreateStrategyRequest,
  StrategyParameters,
} from '@/lib/api'

function numOrEmpty(v: number | undefined): number | '' {
  if (v === undefined || v === null) {
    return ''
  }
  const n = Number(v)
  return Number.isFinite(n) ? n : ''
}

function readOptionalNumber(raw: string): number | '' {
  if (raw.trim() === '') {
    return ''
  }
  const n = Number(raw)
  return Number.isFinite(n) ? n : ''
}

export default function StrategyEdit() {
  const { id } = useParams<{ id: string }>()
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const { toast } = useToast()

  const skipTypeResetOnce = useRef(true)

  const { data: strategy, isLoading, isError } = useQuery({
    queryKey: ['strategy', id],
    queryFn: () => getStrategy(id!),
    enabled: !!id,
  })

  const [name, setName] = useState('')
  const [strategyType, setStrategyType] = useState<StrategyType>('static_range')
  const [rebalanceThresholdPct, setRebalanceThresholdPct] = useState<number | ''>('')
  const [maxIlPct, setMaxIlPct] = useState<number | ''>('')
  const [minRebalanceIntervalHours, setMinRebalanceIntervalHours] = useState<number | ''>('')
  const [rangeWidthPct, setRangeWidthPct] = useState<number | ''>('')
  const [dryRun, setDryRun] = useState(true)
  const [autoExecute, setAutoExecute] = useState(false)
  // Semantics toggles (defaults = old behavior).
  const [periodicRequiresOutOfRange, setPeriodicRequiresOutOfRange] = useState(false)
  const [rebalanceOnRangeExitImmediately, setRebalanceOnRangeExitImmediately] = useState(true)
  const [autoStart, setAutoStart] = useState(true)

  useEffect(() => {
    if (!strategy) {
      return
    }
    skipTypeResetOnce.current = true
    setName(strategy.name)
    setStrategyType(strategy.strategy_type)
    const p = strategy.parameters
    setRangeWidthPct(numOrEmpty(p.range_width_pct))
    setMaxIlPct(numOrEmpty(p.max_il_pct))
    setRebalanceThresholdPct(numOrEmpty(p.rebalance_threshold_pct))
    setMinRebalanceIntervalHours(numOrEmpty(p.min_rebalance_interval_hours))
    setPeriodicRequiresOutOfRange(Boolean(p.periodic_requires_out_of_range))
    setAutoStart(p.auto_start === undefined ? true : Boolean(p.auto_start))
    // Default to old behavior when absent.
    setRebalanceOnRangeExitImmediately(
      p.rebalance_on_range_exit_immediately === undefined
        ? true
        : Boolean(p.rebalance_on_range_exit_immediately),
    )
    setDryRun(strategy.dry_run ?? true)
    setAutoExecute(strategy.auto_execute ?? false)
  }, [strategy])

  useEffect(() => {
    if (skipTypeResetOnce.current) {
      skipTypeResetOnce.current = false
      return
    }
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

    if (strategyType !== 'periodic') {
      setPeriodicRequiresOutOfRange(false)
    }
  }, [strategyType])

  const enabled = FIELD_ENABLED[strategyType] ?? FIELD_ENABLED.static_range

  const mutation = useMutation({
    mutationFn: (data: CreateStrategyRequest) => updateStrategy(id!, data),
    onSuccess: (updated) => {
      queryClient.invalidateQueries({ queryKey: ['strategies'] })
      queryClient.invalidateQueries({ queryKey: ['strategy', updated.id] })
      navigate(`/strategies/${updated.id}`)
    },
    onError: (err: Error) => {
      toast({
        title: 'Could not save strategy',
        description: err.message,
        variant: 'destructive',
      })
    },
  })

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()
    if (!strategy) {
      return
    }
    if (!name.trim()) {
      toast({
        title: 'Name required',
        description: 'Enter a strategy name before saving.',
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

    const built = buildParameters(strategyType, {
      rangeWidthPct,
      maxIlPct,
      rebalanceThresholdPct,
      minRebalanceIntervalHours,
      periodicRequiresOutOfRange,
      rebalanceOnRangeExitImmediately,
      autoStart,
    })

    const parameters: StrategyParameters = {
      ...built,
      ...(strategy.parameters.optimize_apply_policy
        ? { optimize_apply_policy: strategy.parameters.optimize_apply_policy }
        : {}),
    }

    const payload: CreateStrategyRequest = {
      name: name.trim(),
      strategy_type: strategyType,
      parameters,
      ...(strategy.pool_address
        ? { pool_address: strategy.pool_address }
        : {}),
      auto_execute: autoExecute,
      dry_run: dryRun,
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

  if (isLoading || !id) {
    return <div className="text-center py-8 text-muted-foreground">Loading...</div>
  }

  if (isError || !strategy) {
    return <div className="text-center py-8">Strategy not found</div>
  }

  return (
    <TooltipProvider delayDuration={200}>
      <div className="space-y-6">
        <div className="flex items-center gap-4">
          <Link to={`/strategies/${id}`}>
            <Button variant="ghost" size="icon">
              <ArrowLeft className="h-4 w-4" />
            </Button>
          </Link>
          <h1 className="text-3xl font-bold">Edit Strategy</h1>
        </div>

        <Card>
          <CardHeader>
            <CardTitle>Configuration</CardTitle>
          </CardHeader>
          <CardContent>
            <form className="space-y-4" onSubmit={handleSubmit} noValidate>
              <div>
                <FieldLabel htmlFor="edit-strategy-name" label="Name" tooltip={TOOLTIPS.name} />
                <input
                  id="edit-strategy-name"
                  className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  required
                />
              </div>

              <p className="text-xs text-muted-foreground rounded-md border border-border bg-muted/20 px-3 py-2">
                Pool is chosen when you{' '}
                <Link to="/positions/new" className="text-primary underline underline-offset-2">
                  open a position
                </Link>
                ; linked position PDAs are kept on save.
              </p>

              <div>
                <FieldLabel
                  htmlFor="edit-strategy-type"
                  label="Strategy Type"
                  tooltip={TOOLTIPS.strategyType}
                />
                <select
                  id="edit-strategy-type"
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
                    htmlFor="edit-range-width"
                    label={
                      enabled.rangeWidth ? 'Range Width % (required)' : 'Range Width % (n/a for this type)'
                    }
                    tooltip={TOOLTIPS.rangeWidth}
                  />
                  <input
                    id="edit-range-width"
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
                    placeholder={enabled.rangeWidth ? 'e.g. 1.0' : '—'}
                  />
                </div>
                <div>
                  <FieldLabel htmlFor="edit-max-il" label="Max IL % (optional)" tooltip={TOOLTIPS.maxIl} />
                  <input
                    id="edit-max-il"
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
                    htmlFor="edit-rebalance-threshold"
                    label={
                      strategyType === 'il_limit'
                        ? 'IL rebalance threshold % (optional)'
                        : 'Rebalance threshold % (optional)'
                    }
                    tooltip={rebalanceThresholdTooltip}
                  />
                  <input
                    id="edit-rebalance-threshold"
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
                    htmlFor="edit-min-interval"
                    label={`${minIntervalLabel} (optional)`}
                    tooltip={minIntervalTooltip}
                  />
                  <input
                    id="edit-min-interval"
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
                <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:gap-8">
                  <div className="flex items-center gap-2">
                    <input
                      id="edit-dry-run"
                      type="checkbox"
                      checked={dryRun}
                      onChange={(e) => setDryRun(e.target.checked)}
                      className="rounded border-input"
                    />
                    <FieldLabel htmlFor="edit-dry-run" label="Dry run" tooltip={TOOLTIPS.dryRun} />
                  </div>
                  <div className="flex items-center gap-2">
                    <input
                      id="edit-auto-exec"
                      type="checkbox"
                      checked={autoExecute}
                      onChange={(e) => setAutoExecute(e.target.checked)}
                      className="rounded border-input"
                    />
                    <FieldLabel
                      htmlFor="edit-auto-exec"
                      label="Auto-execute"
                      tooltip={TOOLTIPS.autoExecute}
                    />
                  </div>
                </div>
                {dryRun && autoExecute && (
                  <p className="text-xs text-muted-foreground pl-6 sm:pl-0">
                    Dry run is on — rebalance steps are simulated only; turn off Dry run for real
                    on-chain transactions (requires API wallet).
                  </p>
                )}
              </div>

              <div className="space-y-2 rounded-md border border-border bg-muted/20 px-3 py-3">
                <p className="text-sm font-medium text-foreground">Startup</p>
                <div className="flex items-start gap-2">
                  <input
                    id="edit-auto-start"
                    type="checkbox"
                    checked={autoStart}
                    onChange={(e) => setAutoStart(e.target.checked)}
                    className="mt-0.5 rounded border-input"
                  />
                  <div className="flex-1">
                    <FieldLabel
                      htmlFor="edit-auto-start"
                      label="Auto-start on API boot"
                      tooltip={TOOLTIPS.autoStart}
                    />
                    <p className="text-xs text-muted-foreground">
                      Server: autostart is <strong>on</strong> if <code className="text-[11px]">CLMM_STRATEGY_AUTOSTART_ON_BOOT</code> is unset. Set it to <code className="text-[11px]">0</code> or <code className="text-[11px]">false</code> to disable boot autostart globally.
                    </p>
                  </div>
                </div>
              </div>

              <div className="space-y-2 rounded-md border border-border bg-muted/20 px-3 py-3">
                <p className="text-sm font-medium text-foreground">Semantyka rebalance</p>
                <div className="grid gap-3 md:grid-cols-2">
                  <div className="flex items-start gap-2">
                    <input
                      id="edit-rebalance-on-exit"
                      type="checkbox"
                      checked={rebalanceOnRangeExitImmediately}
                      onChange={(e) => setRebalanceOnRangeExitImmediately(e.target.checked)}
                      className="mt-0.5 rounded border-input"
                    />
                    <div className="flex-1">
                      <FieldLabel
                        htmlFor="edit-rebalance-on-exit"
                        label="Rebalance immediately on range-exit (OOR)"
                        tooltip={TOOLTIPS.rebalanceOnRangeExitImmediately}
                      />
                    </div>
                  </div>

                  <div className="flex items-start gap-2">
                    <input
                      id="edit-periodic-requires-oor"
                      type="checkbox"
                      checked={periodicRequiresOutOfRange}
                      onChange={(e) => setPeriodicRequiresOutOfRange(e.target.checked)}
                      className="mt-0.5 rounded border-input"
                      disabled={strategyType !== 'periodic'}
                    />
                    <div className={cn('flex-1', strategyType !== 'periodic' && 'opacity-60')}>
                      <FieldLabel
                        htmlFor="edit-periodic-requires-oor"
                        label="Periodic only when OOR"
                        tooltip={TOOLTIPS.periodicRequiresOor}
                      />
                    </div>
                  </div>
                </div>
              </div>

              {mutation.isError && (
                <p className="text-sm text-destructive" role="alert">
                  {(mutation.error as Error)?.message ?? 'Save failed.'}
                </p>
              )}

              <div className="flex justify-end gap-2 pt-2">
                <Link to={`/strategies/${id}`}>
                  <Button variant="outline" type="button">
                    Cancel
                  </Button>
                </Link>
                <Button type="submit" disabled={mutation.isPending}>
                  {mutation.isPending ? 'Saving...' : 'Save changes'}
                </Button>
              </div>
            </form>
          </CardContent>
        </Card>
      </div>
    </TooltipProvider>
  )
}
