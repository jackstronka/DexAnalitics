# Warstwa decyzyjna i symulacje równoległe (orkiestrator LP)

**Status:** dokument **produktowo-architektoniczny** — konsolidacja ustaleń (2026-05); **implementacja** warstwy jako osobnego procesu/modułu nad istniejącymi crate’ami jest **planowana / częściowa** (patrz: co już jest w kodzie poniżej).

**keywords:** decision-layer, orchestrator, shadow positions, counterfactual, backtest, backtest-optimize, AgentDecision, apply-optimize-result, stream-pnl, IL, lineage, data-quality, simulation, ROADMAP, implementation-audit, operator-goals, requirements-mapping, capability-registry, kopalnia-wiedzy, NO-GO, IMPLEMENTATION_PLAN_DECISION_LAYER

**Powiązane (istniejące w repo):**

- [`AI_AGENT_LAYER.md`](AI_AGENT_LAYER.md) — co dziś znaczy „agent” (envelope optymalizacji, Position Agent, `DecisionEngine` live, `agent_decisions.jsonl`).
- [`PROJECT_OVERVIEW.md`](PROJECT_OVERVIEW.md) — apply optimize, polityki `optimize_apply_policy`, pipeline danych.
- [`ROADMAP.md`](ROADMAP.md) — shadow strategies per position, historia przypisań (blisko koncepcji „wirtualnych” wariantów).
- [`TODO_CHART_AGENT_LAYER.md`](TODO_CHART_AGENT_LAYER.md) — opcjonalna warstwa wykresu / wieloagentowa (**osobny profil**, nie ten dokument).
- [`ASYNC_COMMUNICATION_LAYER.md`](ASYNC_COMMUNICATION_LAYER.md) — event bus / skalowanie komunikacji decyzji (przyszłość).
- [`IMPERMANENT_LOSS_USD_AND_FEES.md`](IMPERMANENT_LOSS_USD_AND_FEES.md), stream-pnl / lineage w kodzie API — metryki ciągu pozycji.

---

## 1. Cel biznesowy

**Maksymalizować oczekiwany wynik ekonomiczny LP** (fees i inne przychody netto ryzyka, IL i kosztów operacyjnych), w oparciu o **lokalne dane on-chain** i **te same narzędzia symulacji**, których używa się do researchu.

Warstwa decyzyjna (orkiestrator) **najpierw** dostarcza **analizę, rekomendacje i pełny audyt**; **później** może być spięta z warstwą wykonawczą (otwarcie / zamknięcie / rebalance / apply siatki).

---

## 1a. Mapowanie celu operatora → wymagania w dokumentacji

Poniżej: odniesienie **Twojej wypowiedzi** (cel, orkiestrator, what-if, wolumen, pary, kapitał, log, fazy) do miejsc w **`DECISION_LAYER.md`** i **`FUNCTIONAL_SPECIFICATION.md` §8**.  
**Normatywny szczegół** (edge case’y, pola logu, API) — dopiero przy implementacji; tam gdzie brak osobnego akapitu = **szkic / intencja**, nie pełna specyfikacja zachowania.

