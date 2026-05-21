import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Link } from 'react-router-dom'
import { ArrowLeft, ChevronDown, ChevronUp, FlaskConical } from 'lucide-react'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { InlineError } from '@/components/ui/inline-error'
import ExperimentAddStrategySlot from '@/components/ExperimentAddStrategySlot'
import ExperimentCapitalStep, { type ArmBudgetQuote } from '@/components/ExperimentCapitalStep'
import ExperimentFlowSteps from '@/components/ExperimentFlowSteps'
import ExperimentStickyFooter from '@/components/ExperimentStickyFooter'
import ExperimentStrategyRoster from '@/components/ExperimentStrategyRoster'
import ExperimentStrategyCard from '@/components/ExperimentStrategyCard'
import ExperimentStrategyPickerModal from '@/components/ExperimentStrategyPickerModal'
import {
  createStrategy,
  getApiSignerWallet,
  getStrategies,
  getWalletEffectiveBalances,
  getWallets,
  openPosition,
  swapBeforeOpen,
  type StrategyParameters,
  type StrategyType,
  type Strategy,
} from '@/lib/api'
import { computeSolFirstFundingBalances, WSOL_MINT } from '@/lib/solFirstFunding'
import {
  areExperimentArmsValid,
  canAddExperimentArm,
  resolveExperimentCommonPool,
  type ExperimentArm,
} from '@/lib/experimentArm'
import { isArmParamsDirty } from '@/lib/experimentArmDirty'
import {
  armTokenNeedsFromQuotes,
  quotesReady,
  resolveArmBudgetsUsd,
} from '@/lib/experimentBudgetPlan'
import { MAX_EXPERIMENT_ARMS, type AllocationMode } from '@/lib/experimentCapital'
import { buildExperimentFundingPlan } from '@/lib/experimentFundingPlan'
import { launchExperiment, type LaunchExperimentResult } from '@/lib/experimentLaunch'
import {
  buildExperimentArmLaunchSpecs,
  makeCostSessionId,
  makeExperimentBatchId,
  persistExperimentBatch,
} from '@/lib/experimentLaunchSpecs'
import { getDevWalletPubkey } from '@/lib/devWallet'
import { useArmPool } from '@/hooks/useArmPool'
import { useI18n } from '@/lib/i18n'
import { shortenAddress } from '@/lib/utils'

const LS_SELECTED_WALLET_ID = 'clmm.selected_wallet_id'

