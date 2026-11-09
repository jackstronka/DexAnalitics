import { useState } from 'react'
import { Link } from 'react-router-dom'
import { useQuery } from '@tanstack/react-query'
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { ErrorBanner } from '@/components/ui/error-banner'
import { getDipRuntime } from '@/lib/api'
import {
  dipLiveArmed,
  formatLadderDefaultsLine,
  secretPresence,
} from '@/lib/dipRuntimeDisplay'
import { getMetricsMode, type MetricsMode } from '@/lib/metricsMode'
import { useI18n } from '@/lib/i18n'
import { APP_VERSION } from '@/lib/version'

function presenceClass(set: boolean): string {
  return set
    ? 'bg-emerald-100 text-emerald-800 border-emerald-200'
    : 'bg-amber-100 text-amber-800 border-amber-200'
}

export default function Settings() {
  const { locale, t } = useI18n()
  const [metricsMode, setMetricsMode] = useState<MetricsMode>(() => getMetricsMode())
  const [apiKey, setApiKey] = useState('')
  const [rpcUrl, setRpcUrl] = useState('https://api.mainnet-beta.solana.com')
  const [dryRun, setDryRun] = useState(true)

  const runtimeQ = useQuery({
    queryKey: ['dip-runtime'],
    queryFn: getDipRuntime,
    refetchInterval: 60_000,
    retry: 1,
  })
  const rt = runtimeQ.data

  const handleSave = () => {
    localStorage.setItem(
      'clmm-settings',
      JSON.stringify({ apiKey, rpcUrl, dryRun, pnl_mode: metricsMode }),
    )
    alert(locale === 'pl' ? 'Ustawienia zapisane!' : 'Settings saved!')
  }

  return (
    <div className="space-y-6">
      <h1 className="text-3xl font-bold">{t('settings.title')}</h1>

      <Card>
        <CardHeader>
          <CardTitle>{t('settings.apiTitle')}</CardTitle>
          <CardDescription>{t('settings.apiHint')}</CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="rounded-md border p-3 text-sm space-y-1">
            <div className="flex justify-between gap-2">
              <span className="text-muted-foreground">{t('settings.apiUpstream')}</span>
              <code className="text-xs">/api/v1 → Vite proxy (default host API :8081)</code>
            </div>
            <p className="text-xs text-muted-foreground">{t('settings.apiUpstreamHint')}</p>
          </div>
          <div className="space-y-2">
            <label className="text-sm font-medium">{t('settings.apiKeyLocal')}</label>
            <input
              type="password"
              value={apiKey}
              onChange={(e) => setApiKey(e.target.value)}
              placeholder={t('settings.apiKeyPlaceholder')}
              className="w-full px-3 py-2 rounded-md border bg-background"
            />
          </div>
          <div className="space-y-2">
            <label className="text-sm font-medium">RPC URL</label>
            <input
              type="text"
              value={rpcUrl}
              onChange={(e) => setRpcUrl(e.target.value)}
              placeholder="https://api.mainnet-beta.solana.com"
              className="w-full px-3 py-2 rounded-md border bg-background"
            />
            <p className="text-xs text-muted-foreground">{t('settings.rpcLocalHint')}</p>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>{t('settings.dipTitle')}</CardTitle>
          <CardDescription>{t('settings.dipHint')}</CardDescription>
        </CardHeader>
        <CardContent className="space-y-3 text-sm">
          {runtimeQ.isError ? (
            <ErrorBanner>
              {runtimeQ.error instanceof Error ? runtimeQ.error.message : String(runtimeQ.error)}
            </ErrorBanner>
          ) : null}
          {runtimeQ.isLoading && !rt ? (
            <p className="text-muted-foreground">{t('dip.loading')}</p>
          ) : rt ? (
            <>
              <div className="flex flex-wrap gap-2">
                <span className="inline-flex items-center rounded border px-2 py-1 text-xs font-medium">
                  {t('settings.dipMode')}: <strong className="ml-1">{rt.mode}</strong>
                </span>
                <span
                  className={`inline-flex items-center rounded border px-2 py-1 text-xs font-medium ${presenceClass(rt.jupiter_api_key_set)}`}
                >
                  S-JUP: {t(`settings.secret.${secretPresence(rt.jupiter_api_key_set)}`)}
                </span>
                <span
                  className={`inline-flex items-center rounded border px-2 py-1 text-xs font-medium ${presenceClass(rt.trigger_jwt_set)}`}
                >
                  S-JWT: {t(`settings.secret.${secretPresence(rt.trigger_jwt_set)}`)}
                </span>
                <span className="inline-flex items-center rounded border px-2 py-1 text-xs font-medium">
                  DCA: {rt.dca_allowed ? t('settings.yes') : t('settings.no')}
                </span>
                <span className="inline-flex items-center rounded border px-2 py-1 text-xs font-medium">
                  {t('settings.liveArmed')}:{' '}
                  {dipLiveArmed(rt.mode, rt.trigger_jwt_set)
                    ? t('settings.yes')
                    : t('settings.no')}
                </span>
              </div>
              <p className="text-xs text-muted-foreground">
                {t('settings.runtimePath')}: <code className="break-all">{rt.path}</code>
                {rt.file_missing ? ` (${t('settings.fileMissing')})` : ''}
              </p>
              <div className="rounded-md border p-3 space-y-1">
                <div className="font-medium text-xs">{t('settings.ladderDefaults')}</div>
                <p className="text-xs font-mono break-words">
                  {formatLadderDefaultsLine(rt.ladder)}
                </p>
                <p className="text-xs text-muted-foreground">{t('settings.ladderEditHint')}</p>
              </div>
              {rt.notes.length > 0 && (
                <ul className="list-disc pl-5 text-muted-foreground text-xs space-y-1">
                  {rt.notes.map((n) => (
                    <li key={n}>{n}</li>
                  ))}
                </ul>
              )}
              <Button asChild variant="secondary" size="sm">
                <Link to="/dip">{t('settings.openDip')}</Link>
              </Button>
            </>
          ) : null}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>{t('settings.execTitle')}</CardTitle>
          <CardDescription>{t('settings.execHint')}</CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex items-center justify-between">
            <div>
              <div className="font-medium">{t('settings.dryRun')}</div>
              <div className="text-sm text-muted-foreground">{t('settings.dryRunHint')}</div>
            </div>
            <button
              type="button"
              onClick={() => setDryRun(!dryRun)}
              className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors ${
                dryRun ? 'bg-primary' : 'bg-muted'
              }`}
            >
              <span
                className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
                  dryRun ? 'translate-x-6' : 'translate-x-1'
                }`}
              />
            </button>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>{t('settings.metricsTitle')}</CardTitle>
          <CardDescription>{t('settings.metricsHint')}</CardDescription>
        </CardHeader>
        <CardContent className="space-y-2">
          <label className="text-sm font-medium">{t('settings.pnlMode')}</label>
          <select
            value={metricsMode}
            onChange={(e) => setMetricsMode(e.target.value as MetricsMode)}
            className="w-full px-3 py-2 rounded-md border bg-background"
          >
            <option value="live">{t('settings.pnlLive')}</option>
            <option value="settlement_v1">Settlement v1</option>
          </select>
          <p className="text-xs text-muted-foreground">{t('settings.pnlHint')}</p>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>{t('settings.aboutTitle')}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-2">
          <div className="flex justify-between">
            <span className="text-muted-foreground">{t('settings.version')}</span>
            <span>{APP_VERSION}</span>
          </div>
          <div className="flex justify-between">
            <span className="text-muted-foreground">{t('settings.build')}</span>
            <span>{t('settings.buildDev')}</span>
          </div>
          <div className="flex justify-between">
            <span className="text-muted-foreground">{t('settings.license')}</span>
            <span>MIT / Apache-2.0</span>
          </div>
        </CardContent>
      </Card>

      <div className="flex justify-end">
        <Button onClick={handleSave}>{t('settings.save')}</Button>
      </div>
    </div>
  )
}
