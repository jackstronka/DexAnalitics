import { useEffect, useMemo, useState } from 'react'
import { useMutation, useQueries, useQuery, useQueryClient } from '@tanstack/react-query'
import { Button } from '@/components/ui/button'
import { InlineError } from '@/components/ui/inline-error'
import {
  getApiSignerWallet,
  getMintPricesUsd,
  getWalletEffectiveBalances,
  getWallets,
  quoteOpenBudget,
  swapBeforeOpen,
  type QuoteOpenBudgetResponse,
} from '@/lib/api'
import ExperimentLaunchPipeline from '@/components/ExperimentLaunchPipeline'
import type { ExperimentArm } from '@/lib/experimentArm'
import {
  armTokenNeedsFromQuotes,
  quotesReady,
  resolveArmBudgetsUsd,
  type ArmBudgetQuote,
} from '@/lib/experimentBudgetPlan'
import {
  DEFAULT_MIN_ARM_BUDGET_USD,
  isBelowMinArmBudget,
  splitBudgetEqual,
  validateArmBudgets,
  type AllocationMode,
} from '@/lib/experimentCapital'
import { buildExperimentFundingPlan } from '@/lib/experimentFundingPlan'
import { computeSolFirstFundingBalances, WSOL_MINT } from '@/lib/solFirstFunding'
import { getDevWalletPubkey } from '@/lib/devWallet'
import { formatUSD } from '@/lib/utils'
import { useI18n } from '@/lib/i18n'

const LS_SELECTED_WALLET_ID = 'clmm.selected_wallet_id'

type TokenMeta = { mint: string; symbol: string; decimals: number }

type Props = {
  poolAddress: string
  poolPriceRaw?: number
  tokenA: TokenMeta
  tokenB: TokenMeta
  arms: ExperimentArm[]
  totalBudgetUsd: number | ''
  allocationMode: AllocationMode
  onTotalBudgetChange: (v: number | '') => void
  onAllocationModeChange: (mode: AllocationMode) => void
  onArmBudgetChange: (armId: string, budgetUsd: number | '') => void
  sharedSwapSignature: string | null
  sharedCostSessionId: string | null
  onSharedSwapComplete: (sig: string | null, sessionId: string | null) => void
  onReadyChange?: (ready: boolean) => void
  onFundingReadyChange?: (ready: boolean) => void
  onArmRowsChange?: (rows: ArmBudgetQuote[]) => void
  onPipelineChange?: (snap: {
    sharedSwapNeeded: boolean
    sharedSwapDone: boolean
    shortA: boolean
    shortB: boolean
    shortBoth: boolean
  }) => void
  /** Budget controls live on roster bar. */
  hideBudgetControls?: boolean
}

function aggregateNeedSolLegUi(
  armRows: ArmBudgetQuote[],
  tokenAMint: string,
  tokenBMint: string,
): number {
  if (!quotesReady(armRows)) return 0
  let total = 0
  for (const row of armRows) {
    if (!row.quote) continue
    if (tokenAMint === WSOL_MINT) total += row.quote.amount_a_ui
    else if (tokenBMint === WSOL_MINT) total += row.quote.amount_b_ui
  }
  return total
}

function readBudgetInput(raw: string): number | '' {
  if (raw.trim() === '') return ''
  const n = Number(raw)
  return Number.isFinite(n) && n >= 0 ? n : ''
}

function makeCostSessionId() {
  if (typeof crypto !== 'undefined' && 'randomUUID' in crypto) {
    return crypto.randomUUID()
  }
  return `${Date.now()}-${Math.random().toString(16).slice(2)}`
}

