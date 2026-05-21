import { Plus } from 'lucide-react'
import { MAX_EXPERIMENT_ARMS } from '@/lib/experimentCapital'
import { useI18n } from '@/lib/i18n'

type Props = {
  onClick: () => void
  disabled?: boolean
  disabledReason?: string
  size?: 'hero' | 'inline'
  currentCount?: number
}

export default function ExperimentAddStrategySlot({
  onClick,
  disabled,
  disabledReason,
  size = 'inline',
  currentCount = 0,
}: Props) {
  const { t } = useI18n()
  const hero = size === 'hero'
  const remaining = MAX_EXPERIMENT_ARMS - currentCount

  return (
    <div className="relative pl-0 sm:pl-8">
      {size === 'inline' ? (
        <div className="hidden sm:block absolute left-3 top-0 h-1/2 w-px bg-border" aria-hidden />
      ) : null}
      <button
        type="button"
        disabled={disabled}
        onClick={onClick}
        title={disabled ? disabledReason : undefined}
        className={[
          'group w-full rounded-xl border-2 border-dashed transition-all duration-200',
          disabled
            ? 'border-border/60 bg-muted/5 opacity-60 cursor-not-allowed'
            : 'border-border bg-muted/5 hover:border-primary hover:bg-primary/5 hover:shadow-sm active:scale-[0.99]',
          hero ? 'py-14 px-6' : 'py-6 px-4',
        ].join(' ')}
      >
        <div className="flex flex-col items-center justify-center gap-2 text-muted-foreground group-hover:text-primary group-disabled:hover:text-muted-foreground">
          <span
            className={[
              'flex items-center justify-center rounded-full border-2 border-current transition-transform group-hover:scale-105',
              hero ? 'h-14 w-14' : 'h-9 w-9',
            ].join(' ')}
          >
            <Plus className={hero ? 'h-7 w-7' : 'h-4 w-4'} strokeWidth={2.5} />
          </span>
          <span className={hero ? 'text-base font-semibold' : 'text-sm font-medium'}>
            {hero ? t('experiment.addStrategyHero') : t('experiment.addStrategyInline')}
          </span>
          <span className="text-xs text-muted-foreground text-center max-w-xs leading-relaxed">
            {disabled && disabledReason
              ? disabledReason
              : hero
                ? t('experiment.addStrategyHeroHint')
                : t('experiment.addStrategyInlineHint').replace('{remaining}', String(remaining))}
          </span>
        </div>
      </button>
    </div>
  )
}