| Element celu (skrót) | Gdzie opisane jako wymaganie / kierunek | Uwaga |
| -------------------- | ---------------------------------------- | ----- |
| Cel = wynik ekonomiczny LP (zysk netto ryzyka i kosztów) | §1 Cel biznesowy; §8 FUNCTIONAL **Goal** | Sformułowane ogólnie, bez progów zysku. |
| Analiza danych + metryki ciągu (m.in. IL) + dobór zakresów | §4 Wejścia, §5 Wyjścia, §8 Metryki; §8 FUNCTIONAL **Inputs** | IL/fees/ciąg: odsyłacze do stream-pnl / lineage / doc IL; **dobór zakresu** jako wynik optimize/backtest, nie osobna norma per tick. |
| Kilka **wirtualnych** pozycji (what-if) równolegle do realnej | §2 Terminologia, §6 faza 2, §7 | Zasada: ten sam silnik symulacji; brak jeszcze normy „ile wariantów max” (TBD w implementacji). |
| Monitor **rynku** (np. wolumen w czasie: rośnie / maleje) | §4 pkt 2 (snapshoty, swapy — proxy) | **Jawnego** wymogu „sygnał wolumen ↑↓” **nie** rozpisano osobno; wolumen/fees wynikają z danych lokalnych, szczegół **do doprecyzowania** w spec przy kodzie (np. definicja serii z `decoded_swaps`). |
| Inne pary → następny etap; inne projekty/DEX-y | §4 pkt 5, §6 faza 3 | Kierunek jest; brak listy kompletności danych per venue w jednym miejscu (TODO przy multi-pool). |
| Porównanie „ta para vs inna” (fees przy podobnym zakresie %) | §5 wyjścia (rekomendacja / ranking), §6 faza 3, §8 (normalizacja ryzyka) | **„Podobny zakres %”** jako warunek porównania — **wspomniane** w §8 jako kierunek (normalizacja), nie jako twarda definicja. |
| Warstwa korzysta z **backtestów** | §4.3, §6, §7 | Tak. |
| **Alokacja kapitału** (zwiększenie, druga pozycja na tej samej parze, **jeden ciąg** PnL/straty) | §2 Alokacja; §8 (agregacja legów); §8 FUNCTIONAL **Edge cases** | „Dwa legi = jeden ciąg” — w FUNCTIONAL §8 jako edge case + §8 tu; **brak** pełnej normy księgowej w jednym akapicie (należy dopiąć w specu przy modelu danych). |
| **Pełne logowanie** decyzji (co, dlaczego, na jakich danych) | §5 Wyjścia; §8 FUNCTIONAL **Outputs**, **Observability** | Kierunek + kanały możliwe; **schema pola** — TBD. |
| Najpierw **analiza/informowanie**, potem **wykonanie** | §1, §6 (fazy 1 vs 5); §8 FUNCTIONAL **Invariants** | Tak, spójnie z Twoją kolejnością. |
| Docelowo: open/close, zakresy, strategie, **budowa strategii z historii**, cały ciąg | §6 fazy 4–5; akapit o generatorze strategii | W FUNCTIONAL §8 tylko ogólnie w **Goal**; szczegół strategii = osobne sekcje speca / `BACKTEST_OPTIMIZE_STRATEGIES.md` przy implementacji. |

**Podsumowanie:** **Tak — intencja i większość bloków** jest w `DECISION_LAYER.md` + **stub normatywny** w `FUNCTIONAL_SPECIFICATION.md` §8. **Nie** — nie wszystko jest już **pełnymi requirements** (np. jawna semantyka wolumenu w czasie, twarde definicje „podobnego zakresu %”, pełny schemat logu orkiestratora). Przy pierwszym slice implementacji rozszerz §8 i ewentualnie dopisz podsekcję pod §4 (wejścia) dla sygnałów z volumenu.

---

## 1b. Rejestr zdolności („kopalnia” → narzędzia → cele)

**Cel sekcji:** jedna tabela, żeby warstwa decyzyjna (i operator) **wiedziały, co jest pod ręką**, **do czego to służy** i **kiedy nie uruchamiać** kolejnych kroków.  
**Źródło:** istniejące komendy CLI (`crates/cli/src/main.rs`), trasy API (`crates/api/src/routes.rs`), ścieżki danych z [`PROJECT_OVERVIEW.md`](PROJECT_OVERVIEW.md); **nie** wymyślone narzędzia.

**Utrzymanie:** przy dodaniu nowej komendy / endpointu istotnego dla orkiestratora — **dopisz wiersz** w tym samym PR co kod (albo osobny PR dokumentacyjny od razu po merge).

