import { useEffect, useMemo, useRef, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useNavigate, Link, useParams } from 'react-router-dom'
import { ArrowLeft } from 'lucide-react'
import { Card, CardHeader, CardTitle, CardContent } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { ErrorBanner } from '@/components/ui/error-banner'
import { TooltipProvider } from '@/components/ui/tooltip'
import { useToast } from '@/hooks/use-toast'
import { useI18n } from '@/lib/i18n'
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
  const { locale } = useI18n()
  const L = (pl: string, en: string) => (locale === 'pl' ? pl : en)
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
  const [minRebalanceIntervalMinutes, setMinRebalanceIntervalMinutes] = useState<number | ''>('')
  const [retouchOffsetPct, setRetouchOffsetPct] = useState<number | ''>('')
  const [candleMinutes, setCandleMinutes] = useState<number | ''>('')
  const [bollingerWindow, setBollingerWindow] = useState<number | ''>(20)
  const [bollingerK, setBollingerK] = useState<number | ''>(2)
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
    setMinRebalanceIntervalMinutes(
      numOrEmpty(
        p.min_rebalance_interval_minutes ??
          (typeof p.min_rebalance_interval_hours === 'number'
            ? p.min_rebalance_interval_hours * 60
            : undefined),
      ),
    )
    setCandleMinutes(
      numOrEmpty(
        typeof p.candle_seconds === 'number' ? p.candle_seconds / 60 : undefined,
      ),
    )
    setRetouchOffsetPct(numOrEmpty(p.retouch_offset_pct))
    setBollingerWindow(numOrEmpty(p.bollinger_window))
    setBollingerK(numOrEmpty(p.bollinger_k))
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
        setMinRebalanceIntervalMinutes('')
        break
      case 'periodic':
        setMaxIlPct('')
        setRebalanceThresholdPct('')
        break
      case 'threshold':
      case 'bollinger':
      case 'oor_recenter':
      case 'retouch_shift':
      case 'last_candle':
      case 'last_candle_periodic':
        setMaxIlPct('')
        break
      default:
        break
    }

    if (strategyType !== 'periodic') {
      setPeriodicRequiresOutOfRange(false)
    }
    if (strategyType !== 'retouch_shift') {
      setRetouchOffsetPct('')
    }
    if (strategyType !== 'bollinger') {
      setBollingerWindow(20)
      setBollingerK(2)
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
        title: L('Nie udało się zapisać strategii', 'Could not save strategy'),
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
        title: L('Nazwa wymagana', 'Name required'),
        description: L('Podaj nazwę strategii przed zapisem.', 'Enter a strategy name before saving.'),
        variant: 'destructive',
      })
      return
    }
    if (!isRangeWidthSatisfied(strategyType, rangeWidthPct)) {
      toast({
        title: L('Szerokość zakresu wymagana', 'Range width required'),
        description: L('Ustaw Range Width % (większe od 0, maks. 100) dla tego typu strategii.', 'Set Range Width % (greater than 0, at most 100) for this strategy type.'),
        variant: 'destructive',
      })
      return
    }
    if (strategyType === 'periodic' && minRebalanceIntervalMinutes === 0) {
      toast({
        title: L('Nieprawidłowy interwał dla Periodic', 'Invalid interval for Periodic'),
        description:
          L('Dla strategii Periodic interwał musi mieć co najmniej 1 minutę lub być pusty.', 'For Periodic strategy, interval must be at least 1 minute or left empty.'),
        variant: 'destructive',
      })
      return
    }

    const built = buildParameters(strategyType, {
      rangeWidthPct,
      maxIlPct,
      rebalanceThresholdPct,
      minRebalanceIntervalMinutes,
      retouchOffsetPct,
      candleMinutes,
      bollingerWindow,
      bollingerK,
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
      return L('Rebalance co N minut', 'Rebalance every N minutes')
    }
    return L('Min. odstęp rebalance (min)', 'Min. rebalance spacing (min)')
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

  if (isLoading || !id) {
    return <div className="text-center py-8 text-muted-foreground">{L('Ładowanie...', 'Loading...')}</div>
  }

  if (isError || !strategy) {
    return <div className="text-center py-8">{L('Nie znaleziono strategii', 'Strategy not found')}</div>
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
          <h1 className="text-3xl font-bold">{L('Edytuj strategię', 'Edit Strategy')}</h1>
        </div>

        <Card>
          <CardHeader>
            <CardTitle>{L('Konfiguracja', 'Configuration')}</CardTitle>
          </CardHeader>
          <CardContent>
            <form className="space-y-4" onSubmit={handleSubmit} noValidate>
              <div>
                <FieldLabel htmlFor="edit-strategy-name" label={L('Nazwa', 'Name')} tooltip={TOOLTIPS.name} />
                <input
                  id="edit-strategy-name"
                  className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  required
                />
              </div>

              <p className="text-xs text-muted-foreground rounded-md border border-border bg-muted/20 px-3 py-2">
                {L('Pula jest wybierana podczas', 'Pool is chosen when you')}{' '}
                <Link to="/positions/new" className="text-primary underline underline-offset-2">
                  {L('otwierania pozycji', 'open a position')}
                </Link>
                {L('; podpięte PDA pozycji są zachowane przy zapisie.', '; linked position PDAs are kept on save.')}
              </p>

              <div>
                <FieldLabel
                  htmlFor="edit-strategy-type"
                  label={L('Typ strategii', 'Strategy Type')}
                  tooltip={TOOLTIPS.strategyType}
                />
                <select
                  id="edit-strategy-type"
                  className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                  value={strategyType}
                  onChange={(e) => setStrategyType(e.target.value as StrategyType)}
                >
                  <option value="static_range">{L('Statyczna', 'Static')}</option>
                  <option value="periodic">{L('Okresowa', 'Periodic')}</option>
                  <option value="threshold">Threshold</option>
                  <option value="bollinger">Bollinger</option>
                  <option value="il_limit">IL Limit</option>
                  <option value="oor_recenter">{L('OOR recenter', 'OOR recenter')}</option>
                  <option value="retouch_shift">{L('Retouch shift', 'Retouch shift')}</option>
                  <option value="last_candle">{L('Last candle', 'Last candle')}</option>
                  <option value="last_candle_periodic">{L('Last candle (periodic)', 'Last candle (periodic)')}</option>
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
                {enabled.rangeWidth ? (
                <div>
                  <FieldLabel
                    htmlFor="edit-range-width"
                    label={L('Range Width % (wymagane)', 'Range Width % (required)')}
                    tooltip={TOOLTIPS.rangeWidth}
                  />
                  <input
                    id="edit-range-width"
                    type="number"
                    step="0.1"
                    min={0.01}
                    max={100}
                    required
                    className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                    value={rangeWidthPct}
                    onChange={(e) => setRangeWidthPct(readOptionalNumber(e.target.value))}
                    placeholder={L('np. 1.0', 'e.g. 1.0')}
                  />
                </div>
                ) : null}
                {enabled.maxIl ? (
                <div>
                  <FieldLabel htmlFor="edit-max-il" label={L('Max IL % (opcjonalnie)', 'Max IL % (optional)')} tooltip={TOOLTIPS.maxIl} />
                  <input
                    id="edit-max-il"
                    type="number"
                    step="0.1"
                    className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                    value={maxIlPct}
                    onChange={(e) => setMaxIlPct(readOptionalNumber(e.target.value))}
                    placeholder={L('np. 2.0', 'e.g. 2.0')}
                  />
                </div>
                ) : null}
              </div>

              <div className="grid gap-4 md:grid-cols-2">
                {strategyType === 'retouch_shift' ? (
                  <div>
                    <FieldLabel
                      htmlFor="edit-retouch-offset-pct"
                      label={L('Retouch offset % (opcjonalnie)', 'Retouch offset % (optional)')}
                      tooltip={TOOLTIPS.retouchOffsetPct}
                    />
                    <input
                      id="edit-retouch-offset-pct"
                      type="number"
                      step="0.01"
                      className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                      value={retouchOffsetPct}
                      onChange={(e) => setRetouchOffsetPct(readOptionalNumber(e.target.value))}
                      placeholder={L('np. 0.1 lub -0.1', 'e.g. 0.1 or -0.1')}
                    />
                  </div>
                ) : null}
                {strategyType === 'last_candle' || strategyType === 'last_candle_periodic' ? (
                  <div>
                    <FieldLabel
                      htmlFor="edit-candle-minutes"
                      label={L('Interwał świecy (min, opcjonalnie)', 'Candle interval (min, optional)')}
                      tooltip={TOOLTIPS.candleSeconds}
                    />
                    <input
                      id="edit-candle-minutes"
                      type="number"
                      step="1"
                      min={1}
                      className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                      value={candleMinutes}
                      onChange={(e) => setCandleMinutes(readOptionalNumber(e.target.value))}
                      placeholder={L('np. 60', 'e.g. 60')}
                    />
                    <p className="mt-1 text-xs text-muted-foreground">{L('Przykłady: 15, 30, 60.', 'Examples: 15, 30, 60.')}</p>
                  </div>
                ) : null}
                {strategyType === 'bollinger' ? (
                  <>
                    <div>
                      <FieldLabel
                        htmlFor="edit-bollinger-window"
                        label={L('Bollinger window (punkty)', 'Bollinger window (points)')}
                        tooltip={TOOLTIPS.bollingerWindow}
                      />
                      <input
                        id="edit-bollinger-window"
                        type="number"
                        step="1"
                        min={2}
                        className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                        value={bollingerWindow}
                        onChange={(e) => setBollingerWindow(readOptionalNumber(e.target.value))}
                        placeholder={L('np. 20', 'e.g. 20')}
                      />
                      <p className="mt-1 text-xs text-muted-foreground">
                        {L('Start: 20-30. Punkt = jedna próbka live z pętli strategii (nie snapshot 5m/10m).', 'Start: 20-30. One point = one live sample from strategy loop (not backtest 5m/10m snapshots).')}
                      </p>
                    </div>
                    <div>
                      <FieldLabel
                        htmlFor="edit-bollinger-k"
                        label="Bollinger k"
                        tooltip={TOOLTIPS.bollingerK}
                      />
                      <input
                        id="edit-bollinger-k"
                        type="number"
                        step="0.1"
                        min={0.1}
                        className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                        value={bollingerK}
                        onChange={(e) => setBollingerK(readOptionalNumber(e.target.value))}
                        placeholder={L('np. 2.0', 'e.g. 2.0')}
                      />
                      <p className="mt-1 text-xs text-muted-foreground">
                        {L('Start: 2.0 (agresywniej 1.5, spokojniej 2.5).', 'Start: 2.0 (more aggressive 1.5, more conservative 2.5).')}
                      </p>
                    </div>
                  </>
                ) : null}
                {enabled.rebalanceThreshold ? (
                <div>
                  <FieldLabel
                    htmlFor="edit-rebalance-threshold"
                    label={
                      strategyType === 'il_limit'
                        ? L('Próg IL rebalance % (opcjonalnie)', 'IL rebalance threshold % (optional)')
                        : L('Próg rebalance % (opcjonalnie)', 'Rebalance threshold % (optional)')
                    }
                    tooltip={rebalanceThresholdTooltip}
                  />
                  <input
                    id="edit-rebalance-threshold"
                    type="number"
                    step="0.1"
                    className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                    value={rebalanceThresholdPct}
                    onChange={(e) => setRebalanceThresholdPct(readOptionalNumber(e.target.value))}
                    placeholder={L('np. 5.0', 'e.g. 5.0')}
                  />
                </div>
                ) : null}
                {enabled.minInterval ? (
                <div>
                  <FieldLabel
                    htmlFor="edit-min-interval"
                    label={`${minIntervalLabel} (${L('opcjonalnie', 'optional')})`}
                    tooltip={minIntervalTooltip}
                  />
                  <input
                    id="edit-min-interval"
                    type="number"
                    step="1"
                    min={strategyType === 'periodic' ? 1 : 0}
                    className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                    value={minRebalanceIntervalMinutes}
                    onChange={(e) =>
                      setMinRebalanceIntervalMinutes(readOptionalNumber(e.target.value))
                    }
                    placeholder={L('np. 60', 'e.g. 60')}
                  />
                  <p className="mt-1 text-xs text-muted-foreground">{L('Przykłady: 15 = 15m, 60 = 1h, 240 = 4h.', 'Examples: 15 = 15m, 60 = 1h, 240 = 4h.')}</p>
                </div>
                ) : null}
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
                    <FieldLabel htmlFor="edit-dry-run" label={L('Dry run', 'Dry run')} tooltip={TOOLTIPS.dryRun} />
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
                      label={L('Auto-execute', 'Auto-execute')}
                      tooltip={TOOLTIPS.autoExecute}
                    />
                  </div>
                </div>
                {dryRun && autoExecute && (
                  <p className="text-xs text-muted-foreground pl-6 sm:pl-0">
                    {L(
                      'Dry run jest włączony — kroki rebalance są tylko symulowane; wyłącz Dry run dla realnych transakcji on-chain (wymaga walleta API).',
                      'Dry run is on — rebalance steps are simulated only; turn off Dry run for real on-chain transactions (requires API wallet).',
                    )}
                  </p>
                )}
              </div>

              <div className="space-y-2 rounded-md border border-border bg-muted/20 px-3 py-3">
                <p className="text-sm font-medium text-foreground">{L('Start', 'Startup')}</p>
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
                      label={L('Auto-start przy starcie API', 'Auto-start on API boot')}
                      tooltip={TOOLTIPS.autoStart}
                    />
                    <p className="text-xs text-muted-foreground">
                      {locale === 'pl'
                        ? <>Serwer: autostart jest <strong>włączony</strong>, jeśli <code className="text-[11px]">CLMM_STRATEGY_AUTOSTART_ON_BOOT</code> nie jest ustawione. Ustaw <code className="text-[11px]">0</code> lub <code className="text-[11px]">false</code>, aby globalnie wyłączyć autostart przy starcie.</>
                        : <>Server: autostart is <strong>on</strong> if <code className="text-[11px]">CLMM_STRATEGY_AUTOSTART_ON_BOOT</code> is unset. Set it to <code className="text-[11px]">0</code> or <code className="text-[11px]">false</code> to disable boot autostart globally.</>}
                    </p>
                  </div>
                </div>
              </div>

              <div className="space-y-2 rounded-md border border-border bg-muted/20 px-3 py-3">
                <p className="text-sm font-medium text-foreground">{L('Semantyka rebalance', 'Rebalance semantics')}</p>
                <div className="grid gap-3 md:grid-cols-2">
                  {strategyType !== 'periodic' && strategyType !== 'last_candle_periodic' && (
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
                          label={L('Rebalance natychmiast po wyjściu z zakresu (OOR)', 'Rebalance immediately on range-exit (OOR)')}
                          tooltip={TOOLTIPS.rebalanceOnRangeExitImmediately}
                        />
                      </div>
                    </div>
                  )}

                  {strategyType === 'periodic' ? (
                  <div className="flex items-start gap-2">
                    <input
                      id="edit-periodic-requires-oor"
                      type="checkbox"
                      checked={periodicRequiresOutOfRange}
                      onChange={(e) => setPeriodicRequiresOutOfRange(e.target.checked)}
                      className="mt-0.5 rounded border-input"
                    />
                    <div className="flex-1">
                      <FieldLabel
                        htmlFor="edit-periodic-requires-oor"
                        label="Wymagaj OOR w chwili wyzwolenia"
                        tooltip={TOOLTIPS.periodicRequiresOor}
                      />
                    </div>
                  </div>
                  ) : null}
                </div>
              </div>

              {mutation.isError && (
                <ErrorBanner role="alert">
                  {(mutation.error as Error)?.message ?? L('Zapis nieudany.', 'Save failed.')}
                </ErrorBanner>
              )}

              <div className="flex justify-end gap-2 pt-2">
                <Link to={`/strategies/${id}`}>
                  <Button variant="outline" type="button">
                    {L('Anuluj', 'Cancel')}
                  </Button>
                </Link>
                <Button type="submit" disabled={mutation.isPending}>
                  {mutation.isPending ? L('Zapisywanie...', 'Saving...') : L('Zapisz zmiany', 'Save changes')}
                </Button>
              </div>
            </form>
          </CardContent>
        </Card>
      </div>
    </TooltipProvider>
  )
}
