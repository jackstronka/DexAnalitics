# Testy — katalog i jak je włączyć

**To jest dział o testach.** Żywy spis: co mamy, co sprawdza, jak odpalić.  
**Nie** jest to plan budowy siatki — ten jest w [`IMPLEMENTATION_PLAN_REGRESSION_RESILIENCE.md`](IMPLEMENTATION_PLAN_REGRESSION_RESILIENCE.md).  
**Nie** jest to audyt z 2026-09-30 — ten (plus log PR) jest w [`TESTING_REGRESSION_PLAN.md`](TESTING_REGRESSION_PLAN.md).

**keywords:** tests, catalog, cargo-test, cargo-nextest, nextest, vitest, make-verify, golden, insta, session_gl_integration, hermetic, bugs-test-guard

**Utrzymanie (obowiązkowe):** przy nowym pliku testowym, goldenie/`insta`, harnessie `crates/*/tests/`, `#[ignore]`, pliku `web/src/lib/*.test.ts` albo nowej rodzinie testów w module — **w tym samym commicie/PR** zaktualizuj ten katalog (wiersz + komenda). Sam kolejny `#[test]` obok istniejących w opisanym już module nie wymaga wpisu. Reguła: [`.cursor/rules/test-integrity.mdc`](../.cursor/rules/test-integrity.mdc) pkt 8; checklista: [`AI_MERGE_CHECKLIST.md`](AI_MERGE_CHECKLIST.md).

---

## 1. Jedna komenda (to samo co CI)

| Środowisko | Komenda | Co robi |
| ---------- | ------- | ------- |
| Linux / macOS | `make verify` | `fmt --check` + clippy `-D warnings` + `cargo nextest run --workspace` + web `check:api-gen` + `tsc` + `vitest` + `eslint` |
| Windows | `.\tools\verify.ps1` | to samo; `-SkipWeb` / `-SkipRust` |
| Szybko (E1) | `make verify-fast` / `.\tools\verify-fast.ps1` | tylko crate’y z diffu vs `origin/main` (+ reverse deps); web tylko przy `web/` / `openapi.json` |
| Szybko, tylko Rust | `make test` albo `LOGLEVEL=WARN cargo nextest run --workspace` | bez fmt/clippy/web; wymaga `cargo-nextest` (**E2**) |
| Szybko, tylko web | `cd web && npm run check:api-gen && npx tsc --noEmit && npx vitest run && npm run lint` | C2 gen + typy + 90 testy `src/lib/*.test.ts` + ESLint (C6) |
| Hook przed pushem | `git config core.hooksPath .githooks` | pre-push odpala `verify` (na Windows `verify.ps1`) |

`make verify` **nie** odpala testów Postgres. Te są w jobie CI `db` albo ręcznie (§5).

**E2 — cargo-nextest:** zainstaluj raz `cargo install --locked cargo-nextest`. Konfig: [`.config/nextest.toml`](../.config/nextest.toml) (`retries=0`; profil `ci` w Actions). Pojedynczy filtr jak dawniej: `cargo nextest run -p clmm-lp-api golden_9vhky`. `cargo test` nadal działa do diagnostyki jednego testu / doctestów.

**R1 — obszary:** mapa [`ci/test-areas.toml`](../ci/test-areas.toml). Klasyfikacja: `cargo nextest list --workspace --lib --bins --tests --message-format oneline | python tools/test_areas.py --stdin --web-glob 'web/src/**/*.test.ts'`. Walidacja mapy + unit: `make test-areas`.

**R2 — raport:** po `make verify` / `.\tools\verify.ps1` tabela w `target/test-report/report.md` (JUnit nextest + vitest). Sam skrypt: `python tools/test_report.py --junit … --out-dir target/test-report`. CI: job `test_report` + Job Summary.

**R4 — trend:** `make test-trend` (fixture) albo `python tools/test_trend.py --fetch-github --branch main --days 14`. Workflow: `.github/workflows/weekly_test_trend.yml`.

**R5 — ochrona subsystemów:** mapa [`ci/subsystem-protection.toml`](../ci/subsystem-protection.toml) → kolumna w [`MASTER_IMPLEMENTATION_PLAN.md`](MASTER_IMPLEMENTATION_PLAN.md) §2.2. `make test-protection` (fixture) / `REPORT=target/test-report/report.json make test-protection`; zapis: `python tools/test_protection.py --report … --write-master`.

