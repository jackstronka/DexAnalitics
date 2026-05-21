import type { SharedSwapPlan } from '@/lib/experimentFundingPlan'

export type CreateStrategyPayload = {
  name: string
  strategy_type: string
  parameters: Record<string, unknown>
  auto_execute?: boolean
  dry_run?: boolean
}

export type ExperimentArmLaunchSpec = {
  armId: string
  enabled: boolean
  tick_lower: number
  tick_upper: number
  amount_a: number
  amount_b: number
  strategyId?: string
  createStrategy?: CreateStrategyPayload
  cost_session_id?: string
  slippage_tolerance_bps?: number
}

export type LaunchArmResult = {
  armId: string
  status: 'pending' | 'opened' | 'failed' | 'skipped'
  strategyId?: string
  costSessionId?: string
  positionPda?: string
  error?: string
}

export type FundingStatus = {
  shortA: boolean
  shortB: boolean
  deficitA: number
  deficitB: number
}

export type LaunchExperimentInput = {
  poolAddress: string
  arms: ExperimentArmLaunchSpec[]
  sharedSwap?: SharedSwapPlan | null
  /** When set, skip swap step (already executed). */
  sharedSwapSignature?: string
  slippage_tolerance_bps?: number
  batchId?: string
}

export type LaunchExperimentResult = {
  batchId: string
  sharedSwapSignature?: string
  arms: LaunchArmResult[]
  aborted?: boolean
  abortReason?: string
}

export type LaunchExperimentDeps = {
  generateSessionId: () => string
  generateBatchId: () => string
  createStrategy?: (payload: CreateStrategyPayload) => Promise<{ id: string }>
  swapBeforeOpen?: (request: {
    pool_address: string
    specified_mint: string
    amount_in: number
    slippage_tolerance_bps?: number
    cost_session_id?: string
  }) => Promise<{ swap_signature?: string; cost_session_id?: string; message?: string }>
  openPosition: (request: {
    pool_address: string
    tick_lower: number
    tick_upper: number
    amount_a: number
    amount_b: number
    strategy_id?: string
    cost_session_id?: string
    slippage_tolerance_bps?: number
  }) => Promise<{ position_pda?: string; cost_session_id?: string; message?: string }>
  getFundingStatus?: () => Promise<FundingStatus>
}

/** BUG-20260512-05: block open when wallet still short after confirmed swap. */
export function shouldBlockOpenAfterConfirmedSwap(funding: {
  shortA: boolean
  shortB: boolean
}): boolean {
  return funding.shortA || funding.shortB
}

function fundingBlockMessage(funding: FundingStatus): string {
  const parts: string[] = []
  if (funding.shortA && funding.deficitA > 0) parts.push(`A: ${funding.deficitA}`)
  if (funding.shortB && funding.deficitB > 0) parts.push(`B: ${funding.deficitB}`)
  const missing = parts.length > 0 ? parts.join(', ') : 'tokens'
  return `Swap confirmed, but wallet still does not cover open amounts. Missing: ${missing}.`
}

/**
 * Sequential experiment launch: optional shared swap, then per-arm strategy + open.
 * Partial failure on open does not stop subsequent arms; shared swap failure aborts all.
 */
export async function launchExperiment(
  input: LaunchExperimentInput,
  deps: LaunchExperimentDeps,
): Promise<LaunchExperimentResult> {
  const batchId = input.batchId ?? deps.generateBatchId()
  const enabledArms = input.arms.filter((a) => a.enabled)
  const armResults: LaunchArmResult[] = enabledArms.map((a) => ({
    armId: a.armId,
    status: 'pending',
  }))

  let sharedSwapSignature = input.sharedSwapSignature

  if (input.sharedSwap && !sharedSwapSignature) {
    if (!deps.swapBeforeOpen) {
      return {
        batchId,
        arms: armResults.map((r) => ({
          ...r,
          status: 'skipped',
          error: 'swapBeforeOpen dependency missing',
        })),
        aborted: true,
        abortReason: 'swap dependency missing',
      }
    }
    try {
      const swapRes = await deps.swapBeforeOpen({
        pool_address: input.poolAddress,
        specified_mint: input.sharedSwap.specified_mint,
        amount_in: input.sharedSwap.amount_in,
        slippage_tolerance_bps: input.slippage_tolerance_bps,
        cost_session_id: deps.generateSessionId(),
      })
      sharedSwapSignature = swapRes.swap_signature
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e)
      return {
        batchId,
        arms: armResults.map((r) => ({
          ...r,
          status: 'skipped',
          error: `Shared swap failed: ${msg}`,
        })),
        aborted: true,
        abortReason: msg,
      }
    }
  }

  const hadSharedSwap = Boolean(input.sharedSwap && sharedSwapSignature)

  for (let i = 0; i < enabledArms.length; i++) {
    const spec = enabledArms[i]
    const result = armResults[i]

    try {
      if (hadSharedSwap && deps.getFundingStatus) {
        const funding = await deps.getFundingStatus()
        if (shouldBlockOpenAfterConfirmedSwap(funding)) {
          result.status = 'failed'
          result.error = fundingBlockMessage(funding)
          continue
        }
      }

      let strategyId = spec.strategyId
      if (!strategyId && spec.createStrategy) {
        if (!deps.createStrategy) {
          throw new Error('createStrategy dependency missing')
        }
        const created = await deps.createStrategy(spec.createStrategy)
        strategyId = created.id
      }

      const costSessionId = spec.cost_session_id ?? deps.generateSessionId()
      const openRes = await deps.openPosition({
        pool_address: input.poolAddress,
        tick_lower: spec.tick_lower,
        tick_upper: spec.tick_upper,
        amount_a: spec.amount_a,
        amount_b: spec.amount_b,
        strategy_id: strategyId,
        cost_session_id: costSessionId,
        slippage_tolerance_bps: spec.slippage_tolerance_bps ?? input.slippage_tolerance_bps,
      })

      result.status = 'opened'
      result.strategyId = strategyId
      result.costSessionId = openRes.cost_session_id ?? costSessionId
      result.positionPda = openRes.position_pda
    } catch (e) {
      result.status = 'failed'
      result.error = e instanceof Error ? e.message : String(e)
    }
  }

  return {
    batchId,
    sharedSwapSignature,
    arms: armResults,
  }
}
