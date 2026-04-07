# Pierwsze uruchomienie: 3 pozycje whETH/SOL — strategie i dziennik

**Wariant 2 pozycje / ~10 USD deploy, zakres ~25–25,5 SOL/whETH:** [`WHETH_SOL_TWO_BOTS_10USD.md`](WHETH_SOL_TWO_BOTS_10USD.md).

## Cel

- Jedna para (**whETH/SOL**, pool `HktfL7iwGKT5QHjywQkcDnZXScoh811k7akrMZJkCcEF`), **trzy osobne pozycje (NFT)** z **podobnym kapitałem startowym** (te same capy `--amount-a` / `--amount-b` przy openie).
- Każdy proces `orca-bot-run` dostaje **inny profil decyzji** (`--optimize-result-json`), żeby porównać zachowanie rebalansów w tych samych warunkach rynkowych.
- **Pierwsze uruchomienie:** priorytetem jest **obserwowalność i kontrola**, nie maksymalizacja zysku.

Powiązane: [`doc/BOT_OPERATIONS_MODEL_2026-03-23.md`](BOT_OPERATIONS_MODEL_2026-03-23.md) (tryby dry-run / limited live), [`doc/ORCA_RUNBOOK.md`](ORCA_RUNBOOK.md) (RPC, ledger, `CLMM_REBALANCE_SESSION_ID`), [`doc/POSITION_REGISTRY.md`](POSITION_REGISTRY.md) (rejestr pozycji).

## Dostępne tryby strategii (skrót)

Źródło semantyki: `StrategyMode` w `crates/execution/src/strategy/decision.rs`. W pliku wyniku optymalizacji (`optimize-result` JSON) pole `winner.strategy_kind` mapuje się przez `decision_config_from_optimize_result` (`crates/execution/src/optimize_profile.rs`):

| `strategy_kind` (JSON) | Zachowanie (intuicja) | Uwagi przy pierwszym teście |
|------------------------|------------------------|-----------------------------|
| `oor_recenter` | Rebalans **tylko gdy cena jest poza zakresem** (OOR), potem centrowanie na aktualnej cenie | Zwykle **mniej transakcji** niż okresowy; dobra „baza ostrożna”. |
| `periodic` | Rebalans co **`periodic_interval_hours`** (domyślnie **tylko gdy pozycja jest poza zakresem**) | Przewidywalny rytm; **koszt tx** może być wyższy przy krótkim interwale. Ustaw `periodic_requires_out_of_range=false`, jeśli chcesz “zegar niezależnie od in-range”. |
| `threshold` | OOR → rebalans; w zakresie → rebalans gdy odchylenie od **środka zakresu** ≥ `threshold_ratio` | Łączy reakcję na wyjście z pasma i „przeciągnięcie” środka. |
| `retouch_shift` | Poza zakresem: **jeden** krok „retouch” krawędzi (logika `RetouchShift` + bramka `retouch_armed` w executorze) | Inna geometria niż pełne centrowanie; **obserwuj** pierwsze OOR na sucho (dry-run). |
| `il_limit` | Progi IL (close/rebalance) | Na start często **trudniejszy** do interpretacji niż OOR/periodic; zostaw na później albo osobny eksperyment. |
| `static` | Bez rebalansu (`StaticRange`) — raczej **nie** do tego eksperymentu (brak porównania rebalansów). | — |

## Propozycja zestawu A / B / C (do skopiowania w dziennik)

**Konserwatywny pierwszy zestaw (łatwe porównanie):**

| Slot | Rola | `strategy_kind` | Intuicja |
|------|------|-----------------|----------|
| **A** | Ostrożna baza | `oor_recenter` | Reagujesz dopiero po wyjściu ceny z pasma. |
| **B** | Rytm czasu | `periodic` | Widzisz „koszt utrzymania” strategii czasowej przy tym samym kapitale. |
| **C** | Środek + OOR | `threshold` | Więcej ruchu w zakresie niż sama baza OOR. |

Alternatywa: zamień **C** na `retouch_shift`, jeśli chcesz porównać **retouch** vs pełne **recentrowanie** po OOR — wtedy miej na oku logi przy pierwszym evencie poza zakresem.

Każdy slot musi mieć **własny** plik `--optimize-result-json` (ten sam schemat `OptimizeResultFile`, ten sam `winner.width_pct` co przy openach, żeby szerokość pasma była spójna).

