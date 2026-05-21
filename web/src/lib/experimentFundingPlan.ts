import { rawToUi } from '@/lib/experimentCapital'
import {
  estimateSwapInputRawExactIn,
  estimateSwapInputRawFromPoolPrice,
  isInsufficientBalance,
} from '@/lib/openPositionSwapEstimates'

export type ArmTokenNeed = {
  armId: string
  /** Raw deposit caps from quote-open-budget (`token_max_*`). */
  amount_a_raw: number
  amount_b_raw: number
}

export type AggregateTokenNeeds = {
  total_a_raw: number
  total_b_raw: number
  total_a_ui: number
  total_b_ui: number
}

export type FundingDeficits = {
  deficit_a_ui: number
  deficit_b_ui: number
  short_a: boolean
  short_b: boolean
  short_both: boolean
}

export type SharedSwapPlan = {
  specified_mint: string
  amount_in: number
  direction: 'a_to_b' | 'b_to_a'
  label: string
}

export type ExperimentFundingPlan = {
  aggregate: AggregateTokenNeeds
  deficits: FundingDeficits
  recommended_swap: SharedSwapPlan | null
}

const DEFAULT_SPEND_CAP_PCT = 0.92

export function aggregateTokenNeeds(
  arms: ArmTokenNeed[],
  decimalsA: number,
  decimalsB: number,
): AggregateTokenNeeds {
  let total_a_raw = 0
  let total_b_raw = 0
  for (const arm of arms) {
    total_a_raw += arm.amount_a_raw
    total_b_raw += arm.amount_b_raw
  }
  return {
    total_a_raw,
    total_b_raw,
    total_a_ui: rawToUi(total_a_raw, decimalsA),
    total_b_ui: rawToUi(total_b_raw, decimalsB),
  }
}

export function computeFundingDeficits(
  totalNeedAUi: number,
  totalNeedBUi: number,
  haveAUi: number,
  haveBUi: number,
): FundingDeficits {
  const short_a = isInsufficientBalance(totalNeedAUi, haveAUi)
  const short_b = isInsufficientBalance(totalNeedBUi, haveBUi)
  return {
    deficit_a_ui: short_a ? Math.max(0, totalNeedAUi - haveAUi) : 0,
    deficit_b_ui: short_b ? Math.max(0, totalNeedBUi - haveBUi) : 0,
    short_a,
    short_b,
    short_both: short_a && short_b,
  }
}

export type PlanSharedSwapInput = {
  mintA: string
  mintB: string
  symbolA: string
  symbolB: string
  decimalsA: number
  decimalsB: number
  totalNeedAUi: number
  totalNeedBUi: number
  haveAUi: number
  haveBUi: number
  pricesUsd?: Record<string, number>
  poolPriceRaw?: number
  spendCapPct?: number
}

/**
 * Single in-pool Orca swap plan for aggregated experiment funding (§6.2).
 * Returns null when funded, both legs short, or swap amount cannot be estimated.
 */
export function planSharedSwap(input: PlanSharedSwapInput): SharedSwapPlan | null {
  const deficits = computeFundingDeficits(
    input.totalNeedAUi,
    input.totalNeedBUi,
    input.haveAUi,
    input.haveBUi,
  )
  if (deficits.short_both || (!deficits.short_a && !deficits.short_b)) {
    return null
  }

  const capPct = input.spendCapPct ?? DEFAULT_SPEND_CAP_PCT
  const px = input.pricesUsd
  const poolPriceRaw = input.poolPriceRaw ?? Number.NaN

  if (deficits.short_b && !deficits.short_a) {
    const rawEstUsd = estimateSwapInputRawExactIn(
      input.mintA,
      input.decimalsA,
      input.mintB,
      deficits.deficit_b_ui,
      px,
    )
    const rawEstPool = estimateSwapInputRawFromPoolPrice(
      deficits.deficit_a_ui,
      deficits.deficit_b_ui,
      true,
      input.decimalsA,
      input.decimalsB,
      poolPriceRaw,
    )
    const rawEst = Math.max(rawEstUsd ?? 0, rawEstPool ?? 0)
    if (rawEst <= 0) return null
    const maxRaw = Math.floor(input.haveAUi * 10 ** input.decimalsA * capPct)
    const amount_in = Math.min(Math.floor(rawEst), maxRaw)
    if (amount_in <= 0) return null
    return {
      specified_mint: input.mintA,
      amount_in,
      direction: 'a_to_b',
      label: `${input.symbolA} → ${input.symbolB} (aggregated, in-pool Orca)`,
    }
  }

  if (deficits.short_a && !deficits.short_b) {
    const rawEstUsd = estimateSwapInputRawExactIn(
      input.mintB,
      input.decimalsB,
      input.mintA,
      deficits.deficit_a_ui,
      px,
    )
    const rawEstPool = estimateSwapInputRawFromPoolPrice(
      deficits.deficit_a_ui,
      deficits.deficit_b_ui,
      false,
      input.decimalsA,
      input.decimalsB,
      poolPriceRaw,
    )
    const rawEst = Math.max(rawEstUsd ?? 0, rawEstPool ?? 0)
    if (rawEst <= 0) return null
    const maxRaw = Math.floor(input.haveBUi * 10 ** input.decimalsB * capPct)
    const amount_in = Math.min(Math.floor(rawEst), maxRaw)
    if (amount_in <= 0) return null
    return {
      specified_mint: input.mintB,
      amount_in,
      direction: 'b_to_a',
      label: `${input.symbolB} → ${input.symbolA} (aggregated, in-pool Orca)`,
    }
  }

  return null
}

export type BuildExperimentFundingPlanInput = {
  arms: ArmTokenNeed[]
  decimalsA: number
  decimalsB: number
  mintA: string
  mintB: string
  symbolA: string
  symbolB: string
  haveAUi: number
  haveBUi: number
  pricesUsd?: Record<string, number>
  poolPriceRaw?: number
}

export function buildExperimentFundingPlan(
  input: BuildExperimentFundingPlanInput,
): ExperimentFundingPlan {
  const aggregate = aggregateTokenNeeds(input.arms, input.decimalsA, input.decimalsB)
  const deficits = computeFundingDeficits(
    aggregate.total_a_ui,
    aggregate.total_b_ui,
    input.haveAUi,
    input.haveBUi,
  )
  const recommended_swap = planSharedSwap({
    mintA: input.mintA,
    mintB: input.mintB,
    symbolA: input.symbolA,
    symbolB: input.symbolB,
    decimalsA: input.decimalsA,
    decimalsB: input.decimalsB,
    totalNeedAUi: aggregate.total_a_ui,
    totalNeedBUi: aggregate.total_b_ui,
    haveAUi: input.haveAUi,
    haveBUi: input.haveBUi,
    pricesUsd: input.pricesUsd,
    poolPriceRaw: input.poolPriceRaw,
  })
  return { aggregate, deficits, recommended_swap }
}
