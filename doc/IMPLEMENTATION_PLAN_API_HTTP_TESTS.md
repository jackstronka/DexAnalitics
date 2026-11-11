# Plan: testy HTTP / kontrakt API

**Status:** plan — **H0–H5.1/H5.2 wdrożone** 2026-11-11; H5.3 (F2.9) = osobny C6  
**Powiązane:** [`IMPLEMENTATION_PLAN_REGRESSION_RESILIENCE.md`](IMPLEMENTATION_PLAN_REGRESSION_RESILIENCE.md) (C1/C2, §7 F2.8/F2.9), [`TESTS.md`](TESTS.md), [`MASTER_IMPLEMENTATION_PLAN.md`](MASTER_IMPLEMENTATION_PLAN.md) Fala 2, PR #33 (`api_http_contract_tests`).

## Cel

Uporządkować i domknąć **granicę HTTP** API (`clmm-lp-api`): dziś C1/C2 chronią schemat typów, a pieniądze siedzą w goldenach serwisów — ale **duża powierzchnia REST nie ma typed contract ani nawet smoke**. Plan nie duplikuje B1/B5/B6 na routerze; pinuje **envelopy, statusy, persistence i bramki live**.

## Werdykt z analizy (2026-11-11)

### Co już jest

| Warstwa | Pliki / bramki | Siła |
| ------- | -------------- | ---- |
| C1 OpenAPI snapshot | `crates/api/openapi.json`, `openapi_matches_committed*` | silna na drift pól/ścieżek |
| C2 TS gen | `web/src/lib/api.gen.ts`, job `check:api-gen` | silna na FE↔spec |
| HTTP smoke | `handlers/endpoint_coverage_tests.rs` | płytka (status / walidacja) |
| Typed HTTP contract | `handlers/api_http_contract_tests.rs` (**7** przypadków) | wąska, dobra jakość hermetyki |
| Handler unit | `phantom_auth`, `orca`, `pools`, `tx`, inline wallets/backtests/positions | mieszana |
| Domain goldens | B1/B5/B6 w `services/*` | silna na pieniądze **pod** HTTP |
| DB GL | `session_gl_integration` (crate data) | silna na księgowość, nie na DTO REST |
| Web Vitest | `web/src/lib/*.test.ts` | **bez** żywego API (świadomie) |
| Devnet E2E | `devnet_e2e_tests.rs` | `#[ignore]`, poza CI |

**Inwentarz OpenAPI:** ~**127** paths / **136** ops.  
**Smoke HTTP:** ~40–45% ops (status).  
**Typed contract:** ~**5%** ops.  
**Technika obecna:** hybryda *contract-first (C1/C2) + shallow smoke + deep domain* — nie pełny API test design (Pact, schema-validate każdego response, stateful E2E).

### Luki (severity)

| Priorytet | Obszar | OpenAPI (ok.) | HTTP dziś | Ryzyko produktowe |
| --------- | ------ | ------------: | --------- | ----------------- |
| **P0** | Close-all (F2.8) | 3 | tylko GET 404 | batch kapitału; job in-memory ginie po restarcie |
| **P0** | Chain-portfolio HTTP | 4 | brak | UI NAV/fees; B6 nie pinuje query/DTO |
| **P1** | Dip `/dip/*` | 9 | brak | Trigger/live gate; sekrety; propose |
| **P1** | Agent (worker + position) | ~10–12 | brak | LLM/scan bez kontraktu HTTP |
| **P2** | Wallets balances / effective / api-signer | część z 27 | słaby smoke | F2.6-adjacent; open UI |
| **P2** | Session/Wallet GL HTTP | kilka | brak / 503 | DTO ≠ db integration |
| **P3** | Bot stranded / pending-open | 7/8 bez smoke | 1 GET | recovery UX |
| **P3** | WebSocket positions/alerts | 2 (poza OpenAPI) | tylko upgrade | brak kształtu eventu |
| **P3** | Backtest jobs (full/auto-tune/…) | 9 poza readiness | tylko readiness | długie joby |
| **P4** | Stream PnL / lineage / lifecycle HTTP | kilka | brak | Detail page envelope |
| — | Prices / EVM / data raw | niszowe | brak | niski priorytet |

### Porównanie technik (decyzja planowa)