**D5 — cargo-mutants:** `make mutants-d5` (wymaga `cargo install --locked cargo-mutants`) albo Actions → **Mutants D5**. Summary: `target/mutants-d5/summary.md`. Unit parsera: `make mutants-d5-unit`.

Czas lokalnie: zwykle **2,5–6 min** (pełny verify).

---

## 2. Jak odpalić wycinek

Filtrowanie po **nazwie** (Cargo traktuje kolejne słowa jako argumenty testu — na Windows lepiej jeden token):

```bash
# jeden golden
cargo test -p clmm-lp-api --lib golden_chain_portfolio_ledger
cargo test -p clmm-lp-data golden_9vhky
cargo test -p clmm-lp-execution golden_reopen_sizing
cargo test -p clmm-lp-cli golden_backtest_mini

# jeden crate
cargo test -p clmm-lp-api --lib
cargo test -p clmm-lp-cli --bin clmm-lp-cli

# integracja (osobny harness)
cargo test -p clmm-lp-data --test session_gl_integration
cargo test -p clmm-lp-cli --test decode_fixture_tests
cargo test -p clmm-lp-cli --test snapshot_readiness_regression_test
```

Lista nazw bez uruchamiania:

```bash
cargo test --workspace -- --list
```

Web, jeden plik:

```bash
cd web && npx vitest run src/lib/experimentCapital.test.ts
```

---

## 3. Warstwy (co w ogóle jest)

| Warstwa | Gdzie | Kiedy pada |
| ------- | ----- | ---------- |
| Unit Rust (`#[test]` / `#[tokio::test]`) | `crates/*/src/**` w `mod tests` | zmiana logiki w tym module |
| Integracja Rust | `crates/*/tests/*.rs` | dekoder kont, readiness, **GL w Postgres** |
| Golden `insta` | snapshot `*.snap` + fixture | zmiana **liczby pieniężnej** albo rankingu — to `economic_regression` |
| Kontrakt OpenAPI (C1) | `crates/api/openapi.json` | zmiana pola / ścieżki API — diff w PR; update `UPDATE_OPENAPI=1` |
| Typy TS z OpenAPI (C2) | `web/src/lib/api.gen.ts` | job `web` `check:api-gen`; update `cd web && npm run gen:api` |
| Niezmienniki `proptest` (C3) | property tests w api/data/domain/protocols | złamany wzór ekonomii / tick / GL, nie rename pola |
| Web Vitest | `web/src/lib/*.test.ts` | zmiana czystej logiki TS (nie stron React) |
| `tsc --noEmit` | cały `web/` | rozjazd typów |
| Ignorowane (sieć / live) | `#[ignore]` | nie wchodzą w `verify`; tylko ręcznie `--ignored` |

Aktywne testy są **hermetyczne**: bez sieci (poza mockiem localhost), bez repo `data/`, bez `set_current_dir`. Mutacja env tylko przez `test_env::EnvGuard`.

---

## 4. Goldeny finansowe (B1–B6) — to chroni pieniądze

Odpalane w zwykłym `cargo test`. Diff `.snap` = zmiana ekonomii; update **tylko lokalnie** + sekcja **Golden delta** w PR. W CI (`CI=true`) `insta` nie nadpisuje snapshotów.

