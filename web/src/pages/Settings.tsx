import { useState } from 'react'
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { getMetricsMode, type MetricsMode } from '@/lib/metricsMode'
import { useI18n } from '@/lib/i18n'
import { APP_VERSION } from '@/lib/version'

export default function Settings() {
  const { locale } = useI18n()
  const [metricsMode, setMetricsMode] = useState<MetricsMode>(() => getMetricsMode())
  const [apiKey, setApiKey] = useState('')
  const [rpcUrl, setRpcUrl] = useState('https://api.mainnet-beta.solana.com')
  const [dryRun, setDryRun] = useState(true)

  const handleSave = () => {
    // Save settings to localStorage or API
    localStorage.setItem(
      'clmm-settings',
      JSON.stringify({ apiKey, rpcUrl, dryRun, pnl_mode: metricsMode }),
    )
    alert(locale === 'pl' ? 'Ustawienia zapisane!' : 'Settings saved!')
  }

  return (
    <div className="space-y-6">
      <h1 className="text-3xl font-bold">{locale === 'pl' ? 'Ustawienia' : 'Settings'}</h1>

      <Card>
        <CardHeader>
          <CardTitle>{locale === 'pl' ? 'Konfiguracja API' : 'API Configuration'}</CardTitle>
          <CardDescription>{locale === 'pl' ? 'Skonfiguruj ustawienia połączenia API' : 'Configure your API connection settings'}</CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="space-y-2">
            <label className="text-sm font-medium">{locale === 'pl' ? 'Klucz API' : 'API Key'}</label>
            <input
              type="password"
              value={apiKey}
              onChange={(e) => setApiKey(e.target.value)}
              placeholder={locale === 'pl' ? 'Wpisz klucz API' : 'Enter your API key'}
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
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>{locale === 'pl' ? 'Ustawienia wykonania' : 'Execution Settings'}</CardTitle>
          <CardDescription>{locale === 'pl' ? 'Skonfiguruj sposób wykonywania transakcji' : 'Configure how transactions are executed'}</CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex items-center justify-between">
            <div>
              <div className="font-medium">{locale === 'pl' ? 'Tryb Dry Run' : 'Dry Run Mode'}</div>
              <div className="text-sm text-muted-foreground">
                {locale === 'pl' ? 'Symuluj transakcje bez wykonania on-chain' : 'Simulate transactions without executing on-chain'}
              </div>
            </div>
            <button
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
          <CardTitle>{locale === 'pl' ? 'Tryb metryk' : 'Metrics Mode'}</CardTitle>
          <CardDescription>{locale === 'pl' ? 'Wybierz źródło metryk dla widoków pozycji' : 'Select metrics source for position views'}</CardDescription>
        </CardHeader>
        <CardContent className="space-y-2">
          <label className="text-sm font-medium">{locale === 'pl' ? 'Tryb PnL/IL' : 'PnL/IL Mode'}</label>
          <select
            value={metricsMode}
            onChange={(e) => setMetricsMode(e.target.value as MetricsMode)}
            className="w-full px-3 py-2 rounded-md border bg-background"
          >
            <option value="live">{locale === 'pl' ? 'Live stream (domyślny)' : 'Live stream (current default)'}</option>
            <option value="settlement_v1">Settlement v1</option>
          </select>
          <p className="text-xs text-muted-foreground">
            {locale === 'pl'
              ? 'Settlement v1 używa finalnych etykiet interpretacji w kartach szczegółów pozycji.'
              : 'Settlement v1 uses finalized interpretation labels in position detail cards.'}
          </p>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>{locale === 'pl' ? 'Informacje' : 'About'}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-2">
          <div className="flex justify-between">
            <span className="text-muted-foreground">{locale === 'pl' ? 'Wersja' : 'Version'}</span>
            <span>{APP_VERSION}</span>
          </div>
          <div className="flex justify-between">
            <span className="text-muted-foreground">{locale === 'pl' ? 'Build' : 'Build'}</span>
            <span>{locale === 'pl' ? 'Deweloperski' : 'Development'}</span>
          </div>
          <div className="flex justify-between">
            <span className="text-muted-foreground">{locale === 'pl' ? 'Licencja' : 'License'}</span>
            <span>MIT / Apache-2.0</span>
          </div>
        </CardContent>
      </Card>

      <div className="flex justify-end">
        <Button onClick={handleSave}>{locale === 'pl' ? 'Zapisz ustawienia' : 'Save Settings'}</Button>
      </div>
    </div>
  )
}
