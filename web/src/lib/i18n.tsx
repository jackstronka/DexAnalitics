import { createContext, useContext, useEffect, useMemo, useState, type ReactNode } from 'react'

export type Locale = 'pl' | 'en'

type Dictionary = Record<string, string>

const LS_LOCALE_KEY = 'clmm.locale'

const pl: Dictionary = {
  'nav.dashboard': 'Dashboard',
  'nav.wallet': 'Portfel',
  'nav.walletLedger': 'Dziennik portfela',
  'nav.swap': 'Swap',
  'nav.positions': 'Pozycje',
  'nav.closed': 'Zamknięte',
  'nav.strategies': 'Strategie',
  'nav.backtests': 'Backtesty',
  'nav.dataQuality': 'Jakość danych',
  'nav.pools': 'Pule',
  'nav.scripts': 'Skrypty',
  'nav.logs': 'Logi',
  'nav.botActivity': 'Aktywność bota',
  'nav.settings': 'Ustawienia',
  'layout.apiPending': 'API…',
  'layout.apiMissing': 'Brak API',
  'layout.apiOk': 'API OK',
  'layout.langLabel': 'Język',
  'wallet.title': 'Portfel',
  'wallet.subtitle':
    'Dwie warstwy danych: (1) on-chain saldo dla wybranego portfela (read-only RPC), (2) agregaty USD z /analytics/portfolio (monitor pozycji) — mogą być 0, jeśli monitor nie ma pozycji.',
  'wallet.walletsTitle': 'Portfele (pliki keypair na hoście API)',
  'wallet.walletsHint':
    'Lista portfeli pochodzi z plików keypair na hoście API (jeden plik JSON = jeden wpis). Saldo i Orca na dole dotyczą wybranego przyciskiem portfela — nie musi to być ten sam co przy transferze.',
  'wallet.noWalletsHint':
    'Brak portfeli w skonfigurowanym katalogu na hoście API. Dodaj pliki *.json albo utwórz portfel poniżej i zrestartuj API jeśli trzeba.',
  'wallet.transferFrom': 'Z portfela',
  'wallet.transferTo': 'Do portfela / odbiorcy',
  'wallet.transferToCustom': 'Inny adres (wklej poniżej)',
  'wallet.recipientPubkey': 'Adres odbiorcy (pubkey)',
  'wallet.lamportsLabel': 'Lamports',
  'wallet.lamportsTooltip':
    '1 SOL = 1 000 000 000 lamportów — najmniejsza jednostka natywnego SOL na Solanie.',
  'wallet.solPreview': 'SOL (podgląd)',
  'wallet.quickAmounts': 'Szybkie kwoty',
  'wallet.transferHistory': 'Ostatnie transfery (lokalny log)',
  'walletLedger.title': 'Dziennik portfela (GL)',
  'walletLedger.subtitle':
    'Append-only JSONL z API: pending / confirmed / failed dla swap-before-open, open, transfer SOL, convert SOL. Nie jest źródłem sald — tylko audyt operacji.',
  'walletLedger.refresh': 'Odśwież',
  'walletLedger.filters': 'Filtry',
  'walletLedger.ownerFilter': 'Filtr owner (substring)',
  'walletLedger.ownerPlaceholder': 'opcjonalnie pubkey…',
  'walletLedger.limit': 'Limit',
  'walletLedger.filePath': 'Plik',
  'walletLedger.empty': 'Brak zdarzeń (albo plik jeszcze nie istnieje).',
  'walletLedger.colTime': 'Czas',
  'walletLedger.colStatus': 'Status',
  'walletLedger.colKind': 'Rodzaj',
  'walletLedger.colOwner': 'Owner',
  'walletLedger.colCorr': 'Correlation',
  'walletLedger.colSig': 'Sygnatura',
  'walletLedger.colDeltas': 'Deltas (mint / raw)',
  'walletLedger.colErr': 'Błąd',
  'walletLedger.linkFromWallet': 'Dziennik operacji',
  'wallet.currentWallet': 'Aktualny portfel',
  'wallet.copy': 'Kopiuj',
  'wallet.onChainTitle': 'Saldo on-chain (read-only)',
  'wallet.loading': 'Ładowanie salda…',
  'wallet.tokenCount': 'Tokeny SPL',
  'wallet.showZeros': 'Pokaż zera',
  'wallet.hideZeros': 'Ukryj zera',
  'wallet.noTokens': 'Brak tokenów SPL (lub chwilowy problem RPC).',
  'wallet.totalValue': 'Total value',
  'wallet.netPnl': 'Net PnL',
  'wallet.feesUsd': 'Fees (USD)',
  'wallet.ilAvg': 'IL (avg %)',
  'wallet.openPositions': 'Otwarte pozycje',
  'wallet.allPositions': 'Wszystkie pozycje',
  'positions.title': 'Pozycje',
  'positions.refresh': 'Odśwież',
  'positions.openPosition': 'Otwórz pozycję',
  'positions.monitoredTitle': 'Monitorowane pozycje (API)',
  'positions.loading': 'Ładowanie...',
  'positions.notLinked': 'Niepodpięta',
  'positions.checking': 'Sprawdzanie…',
  'positions.pendingTitle': 'Zamknięte przez bota, oczekują na reopen',
  'positions.pendingEmpty': 'Brak oczekujących close->open.',
  'positions.remove': 'Usuń',
  'positions.removing': 'Usuwanie...',
  'positions.onchainTitle': 'Pozycje Orca on-chain (RPC)',
  'positions.loadOnchain': 'Wczytaj on-chain',
  'swap.title': 'Swap',
  'swap.modeOrca': 'Tryb Orca',
  'swap.modeJupiter': 'Tryb Jupiter',
  'swap.orcaDescription':
    'Tryb Orca: wykonuje swap ExactIn w wybranej puli Whirlpool przez backend (ten sam mechanizm co „swap-before-open”).',
  'swap.jupiterDescription': 'Jupiter deep-link ma prefill mintów i kwoty (ExactIn).',
  'swap.swapNow': 'Swap teraz (Orca pool)',
  'swap.swapping': 'Swapowanie…',
  'backtests.title': 'Backtesty',
  'backtests.subtitle': 'Pełne porównanie strategii i parametrów dla okien 24/48/72/96h.',
  'backtests.timeWindows': 'Okna czasowe (h)',
  'backtests.pairs': 'Pary',
  'backtests.strategies': 'Strategie (rodziny + parametry)',
  'backtests.qualifying': 'Strategie spełniające target',
  'backtests.liquidityRegime': 'Reżim płynności (orientacyjnie)',
  'backtests.targetPerWindow': 'Target per okno',
  'backtests.globalTop': 'Globalny ranking TOP (cały run)',
  'backtests.customWindowHours': 'Własne okno (h)',
  'backtests.customWindowPlaceholder': 'np. 168 dla 7 dni',
  'backtests.dataConsistencySelected': 'Spójność danych (wybrane pule + warianty):',
  'backtests.calculating': 'liczę...',
  'backtests.unavailable': 'brak odczytu',
  'backtests.recommended': 'zalecane',
  'backtests.hardLimit': 'twardy limit',
  'backtests.customWindowExceedsHard': 'Własne okno przekracza twardy limit spójności danych.',
  'backtests.snapshotDataVariant': 'Wariant danych snapshot',
  'backtests.snapshotVariantHint':
    'Możesz zaznaczyć oba warianty jednocześnie, żeby porównać strategie 10m vs 5m w jednym runie.',
  'backtests.parameters': 'parametry',
  'backtests.whatItDoes': 'Co robi:',
  'backtests.trigger': 'Trigger:',
  'backtests.whenToUse': 'Kiedy używać:',
  'backtests.risk': 'Ryzyko:',
  'backtests.howToReadParams': 'Jak czytać parametry',
  'backtests.objective': 'Objective',
  'backtests.exampleUseCase': 'Przykład użycia',
  'backtests.lpShareMeteoraOptional': 'LP share (Meteora, opcjonalnie)',
  'backtests.simAmountUsd': 'Kwota symulacji (USD)',
  'backtests.targetVsHodlUsd': 'Cel vs HODL (USD)',
  'backtests.targetVsHodlPlaceholder': 'np. 50 (zostaw puste = bez filtra)',
  'backtests.targetVsHodlHint':
    'Pokazujemy tylko strategie spelniajace warunek: vs_hodl >= target.',
  'backtests.includeIndicatorStrategies': 'Dodaj strategie wskaznikowe (Bollinger + Last Candle)',
  'backtests.includeIndicatorsHint':
    'Dodaje do siatki optimize: Bollinger (6 presetów: k = 1.5/2.0/2.5 x rebalance co 4/8h; API mapuje to na kroki dla 10m/5m) oraz Last Candle (14 presetów różnych okien świecy i częstotliwości rebalansu). To zwiększa liczbę testowanych konfiguracji i czas liczenia.',
  'backtests.strategyGridConfig': 'Konfiguracja parametrow strategii (grid)',
  'backtests.applyPresetTitle': 'Ustaw preset',
  'backtests.staticDeviationHelp':
    'Opcjonalny stały zakres dla `static`: X oznacza `entry * (1±X%)` (np. 10). Gdy ustawione, siatka width jest spinana do jednego wariantu. Używane dla wielu par.',
  'backtests.eg10': 'np. 10',
  'backtests.staticManualTitleHelp':
    'Manualny zakres static działa tylko przy jednej wybranej parze. Gdy podasz poprawny lower/upper, ma priorytet nad static_deviation_pct.',
  'backtests.staticManualHelp':
    'Ręczny zakres ceny dla `static` (dwa inputy). Jest użyty tylko gdy wybrana jest jedna para.',
  'backtests.staticManualIgnoredMultiPool':
    'Wybrano wiele par: ręczny `lower/upper` nie będzie użyty. Dla tego runu działa `static_deviation_pct`.',
  'backtests.oorRecenterDeviationHelp':
    'Osobny stały zakres dla `oor_recenter`: X oznacza `entry * (1±X%)`. Użyj przy porównaniu `static` vs `oor_recenter` na tej samej szerokości.',
  'backtests.thresholdGridHelp':
    'Progi (%) dla strategii threshold; nizsze = czestsze triggerowanie.',
  'backtests.retouchOffsetTitle':
    'Offset procentowy wzgledem ceny OOR (w % ceny, nie w jednostkach absolutnych). 0 = nowy zakres dotyka ceny; +0.1 przesuwa zakres o +0.1% (w prawo), -0.1 o -0.1% (w lewo).',
  'backtests.retouchOffsetExample':
    'Przyklad: zakres startowy 98-100, cena OOR=101, width bez zmian; offset 0 => zakres dotyka 101. Offset +0.1 => caly zakres przesuniety o +0.1% ceny.',
  'backtests.starting': 'Uruchamiam...',
  'backtests.runFullComparison': 'Uruchom FULL porownanie',
  'backtests.autoTuneBg': 'Auto-Tune (background)',
  'backtests.intervalMin': 'Interwał (min)',
  'backtests.startAutoTune': 'Start Auto-Tune',
  'backtests.stopAutoTune': 'Stop Auto-Tune',
  'backtests.status': 'Status',
  'backtests.running': 'running',
  'backtests.stopped': 'stopped',
  'backtests.note': 'notatka',
  'backtests.latestWinner': 'Najnowszy winner',
  'backtests.qualifyingDesc':
    'Szybki przeglad: TOP 3 warianty z kazdej rodziny strategii dla kazdej pary i okna. Wynik finansowy (PnL, vs HODL) w kolorze: zieleń = na plusie, czerwień = na minusie; kwoty w USD z separatorem tysięcy.',
  'backtests.sortTopBy': 'Sortuj TOP wg:',
  'backtests.noStrategiesMeetCondition': 'Brak strategii spelniajacych warunek.',
  'backtests.liquidityRegimeDesc': 'Kontekst wolumenu z biezacego API Orca dla pooli użytych w rankingu.',
  'backtests.approxVolumeWindow': 'Szac. wolumen (okno)',
  'backtests.regime': 'Reżim',
  'backtests.targetPerWindowDesc':
    'Szybki podglad: ile strategii przechodzi target i jaka jest mediana vs HODL.',
  'backtests.targetPass': 'target pass',
  'backtests.medianVsHodl': 'mediana vs HODL',
  'backtests.globalTopDesc': 'Agregacja przez wszystkie pary i okna czasowe.',
  'backtests.appearancesHint':
    'Wystapienia = liczba wariantow (strategia + inny range) policzonych w calym runie.',
  'backtests.strategy': 'Strategia',
  'backtests.rank': 'Miejsce',
  'dataQuality.title': 'Jakość danych',
  'dataQuality.subtitle':
    'Statystyki spójności snapshotów dla backtestów: kompletność, największa luka i maksymalne okno cofnięcia.',
  'dataQuality.aggregateMinimum': 'Agregat (minimalne okno od teraz):',
  'dataQuality.safe': 'bezpieczne',
  'dataQuality.maximum': 'maksymalne',
  'dataQuality.source': 'Źródło:',
  'dataQuality.overallStatus': 'Status agregatu:',
  'dataQuality.statusCounts': 'Liczniki statusów:',
  'dataQuality.rangeTitle': 'Zakres analizy',
  'dataQuality.rangeStart': 'Od',
  'dataQuality.rangeEnd': 'Do',
  'dataQuality.rangeReset72h': 'Ustaw ostatnie 72h',
  'dataQuality.rangeHint': 'Domyślnie analizujemy ostatnie 72h; możesz podać własny zakres dat.',
  'dataQuality.rangeInvalid': 'Nieprawidłowy zakres: data "Od" musi być <= "Do".',
  'dataQuality.sourceDbFresh': 'DB (świeże)',
  'dataQuality.sourceFallback': 'Fallback (skan JSONL)',
  'dataQuality.staleDbRows': 'Stare rekordy DB:',
  'dataQuality.thresholdsTitle': 'Aktywne progi z API',
  'dataQuality.maxGapNote':
    'Uwaga: "Największa luka (min)" = największa pojedyncza luka (nie suma luk) w analizowanym ciągu danych od teraz wstecz.',
  'dataQuality.detailsTitle': 'Szczegóły per pula',
  'dataQuality.loading': 'Ładowanie...',
  'dataQuality.loadError': 'Błąd odczytu danych.',
  'dataQuality.colPool': 'Pula',
  'dataQuality.colVariant': 'Wariant',
  'dataQuality.colStatus': 'Status',
  'dataQuality.colCoverage': 'Kompletność %',
  'dataQuality.colMaxGap': 'Największa luka (min)',
  'dataQuality.colSafeLookback': 'Bezpieczne okno od teraz (h)',
  'dataQuality.colMaxLookback': 'Maksymalne okno od teraz (h)',
  'dataQuality.colContinuousFrom': 'Ciągłe dane od',
  'dataQuality.colLatestPoint': 'Najnowszy punkt',
  'dataQuality.tipPool': 'Pula i para handlowa.',
  'dataQuality.tipVariant': 'Wariant snapshotu: 10m albo 5m.',
  'dataQuality.tipStatus': 'Status operacyjny: ok, degraded, recovering albo missing.',
  'dataQuality.tipCoverage':
    'Kompletność danych w ciągłym oknie od teraz wstecz (im bliżej 100%, tym lepiej).',
  'dataQuality.tipMaxGap':
    'Największa pojedyncza luka czasu między snapshotami (w minutach), nie suma luk.',
  'dataQuality.tipSafeLookback':
    'Bezpieczny lookback od teraz (godziny), wg bardziej restrykcyjnych progów jakości.',
  'dataQuality.tipMaxLookback':
    'Maksymalny lookback od teraz (godziny), wg mniej restrykcyjnej granicy ciągłości.',
  'dataQuality.tipContinuousFrom':
    'Najstarszy timestamp ciągłego okna danych od teraz; starsze dane mogą być po dużej luce.',
  'dataQuality.tipLatestPoint':
    'Najnowszy timestamp wykryty w pliku snapshotów dla tej puli i wariantu.',
  'dataQuality.status.ok': 'OK',
  'dataQuality.status.degraded': 'Degraded',
  'dataQuality.status.recovering': 'Recovering',
  'dataQuality.status.missing': 'Missing',
  'positionDetail.title': 'Szczegóły pozycji',
  'positionDetail.info': 'Informacje o pozycji',
  'positionDetail.performance': 'Wyniki',
  'positionDetail.automation': 'Automatyzacja strategii (ta pozycja)',
  'positionDetail.tabLedger': 'Logi / rebalanse',
  'positionDetail.tabChainHistoryPostgres': 'Historia (Postgres)',
  'positionDetail.positionHistoryPostgres': 'Historia pozycji (Postgres)',
  'positionDetail.lineageStreamOnlyIntro':
    'Ta zakładka używa wyłącznie GET …/stream-lineage (przeliczenie na żądanie). Porównaj z zakładką „Historia (Postgres)”.',
  'positionDetail.chainHistoryPgApiIntro':
    'Ta zakładka używa wyłącznie GET …/chain-history (odczyt zmaterializowanych wierszy w Postgresie). HTTP 404 = brak zapisu dla tego anchoru i trybu metryk.',
  'positionDetail.chainHistoryPgStreamFallbackBanner':
    'Brak jeszcze zmaterializowanego łańcucha w Postgresie dla tej pozycji — poniżej ten sam wynik co GET …/stream-lineage (przeliczanie na żądanie). Żeby zapisać snapshot w Postgresie, użyj „Odśwież zapis w Postgres”.',
  'positionDetail.chainHistoryPgStreamFallbackApiIntro':
    'Ten widok pokazuje dane jak GET …/stream-lineage (compute on read), bo w Postgresie nie ma jeszcze wierszy chain-history dla tego PDA. Po odświeżeniu materializacji wróci odczyt wyłącznie z Postgresa.',
  'positionDetail.chainHistoryPgChainHelp':
    'Dane zapisane przez writera (triggery po mutacjach / refresh). Semantyka wierszy jak w stream-lineage; źródło odczytu to Postgres, nie przeliczanie IL edges w locie.',
  'positionDetail.chainHistoryPgHintDb':
    'To jest błąd połączenia z bazą (zwykle HTTP 503): proces `clmm-lp-api` nie ma działającego Postgresa. Ustaw `DATABASE_URL` w środowisku tego procesu (np. `.env` w katalogu repo — skrypt startowy 8081 wczytuje tę zmienną), upewnij się że usługa PostgreSQL działa, zrestartuj API i sprawdź logi (connect / migrate).',
  'positionDetail.chainHistoryPgHint404':
    'To jest brak zapisu w Postgres (zwykle HTTP 404): dla tego anchoru i trybu metryk nie ma jeszcze zmaterializowanego łańcucha. Użyj przycisku „Odśwież zapis w Postgres” poniżej (albo `POST …/chain-history/refresh` / CLI), ewentualnie poczekaj na automatyczną materializację po operacji na pozycji. Po `git pull` zrestartuj `clmm-lp-api` — mapowanie PDA → zapis w Postgresie jest po stronie serwera.',
  'positionDetail.chainHistoryPgHintGeneric':
    'Jeśli komunikat powyżej jest niejasny, sprawdź log `clmm-lp-api`, proxy Vite → backend (port 8081) oraz czy `GET …/chain-history` i `GET …/health` zwracają spójny stan.',
  'positionDetail.chainHistoryPgMaterializedLabel': 'Zmaterializowano (Postgres):',
  'positionDetail.chainHistoryPgRefresh': 'Odśwież zapis w Postgres',
  'positionDetail.chainHistoryPgRefreshHint':
    'POST …/chain-history/refresh przelicza lineage jak stream-lineage i nadpisuje wiersze (może potrwać do ~2 min). Jeśli API ma CLMM_CHAIN_HISTORY_REFRESH_SECRET, ustaw to samo w web jako VITE_CHAIN_HISTORY_REFRESH_SECRET.',
  'positionDetail.chainHistoryPgStaleVsStream':
    'Łańcuch w Postgresie jest krótszy niż bieżący wynik stream-lineage — snapshot jest prawdopodobnie nieaktualny. Użyj odświeżenia powyżej.',
  'positionDetail.positionHistory': 'Historia pozycji',
  'positionDetail.lineageReadBadgePostgres': 'odczyt: Postgres (materializacja)',
  'positionDetail.lineageReadBadgeCompute': 'odczyt: przeliczane (stream-lineage)',
  'positionDetail.lineageHistoryApiIntro':
    'UI najpierw próbuje GET …/chain-history (zapis w Postgresie); przy braku danych lub błędzie używa GET …/stream-lineage.',
  'positionDetail.openLedgerTabHintBefore': 'Otwórz zakładkę ',
  'positionDetail.openLedgerTabHintAfter': ', aby zobaczyć surowe wiersze.',
}

