import { useMemo } from 'react'
import { useQuery } from '@tanstack/react-query'
import { getOrcaToken, getPool, getPoolState } from '@/lib/api'
import { shortenAddress } from '@/lib/utils'

export function useArmPool(poolAddress: string) {
  const trimmed = poolAddress.trim()

  const poolQ = useQuery({
    queryKey: ['pool', trimmed],
    queryFn: () => getPool(trimmed),
    enabled: trimmed.length > 0,
    staleTime: 60_000,
  })

  const poolStateQ = useQuery({
    queryKey: ['pool-state', trimmed],
    queryFn: () => getPoolState(trimmed),
    enabled: trimmed.length > 0,
    staleTime: 0,
    refetchInterval: 10_000,
  })

  const tokenAMint = poolQ.data?.token_mint_a
  const tokenBMint = poolQ.data?.token_mint_b

  const orcaAQ = useQuery({
    queryKey: ['orca-token', tokenAMint],
    queryFn: () => getOrcaToken(tokenAMint!),
    enabled: Boolean(tokenAMint),
    staleTime: 300_000,
  })

  const orcaBQ = useQuery({
    queryKey: ['orca-token', tokenBMint],
    queryFn: () => getOrcaToken(tokenBMint!),
    enabled: Boolean(tokenBMint),
    staleTime: 300_000,
  })

  const tokenA = useMemo(() => {
    if (!tokenAMint) return null
    return {
      mint: tokenAMint,
      symbol: orcaAQ.data?.symbol ?? shortenAddress(tokenAMint, 4),
      decimals: orcaAQ.data?.decimals ?? 9,
    }
  }, [tokenAMint, orcaAQ.data])

  const tokenB = useMemo(() => {
    if (!tokenBMint) return null
    return {
      mint: tokenBMint,
      symbol: orcaBQ.data?.symbol ?? shortenAddress(tokenBMint, 4),
      decimals: orcaBQ.data?.decimals ?? 6,
    }
  }, [tokenBMint, orcaBQ.data])

  const poolCurrentTick =
    poolStateQ.data?.current_tick ?? poolQ.data?.current_tick ?? undefined
  const tickSpacing = poolQ.data?.tick_spacing
  const poolPriceRaw = Number(poolStateQ.data?.price ?? poolQ.data?.price ?? Number.NaN)

  const poolReady = Boolean(poolQ.data && trimmed)
  const pairLabel =
    tokenA && tokenB
      ? `${tokenA.symbol}/${tokenB.symbol}${poolCurrentTick != null ? ` · tick ${poolCurrentTick}` : ''}`
      : null

  return {
    pool: poolQ.data,
    poolReady,
    poolLoading: poolQ.isLoading,
    poolError: poolQ.error,
    poolCurrentTick,
    tickSpacing,
    poolPriceRaw,
    tokenA,
    tokenB,
    pairLabel,
  }
}
