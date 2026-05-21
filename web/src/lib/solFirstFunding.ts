import type { WalletEffectiveBalancesResponse } from '@/lib/api'

/** Wrapped SOL mint — Orca pool leg; operators fund via native SOL + pre-wrap (SOL-first). */
export const WSOL_MINT = 'So11111111111111111111111111111111111111112'

/** Estimated ATA rent when WSOL token account must be created before open. */
export const WSOL_ATA_RENT_LAMPORTS_EST = 2_039_280

export function getSplTokenUi(
  mint: string,
  balances: WalletEffectiveBalancesResponse | undefined,
): number {
  if (!balances) return 0
  const row = balances.tokens.find((t) => t.mint === mint)
  if (!row) return 0
  const v = parseFloat(row.ui_amount)
  return Number.isFinite(v) ? v : 0
}

export function hasTokenAccount(
  mint: string,
  balances: WalletEffectiveBalancesResponse | undefined,
): boolean {
  if (!balances) return false
  return balances.tokens.some((t) => t.mint === mint)
}

export type SolFirstFundingInput = {
  balances: WalletEffectiveBalancesResponse | undefined
  tokenAMint: string
  tokenBMint: string
  /** From `/wallets/api-signer` — native SOL reserve for tx/rent. */
  minOpenLamports?: number
  /** WSOL leg deposit need (UI) — gates ATA rent estimate like `PositionCreate`. */
  needSolLegUi?: number
}

export type SolFirstFundingBalances = {
  nativeSolUi: number
  splWsolUi: number
  haveA: number
  haveB: number
  effectiveHaveA: number
  effectiveHaveB: number
  /** Wallet line: native SOL for WSOL mint leg, SPL otherwise. */
  walletDisplayA: number
  walletDisplayB: number
  nativeSolAvailableForWrapUi: number
}

/**
 * SOL-first funding balances aligned with `PositionCreate` fundingCheck:
 * WSOL pool leg counts spendable native SOL (minus open pad / ATA rent), not SPL WSOL alone.
 */
export function computeSolFirstFundingBalances(input: SolFirstFundingInput): SolFirstFundingBalances {
  const { balances, tokenAMint, tokenBMint, minOpenLamports = 0, needSolLegUi = 0 } = input

  const nativeParsed = balances ? parseFloat(balances.sol) : Number.NaN
  const nativeSolUi = Number.isFinite(nativeParsed) ? nativeParsed : 0
  const minOpenSolUi = minOpenLamports / 1e9

  const haveA = getSplTokenUi(tokenAMint, balances)
  const haveB = getSplTokenUi(tokenBMint, balances)
  const splWsolUi = getSplTokenUi(WSOL_MINT, balances)

  const wsolAccountExists = hasTokenAccount(WSOL_MINT, balances)
  const wsolAtaRentUi =
    !wsolAccountExists && needSolLegUi > 0 ? WSOL_ATA_RENT_LAMPORTS_EST / 1e9 : 0

  const nativeSolAvailableForWrapUi = Math.max(0, nativeSolUi - minOpenSolUi - wsolAtaRentUi)

  const effectiveHaveA =
    tokenAMint === WSOL_MINT ? Math.max(haveA, nativeSolAvailableForWrapUi) : haveA
  const effectiveHaveB =
    tokenBMint === WSOL_MINT ? Math.max(haveB, nativeSolAvailableForWrapUi) : haveB

  const walletDisplayA = tokenAMint === WSOL_MINT ? nativeSolUi : haveA
  const walletDisplayB = tokenBMint === WSOL_MINT ? nativeSolUi : haveB

  return {
    nativeSolUi,
    splWsolUi,
    haveA,
    haveB,
    effectiveHaveA,
    effectiveHaveB,
    walletDisplayA,
    walletDisplayB,
    nativeSolAvailableForWrapUi,
  }
}