| ID | Test | Wejście | Co zamraża | Update |
| -- | ---- | ------- | ---------- | ------ |
| B1 | `chain_economic_totals::tests::golden_9vhky_*` (`clmm-lp-api`) | `crates/api/tests/fixtures/chain_history_9vhKY.json` | headline PnL / NAV / IL / fees / cashflow + net per węzeł | `INSTA_UPDATE=always cargo test -p clmm-lp-api golden_9vhky` |
| B2 | `wallet_session::tests::golden_9vhky_session_and_chain_sums` (`clmm-lp-data`) | `crates/data/tests/fixtures/lifecycle_9vhKY.jsonl` | saldo mint→raw SESSION i CHAIN | `INSTA_UPDATE=always cargo test -p clmm-lp-data golden_9vhky` |
| B3 | `rebalance::tests::golden_reopen_sizing_table` (`clmm-lp-execution`) | tabela w teście | `target_usd_*`, cap, `covers` | `INSTA_UPDATE=always cargo test -p clmm-lp-execution golden_reopen_sizing` |
| B4 | `engine::golden_backtest::tests::golden_backtest_mini_run_single_ranking` (`clmm-lp-cli`) | `crates/cli/tests/fixtures/backtest_mini/` | fees / IL / vs_hodl / rebalance / ranking 8 strategii | `INSTA_UPDATE=always cargo test -p clmm-lp-cli golden_backtest_mini` |
| B5 | `position_stream_lineage::tests::golden_lineage_multi_rotation_continuity` (+ shadow) | inline + `stream_lineage` | ciągłość rotacji, fork, ręczny open | `INSTA_UPDATE=always cargo test -p clmm-lp-api golden_lineage` |
| B6 | `chain_portfolio::tests::golden_chain_portfolio_ledger_events_and_footer` | syntetyczny cykl w teście | start / eventy / fees / stopka USD | `INSTA_UPDATE=always cargo test -p clmm-lp-api golden_chain_portfolio_ledger` |

Po `INSTA_UPDATE` zawsze: `git diff` na `*.snap` i opis „było / jest / dlaczego”. Nie edytuj `.snap` „żeby CI było zielone”.

**Tabela liczb (R3)** — liście numeryczne stary vs nowy `.snap`, posortowane po |Δ|:

**Kontrakt OpenAPI (C1)** — `crates/api/openapi.json` vs live `ApiDoc`:

```bash
cargo test -p clmm-lp-api --lib openapi_matches_committed
# update (Windows): $env:UPDATE_OPENAPI='1'; cargo test -p clmm-lp-api --lib openapi_matches_committed
# update (Unix):    UPDATE_OPENAPI=1 cargo test -p clmm-lp-api --lib openapi_matches_committed
make openapi
```

**Typy TS z OpenAPI (C2)** — `web/src/lib/api.gen.ts` vs committed `openapi.json`. Nowe endpointy w kliencie typować z `web/src/lib/api.contract.ts` (`OkJson` / `Schema`). Nie edytować `api.gen.ts` ręcznie. Po zmianie spec: `make openapi` i `make openapi-ts`.

**BUGS.md → testy (C5)** — każdy wpis `high`/`critical` musi w `Guards/tests` cytować istniejącą nazwę testu (backtick `fn` / `cargo test --test foo` / `mod foo_tests`) albo mieć `manual:`. `cargo check` i `tsc` to nie test.

```bash
python tools/bugs_test_guard.py
# Windows: py -3 tools/bugs_test_guard.py
cd tools && python test_bugs_test_guard.py
make bugs-test-guard
```

```bash
cd web && npm run check:api-gen
cd web && npm run gen:api
make openapi-ts
```

```bash
# vs origin/main (to samo co job CI golden_delta)
python tools/golden_delta.py --git-base origin/main
# Windows: py -3 tools/golden_delta.py --git-base origin/main
make golden-delta

# dwa pliki
python tools/golden_delta.py --old old.snap --new new.snap --fixture b1

# testy skryptu (hermetyczne)
cd tools && python test_golden_delta.py
```

CI (`quality_gates` / job `golden_delta`) wkleja tabelę do Job Summary i komentarza PR. Wklej ten sam markdown do sekcji **Golden delta** w opisie PR. Job **nie pada** przy zmianie liczb (to `economic_regression` dla człowieka). **D1:** gdy w diffie jest `*.snap`, `**/tests/fixtures/**`, `**/snapshots/**` albo `openapi.json`, brak sekcji `Golden delta:` (nagłówek + uzasadnienie) = czerwony job. Edycja opisu PR odpala job ponownie (`edited`). Testy bramki: `cd tools && python test_golden_delta.py`. **D2:** te same ścieżki (plus `.github/CODEOWNERS`) są w [`.github/CODEOWNERS`](../.github/CODEOWNERS) — GitHub prosi `@jackstronka` o review. Blokada merge wymaga „Require review from Code Owners” w branch protection (`main`); A10 ma 0 wymaganych review. **D3:** [`.cursor/BUGBOT.md`](../.cursor/BUGBOT.md) każe Bugbotowi komentować osłabianie testów (`assertion_weakened`: słabsza asercja, skasowany golden, nowy `#[ignore]`, wczesny `return`). **D4:** przy padzie CI agent idzie przez [`templates/CI_TRIAGE.md`](templates/CI_TRIAGE.md) — `economic_regression` = raport + STOP, bez update expected bez GO.

