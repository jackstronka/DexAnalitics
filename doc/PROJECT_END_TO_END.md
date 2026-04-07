# Bociarz LP — co projekt robi end-to-end

## Cel i założenia

Projekt **CLMM Liquidity Provider (Bociarz LP)** jest narzędziem dla dostawców płynności (LP) na Solanie (CLMM), które ma:

- pomagać **nie tylko maksymalizować APY**, ale przede wszystkim **kwantyfikować ryzyko** (np. impermanent loss, drawdown, zachowanie ścieżki w czasie),
- wybierać i stroić **range/ticki** tak, aby decyzje miały uzasadnienie w historii,
- symulować historyczną skuteczność **tych samych strategii**, które docelowo mają działać na żywo,
- następnie umożliwić uruchomienie strategii jako **automatycznego bota** z kontrolą operacyjną i czytelnym audytem zdarzeń.

W warstwie produktowej projekt jest „local-first”: dane wejściowe są budowane i cache’owane lokalnie (snapshoty, swapy, dekodowania), a zewnętrzne feedy (Dune/Birdeye itp.) są traktowane jako opcjonalne uzupełnienie.

## Co jest w scope (protokóły i tryby)

Na poziomie danych i adapterów projekt obsługuje:

- **Orca Whirlpool**
- **Raydium CLMM**
- **Meteora DLMM** (bin-based)

Warstwa bota/wykonania jest obecnie opisana jako **Orca-first** (w runbookach i modelu operacyjnym), z jasnymi etapami rozwoju typu devnet → limited-live → standard-live.

## Artefakty i „źródła prawdy”

Projekt przechowuje dane w dwóch głównych kategoriach:

1) **Lokalne pliki historyczne (JSONL, append-only)** wykorzystywane do backtestów/optimize oraz do budowania modeli fee:

- snapshoty pooli:
  - `data/pool-snapshots/<protocol>/<pool>/snapshots.jsonl`
- strumień swapów (surowy i dekodowany):
  - `data/swaps/<protocol>/<pool>/swaps.jsonl`
  - `data/swaps/<protocol>/<pool>/decoded_swaps.jsonl`

2) **Baza danych (PostgreSQL)** używana przez API/dashboard (m.in. do przechowywania/obsługi części stanu i metadanych).

Dodatkowo, dla ciągłości operacyjnej i audytu kosztów/IL:

- lifecycle kosztów (tx fees i koszty w ledgerze):
  - domyślnie `data/ledger/orca_position_lifecycle.jsonl` (override: `CLMM_POSITION_LIFECYCLE_LEDGER_PATH`)
- IL ledger (opcjonalny plik do rekonstrukcji IL „po krokach”):
  - własny `il_ledger_path` (bot/exec zapisuje timeline zdarzeń)
- rejestr otwartych/zamkniętych pozycji (append-only):
  - `data/positions/registry.jsonl` (override: `CLMM_POSITION_REGISTRY_PATH`)

## End-to-end pipeline (dane → modele → decyzje → wykonanie)

### 1. Ingest: przygotowanie snapshotów (fee modelling Tier 2)

Snapshoty są cyklicznie zbierane dla curated puli (definiowanych w `STARTUP.md`). Celem snapshotów jest przygotowanie plików, które spełniają warunki „readiness” dla fee modelling typu:

- **Tier 1**: proxy udziału LP (share proxy)
- **Tier 2**: snapshot-heurystyki fee (eksperymentalne, ale docelowo domyślne)

Schemat „Tier 2 readiness” zależy od protokołu (np. pola `fee_growth_global_*` albo liczniki `protocol_fee_owed_*` dla Orca).

Kluczowe elementy workflow:

- `snapshot-run-curated-all` / komendy snapshotów dla Orca/Raydium/Meteora,
- `snapshot-readiness` (audit jakości coverage),
- w dalszym kroku backtest/optimize wybiera `--fee-source snapshots`.

### 2. Ingest: sync surowych swapów i dekodowanie (Tier 3 kierunek „on-chain truth”)

Projekt równolegle zbiera strumień swapów:

