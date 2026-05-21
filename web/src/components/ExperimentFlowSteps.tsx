import { Check } from 'lucide-react'
import { useI18n } from '@/lib/i18n'

type StepId = 'strategies' | 'capital' | 'launch'

type Props = {
  strategiesCount: number
  strategiesReady: boolean
  capitalReady: boolean
  launchReady: boolean
}

const STEPS: { id: StepId; labelKey: string }[] = [
  { id: 'strategies', labelKey: 'experiment.flowStrategies' },
  { id: 'capital', labelKey: 'experiment.flowCapital' },
  { id: 'launch', labelKey: 'experiment.flowLaunch' },
]

function stepDone(id: StepId, props: Props): boolean {
  switch (id) {
    case 'strategies':
      return props.strategiesReady
    case 'capital':
      return props.capitalReady
    case 'launch':
      return props.launchReady
  }
}

export default function ExperimentFlowSteps(props: Props) {
  const { t } = useI18n()
  const activeIdx = (() => {
    if (props.strategiesCount === 0 || !props.strategiesReady) return 0
    if (!props.capitalReady) return 1
    return 2
  })()

  return (
    <nav
      className="flex flex-wrap items-center gap-1 sm:gap-2 text-xs"
      aria-label={t('experiment.flowNav')}
    >
      {STEPS.map((step, idx) => {
        const done = stepDone(step.id, props)
        const active = idx === activeIdx
        return (
          <div key={step.id} className="flex items-center gap-1 sm:gap-2">
            {idx > 0 ? (
              <span className="hidden sm:inline text-muted-foreground/40" aria-hidden>
                →
              </span>
            ) : null}
            <span
              className={[
                'inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 border transition-colors',
                done
                  ? 'border-primary/30 bg-primary/10 text-primary'
                  : active
                    ? 'border-primary bg-primary/5 text-foreground font-medium'
                    : 'border-border bg-muted/20 text-muted-foreground',
              ].join(' ')}
            >
              {done ? <Check className="h-3 w-3 shrink-0" aria-hidden /> : null}
              {t(step.labelKey)}
              {step.id === 'strategies' && props.strategiesCount > 0 ? (
                <span className="tabular-nums opacity-80">({props.strategiesCount})</span>
              ) : null}
            </span>
          </div>
        )
      })}
    </nav>
  )
}
