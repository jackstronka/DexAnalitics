# Plan: siatka testów chroniąca przed regresjami

**Katalog (co jest + jak odpalić):** [`TESTS.md`](TESTS.md).

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

### A2 (2026-10-01) — `make verify` + hook pre-push

`make verify` (= `verify-rust`: fmt-check, clippy `-D warnings`, `cargo test --workspace`; `verify-web`: `tsc --noEmit`, `vitest run`) i `tools/verify.ps1` (Windows; flagi `-SkipRust` / `-SkipWeb`; fail-fast + tabela krok / status / czas). `.githooks/pre-push` patrzy na pliki w pushowanym zakresie: tylko docs → pomija; tylko `web/` → same kroki web; Rust / `Cargo.*` / `Makefile` / `.github/` → kroki Rust. Włączenie: `git config core.hooksPath .githooks` (per klon). `.gitattributes`: `*.sh` i `.githooks/*` z LF. `npm run lint` poza verify (brak configu ESLint — C6). Sprawdzone: ścieżka FAIL (fmt) zatrzymuje się na 1. kroku; docs-only i delete → skip; web-only → 2 kroki web (~25 s); pełna ścieżka Rust — przy pushu tej gałęzi.

### A3–A5 (2026-10-01) — jeden `ci.yml`: `rust` / `web` / `db`

- **`rust`** (zastępuje `tests.yml`, `lint.yml`, `build.yml`, `format_check.yml`): fmt-check → clippy `-D warnings` → `cargo test --workspace`, jedna kompilacja, `Swatinem/rust-cache`. Osobny `cargo build` usunięty (clippy `--all-targets` kompiluje wszystkie cele; release build robią obrazy Docker). Matryca `ubuntu-22.04` usunięta.
- **`web`**: `npm install` (lock gitignorowany) → `tsc --noEmit` → `vitest run`.
- **`db`**: `postgres:16` jako service, `CLMM_REQUIRE_DB_TESTS=1` → `session_gl_integration` bez bazy = FAIL (wcześniej cichy pass). Lokalnie bez env dalej skip.
- Triggery: PR do `main` / `release/**` + push na `main` (koniec podwójnych runów push+PR na gałęziach); `concurrency` anuluje stare runy PR. `quality_gates`: usunięty duplikat `lineage_shadow_diff` (golden jest w `cargo test --workspace`). `code_coverage`: tylko push `main` / ręcznie.
- Branch protection: wymagane `rust`, `web`, `db`, `critical_area_requires_tests`.

### A6–A7 (2026-10-01) — hermetyczne testy + CI bez sieci

- **A6:** `test_env::EnvGuard` (wspólna blokada + przywracanie w `Drop`, także przy panice) w `api`, `cli`, `data`, `execution`; każdy test zmieniający env go używa (`session_capital::TEST_ENV_LOCK` i lokalny lock `position_agent_service` usunięte). `local_swap_fees`: test pisze do tempdir (w testach `repo_data_dir()` bez ustawionego katalogu = panic, nie repo `data/`). `wallets.rs` stale-age: wstrzyknięty zegar, dokładnie 6000 ms. `test_state_no_db` (lineage): `fallback_urls` puste (wcześniej `RpcConfig::default()` dokładał publiczne RPC mainnet).
- **A7:** job `rust`: `cargo test --workspace --no-run`, potem `unshare -rn` (tylko loopback) + `--offline --no-fail-fast`. Pierwszy run: 0 testów wymagało sieci (api lib 1,4 s w CI vs ~10 s lokalnie — lokalnie część testów dalej próbuje sieci, patrz BUG-20260930-04 residual).

### A8, A9, A11 (2026-10-01) — bramka plików krytycznych, reguła agenta, sekcja testów w planach

