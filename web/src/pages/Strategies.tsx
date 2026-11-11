import { useMemo } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { Link, useNavigate } from 'react-router-dom'
import { Plus, Play, Square, RefreshCw, Pencil, FlaskConical } from 'lucide-react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import {
  applyBacktestAutoTuneToStrategy,
  getBacktestAutoTuneStatus,
  getHealth,
  getStrategies,
  startStrategy,
  stopStrategy,
} from '@/lib/api'
import { strategyExecBadge, summarizeStrategyFleet } from '@/lib/botControlDisplay'
import { useI18n } from '@/lib/i18n'

export default function Strategies() {
  const { locale, t } = useI18n()
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const { data, isLoading, refetch } = useQuery({
    queryKey: ['strategies'],
    queryFn: getStrategies,
  })
  const healthQ = useQuery({
    queryKey: ['health'],
    queryFn: getHealth,
    refetchInterval: 30_000,
    retry: 1,
  })
  const autoTuneQ = useQuery({
    queryKey: ['backtests-auto-tune-status'],
    queryFn: getBacktestAutoTuneStatus,
    refetchInterval: 15_000,
  })

  const startMutation = useMutation({
    mutationFn: startStrategy,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['strategies'] }),
  })

  const stopMutation = useMutation({
    mutationFn: stopStrategy,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['strategies'] }),
  })
  const applyAutoTuneMutation = useMutation({
    mutationFn: applyBacktestAutoTuneToStrategy,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['strategies'] }),
  })

  const strategies = data?.strategies || []
  const fleet = useMemo(() => summarizeStrategyFleet(strategies), [strategies])
  const apiDryRun = healthQ.data?.dry_run

  return (
    <div className="space-y-6">
      <p className="text-xs text-muted-foreground">
        {locale === 'pl'
          ? 'Lista strategii pochodzi z API — utwórz wpis lub zobacz pełne wyjaśnienie źródeł danych na stronie Dashboard.'
          : 'Strategy list comes from API — create an entry or see full data-source explanation on the Dashboard page.'}
      </p>

      <Card>
        <CardHeader className="pb-2">
          <CardTitle className="text-base">
            {locale === 'pl' ? 'Bot Control (BC1–BC2)' : 'Bot Control (BC1–BC2)'}
          </CardTitle>
        </CardHeader>
        <CardContent className="text-sm flex flex-wrap gap-x-6 gap-y-2">
          <div>
            <span className="text-muted-foreground">
              {locale === 'pl' ? 'Uruchomione' : 'Running'}:{' '}
            </span>
            <span className="font-medium">
              {fleet.running}/{fleet.total}
            </span>
          </div>
          <div>
            <span className="text-muted-foreground">
              {locale === 'pl' ? 'Live armed (running)' : 'Live armed (running)'}:{' '}
            </span>
            <span className="font-medium">{fleet.liveArmedRunning}</span>
          </div>
          <div>
            <span className="text-muted-foreground">API dry_run: </span>
            <span className="font-medium">
              {apiDryRun == null ? '—' : apiDryRun ? 'true' : 'false'}
            </span>
          </div>
          <Link to="/settings" className="text-primary hover:underline text-xs self-center">
            {locale === 'pl' ? '→ Settings (BC3)' : '→ Settings (BC3)'}
          </Link>
        </CardContent>
      </Card>

      <div className="flex items-center justify-between">
        <h1 className="text-3xl font-bold">{locale === 'pl' ? 'Strategie' : 'Strategies'}</h1>
        <div className="flex gap-2">
          <Button variant="outline" size="sm" onClick={() => refetch()}>
            <RefreshCw className="h-4 w-4 mr-2" />
            {locale === 'pl' ? 'Odśwież' : 'Refresh'}
          </Button>
          <Button variant="outline" size="sm" onClick={() => navigate('/experiments/new')}>
            <FlaskConical className="h-4 w-4 mr-2" />
            {t('positions.newExperiment')}
          </Button>
          <Button size="sm" onClick={() => navigate('/strategies/new')}>
            <Plus className="h-4 w-4 mr-2" />
            {locale === 'pl' ? 'Utwórz strategię' : 'Create Strategy'}
          </Button>
        </div>
      </div>
      <div className="text-xs text-muted-foreground">
        Auto-Tune winner:{' '}
        {autoTuneQ.data?.latest_winner
          ? `${autoTuneQ.data.latest_winner.strategy} (${autoTuneQ.data.latest_winner.pool_label}, ${autoTuneQ.data.latest_winner.window_hours}h)`
          : locale === 'pl'
            ? 'brak (uruchom Auto-Tune w Backtests)'
            : 'none (run Auto-Tune in Backtests)'}
      </div>

      {isLoading ? (
        <div className="text-center py-8 text-muted-foreground">{locale === 'pl' ? 'Ładowanie...' : 'Loading...'}</div>
      ) : strategies.length === 0 ? (
        <Card>
          <CardContent className="py-8 text-center text-muted-foreground">
            {locale === 'pl'
              ? 'Nie znaleziono strategii. Utwórz pierwszą strategię, aby automatyzować pozycje LP.'
              : 'No strategies found. Create your first strategy to automate your LP positions.'}
          </CardContent>
        </Card>
      ) : (
        <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
          {strategies.map((strategy) => (
            <Card key={strategy.id}>
              <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
                <CardTitle className="text-lg">{strategy.name}</CardTitle>
                <span className={`px-2 py-1 rounded-full text-xs font-medium ${
                  strategy.running 
                    ? 'bg-green-500/10 text-green-500' 
                    : 'bg-muted text-muted-foreground'
                }`}>
                  {strategy.running
                    ? locale === 'pl'
                      ? 'Działa'
                      : 'Running'
                    : locale === 'pl'
                      ? 'Zatrzymana'
                      : 'Stopped'}
                </span>
              </CardHeader>
              <CardContent className="space-y-4">
                <div className="text-sm text-muted-foreground">
                  {strategy.description || (locale === 'pl' ? 'Brak opisu' : 'No description')}
                </div>
                <div className="flex items-center gap-2 text-sm">
                  <span className="text-muted-foreground">{locale === 'pl' ? 'Typ:' : 'Type:'}</span>
                  <span className="capitalize">{strategy.strategy_type.replace('_', ' ')}</span>
                </div>
                {(() => {
                  const badge = strategyExecBadge(strategy)
                  return (
                    <div className="flex flex-wrap gap-1.5 text-xs">
                      <span className="inline-flex rounded-full border px-2 py-0.5 text-muted-foreground">
                        dry_run: {badge.dryRun ? 'yes' : 'no'}
                      </span>
                      <span className="inline-flex rounded-full border px-2 py-0.5 text-muted-foreground">
                        auto_execute: {badge.autoExecute ? 'yes' : 'no'}
                      </span>
                      {badge.liveArmed ? (
                        <span className="inline-flex rounded-full border border-rose-300 bg-rose-50 text-rose-800 px-2 py-0.5">
                          live armed
                        </span>
                      ) : null}
                    </div>
                  )
                })()}
                {strategy.parameters?.auto_start ? (
                  <div className="text-xs">
                    <span className="inline-flex items-center rounded-full border border-border px-2 py-0.5 text-muted-foreground">
                      {locale === 'pl' ? 'auto-start przy starcie' : 'auto-start on boot'}
                    </span>
                  </div>
                ) : null}
                <div className="flex gap-2">
                  <Button
                    variant="outline"
                    size="sm"
                    className="flex-1"
                    disabled={!autoTuneQ.data?.latest_winner || applyAutoTuneMutation.isPending}
                    onClick={() => applyAutoTuneMutation.mutate(strategy.id)}
                  >
                    {locale === 'pl' ? 'Zastosuj Auto-Tune' : 'Apply Auto-Tune'}
                  </Button>
                  <Link to={`/strategies/${strategy.id}/edit`} className="flex-1">
                    <Button variant="secondary" size="sm" className="w-full">
                      <Pencil className="h-4 w-4 mr-2" />
                      {locale === 'pl' ? 'Edytuj' : 'Edit'}
                    </Button>
                  </Link>
                  <Link to={`/strategies/${strategy.id}`} className="flex-1">
                    <Button variant="outline" size="sm" className="w-full">
                      {locale === 'pl' ? 'Zobacz szczegóły' : 'View Details'}
                    </Button>
                  </Link>
                  {strategy.running ? (
                    <Button 
                      variant="destructive" 
                      size="sm"
                      onClick={() => stopMutation.mutate(strategy.id)}
                      disabled={stopMutation.isPending}
                    >
                      <Square className="h-4 w-4" />
                    </Button>
                  ) : (
                    <Button 
                      variant="default" 
                      size="sm"
                      onClick={() => startMutation.mutate(strategy.id)}
                      disabled={startMutation.isPending}
                    >
                      <Play className="h-4 w-4" />
                    </Button>
                  )}
                </div>
              </CardContent>
            </Card>
          ))}
        </div>
      )}
    </div>
  )
}