| Zasób / narzędzie | Przykładowe cele (co wspiera) | Jak wołać (CLI / API / pliki) | NO-GO / ostrożność (wysoki poziom) |
| ----------------- | ------------------------------ | ------------------------------ | ----------------------------------- |
| **Snapshoty pooli** (`snapshots.jsonl`) | Fee proxy w czasie, kontekst ceny/ticków pod backtest | Pliki: `data/pool-snapshots/{orca\|raydium\|meteora}/<pool>/…`; zasilanie: `cargo run --bin clmm-lp-cli -- snapshot-run-curated-all` (curated) | Zbyt stare snapshoty → **nie** porównuj rankingów „świeżych” decyzji bez odświeżenia; patrz health-check |
| **Swapy surowe + zdekodowane** | Wolumen / proxy fee z lokalnych swapów, kierunek | `… swaps-sync-curated-all …`; `… swaps-enrich-curated-all …`; `data/swaps/…/swaps.jsonl`, `decoded_swaps.jsonl` | Niski `% decode OK` lub puste okno → **NO-GO** na ranking między parami; najpierw enrich / audit |
| **`swaps-decode-audit`** | Raport jakości dekodowania | `cargo run --bin clmm-lp-cli -- swaps-decode-audit` (`--save-report`) | Wynik zły → tylko diagnostyka, nie „optymalizacja produkcyjna” |
| **`data-health-check`** | Staleness snapshotów + decode % | `cargo run --bin clmm-lp-cli -- data-health-check` (flagi min wiek / min decode — patrz `--help`) | `--fail-on-alert` pod CI/harmonogram; orkiestrator: **traktuj alert jako bramkę** |
| **`orchestrator-backtests-full`** | Gate (opcjonalnie) + macierz `backtest-optimize` przez API (`POST /backtests/full`) + audyt | `cargo run --bin clmm-lp-cli -- orchestrator-backtests-full --request-json …` (`CLMM_API_BASE_URL`, `--decisions-via-http`, `--save-job-json`, `--fail-on-no-go`, `--fail-on-job-partial`) | Wymaga działającego API + danych snapshotów zgodnych z FULL; długie runy — ustaw `--poll-timeout-secs` |
| **`orchestrator-gate`** | Ta sama bramka co `data-health-check` + **jeden** audytowy wiersz w `agent_decisions.jsonl` (`decision.kind`: `gate_health`, schema v1 — patrz [`FUNCTIONAL_SPECIFICATION.md`](FUNCTIONAL_SPECIFICATION.md) §8) | `cargo run --bin clmm-lp-cli -- orchestrator-gate` (`--max-age-minutes`, `--min-decode-ok-pct`, `--fail-on-no-go`, `--jsonl-out`, `CLMM_AGENT_DECISIONS_JSONL_PATH`, `--source`, `--chain-id`) | Domyślnie exit 0 nawet przy `no_go`; `--fail-on-no-go` dla harmonogramu; **nie** uruchamia `backtest-optimize` |
| **`ops-ingest-cycle`** | Jednorazowy łańcuch: snapshot → sync → enrich → audit → health | `cargo run --bin clmm-lp-cli -- ops-ingest-cycle` (parametry domyślne w `main.rs`) | Długi czas / RPC; ograniczaj `--limit`, limity sygnatur/dekodów |
| **`backtest`** | Pojedyncza symulacja zakresu / strategii na historii | `cargo run --bin clmm-lp-cli -- backtest …` | Brak danych w oknie → błąd lub pusty wynik; sprawdź przygotowanie snapshotów |
| **`backtest-optimize`** | Siatka wariantów + wybór zwycięzcy pod metrics | `cargo run --bin clmm-lp-cli -- backtest-optimize …`; wynik JSON `--optimize-result-json` | Ten sam co wyżej + koszt CPU; **nie** apply bez review / bez `AgentDecision` w fazie ostrożnej |
| **`POST …/apply-optimize-result`** | Wczytanie wyniku siatki do **działającego** executora strategii | `POST /api/v1/strategies/{id}/apply-optimize-result`; opcjonalnie envelope `AgentDecision` ([`AI_AGENT_LAYER.md`](AI_AGENT_LAYER.md)) | Strategia nieaktywna → 400; `optimization_busy` / polityka `optimize_apply_policy` → 409; patrz [`PROJECT_OVERVIEW.md`](PROJECT_OVERVIEW.md) |
| **`GET …/stream-pnl`** | Ciąg PnL / IL / cashflow w czasie dla pozycji | `GET /api/v1/positions/{address}/stream-pnl` | Brak danych / niepełna historia → metryki „best effort”; nie mylić z przyszłą prognozą |
| **`GET …/stream-lineage`** | Lineage / rebalance / fee zebrane w łańcuchu | `GET /api/v1/positions/{address}/stream-lineage` | Jak wyżej |
| **`GET …/agent/supervisor`** | Skrót koszt/wynik + scenariusze dla operatora | `GET /api/v1/positions/{address}/agent/supervisor` | To **podgląd** nadzoru, nie automatyczny apply optimize |
| **`GET`/`POST …/data/agent/decisions`** | Append / odczyt decyzji pod audyt orchestracji | `GET`/`POST /api/v1/data/agent/decisions`; plik `data/agent/agent_decisions.jsonl` | Pole `decision` jest ogólne JSON; dla runów bramki patrz schema **`gate_health`** w [`FUNCTIONAL_SPECIFICATION.md`](FUNCTIONAL_SPECIFICATION.md) §8 i [`doc/examples/orchestrator-run-v1.example.json`](examples/orchestrator-run-v1.example.json) |
| **Position Agent** (czat, skan, LLM) | Wyjaśnianie, sugestie przy pozycji | `/api/v1/positions/{address}/agent/…` ([`AI_AGENT_LAYER.md`](AI_AGENT_LAYER.md)) | Skan startowy = **szablony tekstowe** ([`DECISION_LAYER.md`](DECISION_LAYER.md) §11.2); nie zastępuje backtestu |
| **Dokumentacja + changelog** | Semantyka produktu, słowa kluczowe pod wyszukiwanie | `doc/DECISION_LAYER.md`, `doc/PROJECT_OVERVIEW.md`, `doc/FUNCTIONAL_SPECIFICATION.md`, `doc/ENGINEERING_NOTES.md` | Dla LLM: kontekst z **wersji** pliku / daty; bez tego halucynacja „starego” API |