- **A8:** `scripts/ci/critical-area-test-gate.sh` — lista rozszerzona o GL / SESSION / lineage-PnL / migracje (`wallet_gl_posting`, `wallet_ledger*`, `chain_portfolio`, `chain_economic_totals`, `position_stream_pnl`, `position_chain_history`, `data/wallet_session`, `execution/session_capital`, `data/migrations/*`). **Naprawiona luka:** wcześniej „test w PR” = dowolny zmieniony `.rs`, który *zawiera* `#[test]` / `mod tests` — pliki krytyczne same mają moduły testów, więc bramka przepuszczała każdą zmianę. Teraz liczy się plik testowy (`tests/`, `*_tests.rs`, `*.test.ts(x)`, `*.snap`) albo **dodane** linie testowe w diffie (`#[test]`, `#[tokio::test]`, `assert*!`, `proptest!`, `insta::assert`). Furtka: etykieta PR `no-tests-needed` (powód w opisie PR); workflow reaguje na `labeled` / `unlabeled`. `rg` niepotrzebny (grep). Sprawdzone lokalnie: docs-only → pass, krytyczny bez testu → FAIL, z etykietą → skip, z dodanym `#[test]` → pass.
- **A9:** `.cursor/rules/test-integrity.mdc` (always apply) — klasy padów §3 z dozwolonymi akcjami, zasady §4, zakaz edycji goldenów bez „Golden delta”. `AI_MERGE_CHECKLIST.md`: punkty test integrity / golden delta / hermetyczność / sekcja testów w planie.
- **A11:** `doc/templates/TEST_SECTION.md`; sekcja „Testy i kryteria regresji” w `WALLET_SESSION_CAPITAL_EXECUTOR_PLAN` (§8.3), `POSITIONS_CLOSE_ALL_IMPLEMENTATION_PLAN` (§9.1), `IMPLEMENTATION_PLAN_DECISION_LAYER`, `IMPLEMENTATION_PLAN_BOLLINGER_CANDLE_STRATEGIES` — z mapy §7.

### B1 (2026-10-02) — pierwszy golden pieniędzy: łańcuch 9vhKY

