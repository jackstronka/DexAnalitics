/** Pure helpers for Bot Control UI (U5 / BC1–BC3). */

export type BotDryRunMismatch = 'none' | 'local_live_api_dry' | 'local_dry_api_live'

/** Compare localStorage UI dry-run preference vs API host `DRY_RUN`. */
export function botDryRunMismatch(
  localDryRun: boolean,
  apiDryRun: boolean | null | undefined,
): BotDryRunMismatch {
  if (apiDryRun == null) return 'none'
  if (!localDryRun && apiDryRun) return 'local_live_api_dry'
  if (localDryRun && !apiDryRun) return 'local_dry_api_live'
  return 'none'
}

export type StrategyExecBadge = {
  running: boolean
  dryRun: boolean
  autoExecute: boolean
  /** True when strategy could submit live txs (not dry-run + auto_execute). */
  liveArmed: boolean
}

export function strategyExecBadge(s: {
  running?: boolean
  dry_run?: boolean
  auto_execute?: boolean
}): StrategyExecBadge {
  const dryRun = Boolean(s.dry_run)
  const autoExecute = Boolean(s.auto_execute)
  return {
    running: Boolean(s.running),
    dryRun,
    autoExecute,
    liveArmed: !dryRun && autoExecute,
  }
}

export function summarizeStrategyFleet(
  strategies: Array<{ running?: boolean; dry_run?: boolean; auto_execute?: boolean }>,
): {
  total: number
  running: number
  liveArmedRunning: number
} {
  let running = 0
  let liveArmedRunning = 0
  for (const s of strategies) {
    const b = strategyExecBadge(s)
    if (b.running) {
      running += 1
      if (b.liveArmed) liveArmedRunning += 1
    }
  }
  return { total: strategies.length, running, liveArmedRunning }
}
