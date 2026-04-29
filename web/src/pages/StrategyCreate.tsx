import { useEffect, useMemo, useState } from 'react'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { useNavigate, Link } from 'react-router-dom'
import { ArrowLeft } from 'lucide-react'
import { Card, CardHeader, CardTitle, CardContent } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { ErrorBanner } from '@/components/ui/error-banner'
import { TooltipProvider } from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'
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
  const { locale } = useI18n()
  const L = (pl: string, en: string) => (locale === 'pl' ? pl : en)
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const { toast } = useToast()

  const [name, setName] = useState('')
  const [description, setDescription] = useState('')
  const [strategyType, setStrategyType] = useState<StrategyType>('static_range')
  const [rebalanceThresholdPct, setRebalanceThresholdPct] = useState<number | ''>('')
  const [maxIlPct, setMaxIlPct] = useState<number | ''>('')
  const [minRebalanceIntervalMinutes, setMinRebalanceIntervalMinutes] = useState<number | ''>('')
  const [retouchOffsetPct, setRetouchOffsetPct] = useState<number | ''>('')
  const [candleMinutes, setCandleMinutes] = useState<number | ''>('')
  const [bollingerWindow, setBollingerWindow] = useState<number | ''>(20)
  const [bollingerK, setBollingerK] = useState<number | ''>(2)
  const [rangeWidthPct, setRangeWidthPct] = useState<number | ''>('')
  const [dryRun, setDryRun] = useState(false)
  const [autoExecute, setAutoExecute] = useState(false)
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

    // Periodic-only toggle is ignored unless the type is periodic.
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
    if (strategyType === 'periodic' && minRebalanceIntervalMinutes === 0) {
      toast({
        title: 'Invalid interval for Periodic',
        description:
          'For Periodic strategy, interval must be at least 1 minute or left empty.',
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
        minRebalanceIntervalMinutes,
        retouchOffsetPct,
        candleMinutes,
        bollingerWindow,
        bollingerK,
        periodicRequiresOutOfRange,
        rebalanceOnRangeExitImmediately,
        autoStart,
      }),
      // Zawsze wysyłaj pole — starsze API wymagały `pool_address` w body; pusty = brak puli (pool przy Open Position).
      pool_address: '',
      auto_execute: autoExecute,
      dry_run: dryRun,
    }

    mutation.mutate(payload)
  }

  const strategyBlurb = STRATEGY_COPY[strategyType]

  const minIntervalLabel = useMemo(() => {
    if (strategyType === 'periodic') {
      return 'Rebalance co N minut'
    }
    return 'Min. rebalance spacing (min)'
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

  return (
    <TooltipProvider delayDuration={200}>
      <div className="space-y-6">
        <div className="flex items-center gap-4">
          <Link to="/strategies">
            <Button variant="ghost" size="icon">
              <ArrowLeft className="h-4 w-4" />
            </Button>
          </Link>
          <h1 className="text-3xl font-bold">{L('Utwórz strategię', 'Create Strategy')}</h1>
        </div>

        <Card>
          <CardHeader>
            <CardTitle>{L('Konfiguracja', 'Configuration')}</CardTitle>
          </CardHeader>
          <CardContent>
            <form className="space-y-4" onSubmit={handleSubmit} noValidate>
              <div>
                <FieldLabel htmlFor="strategy-name" label={L('Nazwa', 'Name')} tooltip={TOOLTIPS.name} />
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
                <span className="text-foreground font-medium">{L('Tu nie trzeba wybierać puli', 'No pool needed here')}</span> — {L('pula jest wybierana podczas', 'pool is chosen when you')}{' '}
                <Link to="/positions/new" className="text-primary underline underline-offset-2">
                  {L('otwierania pozycji', 'open a position')}
                </Link>
                {L('; tam podpinasz tę strategię do pozycji.', '; there you attach this strategy to that position.')}
              </p>

              <div>
                <FieldLabel
                  htmlFor="strategy-type"
                  label={L('Typ strategii', 'Strategy Type')}
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
                  <option value="bollinger">Bollinger</option>
                  <option value="il_limit">IL Limit</option>
                  <option value="oor_recenter">OOR recenter</option>
                  <option value="retouch_shift">Retouch shift</option>
                  <option value="last_candle">Last candle</option>
                  <option value="last_candle_periodic">Last candle (periodic)</option>
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
                    htmlFor="range-width"
                    label={L('Szerokość zakresu % (wymagane)', 'Range Width % (required)')}
                    tooltip={TOOLTIPS.rangeWidth}
                  />
                  <input
                    id="range-width"
                    type="number"
                    step="0.1"
                    min={0.01}
                    max={100}
                    required
                    className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                    value={rangeWidthPct}
                    onChange={(e) => setRangeWidthPct(readOptionalNumber(e.target.value))}
                    placeholder={L('np. 1.0 dla pasma ~±0.5%', 'e.g. 1.0 for ~±0.5% price band')}
                  />
                </div>
                ) : null}
                {enabled.maxIl ? (
                <div>
                  <FieldLabel
                    htmlFor="max-il"
                    label={L('Max IL % (opcjonalnie)', 'Max IL % (optional)')}
                    tooltip={TOOLTIPS.maxIl}
                  />
                  <input
                    id="max-il"
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
                      htmlFor="retouch-offset-pct"
                      label={L('Retouch offset % (opcjonalnie)', 'Retouch offset % (optional)')}
                      tooltip={TOOLTIPS.retouchOffsetPct}
                    />
                    <input
                      id="retouch-offset-pct"
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
                      htmlFor="candle-minutes"
                      label={L('Interwał świecy (min, opcjonalnie)', 'Candle interval (min, optional)')}
                      tooltip={TOOLTIPS.candleSeconds}
                    />
                    <input
                      id="candle-minutes"
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
                        htmlFor="bollinger-window"
                        label={L('Bollinger window (punkty)', 'Bollinger window (points)')}
                        tooltip={TOOLTIPS.bollingerWindow}
                      />
                      <input
                        id="bollinger-window"
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
                        htmlFor="bollinger-k"
                        label="Bollinger k"
                        tooltip={TOOLTIPS.bollingerK}
                      />
                      <input
                        id="bollinger-k"
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
                    htmlFor="rebalance-threshold"
                    label={
                      strategyType === 'il_limit'
                        ? L('Próg IL rebalance % (opcjonalnie)', 'IL rebalance threshold % (optional)')
                        : L('Próg rebalance % (opcjonalnie)', 'Rebalance threshold % (optional)')
                    }
                    tooltip={rebalanceThresholdTooltip}
                  />
                  <input
                    id="rebalance-threshold"
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
                    htmlFor="min-interval"
                    label={`${minIntervalLabel} (${L('opcjonalnie', 'optional')})`}
                    tooltip={minIntervalTooltip}
                  />
                  <input
                    id="min-interval"
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
                <p className="text-sm font-medium text-foreground">{L('Semantyka rebalance', 'Rebalance semantics')}</p>
                <div className="grid gap-3 md:grid-cols-2">
                  {strategyType !== 'periodic' && strategyType !== 'last_candle_periodic' && (
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
                          label={L('Rebalance natychmiast po wyjściu z zakresu (OOR)', 'Rebalance immediately on range-exit (OOR)')}
                          tooltip={TOOLTIPS.rebalanceOnRangeExitImmediately}
                        />
                        <p className="text-xs text-muted-foreground">
                          {L('Domyślnie: on (stare zachowanie).', 'Default: on (old behavior).')}
                        </p>
                      </div>
                    </div>
                  )}

                  {strategyType === 'periodic' ? (
                  <div className="flex items-start gap-2">
                    <input
                      id="create-periodic-requires-oor"
                      type="checkbox"
                      checked={periodicRequiresOutOfRange}
                      onChange={(e) => setPeriodicRequiresOutOfRange(e.target.checked)}
                      className="mt-0.5 rounded border-input"
                    />
                    <div className="flex-1">
                      <FieldLabel
                        htmlFor="create-periodic-requires-oor"
                        label="Wymagaj OOR w chwili wyzwolenia"
                        tooltip={TOOLTIPS.periodicRequiresOor}
                      />
                      <p className="text-xs text-muted-foreground">
                        {L('Działa tylko dla strategii Periodic.', 'Enabled only for Periodic strategy type.')}
                      </p>
                    </div>
                  </div>
                  ) : null}
                </div>
              </div>

              <div className="space-y-2 rounded-md border border-border bg-muted/20 px-3 py-3">
                <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:gap-8">
                  <div className="flex items-center gap-2">
                    <input
                      id="create-dry-run"
                      type="checkbox"
                      checked={dryRun}
                      onChange={(e) => setDryRun(e.target.checked)}
                      className="rounded border-input"
                    />
                    <FieldLabel htmlFor="create-dry-run" label={L('Dry run', 'Dry run')} tooltip={TOOLTIPS.dryRun} />
                  </div>
                  <div className="flex items-center gap-2">
                    <input
                      id="create-auto-exec"
                      type="checkbox"
                      checked={autoExecute}
                      onChange={(e) => setAutoExecute(e.target.checked)}
                      className="rounded border-input"
                    />
                    <FieldLabel
                      htmlFor="create-auto-exec"
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
                    id="create-auto-start"
                    type="checkbox"
                    checked={autoStart}
                    onChange={(e) => setAutoStart(e.target.checked)}
                    className="mt-0.5 rounded border-input"
                  />
                  <div className="flex-1">
                    <FieldLabel
                      htmlFor="create-auto-start"
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

              {mutation.isError && (
                <ErrorBanner role="alert">
                  {(mutation.error as Error)?.message ?? L('Żądanie nieudane.', 'Request failed.')}
                </ErrorBanner>
              )}

              <div className="flex justify-end gap-2 pt-2">
                <Link to="/strategies">
                  <Button variant="outline" type="button">
                    {L('Anuluj', 'Cancel')}
                  </Button>
                </Link>
                <Button type="submit" disabled={mutation.isPending}>
                  {mutation.isPending ? L('Tworzenie...', 'Creating...') : L('Utwórz strategię', 'Create Strategy')}
                </Button>
              </div>
            </form>
          </CardContent>
        </Card>
      </div>
    </TooltipProvider>
  )
}
