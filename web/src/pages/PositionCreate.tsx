import { useEffect, useMemo, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useNavigate, Link } from 'react-router-dom'
import { ArrowLeft } from 'lucide-react'
import { Card, CardHeader, CardTitle, CardContent } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { getJupiterPricesUsd, getPool, openPosition } from '@/lib/api'

export default function PositionCreate() {
  const navigate = useNavigate()
  const queryClient = useQueryClient()

  const curatedPools = useMemo(
    () => [
      {
        label: 'SOL/USDC (0.04%)',
        address: 'Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE',
      },
      {
        label: 'whETH/SOL (0.05%)',
        address: 'HktfL7iwGKT5QHjywQkcDnZXScoh811k7akrMZJkCcEF',
      },
      {
        label: 'cbBTC/USDC (0.04%)',
        address: 'HxA6SKW5qA4o12fjVgTpXdq2YnZ5Zv1s7SB4FFomsyLM',
      },
    ],
    [],
  )

  const [poolAddress, setPoolAddress] = useState('')
  const [tickLower, setTickLower] = useState<number | ''>('')
  const [tickUpper, setTickUpper] = useState<number | ''>('')

  // Human units in the UI
  const [amountAUi, setAmountAUi] = useState<number | ''>('')
  const [amountBUi, setAmountBUi] = useState<number | ''>('')

  // Budget split mode (USD)
  const [mode, setMode] = useState<'tokens' | 'budget'>('tokens')
  const [totalUsd, setTotalUsd] = useState<number | ''>('')
  const [splitPctA, setSplitPctA] = useState<number>(50)

  const poolQ = useQuery({
    queryKey: ['pool', poolAddress],
    queryFn: () => getPool(poolAddress.trim()),
    enabled: poolAddress.trim().length > 0,
    staleTime: 60_000,
  })

  const tokenA = poolQ.data?.token_a
  const tokenB = poolQ.data?.token_b

  const pricesQ = useQuery({
    queryKey: ['jupiter-prices', tokenA?.mint, tokenB?.mint],
    queryFn: async () => {
      const mints = [tokenA?.mint, tokenB?.mint].filter(Boolean) as string[]
      return await getJupiterPricesUsd(mints)
    },
    enabled: mode === 'budget' && !!tokenA?.mint && !!tokenB?.mint,
    staleTime: 60_000,
  })

  useEffect(() => {
    if (mode !== 'budget') return
    if (!tokenA || !tokenB) return
    if (totalUsd === '' || !Number.isFinite(totalUsd) || totalUsd <= 0) return
    const pxA = pricesQ.data?.[tokenA.mint]
    const pxB = pricesQ.data?.[tokenB.mint]
    if (!pxA || !pxB) return

    const usdA = (Number(totalUsd) * splitPctA) / 100
    const usdB = Number(totalUsd) - usdA
    const a = usdA / pxA
    const b = usdB / pxB
    setAmountAUi(Number.isFinite(a) ? Number(a.toFixed(8)) : '')
    setAmountBUi(Number.isFinite(b) ? Number(b.toFixed(8)) : '')
  }, [mode, tokenA, tokenB, totalUsd, splitPctA, pricesQ.data])

  const toBaseUnitsU64 = (ui: number, decimals: number): number | null => {
    if (!Number.isFinite(ui) || ui < 0) return null
    const mul = 10 ** decimals
    const raw = Math.round(ui * mul)
    if (!Number.isFinite(raw) || raw < 0) return null
    if (raw > Number.MAX_SAFE_INTEGER) return null
    return raw
  }

  const mutation = useMutation({
    mutationFn: openPosition,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['positions'] })
      navigate('/positions')
    },
  })

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()
    const pool = poolQ.data
    if (
      !poolAddress.trim() ||
      tickLower === '' ||
      tickUpper === '' ||
      amountAUi === '' ||
      amountBUi === '' ||
      !pool
    ) {
      return
    }

    const aRaw = toBaseUnitsU64(Number(amountAUi), pool.token_a.decimals)
    const bRaw = toBaseUnitsU64(Number(amountBUi), pool.token_b.decimals)
    if (aRaw === null || bRaw === null) {
      return
    }

    mutation.mutate({
      pool_address: poolAddress.trim(),
      tick_lower: Number(tickLower),
      tick_upper: Number(tickUpper),
      amount_a: aRaw,
      amount_b: bRaw,
    })
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center gap-4">
        <Link to="/positions">
          <Button variant="ghost" size="icon">
            <ArrowLeft className="h-4 w-4" />
          </Button>
        </Link>
        <h1 className="text-3xl font-bold">Open Position</h1>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Position Configuration</CardTitle>
        </CardHeader>
        <CardContent>
          <form className="space-y-4" onSubmit={handleSubmit}>
            <div>
              <label className="block text-sm font-medium mb-1">Pool (curated)</label>
              <select
                className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                value={poolAddress}
                onChange={(e) => setPoolAddress(e.target.value)}
              >
                <option value="">Wybierz parę…</option>
                {curatedPools.map((p) => (
                  <option key={p.address} value={p.address}>
                    {p.label}
                  </option>
                ))}
              </select>
              <div className="mt-2">
                <label className="block text-xs text-muted-foreground mb-1">Pool Address</label>
                <input
                  className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm font-mono"
                  value={poolAddress}
                  onChange={(e) => setPoolAddress(e.target.value)}
                  placeholder="Whirlpool pool address"
                  required
                />
              </div>
              {poolQ.isLoading ? (
                <div className="text-xs text-muted-foreground mt-2">Ładuję metadane puli…</div>
              ) : poolQ.error ? (
                <div className="text-xs text-destructive mt-2">
                  {(poolQ.error as Error).message}
                </div>
              ) : poolQ.data ? (
                <div className="text-xs text-muted-foreground mt-2">
                  {poolQ.data.protocol.toUpperCase()} · {poolQ.data.token_a.symbol}/
                  {poolQ.data.token_b.symbol} · tick_spacing {poolQ.data.tick_spacing} · fee tier{' '}
                  {poolQ.data.fee_tier}%
                </div>
              ) : null}
            </div>

            <div className="grid gap-4 md:grid-cols-2">
              <div>
                <label className="block text-sm font-medium mb-1">Tick Lower</label>
                <input
                  type="number"
                  className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                  value={tickLower}
                  onChange={(e) => setTickLower(e.target.value === '' ? '' : Number(e.target.value))}
                  required
                />
              </div>
              <div>
                <label className="block text-sm font-medium mb-1">Tick Upper</label>
                <input
                  type="number"
                  className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                  value={tickUpper}
                  onChange={(e) => setTickUpper(e.target.value === '' ? '' : Number(e.target.value))}
                  required
                />
              </div>
            </div>

            <div className="rounded-md border border-border p-3 space-y-3">
              <div className="flex flex-wrap gap-3 items-center">
                <span className="text-sm font-medium">Kwota</span>
                <label className="text-sm flex items-center gap-2">
                  <input
                    type="radio"
                    name="mode"
                    value="tokens"
                    checked={mode === 'tokens'}
                    onChange={() => setMode('tokens')}
                  />
                  Token A/B (ręcznie)
                </label>
                <label className="text-sm flex items-center gap-2">
                  <input
                    type="radio"
                    name="mode"
                    value="budget"
                    checked={mode === 'budget'}
                    onChange={() => setMode('budget')}
                  />
                  Wspólna kwota USD do rozdziału
                </label>
              </div>

              {mode === 'budget' && (
                <div className="grid gap-4 md:grid-cols-3">
                  <div>
                    <label className="block text-sm font-medium mb-1">Total USD</label>
                    <input
                      type="number"
                      step="0.01"
                      className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                      value={totalUsd}
                      onChange={(e) => setTotalUsd(e.target.value === '' ? '' : Number(e.target.value))}
                      placeholder="np. 100"
                    />
                    {pricesQ.isLoading ? (
                      <div className="text-xs text-muted-foreground mt-1">Pobieram ceny (Jupiter)…</div>
                    ) : pricesQ.error ? (
                      <div className="text-xs text-destructive mt-1">
                        {(pricesQ.error as Error).message}
                      </div>
                    ) : null}
                  </div>
                  <div className="md:col-span-2">
                    <label className="block text-sm font-medium mb-1">
                      Split {tokenA?.symbol ?? 'Token A'} / {tokenB?.symbol ?? 'Token B'}: {splitPctA}%
                      / {100 - splitPctA}%
                    </label>
                    <input
                      type="range"
                      min={0}
                      max={100}
                      value={splitPctA}
                      onChange={(e) => setSplitPctA(Number(e.target.value))}
                      className="w-full"
                    />
                  </div>
                </div>
              )}

              <div className="grid gap-4 md:grid-cols-2">
                <div>
                  <label className="block text-sm font-medium mb-1">
                    Amount {tokenA?.symbol ?? 'Token A'}
                  </label>
                  <input
                    type="number"
                    step="0.000001"
                    className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                    value={amountAUi}
                    onChange={(e) => setAmountAUi(e.target.value === '' ? '' : Number(e.target.value))}
                    required
                  />
                  {tokenA && (
                    <div className="text-[11px] text-muted-foreground mt-1">
                      raw u64 = round(amount × 10^{tokenA.decimals})
                    </div>
                  )}
                </div>
                <div>
                  <label className="block text-sm font-medium mb-1">
                    Amount {tokenB?.symbol ?? 'Token B'}
                  </label>
                  <input
                    type="number"
                    step="0.000001"
                    className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                    value={amountBUi}
                    onChange={(e) => setAmountBUi(e.target.value === '' ? '' : Number(e.target.value))}
                    required
                  />
                  {tokenB && (
                    <div className="text-[11px] text-muted-foreground mt-1">
                      raw u64 = round(amount × 10^{tokenB.decimals})
                    </div>
                  )}
                </div>
              </div>

              {(tokenA && amountAUi !== '' && toBaseUnitsU64(Number(amountAUi), tokenA.decimals) === null) ||
              (tokenB && amountBUi !== '' && toBaseUnitsU64(Number(amountBUi), tokenB.decimals) === null) ? (
                <div className="text-xs text-destructive">
                  Kwota jest nieprawidłowa albo za duża (przekracza limit bezpiecznych liczb JS dla u64).
                </div>
              ) : null}
            </div>

            <div className="flex justify-end gap-2 pt-2">
              <Link to="/positions">
                <Button variant="outline" type="button">
                  Cancel
                </Button>
              </Link>
              <Button type="submit" disabled={mutation.isPending}>
                {mutation.isPending ? 'Opening...' : 'Open Position'}
              </Button>
            </div>
          </form>
        </CardContent>
      </Card>
    </div>
  )
}