## Skąd bierze się optimize: CLI vs API (ważne)

| Co | Gdzie działa | Uwagi |
|----|----------------|--------|
| **Siatka `backtest-optimize`** (porównanie strategii + `width_pct`, zapis `--optimize-result-json`) | **Tylko `clmm-lp-cli`** | Tu jest faktyczna symulacja historyczna na danych (snapshots / ścieżka ceny itd.). |
| **API `clmm-lp-api`** | **Nie** liczy optimize wewnątrz Rusta | Może **uruchomić ten sam** CLI jako **subproces** (`StrategyService`: `optimize_command`, `optimize_on_start`, `optimize_interval_secs` → `crates/api/src/services/optimization_runner.rs`) albo **wczytać gotowy JSON** przez `POST /strategies/{id}/apply-optimize-result`. |
| **Dashboard `POST /api/v1/analytics/simulate`** | **Symulacja `clmm_lp_simulation`** | Pobiera fee/tick z RPC, **syntetyczna ścieżka GBM** (nie replay snapshotów); do rankingów jak CLI `backtest-optimize` nadal używaj **`backtest-optimize`** na lokalnych danych. Szczegóły: `methodology_note` w odpowiedzi, kod: `crates/api/src/services/simulation_analytics.rs`. |

**Wniosek:** do **doboru zakresu i parametrów** liczących się naukowo używasz **CLI** (`backtest`, `backtest-optimize`). API jest wygodne do **cyklicznego** odpalania optimize w tle albo do **aplikowania** już wygenerowanego pliku na strategię w API — ale źródłem prawdy dla siatki nadal jest **binarka CLI**.

## Symulacja doboru zakresu dla botów A / B / C

**Cel:** wybrać **`winner.width_pct`** (i ewent. progi czasu / `threshold_ratio`) **przed** trzema openami, na tej samej parze i możliwie tym samym oknie historii, żeby porównanie było uczciwe.

1. **Dane:** zgodnie z [`doc/ORCA_RUNBOOK.md`](ORCA_RUNBOOK.md) — snapshoty + (w miarę możliwości) dekodowane swapy dla puli whETH/SOL; ewent. `snapshot-backtest-prep` dla szybszego okna (`--prepared-snapshot-window`).
2. **Jedna wspólna szerokość (zalecane na start):** uruchom **`backtest-optimize`** z siatką strategii obejmującą `oor_recenter`, `periodic`, `threshold` (i ten sam zestaw `width_pct` / okresów / progów). Wybierz **jedną** wartość `width_pct` z wierszy zwycięskich (albo świadomie **trzy różne** pliki, jeśli chcesz porównać też szerokości — wtedy w dzienniku zapisz, że porównujesz „strategia + width”).
3. **Wyjście:** dla każdego slotu zapisz **`--optimize-result-json`** (np. `data/experiments/wheth-sol/winner-A.json` itd.). W pliku: `winner.strategy_kind`, `winner.width_pct`, oraz pola zależne (`periodic_interval_hours`, `threshold_ratio`).
4. **Open na mainnecie:** przy `orca-position-open` / skryptach ustaw **`RangeWidthPct`** (lub odpowiednik) **zgodny** z `winner.width_pct` z danego pliku — wtedy ticki wyliczy CLI od **aktualnej** ceny, ale **szerokość** odpowiada backtestowi.
5. **Weryfikacja:** jednorazowo możesz odpalić **`backtest`** (pojedynczy przebieg) z tymi samymi parametrami na tym samym oknie, żeby zobaczyć zachowanie jednej konfiguracji przed live (bez gwarancji zysku, ale spójność ścieżki danych).

**Czy optimize „wystarczy”?** Daje **ranking wśród założeń modelu** (fee proxy, okno czasu, szerokość). Nie zastępuje **MAINNET min. rozmiaru pozycji** ani przyszłej zmienności — po starcie live i tak walidujesz preflightem i pierwszymi dniami obserwacji.

## Bezpieczna kolejność uruchomienia

