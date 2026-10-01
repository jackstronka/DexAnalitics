import { AlertTriangle } from 'lucide-react'
import type { PositionStreamPnLResponse } from '@/lib/api'
import {
  chainEconomicQualityLabel,
  needsChainEconomicQualityBanner,
  parseChainEconomicQuality,
} from '@/lib/chainEconomicQuality'
import { useI18n } from '@/lib/i18n'

type Props = {
  totals: Pick<
    PositionStreamPnLResponse,
    'economic_quality' | 'end_nav_source'
  > | null | undefined
  className?: string
}

export function ChainEconomicQualityBanner({ totals, className }: Props) {
  const { t, locale } = useI18n()
  if (!needsChainEconomicQualityBanner(totals)) {
    return null
  }
  const quality = parseChainEconomicQuality(totals?.economic_quality)
  if (!quality) {
    return null
  }
  const label = chainEconomicQualityLabel(quality, locale)
  const endSrc = totals?.end_nav_source?.trim()
  const message =
    quality === 'degraded'
      ? t('chainEconomic.qualityBannerDegraded')
      : t('chainEconomic.qualityBannerEstimated')

  return (
    <div
      className={
        className ??
        'rounded-md border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-xs text-amber-900 dark:text-amber-100 flex gap-2'
      }
      role="status"
    >
      <AlertTriangle className="h-4 w-4 shrink-0 mt-0.5" aria-hidden />
      <span>
        {message}{' '}
        <span className="font-medium">
          ({t('chainEconomic.qualityLabel')}: {label}
          {endSrc ? `, end NAV: ${endSrc}` : ''})
        </span>
      </span>
    </div>
  )
}