**Niezmienniki C3** (`proptest`, dane syntetyczne — nie golden):

```bash
cargo test -p clmm-lp-api --lib net_pnl_identity_after_refresh
cargo test -p clmm-lp-api --lib stream_il_identities
cargo test -p clmm-lp-api --lib session_continuity_stitches
cargo test -p clmm-lp-data cap_open_debits_keeps_running
cargo test -p clmm-lp-domain tick_price_roundtrip
cargo test -p clmm-lp-protocols tick_price_roundtrip
```

Znane zamrożone niespójności: **BUG-20261002-01** (cashflow swapów jednostronny) — B1 i B2 celowo trzymają obecne liczby do osobnego GO na fix. C3(1) tego nie łapie (tożsamość agregatora, nie wejście swapów).

---

## 5. Postgres (job `db`) — nie na żywej `clmm_lp`

Plik: `crates/data/tests/session_gl_integration.rs`.

| Test | Co robi |
| ---- | ------- |
| `session_gl_lifecycle_posting_matches_pslr_and_caps` | open/close → salda SESSION = PSLR, cap mintów |
| `session_gl_collect_row_accumulates` | collect dopisuje LP do SESSION |
| `chain_gl_lifecycle_posting_matches_pslr` | to samo dla CHAIN |
| `wallet_gl_opening_import_and_journal_postings` | import otwarcia + journal WALLET |
| `session_and_chain_gl_lifecycle_row_replay_does_not_change_balances` | **C4:** ten sam wiersz 2× → `SkippedAlready`, salda SESSION/CHAIN i liczba postingów bez zmian (close, collect, CHAIN close) |

```bash
# osobna baza — nigdy produkcyjna clmm_lp
set DATABASE_URL=postgres://clmm_user:clmm_password@localhost:5432/clmm_lp_test
set CLMM_REQUIRE_DB_TESTS=1
cargo test -p clmm-lp-data --test session_gl_integration
```

Bez `DATABASE_URL` te testy **cicho wracają** (nie fail). Z `CLMM_REQUIRE_DB_TESTS=1` brak bazy = **panic** (tak działa CI).

---

## 6. Spis po crate / obszarze

Liczby `#[test]` rosną; dokładny stan: `cargo test --workspace -- --list`. Poniżej **co jest w środku**, nie każda funkcja.

### `clmm-lp-api` (największy zestaw; `cargo test -p clmm-lp-api --lib`)