**keywords (dla grep):** capability-registry, kopalnia-wiedzy, NO-GO, ops-ingest-cycle, data-health-check, orchestrator-gate, gate_health, orchestrator-backtests-full, api_backtests_full

---

## 2. Terminologia

| Termin | Znaczenie |
| ------ | --------- |
| **Warstwa decyzyjna / orkiestrator** | Proces lub moduł, który zbiera sygnały i metryki, stosuje **politykę**, wywołuje narzędzia (CLI/API), **loguje** decyzje z uzasadnieniem i **nie musi** sam wysyłać transakcji (faza 1). |
| **„Agent” (marketingowo)** | Często to samo + narracja autonomii lub LLM; w kodzie patrz [`AI_AGENT_LAYER.md`](AI_AGENT_LAYER.md). |
| **Warstwa wykonawcza** | `clmm-lp-execution`, bot, API akcji na pozycji — **już istnieje**; podłączenie pod orkiestrator to późniejsza faza. |
| **Warstwa symulacyjna** | **Wielokrotne uruchomienia tego samego silnika** backtestu/symulacji z różnymi parametrami (zakres, rozmiar, pool) na **tych samych lub zsynchronizowanych danych** — porównanie wyników, nie osobna „magia regułkowa”. |
| **Pozycje wirtualne / shadow / what-if** | Warianty **off-chain** (symulacja), utrzymywane **równolegle** do pozycji realnej, aby porównywać ścieżki ekonomiczne przy **jednakowej metodologii metryk**. |
| **Alokacja kapitału** | Wybór *gdzie* i *ile* trzymać w LP (para, pool, protokół), z uwzględnieniem kosztu przejścia i jakości danych. |

---

## 3. Stan obecny w repozytorium (klocki)

- **Reguły na pozycji (live):** `DecisionEngine` — hold / rebalance wg `StrategyMode` ([`AI_AGENT_LAYER.md`](AI_AGENT_LAYER.md) §1C).
- **Siatka + zastosowanie do executora:** `backtest-optimize`, `POST /api/v1/strategies/{id}/apply-optimize-result`, `AgentDecision`, `optimize_apply_policy`, `optimization_busy` ([`PROJECT_OVERVIEW.md`](PROJECT_OVERVIEW.md)).
- **Metryki ciągu / IL / fees / koszty:** m.in. stream PnL, lineage, dokumentacja IL — silny rdzeń analityczny; szczegóły w powiązanych docach i kodzie API.
- **Audyt decyzji (zewnętrzny / ogólny):** `GET`/`POST /api/v1/data/agent/decisions`, plik `data/agent/agent_decisions.jsonl`.
- **Asystent operatorski:** Position Agent (czat, supervisor, opcjonalny LLM) — **nie** zastępuje orkiestratora meta-decyzji.

**Luka:** brak **jednego** zdefiniowanego komponentu, który **systematycznie** łączy: ~~jakość danych~~ → wiele wariantów symulacji → ranking → rekomendacja → log (z opcjonalnym późniejszym apply / tx). **Stan (2026-05-14):** bramka jakości + append do `agent_decisions` jest w CLI (`orchestrator-gate`); nadal brak zautomatyzowanego rankingu wariantów po pozytywnym gate.

