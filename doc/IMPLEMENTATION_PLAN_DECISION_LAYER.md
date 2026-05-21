# Plan implementacji: świadomość środowiska + orkiestrator LP

**keywords:** decision-layer, orchestrator, implementation-plan, capability-registry, NO-GO, data-health-check, backtest-optimize, agent_decisions, observability, phased-rollout

**Powiązane:** [`DECISION_LAYER.md`](DECISION_LAYER.md), [`FUNCTIONAL_SPECIFICATION.md`](FUNCTIONAL_SPECIFICATION.md) §8, [`PROJECT_OVERVIEW.md`](PROJECT_OVERVIEW.md), [`ROADMAP.md`](ROADMAP.md), [`AGENT_ROLLING_MEMORY_PLAN.md`](AGENT_ROLLING_MEMORY_PLAN.md), [`MASTER_IMPLEMENTATION_PLAN.md`](MASTER_IMPLEMENTATION_PLAN.md) (analiza), [`IMPLEMENTATION_PLAN.md`](IMPLEMENTATION_PLAN.md) (**backlog PR — start tutaj**).

**Cel planu:** przejść od **„wiem, co mnie otacza i jakie mam narzędzia”** do **„potrafię w sposób powtarzalny ocenić sytuację, zasymulować warianty i zalogować decyzję”** — **najpierw bez autonomicznych transakcji** (zgodnie z fazą 1 w `DECISION_LAYER.md`).

---

## 0. Zasady nadrzędne (nie do negocjacji przy implementacji)

1. **Źródło prawdy ekonomicznej:** dane mainnetu + lokalne pliki (nie devnet jako substytut rynku).
2. **Symulacje what-if:** ten sam silnik `backtest` / `backtest-optimize` co w researchu; brak ukrytych regułek „volume↑ → wybierz B” poza silnikiem.
3. **Bramki NO-GO:** jeśli `data-health-check` / audyt decode mówi „źle” → **brak** rankingu między parami i brak apply (tylko log + alert).
4. **Powtarzalność:** każdy przebieg orkiestratora zapisuje **wejścia** (wersje plików / zakres czasu / flagi CLI) i **wynik** tak, żeby dało się odtworzyć decyzję z logu.
5. **Rozdział od Position Agent:** asystent UI zostaje; orkiestrator to **osobna ścieżka** deterministyczna (można później tylko **czytać** ten sam log w UI).

---

## Faza 0 — Kontrakt logu i ewentualny manifest (documentation-first)

**Status (2026-05-14):** **Zamknięta dla gate_health (MVP)** — kontrakt `decision` + [`doc/examples/orchestrator-run-v1.example.json`](examples/orchestrator-run-v1.example.json). `inputs_ref`: statystyki ścieżek (`path`, `mtime_unix_secs`, `size_bytes`) dla `STARTUP.md` oraz plików danych per curated pool (`swap_sync::health_check_curated_all_collect`). **Opcjonalnie później:** skróty kryptograficzne zawartości plików albo `inputs_ref` rozszerzone o inne źródła (faza 2+).

**Cel:** jeden **format** wpisu „run orkiestratora”, żeby fazy 1–3 nie pisały każdy innego JSON-a.

| Działanie | Wynik (deliverable) |
| --------- | ------------------- |
| Zdefiniować minimalne pole `decision` (lub osobny plik `orchestrator_runs.jsonl`) | Pola min.: `schema_version`, `run_id`, `ts_utc`, `source`, `tools_invoked[]`, `data_quality` (skrót wyniku health/audit), `outcome` (`ok` / `no_go` / `partial`), `no_go_reason`, `inputs_ref` (ścieżki lub hashe plików danych). |
| Uzgodnić z `POST /api/v1/data/agent/decisions` | Albo **rozszerzenie** istniejącego JSONL, albo **nowy** plik + opcjonalny GET (wtedy osobny endpoint w przyszłości). |
| Opcjonalnie: **manifest** narzędzi | Krótki YAML/JSON generowany ręcznie z §1b (`DECISION_LAYER`) — jeden plik dla skryptów (wersjonowany w git). |

**Test akceptacji:** przykładowy wpis JSON waliduje się względem opisu w [`FUNCTIONAL_SPECIFICATION.md`](FUNCTIONAL_SPECIFICATION.md) §8 (rozszerzonym o ten schemat).