1. **Kapitał i RPC:** `tools/orca_wheth_sol_three_bots_plan.ps1` + [`doc/MAINNET_OPERATIONAL_CHECKLIST.md`](MAINNET_OPERATIONAL_CHECKLIST.md).
2. **3× open** z **identycznymi** `AmountA` / `AmountB` / `RangeWidthPct` (jak w planie); zapisz **trzy adresy pozycji (PDA)** w dzienniku poniżej.
3. **Dry-run:** dla każdej pozycji odpal `orca-bot-run` **bez** `--execute` (lub z domyślnym dry-run), z właściwym `--optimize-result-json`, min. kilka cykli — sprawdź, że decyzje mają sens (`Hold` / `Rebalance` / powody w logach).
4. **Jedna pozycja na raz — live:** włącz `--execute` + `--keypair` tylko dla **jednego** bota; obserwuj jedną sesję; potem drugi, potem trzeci. Unikasz nakładania się pierwszych nieprzewidywalnych tx przy tym samym portfelu i uczysz się jednej strategii naraz.
5. **Ledger / sesja:** przed pierwszym live ustaw **`CLMM_REBALANCE_SESSION_ID`** (np. `wheth3-2026-03-31-A`) i zmień na unikalne ID przy kolejnym bocie — ułatwi sumowanie kosztów w `data/ledger/orca_position_lifecycle.jsonl` (patrz ORCA_RUNBOOK).

## Identyfikacja procesów (żeby się nie pomylić)

- **Nazwa okna / tytuł sesji:** np. `WHETH_SOL-A-oor`, `WHETH_SOL-B-periodic`, `WHETH_SOL-C-threshold`.
- **Plik logu:** przekieruj stdout do pliku per bot (`Start-Transcript` albo `orca_bot_run_supervised.ps1` jeśli używasz).
- **Ścieżka JSON strategii:** trzymać w repo lub w `data/experiments/wheth-sol-2026/` **poza sekretami** (sam plik optimize nie zawiera klucza).

## Szablon dziennika (wklej do Notion / Markdown / wydrukuj)

### Nagłówek eksperymentu

- **Data startu (UTC):**
- **Operator:**
- **Pool:** `HktfL7iwGKT5QHjywQkcDnZXScoh811k7akrMZJkCcEF` (whETH/SOL)
- **Szacowany kapitał na pozycję (plan):** AmountA lamportów: ______ / AmountB whETH raw: ______ / szerokość: ______ %
- **Rezerwa SOL na fee (lamporty):** ______
- **RPC (primary):** ______

### Tabela pozycji ↔ strategie

| Slot | Etykieta | Position PDA | Plik `optimize-result.json` (ścieżka) | `strategy_kind` | Start dry-run (czas) | Start execute (czas) | Uwagi |
|------|----------|--------------|----------------------------------------|-----------------|----------------------|----------------------|-------|
| A | | | | | | | |
| B | | | | | | | |
| C | | | | | | | |

### Dziennik zdarzeń (krótkie wpisy)

| Data (UTC) | Slot | Zdarzenie | Decyzja / tx | Notatka (1–2 zdania) |
|------------|------|-----------|--------------|----------------------|
| | | np. pierwszy `Rebalance` | sygnatura / dry-run | |
| | | RPC timeout | | |
| | | Stop operatora | | |

### Checklist review (koniec dnia / tygodnia)

- [ ] Czy trzy pozycje nadal zgodne z rejestrem (`registry.jsonl` / `POSITION_REGISTRY`)?
- [ ] Czy powody rebalansu w logach zgadzają się z trybem (`Periodic` vs `RangeExit` vs `Optimization` vs `RetouchShift`)?
- [ ] Czy suma opłat tx mieści się w założeniach?
- [ ] Czy któryś bot wymaga **interwencji** (stop / zmiana `eval_interval_secs`)?

## Kiedy zatrzymać („czerwone linie”)

- Powtarzające się błędy tx lub **pętla** nieudanych prób.
- **Rozjazd** stanu pozycji względem tego, co pokazuje monitor (wtedy nie dokładaj kolejnego `--execute`).
- RPC niestabilny — najpierw stabilizacja endpointów, potem wznów (patrz tryb Warning w `BOT_OPERATIONS_MODEL`).

## Skrót komend (referencja)

- Bot (po uzupełnieniu ścieżek):  
  `cargo run --bin clmm-lp-cli -- orca-bot-run --position <PDA> --optimize-result-json <plik.json> --eval-interval-secs 300 --poll-interval-secs 30`  
  Live: dopisz `--execute --keypair <json>` zgodnie z CLI.
- Plan kapitału: `tools/orca_wheth_sol_three_bots_plan.ps1` (bez tx).

---

**keywords:** WHETH_SOL, orca-bot-run, StrategyMode, first run, operations journal, optimize-result-json
