/**
 * Explains why dashboard lists can be empty even with on-chain Orca positions.
 */
import { useI18n } from '@/lib/i18n'

export default function ApiDataHint() {
  const { locale } = useI18n()
  const L = (pl: string, en: string) => (locale === 'pl' ? pl : en)
  return (
    <div className="rounded-lg border border-amber-500/40 bg-amber-500/5 px-4 py-3 text-sm text-muted-foreground space-y-2">
      <p className="font-medium text-foreground">{L('Skąd biorą się dane w tym panelu?', 'Where does data in this panel come from?')}</p>
      <ul className="list-disc pl-5 space-y-1 text-xs leading-relaxed">
        <li>
          <strong className="text-foreground">{L('Positions / Wallet (agregaty)', 'Positions / Wallet (aggregates)')}</strong> — {L('lista pochodzi z', 'list comes from')}{' '}
          <strong className="text-foreground">{L('monitora w procesie API', 'in-process API monitor')}</strong> {L('(positions added when strategy / bot starts or via another seeding path).', '(pozycje dodane przy starcie strategii / bota albo inną ścieżką seedingu).')}{' '}
          {L('To', 'This is')} <strong className="text-foreground">{L('nie', 'not')}</strong> {L('jest automatyczny odczyt wszystkich LP na Orca dla Twojego portfela.', 'an automatic read of all Orca LP positions for your wallet.')}{' '}
          {L('Pełny skan on-chain:', 'Full on-chain scan:')}{' '}
          <code className="text-[11px]">GET /api/v1/orca/positions-by-owner?owner=…</code> (strona Positions) albo CLI{' '}
          <code className="text-[11px]">orca-positions-list</code> (STARTUP / POSITION_REGISTRY).
        </li>
        <li>
          <strong className="text-foreground">Strategies</strong> — {L('rekordy tworzysz w API (UI lub REST); po restarcie API bez bazy mogą zniknąć, zależnie od konfiguracji.', 'records are created in API (UI or REST); after API restart without DB they may disappear, depending on configuration.')}
        </li>
        <li>
          <strong className="text-foreground">Pools</strong> — {L('publiczne API Orca; jeśli pusto, sprawdź sieć i czy Vite proxy trafia w działający backend (port zgodny z ', 'public Orca API; if empty, check network and whether Vite proxy targets a running backend (port matching ')}
          <code className="text-[11px]">API_PORT</code>{L(').', ').')}
        </li>
        <li>
          <strong className="text-foreground">Scripts</strong> — {L('wymaga', 'requires')}{' '}
          <code className="text-[11px]">CLMM_REPO_ROOT</code> {L('na hoście API wskazującego na to repo (manifest + historia runów).', 'on API host pointing to this repo (manifest + run history).')}
        </li>
      </ul>
      <p className="text-xs">
        <strong className="text-foreground">{L('Wolne ładowanie:', 'Slow loading:')}</strong>{' '}
        {L(
          'pierwsze wejście zawsze czeka na odpowiedź API/RPC; ponowne odwiedziny w tej samej sesji korzystają z cache React Query (kilka minut).',
          'first visit always waits for API/RPC response; repeated visits in the same session use React Query cache (few minutes).',
        )}
      </p>
    </div>
  )
}