---

## 4. Wejścia orkiestratora (docelowo)

1. **Stan pozycji realnych** — otwarcia, zakresy, pool, protokół, historia ciągu.
2. **Dane rynkowe / proxy** — snapshoty, swapy, zdekodowane zdarzenia (tier jakości jawny).
3. **Symulacje** — wyniki backtestu / `backtest-optimize` na tych samych zasadach co research.
4. **Jakość danych** — m.in. wiek snapshotów, `% decode OK`, wyniki health-check — **bramki NO-GO** zamiast fałszywych rankingów między parami.
5. **Kolejny etap produktowy** — te same mechanizmy na **wiele par i protokołów** (wymaga spójnych serii danych per para).

---

## 5. Wyjścia orkiestratora

1. **Rekomendacja** — zostań / zmień zakres / rozważ inny rozmiar / rozważ inną parę lub venue (gdy dane na to pozwalają).
2. **Warianty liczbowe** — kilka scenariuszy z metrykami i **jawnymi założeniami**.
3. **Pełny log decyzji** — timestamp, sygnały wejściowe, wersja/kompletność danych, użyte narzędzia (np. ścieżka wyniku optimize), odrzucone alternatywy, powód.  
   Możliwe kanały: rozszerzenie `agent_decisions.jsonl` lub osobny strumień JSONL (do ustalenia przy implementacji).

---

## 6. Fazy wdrożenia

| Faza | Zachowanie | Uwaga |
| ---- | ---------- | ----- |
| **1 — Analiza i informowanie** | Tylko raporty / UI / alerty; **brak** automatycznego tx. | Buduje zaufanie do metryk i logów. |
| **2 — Shadow / what-if (ta sama para lub proste warianty)** | Równoległe **symulacje** (inny zakres %, inny rozmiar) vs real, **ten sam silnik** symulacji. | Niski narzut koncepcyjny; porównanie uczciwe przy wspólnej metodologii. |
| **3 — Rozszerzenie na wiele par / venue** | Ranking alokacji **tylko przy spełnionych bramkach jakości** danych dla każdej konkurencyjnej serii. | Tu rośnie złożoność źródeł, nie samej idei „wielu runów”. |
| **4 — Sugestie pod wykonanie** | Propozycje pod operatora lub pod `apply-optimize-result` / checklistę. | Wymaga twardych guardraili. |
| **5 — Wykonanie autonomiczne** | Open / close / rebalance / dobór strategii z limitami. | Najwyższe ryzyko; osobny Go/No-Go. |

**Generator strategii z historii** — raczej faza 4–5: kandydaci z danych + walidacja backtestem + polityka wdrożeń.

---

## 7. Symulacje równoległe — jak unikać „heurystyki od tak”

- **Zasada:** warianty wirtualne to **wielokrotne uruchomienie tego samego** silnika backtestu/symulacji z **innymi parametrami** na **tym samym oknie danych** (lub rolling), a nie osobny kod typu „jeśli volume↑ wybierz B”.
- **Niepewność modelu** (fee proxy, luki w swapach) jest **wspólna** dla wszystkich wariantów; **nie** udajemy precyzji — dokumentujemy i **gating** przy słabych danych.
- **Devnet:** **nie** służy do porównań ekonomicznych 1:1 z mainnetem (inny rynek); do testów integracji tx — tak. Porównania „gdzie zarobić więcej” — **dane mainnetu** (replay / read-only) + symulacja off-chain.
- **Wydajność:** ograniczona liczba aktywnych wariantów, wspólny cache okien danych, rzadsze przebiegi niż tick bota — szczegóły przy implementacji.

---

## 8. Metryki — co jest mocne, co domknąć pod porównania

**Mocny rdzeń (ciąg, IL, fees, lineage, stream):** patrz [`IMPERMANENT_LOSS_USD_AND_FEES.md`](IMPERMANENT_LOSS_USD_AND_FEES.md) i implementacje API.

**Pod uczciwe porównanie real vs shadow i między parami warto dopinać (konceptualnie):**

