import { useMemo, useState } from 'react'
import { Link } from 'react-router-dom'
import { Plus, Search, X } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { applyStrategyToArm, createExperimentArm, type ExperimentArm } from '@/lib/experimentArm'
import type { Strategy } from '@/lib/api'
import { useI18n } from '@/lib/i18n'
import { shortenAddress } from '@/lib/utils'

type Props = {
  strategies: Strategy[]
  insertLabel: string
  onSelectArm: (arm: ExperimentArm) => void
  onCancel: () => void
}

export default function ExperimentStrategyPicker({
  strategies,
  insertLabel,
  onSelectArm,
  onCancel,
}: Props) {
  const { t, locale } = useI18n()
  const pl = locale === 'pl'
  const [query, setQuery] = useState('')

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase()
    if (!q) return strategies
    return strategies.filter(
      (s) =>
        s.name.toLowerCase().includes(q) ||
        s.strategy_type.toLowerCase().includes(q),
    )
  }, [strategies, query])

  function pickExisting(strategy: Strategy) {
    const base = createExperimentArm(1)
    onSelectArm(applyStrategyToArm({ ...base, label: strategy.name }, strategy))
  }

  return (
    <Card className="border-0 sm:border-primary/30 shadow-none sm:shadow-sm rounded-none sm:rounded-lg">
      <CardHeader className="pb-3 sticky top-0 bg-card z-10 border-b sm:border-b-0">
        <div className="flex items-start justify-between gap-3">
          <div>
            <CardTitle className="text-lg">{t('experiment.pickStrategyTitle')}</CardTitle>
            <CardDescription>{insertLabel}</CardDescription>
          </div>
          <Button type="button" variant="ghost" size="icon" onClick={onCancel} aria-label={t('experiment.cancelPick')}>
            <X className="h-4 w-4" />
          </Button>
        </div>
      </CardHeader>
      <CardContent className="space-y-3 pt-4">
        {strategies.length === 0 ? (
          <div className="rounded-xl border border-dashed border-border bg-muted/10 px-4 py-8 text-center space-y-3">
            <p className="text-sm text-muted-foreground">{t('experiment.noSavedStrategiesPick')}</p>
            <Button asChild size="sm" className="gap-1.5">
              <Link to="/strategies/new" onClick={onCancel}>
                <Plus className="h-4 w-4" />
                {t('experiment.createStrategyFirst')}
              </Link>
            </Button>
          </div>
        ) : (
          <>
            <div className="relative">
              <Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground pointer-events-none" />
              <input
                type="search"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder={t('experiment.searchStrategies')}
                className="w-full rounded-lg border border-input bg-background pl-9 pr-3 py-2 text-sm"
                autoFocus
              />
            </div>

            <div className="grid gap-2 max-h-[min(52vh,420px)] overflow-y-auto pr-1">
              {filtered.length === 0 ? (
                <p className="text-sm text-muted-foreground py-6 text-center">{t('experiment.noStrategyMatch')}</p>
              ) : (
                filtered.map((s) => (
                  <button
                    key={s.id}
                    type="button"
                    className="rounded-lg border border-border bg-background px-3 py-3 text-left hover:border-primary/40 hover:bg-accent/40 transition-colors"
                    onClick={() => pickExisting(s)}
                  >
                    <div className="font-medium truncate">{s.name}</div>
                    <div className="text-xs text-muted-foreground capitalize mt-1">
                      {s.strategy_type.replace(/_/g, ' ')}
                      {s.pool_address ? ` · ${shortenAddress(s.pool_address, 4)}` : ''}
                      {s.running ? ` · ${pl ? 'działa' : 'running'}` : ''}
                    </div>
                  </button>
                ))
              )}
            </div>
          </>
        )}
      </CardContent>
    </Card>
  )
}