---

## Faza 1 — „Co mnie otacza” (gate runner)

**Status (2026-05-14):** **Zaimplementowana (MVP)** — subcommand `clmm-lp-cli orchestrator-gate`: ta sama logika co `data-health-check`, append jednego wiersza do `agent_decisions.jsonl`, opcja `--fail-on-no-go`. **Ścieżka B** z tabeli poniżej. Test: `cargo test -p clmm-lp-cli orchestrator_gate`.

**Cel:** automatyczny, powtarzalny **obraz jakości środowiska** przed jakąkolwiek symulacją rankingową.

| Działanie | Sugerowana realizacja | Uwagi |
| --------- | ---------------------- | ----- |
| Łańcuch ingestu (opcjonalnie przed gate) | Już jest: `ops-ingest-cycle` ([§1b](DECISION_LAYER.md)) | Harmonogram (Task Scheduler / cron), limity `--limit` / swapów. |
| Gate | `data-health-check` (+ ewentualnie `swaps-decode-audit --save-report` wg polityki) | Exit code ≠ 0 lub alert → zapis **`no_go`** w logu z powodem. |
| Zapis wyniku | Append do uzgodnionego w fazie 0 kanału (`agent_decisions` lub `orchestrator_runs.jsonl`) | Pola: które komendy, jakie progi, surowy skrót stdout lub ścieżka do raportu. |

**Implementacja techniczna (wybór jednej ścieżki w PR):**

- **A)** Skrypt `tools/*.ps1` (szybko, bez kompilacji), albo  
- **B)** Nowy subcommand `clmm-lp-cli` (np. `orchestrator-gate`) — spójność z resztą CLI, łatwiejsze testy `#[cfg(test)]`.

**Test akceptacji:** dwa uruchomienia — sztucznie „zły” katalog danych → `no_go`; po naprawie danych (lub mocku) → `ok`. Brak wywołania `backtest-optimize` przy `no_go`.

---

## Faza 2 — Wielokrotny backtest / optimize (jedna para / jeden pool)

**Status (2026-05-14):** **Częściowo** — CLI `orchestrator-backtests-full` (API-first): opcjonalny gate → `POST /api/v1/backtests/full` → poll → wiersz `agent_decisions` (`decision.kind`: `api_backtests_full`) lub `POST /data/agent/decisions`. Pełny raport N-wariantów poza API (jeden JSON agregujący wiele runów **bez** FULL) — nadal opcjonalnie / później.

**Cel:** **N konfiguracji**, **jedno okno danych**, **jeden raport** — pierwszy krok w stronę what-if bez live shadow w kodzie produktu.

| Działanie | Wynik |
| --------- | ----- |
| Plik konfiguracyjny lub lista presetów | Np. kilka `width_pct` / zestawów flag zgodnych z już istniejącym CLI | Nie wymyślać nowej semantyki strategii — użyć `backtest` / `backtest-optimize` jak dziś. |
| Orkiestracja | Pętla wołająca CLI z tymi samymi ścieżkami snapshotów | Limit równoległości (sekwencyjnie lub pool 2–3) żeby nie zabić RAM/RPC. |
| Raport | Jeden JSON lub Markdown: tabela wariantów + metryki zwycięzcy | Ścieżka zapisu wersjonowana datą (`data/reports/orchestrator-*.json`). |

**Test akceptacji:** ten sam zestaw danych → dwa runy dają **identyczny** ranking (determinizm przy tych samych seedach/flagach).

---

## Faza 3 — Porównanie z realiem (ciąg pozycji)

**Cel:** obok wyniku fazy 2 dołączyć **stan realnej pozycji** z API (już istniejące).

| Działanie | Źródło |
| --------- | ------ |
| Metryki ciągu | `GET /api/v1/positions/{addr}/stream-pnl`, `…/stream-lineage`, ewentualnie `…/agent/supervisor` |
| Sklejenie raportu | Skrypt (curl + jq) lub mały moduł w Rust czytający JSON z fazy 2 + odpowiedzi API | Token/API: użyć istniejącego auth jeśli wymagane w deployu. |