export default function ExperimentLaunch() {
  const { t } = useI18n()
  const [arms, setArms] = useState<ExperimentArm[]>([])
  const [picker, setPicker] = useState<
    | { mode: 'insert'; index: number }
    | { mode: 'replace'; armId: string }
    | null
  >(null)
  const [selectedArmId, setSelectedArmId] = useState<string | null>(null)
  const [capitalOpen, setCapitalOpen] = useState(true)
  const [launchOpen, setLaunchOpen] = useState(false)
  const [totalBudgetUsd, setTotalBudgetUsd] = useState<number | ''>(30)
  const [allocationMode, setAllocationMode] = useState<AllocationMode>('equal')
  const [capitalStepOk, setCapitalStepOk] = useState(false)
  const [fundingReady, setFundingReady] = useState(false)
  const [sharedSwapSignature, setSharedSwapSignature] = useState<string | null>(null)
  const [sharedCostSessionId, setSharedCostSessionId] = useState<string | null>(null)
  const [launchArmRows, setLaunchArmRows] = useState<ArmBudgetQuote[]>([])
  const [launchResult, setLaunchResult] = useState<LaunchExperimentResult | null>(null)
  const [launchError, setLaunchError] = useState<string | null>(null)
  const [launching, setLaunching] = useState(false)
  const [batchId, setBatchId] = useState<string | null>(null)
  const [pipelineSwap, setPipelineSwap] = useState({
    sharedSwapNeeded: false,
    sharedSwapDone: true,
    shortA: false,
    shortB: false,
    shortBoth: false,
  })

  const capitalRef = useRef<HTMLDivElement>(null)
  const queryClient = useQueryClient()
  const devPk = getDevWalletPubkey()

  const strategiesQ = useQuery({
    queryKey: ['strategies'],
    queryFn: getStrategies,
    staleTime: 30_000,
  })

  const enabledArmCount = useMemo(() => arms.filter((a) => a.enabled).length, [arms])

  useEffect(() => {
    if (arms.length === 0) {
      setSelectedArmId(null)
      return
    }
    if (!selectedArmId || !arms.some((a) => a.id === selectedArmId)) {
      setSelectedArmId(arms[arms.length - 1]!.id)
    }
  }, [arms, selectedArmId])

  const selectedArmIndex = useMemo(
    () => arms.findIndex((a) => a.id === selectedArmId),
    [arms, selectedArmId],
  )
  const selectedArm = selectedArmIndex >= 0 ? arms[selectedArmIndex] : null

  const enabledArms = useMemo(() => arms.filter((a) => a.enabled), [arms])
  const commonPoolAddress = useMemo(() => resolveExperimentCommonPool(enabledArms), [enabledArms])
  const launchPool = useArmPool(commonPoolAddress ?? '')
  const poolsMismatch = useMemo(() => {
    const withPool = enabledArms.filter((a) => a.poolAddress.trim())
    return withPool.length > 0 && !commonPoolAddress
  }, [enabledArms, commonPoolAddress])

  const tokenA = launchPool.tokenA
  const tokenB = launchPool.tokenB
  const poolPriceRaw = launchPool.poolPriceRaw

  const armsStepOk = areExperimentArmsValid(arms, launchPool.poolReady)
  const strategiesReady = arms.length > 0 && armsStepOk && Boolean(commonPoolAddress) && !poolsMismatch

  const updateArm = useCallback((armId: string, next: ExperimentArm) => {
    setArms((prev) => prev.map((a) => (a.id === armId ? next : a)))
  }, [])

  const handleCapitalReady = useCallback((ready: boolean) => {
    setCapitalStepOk(ready)
  }, [])

  const handleFundingReady = useCallback((ready: boolean) => {
    setFundingReady(ready)
  }, [])

  const handleArmBudgetChange = useCallback((armId: string, budgetUsd: number | '') => {
    setArms((prev) => prev.map((a) => (a.id === armId ? { ...a, budgetUsd } : a)))
  }, [])

  const handleArmRowsChange = useCallback((rows: ArmBudgetQuote[]) => {
    setLaunchArmRows(rows)
  }, [])

  const budgetByArmId = useMemo(
    () => resolveArmBudgetsUsd(arms, totalBudgetUsd, allocationMode),
    [arms, totalBudgetUsd, allocationMode],
  )

  const strategiesById = useMemo(() => {
    const map = new Map<string, Strategy>()
    for (const s of strategiesQ.data?.strategies ?? []) {
      map.set(s.id, s)
    }
    return map
  }, [strategiesQ.data?.strategies])

  const hasUnsavedFormParams = useMemo(
    () =>
      enabledArms.some((a) => {
        const s = a.reuseStrategyId ? strategiesById.get(a.reuseStrategyId) : undefined
        return isArmParamsDirty(a, s)
      }),
    [enabledArms, strategiesById],
  )

  const launchQuotesReady = quotesReady(launchArmRows) && launchArmRows.length === enabledArms.length
  const ticksReady = enabledArms.every((a) => a.tickLower !== '' && a.tickUpper !== '')
  const launchReady =
    capitalStepOk &&
    fundingReady &&
    launchQuotesReady &&
    armsStepOk &&
    ticksReady &&
    pipelineSwap.sharedSwapDone &&
    !hasUnsavedFormParams &&
    !launching

  const footerBlocker = useMemo(() => {
    if (enabledArms.length === 0) return null
    if (enabledArms.some((a) => !a.poolAddress.trim())) return t('experiment.footerNeedPool')
    if (poolsMismatch) return t('experiment.mixedPoolsError')
    if (hasUnsavedFormParams) return t('experiment.footerUnsavedParams')
    if (!ticksReady) return t('experiment.footerNeedTicks')
    if (!launchQuotesReady) return t('experiment.footerNeedQuotes')
    if (pipelineSwap.sharedSwapNeeded && !pipelineSwap.sharedSwapDone) {
      return t('experiment.footerNeedSwap')
    }
    if (!fundingReady) {
      if (pipelineSwap.shortBoth) return t('experiment.deficitBothLegs')
      if (pipelineSwap.shortA && pipelineSwap.shortB) return t('experiment.deficitBothLegs')
      if (pipelineSwap.shortA) return t('experiment.footerShortSol')
      if (pipelineSwap.shortB) return t('experiment.footerShortUsdc')
      return t('experiment.capitalNotReady')
    }
    if (!capitalStepOk) return t('experiment.capitalNotReady')
    return null
  }, [
    enabledArms,
    ticksReady,
    launchQuotesReady,
    pipelineSwap,
    capitalStepOk,
    fundingReady,
    poolsMismatch,
    hasUnsavedFormParams,
    t,
  ])

  const walletsQ = useQuery({ queryKey: ['wallets'], queryFn: getWallets, staleTime: 30_000 })
  const apiSignerQ = useQuery({
    queryKey: ['api-signer-wallet'],
    queryFn: getApiSignerWallet,
    staleTime: 20_000,
  })

  const effectiveOwnerPk = useMemo(() => {
    if (typeof window === 'undefined') return devPk
    const id = window.localStorage.getItem(LS_SELECTED_WALLET_ID) || ''
    const picked = walletsQ.data?.wallets.find((w) => w.id === id)
    return (
      apiSignerQ.data?.pubkey?.trim() ||
      picked?.pubkey ||
      devPk ||
      walletsQ.data?.wallets[0]?.pubkey ||
      null
    )
  }, [walletsQ.data?.wallets, apiSignerQ.data?.pubkey, devPk])

  async function resolveOwnerPk(): Promise<string | null> {
    if (effectiveOwnerPk) return effectiveOwnerPk
    try {
      const signer = await getApiSignerWallet()
      if (signer.pubkey?.trim()) return signer.pubkey.trim()
    } catch {
      // ignore
    }
    const wallets = walletsQ.data ?? (await getWallets())
    if (typeof window !== 'undefined') {
      const id = window.localStorage.getItem(LS_SELECTED_WALLET_ID) || ''
      const picked = wallets.wallets.find((w) => w.id === id)
      if (picked?.pubkey) return picked.pubkey
    }
    return devPk ?? wallets.wallets[0]?.pubkey ?? null
  }

  async function handleLaunchAll() {
    if (!capitalUiReady || !tokenA || !tokenB || !launchReady || !commonPoolAddress) return

    setLaunching(true)
    setLaunchError(null)
    setLaunchOpen(true)

    const nextBatchId = batchId ?? makeExperimentBatchId()
    if (!batchId) setBatchId(nextBatchId)

    try {
      const quotesByArmId = new Map(
        launchArmRows.filter((r) => r.quote).map((r) => [r.arm.id, r.quote!]),
      )
      const specs = buildExperimentArmLaunchSpecs(
        arms,
        budgetByArmId,
        quotesByArmId,
        nextBatchId,
      )

      const ownerPk = await resolveOwnerPk()

      let effectiveHaveA = 0
      let effectiveHaveB = 0
      let freshBalances = null as Awaited<ReturnType<typeof getWalletEffectiveBalances>> | null
      if (ownerPk) {
        freshBalances = await getWalletEffectiveBalances(ownerPk, { force: true })
        queryClient.setQueryData(['wallet-balances', ownerPk], freshBalances)
      }

      const needs = armTokenNeedsFromQuotes(launchArmRows)
      let needSolLegUi = 0
      for (const row of launchArmRows) {
        if (!row.quote) continue
        if (tokenA.mint === WSOL_MINT) {
          needSolLegUi += row.quote.amount_a_ui
        } else if (tokenB.mint === WSOL_MINT) {
          needSolLegUi += row.quote.amount_b_ui
        }
      }

      let minOpenLamports = apiSignerQ.data?.min_open_lamports ?? 0
      if (!minOpenLamports) {
        try {
          minOpenLamports = (await getApiSignerWallet()).min_open_lamports ?? 0
        } catch {
          // ignore
        }
      }

      if (freshBalances) {
        const solFirst = computeSolFirstFundingBalances({
          balances: freshBalances,
          tokenAMint: tokenA.mint,
          tokenBMint: tokenB.mint,
          minOpenLamports,
          needSolLegUi,
        })
        effectiveHaveA = solFirst.effectiveHaveA
        effectiveHaveB = solFirst.effectiveHaveB
      }

      const fundingPlan = buildExperimentFundingPlan({
        arms: needs,
        decimalsA: tokenA.decimals,
        decimalsB: tokenB.decimals,
        mintA: tokenA.mint,
        mintB: tokenB.mint,
        symbolA: tokenA.symbol,
        symbolB: tokenB.symbol,
        haveAUi: effectiveHaveA,
        haveBUi: effectiveHaveB,
        poolPriceRaw: Number.isFinite(poolPriceRaw) ? poolPriceRaw : undefined,
      })

      const sharedSwap =
        fundingPlan.recommended_swap && !sharedSwapSignature ? fundingPlan.recommended_swap : null

      const result = await launchExperiment(
        {
          poolAddress: commonPoolAddress,
          arms: specs,
          sharedSwap,
          sharedSwapSignature: sharedSwapSignature ?? undefined,
        },
        {
          generateSessionId: makeCostSessionId,
          generateBatchId: () => nextBatchId,
          createStrategy: async (payload) => {
            const created = await createStrategy({
              name: payload.name,
              strategy_type: payload.strategy_type as StrategyType,
              parameters: payload.parameters as StrategyParameters,
              auto_execute: payload.auto_execute,
              dry_run: payload.dry_run,
            })
            return { id: created.id }
          },
          swapBeforeOpen,
          openPosition,
          getFundingStatus: async () => {
            if (!ownerPk) {
              return { shortA: false, shortB: false, deficitA: 0, deficitB: 0 }
            }
            const bal = await getWalletEffectiveBalances(ownerPk, { force: true })
            const solFirst = computeSolFirstFundingBalances({
              balances: bal,
              tokenAMint: tokenA.mint,
              tokenBMint: tokenB.mint,
              minOpenLamports,
              needSolLegUi,
            })
            const plan = buildExperimentFundingPlan({
              arms: needs,
              decimalsA: tokenA.decimals,
              decimalsB: tokenB.decimals,
              mintA: tokenA.mint,
              mintB: tokenB.mint,
              symbolA: tokenA.symbol,
              symbolB: tokenB.symbol,
              haveAUi: solFirst.effectiveHaveA,
              haveBUi: solFirst.effectiveHaveB,
              poolPriceRaw: Number.isFinite(poolPriceRaw) ? poolPriceRaw : undefined,
            })
            return {
              shortA: plan.deficits.short_a,
              shortB: plan.deficits.short_b,
              deficitA: plan.deficits.deficit_a_ui,
              deficitB: plan.deficits.deficit_b_ui,
            }
          },
        },
      )

      if (result.sharedSwapSignature) {
        setSharedSwapSignature(result.sharedSwapSignature)
      }

      setLaunchResult(result)
      persistExperimentBatch({
        batchId: result.batchId,
        poolAddress: commonPoolAddress,
        createdAt: new Date().toISOString(),
        sharedSwapSignature: result.sharedSwapSignature ?? sharedSwapSignature ?? undefined,
        arms: result.arms.map((r) => {
          const arm = arms.find((a) => a.id === r.armId)
          return {
            armId: r.armId,
            label: arm?.label ?? r.armId,
            status: r.status,
            positionPda: r.positionPda,
            strategyId: r.strategyId,
            costSessionId: r.costSessionId,
            error: r.error,
          }
        }),
      })

      await queryClient.invalidateQueries({ queryKey: ['strategies'] })
      await queryClient.invalidateQueries({ queryKey: ['positions'] })
    } catch (e) {
      setLaunchError(e instanceof Error ? e.message : String(e))
    } finally {
      setLaunching(false)
    }
  }

  function launchStatusLabel(status: string): string {
    switch (status) {
      case 'opened':
        return t('experiment.launchStatusOpened')
      case 'failed':
        return t('experiment.launchStatusFailed')
      case 'skipped':
        return t('experiment.launchStatusSkipped')
      default:
        return t('experiment.launchStatusPending')
    }
  }

  const handleSharedSwapComplete = useCallback((sig: string | null, sessionId: string | null) => {
    setSharedSwapSignature(sig)
    if (sessionId) setSharedCostSessionId(sessionId)
  }, [])

  function openInsertPicker(insertIndex: number) {
    if (!canAddExperimentArm(arms)) return
    setPicker({ mode: 'insert', index: insertIndex })
  }

  function openReplacePicker(armId: string) {
    setPicker({ mode: 'replace', armId })
  }

  function applyPickedArm(arm: ExperimentArm) {
    if (!picker) return
    const defaultPool =
      arms.find((a) => a.poolAddress.trim())?.poolAddress.trim() ?? ''
    const withPool = arm.poolAddress.trim()
      ? arm
      : { ...arm, poolAddress: defaultPool }
    if (picker.mode === 'insert') {
      const nextArm = { ...withPool, label: withPool.label || `Arm ${picker.index + 1}` }
      setArms((prev) => {
        const next = [...prev]
        next.splice(picker.index, 0, nextArm)
        return next
      })
      setSelectedArmId(nextArm.id)
    } else {
      setArms((prev) =>
        prev.map((a) =>
          a.id === picker.armId
            ? { ...withPool, id: picker.armId, label: withPool.label || a.label }
            : a,
        ),
      )
      setSelectedArmId(picker.armId)
    }
    setPicker(null)
  }

  function duplicateArm(armId: string) {
    if (!canAddExperimentArm(arms)) return
    setArms((prev) => {
      const idx = prev.findIndex((a) => a.id === armId)
      if (idx < 0) return prev
      const src = prev[idx]!
      const copy: ExperimentArm = {
        ...src,
        id: crypto.randomUUID(),
        label: `${src.label} (${t('experiment.copySuffix')})`,
      }
      const next = [...prev]
      next.splice(idx + 1, 0, copy)
      setSelectedArmId(copy.id)
      return next
    })
  }

  function removeArm(id: string) {
    setArms((prev) => prev.filter((a) => a.id !== id))
    setPicker(null)
    if (selectedArmId === id) setSelectedArmId(null)
  }

  const capitalUiReady =
    Boolean(commonPoolAddress) &&
    launchPool.poolReady &&
    tokenA &&
    tokenB &&
    !poolsMismatch
  const canAddMore = canAddExperimentArm(arms) && picker == null

  const addDisabledReason = !canAddExperimentArm(arms)
    ? t('experiment.maxArmsReached').replace('{max}', String(MAX_EXPERIMENT_ARMS))
    : undefined

  const pickerLabel = !picker
    ? ''
    : picker.mode === 'replace'
      ? t('experiment.pickStrategyReplace')
      : t('experiment.pickStrategyInsert').replace('{n}', String(picker.index + 1))

  const poolDisplayLabel = launchPool.pairLabel

  const scrollToCapital = () => {
    setCapitalOpen(true)
    capitalRef.current?.scrollIntoView({ behavior: 'smooth', block: 'start' })
  }

  return (
    <div className="space-y-6 max-w-6xl mx-auto pb-32">
      <div className="space-y-4">
        <div className="flex items-center gap-4">
          <Link to="/positions">
            <Button variant="ghost" size="icon" aria-label={t('experiment.backToPositions')}>
              <ArrowLeft className="h-4 w-4" />
            </Button>
          </Link>
          <div className="min-w-0">
            <h1 className="text-2xl sm:text-3xl font-bold flex items-center gap-2">
              <FlaskConical className="h-7 w-7 sm:h-8 sm:w-8 text-primary shrink-0" />
              <span className="truncate">{t('experiment.title')}</span>
            </h1>
            <p className="text-sm text-muted-foreground mt-1">{t('experiment.subtitleStack')}</p>
          </div>
        </div>
        <ExperimentFlowSteps
          strategiesCount={arms.length}
          strategiesReady={strategiesReady}
          capitalReady={capitalStepOk}
          launchReady={Boolean(launchResult?.arms.every((a) => a.status === 'opened'))}
        />
      </div>

      <section className="space-y-4">
        {arms.length === 0 ? (
          <ExperimentAddStrategySlot
            size="hero"
            currentCount={0}
            onClick={() => openInsertPicker(0)}
            disabled={!canAddMore}
            disabledReason={addDisabledReason}
          />
        ) : (
          <>
            <div className="sticky top-0 z-20 -mx-1 px-1 pt-1 pb-3 bg-background/95 backdrop-blur border-b border-border/50 supports-[backdrop-filter]:bg-background/80">
              <ExperimentStrategyRoster
                arms={arms}
                strategies={strategiesQ.data?.strategies ?? []}
                selectedArmId={selectedArmId}
                totalBudgetUsd={totalBudgetUsd}
                allocationMode={allocationMode}
                budgetByArmId={budgetByArmId}
                onTotalBudgetChange={setTotalBudgetUsd}
                onAllocationModeChange={setAllocationMode}
                onSelectArm={setSelectedArmId}
                onArmChange={updateArm}
                onRemoveArm={removeArm}
                onAdd={() => openInsertPicker(arms.length)}
                canAddMore={canAddMore}
                addDisabledReason={addDisabledReason}
              />
            </div>

            {selectedArm && selectedArmIndex >= 0 ? (
              <div className="space-y-2">
                <div>
                  <h2 className="text-sm font-semibold">{t('experiment.rosterDetailTitle')}</h2>
                  <p className="text-xs text-muted-foreground mt-0.5">{t('experiment.rosterDetailHint')}</p>
                </div>
                <ExperimentStrategyCard
                  arm={selectedArm}
                  index={selectedArmIndex}
                  strategies={strategiesQ.data?.strategies ?? []}
                  defaultEditOpen
                  allocationMode={allocationMode}
                  resolvedBudgetUsd={budgetByArmId.get(selectedArm.id)}
                  onArmBudgetChange={(budgetUsd) => handleArmBudgetChange(selectedArm.id, budgetUsd)}
                  onChange={(next) => updateArm(selectedArm.id, next)}
                  onRemove={() => removeArm(selectedArm.id)}
                  onDuplicate={() => duplicateArm(selectedArm.id)}
                  onChangeStrategy={() => openReplacePicker(selectedArm.id)}
                  canRemove
                />
              </div>
            ) : null}
          </>
        )}

        {!armsStepOk && arms.length > 0 ? (
          <InlineError className="mt-2">{t('experiment.armsInvalid')}</InlineError>
        ) : null}
        {poolsMismatch ? (
          <InlineError className="mt-2">{t('experiment.mixedPoolsError')}</InlineError>
        ) : null}
      </section>

      {arms.length > 0 && commonPoolAddress ? (
        <>
          <div ref={capitalRef}>
            <Card>
              <CardHeader className="cursor-pointer" onClick={() => setCapitalOpen((o) => !o)}>
                <div className="flex items-center justify-between gap-2">
                  <div>
                    <CardTitle className="text-base">{t('experiment.step3Title')}</CardTitle>
                    <CardDescription>{t('experiment.step3Desc')}</CardDescription>
                  </div>
                  <Button type="button" variant="ghost" size="icon" tabIndex={-1}>
                    {capitalOpen ? <ChevronUp className="h-4 w-4" /> : <ChevronDown className="h-4 w-4" />}
                  </Button>
                </div>
              </CardHeader>
              {capitalOpen && capitalUiReady ? (
                <CardContent>
                  <ExperimentCapitalStep
                    poolAddress={commonPoolAddress}
                    poolPriceRaw={Number.isFinite(poolPriceRaw) ? poolPriceRaw : undefined}
                    tokenA={tokenA!}
                    tokenB={tokenB!}
                    arms={arms}
                    totalBudgetUsd={totalBudgetUsd}
                    allocationMode={allocationMode}
                    onTotalBudgetChange={setTotalBudgetUsd}
                    onAllocationModeChange={setAllocationMode}
                    onArmBudgetChange={handleArmBudgetChange}
                    sharedSwapSignature={sharedSwapSignature}
                    sharedCostSessionId={sharedCostSessionId}
                    onSharedSwapComplete={handleSharedSwapComplete}
                    onReadyChange={handleCapitalReady}
                    onFundingReadyChange={handleFundingReady}
                    onArmRowsChange={handleArmRowsChange}
                    onPipelineChange={setPipelineSwap}
                    hideBudgetControls
                  />
                  {!capitalStepOk ? (
                    <InlineError className="mt-3">{t('experiment.capitalNotReady')}</InlineError>
                  ) : null}
                </CardContent>
              ) : null}
            </Card>
          </div>

          <Card>
            <CardHeader className="cursor-pointer" onClick={() => setLaunchOpen((o) => !o)}>
              <div className="flex items-center justify-between gap-2">
                <div>
                  <CardTitle className="text-base">{t('experiment.step4Title')}</CardTitle>
                  <CardDescription>{t('experiment.step4Desc')}</CardDescription>
                </div>
                <Button type="button" variant="ghost" size="icon" tabIndex={-1}>
                  {launchOpen ? <ChevronUp className="h-4 w-4" /> : <ChevronDown className="h-4 w-4" />}
                </Button>
              </div>
            </CardHeader>
            {launchOpen ? (
              <CardContent className="space-y-4">
                {launchResult ? (
                  <div className="space-y-2">
                    <div className="text-sm font-medium">{t('experiment.launchProgress')}</div>
                    {launchResult.aborted ? (
                      <InlineError>
                        {t('experiment.launchAbort').replace('{reason}', launchResult.abortReason ?? '—')}
                      </InlineError>
                    ) : (
                      <p className="text-xs text-muted-foreground">{t('experiment.launchComplete')}</p>
                    )}
                    <div className="overflow-x-auto rounded-lg border border-border">
                      <table className="w-full text-sm border-collapse">
                        <thead>
                          <tr className="border-b border-border text-left text-xs text-muted-foreground bg-muted/20">
                            <th className="py-2 px-3">Arm</th>
                            <th className="py-2 px-3">Status</th>
                            <th className="py-2 px-3">PDA</th>
                            <th className="py-2 px-3">Error</th>
                          </tr>
                        </thead>
                        <tbody>
                          {launchResult.arms.map((row) => {
                            const arm = arms.find((a) => a.id === row.armId)
                            return (
                              <tr key={row.armId} className="border-b border-border/60">
                                <td className="py-2 px-3 font-medium">{arm?.label ?? row.armId.slice(0, 8)}</td>
                                <td className="py-2 px-3">{launchStatusLabel(row.status)}</td>
                                <td className="py-2 px-3 font-mono text-xs">
                                  {row.positionPda ? (
                                    <Link
                                      to={`/positions/${encodeURIComponent(row.positionPda)}`}
                                      className="text-primary hover:underline"
                                    >
                                      {shortenAddress(row.positionPda, 4)}
                                    </Link>
                                  ) : (
                                    '—'
                                  )}
                                </td>
                                <td className="py-2 px-3 text-xs text-red-600">{row.error ?? ''}</td>
                              </tr>
                            )
                          })}
                        </tbody>
                      </table>
                    </div>
                  </div>
                ) : (
                  <p className="text-sm text-muted-foreground">{t('experiment.launchPanelHint')}</p>
                )}
                {launchError ? <InlineError>{launchError}</InlineError> : null}
              </CardContent>
            ) : null}
          </Card>

          <ExperimentStickyFooter
            strategyCount={enabledArmCount}
            budgetUsd={totalBudgetUsd}
            capitalReady={capitalStepOk}
            launchReady={launchReady}
            launching={launching}
            poolLabel={poolDisplayLabel}
            blockerHint={footerBlocker}
            onLaunch={handleLaunchAll}
            onScrollToCapital={scrollToCapital}
          />
        </>
      ) : null}

      <ExperimentStrategyPickerModal
        open={picker != null}
        strategies={strategiesQ.data?.strategies ?? []}
        insertLabel={pickerLabel}
        onSelectArm={applyPickedArm}
        onCancel={() => setPicker(null)}
      />
    </div>
  )
}
