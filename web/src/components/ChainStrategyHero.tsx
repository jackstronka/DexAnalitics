import { AlertTriangle } from 'lucide-react'
import { useMemo } from 'react'
import { useI18n } from '@/lib/i18n'

export type ChainStrategyHeroProps = {
  startUsd?: string | null
  chainWalletUsd?: string | null
  lpNavUsd?: string | null
  txFeesUsd?: string | null
  needsBackfill?: boolean
  metricsUntrusted?: boolean
}

function parseUsd(v: string | null | undefined): number | null {
  if (v == null || v.trim() === '') return null
  const n = parseFloat(v)
  return Number.isFinite(n) ? n : null
}

function formatUsd(n: number | null): string {
  if (n == null) return '—'
  return `$${n.toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`
}

function formatPct(n: number | null, start: number | null): string {
  if (n == null || start == null || start === 0) return '—'
  const pct = (n / start) * 100
  const sign = pct >= 0 ? '+' : ''
  return `${sign}${pct.toFixed(1)}%`
}

function HeroRow({ label, value, emphasize }: { label: string; value: string; emphasize?: boolean }) {
  return (
    <div className="flex justify-between gap-3 text-sm tabular-nums">
      <span className="text-muted-foreground shrink-0">{label}</span>
      <span className={emphasize ? 'font-semibold text-foreground' : 'font-medium'}>{value}</span>
    </div>
  )
}

export function ChainStrategyHero({
  startUsd,
  chainWalletUsd,
  lpNavUsd,
  txFeesUsd,
  needsBackfill,
  metricsUntrusted,
}: ChainStrategyHeroProps) {
  const { t } = useI18n()
  const calc = useMemo(() => {
    const start = parseUsd(startUsd)
    const chain = parseUsd(chainWalletUsd)
    const lp = parseUsd(lpNavUsd)
    const tx = parseUsd(txFeesUsd) ?? 0
    const total = chain != null && lp != null ? chain + lp : null
    const vsStart = start != null && total != null ? total - start - tx : null
    return { start, chain, lp, tx, total, vsStart }
  }, [startUsd, chainWalletUsd, lpNavUsd, txFeesUsd])

  const vsStartTone =
    calc.vsStart == null
      ? 'text-foreground'
      : calc.vsStart >= 0
        ? 'text-emerald-600 dark:text-emerald-400'
        : 'text-red-600 dark:text-red-400'

  return (
    <div className="rounded-md border-2 border-primary/40 bg-primary/5 px-3 py-3 space-y-2">
      <div>
        <p className="text-sm font-semibold text-foreground">{t('chainStrategy.title')}</p>
        <p className="text-[11px] text-muted-foreground leading-relaxed mt-0.5">{t('chainStrategy.subtitle')}</p>
      </div>
      {needsBackfill ? (
        <div className="rounded-md border border-amber-500/40 bg-amber-500/10 px-2 py-1.5 text-xs flex gap-2 text-amber-900 dark:text-amber-100">
          <AlertTriangle className="h-3.5 w-3.5 shrink-0 mt-0.5" aria-hidden />
          <span>{t('chainStrategy.needsBackfill')}</span>
        </div>
      ) : null}
      {metricsUntrusted ? (
        <p className="text-[11px] text-amber-800 dark:text-amber-200">{t('chainStrategy.metricsUntrustedNote')}</p>
      ) : null}
      <div className="space-y-1.5 border-t border-border/50 pt-2">
        <HeroRow label={t('chainStrategy.start')} value={formatUsd(calc.start)} />
        <HeroRow label={t('chainStrategy.lpNav')} value={formatUsd(calc.lp)} />
        <HeroRow label={t('chainStrategy.chainWallet')} value={formatUsd(calc.chain)} />
        <HeroRow label={t('chainStrategy.total')} value={formatUsd(calc.total)} emphasize />
        <HeroRow label={t('chainStrategy.txFees')} value={formatUsd(calc.tx)} />
        <div className="flex justify-between gap-3 text-sm tabular-nums pt-1 border-t border-border/40">
          <span className="text-muted-foreground shrink-0">{t('chainStrategy.vsStart')}</span>
          <span className={`font-semibold ${vsStartTone}`}>
            {formatUsd(calc.vsStart)} ({formatPct(calc.vsStart, calc.start)})
          </span>
        </div>
      </div>
    </div>
  )
}
