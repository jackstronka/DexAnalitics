import { useState } from 'react'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { useNavigate, Link } from 'react-router-dom'
import { ArrowLeft } from 'lucide-react'
import { Card, CardHeader, CardTitle, CardContent } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { openPosition } from '@/lib/api'

export default function PositionCreate() {
  const navigate = useNavigate()
  const queryClient = useQueryClient()

  const [poolAddress, setPoolAddress] = useState('')
  const [tickLower, setTickLower] = useState<number | ''>('')
  const [tickUpper, setTickUpper] = useState<number | ''>('')
  const [amountA, setAmountA] = useState<number | ''>('')
  const [amountB, setAmountB] = useState<number | ''>('')

  const mutation = useMutation({
    mutationFn: openPosition,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['positions'] })
      navigate('/positions')
    },
  })

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()
    if (
      !poolAddress.trim() ||
      tickLower === '' ||
      tickUpper === '' ||
      amountA === '' ||
      amountB === ''
    ) {
      return
    }

    mutation.mutate({
      pool_address: poolAddress.trim(),
      tick_lower: Number(tickLower),
      tick_upper: Number(tickUpper),
      amount_a: Number(amountA),
      amount_b: Number(amountB),
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
              <label className="block text-sm font-medium mb-1">Pool Address</label>
              <input
                className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm font-mono"
                value={poolAddress}
                onChange={(e) => setPoolAddress(e.target.value)}
                placeholder="Whirlpool pool address"
                required
              />
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

            <div className="grid gap-4 md:grid-cols-2">
              <div>
                <label className="block text-sm font-medium mb-1">Amount Token A</label>
                <input
                  type="number"
                  step="0.000001"
                  className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                  value={amountA}
                  onChange={(e) => setAmountA(e.target.value === '' ? '' : Number(e.target.value))}
                  required
                />
              </div>
              <div>
                <label className="block text-sm font-medium mb-1">Amount Token B</label>
                <input
                  type="number"
                  step="0.000001"
                  className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                  value={amountB}
                  onChange={(e) => setAmountB(e.target.value === '' ? '' : Number(e.target.value))}
                  required
                />
              </div>
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