const en: Dictionary = {
  'nav.dashboard': 'Dashboard',
  'nav.wallet': 'Wallet',
  'nav.walletLedger': 'Wallet ledger',
  'nav.swap': 'Swap',
  'nav.positions': 'Positions',
  'nav.closed': 'Closed',
  'nav.strategies': 'Strategies',
  'nav.backtests': 'Backtests',
  'nav.dataQuality': 'Data quality',
  'nav.pools': 'Pools',
  'nav.scripts': 'Scripts',
  'nav.logs': 'Logs',
  'nav.botActivity': 'Bot activity',
  'nav.settings': 'Settings',
  'layout.apiPending': 'API…',
  'layout.apiMissing': 'API unavailable',
  'layout.apiOk': 'API OK',
  'layout.langLabel': 'Language',
  'wallet.title': 'Wallet',
  'wallet.subtitle':
    'Two data layers: (1) on-chain balance for selected wallet (read-only RPC), (2) USD aggregates from /analytics/portfolio (position monitor) — may be 0 if monitor has no positions.',
  'wallet.walletsTitle': 'Wallets (keypair files on API host)',
  'wallet.walletsHint':
    'Wallet list comes from keypair files on the API host (one JSON file = one row). Balance/Orca below follow the wallet selected via buttons — it may differ from transfer source.',
  'wallet.noWalletsHint':
    'No wallets in the configured directory on the API host. Add *.json files or create a wallet below and restart the API if needed.',
  'wallet.transferFrom': 'From wallet',
  'wallet.transferTo': 'To wallet / recipient',
  'wallet.transferToCustom': 'Other address (paste below)',
  'wallet.recipientPubkey': 'Recipient pubkey',
  'wallet.lamportsLabel': 'Lamports',
  'wallet.lamportsTooltip':
    '1 SOL = 1,000,000,000 lamports — the smallest unit of native SOL on Solana.',
  'wallet.solPreview': 'SOL (preview)',
  'wallet.quickAmounts': 'Quick amounts',
  'wallet.transferHistory': 'Recent transfers (local log)',
  'walletLedger.title': 'Wallet ledger (GL-style)',
  'walletLedger.subtitle':
    'Append-only JSONL from the API: pending / confirmed / failed for swap-before-open, open, SOL transfer, SOL↔WSOL convert. Not a balance source — operational audit only.',
  'walletLedger.refresh': 'Refresh',
  'walletLedger.filters': 'Filters',
  'walletLedger.ownerFilter': 'Owner filter (substring)',
  'walletLedger.ownerPlaceholder': 'optional pubkey…',
  'walletLedger.limit': 'Limit',
  'walletLedger.filePath': 'File',
  'walletLedger.empty': 'No events yet (or the file does not exist).',
  'walletLedger.colTime': 'Time',
  'walletLedger.colStatus': 'Status',
  'walletLedger.colKind': 'Kind',
  'walletLedger.colOwner': 'Owner',
  'walletLedger.colCorr': 'Correlation',
  'walletLedger.colSig': 'Signature',
  'walletLedger.colDeltas': 'Deltas (mint / raw)',
  'walletLedger.colErr': 'Error',
  'walletLedger.linkFromWallet': 'Operation ledger',
  'wallet.currentWallet': 'Current wallet',
  'wallet.copy': 'Copy',
  'wallet.onChainTitle': 'On-chain balance (read-only)',
  'wallet.loading': 'Loading balance…',
  'wallet.tokenCount': 'SPL tokens',
  'wallet.showZeros': 'Show zero balances',
  'wallet.hideZeros': 'Hide zero balances',
  'wallet.noTokens': 'No SPL tokens found (or temporary RPC issue).',
  'wallet.totalValue': 'Total value',
  'wallet.netPnl': 'Net PnL',
  'wallet.feesUsd': 'Fees (USD)',
  'wallet.ilAvg': 'IL (avg %)',
  'wallet.openPositions': 'Open positions',
  'wallet.allPositions': 'All positions',
  'positions.title': 'Positions',
  'positions.refresh': 'Refresh',
  'positions.openPosition': 'Open position',
  'positions.monitoredTitle': 'Monitored positions (API)',
  'positions.loading': 'Loading...',
  'positions.notLinked': 'Not linked',
  'positions.checking': 'Checking…',
  'positions.pendingTitle': 'Closed by bot, waiting for reopen',
  'positions.pendingEmpty': 'No pending close->open items.',
  'positions.remove': 'Remove',
  'positions.removing': 'Removing...',
  'positions.onchainTitle': 'On-chain Orca positions (RPC)',
  'positions.loadOnchain': 'Load on-chain',
  'swap.title': 'Swap',
  'swap.modeOrca': 'Orca mode',
  'swap.modeJupiter': 'Jupiter mode',
  'swap.orcaDescription':
    'Orca mode: executes ExactIn swap in selected Whirlpool pool through backend (same mechanism as swap-before-open).',
  'swap.jupiterDescription': 'Jupiter deep-link pre-fills token mints and amount (ExactIn).',
  'swap.swapNow': 'Swap now (Orca pool)',
  'swap.swapping': 'Swapping…',
  'backtests.title': 'Backtests',
  'backtests.subtitle': 'Full comparison of strategies and parameters for 24/48/72/96h windows.',
  'backtests.timeWindows': 'Time windows (h)',
  'backtests.pairs': 'Pairs',
  'backtests.strategies': 'Strategies (families + parameters)',
  'backtests.qualifying': 'Strategies meeting target',
  'backtests.liquidityRegime': 'Liquidity regime (approximate)',
  'backtests.targetPerWindow': 'Target per window',
  'backtests.globalTop': 'Global TOP ranking (full run)',
  'backtests.customWindowHours': 'Custom window (h)',
  'backtests.customWindowPlaceholder': 'e.g. 168 for 7 days',
  'backtests.dataConsistencySelected': 'Data consistency (selected pools + variants):',
  'backtests.calculating': 'calculating...',
  'backtests.unavailable': 'unavailable',
  'backtests.recommended': 'recommended',
  'backtests.hardLimit': 'hard limit',
  'backtests.customWindowExceedsHard': 'Custom window exceeds hard data-consistency limit.',
  'backtests.snapshotDataVariant': 'Snapshot data variant',
  'backtests.snapshotVariantHint':
    'You can select both variants at once to compare 10m vs 5m strategy results in a single run.',
  'backtests.parameters': 'parameters',
  'backtests.whatItDoes': 'What it does:',
  'backtests.trigger': 'Trigger:',
  'backtests.whenToUse': 'When to use:',
  'backtests.risk': 'Risk:',
  'backtests.howToReadParams': 'How to read parameters',
  'backtests.objective': 'Objective',
  'backtests.exampleUseCase': 'Example use case',
  'backtests.lpShareMeteoraOptional': 'LP share (Meteora, optional)',
  'backtests.simAmountUsd': 'Simulation amount (USD)',
  'backtests.targetVsHodlUsd': 'Target vs HODL (USD)',
  'backtests.targetVsHodlPlaceholder': 'e.g. 50 (leave empty = no filter)',
  'backtests.targetVsHodlHint':
    'Only strategies meeting condition are shown: vs_hodl >= target.',
  'backtests.includeIndicatorStrategies': 'Include indicator strategies (Bollinger + Last Candle)',
  'backtests.includeIndicatorsHint':
    'Adds to optimize grid: Bollinger (6 presets: k = 1.5/2.0/2.5 x rebalance every 4/8h; API maps this to steps for 10m/5m) and Last Candle (14 presets with different candle windows and rebalance frequency). This increases the number of tested configurations and runtime.',
  'backtests.strategyGridConfig': 'Strategy parameter configuration (grid)',
  'backtests.applyPresetTitle': 'Apply preset',
  'backtests.staticDeviationHelp':
    'Optional fixed range for `static`: X means `entry * (1±X%)` (e.g. 10). When set, width grid is pinned to one variant. Used for many pairs.',
  'backtests.eg10': 'e.g. 10',
  'backtests.staticManualTitleHelp':
    'Manual static range works only with one selected pair. When valid lower/upper is provided, it takes priority over static_deviation_pct.',
  'backtests.staticManualHelp':
    'Manual price range for `static` (two inputs). It is used only when one pair is selected.',
  'backtests.staticManualIgnoredMultiPool':
    'Multiple pairs selected: manual `lower/upper` is ignored. `static_deviation_pct` is used for this run.',
  'backtests.oorRecenterDeviationHelp':
    'Separate fixed range for `oor_recenter`: X means `entry * (1±X%)`. Use when comparing `static` vs `oor_recenter` at the same width.',
  'backtests.thresholdGridHelp':
    'Threshold (%) for threshold strategy; lower values trigger more often.',
  'backtests.retouchOffsetTitle':
    'Percent offset relative to OOR price (% of price, not absolute units). 0 = range edge touches price; +0.1 shifts range by +0.1% (right), -0.1 by -0.1% (left).',
  'backtests.retouchOffsetExample':
    'Example: starting range 98-100, OOR price=101, unchanged width; offset 0 => range touches 101. Offset +0.1 => whole range shifted by +0.1% of price.',
  'backtests.starting': 'Starting...',
  'backtests.runFullComparison': 'Run FULL comparison',
  'backtests.autoTuneBg': 'Auto-Tune (background)',
  'backtests.intervalMin': 'Interval (min)',
  'backtests.startAutoTune': 'Start Auto-Tune',
  'backtests.stopAutoTune': 'Stop Auto-Tune',
  'backtests.status': 'Status',
  'backtests.running': 'running',
  'backtests.stopped': 'stopped',
  'backtests.note': 'note',
  'backtests.latestWinner': 'Latest winner',
  'backtests.qualifyingDesc':
    'Quick overview: TOP 3 variants from each strategy family for each pair and window. Financial outcome (PnL, vs HODL) is color-coded: green = positive, red = negative; USD amounts include thousands separators.',
  'backtests.sortTopBy': 'Sort TOP by:',
  'backtests.noStrategiesMeetCondition': 'No strategies satisfy the condition.',
  'backtests.liquidityRegimeDesc': 'Volume context from current Orca API for pools used in ranking.',
  'backtests.approxVolumeWindow': 'Approx volume (window)',
  'backtests.regime': 'Regime',
  'backtests.targetPerWindowDesc':
    'Quick view: how many strategies pass target and median vs HODL.',
  'backtests.targetPass': 'target pass',
  'backtests.medianVsHodl': 'median vs HODL',
  'backtests.globalTopDesc': 'Aggregation across all pairs and time windows.',
  'backtests.appearancesHint':
    'Appearances = number of variants (strategy + different range) computed in full run.',
  'backtests.strategy': 'Strategy',
  'backtests.rank': 'Rank',
  'dataQuality.title': 'Data quality',
  'dataQuality.subtitle':
    'Snapshot consistency stats for backtests: coverage, maximum gap, and maximum lookback window.',
  'dataQuality.aggregateMinimum': 'Aggregate (minimum lookback from now):',
  'dataQuality.safe': 'safe',
  'dataQuality.maximum': 'maximum',
  'dataQuality.source': 'Source:',
  'dataQuality.overallStatus': 'Aggregate status:',
  'dataQuality.statusCounts': 'Status counters:',
  'dataQuality.rangeTitle': 'Analysis window',
  'dataQuality.rangeStart': 'From',
  'dataQuality.rangeEnd': 'To',
  'dataQuality.rangeReset72h': 'Set last 72h',
  'dataQuality.rangeHint': 'By default we analyze the last 72h; you can enter a custom date range.',
  'dataQuality.rangeInvalid': 'Invalid range: "From" must be <= "To".',
  'dataQuality.sourceDbFresh': 'DB (fresh)',
  'dataQuality.sourceFallback': 'Fallback (JSONL scan)',
  'dataQuality.staleDbRows': 'Stale DB rows:',
  'dataQuality.thresholdsTitle': 'Active API thresholds',
  'dataQuality.maxGapNote':
    'Note: "Max gap (min)" is the largest single gap (not a sum of gaps) in the analyzed sequence from now backwards.',
  'dataQuality.detailsTitle': 'Per-pool details',
  'dataQuality.loading': 'Loading...',
  'dataQuality.loadError': 'Failed to load data.',
  'dataQuality.colPool': 'Pool',
  'dataQuality.colVariant': 'Variant',
  'dataQuality.colStatus': 'Status',
  'dataQuality.colCoverage': 'Coverage %',
  'dataQuality.colMaxGap': 'Max gap (min)',
  'dataQuality.colSafeLookback': 'Safe lookback from now (h)',
  'dataQuality.colMaxLookback': 'Maximum lookback from now (h)',
  'dataQuality.colContinuousFrom': 'Continuous data from',
  'dataQuality.colLatestPoint': 'Latest point',
  'dataQuality.tipPool': 'Pool and trading pair.',
  'dataQuality.tipVariant': 'Snapshot variant: 10m or 5m.',
  'dataQuality.tipStatus': 'Operational state: ok, degraded, recovering, or missing.',
  'dataQuality.tipCoverage':
    'Data completeness in the continuous lookback window from now (closer to 100% is better).',
  'dataQuality.tipMaxGap': 'Largest single time gap between snapshots (minutes), not cumulative.',
  'dataQuality.tipSafeLookback':
    'Safe lookback from now (hours), using stricter quality thresholds.',
  'dataQuality.tipMaxLookback':
    'Maximum lookback from now (hours), using a less strict continuity boundary.',
  'dataQuality.tipContinuousFrom':
    'Oldest timestamp of the continuous window from now; older data may be behind a large gap.',
  'dataQuality.tipLatestPoint':
    'Latest timestamp detected in snapshot file for this pool and variant.',
  'dataQuality.status.ok': 'OK',
  'dataQuality.status.degraded': 'Degraded',
  'dataQuality.status.recovering': 'Recovering',
  'dataQuality.status.missing': 'Missing',
  'positionDetail.title': 'Position details',
  'positionDetail.info': 'Position info',
  'positionDetail.performance': 'Performance',
  'positionDetail.automation': 'Strategy automation (this position)',
  'positionDetail.tabLedger': 'Logs / rebalances',
  'positionDetail.tabChainHistoryPostgres': 'History (Postgres)',
  'positionDetail.positionHistoryPostgres': 'Position history (Postgres)',
  'positionDetail.lineageStreamOnlyIntro':
    'This tab uses GET …/stream-lineage only (compute on read). Compare with the “History (Postgres)” tab.',
  'positionDetail.chainHistoryPgApiIntro':
    'This tab uses GET …/chain-history only (materialized rows in Postgres). HTTP 404 means no stored rows for this anchor and metrics mode.',
  'positionDetail.chainHistoryPgStreamFallbackBanner':
    'No Postgres materialized chain-history for this position yet — below is the same result as GET …/stream-lineage (compute on read). Use “Refresh Postgres snapshot” to persist rows.',
  'positionDetail.chainHistoryPgStreamFallbackApiIntro':
    'This view matches GET …/stream-lineage because there is no chain-history row in Postgres for this PDA yet. After a successful materialize refresh, reads switch back to Postgres-only.',
  'positionDetail.chainHistoryPgChainHelp':
    'Rows are written by the chain-history writer (triggers after mutations / refresh). Row semantics match stream-lineage; the read path is Postgres, not live IL-edge compute.',
  'positionDetail.chainHistoryPgHintDb':
    'This is a database connectivity issue (usually HTTP 503): the `clmm-lp-api` process has no working Postgres connection. Set `DATABASE_URL` for that process (e.g. repo `.env` — the :8081 start script loads it), ensure PostgreSQL is running, restart the API, and check logs (connect / migrate).',
  'positionDetail.chainHistoryPgHint404':
    'This is a missing snapshot (usually HTTP 404): there is no materialized chain-history yet for this anchor/mode. Use “Refresh Postgres snapshot” below (or `POST …/chain-history/refresh` / CLI), or wait for automatic materialization after a position-changing operation. After `git pull`, restart `clmm-lp-api` — PDA→Postgres row mapping is server-side.',
  'positionDetail.chainHistoryPgHintGeneric':
    'If the message above is unclear, check the `clmm-lp-api` log, the Vite proxy → backend (port 8081), and whether `GET …/chain-history` vs `GET …/health` look consistent.',
  'positionDetail.chainHistoryPgMaterializedLabel': 'Materialized at (Postgres):',
  'positionDetail.chainHistoryPgRefresh': 'Refresh Postgres snapshot',
  'positionDetail.chainHistoryPgRefreshHint':
    'POST …/chain-history/refresh recomputes lineage like stream-lineage and overwrites rows (can take up to ~2 minutes). If the API sets CLMM_CHAIN_HISTORY_REFRESH_SECRET, set the same value in the web app as VITE_CHAIN_HISTORY_REFRESH_SECRET.',
  'positionDetail.chainHistoryPgStaleVsStream':
    'The Postgres chain is shorter than the current stream-lineage result — the snapshot is likely stale. Use refresh above.',
  'positionDetail.positionHistory': 'Position history',
  'positionDetail.lineageReadBadgePostgres': 'read: Postgres (materialized)',
  'positionDetail.lineageReadBadgeCompute': 'read: live compute (stream-lineage)',
  'positionDetail.lineageHistoryApiIntro':
    'The UI tries GET …/chain-history (Postgres) first; on missing rows or errors it falls back to GET …/stream-lineage.',
  'positionDetail.openLedgerTabHintBefore': 'Open the ',
  'positionDetail.openLedgerTabHintAfter': ' tab to see raw rows.',
}

const dictByLocale: Record<Locale, Dictionary> = { pl, en }

type I18nContextValue = {
  locale: Locale
  setLocale: (next: Locale) => void
  t: (key: string, fallback?: string) => string
}

const I18nContext = createContext<I18nContextValue | null>(null)

export function I18nProvider({ children }: { children: ReactNode }) {
  const [locale, setLocaleState] = useState<Locale>(() => {
    if (typeof window === 'undefined') return 'pl'
    const raw = window.localStorage.getItem(LS_LOCALE_KEY)
    return raw === 'en' ? 'en' : 'pl'
  })

  useEffect(() => {
    if (typeof window === 'undefined') return
    window.localStorage.setItem(LS_LOCALE_KEY, locale)
  }, [locale])

  const value = useMemo<I18nContextValue>(() => {
    const dict = dictByLocale[locale]
    const setLocale = (next: Locale) => setLocaleState(next)
    const t = (key: string, fallback?: string) => dict[key] ?? fallback ?? key
    return { locale, setLocale, t }
  }, [locale])

  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>
}

export function useI18n() {
  const ctx = useContext(I18nContext)
  if (!ctx) throw new Error('useI18n must be used within I18nProvider')
  return ctx
}