- **Wspólna definicja** punktu startu, waluty referencyjnej (np. USD) i kosztów tx w symulacji przejścia.
- **Koszt przejścia** przy zmianie pary/poolu (slippage, tx, czas poza rynkiem) — żeby shadow nie wyglądał przesadnie korzystnie.
- **Agregacja wielu legów** na tej samej parze („jeden logiczny book”) — jawna reguła w specu, żeby nie podwajać lub nie gubić fees/IL.
- **Normalizacja ryzyka** przy różnych parach (np. szerokość % ≠ to samo ryzyko) — proste filtry lub metryki pomocnicze; bez udawania pełnego modelu rynku.

---

## 9. Integracja z istniejącym „agentem”

- **`AgentDecision` + apply optimize** — gotowy **kanał wykonawczy** na wynik siatki, gdy orkiestrator (lub operator) zdecyduje się na apply ([`AI_AGENT_LAYER.md`](AI_AGENT_LAYER.md)).
- **LLM** — opcjonalnie tylko jako **komentarz / klasyfikacja** na już policzonych liczbach, nie jako źródło prawdy o rynku; zgodnie z priorytetem darmowych danych on-chain.

---

## 10. Następne kroki dokumentacyjne / spec

- **Plan implementacji (fazy 0–6+, PR-y):** [`IMPLEMENTATION_PLAN_DECISION_LAYER.md`](IMPLEMENTATION_PLAN_DECISION_LAYER.md).
- **§8** w [`FUNCTIONAL_SPECIFICATION.md`](FUNCTIONAL_SPECIFICATION.md) — draft normatywny; **gate:** schemat `gate_health` + przykład JSON (faza 1); dalej rozszerzać **Happy path / Invariants** przy fazie 2+ (wiele runów symulacji, ranking).
- **§1b** — przy nowej komendzie CLI / endpoincie używanym przez orkiestratora dopisz wiersz w rejestrze zdolności (ten sam PR co kod lub natychmiast po).
- Utrzymać spójność z [`ROADMAP.md`](ROADMAP.md) (shadow na pozycji) — zdecydować, czy shadow w roadmapie i „shadow symulacja orkiestratora” to **jeden** mechanizm storage, czy dwa powiązane byty.

---

## 11. Audyt: co jest w kodzie / co nie (bez zgadywania)

**Metoda (2026-05-13, aktualizacja 2026-05-14):** przegląd `crates/` (`rg` na `orchestrat`, `shadow`, `apply-optimize`, `BacktestOptimize`, `DataHealthCheck`, itd.) oraz `crates/api/src/routes.rs`. **Brak** dedykowanego modułu „LP orchestrator” spięcia *wiele runów symulacji → ranking → jeden log* — jeśli się pojawi, zaktualizuj tę tabelę w tym samym commicie. **Jest** minimalny gate runner: `orchestrator-gate` → `orchestrator_gate.rs`.

### 11.1 Zaimplementowane (istniejące ścieżki)

