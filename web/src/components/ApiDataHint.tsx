/**
 * Explains why dashboard lists can be empty even with on-chain Orca positions.
 */
export default function ApiDataHint() {
  return (
    <div className="rounded-lg border border-amber-500/40 bg-amber-500/5 px-4 py-3 text-sm text-muted-foreground space-y-2">
      <p className="font-medium text-foreground">Skąd biorą się dane w tym panelu?</p>
      <ul className="list-disc pl-5 space-y-1 text-xs leading-relaxed">
        <li>
          <strong className="text-foreground">Positions / Wallet (agregaty)</strong> — lista pochodzi z{' '}
          <strong className="text-foreground">monitora w procesie API</strong> (pozycje dodane przy starcie strategii /
          bota albo inną ścieżką seedingu). To <strong className="text-foreground">nie</strong> jest automatyczny
          odczyt wszystkich LP na Orca dla Twojego portfela. Pełny skan on-chain:{' '}
          <code className="text-[11px]">GET /api/v1/orca/positions-by-owner?owner=…</code> (strona Positions) albo CLI{' '}
          <code className="text-[11px]">orca-positions-list</code> (STARTUP / POSITION_REGISTRY).
        </li>
        <li>
          <strong className="text-foreground">Strategies</strong> — rekordy tworzysz w API (UI lub REST); po restarcie
          API bez bazy mogą zniknąć, zależnie od konfiguracji.
        </li>
        <li>
          <strong className="text-foreground">Pools</strong> — publiczne API Orca; jeśli pusto, sprawdź sieć i czy
          Vite proxy trafia w działający backend (port zgodny z <code className="text-[11px]">API_PORT</code>).
        </li>
        <li>
          <strong className="text-foreground">Scripts</strong> — wymaga{' '}
          <code className="text-[11px]">CLMM_REPO_ROOT</code> na hoście API wskazującego na to repo (manifest + historia
          runów).
        </li>
      </ul>
      <p className="text-xs">
        <strong className="text-foreground">Wolne ładowanie:</strong> pierwsze wejście zawsze czeka na odpowiedź API/RPC;
        ponowne odwiedziny w tej samej sesji korzystają z cache React Query (kilka minut).
      </p>
    </div>
  )
}