| Moduł / plik | Co robi |
| ------------ | ------- |
| `chain_economic_totals` | B1 golden; C3(1) `net_pnl` identity; D5: exact `close_nav` / CloseEstimate / `reconcile_stream_pnl_*` (healthy noop, 80% hodl, current_missing, IL rewrite) — `cargo test -p clmm-lp-api --lib reconcile_stream_pnl` |
| `rebalance` / `session_capital` (D5) | `target_usd_*` non-finite → 0; preflight wallet_cap; WSOL leg-B spend cap — `cargo test -p clmm-lp-execution --lib target_usd_reopen_sizing_rejects` / `apply_portfolio_caps_spendable` |
| `chain_portfolio` | B6 golden; start ledgera, fees, nogi USD, filtry phantom mint |
| `position_stream_lineage` | B5 golden + shadow; C3(5) close end = next baseline przy wspólnej sesji |
| `position_stream_pnl` / `position_chain_history` | PnL strumienia; C3(4) `clean_il` / `lp_vs_hodl` identities |
| `wallet_gl_posting` / `wallet_ledger*` | księgowanie GL z lifecycle |
| `openapi` | C1: `openapi_matches_committed_snapshot` — live `ApiDoc` (utoipa) = `crates/api/openapi.json`; update: `UPDATE_OPENAPI=1 cargo test -p clmm-lp-api --lib openapi_matches_committed` |
| `handlers::endpoint_coverage_tests` | czy endpointy odpowiadają (404/walidacja), bez pełnego DB-happy-path |
| `handlers::http_test_support` | API-H0: wspólne `hermetic_rpc` (`fallback_urls: []`) + `oneshot` helpers dla contract/coverage |
| `handlers::api_http_contract_tests` | HTTP→JSON→modele: health/metrics/lists; close-all 404 + preview/start (zero closable) + **F2.8** GET po disk reload (`close_all_http_get_survives_disk_reload`); jobs path = tempdir |
| `handlers::dip_http_tests` | API-H2: `/dip/snapshot|runtime|session|propose` + live `confirm_execute`/mode gates; vault/craft/history via httpmock (`CLMM_JUPITER_TRIGGER_BASE_URL`) |
| `handlers::agent_http_tests` | API-H2: worker settings/status; position agent start→chat; supervisor bez chain/DB → stabilny 4xx/5xx (`CLMM_AGENT_DATA_DIR`) |
| `handlers::wallets_http_tests` | API-H3: chain-portfolio no-DB envelope + B6 footer helpers; history/backfill bez DB; balances httpmock; effective warmup; api-signer unconfigured |
| `handlers::ops_http_tests` | API-H4: pending-open/stranded list+dismiss (tempdir ledgers); WS upgrade + injected `position.updated` payload shape; backtest JOBS/FULL seed GET + 4xx validation (no CLI) |
| `handlers::positions_detail_http_tests` | API-H5: stream-pnl no-DB degraded; lifecycle-summary missing/seeded ledger; diagnostics empty; invalid address → 400 |
| `position_close_all` | send_mode/scope unit + **F2.8** `close_all_job_survives_store_reload_from_tempdir`; store: `CLMM_CLOSE_ALL_JOBS_PATH` / `data/close_all_jobs.json` |
| *(plan)* API HTTP hardening | H0–H5 ✅ (H5.3 F2.9 = osobny C6): [`IMPLEMENTATION_PLAN_API_HTTP_TESTS.md`](IMPLEMENTATION_PLAN_API_HTTP_TESTS.md) |
| `handlers::backtests` | readiness snapshotów, warianty okien |
| `handlers::wallets` | merge sald, monotonic guard, fan-out RPC |
| `handlers::orca_tests` | proxy REST Orca (mock) |
| `handlers::phantom_auth_tests` | challenge / verify / replay nonce |
| `handlers::tx_tests` | budowa tx: wymagane pola, zły base64 |
| `auth` | JWT claims, role |
| `position_service` / close-all / valuation | serwisy pozycji (unit) |
| `handlers::devnet_e2e_tests` | **`#[ignore]`** — live devnet / Orca / funded wallet; `cargo test -p clmm-lp-api -- --ignored` |

### `clmm-lp-data`

| Moduł | Co robi |
| ----- | ------- |
| `wallet_session` | B2 golden; C3(2) `cap_open_debits` saldo mint ≥ 0 |
| `session_gl_integration` | Postgres, §5 |
| cache / timeseries / providers | cache, CSV, Orca REST (hermetyczne) |
| `providers::jupiter_trigger` | Trigger V2: role classify (buy_dip/TP/SL); open filters; mock history/verify; E1–E5 vault/craft/create/cancel/patch/DCA |
| `providers::jupiter_swap` | Swap V2: order+execute parse (`order_and_execute_parse_with_mock`) |
| `api handlers::dip` partial sign | BUG-20261111-01: host partial-signs multi-signer Trigger tx (`partial_sign_succeeds_when_try_new_needs_all_signers`) |
| `dip::zones` | klasyfikacja stref A/B/C + default `config/dip/zones.json` |
| `dip::propose` | envelope `kind=dip_propose`, walidacja, hermetic append JSONL |
| `dip::runtime` | parse `runtime.yaml` mode + ladder defaults (U2) |
| `dip::trigger_session` | per-wallet Trigger JWT store (E0 attach; no secrets in status) |

```bash
cargo test -p clmm-lp-data jupiter_trigger
cargo test -p clmm-lp-data jupiter_swap
cargo test -p clmm-lp-data vault_and_craft
cargo test -p clmm-lp-data dip::zones
cargo test -p clmm-lp-data dip::propose
cargo test -p clmm-lp-data dip::runtime
cargo test -p clmm-lp-data dip::trigger_session
```

### `clmm-lp-execution`

