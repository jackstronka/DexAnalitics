import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { Copy, Play, Terminal, ScrollText, X } from 'lucide-react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { getScripts, runScript, type ScriptRunRecord } from '@/lib/api'
import { formatDate } from '@/lib/utils'
import { useToast } from '@/hooks/use-toast'

function copyText(text: string) {
  void navigator.clipboard.writeText(text)
}

type ScriptItem = NonNullable<Awaited<ReturnType<typeof getScripts>>['scripts']>[number]

function isDataQualityScript(id: string): boolean {
  const x = id.toLowerCase()
  return (
    x.includes('snapshot') ||
    x.includes('collector') ||
    x.includes('quick_verify') ||
    x.includes('data_alert') ||
    x.includes('health') ||
    x.includes('decode') ||
    x.includes('backtest_prep')
  )
}

function isBotScript(id: string): boolean {
  const x = id.toLowerCase()
  return (
    x.startsWith('bot_') ||
    x.includes('orca_bot') ||
    x.includes('position_monitor') ||
    x.includes('rebalance') ||
    x.includes('strategy')
  )
}

export default function Scripts() {
  const { toast } = useToast()
  const qc = useQueryClient()
  const [runLog, setRunLog] = useState<{ scriptId: string; run: ScriptRunRecord } | null>(null)
  const [runningIds, setRunningIds] = useState<Record<string, boolean>>({})

  const { data, isLoading, error } = useQuery({
    queryKey: ['scripts'],
    queryFn: getScripts,
  })

  const runMutation = useMutation({
    mutationFn: (id: string) => runScript(id, 'web'),
    onMutate: (id) => {
      setRunningIds((prev) => ({ ...prev, [id]: true }))
      toast({ title: 'Uruchamianie skryptu…', description: id })
    },
    onSuccess: () => {
      toast({ title: 'Skrypt zakończony', description: 'Run zapisany w data/script_runs.jsonl.' })
      void qc.invalidateQueries({ queryKey: ['scripts'] })
    },
    onError: (e: Error) => {
      toast({ title: 'Run nieudany', description: e.message, variant: 'destructive' })
    },
    onSettled: (_data, _err, id) => {
      setRunningIds((prev) => {
        const next = { ...prev }
        delete next[id]
        return next
      })
    },
  })

  const runnerHint = data?.runner_configured
    ? 'Runner jest skonfigurowany na API (URL + token).'
    : 'Ustaw SCRIPT_RUNNER_URL i SCRIPT_RUNNER_TOKEN w root .env i uruchom tools/Start-ClmmScriptRunner.ps1.'

  const allScripts = data?.scripts ?? []
  const priorityDataQuality = allScripts.filter((s) => isDataQualityScript(s.id))
  const priorityBot = allScripts.filter((s) => isBotScript(s.id))

  function renderTable(items: ScriptItem[]) {
    return (
      <div className="overflow-x-auto">
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b text-left text-muted-foreground">
              <th className="py-2 pr-4">Script</th>
              <th className="py-2 pr-4">Summary</th>
              <th className="py-2 pr-4">Last run</th>
              <th className="py-2 pr-4">Status</th>
              <th className="py-2 pr-4">Error</th>
              <th className="py-2 pr-2">Logi</th>
              <th className="py-2">Actions</th>
            </tr>
          </thead>
          <tbody>
            {items.map((s) => {
              const lr = s.last_run
              const cmd = `pwsh -NoProfile -File ${s.path}`
              const isRunning = !!runningIds[s.id]
              return (
                <tr key={s.id} className="border-b border-border/60">
                  <td className="py-2 pr-4 align-top">
                    <span className="font-mono text-xs" title={s.id}>
                      {s.id}
                    </span>
                    {s.auto_discovered && (
                      <span
                        className="ml-2 text-[10px] uppercase tracking-wide text-amber-600 dark:text-amber-500"
                        title="Brak wpisu w scripts-manifest.json — wykryto plik na dysku"
                      >
                        auto
                      </span>
                    )}
                    {s.risk && <span className="ml-2 text-xs text-muted-foreground">({s.risk})</span>}
                  </td>
                  <td className="py-2 pr-4 align-top max-w-md">
                    <span title={s.summary}>{s.summary}</span>
                    {s.when_to_use && (
                      <p className="text-xs text-muted-foreground mt-1" title={s.when_to_use}>
                        {s.when_to_use}
                      </p>
                    )}
                  </td>
                  <td className="py-2 pr-4 align-top whitespace-nowrap">{lr ? formatDate(lr.ts_utc) : '—'}</td>
                  <td className="py-2 pr-4 align-top">
                    {lr ? (
                      <span className={lr.ok ? 'text-green-500' : 'text-red-500'}>{lr.ok ? 'OK' : 'Error'}</span>
                    ) : (
                      '—'
                    )}
                    {isRunning && <span className="ml-2 text-xs text-muted-foreground">Running…</span>}
                  </td>
                  <td className="py-2 pr-4 align-top max-w-xs truncate" title={lr?.error_excerpt ?? ''}>
                    {lr?.error_excerpt ?? '—'}
                  </td>
                  <td className="py-2 pr-2 align-top">
                    {lr ? (
                      <Button
                        type="button"
                        variant="ghost"
                        size="sm"
                        className="h-8 px-2"
                        onClick={() => setRunLog({ scriptId: s.id, run: lr })}
                        title="Fragmenty stdout/stderr z ostatniego runu (JSONL)"
                      >
                        <ScrollText className="h-3.5 w-3.5 mr-1" />
                        Logi
                      </Button>
                    ) : (
                      '—'
                    )}
                  </td>
                  <td className="py-2 align-top">
                    <div className="flex flex-wrap gap-2">
                      <Button
                        type="button"
                        variant="outline"
                        size="sm"
                        onClick={() => {
                          copyText(cmd)
                          toast({ title: 'Copied', description: cmd })
                        }}
                      >
                        <Copy className="h-3 w-3 mr-1" />
                        Copy
                      </Button>
                      {s.runnable && (
                        <Button
                          type="button"
                          size="sm"
                          disabled={isRunning || runMutation.isPending || !data?.runner_configured}
                          onClick={() => runMutation.mutate(s.id)}
                          title={
                            !data?.runner_configured
                              ? 'Skonfiguruj runner (SCRIPT_RUNNER_URL/TOKEN) na API'
                              : isRunning
                                ? 'Skrypt jest w trakcie uruchamiania'
                                : 'Uruchom przez localhost runner'
                          }
                        >
                          <Play className="h-3 w-3 mr-1" />
                          {isRunning ? 'Running…' : 'Run'}
                        </Button>
                      )}
                    </div>
                  </td>
                </tr>
              )
            })}
          </tbody>
        </table>
      </div>
    )
  }

  return (
    <div className="space-y-6">
      <p className="text-xs text-muted-foreground">
        Lista = <code className="text-[11px]">tools/scripts-manifest.json</code> oraz brakujące pliki{' '}
        <code className="text-[11px]">tools/*.ps1</code> (pierwszy poziom katalogu). API:{' '}
        <code className="text-[11px]">CLMM_REPO_ROOT</code> musi wskazywać na katalog repozytorium.
      </p>

      <div>
        <h1 className="text-3xl font-bold">Skrypty</h1>
        <p className="text-muted-foreground text-sm mt-1">
          Skrypty operatorskie z <code className="text-xs">tools/scripts-manifest.json</code>. {runnerHint}
        </p>
        <p className="text-muted-foreground text-xs mt-2">
          Pełny spis i słowa kluczowe (wymaganie §1 — dokumentacja w repo):{' '}
          <code className="text-[11px]">doc/SCRIPTS_CATALOG.md</code> w klonie repozytorium (np. IDE).
        </p>
        <p className="text-muted-foreground text-xs mt-2">
          Kontrakt działania: kliknięcie <strong>Run</strong> uruchamia skrypt na hoście (runner localhost) i zapisuje wynik do{' '}
          <code className="text-[11px]">data/script_runs.jsonl</code>. W UI blokujemy wieloklik dla tego samego skryptu do czasu zakończenia.
        </p>
      </div>

      {runLog && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
          role="dialog"
          aria-modal="true"
          aria-labelledby="run-log-title"
          onClick={() => setRunLog(null)}
        >
          <div
            className="bg-card max-h-[85vh] w-full max-w-3xl overflow-hidden rounded-lg border shadow-lg flex flex-col"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="flex items-center justify-between border-b px-4 py-3">
              <h2 id="run-log-title" className="font-semibold text-sm">
                Ostatni run: <span className="font-mono">{runLog.scriptId}</span>
              </h2>
              <button
                type="button"
                className="rounded-md p-1.5 hover:bg-muted"
                onClick={() => setRunLog(null)}
                aria-label="Zamknij"
              >
                <X className="h-4 w-4" />
              </button>
            </div>
            <div className="overflow-auto p-4 text-xs space-y-3">
              <p className="text-muted-foreground">
                exit={runLog.run.exit_code} · {runLog.run.duration_ms} ms · {runLog.run.ts_utc}
              </p>
              {runLog.run.error_excerpt && (
                <div>
                  <div className="font-medium text-destructive mb-1">error_excerpt</div>
                  <pre className="whitespace-pre-wrap rounded-md bg-muted/50 p-2">{runLog.run.error_excerpt}</pre>
                </div>
              )}
              {runLog.run.stderr_excerpt && (
                <div>
                  <div className="font-medium mb-1">stderr_excerpt</div>
                  <pre className="whitespace-pre-wrap rounded-md bg-muted/50 p-2">{runLog.run.stderr_excerpt}</pre>
                </div>
              )}
              {runLog.run.stdout_excerpt && (
                <div>
                  <div className="font-medium mb-1">stdout_excerpt</div>
                  <pre className="whitespace-pre-wrap rounded-md bg-muted/50 p-2">{runLog.run.stdout_excerpt}</pre>
                </div>
              )}
              {!runLog.run.error_excerpt &&
                !runLog.run.stderr_excerpt &&
                !runLog.run.stdout_excerpt && (
                  <p className="text-muted-foreground">Brak excerptów w rekordzie (zob. plik runów na hoście API).</p>
                )}
            </div>
          </div>
        </div>
      )}

      {data?.manifest_missing && (
        <Card className="border-yellow-600/50">
          <CardHeader>
            <CardTitle className="text-yellow-500">Brak tools/scripts-manifest.json</CardTitle>
          </CardHeader>
          <CardContent className="text-sm text-muted-foreground">
            API pokazuje skrypty wyłącznie ze skanu <code className="text-xs">tools/*.ps1</code>. Utwórz manifest w repo,
            żeby mieć opisy i spójność z runnerem. Korzeń: <code className="text-xs">{data.repo_root}</code>
          </CardContent>
        </Card>
      )}

      {error && (
        <Card className="border-destructive/50">
          <CardContent className="pt-6 text-destructive text-sm">
            {(error as Error).message}
          </CardContent>
        </Card>
      )}

      <Card className="border-primary/30">
        <CardHeader>
          <CardTitle>Priorytet: poprawność i jakość danych</CardTitle>
        </CardHeader>
        <CardContent>
          {(priorityDataQuality ?? []).length === 0 ? (
            <div className="text-muted-foreground text-sm">Brak pasujących skryptów w manifeście.</div>
          ) : (
            renderTable(priorityDataQuality)
          )}
        </CardContent>
      </Card>

      <Card className="border-primary/30">
        <CardHeader>
          <CardTitle>Priorytet: obsługa bota</CardTitle>
        </CardHeader>
        <CardContent>
          {(priorityBot ?? []).length === 0 ? (
            <div className="text-muted-foreground text-sm">Brak pasujących skryptów w manifeście.</div>
          ) : (
            renderTable(priorityBot)
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Terminal className="h-5 w-5" />
            Katalog
          </CardTitle>
        </CardHeader>
        <CardContent>
          {isLoading ? (
            <div className="text-muted-foreground">Loading…</div>
          ) : (data?.scripts ?? []).length === 0 ? (
            <div className="rounded-md border border-amber-500/40 bg-amber-500/5 px-4 py-3 text-sm space-y-2">
              <p className="font-medium text-foreground">Brak skryptów w odpowiedzi API</p>
              <p className="text-muted-foreground text-xs leading-relaxed">
                Backend szuka <code className="text-[11px]">tools/scripts-manifest.json</code> i{' '}
                <code className="text-[11px]">tools/*.ps1</code> względem <strong>korzenia repozytorium</strong>. Jeśli
                proces API wystartował z innego katalogu (np. tylko <code className="text-[11px]">web/</code>), wcześniej
                lista bywała pusta — po poprawce API samo znajduje root (lub ustaw{' '}
                <code className="text-[11px]">CLMM_REPO_ROOT</code>).
              </p>
              <p className="text-xs text-muted-foreground">
                Rozpoznany korzeń (diagnostyka):{' '}
                <code className="break-all text-[11px]">{data?.repo_root ?? '—'}</code>
              </p>
            </div>
          ) : (
            renderTable(allScripts)
          )}
        </CardContent>
      </Card>
    </div>
  )
}
