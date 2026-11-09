import type { DipLadderRow } from '@/lib/dipLadder'
import { formatDipUsd } from '@/lib/dipSnapshotDisplay'
import { useI18n } from '@/lib/i18n'

function fmtPct(n: number): string {
  if (!Number.isFinite(n)) return '—'
  return `${n.toFixed(2)}%`
}

export function DipLadderPreviewTable({ rows }: { rows: DipLadderRow[] }) {
  const { t } = useI18n()
  if (rows.length === 0) {
    return <p className="text-sm text-muted-foreground">{t('dip.ladderEmpty')}</p>
  }
  return (
    <div className="overflow-x-auto rounded-md border">
      <table className="w-full text-left text-sm">
        <thead className="bg-muted/50">
          <tr>
            <th className="px-3 py-2 font-medium whitespace-nowrap">{t('dip.ladderColLevel')}</th>
            <th className="px-3 py-2 font-medium whitespace-nowrap">{t('dip.ladderColDev')}</th>
            <th className="px-3 py-2 font-medium whitespace-nowrap">{t('dip.ladderColQuote')}</th>
            <th className="px-3 py-2 font-medium whitespace-nowrap">{t('dip.ladderColBase')}</th>
            <th className="px-3 py-2 font-medium whitespace-nowrap">{t('dip.ladderColPrice')}</th>
            <th className="px-3 py-2 font-medium whitespace-nowrap">{t('dip.ladderColAvg')}</th>
            <th className="px-3 py-2 font-medium whitespace-nowrap">{t('dip.ladderColTp')}</th>
            <th className="px-3 py-2 font-medium whitespace-nowrap">{t('dip.ladderColCum')}</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => (
            <tr key={r.level} className="border-t">
              <td className="px-3 py-2 font-medium">{r.level}</td>
              <td className="px-3 py-2">{fmtPct(r.deviationPct)}</td>
              <td className="px-3 py-2">
                {r.kind === 'tp' ? '—' : formatDipUsd(r.orderSizeQuote)}
              </td>
              <td className="px-3 py-2 font-mono text-xs">
                {Number.isFinite(r.orderSizeBase) ? r.orderSizeBase.toFixed(6) : '—'}
              </td>
              <td className="px-3 py-2">{formatDipUsd(r.estPrice)}</td>
              <td className="px-3 py-2">{formatDipUsd(r.avgEntryAfter)}</td>
              <td className="px-3 py-2">{formatDipUsd(r.tpPriceAfter)}</td>
              <td className="px-3 py-2">{formatDipUsd(r.cumulativeQuote)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}