| Moduł | Co robi |
| ----- | ------- |
| `strategy::rebalance` | B3 golden sizingu reopen; **F2.2** `reopen_missing_quote_blocks_*` + final_caps; walidacja wyniku close/open |
| `strategy::session_capital` | flagi CHAIN, clamp capów, load mint caps |
| `strategy::decision` / `executor` / `pending_open` | silnik decyzji, pending open; **F2.1** `rebalance_needs_incomplete_recovery` + `upsert_preserves_rebalance_session_id` |
| `strategy::rebalance` (F2.1) | close-without-open → incomplete handoff units (`incomplete_recovery_*`) — `cargo test -p clmm-lp-execution incomplete_recovery` |
| `agent_decision` | kontrakt decyzji agenta |
| alerts / emergency / scheduler / tx | unit reguł i builderów |

### `clmm-lp-cli` (testy w **binarce**, nie w lib)

```bash
cargo test -p clmm-lp-cli --bin clmm-lp-cli
```

| Moduł / harness | Co robi |
| --------------- | ------- |
| `engine::tests` | `run_single`: HODL, periodic, threshold vs static, snapshot fees, Bollinger/LastCandle, Oor vs Retouch |
| `engine::golden_backtest` | B4 ranking |
| `tests/decode_fixture_tests` | Raydium/Meteora: inline base64 konta → parse |
| `tests/snapshot_readiness_regression_test` | `snapshot_readiness` na syntetycznym JSONL w tempdir (spawn binarki) |
| `local_swap_fees` | fee index z tempdir, nie z `data/` |
| `orca_wallet` / `orchestrator_*` | wycinki CLI |

### `clmm-lp-simulation`

Silnik pozycji i strategie: static / periodic / threshold / IL-limit, tracker (IL nie miesza się po rebalance), `hodl`, płaska vs ruchoma cena.

### `clmm-lp-optimization`

Cele (fees, PnL, Sharpe, IL), grid / analytical optimizer, parametry IL-limit / periodic / threshold / static / retouch.

### `clmm-lp-protocols`

Orca: tick↔price (C3(3) roundtrip `|t|≤443636`), deposit quote, wrap/unwrap, discriminators. RPC config (w testach `fallback_urls: Vec::new()`). Eventy Whirlpool / Raydium / Meteora (roundtrip). Aerodrome: stałe gauge.

### `clmm-lp-domain`

Tick/price (C3(3) roundtrip + granica 443636), IL, fee math, concentrated liquidity, constant product, price impact.

### Web (`cd web && npx vitest run`) — 19 plików / 90 testy

Tylko `web/src/lib/*.test.ts` (logika liczbowa). **Brak** testów stron/komponentów.

| Plik | Co robi |
| ---- | ------- |
| `api.gen.test.ts` | C2: `HealthResponse` + `DipSnapshotResponse` + `DipRuntimeResponse` + `/dip/runtime` |
| `botControlDisplay.test.ts` | U5 BC1–BC3: dry-run mismatch, liveArmed, fleet summary |
| `dipProposePayload.test.ts` | U4 AH4: ladder→proposed_orders, propose-only body, confirm/reject resolve |
| `agentDecisionsDisplay.test.ts` | U3 AH5: kind/action extract + kind filter + preview truncate |
| `dipSnapshotDisplay.test.ts` | U1 Dip UI: empty CTA (`no_jwt`/…), format USD/strefy |
| `dipLadder.test.ts` | U1b: drabinka 3C (scale=1/≠1), Required&gt;Available, min size |
| `dipRuntimeDisplay.test.ts` | U2: presence / live-armed / ladder summary line |
| `whirlpoolTicks.test.ts` | C6: tick↔price UI, spacing, expand in-range |
| `chainCapital.test.ts` / `sessionCapital.test.ts` | C6: quote body, inventory raw→UI, spend cap ≥ 0 |
| `openPositionSwapEstimates.test.ts` | C6: deficyt vs saldo, ExactIn +5%, cena puli |
| `lineageLedgerOpenQuote.test.ts` | C6: latest open-quote USD per PDA |
| `chainEconomicQuality.test.ts` | C6: end NAV fallback, banner estimated/degraded |
| `experimentCapital.test.ts` | alokacja kapitału eksperymentu |
| `experimentFundingPlan.test.ts` / `experimentBudgetPlan.test.ts` | plan finansowania / budżet |
| `experimentLaunch.test.ts` / `experimentLaunchSpecs.test.ts` | start eksperymentu, specy |
| `experimentArm.test.ts` / `experimentArmDirty.test.ts` | ramię eksperymentu, brudny stan |
| `solFirstFunding.test.ts` | funding SOL-first |
| `positionListDisplay.test.ts` | format listy pozycji |