| Technika | Decyzja |
| -------- | ------ |
| Rozszerzać C1/C2 | **Tak** przy każdym nowym op — już exit gate |
| Typed `oneshot` → serde modele (jak PR #33) | **Główna oś P0–P2** |
| Status smoke w `endpoint_coverage` | **Minimum** dla każdego nowego op; nie zastępuje contract |
| Domain golden na routerze | **Nie** — zostaje w services; HTTP aserty tylko kluczowe pola / source tag |
| Pact / multi-consumer | **Nie** (jeden dashboard; C2 wystarczy) |
| Vitest → żywe API w required `web` | **Nie** (flaki); opcjonalnie później osobny job + MSW |
| Playwright E2E | **Poza zakresem** (jak resilience §10) |
| Schema lib (jsonschema) vs serde | Zostajemy przy **serde modeli OpenAPI** (jeden source of truth w Rust) |

---

## Zasady implementacji (obowiązkowe)

1. Hermetycznie: `RpcConfig { primary_url: "http://127.0.0.1:1", fallback_urls: vec![], max_retries: 0, … }`; env tylko `test_env::EnvGuard`; bez repo `data/`.
2. Mocki zewnętrzne: **httpmock** (Orca/Jupiter/RPC JSON-RPC) na localhost.
3. Close-all / agent / dip: **fake executor / stub LLM** — zero prawdziwych tx.
4. Nowy plik testowy lub nowa suite → wpis w [`TESTS.md`](TESTS.md) w tym samym PR.
5. Nie osłabiać asercji smoke → `is_ok()`; nie ruszać goldenów B1/B5/B6 bez Golden delta + GO przy money.
6. Ticket: [`templates/AGENT_TICKET.md`](templates/AGENT_TICKET.md) z ID fazy poniżej.

---

## Fazy / PR-y

### API-H0 — Fundament kontraktu (S) — ✅ 2026-11-11

**Cel:** wzorzec typed contract + close-all write-path bez pełnej persistence.

| ID | Zakres | Rozm. | Stan |
| -- | ------ | ----- | ---- |
| **H0.1** | `handlers/http_test_support`: `hermetic_rpc()`, `oneshot` helpers | S | ✅ |
| **H0.2** | Close-all: preview (empty / validation / unmanaged) + start→GET gdy `closable==0` (worker nie startuje) | S | ✅ |
| **H0.3** | `endpoint_coverage` na `http_test_support` (bez `RpcConfig { ..Default }` z publicznymi fallbackami) | S | ✅ |

**Done when:** `cargo test -p clmm-lp-api --lib api_http_contract` + `endpoint_coverage` zielone; `TESTS.md` zaktualizowany.

### API-H1 — F2.8 persistence + HTTP round-trip (M) — ✅ 2026-11-11

| ID | Zakres | Rozm. | Stan |
| -- | ------ | ----- | ---- |
| **H1.1** | JSON store `data/close_all_jobs.json` (`CLMM_CLOSE_ALL_JOBS_PATH`); persist on insert/item/finish; reload in `ApiServer::new` | M | ✅ |
| **H1.2** | `close_all_job_survives_store_reload_from_tempdir` | S | ✅ |
| **H1.3** | `close_all_http_get_survives_disk_reload` | S | ✅ |

**Done when:** F2.8 exit gate; `BUG-20261111-02` Guards/tests.

### API-H2 — Dip + Agent HTTP (M) — ✅ 2026-11-11

| ID | Zakres | Rozm. | Stan |
| -- | ------ | ----- | ---- |
| **H2.1** | `dip_http_tests`: snapshot / runtime / session; propose → JSONL; live bez `confirm_execute` / propose-only → 4xx | M | ✅ |
| **H2.2** | httpmock history + vault/craft; `CLMM_JUPITER_TRIGGER_BASE_URL` | S | ✅ |
| **H2.3** | Agent start→chat + worker settings; supervisor bez chain/DB → 4xx/5xx; bez live LLM | S | ✅ |

### API-H3 — Chain-portfolio + wallets balances (M) — ✅ 2026-11-11

| ID | Zakres | Rozm. | Stan |
| -- | ------ | ----- | ---- |
| **H3.1** | GET chain-portfolio no-DB envelope; B6 footer via shared helpers (8.50); history requires DB | M | ✅ |
| **H3.2** | Backfill bez DB: 2× stabilny 500 (brak cichego sukcesu); pełna idempotencja = job `db` | S | ✅ |
| **H3.3** | balances httpmock; effective warmup; api-signer unconfigured; invalid owner → 400 | S | ✅ |

### API-H4 — Ops surface (S)

| ID | Zakres | Rozm. |
| -- | ------ | ----- |
| **H4.1** | Bot stranded / pending-open list+dismiss (in-memory seed) | S | ✅ |
| **H4.2** | WS: jeden event JSON po upgrade (injected broadcast) — kształt, nie Solana WS | S | ✅ |
| **H4.3** | Backtest job: start in-memory → GET status deserialize; zły body → 4xx | S | ✅ |

### API-H5 — Detail envelopes + nawyk (S)

| ID | Zakres | Rozm. |
| -- | ------ | ----- |
| **H5.1** | Stream-pnl / lifecycle-summary / diagnostics: typed smoke z fixture state | S | ✅ |
| **H5.2** | Reguła: nowy OpenAPI op = C1 + C2 + **jeden** wiersz smoke **lub** typed contract (`AI_MERGE_CHECKLIST`; skrypt „paths bez testu” opcjonalnie później) | S | ✅ |
| **H5.3** | F2.9 (web lib message map) — osobny PR C6; nie mylić z HTTP | S | ⏸ poza tym PR |

### Poza zakresem tego planu

- Playwright / Vitest against running API w required job `web`.
- Mutants D5 na handlerach (zostaje na agregatorach).
- Pełna migracja `api.ts` → tylko gen.
- Live RPC / funded wallet w CI.

---

## Kolejność vs produkt

```
H0 (contract + close-all dry)
 └─► H1 (F2.8 persistence)     ← równolegle z produktem close-all store
 └─► H2 (dip + agent HTTP)     ← po/obok Dip UI
 └─► H3 (chain-portfolio HTTP + balances)
      └─► H4 (bot/WS/backtest jobs)
           └─► H5 (detail + habit gate)
```

F2.1/F2.3 (reopen/lineage) **nie** są w tym planie — to execution/lineage; tu tylko HTTP envelopes gdy Detail czyta te dane (H5.1).

---

## Testy i kryteria regresji

**Obszar testów (R1):** API / kontrakt (wspiera: Ekonomia / GL, Decision layer / agent, Web)  
**Pozycja w mapie rozwoju:** nowa pozycja **API-H0…H5** (dopisana do §7 [`IMPLEMENTATION_PLAN_REGRESSION_RESILIENCE.md`](IMPLEMENTATION_PLAN_REGRESSION_RESILIENCE.md)); spina **F2.8**, wspiera **F2.6/F2.9** (balances DTO / message map osobno w web).

| Co chronimy | Typ testu | Fixture / dane | Stan |
| ----------- | --------- | -------------- | ---- |
| C1: live ApiDoc = committed OpenAPI | kontrakt | `crates/api/openapi.json` | ✅ |
| C2: `api.gen.ts` sync | kontrakt | `web/src/lib/api.gen.ts` | ✅ |
| Health/metrics/list positions/strategies/wallets shape | kontrakt HTTP | in-memory AppState | ✅ |
| Close-all unknown batch → 404 JSON | kontrakt HTTP | — | ✅ |
| Close-all preview/start zero-closable + GET status (no worker) | kontrakt HTTP | in-memory AppState | ✅ H0 |
| Close-all job przeżywa restart procesu | unit + HTTP | tempdir `CLMM_CLOSE_ALL_JOBS_PATH` | ✅ H1 / F2.8 |
| Dip snapshot/runtime/propose + live gate | kontrakt HTTP + httpmock | tempdir zones/runtime | ✅ H2 |
| Agent start/chat bez live LLM | kontrakt HTTP | tempdir agent data | ✅ H2 |
| Chain-portfolio GET envelope + B6 footer helpers | kontrakt HTTP + unit helpers | no-DB + B6 numbers | ✅ H3 |
| Wallet balances/effective typed | kontrakt HTTP + httpmock RPC | localhost mock | ✅ H3 |
| Bot stranded/pending-open list/dismiss | smoke + kontrakt | tempdir ledger + EnvGuard | ✅ H4 |
| WS event shape po upgrade | kontrakt | injected broadcast | ✅ H4 |
| Backtest job status envelope | kontrakt | in-memory job map | ✅ H4 |
| Stream-pnl / lifecycle / diagnostics HTTP envelope | kontrakt | no-DB + tempdir ledger | ✅ H5.1 |
| Nowy OpenAPI op = C1+C2+smoke\|typed | nawyk | `AI_MERGE_CHECKLIST` | ✅ H5.2 |
| Vitest ↔ żywe API | E2E | — | ❌ poza zakresem |
| Devnet funded flows | manual `#[ignore]` | live | 🟡 istniejące |

**Test exit gate:**

- Po **H0:** `cargo test -p clmm-lp-api api_http_contract` (+ nowe close-all dry) zielone w job `rust`.
- Po **H1:** test restart close-all + HTTP round-trip w `rust` (i `db` jeśli store = PG).
- Po **H2–H5:** nowe moduły `*_http_tests` / rozszerzenia catalogowane w `TESTS.md`; job `web` bez zmian wymaganych (chyba że C2).
- C1/C2 nadal wymagane przy każdej zmianie `openapi.json`.

**Bugi powiązane:** F2.8 (close-all persistence); F2.6 (api-signer — test web lib, HTTP balances wspiera); ewentualne wpisy `BUGS.md` przy odkryciu regresji wiring — `Guards/tests` = nazwa nowego testu HTTP.

**Golden delta:** ten plan **nie** powinien ruszać snapów B1/B5/B6. Jeśli H3 porównuje liczby z B6 — asercje pól w teście HTTP, nie update `.snap`. Zmiana `openapi.json` → sekcja Golden delta w PR (D1).

**Poza automatem:** Playwright; Vitest→API; `devnet_e2e_tests`; live Jupiter create order.

---

## Document status

| Field | Value |
| ----- | ----- |
| Role | Plan wykonawczy testów granicy HTTP API |
| Created | 2026-11-11 |
| Next review | Po zamknięciu H1 (F2.8) lub zmianie priorytetu Dip vs chain-portfolio |
| Supersedes | — (uzupełnia C1/C2 i PR #33; nie zastępuje planu regresji) |
