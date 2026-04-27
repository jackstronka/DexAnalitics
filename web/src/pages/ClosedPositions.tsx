import { useQuery } from '@tanstack/react-query'
import { Link } from 'react-router-dom'
import { RefreshCw } from 'lucide-react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { closedPositionsListQueryOptions } from '@/lib/api'
import { shortenAddress, formatDate } from '@/lib/utils'
import { PoolPairLabels } from '@/components/PoolPairLabels'
import { useI18n } from '@/lib/i18n'

const CLOSED_PAGE_LIMIT = 100
const CLOSED_PAGE_OFFSET = 0

export default function ClosedPositions() {
  const { locale } = useI18n()
  // Staged load: registry-only first (no RPC), then enrich pair labels in the background.
  const fastQ = useQuery(closedPositionsListQueryOptions(CLOSED_PAGE_LIMIT, CLOSED_PAGE_OFFSET, false))
  const fullQ = useQuery({
    ...closedPositionsListQueryOptions(CLOSED_PAGE_LIMIT, CLOSED_PAGE_OFFSET, true),
    placeholderData: fastQ.data,
  })

  const items = (fullQ.data ?? fastQ.data)?.items ?? []
  const enrichingPairs = Boolean(fullQ.isFetching && fullQ.isPlaceholderData)
  const showInitialLoad = fastQ.isPending && !fastQ.data
  const loadError =
    fastQ.isError && !fastQ.data
      ? fastQ.error
      : fullQ.isError && !fastQ.data
        ? fullQ.error
        : null

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-3xl font-bold">{locale === 'pl' ? 'Zamknięte pozycje' : 'Closed positions'}</h1>
        <Button
          variant="outline"
          size="sm"
          onClick={() => {
            void fastQ.refetch()
            void fullQ.refetch()
          }}
        >
          <RefreshCw className="h-4 w-4 mr-2" />
          {locale === 'pl' ? 'Odśwież' : 'Refresh'}
        </Button>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>{locale === 'pl' ? 'Historia (rejestr)' : 'History (registry)'}</CardTitle>
          <p className="text-sm text-muted-foreground font-normal">
            Źródło: append-only <code className="text-[11px]">registry.jsonl</code>. Ta lista działa nawet bez DB.
          </p>
          {enrichingPairs ? (
            <p className="text-xs text-muted-foreground font-normal">Ładowanie par tokenów (RPC)…</p>
          ) : null}
          {fullQ.data?.note ? (
            <p className="text-xs text-muted-foreground font-normal">{fullQ.data.note}</p>
          ) : null}
          {fullQ.isError && fastQ.isSuccess && items.length > 0 ? (
            <p className="text-xs text-amber-700 dark:text-amber-500 font-normal">
              Nie udało się wczytać etykiet par (RPC); widać dane z rejestru. Użyj Refresh albo sprawdź RPC.
            </p>
          ) : null}
        </CardHeader>
        <CardContent>
          {showInitialLoad ? (
            <div className="text-center py-8 text-muted-foreground">{locale === 'pl' ? 'Ładowanie...' : 'Loading...'}</div>
          ) : loadError ? (
            <div className="text-center py-8 text-muted-foreground">
              {(loadError instanceof Error ? loadError.message : String(loadError)) ?? (locale === 'pl' ? 'Błąd.' : 'Failed.')}
            </div>
          ) : items.length === 0 ? (
            <div className="text-center py-8 text-muted-foreground">{locale === 'pl' ? 'Brak zamkniętych pozycji.' : 'No closed positions found.'}</div>
          ) : (
            <div className="overflow-x-auto">
              <table className="w-full">
                <thead>
                  <tr className="border-b text-left text-sm text-muted-foreground">
                    <th className="pb-3 font-medium">{locale === 'pl' ? 'Pozycja' : 'Position'}</th>
                    <th className="pb-3 font-medium">{locale === 'pl' ? 'Para' : 'Pair'}</th>
                    <th className="pb-3 font-medium">{locale === 'pl' ? 'Właściciel' : 'Owner'}</th>
                    <th className="pb-3 font-medium">{locale === 'pl' ? 'Rodzaj zamknięcia' : 'Close kind'}</th>
                    <th className="pb-3 font-medium">{locale === 'pl' ? 'Otwarcie' : 'Opened'}</th>
                    <th className="pb-3 font-medium">{locale === 'pl' ? 'Zamknięcie' : 'Closed'}</th>
                    <th className="pb-3 font-medium">{locale === 'pl' ? 'Sesja' : 'Session'}</th>
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
                      <td className="py-4">
                        <PoolPairLabels
                          labelA={p.token_a_label}
                          labelB={p.token_b_label}
                          mintA={p.token_mint_a}
                          mintB={p.token_mint_b}
                        />
                        <div className="text-[10px] text-muted-foreground font-mono mt-1">
                          {shortenAddress(p.pool_address, 4)}
                        </div>
                      </td>
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