Nowa liczba w UI → funkcja w `web/src/lib/*.ts` + test tutaj, nie w komponencie.

---

## 7. CI (GitHub Actions)

Wymagane na `main` (branch protection, A10 / G1):

| Check | Co odpala |
| ----- | --------- |
| `rust` | fmt/clippy + testy w namespace bez sieci (`unshare -rn`, `--offline`) |
| `web` | `check:api-gen` (C2) + `tsc` + `vitest` + `eslint` (C6) |
| `db` | Postgres 16 + `CLMM_REQUIRE_DB_TESTS=1` + `session_gl_integration` |
| `quality_gates` | agregat workflow Quality Gates: `critical_area_requires_tests`, `bugs_have_tests`, `golden_delta`, `test_areas`, `test_report_unit`, `test_trend_unit`, `test_protection_unit`, `mutants_d5_summary_unit`, `verify_fast_unit` |

Joby wewnątrz Quality Gates (nie osobno w protection — pokryte przez `quality_gates`):

| Job | Rola |
| --- | ---- |
| `critical_area_requires_tests` | zmiana pliku krytycznego wymaga linii testowych albo etykiety `no-tests-needed` |
| `golden_delta` | tabela było/jest/Δ + **D1** sekcja `Golden delta:` przy fixture/snap/`openapi.json` |
| `bugs_have_tests` | **C5:** `high`/`critical` w `BUGS.md` → istniejący test albo `manual:` |
| `test_protection_unit` (R5) | poziomy ochrony vs MASTER §2.2 |
| `mutants_d5_summary_unit` (D5) / `verify_fast_unit` (E1) | unit parserów tooling |

CODEOWNERS (D2): GitHub prosi `@jackstronka` o review przy golden/fixture/`openapi.json`; merge-block tylko po „Require review from Code Owners” (obecnie off — residual A10 / solo).

Docker / semver **nie** są wymagane do merge.

---

## 8. Czego nie ruszać / pułapki

- **Nie** odpalaj `session_gl_integration` na żywej bazie `clmm_lp` — testy **piszą** wiersze.
- `cargo test name1 name2` na Windows bywa parsowane jako extra args; jeden filtr.
- `--exact` bez pełnej ścieżki (`crate::mod::tests::fn`) często matchuje 0 testów.
- `RpcConfig { .. Default::default() }` w teście musi mieć `fallback_urls: Vec::new()` (default dokłada publiczne RPC).
- `#[ignore]` (devnet e2e, ręczny repair registry) **nie** jest częścią verify.
- Klasa pada: `stale_test` / `flake` / `economic_regression` / `assertion_weakened` / `infra` — reguła [`.cursor/rules/test-integrity.mdc`](../.cursor/rules/test-integrity.mdc).

---

## 9. Powiązane dokumenty

| Dokument | Rola |
| -------- | ---- |
| Ten plik | **katalog + jak włączyć** |
| [`IMPLEMENTATION_PLAN_REGRESSION_RESILIENCE.md`](IMPLEMENTATION_PLAN_REGRESSION_RESILIENCE.md) | plan faz A–E / R (co jeszcze zbudować) |
| [`TESTING_REGRESSION_PLAN.md`](TESTING_REGRESSION_PLAN.md) | audyt + log wykonania PR |
| [`templates/TEST_SECTION.md`](templates/TEST_SECTION.md) | obowiązkowa sekcja w nowym planie |
| [`templates/AGENT_TICKET.md`](templates/AGENT_TICKET.md) | E3: ticket dla agenta (ID, zakazy, verify, characterization) |
| [`templates/CI_TRIAGE.md`](templates/CI_TRIAGE.md) | D4: triage pada CI |
| [`AI_MERGE_CHECKLIST.md`](AI_MERGE_CHECKLIST.md) | checklista przed merge |
| [`BUGS.md`](BUGS.md) | `Guards/tests` przy bugach |
