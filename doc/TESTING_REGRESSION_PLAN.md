# Plan: siatka testów chroniąca przed regresjami

**Status:** audyt 2026-09-30 + log wykonania (§6). T-PR1 zrobione (PR #2). **§3–§4 zastąpione** — jedyny plan wykonawczy: [`IMPLEMENTATION_PLAN_REGRESSION_RESILIENCE.md`](IMPLEMENTATION_PLAN_REGRESSION_RESILIENCE.md) (fazy 0/A–E; mapowanie T-PR → nowe ID w kolumnie „stare ID”).  
**Data:** 2026-09-30  
**keywords:** testing, regression, golden-fixture, make-verify, pre-push, ci, postgres-integration, vitest, tsc, api-contract, openapi, invariants, wallet-gl, lineage, stream-pnl, chain-portfolio, backtest

**Powiązane:** [`AI_MERGE_CHECKLIST.md`](AI_MERGE_CHECKLIST.md), [`BUGS.md`](BUGS.md), [`ENGINEERING_NOTES.md`](ENGINEERING_NOTES.md), `scripts/ci/critical-area-test-gate.sh`, `.github/workflows/*.yml`

---

## 1. Cel

Nowe prace (feature, refactor, fix) **nie mogą po cichu zmieniać** zachowania, które już działa — w szczególności liczb finansowych (PnL, IL, fees, salda GL, wynik backtestu) i kontraktów API ↔ web. Każda zmiana takiego zachowania ma być **widoczna w diffie** (test / fixture) i **świadomie zaakceptowana**.

---

## 2. Stan wyjściowy (audyt 2026-09-30)

| Obszar | Co jest | Stan |
| ------ | ------- | ---- |
| Testy Rust | ~650 (`api` 261, `execution` 107, `data` 79, `cli` 52, `protocols` 42, `domain` 41, `simulation` 37, `optimization` 31) | **Nie kompilują się** — `cargo test --workspace` pada na `E0063: missing field chain_session_id in OpenPositionRequest` (`crates/api/src/services/position_service.rs:1362`, inicjalizator testowy). Żaden test w workspace się nie uruchamia. |
| Testy web (vitest) | 9 plików / 49 testów, wyłącznie `web/src/lib/*.test.ts` | Zielone |
| Typy web (`tsc --noEmit`) | cały `web/` | **4 błędy** w testach (`solFirstFunding.test.ts` ×3, `experimentArm.test.ts` ×1) — rozjazd z typami w `api.ts` (np. usunięte `amount_raw` w `WalletTokenBalance`). Vitest tego nie łapie (nie typuje). |
| Integracja Postgres | `crates/data/tests/session_gl_integration.rs` | Bez `DATABASE_URL` testy **cicho przechodzą** (skip). CI nie ma serwisu Postgres → w CI zawsze skip. |
| Golden test | `lineage_shadow_diff_matches_golden_fixture` (`position_stream_lineage.rs`) + `crates/api/tests/fixtures/lineage_shadow_expected.json` | Jedyny golden; uruchamiany w `quality_gates.yml` |
| Gate obszarów krytycznych | `scripts/ci/critical-area-test-gate.sh` | Tylko na PR do `main`; lista plików krytycznych nie obejmuje Wallet GL / chain portfolio / `session_capital` |
| CI GitHub | tests, lint, fmt, build, coverage, docker, quality_gates | Ostatni push 2026-05-21; **Lint i Coverage czerwone**. Brak joba dla `web/`. |
| Proces | praca bezpośrednio na `main` | ~7000 niezacommitowanych linii / 42 pliki — poza jakąkolwiek weryfikacją CI |

**Wspólny mechanizm regresji:** zmiana kontraktu (pole w modelu Rust, typ TS) psuje sąsiedni kod, a nic tego nie zatrzymuje, bo pełny zestaw testów nie jest uruchamiany rutynowo.

---

## 3. Warstwy (kolejność = priorytet)

> **Zastąpione** przez [`IMPLEMENTATION_PLAN_REGRESSION_RESILIENCE.md`](IMPLEMENTATION_PLAN_REGRESSION_RESILIENCE.md) §5. Poniżej wersja historyczna — nie realizować z tej listy.

### Warstwa 1 — Zielony baseline (warunek wszystkiego)

1. Naprawić kompilację testów `clmm-lp-api` (brakujące `chain_session_id` w inicjalizatorach testowych).
2. Naprawić 4 błędy TS w testach web (dostosować fixture'y do aktualnych typów `api.ts`).
3. Doprowadzić `make lint` (`clippy -D warnings`) do zera ostrzeżeń.
4. Zacommitować bieżącą pracę **w logicznych commitach** (np. chain portfolio; Wallet GL faza B/C/D; web panele), każdy z zielonym `cargo test`. Tymczasowe pliki (`tmp_*.json`, `data/dexscreener-cache/*`) — przenieść do fixtures albo `.gitignore`.

**Kryterium done:** `cargo test --workspace`, `make lint`, `npx tsc --noEmit`, `npm test` (web) — wszystko zielone lokalnie i w CI.

### Warstwa 2 — Jedna komenda + automatyczne wymuszenie

1. **`make verify`** (i odpowiednik PowerShell dla Windows, np. `tools/verify.ps1`):
   - `cargo fmt --check`
   - `cargo clippy --all-targets --all-features -- -D warnings`
   - `cargo test --workspace`
   - `web/`: `npx tsc --noEmit`, `npm test`, `npm run lint`
2. **Hook pre-push** (git hook w repo, instalowany skryptem) uruchamiający `make verify` — opcjonalnie szybszy wariant pre-commit (`fmt --check` + `tsc`).
3. **CI:**
   - nowy workflow `web.yml`: `npm ci` / `npm install`, `tsc --noEmit`, `vitest run`, `eslint`;
   - job z `services: postgres:16` + `DATABASE_URL`, uruchamiający `cargo test -p clmm-lp-data --test session_gl_integration` (i przyszłe testy DB);
   - w CI brak `DATABASE_URL` w jobie DB = **błąd**, nie skip (np. env `CLMM_REQUIRE_DB_TESTS=1` → test panikuje zamiast `return`).
4. Rozszerzyć listę plików krytycznych w `critical-area-test-gate.sh` o: `wallet_gl_posting.rs`, `wallet_ledger*.rs`, `chain_portfolio.rs`, `chain_economic_totals.rs`, `position_stream_pnl.rs`, `position_chain_history.rs`, `crates/data/src/wallet_session.rs`, `crates/execution/src/strategy/session_capital.rs`, `crates/data/migrations/*`.
5. Przyjąć zasadę: praca na gałęziach + PR do `main` (żeby `quality_gates.yml` w ogóle się uruchamiał).

### Warstwa 3 — Golden testy ścieżek finansowych (najważniejsza ochrona logiki)

Wzorzec do powielenia: `lineage_shadow_diff_matches_golden_fixture` — zamrożone wejście → deterministyczne wyjście → porównanie z plikiem `expected.json` w `crates/*/tests/fixtures/`.

| Kandydat | Wejście (fixture) | Oczekiwane wyjście |
| -------- | ----------------- | ------------------ |
| stream-pnl / lineage łańcucha | `crates/api/tests/fixtures/stream_lineage_9vhKY.json`, `chain_history_9vhKY.json` (zrzuty API z 2026-05-26; tylko publiczne dane on-chain) | headline net PnL, IL, fees, baseline/end NAV |
| `chain_economic_totals` | ten sam łańcuch rotacji | sumy netto łańcucha |
| Wallet GL posting | zestaw wierszy lifecycle (open / close / collect / rebalance / tx fee) | posty per konto (`SESSION:`, `CHAIN:`, wallet, tx fee) i salda |
| `session_capital` / rebalance sizing | stan sesji + ceny | wyliczone kwoty open |
| `backtest` / `backtest-optimize` | mały zamrożony wycinek `snapshots.jsonl` + `decoded_swaps.jsonl` | metryki wyniku (fees, IL, vs_hodl, ranking top-N) |

Zasady:

- Porównanie strukturalne JSON (z tolerancją dla floatów, np. `1e-9` względnie), nie stringowe.
- Aktualizacja fixture **tylko świadomie**: tryb `UPDATE_GOLDEN=1 cargo test …` nadpisuje `expected.json`; diff fixture'a jest częścią review.
- Fixture bez sekretów / kluczy prywatnych; pubkeye mogą zostać (dane publiczne on-chain).
- Deterministyczność: brak `now()`, RPC, losowości w ścieżce testowanej (wstrzykiwany czas / ceny).

### Warstwa 4 — Kontrakty i niezmienniki

1. **Kontrakt API:** snapshot wygenerowanego OpenAPI (`openapi.rs`) w pliku repo + test porównujący — dodanie/usunięcie pola = widoczny diff.
2. **Docelowo:** generowanie typów TS (`web/src/lib/api.ts` lub osobny `api.gen.ts`) z OpenAPI, żeby web i backend nie mogły się rozjechać.
3. **Niezmienniki (unit / property tests, np. `proptest`):**
   - księgowanie GL: suma delt per mint w jednym zdarzeniu bilansuje się (z uwzględnieniem kont zewnętrznych);
   - idempotencja postingu (ponowne zastosowanie tego samego `event_id` nie zmienia sald);
   - zgodność GL ↔ PSLR (`gl_pslr_match`);
   - ciągłość lineage (close PDA n → open PDA n+1, baseline/end);
   - matematyka domeny: konwersje tick ↔ price odwracalne, IL ≤ 0 względem HODL bez fees.
4. **`BUGS.md` → testy:** każdy wpis `high`/`critical` ma w polu `Guards/tests` nazwę istniejącego testu; skrypt CI sprawdza, że wskazane nazwy testów istnieją w kodzie.

---

## 4. Proponowana kolejność PR

| PR | Zakres | Warstwa |
| -- | ------ | ------- |
| T-PR1 | Naprawa kompilacji testów api + błędy TS + clippy | 1 |
| T-PR2 | `make verify` + `tools/verify.ps1` + hook pre-push | 2 |
| T-PR3 | CI: `web.yml` + job Postgres + wymuszenie DB testów | 2 |
| T-PR4 | Rozszerzenie `critical-area-test-gate.sh` | 2 |
| T-PR5 | Helper golden (porównanie JSON z tolerancją, `UPDATE_GOLDEN`) + golden stream-pnl / chain totals | 3 |
| T-PR6 | Golden Wallet GL posting + `session_capital` | 3 |
| T-PR7 | Golden backtest na zamrożonym wycinku danych | 3 |
| T-PR8 | Snapshot OpenAPI + niezmienniki GL / lineage | 4 |
| T-PR9 | Skrypt `BUGS.md` ↔ testy w CI; (opcjonalnie) generowanie typów TS | 4 |

---

## 5. Ryzyka / otwarte kwestie

- **Wydajność:** pełny `cargo test --workspace` lokalnie trwa kilka minut (sama kompilacja ~4 min przy zimnym cache) — hook pre-push może być odczuwalny; rozważyć `cargo nextest` i wariant „szybki” (crate'y zmienione w diffie).
- **Testy devnet** (`devnet_e2e_tests.rs`, 23× `#[ignore]`) zostają ręczne — nie wchodzą do CI.
- **Golden a zamierzone zmiany:** przy refactorach ekonomii (np. `CHAIN_ECONOMIC_NET_REFACTOR_PLAN.md`) fixture'y będą się zmieniać; wymagane uzasadnienie delty w PR (zgodnie z punktem *Shadow/diff check* w `AI_MERGE_CHECKLIST.md`).
- **Coverage workflow** jest czerwony z nieustalonego jeszcze powodu — do zdiagnozowania osobno (nie blokuje warstw 1–3).

---

## 6. Log wykonania

### T-PR1 (2026-09-30) — zielony baseline

| Sprawdzenie | Przed | Po |
| ----------- | ----- | -- |
| `cargo test --workspace` | nie kompiluje się (E0063) | **634 pass / 0 fail / 24 ignored** |
| niestabilne testy `session_capital` (env race) | 1–2 FAIL losowo | 5× z rzędu zielone (`TEST_ENV_LOCK`) |
| `cargo clippy --all-targets --all-features -D warnings` | ~45 błędów (protocols, execution, api) | **0** |
| `web: npx tsc --noEmit` | 4 błędy | **0** |
| `web: vitest run` | 49/49 | 49/49 |

Odkryte przy okazji (nie naprawione w T-PR1):

- **BUG-20260930-02** — zgadywane decimals mintów w GL opening import / ledger CHAIN (latentny błąd kwot dla tokenów ≠ SOL/USDC).
- **BUG-20260930-03** — `format_check.yml` uruchamia `make fmt` (formatuje) zamiast `--check` → zawsze zielony; `cargo fmt --all --check` zgłasza 38 plików. Do T-PR2: jeden osobny commit `cargo fmt --all` (po zacommitowaniu bieżącej pracy) + zmiana workflow na `make fmt-check`.
- **`npm run lint` w `web/`** nie działa — brak pliku konfiguracyjnego ESLint (`.eslintrc*`). Do T-PR2: dodać config albo usunąć skrypt z `make verify`, zanim hook go wymusi.

### Faza 0 (2026-09-30) — PR #2 w CI (plan: `IMPLEMENTATION_PLAN_REGRESSION_RESILIENCE.md` 0.1–0.4)

| Pad CI | Zmiana | Lokalnie |
| ------ | ------ | -------- |
| `run_tests` (`backfill_9vhky_*`, mainnet RPC) | 3 hermetyczne testy + `#[ignore]` manual repair; `EnvGuard` w `clmm-lp-api` (też `position_close_signer`) | workspace 0 fail; api 240 pass / 24 ignored |
| `critical_area_requires_tests` (brak skryptu) | `.gitignore` `/scripts/*` + `!/scripts/ci/`; skrypt w repo | — |
| `semver-*` (`main^{tree}`) | `baseline-rev: origin/main` | — |

Wynik CI PR #2 (`b5d73ba`): zielone `run_tests` (2 runy), `lint`, `build`, `semver-*` ×7, `critical_area_requires_tests`, `lineage_shadow_diff`, `format_check` (nadal nie sprawdza — BUG-20260930-03, A1), **`code_coverage_report` też zielony** (był czerwony od maja; tarpaulin uruchamia te same testy, więc najpewniej padał na nich — przyczyny historycznej nie weryfikowano).

PR #2 zmergowany do `main` 2026-10-01 (`f823d0f`, merge commit).

### A10 (2026-10-01) — branch protection `main`

Tylko przez PR (0 wymaganych review), obowiązuje też admina, bez force push / usuwania. Wymagane checki: `run_tests`, `lint`, `build (ubuntu-latest)`, `build (ubuntu-22.04)`, `critical_area_requires_tests`, `lineage_shadow_diff`, od A1 także `format_check`. Coverage i Docker — niewymagane. Po A3–A5 (job `rust`, `web`, `db`) zaktualizować listę.

### A1 (2026-10-01) — format egzekwowany w CI

`cargo fmt --all` jako osobny commit `6b0c849` (38 plików, wyłącznie format) + `.git-blame-ignore-revs`; `format_check.yml` → `make fmt-check` (`cargo fmt --all --check`). BUG-20260930-03 → fixed. Lokalnie: `git config blame.ignoreRevsFile .git-blame-ignore-revs`.