| Obszar | Co jest | Dowód (orientacyjnie) |
| ------ | ------- | ---------------------- |
| Reguły live na pozycji | `DecisionEngine` / `StrategyMode` → hold vs rebalance | `crates/execution/src/strategy/decision.rs`, użycie w `crates/execution/src/strategy/executor.rs` |
| Siatka + apply do executora | `backtest-optimize` (CLI), wczytanie wyniku do `DecisionConfig` | `crates/cli/src/main.rs` (`BacktestOptimize`, …), `crates/api/src/services/optimization_runner.rs` (`apply_optimize_result_*`) |
| HTTP apply + envelope | `POST …/apply-optimize-result`, `AgentDecision`, busy per strategia | `crates/api/src/handlers/strategies.rs`, `crates/api/src/routes.rs`, `crates/api/src/state.rs` (`optimization_busy`), `crates/domain/src/agent_decision.rs` |
| Cykl optimize w API | `StrategyService` + subproces CLI wg interwału | `crates/api/src/services/strategy_service.rs` |
| Jakość danych + log gate (CLI) | `data-health-check`, `orchestrator-gate` (append JSONL), `orchestrator-backtests-full` (API FULL + audyt) | `crates/cli/src/main.rs` (`DataHealthCheck`, `OrchestratorGate`, `OrchestratorBacktestsFull`), `crates/cli/src/orchestrator_gate.rs`, `crates/cli/src/orchestrator_api_full.rs`, `crates/cli/src/swap_sync.rs` |
| Audyt dekodowania swapów (CLI) | `swaps-decode-audit` | `crates/cli/src/main.rs` (`SwapsDecodeAudit`) |
| Automatyzacja ingestu (CLI) | `ops-ingest-cycle` (łańcuch snapshot → sync → enrich → audit → health) | `crates/cli/src/main.rs` (`OpsIngestCycle`) |
| Log decyzji (append) | `GET`/`POST /api/v1/data/agent/decisions` → `data/agent/agent_decisions.jsonl` | `crates/api/src/routes.rs`, `crates/api/src/handlers/data.rs` |
| Metryki ciągu / nadzór | `stream-pnl`, `stream-lineage`, supervisor | `crates/api/src/routes.rs` (`/positions/{address}/stream-pnl`, `…/stream-lineage`, `…/agent/supervisor`), handlery w `crates/api/src/handlers/` |
| Position Agent (MVP) | Czat, skan, worker w tle, opcjonalny LLM | `crates/api/src/handlers/agent.rs`, `crates/api/src/services/position_agent_service.rs`, `crates/api/src/services/position_agent_llm.rs`, `crates/api/src/server.rs` (`spawn_position_agent_background_worker`) |
| Event bus (szkielet) | Tryb shadow dla brokera zdarzeń (nie = shadow LP z roadmapy) | `crates/api/src/state.rs` (`event_bus_shadow_mode`), `crates/api/src/events.rs` |

### 11.2 Nie zaimplementowane jako produkt „warstwy decyzyjnej / orkiestrator LP”

| Obszar | Stan | Uwaga |
| ------ | ---- | ----- |
| **Jeden komponent orkiestratora** (pełna pętla: NO-GO → wiele symulacji → ranking → log) | **Częściowo** | `orchestrator-gate` + `orchestrator-backtests-full` (API `POST /backtests/full` + audyt); ranking „poza FULL” / real vs sym — dalej w planie. |
| **Shadow / what-if w sensie DECISION_LAYER** (równoległe warianty kapitału/zakresu **sterowane** przez warstwę decyzyjną) | Brak | `backtest`/`backtest-optimize` można uruchamiać **ręcznie** w wielu konfiguracjach; nie ma zautomatyzowanego pętlowego porównywania „real vs N wirtualnych” pod jednym harmonogramem. |
| **„Wiele strategii na jednej pozycji: 1 live + N shadow”** ([`ROADMAP.md`](ROADMAP.md) §40–51) | **Roadmap**, nie znaleziono w kodzie produktowej ścieżki | `rg` po `crates/` nie zwraca modelu „shadow strategy” przy jednej pozycji (poza testami pomocniczymi lineage, patrz niżej). |
| **Pełny log decyzji orkiestratora** (wejścia + wersje danych + odrzucone warianty) | Kanał `agent_decisions` + eventy Position Agent; `gate_health` + `inputs_ref`; **`api_backtests_full`** (skrót joba FULL + opcjonalnie `--save-job-json`) | Hashe treści plików — **opcjonalnie / TBD** |
| **Skan Position Agent oparty na liczbach z danych** | Tekstowe rekomendacje szablonowe | `scan_recommendations` w `crates/api/src/services/position_agent_service.rs` — stałe stringi, nie podpięty backtest ani pliki snapshotów. |

### 11.3 Nie mylić z „shadow” w roadmapie

- W `crates/api/src/services/position_stream_lineage.rs` istnieją **struktury/testy** z nazwą „shadow” (np. `to_shadow`, golden `lineage_shadow_*`) — to **porównanie lineage do testów**, nie roadmapowy produkt „1 live + N shadow strategies” na pozycji.

### 11.4 Utrzymanie tej sekcji

Przy każdym merge, który dodaje **orkiestrator**, **automatyczne shadow runy** albo **model live+shadow z roadmapy**, zaktualizuj **§11.1 / §11.2** w tym samym PR i dopisz `keywords:` w [`ENGINEERING_NOTES.md`](ENGINEERING_NOTES.md).

---

## Document status

| Field | Value |
| ----- | ----- |
| Role | Product + architecture contract for decision / orchestration layer |
| Created | 2026-05-13 |
| Maintainer | team — update when implementation milestones land |
