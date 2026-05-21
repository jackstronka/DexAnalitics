import { Rocket } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/lib/i18n'
import { formatUSD } from '@/lib/utils'

type Props = {
  strategyCount: number
  budgetUsd: number | ''
  capitalReady: boolean
  launchReady: boolean
  launching: boolean
  poolLabel: string | null
  blockerHint: string | null
  onLaunch: () => void
  onScrollToCapital: () => void
}

export default function ExperimentStickyFooter({
  strategyCount,
  budgetUsd,
  capitalReady,
  launchReady,
  launching,
  poolLabel,
  blockerHint,
  onLaunch,
  onScrollToCapital,
}: Props) {
  const { t } = useI18n()

  return (
    <div className="fixed bottom-0 inset-x-0 z-40 border-t border-border bg-background/95 backdrop-blur supports-[backdrop-filter]:bg-background/80">
      <div className="max-w-3xl mx-auto px-4 py-3 flex flex-wrap items-center gap-3 justify-between">
        <div className="text-sm min-w-0">
          <div className="font-medium truncate">
            {t('experiment.footerSummary')
              .replace('{n}', String(strategyCount))
              .replace('{budget}', budgetUsd === '' ? '—' : formatUSD(Number(budgetUsd)))}
          </div>
          {poolLabel ? (
            <div className="text-xs text-muted-foreground truncate">{poolLabel}</div>
          ) : null}
          {blockerHint && !launchReady ? (
            <div className="text-xs text-amber-600 dark:text-amber-400 truncate">{blockerHint}</div>
          ) : null}
        </div>
        <div className="flex gap-2 shrink-0">
          {!capitalReady ? (
            <Button type="button" variant="outline" size="sm" onClick={onScrollToCapital}>
              {t('experiment.footerSetupCapital')}
            </Button>
          ) : null}
          <Button
            type="button"
            size="sm"
            disabled={!launchReady || launching}
            onClick={onLaunch}
            className="gap-1.5"
          >
            <Rocket className="h-4 w-4" />
            {launching
              ? t('experiment.launching')
              : strategyCount === 1
                ? t('experiment.launchOne')
                : t('experiment.launchAll')}
          </Button>
        </div>
      </div>
    </div>
  )
}