export default function ExperimentCapitalStep({
  poolAddress,
  poolPriceRaw,
  tokenA,
  tokenB,
  arms,
  totalBudgetUsd,
  allocationMode,
  onTotalBudgetChange,
  onAllocationModeChange,
  onArmBudgetChange,
  sharedSwapSignature,
  sharedCostSessionId,
  onSharedSwapComplete,
  onReadyChange,
  onFundingReadyChange,
  onArmRowsChange,
  onPipelineChange,
  hideBudgetControls = false,
}: Props) {
  const { t, locale } = useI18n()
  const L = (pl: string, en: string) => (locale === 'pl' ? pl : en)
  const queryClient = useQueryClient()
  const [swapError, setSwapError] = useState<string | null>(null)
  const [armSwapBusyId, setArmSwapBusyId] = useState<string | null>(null)

  const enabledArms = useMemo(() => arms.filter((a) => a.enabled), [arms])

  const budgetByArmId = useMemo(
    () => resolveArmBudgetsUsd(arms, totalBudgetUsd, allocationMode),
    [arms, totalBudgetUsd, allocationMode],
  )

  const quoteQueries = useQueries({
    queries: enabledArms.map((arm) => {
      const budget = budgetByArmId.get(arm.id)
      const ticksOk = arm.tickLower !== '' && arm.tickUpper !== ''
      return {
        queryKey: [
          'experiment-quote-budget',
          poolAddress,
          arm.id,
          arm.tickLower,
          arm.tickUpper,
          budget,
        ],
        queryFn: () =>
          quoteOpenBudget(poolAddress, {
            tick_lower: Number(arm.tickLower),
            tick_upper: Number(arm.tickUpper),
            target_usd: budget!,
          }),
        enabled:
          poolAddress.trim().length > 0 &&
          ticksOk &&
          budget != null &&
          budget > 0,
        staleTime: 30_000,
      }
    }),
  })

  const armRows: ArmBudgetQuote[] = useMemo(() => {
    return enabledArms.map((arm, i) => ({
      arm,
      budgetUsd: budgetByArmId.get(arm.id) ?? 0,
      quote: quoteQueries[i]?.data,
      quoteError: (quoteQueries[i]?.error as Error | null) ?? null,
      isLoading: quoteQueries[i]?.isLoading ?? false,
    }))
  }, [enabledArms, budgetByArmId, quoteQueries])

  const devPk = getDevWalletPubkey()
  const walletsQ = useQuery({ queryKey: ['wallets'], queryFn: getWallets, staleTime: 30_000 })
  const ownerPk = useMemo(() => {
    if (typeof window === 'undefined') return null
    const id = window.localStorage.getItem(LS_SELECTED_WALLET_ID) || ''
    const picked = walletsQ.data?.wallets.find((w) => w.id === id)
    return picked?.pubkey ?? devPk ?? walletsQ.data?.wallets[0]?.pubkey ?? null
  }, [walletsQ.data?.wallets, devPk])

  const apiSignerQ = useQuery({
    queryKey: ['api-signer-wallet'],
    queryFn: getApiSignerWallet,
    staleTime: 20_000,
  })
  const effectiveOwnerPk = apiSignerQ.data?.pubkey?.trim() || ownerPk

  const balancesQ = useQuery({
    queryKey: ['wallet-balances', effectiveOwnerPk ?? ''],
    queryFn: () => getWalletEffectiveBalances(effectiveOwnerPk!),
    enabled: !!effectiveOwnerPk,
    staleTime: 20_000,
    refetchInterval: 10_000,
  })

  const pricesQ = useQuery({
    queryKey: ['mint-prices', tokenA.mint, tokenB.mint],
    queryFn: () => getMintPricesUsd([tokenA.mint, tokenB.mint]),
    enabled: Boolean(tokenA.mint && tokenB.mint),
    staleTime: 60_000,
  })

  const needSolLegUi = useMemo(
    () => aggregateNeedSolLegUi(armRows, tokenA.mint, tokenB.mint),
    [armRows, tokenA.mint, tokenB.mint],
  )

  const solFirstBalances = useMemo(
    () =>
      computeSolFirstFundingBalances({
        balances: balancesQ.data,
        tokenAMint: tokenA.mint,
        tokenBMint: tokenB.mint,
        minOpenLamports: apiSignerQ.data?.min_open_lamports ?? 0,
        needSolLegUi,
      }),
    [balancesQ.data, tokenA.mint, tokenB.mint, apiSignerQ.data?.min_open_lamports, needSolLegUi],
  )

  const fundingPlan = useMemo(() => {
    if (!quotesReady(armRows)) return null
    const needs = armTokenNeedsFromQuotes(armRows)
    return buildExperimentFundingPlan({
      arms: needs,
      decimalsA: tokenA.decimals,
      decimalsB: tokenB.decimals,
      mintA: tokenA.mint,
      mintB: tokenB.mint,
      symbolA: tokenA.symbol,
      symbolB: tokenB.symbol,
      haveAUi: solFirstBalances.effectiveHaveA,
      haveBUi: solFirstBalances.effectiveHaveB,
      pricesUsd: pricesQ.data?.prices,
      poolPriceRaw,
    })
  }, [armRows, tokenA, tokenB, solFirstBalances.effectiveHaveA, solFirstBalances.effectiveHaveB, pricesQ.data, poolPriceRaw])

  const budgetValidation = useMemo(() => {
    if (totalBudgetUsd === '' || Number(totalBudgetUsd) <= 0) {
      return { valid: false, sum: 0, exceedsTotal: false }
    }
    const budgets = enabledArms.map((a) => budgetByArmId.get(a.id) ?? 0)
    return validateArmBudgets(Number(totalBudgetUsd), budgets)
  }, [totalBudgetUsd, enabledArms, budgetByArmId])

  const capitalReady =
    budgetValidation.valid && quotesReady(armRows) && enabledArms.length > 0 && totalBudgetUsd !== ''

  const fundingReady = Boolean(
    fundingPlan && !fundingPlan.deficits.short_a && !fundingPlan.deficits.short_b,
  )

  const ticksReady = enabledArms.every((a) => a.tickLower !== '' && a.tickUpper !== '')
  const quotesReadyFlag = quotesReady(armRows) && armRows.length === enabledArms.length
  const sharedSwapNeeded = Boolean(fundingPlan?.recommended_swap)
  /** Done when funded, or when no aggregated swap path exists (not merely when a past signature exists). */
  const sharedSwapDone = !sharedSwapNeeded || fundingReady

  useEffect(() => {
    onReadyChange?.(capitalReady)
  }, [capitalReady, onReadyChange])

  useEffect(() => {
    onFundingReadyChange?.(fundingReady)
  }, [fundingReady, onFundingReadyChange])

  useEffect(() => {
    onArmRowsChange?.(armRows)
  }, [armRows, onArmRowsChange])

  useEffect(() => {
    onPipelineChange?.({
      sharedSwapNeeded,
      sharedSwapDone,
      shortA: fundingPlan?.deficits.short_a ?? false,
      shortB: fundingPlan?.deficits.short_b ?? false,
      shortBoth: fundingPlan?.deficits.short_both ?? false,
    })
  }, [sharedSwapNeeded, sharedSwapDone, fundingPlan, onPipelineChange])

  const equalPreview = useMemo(() => {
    if (totalBudgetUsd === '' || Number(totalBudgetUsd) <= 0 || enabledArms.length === 0) {
      return []
    }
    return splitBudgetEqual(Number(totalBudgetUsd), enabledArms.length)
  }, [totalBudgetUsd, enabledArms.length])

  const sharedSwapMutation = useMutation({
    mutationFn: swapBeforeOpen,
    onSuccess: async (data) => {
      setSwapError(null)
      onSharedSwapComplete(data.swap_signature ?? null, data.cost_session_id ?? sharedCostSessionId)
      const owner = effectiveOwnerPk ?? ''
      if (owner) {
        await queryClient.invalidateQueries({ queryKey: ['wallet-balances', owner] })
        await new Promise((r) => setTimeout(r, 800))
        try {
          const fresh = await getWalletEffectiveBalances(owner, { force: true })
          queryClient.setQueryData(['wallet-balances', owner], fresh)
        } catch {
          await queryClient.refetchQueries({ queryKey: ['wallet-balances', owner], type: 'active' })
        }
      }
    },
    onError: (e: Error) => setSwapError(e.message),
  })

  async function runSharedSwap() {
    if (!fundingPlan?.recommended_swap) return
    setSwapError(null)
    const sessionId = sharedCostSessionId ?? makeCostSessionId()
    if (!sharedCostSessionId) {
      onSharedSwapComplete(null, sessionId)
    }
    sharedSwapMutation.mutate({
      pool_address: poolAddress,
      specified_mint: fundingPlan.recommended_swap.specified_mint,
      amount_in: fundingPlan.recommended_swap.amount_in,
      cost_session_id: sessionId,
    })
  }

  async function runArmSwap(row: ArmBudgetQuote) {
    if (!row.quote) return
    const singlePlan = buildExperimentFundingPlan({
      arms: [
        {
          armId: row.arm.id,
          amount_a_raw: row.quote.token_max_a,
          amount_b_raw: row.quote.token_max_b,
        },
      ],
      decimalsA: tokenA.decimals,
      decimalsB: tokenB.decimals,
      mintA: tokenA.mint,
      mintB: tokenB.mint,
      symbolA: tokenA.symbol,
      symbolB: tokenB.symbol,
      haveAUi: solFirstBalances.effectiveHaveA,
      haveBUi: solFirstBalances.effectiveHaveB,
      pricesUsd: pricesQ.data?.prices,
      poolPriceRaw,
    })
    const plan = singlePlan.recommended_swap
    if (!plan) return
    setArmSwapBusyId(row.arm.id)
    setSwapError(null)
    try {
      await swapBeforeOpen({
        pool_address: poolAddress,
        specified_mint: plan.specified_mint,
        amount_in: plan.amount_in,
        cost_session_id: makeCostSessionId(),
      })
      const owner = effectiveOwnerPk ?? ''
      if (owner) {
        await queryClient.invalidateQueries({ queryKey: ['wallet-balances', owner] })
        await getWalletEffectiveBalances(owner, { force: true }).then((fresh) =>
          queryClient.setQueryData(['wallet-balances', owner], fresh),
        )
      }
    } catch (e) {
      setSwapError(e instanceof Error ? e.message : String(e))
    } finally {
      setArmSwapBusyId(null)
    }
  }

  function armFundingStatus(row: ArmBudgetQuote): 'loading' | 'error' | 'funded' | 'short' {
    if (row.isLoading) return 'loading'
    if (row.quoteError || !row.quote) return 'error'
    const plan = buildExperimentFundingPlan({
      arms: [{ armId: row.arm.id, amount_a_raw: row.quote.token_max_a, amount_b_raw: row.quote.token_max_b }],
      decimalsA: tokenA.decimals,
      decimalsB: tokenB.decimals,
      mintA: tokenA.mint,
      mintB: tokenB.mint,
      symbolA: tokenA.symbol,
      symbolB: tokenB.symbol,
      haveAUi: solFirstBalances.effectiveHaveA,
      haveBUi: solFirstBalances.effectiveHaveB,
      pricesUsd: pricesQ.data?.prices,
      poolPriceRaw,
    })
    if (plan.deficits.short_a || plan.deficits.short_b) return 'short'
    return 'funded'
  }

  return (
    <div className="space-y-4">
      <ExperimentLaunchPipeline
        ticksReady={ticksReady}
        quotesReady={quotesReadyFlag}
        capitalReady={capitalReady && sharedSwapDone}
        sharedSwapNeeded={sharedSwapNeeded}
        sharedSwapDone={sharedSwapDone}
        launchDone={false}
      />

      {!hideBudgetControls ? (
        <>
      <div className="grid gap-3 md:grid-cols-2">
        <div>
          <label className="block text-sm font-medium mb-1">{t('experiment.totalBudgetUsd')}</label>
          <input
            type="number"
            min={0}
            step="0.01"
            className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
            value={totalBudgetUsd}
            onChange={(e) => onTotalBudgetChange(readBudgetInput(e.target.value))}
            placeholder="30"
          />
        </div>
        <div>
          <label className="block text-sm font-medium mb-1">{t('experiment.allocationMode')}</label>
          <select
            className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
            value={allocationMode}
            onChange={(e) => onAllocationModeChange(e.target.value as AllocationMode)}
          >
            <option value="equal">{t('experiment.allocationEqual')}</option>
            <option value="fixed_usd">{t('experiment.allocationManual')}</option>
          </select>
        </div>
      </div>

      {allocationMode === 'equal' && equalPreview.length > 0 ? (
        <p className="text-xs text-muted-foreground">
          {t('experiment.equalSplitPreview').replace(
            '{parts}',
            equalPreview.map((v) => formatUSD(v)).join(', '),
          )}
        </p>
      ) : null}

      {!budgetValidation.valid && totalBudgetUsd !== '' ? (
        <InlineError>{t('experiment.budgetExceedsTotal')}</InlineError>
      ) : null}
        </>
      ) : null}

      {fundingPlan ? (
        <div className="rounded-md border border-border bg-muted/10 px-3 py-2 text-sm space-y-1">
          <div className="font-medium">{t('experiment.aggregateNeeds')}</div>
          <div className="text-xs text-muted-foreground font-mono">
            {tokenA.symbol}: {fundingPlan.aggregate.total_a_ui.toFixed(6)} · {tokenB.symbol}:{' '}
            {fundingPlan.aggregate.total_b_ui.toFixed(4)} (~
            {formatUSD(
              armRows.reduce((s, r) => s + (r.quote?.estimated_value_usd ?? r.budgetUsd), 0),
            )}
            )
          </div>
          <div className="text-xs">
            {L('Portfel', 'Wallet')}: {tokenA.symbol}{' '}
            {solFirstBalances.walletDisplayA.toFixed(4)}
            {tokenA.mint === WSOL_MINT && solFirstBalances.splWsolUi > 0
              ? ` (${L('SPL WSOL', 'SPL WSOL')} ${solFirstBalances.splWsolUi.toFixed(4)})`
              : ''}{' '}
            · {tokenB.symbol} {solFirstBalances.walletDisplayB.toFixed(4)}
            {tokenB.mint === WSOL_MINT && solFirstBalances.splWsolUi > 0
              ? ` (${L('SPL WSOL', 'SPL WSOL')} ${solFirstBalances.splWsolUi.toFixed(4)})`
              : ''}
          </div>
          {fundingPlan.deficits.short_both ? (
            <InlineError>{t('experiment.deficitBothLegs')}</InlineError>
          ) : null}
          {fundingPlan.recommended_swap ? (
            <div className="flex flex-wrap items-center gap-2 pt-1">
              <span className="text-xs">{fundingPlan.recommended_swap.label}</span>
              <Button
                type="button"
                size="sm"
                variant="outline"
                disabled={sharedSwapMutation.isPending}
                onClick={runSharedSwap}
              >
                {sharedSwapSignature
                  ? t('experiment.sharedSwapRetry')
                  : t('experiment.sharedSwap')}
              </Button>
              {sharedSwapSignature ? (
                <span className="text-xs text-emerald-600 dark:text-emerald-400">
                  ✓ {sharedSwapSignature.slice(0, 8)}…
                </span>
              ) : null}
            </div>
          ) : fundingReady ? (
            <span className="text-xs text-emerald-600 dark:text-emerald-400">
              {t('experiment.fullyFunded')}
            </span>
          ) : null}
        </div>
      ) : null}

      {swapError ? <InlineError>{swapError}</InlineError> : null}

      <div className="space-y-2">
        <div className="text-sm font-medium">{t('experiment.perArmBudget')}</div>
        {armRows.map((row) => {
          const status = armFundingStatus(row)
          const showManualBudget = allocationMode === 'fixed_usd'
          return (
            <div
              key={row.arm.id}
              className="rounded-md border border-border px-3 py-2 flex flex-wrap gap-x-4 gap-y-2 items-center text-sm"
            >
              <span className="font-medium min-w-[6rem]">{row.arm.label}</span>
              {showManualBudget ? (
                <input
                  type="number"
                  min={0}
                  step="0.01"
                  className="w-24 rounded border border-input bg-background px-2 py-1 text-sm"
                  value={row.arm.budgetUsd}
                  onChange={(e) => onArmBudgetChange(row.arm.id, readBudgetInput(e.target.value))}
                />
              ) : (
                <span>{formatUSD(row.budgetUsd)}</span>
              )}
              {isBelowMinArmBudget(row.budgetUsd) ? (
                <span className="text-xs text-amber-600 dark:text-amber-400">
                  {t('experiment.belowMinArm').replace('{min}', String(DEFAULT_MIN_ARM_BUDGET_USD))}
                </span>
              ) : null}
              {status === 'loading' ? (
                <span className="text-xs text-muted-foreground">{t('experiment.quoteLoading')}</span>
              ) : null}
              {status === 'error' ? (
                <span className="text-xs text-red-600">{row.quoteError?.message ?? 'quote error'}</span>
              ) : null}
              {row.quote ? (
                <span className="text-xs text-muted-foreground font-mono">
                  need {tokenA.symbol} {row.quote.amount_a_ui.toFixed(4)} + {tokenB.symbol}{' '}
                  {row.quote.amount_b_ui.toFixed(2)}
                </span>
              ) : null}
              {status === 'funded' ? (
                <span className="text-xs text-emerald-600">✓</span>
              ) : null}
              {status === 'short' ? (
                <>
                  <span className="text-xs text-amber-600">⚠</span>
                  <Button
                    type="button"
                    size="sm"
                    variant="ghost"
                    disabled={armSwapBusyId === row.arm.id}
                    onClick={() => runArmSwap(row)}
                  >
                    {t('experiment.armSwap')}
                  </Button>
                </>
              ) : null}
            </div>
          )
        })}
      </div>
    </div>
  )
}

export type { ArmBudgetQuote, QuoteOpenBudgetResponse }
