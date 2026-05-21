import type { ExperimentArm } from '@/lib/experimentArm'
import { buildArmParameters } from '@/lib/experimentArm'

export type ArmStatRow = {
  key: string
  label: string
  value: string
}

function fmtNum(v: number | '' | undefined, suffix = ''): string {
  if (v === '' || v == null || !Number.isFinite(Number(v))) return '—'
  return `${Number(v)}${suffix}`
}

export function experimentArmStatRows(
  arm: ExperimentArm,
  locale: 'pl' | 'en',
): ArmStatRow[] {
  const pl = locale === 'pl'
  const p = buildArmParameters(arm.form)
  const rows: ArmStatRow[] = [
    {
      key: 'type',
      label: pl ? 'Typ' : 'Type',
      value: arm.form.strategyType.replace(/_/g, ' '),
    },
    {
      key: 'width',
      label: pl ? 'Szerokość' : 'Range width',
      value: fmtNum(p.range_width_pct as number | undefined, '%'),
    },
    {
      key: 'threshold',
      label: pl ? 'Próg' : 'Threshold',
      value: fmtNum(p.rebalance_threshold_pct as number | undefined, '%'),
    },
    {
      key: 'interval',
      label: pl ? 'Min. odstęp' : 'Min. interval',
      value:
        typeof p.min_rebalance_interval_minutes === 'number'
          ? `${p.min_rebalance_interval_minutes} min`
          : '—',
    },
    {
      key: 'ticks',
      label: 'Ticks',
      value:
        arm.tickLower !== '' && arm.tickUpper !== ''
          ? `[${arm.tickLower}, ${arm.tickUpper}]`
          : '—',
    },
  ]

  if (arm.form.strategyType === 'bollinger') {
    rows.push(
      {
        key: 'bb-window',
        label: 'BB window',
        value: fmtNum(arm.form.bollingerWindow),
      },
      {
        key: 'bb-k',
        label: 'BB k',
        value: fmtNum(arm.form.bollingerK),
      },
    )
  }

  if (arm.presetName) {
    rows.push({
      key: 'preset',
      label: pl ? 'Preset' : 'Preset',
      value: arm.presetName,
    })
  }

  if (arm.budgetUsd !== '') {
    rows.push({
      key: 'budget',
      label: pl ? 'Budżet' : 'Budget',
      value: `$${Number(arm.budgetUsd).toFixed(2)}`,
    })
  }

  return rows
}
