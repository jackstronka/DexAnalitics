import { useMemo, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Link } from 'react-router-dom'
import { Brain, RefreshCw } from 'lucide-react'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { ErrorBanner } from '@/components/ui/error-banner'
import { getAgentDecisions } from '@/lib/api'
import {
  agentDecisionAction,
  agentDecisionKind,
  formatAgentDecisionPreview,
  matchesAgentDecisionKindFilter,
  type AgentDecisionKindFilter,
} from '@/lib/agentDecisionsDisplay'
import { useI18n } from '@/lib/i18n'

export default function AgentDecisions() {
  const { locale } = useI18n()
  const pl = locale === 'pl'
  const [source, setSource] = useState('')
  const [kindFilter, setKindFilter] = useState<AgentDecisionKindFilter>('all')
  const [limit, setLimit] = useState(200)

  const q = useQuery({
    queryKey: ['agent-decisions', source, limit],
    queryFn: () =>
      getAgentDecisions({
        source: source.trim() || undefined,
        limit,
      }),
    refetchInterval: 30_000,
  })

  const rows = useMemo(() => {
    const all = q.data?.rows ?? []
    return all.filter((r) => matchesAgentDecisionKindFilter(r.decision, kindFilter))
  }, [q.data?.rows, kindFilter])

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2">
            <Brain className="h-6 w-6" />
            {pl ? 'Decyzje agenta' : 'Agent decisions'}
          </h1>
          <p className="text-sm text-muted-foreground mt-1 max-w-2xl">
            {pl
              ? 'Przeglądarka lokalnego audytu (agent_decisions.jsonl) — LP + Dip propose.'
              : 'Browser for local audit (agent_decisions.jsonl) — LP + Dip propose.'}
          </p>
        </div>
        <Button
          variant="outline"
          size="sm"
          onClick={() => void q.refetch()}
          disabled={q.isFetching}
        >
          <RefreshCw className={`h-4 w-4 mr-1 ${q.isFetching ? 'animate-spin' : ''}`} />
          {pl ? 'Odśwież' : 'Refresh'}
        </Button>
      </div>

      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-base">{pl ? 'Filtry' : 'Filters'}</CardTitle>
          <CardDescription>
            {pl
              ? 'source idzie do API; kind filtruje lokalnie (np. dip_propose).'
              : 'source hits the API; kind is filtered client-side (e.g. dip_propose).'}
          </CardDescription>
        </CardHeader>
        <CardContent className="flex flex-wrap gap-3 items-end">
          <label className="text-sm space-y-1">
            <span className="text-muted-foreground">source</span>
            <input
              className="block w-44 rounded-md border border-input bg-background px-2 py-1.5 text-sm"
              placeholder="dip_agent"
              value={source}
              onChange={(e) => setSource(e.target.value)}
            />
          </label>
          <label className="text-sm space-y-1">
            <span className="text-muted-foreground">kind</span>
            <select
              className="block w-44 rounded-md border border-input bg-background px-2 py-1.5 text-sm"
              value={kindFilter}
              onChange={(e) => setKindFilter(e.target.value as AgentDecisionKindFilter)}
            >
              <option value="all">{pl ? 'wszystkie' : 'all'}</option>
              <option value="dip_propose">dip_propose</option>
              <option value="other">{pl ? 'inne' : 'other'}</option>
            </select>
          </label>
          <label className="text-sm space-y-1">
            <span className="text-muted-foreground">limit</span>
            <select
              className="block w-28 rounded-md border border-input bg-background px-2 py-1.5 text-sm"
              value={limit}
              onChange={(e) => setLimit(Number(e.target.value))}
            >
              <option value={100}>100</option>
              <option value={200}>200</option>
              <option value={500}>500</option>
              <option value={1000}>1000</option>
            </select>
          </label>
          <Link to="/dip" className="text-sm text-primary hover:underline ml-auto">
            {pl ? '→ Dip / Jupiter' : '→ Dip / Jupiter'}
          </Link>
        </CardContent>
      </Card>

      {q.isError ? (
        <ErrorBanner>{q.error instanceof Error ? q.error.message : String(q.error)}</ErrorBanner>
      ) : null}

      <Card>
        <CardHeader className="pb-2">
          <CardTitle className="text-base">
            {pl ? 'Wiersze' : 'Rows'}
            {q.data && !q.data.file_missing ? (
              <span className="ml-2 text-sm font-normal text-muted-foreground">
                {rows.length}
                {kindFilter !== 'all' ? ` / ${q.data.rows_returned}` : ''}
              </span>
            ) : null}
          </CardTitle>
          {q.data?.path ? (
            <CardDescription className="font-mono text-xs break-all">{q.data.path}</CardDescription>
          ) : null}
        </CardHeader>
        <CardContent>
          {q.isLoading ? (
            <p className="text-sm text-muted-foreground">{pl ? 'Ładowanie…' : 'Loading…'}</p>
          ) : q.data?.file_missing ? (
            <p className="text-sm text-muted-foreground">
              {pl
                ? 'Plik jeszcze nie istnieje. Propose Dip lub LP zapisze pierwsze wiersze.'
                : 'File does not exist yet. Dip/LP propose will create the first rows.'}
            </p>
          ) : rows.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              {pl ? 'Brak wierszy (albo filtr nic nie zwrócił).' : 'No rows (or filter matched nothing).'}
            </p>
          ) : (
            <div className="overflow-x-auto rounded-md border">
              <table className="w-full text-left text-sm">
                <thead className="bg-muted/50">
                  <tr>
                    <th className="px-3 py-2 font-medium whitespace-nowrap">ts_utc</th>
                    <th className="px-3 py-2 font-medium">source</th>
                    <th className="px-3 py-2 font-medium">kind</th>
                    <th className="px-3 py-2 font-medium">action</th>
                    <th className="px-3 py-2 font-medium">strategy_id</th>
                    <th className="px-3 py-2 font-medium">position_id</th>
                    <th className="px-3 py-2 font-medium">decision</th>
                  </tr>
                </thead>
                <tbody>
                  {[...rows].reverse().map((row, i) => {
                    const kind = agentDecisionKind(row.decision) ?? '—'
                    const action = agentDecisionAction(row.decision) ?? '—'
                    const preview = formatAgentDecisionPreview(row.decision, 160)
                    return (
                      <tr key={`${row.ts_utc}-${i}`} className="border-t border-border/60 hover:bg-muted/30">
                        <td className="px-3 py-2 font-mono text-xs whitespace-nowrap">{row.ts_utc}</td>
                        <td className="px-3 py-2 text-xs">{row.source}</td>
                        <td className="px-3 py-2 font-mono text-xs">{kind}</td>
                        <td className="px-3 py-2 font-mono text-xs">{action}</td>
                        <td className="px-3 py-2 font-mono text-xs max-w-[8rem] truncate" title={row.strategy_id ?? ''}>
                          {row.strategy_id ?? '—'}
                        </td>
                        <td className="px-3 py-2 font-mono text-xs max-w-[8rem] truncate" title={row.position_id ?? ''}>
                          {row.position_id ? (
                            <Link
                              to={`/positions/${encodeURIComponent(row.position_id)}`}
                              className="text-primary hover:underline"
                            >
                              {row.position_id.slice(0, 8)}…
                            </Link>
                          ) : (
                            '—'
                          )}
                        </td>
                        <td className="px-3 py-2 font-mono text-xs max-w-[20rem] truncate" title={preview}>
                          {preview}
                        </td>
                      </tr>
                    )
                  })}
                </tbody>
              </table>
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  )
}
