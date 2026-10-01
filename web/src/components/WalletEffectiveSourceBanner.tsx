import { AlertTriangle, CheckCircle2 } from 'lucide-react'
import { useI18n } from '@/lib/i18n'

export type WalletEffectiveSourceBannerProps = {
  source?: string | null
  className?: string
}

export function WalletEffectiveSourceBanner({ source, className }: WalletEffectiveSourceBannerProps) {
  const { t } = useI18n()
  if (source === 'gl_wallet') {
    return (
      <div
        className={`rounded-md border border-emerald-500/40 bg-emerald-500/10 px-3 py-2 text-sm flex gap-2 text-emerald-800 dark:text-emerald-200 ${className ?? ''}`}
      >
        <CheckCircle2 className="h-4 w-4 shrink-0 mt-0.5" aria-hidden />
        <span>{t('wallet.effectiveSourceGl')}</span>
      </div>
    )
  }
  if (source === 'rpc_fallback') {
    return (
      <div
        className={`rounded-md border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-sm flex gap-2 text-amber-900 dark:text-amber-100 ${className ?? ''}`}
      >
        <AlertTriangle className="h-4 w-4 shrink-0 mt-0.5" aria-hidden />
        <span>{t('wallet.effectiveSourceRpcFallback')}</span>
      </div>
    )
  }
  return null
}