**Test akceptacji:** raport HTML/MD zawiera **sekcję „real”** i **sekcję „warianty symulacji”** z tą samą skalą czasu (jawnie zapisana).

---

## Faza 4 — Operator w pętli (sugestia → review → apply)

**Cel:** spięcie z **istniejącym** `apply-optimize-result` + `AgentDecision` bez autonomii.

| Działanie | Wynik |
| --------- | ----- |
| Po pozytywnym gate i review wyniku fazy 2 | Ręczny lub półautomatyczny krok: `POST …/apply-optimize-result` z envelope ([`AI_AGENT_LAYER.md`](AI_AGENT_LAYER.md)) |
| Polityka | Respektować `optimize_apply_policy` i busy ([`PROJECT_OVERVIEW.md`](PROJECT_OVERVIEW.md)) | Test integracyjny: 409 przy równoległym apply. |

**Test akceptacji:** `approved: false` nie zmienia executora; `approved: true` + poprawny JSON → zgodnie z dzisiejszym zachowaniem API.

---

## Faza 5 — UI / alerty (opcjonalnie)

**Cel:** widoczność dla operatora bez wchodzenia na serwer.

- Kafelek lub strona w `web/`: ostatni `no_go` / link do raportu z fazy 3.  
- Opcjonalnie: Slack / istniejące webhooki (`tools/`, patrz [`SCRIPTS_CATALOG.md`](SCRIPTS_CATALOG.md)).

---

## Faza 6+ — Rozszerzenia (po stabilizacji 0–4)

| Temat | Odniesienie |
| ----- | ----------- |
| Wiele par / venue, porównanie alokacji | [`DECISION_LAYER.md`](DECISION_LAYER.md) §4.5, §6 faza 3; jakość danych **per seria** |
| Shadow „1 live + N strategii” na jednej pozycji | [`ROADMAP.md`](ROADMAP.md) — **osobny** projekt danych; uzgodnić z orkiestrator-run logiem |
| Jawne sygnały wolumenu w czasie | [`DECISION_LAYER.md`](DECISION_LAYER.md) §1a — doprecyzowanie normy + implementacja liczników z `decoded_swaps` |
| LLM jako komentarz nad raportem | Opcjonalnie; **wejściem** jest raport z fazy 2–3, nie surowy łańcuch |

---

## Poza zakresem pierwszych faz (świadomie)

- Autonomiczne **open / close / rebalance** bez osobnego Go/No-Go i testów bezpieczeństwa.  
- Fine-tuning modelu pod „uczenie agenta” — nie jest wymagany do celów tego planu.

---

## Kryteria sukcesu (skrót)

| Faza | Sukces = |
| ---- | -------- |
| 0 | Schemat logu zaakceptowany w spec + przykładowy plik (**`gate_health`** + `inputs_ref` + [`doc/examples/orchestrator-run-v1.example.json`](examples/orchestrator-run-v1.example.json); hashe treści — opcjonalnie później) |
| 1 | Gate działa w CI/harmonogramie; `no_go` blokuje dalsze kroki (**`orchestrator-gate --fail-on-no-go`**) |
| 2 | Raport N-wariantów deterministyczny przy tych samych danych (**pełna macierz:** API `POST /backtests/full`; audyt: CLI `orchestrator-backtests-full`) |
| 3 | Raport łączy symulację z `stream-pnl` / lineage |
| 4 | Apply tylko po świadomym `approved: true`; brak regresji 409/policy |

---

## Kolejność PR (sugerowana)

1. ~~Faza 0~~ (kontrakt w §8 + [`doc/examples/orchestrator-run-v1.example.json`](examples/orchestrator-run-v1.example.json); `inputs_ref` — później).  
2. ~~Faza 1~~ (`orchestrator-gate` + test append JSONL).  
3. Faza 2 (wrapper + raport).  
4. Faza 3 (kompozycja raportu).  
5. Faza 4 (dokumentacja runbooka + ewentualnie skrypt curl).  
6. Aktualizacja [`DECISION_LAYER.md`](DECISION_LAYER.md) §11.1 po każdej fazie (nowe pliki / komendy).

---

## Document status

| Field | Value |
| ----- | ----- |
| Role | Implementation plan for decision-layer / situational orchestration |
| Created | 2026-05-14 |
| Maintainer | team — revise after each phase merge |