- `swaps-sync-curated-all` tworzy `swaps.jsonl` (surowe sygnatury/slot/timing),
- `swaps-enrich-curated-all` buduje `decoded_swaps.jsonl` (decoder).

Dekodowanie ma statusy jakości (np. `ok_traded_event`, `ok`, `partial`, `missing_*`) i jest audytowane komendą `swaps-decode-audit`.

W praktyce: swap-level fee truth jest opisane jako plan docelowy, natomiast snapshoty Tier 2 pozostają podstawowym modelem w pipeline’ie.

### 3. Analytics: backtest i backtest-optimize

Po przygotowaniu danych projekt uruchamia:

- `backtest` — symulację historyczną dla pojedynczej konfiguracji (range/strategia) z trackingiem pozycji,
- `backtest-optimize` — grid search range/strategii i ranking wyników pod wybraną funkcję celu (objective).

W `backtest-optimize` można przełączać źródła ceny i fee, np.:

- `--price-path-source snapshots`
- `--fee-source snapshots`

Wyniki służą do wybrania „zwycięzcy” (range + strategia) oraz do porównań w oknach czasu.

### 4. Analytics (alternatywnie): optimize (Monte Carlo / syntetyczne ścieżki)

Komenda `optimize` może używać wielu losowych ścieżek (Monte Carlo), aby dobrać parametry strategii w warunkach zmienności, a nie tylko w oparciu o jedną rzeczywistą ścieżkę ceny.

### 5. Strategy orchestration: apply optimize results i bot loop

W warstwie API/wykonania istnieją dwa typowe tryby odświeżania wyniku:

1) **In-process** w ramach `StrategyService` (API uruchamia cyklicznie `clmm-lp-cli backtest-optimize` i aplikuje JSON wynikowy),
2) **Zewnętrzny scheduler** (cron/Task Scheduler uruchamia CLI, a następnie aplikujesz wynik przez endpoint):
   - `POST /api/v1/strategies/{id}/apply-optimize-result`

API ma też mechanizm polityk „kto ma prawo apply”, aby operator/agent/bot nie walczyli ze sobą o te same zasoby.

### 6. Execution + audyt: ledgers, rebalans i rejestr pozycji

Podczas działania bota:

- zdarzenia finansowo-operacyjne są zapisywane do lifecycle ledgeru (tx fee, delta kosztów),
- (opcjonalnie) IL ledger zapisuje timeline w formie zdarzeń (`position_opened` → `rebalance` → `position_closed`),
- rejestr `registry.jsonl` pozwala kolektorom wiedzieć, które pozycje uznawać za otwarte w danym momencie.

Dla Orca runbook przewiduje dodatkowe mechanizmy spójności kosztów w sesji rebalansu:

- ustawienie `CLMM_REBALANCE_SESSION_ID` dla sekwencji „swap + tx rebalansu + open”.

### Tryby działania bota (operacyjne)

Bot działa w różnych trybach bezpieczeństwa i skali, z jasnym celem każdego trybu:

- **Dry-Run**: pętla decyzji działa, a działania są symulowane/śledzone bez zmiany stanu on-chain (monitorujesz cykle, lifecycle i decyzje).
- **Limited Live (Single-Market)**: prawdziwe wykonanie transakcji na małym kapitale i w ściśle kontrolowanych warunkach (na jednym rynku/puli), żeby potwierdzić stabilność ścieżki.
- **Standard Live**: normalna praca po zaliczeniu ograniczonego live (Orca-first w początkowych etapach, zgodnie z runbookiem i modelem operacyjnym).

### Strategie decyzyjne (kiedy rebalance)

Strategia (konfigurowany `StratConfig`/`DecisionConfig`) steruje tym, *jak często* i *z jakiego powodu* wykonywany jest rebalance (przesunięcie zakresu).

Semantyka strategii używanych w `backtest-optimize` obejmuje:

