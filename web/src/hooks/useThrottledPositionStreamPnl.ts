import { useEffect, useMemo, useRef, useState } from 'react'
import { getPositionStreamPnL, type PositionStreamPnLResponse } from '@/lib/api'

const DEFAULT_MAX_CONCURRENT = 3

export type StreamPnlFetchState = {
  data?: PositionStreamPnLResponse
  isLoading: boolean
  isFetching: boolean
  error?: unknown
}

/**
 * Fetches stream-pnl for visible rows with a small concurrency cap (PERF-PR8).
 */
export function useThrottledPositionStreamPnl(
  positions: { address: string }[],
  visibleAddresses: Set<string>,
  metricsMode: 'live' | 'settlement_v1',
  maxConcurrent = DEFAULT_MAX_CONCURRENT,
): Map<string, StreamPnlFetchState> {
  const [byAddr, setByAddr] = useState<Map<string, StreamPnlFetchState>>(new Map())
  const inFlightRef = useRef(new Set<string>())
  const requestedRef = useRef(new Set<string>())

  const visibleOrdered = useMemo(() => {
    const out: string[] = []
    for (const p of positions) {
      const a = p.address.trim()
      if (visibleAddresses.has(a)) out.push(a)
    }
    return out
  }, [positions, visibleAddresses])

  const visibleKey = visibleOrdered.join(',')

  useEffect(() => {
    requestedRef.current = new Set()
    inFlightRef.current = new Set()
    setByAddr(new Map())
  }, [metricsMode])

  useEffect(() => {
    let cancelled = false

    const mark = (addr: string, patch: Partial<StreamPnlFetchState>) => {
      setByAddr((prev) => {
        const next = new Map(prev)
        const cur = next.get(addr) ?? { isLoading: false, isFetching: false }
        next.set(addr, { ...cur, ...patch })
        return next
      })
    }

    const pump = async () => {
      if (cancelled || inFlightRef.current.size >= maxConcurrent) return
      const next = visibleOrdered.find((a) => !requestedRef.current.has(a))
      if (!next) return
      requestedRef.current.add(next)
      inFlightRef.current.add(next)
      mark(next, { isLoading: true, isFetching: true })
      try {
        const data = await getPositionStreamPnL(next, metricsMode)
        if (!cancelled) mark(next, { data, isLoading: false, isFetching: false, error: undefined })
      } catch (error) {
        if (!cancelled) mark(next, { isLoading: false, isFetching: false, error })
      } finally {
        inFlightRef.current.delete(next)
        if (!cancelled) pump()
      }
    }

    for (let i = 0; i < maxConcurrent; i++) {
      void pump()
    }

    return () => {
      cancelled = true
    }
  }, [visibleKey, metricsMode, maxConcurrent, visibleOrdered])

  return byAddr
}
