import { ArrowRight, Check, Circle, Loader2 } from 'lucide-react'
import { useI18n } from '@/lib/i18n'

type StepState = 'done' | 'active' | 'pending' | 'warn'

type Props = {
  ticksReady: boolean
  quotesReady: boolean
  capitalReady: boolean
  sharedSwapNeeded: boolean
  sharedSwapDone: boolean
  launchDone: boolean
}

function StepIcon({ state }: { state: StepState }) {
  if (state === 'done') return <Check className="h-3.5 w-3.5 text-primary" />
  if (state === 'active') return <Loader2 className="h-3.5 w-3.5 animate-spin text-primary" />
  if (state === 'warn') return <Circle className="h-3.5 w-3.5 text-amber-500 fill-amber-500/20" />
  return <Circle className="h-3.5 w-3.5 text-muted-foreground/40" />
}

export default function ExperimentLaunchPipeline({
  ticksReady,
  quotesReady,
  capitalReady,
  sharedSwapNeeded,
  sharedSwapDone,
  launchDone,
}: Props) {
  const { t } = useI18n()

  const steps: { key: string; label: string; detail: string; state: StepState }[] = [
    {
      key: 'ticks',
      label: t('experiment.pipelineTicks'),
      detail: t('experiment.pipelineTicksDetail'),
      state: ticksReady ? 'done' : 'active',
    },
    {
      key: 'quotes',
      label: t('experiment.pipelineQuotes'),
      detail: t('experiment.pipelineQuotesDetail'),
      state: quotesReady ? 'done' : ticksReady ? 'active' : 'pending',
    },
    {
      key: 'swap',
      label: t('experiment.pipelineSwap'),
      detail: sharedSwapNeeded
        ? t('experiment.pipelineSwapNeeded')
        : t('experiment.pipelineSwapSkip'),
      state: !sharedSwapNeeded
        ? 'done'
        : sharedSwapDone
          ? 'done'
          : quotesReady
            ? 'warn'
            : 'pending',
    },
    {
      key: 'launch',
      label: t('experiment.pipelineLaunch'),
      detail: t('experiment.pipelineLaunchDetail'),
      state: launchDone ? 'done' : capitalReady ? 'active' : 'pending',
    },
  ]

  return (
    <div className="rounded-xl border border-border bg-muted/10 px-4 py-3">
      <div className="text-xs font-medium text-muted-foreground mb-3">{t('experiment.pipelineTitle')}</div>
      <ol className="space-y-2">
        {steps.map((step, idx) => (
          <li key={step.key} className="flex items-start gap-2 text-sm">
            <span className="mt-0.5 shrink-0">
              <StepIcon state={step.state} />
            </span>
            <div className="min-w-0 flex-1">
              <div className="font-medium leading-tight">{step.label}</div>
              <div className="text-xs text-muted-foreground mt-0.5">{step.detail}</div>
            </div>
            {idx < steps.length - 1 ? (
              <ArrowRight className="h-3.5 w-3.5 text-muted-foreground/30 shrink-0 mt-1 hidden sm:block" aria-hidden />
            ) : null}
          </li>
        ))}
      </ol>
    </div>
  )
}