- `static`: brak rebalance (trzymasz początkowy zakres).
- `oor_recenter`: rebalance tylko po wyjściu poza zakres (OOR), a potem otwarcie nowego symetrycznego pasma wokół bieżącej ceny.
- `threshold_<N>%`: rebalance gdy cena jest OOR albo gdy w zakresie od mid odbiega o co najmniej `N%` (mid = środek pasma).
- `periodic_<N>h`: rebalance po upływie N godzin od ostatniego otwarcia/rebalance (w live domyślnie “po staremu” niezależnie od in-range; opcja `periodic_requires_out_of_range=true` ogranicza rebalance do OOR).
- `il_limit`: rebalance (i opcjonalnie close) gdy miary „IL-like” przekraczają progi.
- `retouch_shift`: hybrydowe „przesuwanie krawędzi wyjścia” po pierwszym OOR zamiast pełnego recenter; kolejne retouche mogą działać wg hybrydy czas + procent (flagami opisanymi w runbooku Orca).

Szczegóły semantyki: `doc/BACKTEST_OPTIMIZE_STRATEGIES.md`.

## Interfejsy użytkownika (CLI, API, WebSocket, Dashboard)

### CLI (najważniejsze grupy komend)

Pipeline ingest i analytics realizują komendy `clmm-lp-cli`, m.in.:

- snapshoty:
  - `... orca-snapshot-curated ...`, `... raydium-snapshot ...`, `... meteora-snapshot ...`
- swap pipeline (P1 → P1.1):
  - `swaps-sync-curated-all`, `swaps-enrich-curated-all`, `swaps-decode-audit`, `data-health-check`
- backtest/optimize:
  - `backtest`, `backtest-optimize`, `optimize`
- bot:
  - `orca-bot-run` / `orca-bot-open-and-run` (z trybami dry-run i limited-live/execute)

### REST API i WebSocket

API (`clmm-lp-api`) wystawia REST pod `/api/v1` oraz WebSocket:

- REST: endpointy do pozycji, strategii, pul i endpointy do budowania/submit transakcji unsigned+signed,
- WebSocket:
  - `GET /ws/positions`
  - `GET /ws/alerts`

Swagger/OpenAPI jest dostępne przez `/docs`.

### Dashboard (web)

Panel webowy (React/Vite/Tailwind) udostępnia operatorowi:

- portfolio (PnL, wartość, metryki),
- pozycje i ledger (timeline i koszty),
- strategie (tworzenie/konfiguracja i statusy),
- listy pul i podstawowe metryki,
- sekcję `scripts` (manifest + historia uruchomień + runner proxy).

## Operowanie i utrzymanie (supervision, alerty, Docker)

Projekt ma przygotowane warianty operacyjne:

- Linux:
  - `systemd` dla bota i API (ciągłość po restartach),
- Windows:
  - pętle PowerShell uruchamiane przez NSSM/Shawl lub Task Scheduler,
- Docker:
  - Docker Compose dla `postgres + api + web`,
  - przykłady konfiguracji pod uruchamianie bota jako kontenera.

Dla alertów danych przewidziano podejście „pragmatyczne”:

- skrypty health-checków (`snapshot_health_alert`, `quick_verify_alert`) i opcjonalny Slack digest,
- zasada: jeśli scheduler/collector przestaje działać, widać to w metrykach/ostrzeżeniach.

## Co jeszcze nie jest „pełną prawdą” (ważne ograniczenia)

- Fee „inside-range truth” na poziomie pozycji i swapów jest opisane jako roadmap (Tier 3), natomiast defaultowy pipeline opiera się na snapshotowym modellingu (Tier 2).
- W operacyjnej ciągłości IL są opisane znane luki/etapy domykania (np. onboarding/otwieranie pozycji i pełne domknięcie ścieżek historycznych).

## Pierwszy sensowny checkpoint (jak zacząć testowo)

Żeby zobaczyć end-to-end w praktyce, typowy progres to:

1. uruchomić Postgres i API,
2. zebrać snapshoty curated puli,
3. uruchomić swap sync + dekodowanie (opcjonalnie na początek, ale docelowo ważne),
4. wykonać `backtest-optimize` dla wybranego pary i okna czasu,
5. zaaplikować wynik do strategii (przez API lub cyklicznie),
6. uruchomić bota w trybie `dry-run` / limited-live i sprawdzić ledgers i rejestry.

