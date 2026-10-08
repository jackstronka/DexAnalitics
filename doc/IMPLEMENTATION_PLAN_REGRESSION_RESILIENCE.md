# Plan: szybkie budowanie + odporność na regresje (jedyny plan wykonawczy testów)

**Status:** plan wykonawczy (2026-09-30), oparty na analizie repo + wyniku CI [PR #2](https://github.com/jackstronka/DexAnalitics/pull/2). Faza 0 i kolejne wymagają **GO**.  
**keywords:** regression-resilience, testing-plan, hermetic-tests, golden-fixture, insta, proptest, invariants, openapi-contract, make-verify, ci, postgres-ci, branch-protection, self-heal-pipeline, agent-rules, cargo-mutants, verify-fast, test-report, test-areas, job-summary, golden-delta-table, roadmap-test-gates, master-plan-waves

**Powiązane:** [`TESTS.md`](TESTS.md) (katalog: co jest i jak odpalić), [`MASTER_IMPLEMENTATION_PLAN.md`](MASTER_IMPLEMENTATION_PLAN.md) (fale rozwoju F1–F5 — §7 tego planu mapuje je na testy), [`ROADMAP.md`](ROADMAP.md), [`TESTING_REGRESSION_PLAN.md`](TESTING_REGRESSION_PLAN.md) (audyt 2026-09-30 + log wykonania; **nie** backlog), [`AI_MERGE_CHECKLIST.md`](AI_MERGE_CHECKLIST.md), [`BUGS.md`](BUGS.md) (BUG-20260930-01…04), [`IMPLEMENTATION_PLAN.md`](IMPLEMENTATION_PLAN.md) (backlog produktowy — ten plan go nie zastępuje)

**Rola testów w projekcie:** testy mają (1) chronić to, co już działa, (2) **prowadzić rozwój** — każda fala z master planu ma testowe kryterium zamknięcia (§7), (3) dawać **czytelny, jednolity obraz jakości** per obszar projektu (faza R).

---

## 1. Cele (mierzalne)

| ID | Cel | Miara „osiągnięte” |
| -- | --- | ------------------ |
| **G1** | Nic nie wchodzi do `main` bez zielonego pełnego zestawu | Branch protection: wymagane `rust` (fmt/clippy/test), `web`, `db`, `quality_gates`; brak push na `main` |
| **G2** | Zielone lokalnie ⇔ zielone w CI (hermetyczność) | 0 testów aktywnych z siecią / repo `data/` / `set_current_dir` / niechronionym env; testy Rust w CI przechodzą **bez sieci** |
| **G3** | Zmiana liczby finansowej zawsze widoczna w PR | ≥ 5 goldenów na ścieżkach pieniędzy (§5 B); zmiana snapshotu wymaga sekcji „Golden delta” w PR |
| **G4** | Zmiana kontraktu API łamie build web | Snapshot OpenAPI w repo + test; nowe typy web generowane z OpenAPI |
| **G5** | Refaktor nie wymaga przepisywania testów | ≥ 5 niezmienników (property tests) na GL / lineage / sizing / domain |
| **G6** | Agent nie może po cichu osłabić ochrony | Reguła agenta + CI check na zmiany fixture'ów + CODEOWNERS |
| **G7** | Szybki feedback | `verify-fast` lokalnie < 2 min na typowy diff; CI na PR < 10 min (cache) |
| **G8** | Testy prowadzą rozwój, nie tylko go pilnują | Każda fala F1–F5 i pozycja `ROADMAP.md` ma w §7 wymagane testy; fala nie jest „zamknięta” bez nich; każdy nowy `IMPLEMENTATION_PLAN_*` ma sekcję „Testy i kryteria regresji” |
| **G9** | Wyniki czytelne i spójne | Jeden format raportu lokalnie (`make verify`) i w CI (Job Summary): tabela per **obszar projektu**; zmiana goldena = tabela „metryka / było / jest / Δ” w PR; trend tygodniowy per obszar |

Zasada nadrzędna: **broń wyniku, nie kształtu kodu** — asercje na liczby, salda, JSON API, ciągłość lineage; nie na prywatne pola i nie `is_ok()`.

---

## 2. Stan wyjściowy (dowody z analizy)

**Liczby:** ~658 testów Rust (634 aktywnych), 49 Vitest; jedyne dev-dependency testowe: `tempfile` (brak `insta`, `proptest`, `nextest`).

**CI (PR #2):** zielone `build`, `lint` (pierwszy raz od 05-21), `lineage_shadow_diff`; czerwone:

| Pad | Przyczyna | Klasa |
| --- | --------- | ----- |
| `run_tests` | `registry_stale_reconcile::backfill_9vhky_orphan_close_when_lifecycle_missing` — mainnet RPC + gitignorowany ledger + `set_current_dir`; lokalnie pusty pass (`return`) | hermetyczność |
| `critical_area_requires_tests` | `.gitignore` → `/scripts/` ⇒ `scripts/ci/critical-area-test-gate.sh` nigdy nie był w repo | infrastruktura |
| `semver-*` | `baseline-rev: main` — w checkout PR istnieje tylko `origin/main` | infrastruktura |
| `code_coverage_report` | ~11 min, czerwony od maja (nie diagnozowany) | poza zakresem |

`format_check` jest zielony, bo uruchamia `make fmt` (formatuje) — BUG-20260930-03. Brak cache kompilacji: 5 jobów Rust × 3–4 min od zera.

**Hermetyczność (audyt kodu):**

| Kategoria | Przypadki |
| --------- | --------- |
| Realna sieć | 1 — backfill 9vhKY (wyżej) |
| Zapis do repo `data/` | `cli/src/local_swap_fees.rs` test `build_local_pool_fees_uses_decoded_swaps_when_strict_ok` → `data/swaps/orca/<pool>/decoded_swaps.jsonl` |
| Env bez wspólnej blokady (ten sam proces testów) | `api`: `CLMM_POSITION_REGISTRY_PATH` (`position_close_signer.rs`, bez sprzątania), `ORCA_PUBLIC_API_BASE_URL` (`pools_tests.rs`); `execution`: `CLMM_SWAP_MIX_DEFICIT_USD_EPS` (`rebalance.rs:5173`, poza `TEST_ENV_LOCK`); `data`: `CLMM_POSITION_LIFECYCLE_LEDGER_PATH` (`wallet_session.rs:2615`); `cli`: `KEYPAIR_PATH` (`orca_wallet.rs`) |
| Pusty pass | backfill 9vhKY; 4× `session_gl_integration` bez `DATABASE_URL` |
| Czas | `wallets.rs:3834` `wallet_effective_hydrate_timestamp_preserves_stale_age` (okno 5000–7500 ms) |

**Luki w powyższej diagnozie (weryfikacja 2026-09-30, po logach CI PR #2):**

| # | Luka | Dowód | Skutek dla planu |
| - | ---- | ----- | ---------------- |
| L1 | Większość testów **nigdy nie uruchomiła się w CI** | `make test` = `cargo test` bez `--no-fail-fast`; run zatrzymał się na pierwszym padzie w `clmm-lp-api --lib` (237 pass / 1 fail / 23 ignored). Testy `data`, `domain`, `execution`, `cli`, `protocols`, `simulation`, `optimization` i testy integracyjne nie wykonały się na Linuksie | Stwierdzenie „1 test z siecią” jest udowodnione tylko dla `api --lib`. Po 0.1 mogą wyjść kolejne pady → **0.5** |
| L2 | **Niejawne odczyty** repo `data/` przez domyślne ścieżki nie są audytowane (tabela obejmuje tylko zapis) | `data/src/wallet_session.rs:1217` i `protocols/src/ledger/tx_lifecycle.rs:23` (`DEFAULT_REL_PATH`) — fallback na gitignorowany `data/ledger/orca_position_lifecycle.jsonl`; ten sam mechanizm dał „lokalnie zielone / CI czerwone” w 9vhKY | Nieznane jeszcze, czy aktywny test trafia w fallback → sprawdzenie w **A6** |
| L3 | Brak w tabeli env: `api/services/position_agent_service.rs` | `CLMM_AGENT_DATA_DIR` pod **lokalnym** `TEST_ENV_LOCK` (nie wspólną blokadą crate'a), `remove_var` na końcu testu — przy panice brak przywrócenia. W `api` są dziś dwie niezależne blokady env (`test_env::EnvGuard` i ta) | Dopisane do **A6** |
| L4 | Niezmiennik tick ↔ price (C3(3)) nie wskazuje implementacji | Dwie wersje: `domain/src/math/price_tick.rs` (`f64` `powi`/`log`, `Result`) i `protocols/src/orca/pool_reader.rs` (`Decimal`) | Doprecyzowane w **C3** (implementacja + zakres ticków) |
| L5 | Stan dokumentów rozjechany z kodem | 0.1 częściowo wdrożone (niezacommitowane: `#[ignore]` na teście RPC w `registry_stale_reconcile.rs`, `crates/api/src/test_env.rs` + użycie w `position_close_signer.rs`), opisane w BUG-20260930-04 „Fix”, ale nagłówek planu mówi „wymaga GO”, a log §6 w `TESTING_REGRESSION_PLAN.md` tego nie ma; zielony `run_tests` w CI jeszcze niepotwierdzony | Po zielonym CI dopisać 0.1 do logu §6 |

Niezweryfikowane ponownie: liczba testów (~658, z logu T-PR1), czasy CI (G7), przyczyna czerwonego coverage (poza zakresem).

**Testowalność ścieżek pieniędzy (czyste funkcje, gotowe pod fixture):**

| Obszar | Czyste wejście | Nieczyste (IO) |
| ------ | -------------- | -------------- |
| Sumy łańcucha | `chain_economic_totals::{refresh_lineage_totals_from_nodes, sync_chain_economic_totals_from_nodes, chain_headline_end_nav_usd, close_nav_usd_from_raw_amounts_and_prices}` — na `PositionStreamLineageNode` (Serialize+Deserialize) | `enrich_nodes_lifecycle_close_nav_from_ledger` |
| Lineage | trio ciągłości (`apply_session_continuity_from_lifecycle_rows`, `apply_baseline_fallback_from_prev_end`, `apply_end_value_fallback_from_next_baseline`) — prywatne | `compute_position_stream_lineage` |
| Stream PnL | `compute_stream_il_components`, cashflow helpers — prywatne | `compute_position_stream_pnl` |
| Sesja / CHAIN (GL) | `wallet_session::{aggregate_session_sums_from_lifecycle_rows, aggregate_chain_sums_from_lifecycle_rows, cap_open_debits_against_running_balance, gl_pslr_match, session_mint_deltas_from_lifecycle_json}` — na `serde_json::Value` | `compute_*_balances_from_pslr`, `apply_*_postings_*` |
| Portfel łańcucha | `chain_portfolio::{aggregate_chain_collected_fees, chain_balance_usd_legs_from_balances, ledger_start_event_from_open_start}` | `build_chain_portfolio_ledger` |
| Sizing reopen | `session_capital::{apply_portfolio_caps_to_wallet_raw, clamp_deposit_quote_to_portfolio, clamp_target_usd_to_chain_wallet_notional}`; `rebalance::target_usd_*` — prywatne | `load_chain_scoped_pool_wallet` |
| Backtest | `run_single` / `run_grid` na `StepData` (bez RPC/DB) | loadery CLI; `StepData` bez Deserialize |

Fixtures w repo: `crates/api/tests/fixtures/{chain_history_9vhKY.json, stream_lineage_9vhKY.json}` (odpowiedzi API — **nieużywane** przez żaden test), `lineage_shadow_expected.json`. **Brak** zacommitowanego wycinka `snapshots.jsonl` / `decoded_swaps.jsonl`.

**Raportowanie wyników:** brak. Wynik = zielony/czerwony check + surowy log `cargo test` (~650 linii); brak podziału na obszary, brak JUnit, brak historii. Diff goldena = surowy diff JSON. Master plan wymaga testów, których nie ma (§6: „0 regresji BUG-20260413-05 w testach golden”; F2.4 „regresja golden”; F2.9 „UI regression tests”), a tabela dojrzałości (§2.2) nie mówi, co jest chronione testami.

**Rozwój vs testy:** aktywne bugi Fali 2 master planu nadal otwarte (2026-09-30): BUG-20260413-05 `regressed` (lineage parent), BUG-20260512-03 `regressed` (reopen downsizing), BUG-20260410-06 `open` (rotacja bez reopen), BUG-20260410-04 `open` (brak testów UI), BUG-20260413-06 `open` (PositionCreate wallet race).

**Web:** testy tylko w `web/src/lib` (9 plików, 7 o experiment launcher). Bez testów m.in. `whirlpoolTicks.ts`, `chainCapital.ts`, `sessionCapital.ts`, `openPositionSwapEstimates.ts`, `lineageLedgerOpenQuote.ts`, `chainEconomicQuality.ts`. Strony 2400–2600 linii (`PositionCreate`, `PositionDetail`) z logiką w komponentach. Vitest `environment: 'node'`, brak ESLint config.

---

## 3. Model ochrony

```text
zmiana (człowiek / agent)
  ├─ hermetyczne testy      → ten sam wynik lokalnie i w CI            (G2)
  ├─ kontrakt OpenAPI ↔ TS  → pad przy zmianie pola                   (G4)
  ├─ niezmienniki (proptest)→ pad przy złamanej ekonomii, nie przy rename (G5)
  ├─ golden (insta)         → pad przy zmianie liczby; diff w PR       (G3)
  └─ verify + CI + branch protection → nic z powyższego nie jest opcjonalne (G1, G7)
        └─ reguły agenta + check fixture'ów → ochrona nie jest osłabiana (G6)
```

**Self-healing w tym repo** = proces wokół padu, nie test poprawiający asercję:

| Klasa padu | Przykład | Kto naprawia |
| ---------- | -------- | ------------ |
| `stale_test` | brakujące pole w inicjalizatorze, stary typ w fixture TS | agent — bez zmiany semantyki produktu |
| `flake` / `non_hermetic` | env race, sieć, zależność od lokalnych danych | agent — izolacja (lock, tempdir, fake); **nie** retry |
| `economic_regression` | diff golden PnL/IL/fees/sald, złamany niezmiennik | **człowiek** — bug albo świadoma delta w PR |
| `assertion_weakened` | usunięty snapshot, liczba → `is_ok()` | **blokada** w review / CI |
| `infra` | brak skryptu w repo, zły ref w workflow | agent |

---

## 4. Zasady (obowiązują od fazy 0)

1. **Hermetyczność:** test aktywny nie używa sieci (poza localhost mock), repo `data/`, `set_current_dir`, env bez wspólnej blokady crate'a, zegara w wąskim oknie. Operacje naprawcze na prawdziwych danych = `#[ignore]` + opis „manual”.
2. **Brak pustego passu:** test, który może `return` bez asercji, jest błędem; w CI wymagane zasoby (DB) = fail, nie skip.
3. **Golden i niezmiennik nie są „naprawiane”, żeby CI było zielone.** Update snapshotu tylko lokalnie (`cargo insta review`), commit w tym samym PR z sekcją „Golden delta: co i dlaczego”.
4. **Nowa logika liczbowa w web** → `web/src/lib/*.ts` + test, nie w komponencie strony.
5. **1 temat = 1 PR**, gałąź + PR do `main` (quality gates działają tylko na PR).
6. Devnet E2E (`#[ignore]`) zostaje ręczny. Playwright / pełny E2E UI i coverage % — poza zakresem.
7. **Nowa funkcja = testy w tym samym PR**, zgodnie z mapą §7 dla jej fali. Nowy dokument `IMPLEMENTATION_PLAN_*` / `ROADMAP_*` ma sekcję **„Testy i kryteria regresji”** (co chronimy, jakim typem testu: golden / niezmiennik / kontrakt / unit, na jakim fixture). Fala master planu jest zamknięta dopiero, gdy jej testy są zielone w raporcie (faza R).
8. **Każdy test należy do obszaru projektu** (mapa R1). Test bez obszaru = ostrzeżenie w raporcie.

---

## 5. Fazy i PR

Rozmiar: **S** < 0.5 dnia, **M** ~1 dzień, **L** 2–3 dni. Kolumna „stare ID” — mapowanie z wcześniejszych wersji planu.

### Faza 0 — PR #2 zielony (natychmiast; blokuje wszystko)

| ID | Zakres | Done when | Rozm. | Stare ID |
| -- | ------ | --------- | ----- | -------- |
| **0.1** | `registry_stale_reconcile`: hermetyczny test detekcji orphan close (tempdir ledger/registry przez env pod lockiem, bez RPC); obecny test RPC → `#[ignore]` „manual repair 9vhKY”, bez `set_current_dir` | `run_tests` zielony w CI | S | — |
| **0.2** | `.gitignore`: `!/scripts/ci/` (+ `!/scripts/ci/**`), commit `critical-area-test-gate.sh` | job gate uruchamia skrypt | S | — |
| **0.3** | `semver.yml`: `baseline-rev: origin/main` (PR) | semver zielony albo realny raport API | S | — |
| **0.4** | Uzupełnić BUGS.md (BUG-20260930-04 hermetyczność) | wpis z listą §2 | S | — |
| **0.5** | `make test` → `cargo test --workspace --no-fail-fast` (luka L1); pełna lista padów z pierwszego runu dopisana do BUG-20260930-04 i §2 | CI raportuje pady ze wszystkich crate'ów, nie tylko pierwszego | S | — |

### Faza A — siatka zawsze się odpala i jest hermetyczna (G1, G2, G6, G7)

| ID | Zakres | Done when | Rozm. | Stare ID |
| -- | ------ | --------- | ----- | -------- |
| **A1** | Jednorazowy commit `cargo fmt --all`; `format_check.yml` → `make fmt-check` | BUG-20260930-03 fixed | S | T-PR2b |
| **A2** | `make verify` + `tools/verify.ps1` (fmt-check, clippy `-D warnings`, `cargo test --workspace`, web `tsc --noEmit` + `vitest run`); hook pre-push w `.githooks/` (`git config core.hooksPath .githooks`); `npm run lint` poza verify do czasu configu ESLint | jedna komenda lokalnie = to samo co CI | S | T-PR2 |
| **A3** | CI szybkie: `Swatinem/rust-cache` we wszystkich jobach Rust; połączyć fmt/clippy/test w jeden job `rust`; `concurrency: cancel-in-progress` na PR | PR < 10 min | S | — |
| **A4** | `web.yml`: `npm ci`, `tsc --noEmit`, `vitest run` | web w CI | S | T-PR3 |
| **A5** | Job `db`: `services: postgres:16`, `DATABASE_URL`, `CLMM_REQUIRE_DB_TESTS=1` → `test_db()` panikuje zamiast skip | `session_gl_integration` realnie sprawdza GL | S | T-PR3 |
| **A6** ✅ PR #6 | Hermetyczność reszty (§2): per-crate helper `test_env::EnvGuard` (lock + przywrócenie wartości w `Drop`) dla `api`/`data`/`cli`/`execution` (zastępuje ad-hoc `TEST_ENV_LOCK`, w tym lokalny lock w `position_agent_service.rs` — L3); `local_swap_fees` → tempdir; `wallets.rs:3834` bez zegara ściennego (wstrzyknięty czas albo szersze okno); **L2:** jednorazowo uruchomić testy z przemianowanym `data/ledger/` (lub pustym `CLMM_POSITION_LIFECYCLE_LEDGER_PATH` pod `EnvGuard`) i każdy test, który zmienia wynik, przepiąć na tempdir | 0 pozycji w tabeli hermetyczności i luk L2/L3 | M | — |
| **A7** ✅ PR #6 | **Strażnik hermetyczności w CI:** testy Rust uruchamiane bez sieci — `cargo test --no-run`, potem `sudo unshare -n sh -c 'ip link set lo up && cargo test --workspace'` (localhost mocki działają) | każdy nowy test z siecią pada w CI od razu | S | — |
| **A8** ✅ PR #7 | `critical-area-test-gate.sh`: dodać `wallet_gl_posting.rs`, `wallet_ledger*.rs`, `chain_portfolio.rs`, `chain_economic_totals.rs`, `position_stream_pnl.rs`, `position_chain_history.rs`, `data/src/wallet_session.rs`, `execution/src/strategy/session_capital.rs`, `data/migrations/*` | zmiana tych plików bez testu = fail | S | T-PR4 |
| **A9** ✅ PR #7 | Reguła agenta `.cursor/rules/test-integrity.mdc` (always apply): zasady §4, klasy padów §3, zakaz aktualizacji snapshotów/expected bez „Golden delta”; wpis w `AI_MERGE_CHECKLIST.md` | agent dostaje zasady w każdej sesji | S | R-PR3 (część) |
| **A10** | GitHub: branch protection `main` — wymagane checki `rust`, `web`, `db`, `quality_gates`; zakaz bezpośredniego push | G1 | S | R-PR1 |
| **A11** ✅ PR #7 | Szablon sekcji „Testy i kryteria regresji” (`doc/templates/TEST_SECTION.md`) + punkt w `AI_MERGE_CHECKLIST.md` i regule A9 („nowy plan bez sekcji testów = niekompletny”); dopisać sekcję do aktywnych planów Fali 2–3 (`WALLET_SESSION_CAPITAL_EXECUTOR_PLAN`, `POSITIONS_CLOSE_ALL_IMPLEMENTATION_PLAN`, `IMPLEMENTATION_PLAN_DECISION_LAYER`, `IMPLEMENTATION_PLAN_BOLLINGER_CANDLE_STRATEGIES`) na podstawie §7 | G8 | S | — |

**Kryterium fazy A:** PR nie może być zielony, gdy testy się nie kompilują, web się rozjechał typami, format jest zły, test wymaga sieci albo testy DB się nie wykonały.

### Faza B — goldeny finansowe (G3)

Narzędzie: **`insta`** (feature `json`) w `[workspace.dependencies]`, `cargo insta review` lokalnie; w CI (`CI=true`) snapshoty nie są zapisywane. Liczby w snapshotach **projektowane i zaokrąglane** (jak `to_shadow` w istniejącym goldenie: Decimal → 6 dp / USD → 8 dp), żeby szum float nie dawał diffów. Każdy fixture z `README.md` (źródło, data, co reprezentuje).

| ID | Golden | Wejście | Snapshot | Rozm. | Stare ID |
| -- | ------ | ------- | -------- | ----- | -------- |
| **B1** ✅ PR #8 | Sumy łańcucha 9vhKY | `nodes` z `chain_history_9vhKY.json` → `refresh_lineage_totals_from_nodes` | headline: net PnL, NAV end + source, IL, fees, cashflow, tx fees; per node net | M | T-PR5 |
| **B2** ✅ PR #9 | Salda SESSION / CHAIN z lifecycle | nowy fixture `lifecycle_9vhKY.jsonl` (wiersze łańcucha 9vhKY z lokalnego ledgera, tylko dane on-chain) → `aggregate_session_sums_*`, `aggregate_chain_sums_*`, `cap_open_debits_*` | mint → raw per sesja / chain | M | T-PR6 |
| **B3** ✅ PR #10 | Sizing reopen | tabela przypadków (prev_end, wallet notional, raw balances, caps) → `target_usd_*` (`pub(crate)`), `apply_portfolio_caps_to_wallet_raw`, `clamp_deposit_quote_to_portfolio` | target_usd + amounts | S | T-PR6 |
| **B4** ✅ PR #13 | Backtest mini | nowy `crates/cli/tests/fixtures/backtest_mini/` (kilkaset kroków candles + swaps, wycięte z lokalnych danych) + DTO → `StepData` → `run_single` dla każdej strategii | fees, IL, vs_hodl, rebalance_count, ranking | L | T-PR7 |
| **B5** ✅ PR #11 | Lineage multi-rotation | 4–5 rotacji (w tym fork i mismatch sesji) → trio ciągłości | shadow JSON (rozszerzenie istniejącego) | S | — |
| **B6** ✅ PR #12 | Ledger portfela łańcucha | lifecycle `Value` + open-start → `ledger_start_event_from_open_start`, `aggregate_chain_collected_fees`, `chain_balance_usd_legs_from_balances` | eventy + stopka USD | M | — |

Istniejący `lineage_shadow_expected.json` przeniesiony na `insta` (B5).

### Faza C — kontrakt i niezmienniki (G4, G5)

| ID | Zakres | Done when | Rozm. | Stare ID |
| -- | ------ | --------- | ----- | -------- |
| **C1** ✅ PR #17 | `openapi.json` wygenerowany z `openapi.rs` zacommitowany + test równości (update: `UPDATE_OPENAPI=1`) | zmiana pola = diff w PR | S | T-PR8a |
| **C2** ✅ PR #20 | `openapi-typescript` → `web/src/lib/api.gen.ts`; CI sprawdza, że wygenerowany plik = zacommitowany; **nowe** endpointy tylko z typów generowanych, stare migrowane przy okazji zmian (nie big-bang: `api.ts` ma 2603 linie) | rozjazd typów niemożliwy dla nowych endpointów | M | T-PR9b |
| **C3** ✅ PR #18 | `proptest` (workspace dep) — niezmienniki: (1) `net_pnl = current + cashflow − baseline − tx_fees` po `refresh_lineage_totals_from_nodes`; (2) po dowolnej sekwencji open/close/collect saldo per mint ≥ 0 (`cap_open_debits_*`); (3) `price_to_tick(tick_to_price(t)) == t` — osobno dla `domain::math::price_tick` (`f64`, zakres ticków ograniczony do używanego w pulach, np. \|t\| ≤ 443636 z weryfikacją granicy) i `protocols::orca::pool_reader` (L4); (4) `clean_il = current − hodl`, `lp_vs_hodl = clean_il + fees`; (5) lineage: close_n end = baseline_{n+1} przy zgodnej sesji | 5 property tests zielone | M | T-PR8b |
| **C4** ✅ PR #19 | Idempotencja GL w DB: ten sam wiersz lifecycle zastosowany 2× nie zmienia sald (job `db`) | test w `session_gl_integration` | S | — |
| **C5** ✅ | Skrypt CI: `BUGS.md` wpisy `high`/`critical` → nazwy testów z `Guards/tests` istnieją w kodzie (wyjątek: jawne `manual:`) | nowy critical bug bez testu nie przechodzi | S | T-PR9a |
| **C6** | Web: testy dla `whirlpoolTicks`, `chainCapital`, `sessionCapital`, `openPositionSwapEstimates`, `lineageLedgerOpenQuote`, `chainEconomicQuality`; ESLint config (albo usunięcie skryptu) | logika liczbowa web pokryta | M | — |

### Faza D — self-heal pipeline (G6) — po ≥ 2 goldenach z fazy B

| ID | Zakres | Done when | Rozm. | Stare ID |
| -- | ------ | --------- | ----- | -------- |
| **D1** ✅ PR #16 | CI check: zmiana w `**/tests/fixtures/**`, `**/snapshots/**`, `openapi.json` ⇒ opis PR musi mieć sekcję `Golden delta:` | brak uzasadnienia = fail | S | R-PR4 |
| **D2** | `CODEOWNERS` na fixtures/snapshots | review właściciela wymagane | S | — |
| **D3** | `.cursor/BUGBOT.md`: flaga przy osłabionej asercji, usuniętym snapshocie, nowym `#[ignore]`, `return` w teście | komentarz Bugbota na PR | S | R-PR4 |
| **D4** | Szablon triage pada CI (klasy §3) dla agenta (`@cursor` / Cloud): wolno `stale_test`, `non_hermetic`, `infra`; `economic_regression` → tylko raport + BUGS.md | agent nie zmienia expected bez GO | S | R-PR3, R-PR5 |
| **D5** | Jednorazowy `cargo-mutants` na `chain_economic_totals`, `wallet_session` (agregatory), sizing w `session_capital` → lista przeżytych mutantów → brakujące asercje dopisane do B/C; potem opcjonalnie nightly | przeżyte mutanty w module < 10% | M | — |

### Faza E — tempo (G7), równolegle od A

| ID | Zakres | Rozm. | Stare ID |
| -- | ------ | ----- | -------- |
| **E1** | `verify-fast`: `cargo test -p` dla crate'ów z diffu + zależnych; web `tsc`+`vitest` tylko gdy ruszony `web/` lub `openapi.json` | S | R-PR7 |
| **E2** | `cargo nextest` lokalnie i w CI (równoległość, lepsze raporty flaków) | S | — |
| **E3** | Szablon ticketu dla agenta: ID z planu, zakazane pliki, komenda verify, „characterization najpierw” przy PnL/GL/backtest | S | R-PR6 |

### Faza R — czytelne i spójne wyniki testów (G9), od fazy A

Zasada: **jeden format, jedno słownictwo, te same obszary** co w master planie — lokalnie i w CI. Bez zewnętrznych serwisów (zgodnie z regułą „dane za darmo”): tylko GitHub Actions (Job Summary, artefakty, komentarz PR).

**Obszary projektu** (stałe; odpowiadają subsystemom z `MASTER_IMPLEMENTATION_PLAN.md` §2.2):

| Obszar | Zawiera (przykłady modułów) | Fale |
| ------ | --------------------------- | ---- |
| **Ekonomia / GL** | `wallet_gl_posting`, `wallet_session`, `chain_portfolio`, `session_capital` | F2 |
| **Lineage / PnL** | `position_stream_lineage`, `position_stream_pnl`, `chain_economic_totals`, `position_chain_history` | F2 |
| **Execution / rebalance** | `execution::strategy::*`, `DecisionEngine`, `protocols::orca::executor` | F2, F5 |
| **Dane / decode** | `cli::swap_sync`, enrich/decode, `snapshot_readiness`, `data-health-check` | F1 |
| **Backtest / strategie** | `simulation`, `optimization`, `cli::backtest_engine` | F3, F5 |
| **Decision layer / agent** | `orchestrator_gate`, `agent_decision`, apply policy, memory | F3, F4 |
| **Domain math** | `domain` (tick, IL, liquidity) | wszystkie |
| **API / kontrakt** | handlers, OpenAPI snapshot, routes | wszystkie |
| **Web** | `web/src/**` | F2, F4 |

| ID | Zakres | Done when | Rozm. |
| -- | ------ | --------- | ----- |
| **R1** | Mapa obszarów `ci/test-areas.toml`: prefiks ścieżki testu (crate::moduł, plik web) → obszar; bez zmiany nazw testów. Nieprzypisane → „Inne” + ostrzeżenie | 100% testów ma obszar | S |
| **R2** | Raport testów: `cargo nextest` (E2) z JUnit + `vitest --reporter=junit` → skrypt `tools/test_report` → **Markdown + JSON** o tym samym układzie. `make verify` drukuje tabelę na końcu; CI wkleja ją do **Job Summary** i zapisuje JSON jako artefakt | ten sam raport lokalnie i w CI | M |
| **R3** ✅ | **Golden delta po ludzku:** przy zmianie snapshotów skrypt porównuje stary/nowy `.snap` (liście liczbowe) i publikuje komentarz PR / sekcję summary: tabela „fixture / metryka / było / jest / Δ / Δ%”, posortowana po |Δ USD|; ten sam tekst wklejany do sekcji „Golden delta” (D1) | recenzent widzi zmianę pieniędzy bez czytania JSON | M |
| **R4** | **Trend:** workflow tygodniowy (`schedule`) zbiera JSON z ostatnich runów na `main` (artefakty GitHub) → summary: liczba testów per obszar, pady, testy niestabilne (pad na `main` bez zmiany kodu w obszarze), czas CI, liczba goldenów / niezmienników per obszar | tygodniowy obraz jakości w jednym miejscu | M |
| **R5** | Kolumna **„Ochrona testami”** w tabeli dojrzałości `MASTER_IMPLEMENTATION_PLAN.md` §2.2 (brak / unit / golden / golden+niezmiennik), aktualizowana z raportu R4 przy zamknięciu fali | status rozwoju i status ochrony w jednej tabeli | S |

**Format raportu (wzór, identyczny lokalnie i w CI):**

```text
Test report — feat/xyz @ a1b2c3d (2026-10-02 14:10)   PASS 702 | FAIL 1 | IGNORED 24 | 3m12s

Obszar                  Testy  Pady  Golden  Niezm.  Czas
Ekonomia / GL             118     0       2       2   14s
Lineage / PnL              96     1       2       1   11s
Execution / rebalance     107     0       1       0   22s
...
FAIL  Lineage / PnL  chain_economic_totals::tests::refresh_9vhky_totals_golden
      net_pnl_usd: było 1.234567  jest 1.198201  Δ -0.036366 (-2.9%)
Klasa (§3): economic_regression → wymaga decyzji człowieka
```

Każdy pad dostaje w raporcie **klasę wstępną z §3** według typu testu (golden / niezmiennik → `economic_regression`; błąd kompilacji testu → `stale_test`; pad tylko w CI albo tylko pod `unshare -n` → `non_hermetic`), żeby od razu było widać, czy trzeba „naprawić test”, czy „zmieniły się pieniądze”. Klasę ostatecznie potwierdza człowiek albo triage z D4.

---

## 6. Kolejność i zależności

```text
0.1–0.5 ─► A1 ─► A2 ─► A3/A4/A5 ─► A6 ─► A7 ─► A8/A9/A10/A11
                     │        └─► E2 ─► R1 ─► R2 ─► R4 ─► R5
                     └─► B1 ─► B2 ─► B3 ─► B5 ─► B6 ─► B4
                           └─► R3
                                 └─► C1 ─► C3 ─► C4 ─► C2 ─► C5 ─► C6
                                                   │
                     (≥2 goldeny) ─► D1 ─► D2 ─► D3 ─► D4 ─► D5
E1, E3: równolegle od A2
```

**Sprint 1 (najbliższy):** 0.1–0.5 (w PR #2) → A1 → A2 → A3+A4+A5 → A9 + A11. Efekt: G1/G2/G6 w dużej części, CI szybkie, nowe plany od razu z sekcją testów.  
**Sprint 2:** A6–A8, A10, E2, R1, R2, B1, B2. Efekt: pierwsze pieniądze pod goldenem, czytelny raport per obszar w każdym PR.  
**Sprint 3:** B3, B5, R3, C1, C3, C4. Efekt: testowe kryteria F2.1–F2.4 spełnione (§7).  
**Sprint 4:** B6, B4, C2, C5, C6, D1–D4, R4, R5. **Potem:** D5, E1, E3.

Faza 0–A i R1–R2 **nie czekają** na rozwój; testy z §7 dla danej fali powstają **razem z pracą nad tą falą** (zasada 7).

---

## 7. Mapa rozwoju → testy (G8)

Źródło fal: [`MASTER_IMPLEMENTATION_PLAN.md`](MASTER_IMPLEMENTATION_PLAN.md) §4 (stan 2026-05-20; bugi Fali 2 potwierdzone jako nadal aktywne 2026-09-30, §2) i [`ROADMAP.md`](ROADMAP.md). **Test exit gate** = warunek zamknięcia pozycji: testy istnieją, są zielone w raporcie R2 w swoim obszarze i (dla bugów) są wpisane w `Guards/tests` w `BUGS.md` (C5).

### Fala 1 — fundament danych (obszar: Dane / decode)

| Pozycja | Test exit gate | Typ |
| ------- | -------------- | --- |
| F1.2 decode rebuild | Fixture zapisanych transakcji on-chain (Orca / Raydium / Meteora, po kilka: swap A→B, B→A, nie-swap, niepełne dane) → oczekiwany `decoded_swaps` + `decode_status` (`ok` / `loose` / fail) | golden |
| F1.4 snapshot readiness | Mini fixture `snapshots.jsonl` (z lukami czasu) → raport readiness; obecny `snapshot_readiness_regression_test` (~21 s, 7 procesów) przepisany na wywołanie funkcji zamiast spawnów | golden + hermetyczność |
| F1.5 gate w harmonogramie | `orchestrator-gate --fail-on-no-go`: słabe dane → exit ≠ 0 i brak rankingu | niezmiennik (NO-GO) |
| Metryka „decode OK ≥ 65%” | Liczona przez `data-health-check` — test progu i formatu raportu (fixture) | unit |

### Fala 2 — stabilność live (obszary: Lineage / PnL, Ekonomia / GL, Execution, Web)

| Pozycja | Bug | Test exit gate | Typ / ID |
| ------- | --- | -------------- | -------- |
| F2.1 reopen po rotacji | BUG-20260410-06 `open` | Sekwencja close → open na fałszywym executorze: po close rotacji zawsze open albo jawny stan błędu z powodem (nigdy „wisi”); **najpierw sprawdzić seam** (czy `rebalance` da się uruchomić bez RPC — jeśli nie, wydzielić funkcję decyzji) | niezmiennik |
| F2.2 reopen downsizing | BUG-20260512-03 `regressed` | Przypadek ~$10 → ~$4 odtworzony jako wiersz goldena sizing: notional ≥ target × (1−ε) albo jawny `session_cap_*` | **B3** |
| F2.3 lineage parent | BUG-20260413-05 `regressed` | Manual open ≠ false parent; rebalance bota łączy łańcuch (przypadki w fixture multi-rotation) | **B5** + C3(5) |
| F2.4 valuation vs lineage | — | Performance ≈ lineage baseline na 9vhKY; `net_pnl` identity | **B1** + C3(1) |
| F2.5 session capital default | — | Reopen/open respektuje `SessionMintCaps` przy fladze on/off (istniejące testy `session_capital` pod `EnvGuard`) + saldo SESSION nigdy < 0 | B2 + C3(2) |
| F2.6 PositionCreate wallet race | BUG-20260413-06 `open` | Logika „czy salda są dla właściwego api-signer” wydzielona do `web/src/lib` + test | C6 |
| F2.7 session GL na close/collect | — | `cost_session_id` w żądaniu close/collect z PositionDetail = ten sam co w create (test funkcji budującej request) | C6 |
| F2.8 close-all persistence | — | Job zapisany → „restart” (nowy store z tego samego tempdir / DB) → job odczytany ze statusem | unit + `db` |
| F2.9 UI regression | BUG-20260410-04 `open` | Komunikaty collect/swap przepuszczane 1:1 z API — test funkcji mapującej odpowiedź na komunikat (lib), bez E2E | C6 |

Metryka master planu „Lineage chain integrity: 0 regresji BUG-20260413-05 w testach golden” jest spełniona przez **B5** — od tej chwili liczona w raporcie R4.

### Fala 3 — decision layer MVP (obszar: Decision layer / agent, Backtest)

| Pozycja | Test exit gate | Typ |
| ------- | -------------- | --- |
| F3.3 raport rankingowy | Fixture wyników optimize → raport (N wariantów, winner vs obecny `width_pct`) | golden |
| NO-GO (zasada nadrzędna 3) | Słabe dane → brak rankingu i brak apply, niezależnie od wejścia | niezmiennik (proptest) |
| F3.4 real vs sim composite | Tabela optimize + wiersz stream-pnl na fixture 9vhKY | golden |
| F3.6 apply policy | `optimize_apply_policy` × źródło (subprocess / HTTP) → 200 / 409; `approved: false` nie zmienia executora | tabela przypadków (API) |
| Audyt JSONL (zasada 4) | Schemat wpisu `agent_decisions.jsonl` jako snapshot; odtworzenie wyniku z wpisu | kontrakt |

### Fala 4 — pamięć + produkt równoległy

| Pozycja | Test exit gate | Typ |
| ------- | -------------- | --- |
| F4A rolling memory | Schemat `global.json` / `position/{addr}.json` jako snapshot; hook gate → plik zaktualizowany (tempdir) | kontrakt + unit |
| F4B1 experiment launcher | Kontrakt API batcha (C1/C2); batch przeżywa restart (jak F2.8) | kontrakt |
| F4B2 shadow (ROADMAP) | **Historia przypisań append-only:** żadna operacja nie nadpisuje przypisania pozycja ↔ strategia ↔ rola; shadow nigdy nie wysyła tx (fałszywy executor liczy wywołania = 0) | niezmienniki |

### Fala 5 — research

| Pozycja | Test exit gate | Typ |
| ------- | -------------- | --- |
| F5.2 Bollinger / last-candle | Wskaźnik na fixture świec (SMA, ±K·σ, ostatnia zamknięta świeca) → golden; nowa strategia **automatycznie** dostaje wiersz w B4; niezmiennik „brak ruchu ceny ⇒ brak rebalance” | golden + niezmiennik |
| F5.1 fee truth / events | Fixture eventów → fees; porównanie z proxy jako **raport trendu**, nie asercja dolarowa (reguła „fee proxy”) | golden (trend) |
| F5.4 Meteora/Raydium live | Decode na fixture (jak F1.2); tx tylko devnet `#[ignore]` | golden + manual |
| F5.6 WebSocket invalidation | Funkcja „event → klucze React Query do unieważnienia” w lib + test | unit (web) |
| Każda nowa strategia / venue | Wiersz w B4 + kontrakt OpenAPI (C1) | golden + kontrakt |

---

## 8. Dzień pracy po fazie A

1. Jeden ID z planu (albo PR produktowy z master planu / `IMPLEMENTATION_PLAN.md`), gałąź.
2. Sprawdź w §7, jakie testy wymaga ta pozycja; rusza PnL / GL / sizing / backtest → **najpierw** golden (istniejący albo nowy) na obecnym zachowaniu.
3. Implementacja (agent), `verify-fast`, przed push `make verify` (hook) → raport per obszar (R2).
4. Snapshot się ruszył → tabela R3 w sekcji „Golden delta” w PR albo to jest bug.
5. Review: czy test nie został osłabiony (Bugbot + checklist); czy obszar w raporcie jest zielony.

---

## 9. Kryteria sukcesu

| Po | Sprawdzenie |
| -- | ----------- |
| Faza 0 | PR #2: wszystkie checki poza coverage/docker zielone |
| Faza A | Celowo zepsuty test / typ web / format / test z siecią / brak DB → każdy daje czerwony PR |
| Faza B | Zmiana wzoru net PnL o 1% w `chain_economic_totals` → czerwony golden B1 |
| Faza C | Usunięcie pola z modelu API → czerwony C1 i `tsc`; zamiana znaku w agregatorze → czerwony proptest |
| Faza D | Agent poproszony „napraw CI” przy padzie golden nie zmienia snapshotu (test na sucho) |
| Faza R | Dowolny PR: w Job Summary tabela per obszar identyczna z wyjściem `make verify`; zmiana goldena → tabela było/jest/Δ w PR; tydzień → raport trendu |
| Rozwój (G8) | Fala 2 master planu nie zostaje zamknięta, dopóki F2.1–F2.9 z §7 nie mają zielonych testów; nowy `IMPLEMENTATION_PLAN_*` bez sekcji „Testy” nie przechodzi review |
| Ciągle | Każdy nowy `high`/`critical` w `BUGS.md` ma istniejący test |

---

## 10. Poza zakresem

Playwright / E2E UI; coverage % jako cel (`code_coverage.yml` — osobna diagnoza); devnet E2E w CI; self-healing lokatorów/asercji; pełna migracja `api.ts` naraz; zewnętrzne dashboardy testów (płatne SaaS) — raport żyje w GitHub Actions i lokalnie.

---

## Document status

| Field | Value |
| ----- | ----- |
| Role | Jedyny plan wykonawczy testów / odporności na regresje |
| Created / rewritten | 2026-09-30 (analiza: hermetyczność, testowalność ścieżek pieniędzy, CI PR #2); tego samego dnia uzupełniony o G8/G9, fazę R (raport per obszar) i §7 (mapa fal master planu → testy) |
| Next review | Przy zamknięciu każdej fali master planu albo zmianie priorytetu B1 vs B2 (Fala 4) |
| Supersedes | backlog T-PR2…T-PR9 z `TESTING_REGRESSION_PLAN.md`, R-PR1…R-PR7 z pierwszej wersji tego pliku (mapowanie w kolumnie „stare ID”) |