- `insta` (feature `json`) jako workspace dev-dep (`crates/api`). Testy `chain_economic_totals::tests::golden_9vhky_totals_computed_from_nodes` (totals `None` → liczone z węzłów) i `golden_9vhky_totals_refreshed_from_materialized` (start z `totals` fixture'a, ścieżka chain-history read). Wejście: `crates/api/tests/fixtures/chain_history_9vhKY.json` (22 węzły; wcześniej nieużywany). Snapshot: headline (baseline, end NAV + źródło, jakość, HODL, IL, fees, cashflow, tx fees, net USD / %) + per węzeł (baseline, end NAV, fees, tx, net); kwoty zaokrąglone do 6 miejsc. Snapshoty: `crates/api/src/services/snapshots/*.snap` (LF przez `.gitattributes`; `*.snap.new` w `.gitignore`).
- Sprawdzone: obie ścieżki dają identyczny wynik, zgodny z `totals` zapisanymi przez API (net 3,039781) i z `chain_cost_summary` (fees 0,339417, tx 0,053128); mutacja +0,000001 w net PnL → FAIL z diffem było/jest.
- **Znalezisko:** liczby są wewnętrznie niespójne (headline net 3,04 vs Σ węzłów 0,99; cashflow ~4 USD na dwóch węzłach bez zmiany NAV) → BUG-20261002-01 (`open`, do diagnozy). Golden zamraża obecne zachowanie; poprawka da deltę w B1.
- Aktualizacja snapshotu: `cargo insta review` albo `INSTA_UPDATE=always cargo test -p clmm-lp-api golden_9vhky` + `git diff` `.snap`; zawsze z sekcją „Golden delta” w PR.

### B2 (2026-10-02) — golden sald SESSION / CHAIN z lifecycle (9vhKY)

- Fixture `crates/data/tests/fixtures/lifecycle_9vhKY.jsonl`: 53 wiersze open/close/swap łańcucha 9vhKY wycięte z lokalnego `data/ledger/orca_position_lifecycle.jsonl`, tylko pola czytane przez agregatory (bez `rpc_url`, `fee_payer_pubkey`, notatek). Test `wallet_session::tests::golden_9vhky_session_and_chain_sums` (`clmm-lp-data`, dev-dep `insta`): saldo mint → raw dla każdej z 22 sesji (`aggregate_session_sums_from_lifecycle_rows`) i CHAIN (`aggregate_chain_sums_from_lifecycle_rows`, syntetyczne id — stare wiersze nie mają `chain_session_id`); capowanie debetów open (`cap_open_debits_against_running_balance`) w środku. Mutacja +1 raw w jednym close → FAIL.
- **Znalezisko:** CHAIN SOL zawyżony o ~0,0999 SOL (~8,3 USD) — swapy księgowane jednostronnie (BUG-20261002-01); golden zamraża obecne salda. Aktualizacja: `INSTA_UPDATE=always cargo test -p clmm-lp-data golden_9vhky` + `git diff`.

### B3 (2026-10-02) — golden sizingu reopen (F2.2 / BUG-20260512-03)

- `target_usd_for_reopen_sizing` / `target_usd_for_swap_mix_and_open` / `target_usd_for_close_reopen_preflight` / `target_usd_from_prev_end_clamped` / `final_caps_cover_deposit_quote` są `pub(crate)` (seam do testów, semantyka bez zmian). Test `rebalance::tests::golden_reopen_sizing_table`: 5 wierszy — dust 10→9,95; **must_not_follow_smaller_wallet** (prev_end 9,76 / wallet 4,06 → target 9,7112, legacy clamp 4,0397, half-leg `covers=false`); fallback prev_end=0; session cap (USDC 2,5M, pełny quote nie pokryty); clamp notional CHAIN. Mutacja +0,000001 w target → FAIL.

### C6 (2026-10-08) — testy liczb web + ESLint

- Vitest: `whirlpoolTicks`, `chainCapital`, `sessionCapital`, `openPositionSwapEstimates`, `lineageLedgerOpenQuote`, `chainEconomicQuality`. Config `web/.eslintrc.cjs`; `npm run lint` w jobie `web` / `verify-web`. Ignore generated `api.gen.ts` i handwritten `api.ts`/pages (C2). `alignPriceRatioToTicks`: `tickLower` → `const`.

### C5 (2026-10-08) — `BUGS.md` high/critical → istniejące testy

- `tools/bugs_test_guard.py`: wpisy `high`/`critical` muszą mieć w `Guards/tests` snake_case nazwę testu obecną w `crates/` / `web/src/` / `tools/` (`fn`, stem `tests/*.rs`, `mod *_tests`) albo `manual:`. Przynajmniej jedna nazwa musi istnieć. `cargo check` / `tsc` / `--lib` bez filtra = brak nazwy. Job `bugs_have_tests` w `quality_gates` (unit test skryptu + gate). Historyczne wpisy bez testu: `manual:` albo cytat prawdziwego `fn`.

### C2 (2026-10-08) — typy TS z OpenAPI (`api.gen.ts`)

- `openapi-typescript` 7.13, skrypt `web/scripts/openapi-ts.mjs` (write / `--check` z normalizacją LF). Zacommitowany `web/src/lib/api.gen.ts`. Job `web`: `npm run check:api-gen` przed `tsc`. Nowe endpointy: `web/src/lib/api.contract.ts` (`OkJson` / `Schema`); `api.ts` bez migracji hurtowej. Test `api.gen.test.ts` (HealthResponse + `/health` w snapshotach). Update: `cd web && npm run gen:api`.

### C4 (2026-10-08) — idempotencja GL (replay lifecycle)

- `session_and_chain_gl_lifecycle_row_replay_does_not_change_balances` w `crates/data/tests/session_gl_integration.rs`. Po pierwszym `Applied` snapshot sald + `COUNT(wallet_gl_posting)`; drugi apply tego samego wiersza = `SkippedAlready` i te same liczby (SESSION close, SESSION collect, CHAIN close). Istniejące testy sprawdzały tylko `SkippedAlready` / GL=PSLR na końcu.

### C3 (2026-10-08) — niezmienniki proptest (G5)

- Workspace `proptest = "1.7"`. Testy: `net_pnl_identity_after_refresh_lineage_totals` (api), `cap_open_debits_keeps_running_balance_non_negative` (data), `tick_price_roundtrip` (domain + protocols/orca), `stream_il_identities` (api pnl), `session_continuity_stitches_close_end_to_next_baseline` (api lineage). Granica domain `|t|=443636` z tolerancją 1 tick. Bez sieci, bez `data/`. Nie zmienia snapshotów insta.

### C1 (2026-10-08) — zacommitowany snapshot OpenAPI

- `crates/api/openapi.json` = pretty JSON z `ApiDoc::openapi()` (utoipa). Test `openapi::tests::openapi_matches_committed_snapshot` porównuje `serde_json::Value` (nie string), bez sieci. Update: `UPDATE_OPENAPI=1 cargo test -p clmm-lp-api --lib openapi_matches_committed` (albo `make openapi`). Zmiana pola = diff w PR + sekcja Golden delta (D1). Nie edytować JSON ręcznie.

### D1 (2026-10-07) — sekcja `Golden delta:` wymagana przy zmianie fixture / snap / OpenAPI

- `tools/golden_delta.py --require-pr-section`: gdy w diffie vs `BASE_REF` jest `*.snap`, `**/tests/fixtures/**`, `**/snapshots/**` albo `openapi.json`, body PR musi mieć nagłówek `Golden delta` + przynajmniej jedną linię uzasadnienia. Sama zmiana liczb nadal nie failuje (klasa `economic_regression`). Checklistowy bullet `- [x] **Golden delta:** if any snapshot…` nie zalicza się. `scripts/ci/golden-delta.sh` pobiera body (`gh` / `PR_BODY` / `PR_BODY_FILE`) i kończy job kodem z tej bramki. Workflow `quality_gates` nasłuchuje też `edited`, żeby dopisanie sekcji odświeżyło check. Testy: `D1SectionGateTests` w `tools/test_golden_delta.py` (bez gita/sieci).

### R3 (2026-10-02) — golden delta po ludzku (tabela było / jest / Δ)

- `tools/golden_delta.py`: liście numeryczne w JSON ciała `*.snap` (po nagłówku insta). `python tools/golden_delta.py --git-base origin/main` albo para `--old/--new`. Sort po |Δ|. Testy: `cd tools && python test_golden_delta.py` (bez gita/sieci). CI job `golden_delta` w `quality_gates.yml` → Job Summary + sticky komentarz PR. Nie failuje przy zmianie liczb (to decyzja człowieka; brak sekcji w PR = D1).

### B4 (2026-10-02) — golden backtest mini (`run_single` × strategie, ranking vs_hodl)

- Fixture `crates/cli/tests/fixtures/backtest_mini/`: 572 kroków 5m Orca SOL/USDC `Czfq3x…` (2026-04-01..02) wycięte z lokalnego `snapshots_5m.jsonl` do DTO (`steps.jsonl` + `fees_by_step.json` z `fee_growth`). Test `engine::golden_backtest::tests::golden_backtest_mini_run_single_ranking` (`clmm-lp-cli`, dev-dep `insta`): DTO → `StepData` → `run_single` dla Static / OorRecenter / Threshold 5% / Periodic 24h / IlLimit 5% / RetouchShift / Bollinger / LastCandle; ranking po vs_hodl (tie-break: fees, rebalance_count, nazwa). Snapshot: fees / IL / vs_hodl (6 dp) + `rebalance_count`. Bez RPC/DexScreener/`data/`. Periodic 24h wygrywa (~+89,73 USD vs HODL); wskaźniki 23 rebalance i na minusie.
- Aktualizacja: `INSTA_UPDATE=always cargo test -p clmm-lp-cli golden_backtest_mini` + `git diff` `.snap`; zawsze z sekcją „Golden delta” w PR.

### B6 (2026-10-02) — golden ledgera portfela łańcucha (start + eventy + stopka USD)

- Test `chain_portfolio::tests::golden_chain_portfolio_ledger_events_and_footer`: syntetyczny cykl SOL/USDC ($150 / $1), bez RPC/DB. Wejście: `WalletSessionOpenStartSnapshot` (pre-open 0,1 SOL + 5 USDC) + trzy wiersze lifecycle (`open` / `collect` / `close` + 3× tx fee 5000 lamports) → `ledger_start_event_from_open_start`, `ledger_event_from_lifecycle_row`, `tx_fee_ledger_event`, `aggregate_chain_collected_fees`, `chain_balance_usd_legs_from_balances` / `portfolio_balance_usd_from_balances`. Snapshot: start 20,00; open 11,50 out; collect 2,15 in; close 10,775 in; fees 2,225 (collect + LP na close); stopka 8,50. README liczb: `crates/api/tests/fixtures/chain_portfolio_ledger_b6/README.md`. Mutacja +0,000001 w `footer_usd` → FAIL.
- Aktualizacja: `INSTA_UPDATE=always cargo test -p clmm-lp-api golden_chain_portfolio_ledger` + `git diff` `.snap`; zawsze z sekcją „Golden delta” w PR.

### B5 (2026-10-02) — golden lineage multi-rotation (F2.3 / BUG-20260413-05)

- `lineage_shadow_diff_matches_golden_fixture` na `insta` (usunięty `lineage_shadow_expected.json`). Nowy `golden_lineage_multi_rotation_continuity`: 4 rotacje bota (sesja zszywa baseline; rotC bez NAV close → end z baseline rotD) + ręczny `manualE` (`open_origin=operator_api`, własne 10,00 — bez false parent) + fork (`rotD` = A–B–C–D, `sibX` = A–X). `manual_open_stitch_suppressed=true`, bot `false`.
