import { useQuery } from '@tanstack/react-query'
import { Link } from 'react-router-dom'
import { RefreshCw } from 'lucide-react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { getClosedPositions } from '@/lib/api'
import { shortenAddress, formatDate } from '@/lib/utils'

export default function ClosedPositions() {
  const q = useQuery({
    queryKey: ['closed-positions', 100, 0],
    queryFn: () => getClosedPositions(100, 0),
    staleTime: 30_000,
    retry: 1,
  })

  const items = q.data?.items ?? []

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-3xl font-bold">Closed positions</h1>
        <Button variant="outline" size="sm" onClick={() => q.refetch()}>
          <RefreshCw className="h-4 w-4 mr-2" />
          Refresh
        </Button>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>History (registry)</CardTitle>
          <p className="text-sm text-muted-foreground font-normal">
            Źródło: append-only <code className="text-[11px]">registry.jsonl</code>. Ta lista działa nawet bez DB.
          </p>
          {q.data?.note ? (
            <p className="text-xs text-muted-foreground font-normal">{q.data.note}</p>
          ) : null}
        </CardHeader>
        <CardContent>
          {q.isPending ? (
            <div className="text-center py-8 text-muted-foreground">Loading...</div>
          ) : q.isError ? (
            <div className="text-center py-8 text-muted-foreground">
              {(q.error instanceof Error ? q.error.message : String(q.error)) ?? 'Failed.'}
            </div>
          ) : items.length === 0 ? (
            <div className="text-center py-8 text-muted-foreground">No closed positions found.</div>
          ) : (
            <div className="overflow-x-auto">
              <table className="w-full">
                <thead>
                  <tr className="border-b text-left text-sm text-muted-foreground">
                    <th className="pb-3 font-medium">Position</th>
                    <th className="pb-3 font-medium">Pool</th>
                    <th className="pb-3 font-medium">Owner</th>
                    <th className="pb-3 font-medium">Close kind</th>
                    <th className="pb-3 font-medium">Opened</th>
                    <th className="pb-3 font-medium">Closed</th>
                    <th className="pb-3 font-medium">Session</th>
                  </tr>
                </thead>
                <tbody>
                  {items.map((p) => (
                    <tr key={p.position_address} className="border-b last:border-0">
                      <td className="py-4">
                        <Link
                          to={`/positions/closed/${encodeURIComponent(p.position_address)}`}
                          className="font-medium hover:text-primary"
                        >
                          {shortenAddress(p.position_address)}
                        </Link>
                      </td>
                      <td className="py-4 text-muted-foreground">{shortenAddress(p.pool_address)}</td>
                      <td className="py-4 text-muted-foreground">{shortenAddress(p.owner)}</td>
                      <td className="py-4 text-muted-foreground">
                        {p.close_kind ? (
                          <span className="inline-flex items-center rounded-md border border-border/60 bg-muted/30 px-2 py-0.5 text-xs font-medium">
                            {p.close_kind}
                          </span>
                        ) : (
                          '—'
                        )}
                      </td>
                      <td className="py-4 text-muted-foreground">
                        {p.opened_ts_utc ? formatDate(p.opened_ts_utc) : '—'}
                      </td>
                      <td className="py-4 text-muted-foreground">
                        {p.closed_ts_utc ? formatDate(p.closed_ts_utc) : '—'}
                      </td>
                      <td className="py-4 text-muted-foreground">
                        {p.last_rebalance_session_id ? p.last_rebalance_session_id : '—'}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  )
}

