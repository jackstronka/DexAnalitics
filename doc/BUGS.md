# BUGS Registry

Purpose: durable, searchable bug memory for AI/humans across sessions/models.

Rules for entries:
- Keep newest entries at the top.
- Use stable ids: `BUG-YYYYMMDD-XX`.
- Include `keywords:` for grep/AI recall.
- Record both: what was wrong and how it was fixed.
- Mark status: `open`, `fixed`, `regressed`, `wontfix`.

## Entry Template

### BUG-YYYYMMDD-XX — Short title

status: open|fixed|regressed|wontfix  
severity: low|medium|high|critical  
reported_by: user|ai|monitoring  
first_seen: YYYY-MM-DD  
fixed_in: <commit-sha-or-branch-or-empty>  
keywords: comma,separated,tokens,for,search

- **Symptom:** user-visible behavior.
- **Root cause:** technical reason.
- **Fix:** what changed.
- **Guards/tests:** checks to prevent recurrence.
- **Paths:** `file/a`, `file/b`

---

### BUG-20260429-03 — Swap: partial WSOL->SOL unwrap failed + mixed-language raw errors + low dark-mode contrast

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-29  
fixed_in: local  
keywords: swap, wsol, sol, unwrap, partial, i18n, error-message, dark-theme, contrast

- **Symptom:** In PL locale, `Swap -> Convert WSOL <-> SOL` displayed English raw backend error (`partial unwrap is not supported...`) and conversion failed for non-Max WSOL->SOL amounts. Error readability on dark theme remained poor in multiple pages (`text-destructive` on very dark backgrounds).
- **Root cause:** Backend unwrap path supported only full WSOL ATA close and explicitly rejected partial amounts. Frontend error path rendered raw API messages without normalization/localization. Dark theme destructive token was too dark for widespread `text-destructive` usage and many pages used weak `bg-destructive/5..10`.
- **Fix:** Added partial unwrap support in Orca executor (`close -> re-wrap remainder` flow), added API-side frontend message normalization for known SOL/WSOL failures in `messageFromErrorBody`, updated Swap UI copy (partial supported) and error banners, and increased dark-theme destructive token contrast globally.
- **Symptom (follow-up):** On Swap screen user saw only "Konwersja wysłana", and source WSOL temporarily appeared as `0` after partial unwrap despite not using Max, making final outcome unclear.
- **Symptom (follow-up 2):** `SOL -> WSOL` behaved like "set WSOL target balance" instead of converting the entered amount delta, which could make conversion appear incorrect versus user expectation.
- **Root cause (follow-up):** API convert response exposed only a single "submitted" signature/message while partial unwrap uses multi-step execution semantics; UI had no explicit final confirmation state and no delayed balance refresh for chained tx visibility.
- **Root cause (follow-up 2):** `native_to_wsol` path called `submit_wsol_wrap_with_signature_if_needed(req.amount_raw)` where argument means target WSOL ATA amount, not delta to wrap.
- **Fix (follow-up):** Convert API now reports confirmed outcome metadata (`confirmed`, `partial`, step signatures fields) and Swap UI renders final confirmed status + step signatures with short post-conversion balance refetch loop and partial-RPC warning when token read is degraded.
- **Fix (follow-up 2):** Added delta wrap path in Whirlpool executor (`submit_wsol_wrap_with_signature_delta`) and switched API `native_to_wsol` conversion to wrap exactly requested amount; response now includes post-conversion balances (`post_native_lamports`, `post_wsol_raw`) for deterministic UI confirmation.
- **Fix (follow-up 3):** Added operation ledger + reconciliation for `convert-sol` (`op_id`, `reconciliation_status`) with background verifier and status endpoints (`/wallets/ops`, `/wallets/ops/{op_id}`), so UI can distinguish `confirmed_unreconciled` vs `reconciled`/`mismatch` under RPC instability.
- **Fix (follow-up 4):** Added adaptive hedging + token-bucket budget for idempotent wallet reads, extended reconciliation diagnostics (`reason_code`, `attempts`, `last_verified_at_utc`), and new aggregate telemetry endpoint (`/wallets/ops/stats`) to monitor mismatch/unreconciled pressure.
- **Guards/tests:** `npx tsc --noEmit` (web), `cargo check -p clmm-lp-protocols -p clmm-lp-api`, `cargo test -p clmm-lp-protocols test_compute_unwrap_rewrap_amount -- --nocapture`.
- **Guards/tests (follow-up 3/4):** `cargo test -p clmm-lp-api wallets::tests:: -- --nocapture`, `cargo check -p clmm-lp-api`, `npx tsc --noEmit`.
- **Paths:** `crates/protocols/src/orca/executor.rs`, `crates/api/src/handlers/wallets.rs`, `crates/api/src/models.rs`, `crates/api/src/server.rs`, `crates/api/src/routes.rs`, `crates/api/src/state.rs`, `crates/api/src/openapi.rs`, `web/src/lib/api.ts`, `web/src/pages/Swap.tsx`, `web/src/pages/Wallet.tsx`, `web/src/index.css`

---

### BUG-20260429-02 — `snapshot-run-curated-all` omitted Meteora vault fields needed for auto `lp_share`

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-29  
fixed_in: local  
keywords: backtests, snapshot-run-curated-all, meteora, lp_share, vault_amount_a, vault_amount_b, token-2022, snapshot-only

- **Symptom:** Meteora rows in Backtests FULL often failed with `Meteora snapshot-only: set --lp-share ... include vault_amount_a/vault_amount_b`, even after fresh snapshot cycles.
- **Root cause:** Meteora branch inside `snapshot-run-curated-all` serialized `vault_amount_a/vault_amount_b` as optional fields and skipped them when decode returned `None`; decode path used strict SPL unpack only, so Token-2022/extended account layouts frequently produced missing vault amounts.
- **Fix:** In `snapshot-run-curated-all`, aligned Meteora vault decode behavior with curated collector: added token-account fallback decoder (extension-friendly), made `vault_amount_a/vault_amount_b` always present (`u64`) with explicit `vault_amount_source` (`rpc_token_account` or `missing_fallback_zero`).
- **Guards/tests:** `cargo check -p clmm-lp-cli` (targeted compile guard for CLI snapshot path).
- **Paths:** `crates/cli/src/main.rs`

---

### BUG-20260429-01 — Strategy Create forced `dry_run=true` with no visible toggle

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-04-29  
fixed_in: local  
keywords: strategies, create, dry_run, ui, strategy-create, auto-execute

- **Symptom:** Przy tworzeniu strategii użytkownik nie widział opcji `Dry run`; nowa strategia była zawsze zapisywana z `dry_run=true`, więc trzeba było wejść w edycję i ręcznie odznaczać.
- **Root cause:** `StrategyCreate` wysyłał payload z hardcoded `dry_run: true` i `auto_execute: false`, bez sekcji checkboxów znanej z `StrategyEdit`.
- **Fix:** Dodano kontrolki `Dry run` i `Auto-execute` do formularza tworzenia; payload korzysta teraz z wartości z UI zamiast stałych.
- **Guards/tests:** `npx tsc --noEmit` (web) + lint na dotkniętych plikach.
- **Paths:** `web/src/pages/StrategyCreate.tsx`

---

### BUG-20260427-05 — Mixed PL/EN labels and tiny helper text reduced UI readability

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-04-27  
fixed_in: local  
keywords: web, i18n, mixed-labels, readability, font-size, dashboard, settings, strategies, position-detail

- **Symptom:** UI still showed mixed Polish/English labels on multiple screens (`Dashboard`, `Closed positions`, `Strategies`, `Create Strategy`, `Settings`, `PositionDetail`) and many helper/diagnostic lines were too small to read comfortably.
- **Root cause:** Previous i18n migration covered selected flows only; several pages still had hardcoded literals. Additionally, the app used many `text-xs` / `text-[10px]` / `text-[11px]` utilities in dense diagnostic cards.
- **Fix:** Added bilingual labels on key screens and normalized remaining hardcoded strings in `PositionDetail`; added global readability uplift for smallest text utilities in `web/src/index.css`.
- **Guards/tests:** `npx tsc --noEmit` (web) + lint pass on touched frontend files.
- **Paths:** `web/src/index.css`, `web/src/pages/Dashboard.tsx`, `web/src/pages/ClosedPositions.tsx`, `web/src/pages/Strategies.tsx`, `web/src/pages/StrategyCreate.tsx`, `web/src/pages/Settings.tsx`, `web/src/pages/PositionDetail.tsx`, `doc/ENGINEERING_NOTES.md`

---

### BUG-20260427-04 — Wallet balances endpoint omitted Token-2022 accounts

status: partially fixed  
severity: medium  
reported_by: user  
first_seen: 2026-04-27  
fixed_in: local  
keywords: wallet, balances, token-2022, spl, getTokenAccountsByOwner, rpc, web

- **Symptom:** Po wejściu na `Wallet` część tokenów nie pojawiała się na liście on-chain mimo poprawnego owner pubkey i działającego RPC/SOL.
- **Root cause:** API `GET /wallets/balances` czytało token accounts tylko dla programu SPL Token legacy (`Tokenkeg...`), pomijając konta Token-2022 (`TokenzQd...`).
- **Fix:** Endpoint pobiera teraz token accounts z obu programów (`Tokenkeg...` + `TokenzQd...`), scala wynik per `mint` (sumowanie `ui_amount`) i zachowuje dotychczasowe fallbacki (SOL zwracany nawet przy częściowej niedostępności token RPC).
- **Symptom (follow-up):** Po wdrożeniu odczytu z obu programów UI nadal potrafi pokazać mniej tokenów niż oczekiwane; toggle `Pokaż zera` nie zmienia listy.
- **Root cause (follow-up):** Endpoint `wallets/balances` pracuje w trybie partial-success: gdy któryś call RPC (`legacy` albo `token-2022`) nie powiedzie się, API nadal zwraca `200` z tokenami tylko z działającej gałęzi. UI wcześniej nie pokazywał, że wynik jest częściowy.
- **Fix (follow-up):** API zwraca teraz status diagnostyczny obu odczytów (`token_legacy_ok`, `token_2022_ok`, `token_*_error`, `token_accounts_total`), a Wallet UI pokazuje ostrzeżenie „lista może być niepełna” z konkretnym statusem/błędem RPC.
- **Symptom (new):** Użytkownik widzi `http 403 Forbidden` z `solana.publicnode.com` (`blocked parameter: params.1.programId`) dla `getTokenAccountsByOwner` oraz sporadyczne błędy sieciowe; oba odczyty mogą być `legacy=false | token-2022=false`, mimo że saldo SOL działa.
- **Root cause (new):** Część darmowych RPC blokuje wywołania `getTokenAccountsByOwner` z filtrem `programId` (legacy/Token-2022). Obecna diagnostyka zwracała tylko ostatni błąd z listy endpointów, co zaciemniało który endpoint i dlaczego nie zadziałał.
- **Symptom (newer):** Mimo skonfigurowanych fallbacków API nadal często zwraca `tokens=[]` (`legacy=false`, `token-2022=false`) — oba odczyty kończą timeoutem.
- **Root cause (newer):** `wallets/balances` uruchamiało pojedynczy read per program-id z twardym timeoutem; retry/failover mogły nie zdążyć przejść sensownej liczby endpointów w oknie czasu.
- **Fix (newer):** Dodano fanout first-success-wins per program-id (`CLMM_WALLET_BALANCES_FANOUT`, domyślnie 3): API odpytuje kilka endpointów równolegle i bierze pierwszy sukces. W błędach zwracana jest telemetria prób endpointów.
- **Symptom (latest):** `Wallet` naprzemiennie pokazuje pełną listę tokenów i chwilowe `tokens=[]` (`confidence=degraded`) mimo aktywnego virtual-wallet/WS path; użytkownik obserwuje „flapping” co kilka odświeżeń.
- **Root cause (latest):** Odczyt baseline token accounts nadal był „edge-triggered” na bieżącej jakości public RPC. Przy chwilowym `403/timeout` oba programy mogły zwrócić fail i endpoint nie utrzymywał ostatniego poprawnego snapshotu tokenów; dodatkowo fanout próbował penalizowane endpointy ponownie bez krótkoterminowej pamięci błędów.
- **Fix (latest):** Dodano owner-scoped fallback `last-good token snapshot` w `effective-balances` (używany gdy oba odczyty token programs fail i bieżąca lista jest pusta) oraz mechanizm penalizacji endpointów token-account (`CLMM_WALLET_TOKEN_ENDPOINT_PENALTY_SECS`) dla błędów `403/429/timeout`, aby fanout czasowo omijał niestabilne RPC.
- **Symptom (latest-2):** `GET /wallets/effective-balances` w UI potrafiło timeoutować po 15s na zimnym starcie ownera (`cache miss`), mimo że endpoint miał być szybkim read-model path.
- **Root cause (latest-2):** Przy `cache miss` handler wykonywał synchroniczne `compute_effective_balances` (pełny on-chain read) zamiast natychmiastowego zwrotu i tła.
- **Fix (latest-2):** Wymuszono hard fast-return na `cache miss`: endpoint zwraca od razu placeholder `degraded` (`is_stale=true`) i uruchamia refresh wyłącznie w tle; stale cache zwracane z metadanymi (`is_stale`, `stale_age_ms`).
- **Guards/tests (latest):** testy jednostkowe dla penalizacji endpointów (`penalize_token_endpoint_marks_and_filters`, `penalize_token_endpoint_ignores_non_penalty_errors`) + `cargo check -p clmm-lp-api`.
- **Guards/tests (latest-2):** `cargo check -p clmm-lp-api`, `npm run build`, lint diagnostics dla zmienionych plików API/Web.
- **Guards/tests:** Dodano regresyjny unit test `merge_wallet_token_rows_sums_same_mint`.
- **Paths:** `crates/api/src/handlers/wallets.rs`, `crates/api/src/models.rs`, `web/src/lib/api.ts`, `web/src/pages/Wallet.tsx`, `.env.example`

---

### BUG-20260427-03 — Close Position returned opaque Whirlpool custom 6005

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-27  
fixed_in: local  
keywords: close-position, whirlpool, custom-6005, closepositionnotempty, not-empty, liquidity, fees, rewards

- **Symptom:** Manual close failed with `InstructionError(3, Custom(6005))` on Whirlpool ix (`whirLb...`) and API returned generic bad-request chain without clear remediation.
- **Symptom (session evidence):** Error payload included `custom_code=6005` on `/positions/{pda}` close attempt from Position Detail UI.
- **Root cause:** `classify_close_position_error` in API had dedicated branches for `3007` and `6018`, but no explicit mapping for Whirlpool `6005` (`ClosePositionNotEmpty`), so operators got opaque output.
- **Fix:** Added explicit close error mapping for `custom 6005` with actionable hint: position is not empty yet (remaining liquidity and/or unsettled fee/reward legs) and close should be retried after state refresh/settlement.
- **Guards/tests:** Added regression test `close_position_error_6005_maps_to_bad_request_with_not_empty_hint`.
- **Paths:** `crates/api/src/services/position_service.rs`

---

### BUG-20260427-01 — Position history `range @ close` was blank despite close ticks present

status: fixed  
severity: low  
reported_by: user  
first_seen: 2026-04-27  
fixed_in: local  
keywords: position-detail, range-close, old_tick_lower, old_tick_upper, lifecycle, web

- **Symptom:** W `PositionDetail -> Position history` kolumna `range @ close` pokazywała `—` mimo że event close miał ticki w details.
- **Root cause:** Frontend parser czytał wyłącznie `details.tick_lower/tick_upper`; close rows często zapisują zamykany zakres jako `old_tick_lower/old_tick_upper`, więc parser nie widział danych.
- **Fix:** Rozszerzono parser zakresów w `PositionDetail`: open czyta `tick_*` i fallback `new_tick_*`, close czyta `tick_*` i fallback `old_tick_*`.
- **Guards/tests:** `npx tsc --noEmit` w `web/`.
- **Paths:** `web/src/pages/PositionDetail.tsx`

---

### BUG-20260427-02 — Monitored positions table showed stale/zero PnL from monitor cache

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-27  
fixed_in: local  
keywords: positions, monitored-positions, pnl, fees, source-of-truth, stream-pnl, fallback-monitor

- **Symptom:** W `Positions -> Monitored positions (API)` kolumna `PnL` często pokazywała `0.000%` lub nieadekwatne wartości, mimo aktywnej pozycji i zmian na `Position Detail`. Użytkownik pytał też, czy `Fees` są z właściwego źródła.
- **Root cause:** Lista pozycji renderowała `PnL` z `position.pnl.net_pnl_pct` (monitor cache), które nie jest wiarygodnym live source dla tej tabeli. `Fees` były liczone poprawnie z valuation path (`fees_earned_usd`), ale UI nie pokazywał jawnie źródła (`live_valuation` vs `fallback_monitor`), więc zera wyglądały jak błąd.
- **Fix:** `Positions` pobiera teraz per-wiersz `stream-pnl` i używa `net_pnl_pct` z tego endpointu jako priorytetowego źródła. Gdy stream nie jest dostępny, zostaje fallback do monitor cache z etykietą źródła. Przy `Fees` dodano czytelny znacznik źródła valuation.
- **Guards/tests:** `npx tsc --noEmit` (web), ręczna weryfikacja: PnL na liście zgadza się kierunkiem/skalą z `Position Detail -> stream`.
- **Paths:** `web/src/pages/Positions.tsx`

---

### BUG-20260427-01 — Position history ranges used raw tick ratio (wrong scale for token pair)

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-27  
fixed_in: local  
keywords: position-history, rotations, range-open, range-close, tick-to-price, decimals, ui

- **Symptom:** W `Position history (rotations)` kolumny `range @ open` / `range @ close` pokazywały nielogiczne zakresy (np. `0.085... USDC per 1 SOL` zamiast ~`85...`), mimo poprawnych ticków.
- **Root cause:** UI konwertowało tick -> `tickToPriceRatio` (raw `B_raw/A_raw`) i wyświetlało wynik bez korekty o decymale tokenów (`10^(decA-decB)`), więc zakres był przeskalowany.
- **Fix:** W `PositionDetail` zakres ticków jest teraz liczony jako raw ratio -> UI ratio przez `uiPriceFromRawPriceRatio`, z decimalami z Orca token metadata (`getOrcaToken`) i fallbackiem dla znanych mintów/labeli (USDC/USDT/SOL/BTC/ETH).
- **Guards/tests:** `npx tsc --noEmit` (web), ręczna weryfikacja na `/positions/{pda}`: zakresy dla SOL/USDC są w skali dziesiątek/setek, nie setnych.
- **Paths:** `web/src/pages/PositionDetail.tsx`, `web/src/lib/whirlpoolTicks.ts`

---

### BUG-20260424-05 — Wallet page showed SOL immediately but SPL tokens appeared only after manual refresh

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-04-24  
fixed_in: local  
keywords: wallet, spl-tokens, rpc-timeout, refresh, retry, ux, web

- **Symptom:** Po wejściu na Wallet widać SOL, ale lista tokenów SPL bywa pusta; tokeny pojawiają się dopiero po ręcznym odświeżeniu strony.
- **Root cause:** API endpoint `/wallets/balances` zwraca sukces z pustą listą tokenów, gdy call `getTokenAccountsByOwner` chwilowo nie przejdzie (SOL jest zwracany niezależnie). UI traktował taki stan jako finalny i nie ponawiał automatycznie.
- **Fix:** Added bounded auto-retry in Wallet UI when owner is set, request succeeded, and token list is empty (up to 4 retries, delayed), plus explicit retry status message.
- **Guards/tests:** `npx tsc --noEmit` (web); manual verify: first empty token response triggers automatic retries without page refresh.
- **Paths:** `web/src/pages/Wallet.tsx`

---

### BUG-20260424-04 — Static manual lower/upper was treated as width proxy, not absolute bounds

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-24  
fixed_in: local  
keywords: backtests, static, manual-range, lower-upper, absolute-bounds, api, cli

- **Symptom:** User set `static_manual_lower=85` and `static_manual_upper=90` for SOL/USDC, but result rows still showed different static ranges (e.g. ~83-88, ~85-90) across windows.
- **Root cause:** API converted manual bounds into a derived `% width` and pinned `min/max-range-pct`, so engine still anchored ranges around each window entry price instead of using absolute bounds.
- **Fix:** Added dedicated CLI args `--static-manual-lower` / `--static-manual-upper`, passed through API only for valid single-pool manual runs, and wired backtest engine to apply these as absolute initial bounds for `StratConfig::Static` only.
- **Guards/tests:** `cargo check -p clmm-lp-cli -p clmm-lp-api`; rerun FULL backtest with one pool and manual static range should keep static bounds fixed to entered levels.
- **Paths:** `crates/api/src/handlers/backtests.rs`, `crates/cli/src/main.rs`, `crates/cli/src/backtest_engine.rs`

---

### BUG-20260424-03 — Auto-Tune loop marked completed FULL jobs as failed (`done` vs `succeeded/partial`)

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-24  
fixed_in: local  
keywords: auto-tune, backtests, full-run, status, succeeded, partial, done, api

- **Symptom:** `Status: running | note: Full optimize cycle failed` appeared even when FULL jobs were completing and returning results.
- **Root cause:** Auto-Tune polling logic expected terminal success status `"done"`, but `start_backtest_full` writes `"succeeded"` or `"partial"`. This caused success/partial cycles to be treated as failures.
- **Fix:** Updated Auto-Tune success branch to accept `"succeeded"` and `"partial"` as completed cycles; it now stores latest winner when results exist and sets note to either completed or completed (partial).
- **Guards/tests:** `cargo check -p clmm-lp-api`; manual verification via `/backtests/auto-tune/status` note progression after FULL cycle.
- **Paths:** `crates/api/src/handlers/backtests.rs`

---

### BUG-20260424-02 — Backtests FULL failed with `unexpected argument 'true'` for threshold OOR flag

status: partially fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-24  
fixed_in: local  
keywords: backtests, full-run, threshold, cli, clap, bool-flag, unexpected-argument

- **Symptom:** FULL runs ended as `partial` with CLI parse error: `unexpected argument 'true' found` for `--threshold-rebalance-on-range-exit-immediately`.
- **Symptom (follow-up):** Error persisted in runtime when API still resolved an older `clmm-lp-cli` binary that exposed switch-only syntax for this flag.
- **Root cause:** API passed the threshold OOR toggle as `--threshold-rebalance-on-range-exit-immediately true/false`, while older CLI variants parsed it as a pure boolean switch (set-true flag) and rejected explicit value tokens.
- **Fix:** 
  - CLI parser changed to `ArgAction::Set` for `threshold_rebalance_on_range_exit_immediately` (accepts explicit `true/false`).
  - API now probes `backtest-optimize --help` and auto-selects argument style:
    - value style (`... true/false`) when supported,
    - switch style (flag only for `true`) for older binaries.
  - For older binaries and explicit `false`, API skips the flag and logs warning (falls back to CLI default).
- **Guards/tests:** `cargo check -p clmm-lp-cli -p clmm-lp-api`; verify `backtest-optimize --help` and rerun FULL job with threshold toggle.
- **Paths:** `crates/cli/src/main.rs`, `crates/api/src/handlers/backtests.rs`

---

### BUG-20260424-01 — Backtests: static manual lower/upper looked unusable (inputs disabled by default)

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-04-24  
fixed_in: local  
keywords: backtests, static, manual-range, lower-upper, ux, disabled-input, multi-pool

- **Symptom:** Użytkownik nie mógł wpisać wartości do pól `static_manual_lower` / `static_manual_upper` na stronie Backtests.
- **Root cause:** Inputy były fizycznie zablokowane (`disabled`) dopóki nie była wybrana dokładnie 1 para, a domyślnie zaznaczonych jest wiele par.
- **Fix:** Inputy są teraz zawsze edytowalne; przy wielu parach wartości manual range pozostają wpisywalne, ale UI jasno komunikuje, że nie zostaną użyte i obowiązuje `static_deviation_pct`.
- **Guards/tests:** `npx tsc --noEmit` (web), weryfikacja UX: wpisanie `lower/upper` przy multi-pool nie blokuje edycji i pokazuje komunikat.
- **Paths:** `web/src/pages/Backtests.tsx`

---

### BUG-20260423-02 — Backtests FULL: `oor_recenter` vs `retouch_shift` looked like identical strategies

status: fixed  
severity: low  
reported_by: user  
first_seen: 2026-04-23  
fixed_in: local  
keywords: backtests, full-run, oor_recenter, retouch_shift, run_single, strategy-help, ui

- **Symptom:** W rankingu FULL `oor_recenter` i `retouch_shift` potrafily pokazywac te same (lub bardzo zblizone) metryki.
- **Root cause:** W `crates/cli/src/backtest_engine.rs` obie strategie sa **idle**, dopoki cena nie wyjdzie z pasma — przy calym oknie **in-range** zachowuja sie jak static (0 rebalansow). Przy **jednym** epizodzie OOR na plateau (stala cena poza starym pasmem) czesto wystarcza **jeden** rebalance dla kazdej, wiec koszty/score moga sie zrownac po zaokragleniu; pelna roznica ujawnia sie przy **monotonicznym** uciekaniu ceny po OOR: `OorRecenter` moze rebalance'owac na kolejnych krokach, `RetouchShift` rzadziej (bronia `retouch_armed` + geometria krawedzi).
- **Fix:** Rozszerzono opisy w `STRATEGY_HELP` (Backtests) o semantyke symulacji; dodano testy regresyjne `oor_recenter_matches_retouch_shift_when_price_never_leaves_initial_band` oraz `oor_recenter_rebalances_more_often_than_retouch_on_monotonic_climb_after_oor` w `crates/cli/src/engine/tests.rs`.
- **Guards/tests:** `cargo test -p clmm-lp-cli oor_recenter_`.
- **Paths:** `web/src/pages/Backtests.tsx`, `crates/cli/src/engine/tests.rs`

### BUG-20260423-01 — Backtests FULL: HTTP 422 when grid CSV sent floats into u64 JSON fields

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-04-23  
fixed_in: local  
keywords: backtests, full-run, 422, unprocessable, periodic_grid_steps, bollinger_window_grid, last_candle_seconds_grid, serde, web

- **Symptom:** Klikniecie `Uruchom FULL porownanie` na stronie Backtests zwracalo `HTTP 422` (body requestu odrzucone).
- **Root cause:** `BacktestFullRequest` w API uzywa `Vec<u64>` dla m.in. `periodic_grid_steps`, `bollinger_window_grid`, `last_candle_*_grid`; UI parsowal wszystkie siatki jako `number[]` i serializowal ulamki (np. `0.5` w periodic), co lamalo deserializacje Axum/Serde.
- **Fix:** W `Backtests.tsx` rozdzielono parser CSV: ulamki tylko dla `threshold_grid_pct` i `bollinger_k_grid`; calkowite dla pozostalych gridow (niecalkowite tokeny pomijane). Domyslne wartosci formularza dopasowano do oczekiwanego zestawu (periodic jako calkowite kroki).
- **Guards/tests:** `npx tsc --noEmit` w `web/`.
- **Paths:** `web/src/pages/Backtests.tsx`

### BUG-20260422-03 — Rebalance open/recovery could reuse dust sizing and open near-zero positions

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-22  
fixed_in: local  
keywords: rebalance, recover-open, dust, open-target-usd, close-amounts, lifecycle-ledger

- **Symptom:** Rotation chain could jump from normal values (e.g. ~$2.4) to `start/end ~$0.000` and then continue rebalancing on dust PDAs.
- **Symptom (session evidence):** In the same lineage/session, close rows carried non-zero value, but subsequent `bot_open_position` was sized as near-zero.
- **Symptom (follow-up):** In `Position history`, node could still show `start/open ~4 USD` while the same PDA `current_value` was near zero, because continuity rewrote `baseline_value_usd` from previous node end.
- **Root cause:** Open sizing in rebalance used pre-close calculated `amount_*_before_calc` (which can be stale/tiny in edge flows) instead of authoritative close amounts. Recovery path `recover_open_after_incomplete` hardcoded `amount_a_before_raw=1` and `amount_b_before_raw=1`, forcing `prev_end_value_usd` and `target_usd` toward dust.
- **Root cause (follow-up):** Session continuity in lineage (`close(old)->open(new)` by `rebalance_session_id`) always overwrote node baseline with `prev_end`, even when node baseline was already computed from open-row data (`open_amount_raw` / caps path).
- **Fix:** Standard rebalance open now passes `close_amount_a_raw`/`close_amount_b_raw` from `read_close_amounts_best_effort` into `open_new_range_with_wallet_mix`. Recovery open now loads latest matching close amounts from lifecycle close rows (`details.close_amount_*_raw`) by `position_pubkey` and optional `rebalance_session_id`, falling back to legacy `1,1` only when no row is found. In lineage continuity, baseline from session is now applied only when node baseline is missing (`0`), so explicit open-derived baseline is preserved.
- **Guards/tests:** Added unit test `close_amounts_from_lifecycle_row_parses_matching_close` and `continuity_from_session_does_not_override_existing_baseline`; verified with `cargo check -p clmm-lp-execution`, `cargo check -p clmm-lp-api`, and (2026-04-27 follow-up) `cargo test -p clmm-lp-api --no-run` after fixing stale `DecisionConfig.periodic_interval_hours` -> `periodic_interval_minutes` in `devnet_e2e_tests`.
- **Paths:** `crates/execution/src/strategy/rebalance.rs`, `crates/api/src/services/position_stream_lineage.rs`

---

### BUG-20260422-02 — Position lineage cashflow included open/close principal; rebalance preflight spammed zero collect rows

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-22  
fixed_in: local  
keywords: position-lineage, cashflow, net-pnl, fee_payer_token_deltas, open-close-principal, collect_fees, reopen-preflight

- **Symptom:** In `Position history (rotations)`, per-node `cashflow`/`net PnL` could look inflated/unintuitive (e.g. large positive cashflow despite small `start->end` NAV delta), because principal `open/close` token legs were mixed into cashflow.
- **Symptom:** `Logs / rebalances` showed repeated sessions with `bot_collect_fees` where `A raw: 0, B raw: 0`, followed by many `bot_reopen_widen_ticks`/`bot_reopen_preflight_failed` diagnostic rows.
- **Root cause:** Per-node DB lineage path aggregated `fee_payer_token_deltas` for cashflow without filtering lifecycle `open/close` events. Separately, rebalance flow executed `collect_fees_first` before reopen-feasibility guardrail; when preflight failed, close/open was skipped but zero-fee collect tx was already emitted.
- **Fix:** Lineage cashflow now excludes principal legs by filtering out lifecycle open/close rows in DB path (`non-principal` cashflow only). Rebalance flow no longer performs preflight-time `collect_fees_first`; collection is kept on close paths.
- **Guards/tests:** `cargo check -p clmm-lp-execution -p clmm-lp-api`; unit regression in decision engine to ensure strategy loop does not emit standalone `CollectFees`.
- **Paths:** `crates/api/src/services/position_stream_lineage.rs`, `crates/execution/src/strategy/rebalance.rs`, `crates/execution/src/strategy/decision.rs`

---

### BUG-20260422-01 — Rebalance session could emit two `bot_open_position` rows; second PDA stayed orphaned

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-22  
fixed_in: local  
keywords: rebalance, duplicate-open, rebalance-session-id, strategy-link, orphan-position, logs

- **Symptom:** A single `rebalance_session_id` could contain two `bot_open_position` rows (e.g. `7FW...` and `85A...`), while strategy link/heal followed only one path (`old -> new`), leaving the second position orphaned from strategy management.
- **Root cause:** Open/link flow assumes one successful open per session (`new_position: Option<Pubkey>` + one reopen hook call). There was no explicit guardrail to block duplicate open execution for an already-opened session id.
- **Fix:** Added open guard in rebalance executor: before open, block when lifecycle already has `bot_open_position` for session id, or when session is already inflight/completed in-process; emit diagnostic `bot_open_guard_blocked`. Added session helper in lifecycle ledger reader for `session_has_bot_open_position`.
- **Symptom (2026-04-27, follow-up):** Guard blocked duplicate attempts (`bot_open_guard_blocked`), but duplicate triggers still appeared in logs for the same session (`session_open_inflight_or_completed`), indicating concurrent workers were still attempting recovery/open.
- **Root cause (2026-04-27, follow-up):** Pending-open recovery processing had no cross-executor claim/lease for the same session item, so parallel executor loops could race on one `rebalance_session_id` and rely on late open guard rejection. Additionally, strategy start path could replace executor instances without explicitly stopping/removing any pre-existing one first.
- **Fix (follow-up):** Added global pending-open claim key (`sid:<rebalance_session_id>`; fallback `pool+closed_position`) so only one worker processes a recovery item at a time; non-claiming workers keep item untouched (no extra attempts). Added defensive replacement guard in `start_strategy_executor_core` to stop+remove any existing executor instance before spawning a fresh one for the same strategy id.
- **Fix (observability/UI):** Added tick-range context to close rows (`old_tick_*`, `planned_new_tick_*`) and previous-range context to open rows (`prev_tick_*`, `new_tick_*` in details), then rendered side-by-side graphical range panels in Logs session view.
- **Guards/tests:** `cargo check -p clmm-lp-execution`; `npx tsc --noEmit` (in `web/`).
- **Paths:** `crates/protocols/src/ledger/tx_lifecycle.rs`, `crates/execution/src/strategy/rebalance.rs`, `crates/execution/src/strategy/executor.rs`, `crates/api/src/handlers/strategies.rs`, `web/src/pages/Logs.tsx`

---

### BUG-20260421-04 — Backtests TOP3 per family showed duplicate strategy labels with different ranges

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-04-21  
fixed_in: local  
keywords: backtests, top3, strategy-family, duplicates, range, bollinger, ui

- **Symptom:** In `Strategie spelniajace target`, a family section (e.g. `bollinger`) could show the same strategy label multiple times (e.g. `bollinger_w10_k1_r24`) with slightly different ranges.
- **Root cause:** Family-level TOP3 selection sorted all variants and sliced first 3 rows, but did not deduplicate by `strategy` label before slicing.
- **Fix:** Added deduplication in `qualifyingTop3`: after sorting family rows by selected metric, keep only the best row per `strategy` label, then take TOP3.
- **Guards/tests:** `npx tsc --noEmit` in `web/`.
- **Paths:** `web/src/pages/Backtests.tsx`

---

### BUG-20260421-03 — Backtests FULL failed on stale CLI missing optimize-grid flags

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-21  
fixed_in: local  
keywords: backtests, full-run, backtest-optimize, threshold-grid-pct, stale-cli, api, feature-probe

- **Symptom:** Web Backtests FULL run failed for all pools/windows with `exit Some(2): unexpected argument '--threshold-grid-pct' found`.
- **Root cause:** API always passed optimize-grid override flags (`--threshold-grid-pct`, etc.) whenever request fields were present, but capability probing guarded only `--include-strategy-families`. When API resolved an older `clmm-lp-cli` binary, Clap rejected unknown grid flags.
- **Fix:** Added `backtest-optimize --help` probe for requested grid flags before matrix execution; API now fails fast with explicit rebuild/`CLMM_LP_CLI_PATH` guidance when any requested grid flag is unsupported.
- **Guards/tests:** `cargo check -p clmm-lp-api`.
- **Paths:** `crates/api/src/handlers/backtests.rs`

---

### BUG-20260421-02 — Position Agent tab looked inactive (quick actions were non-clickable, fallback was opaque)

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-21  
fixed_in: local  
keywords: position-agent, quick-actions, ui, llm-fallback, chat, position-detail

- **Symptom:** In `PositionDetail -> Position Agent`, quick actions (`scan_now`, `compare_7d_ranges`, etc.) were visible but did not do anything, so tab looked “dead”.
- **Symptom:** Sending prompts often returned generic fallback text with no clear indication that LLM provider was disabled/fallback mode.
- **Root cause:** UI rendered `quick_actions` as passive labels (`span`) without click handlers. Chat send path used `/agent/message` (message-only response), which dropped provider metadata (`used_fallback`), so users could not tell if real LLM was used.
- **Fix:** Quick actions are now clickable buttons wired to real actions (`scan_now` -> scan endpoint; comparison/cross-pair actions -> send prefilled prompt). Chat send in `Position Agent` now uses `/agent/llm-reply` and surfaces reply source (`fallback/provider:model`) in UI info message.
- **Guards/tests:** `npx tsc --noEmit` in `web/`.
- **Paths:** `web/src/pages/PositionDetail.tsx`, `web/src/lib/api.ts`

---

### BUG-20260421-01 — Backtests FULL: duplicate strategy labels hid different ranges; global metrics were unclear

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-04-21  
fixed_in: local  
keywords: backtests, full-run, optimize, duplicates, strategy-range, meteora, lp_share, tooltips, ranking

- **Symptom:** TOP list for a pool/window could show the same strategy label multiple times (e.g. `bollinger_w20_k2.5_r48` twice) with slightly different metrics and no visible reason.
- **Symptom:** Global ranking column `Wystapienia` looked suspicious (e.g. `320`) and users interpreted it as wins instead of count of tested variants.
- **Symptom:** Meteora rows frequently showed very small fees (e.g. `0.03`) because API silently forced `--lp-share 0.0001` when request had no `lp_share`.
- **Root cause:** API returned only strategy label + metrics parsed from CLI table and dropped per-row range context (`Lower($)`, `Upper($)`), while UI TOP3 selected first 3 rows after sort (not unique strategy labels). Global table lacked metric semantics hints. Meteora fallback share was hardcoded in API handler.
- **Fix:** Added range context to API metric rows (`lower_usd`, `upper_usd`, `width_pct`), made TOP3 unique by strategy label (best variant per label for selected sort), displayed range/width in per-window table, added column/tooltips clarifying semantics (including `Wystapienia`), and removed forced Meteora fallback `lp_share=0.0001` in API.
- **Guards/tests:** `npx tsc --noEmit` in `web/`; `cargo check -p clmm-lp-api`.
- **Paths:** `crates/api/src/models.rs`, `crates/api/src/handlers/backtests.rs`, `web/src/lib/api.ts`, `web/src/pages/Backtests.tsx`

---

### BUG-20260420-04 — `backtest-optimize` skipped documented strategies (`oor_recenter`, `il_limit`, `retouch_shift`)

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-04-20  
fixed_in: local  
keywords: backtest-optimize, strategies, StratConfig, oor_recenter, il_limit, retouch_shift, docs-drift

- **Symptom:** `backtest-optimize` results did not include all strategies described in docs, and rankings were biased toward `static` / indicator variants because `oor_recenter`, `il_limit`, `retouch_shift` were missing from the grid.
- **Root cause:** `StratConfig` + `default_strategies()` in CLI omitted those strategy variants, while documentation still described them as part of optimize strategy catalog.
- **Fix:** Added `OorRecenter`, `IlLimit`, `RetouchShift` to `StratConfig`, wired trigger/range logic in `run_single`, added parser support (`parse_strategy_label`), and restored those variants in `default_strategies()` (using `--il-max-pct`, `--il-close-pct`, `--il-grace-steps`).
- **Guards/tests:** Added parser regression assertions for new labels and a grid regression test ensuring default optimize set includes the missing strategies; verified with `cargo test -p clmm-lp-cli parse_strategy_label_bollinger_and_last_candle` and `cargo test -p clmm-lp-cli default_grid_includes_documented_non_indicator_strategies`.
- **Paths:** `crates/cli/src/backtest_engine.rs`, `crates/cli/src/commands/backtest_optimize.rs`, `crates/cli/src/output/optimize_result_json.rs`, `crates/cli/src/engine/tests.rs`, `doc/BACKTEST_OPTIMIZE_STRATEGIES.md`

---

### BUG-20260420-03 — Stream PnL mixed chain-anchored valuation with component-wide session costs

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-20  
fixed_in: local  
keywords: stream-pnl, stream-lineage, bfs-component, rebalance-session-id, tx-fees, realized-cashflow, forked-edges

- **Symptom:** Stream totals could show internally inconsistent scope: `baseline/current` anchored to the selected lineage chain (`start -> ... -> end`), while `tx_fees_usd` and `realized_cashflow_usd` could include rows from sibling branches in the same BFS component.
- **Symptom (example):** In forked graph shape (`A->B->C` plus `A->X`), querying chain ending at `C` could still include `A->X` session fees/cashflow in totals.
- **Root cause:** `compute_position_stream_pnl_for_stream_members` used `sessions` derived from full component connectivity (`compute_position_stream_performance`) for cost/cashflow aggregation, but valuation anchors were lineage-specific (first/last chain PDA).
- **Fix:** Added chain-local session scoping in `position_stream_pnl`: derive `chain_sessions` from DB edges matching adjacent ordered chain pairs only; aggregate tx fees and token-delta cashflow by those sessions. If chain sessions are unavailable, fallback to chain positions (not component-wide sessions).
- **Guards/tests:** Added unit tests `chain_sessions_ignore_fork_edges_outside_ordered_chain` and `chain_sessions_empty_for_single_node_chain`; verified with `cargo test -p clmm-lp-api chain_sessions_`.
- **Paths:** `crates/api/src/services/position_stream_pnl.rs`

---

### BUG-20260420-02 — `experiment-config` failed to derive USD capital when pool only existed in lifecycle rows

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-04-20  
fixed_in: local  
keywords: experiment-config, derived_initial_capital_usd, pool_address, pool_pubkey, open-session, positions-handler

- **Symptom:** `Detected config (open snapshot)` showed `Could not resolve pool mints to convert open-session deltas to USD; derived_initial_capital_usd unavailable.` even when lifecycle/session data existed.
- **Root cause:** `get_position_experiment_config` resolved pool mints only from `registry_open.details.pool_address`; older/incomplete rows often had pool only in lifecycle ledger lines (`pool_address`/`pool_pubkey`) for the same `rebalance_session_id`.
- **Fix:** Added fallback pool resolution for the open session by scanning lifecycle rows for the same session id and taking `pool_pubkey` or `pool_address` when details are missing.
- **Guards/tests:** `cargo build -p clmm-lp-api`.
- **Paths:** `crates/api/src/handlers/positions.rs`

---

### BUG-20260420-01 — Position Detail totals lost/zeroed chain-level LP collected summary

status: partially fixed  
severity: medium  
reported_by: user  
first_seen: 2026-04-20  
fixed_in: local  
keywords: position-detail, stream-lineage, chain_cost_summary, fees_collected_usd_total, ui-regression, lifecycle-ingest, pool_address, pool_pubkey

- **Symptom:** On `PositionDetail` (`Logs / rebalances`), the totals card still showed baseline/current/tx fees/cashflow/net PnL, but chain-level **LP collected** summary disappeared after the economic-vs-IL layout update.
- **Symptom (follow-up):** After restoring the field, some closed chains still showed zeros for tx/LP collected although lifecycle JSONL rows existed (`bot_collect_fees`, non-zero `tx_fee_lamports`).
- **Symptom (follow-up 2):** In `Closed position` view, chain LP fee leg rows showed only token units/base units (e.g. `SOL`, `whETH`) without per-leg USD approximation, making it hard to reconcile with summed USD.
- **Root cause:** (1) UI refactor replaced the old totals block and omitted the `chain_cost_summary.fees_collected_usd_total` + `collect_events_total` rendering branch. (2) DB ingest/aggregation path was brittle for lifecycle rows using `pool_address` (not `pool_pubkey`) and DB fallback triggered only when **all** node values were empty, so partial DB rows could still zero tx/collect aggregates.
- **Fix:** Restored `LP collected (sum)` in UI totals; lifecycle ingest now accepts `pool_address` as fallback for `pool_pubkey`; DB node metrics now bridge tx/collect aggregates from lifecycle JSONL when DB returns zeros for those fields.
- **Fix (follow-up 2):** Added per-leg USD approximation in chain fee leg rows (`≈ $...`) using current mint prices from Jupiter for displayed token mints; keeps `—` when mint price is unavailable.
- **Guards/tests:** `npx tsc --noEmit` in `web/`; `cargo test -p clmm-lp-api position_stream_lineage`.
- **Paths:** `web/src/pages/PositionDetail.tsx`, `crates/api/src/services/position_stream_performance.rs`, `crates/api/src/services/position_stream_lineage.rs`

---

### BUG-20260419-01 — Stream PnL IL/HODL silently wrong: valuation snapshot queries omitted mint columns

status: fixed  
severity: high  
reported_by: ai  
first_seen: 2026-04-19  
fixed_in: local  
keywords: stream-pnl, position_stream_pnl, hodl, il_usd, valuation_snapshots, token_mint_a, sql

- **Symptom:** With rows in `position_stream_valuation_snapshots` that already stored `token_mint_a` / `token_mint_b`, `/positions/.../stream-lineage` totals still behaved like “IL unavailable” semantics: `hodl_value_usd` fell back to `baseline_value_usd`, so `il_usd` became `current_value − baseline_value` instead of baseline basket × current mint prices − LP mark.
- **Root cause:** `compute_position_stream_pnl_for_stream_members` selected only `ts_utc, value_usd, amount_a_ui, amount_b_ui, pool_pubkey` but later read `token_mint_*` from the row — columns were never fetched, so mint paths were always empty.
- **Fix:** Extended baseline and latest snapshot queries to include `token_mint_a`, `token_mint_b`; resolve pool mints with baseline-first + per-leg fallback to the latest snapshot; annotate `ts_utc` row decode for inference.
- **Guards/tests:** Unit tests `pool_mints_prefers_*`, `pool_mints_falls_back_*`, `pool_mints_mixed_fallback_per_leg` in `services::position_stream_pnl`.
- **Paths:** `crates/api/src/services/position_stream_pnl.rs`

---

### BUG-20260417-02 — Backtests FULL run: `unexpected argument '--include-strategy-families'` (stale CLI)

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-04-17  
fixed_in: local  
keywords: backtests, backtest-optimize, include-strategy-families, clmm-lp-cli, api, resolve_clmm_lp_cli_path

- **Symptom:** Web **Backtests** → FULL comparison failed with `exit Some(2)` and stderr: `unexpected argument '--include-strategy-families' found` (CLI suggested `--indicator-strategies`).
- **Root cause:** API spawned an **older** `clmm-lp-cli` binary (commonly `target/debug/clmm-lp-cli.exe` next to `cargo run` API) that predates `--include-strategy-families`, while the API always passed that flag for strategy-family filtering.
- **Fix:** `resolve_clmm_lp_cli_path` now prefers **`target/release` before `target/debug`** under `CLMM_REPO_ROOT` (and same order for `CLMM_API_TARGET_DIR` / `CARGO_TARGET_DIR`) and only falls back to “next to API exe” after those candidates. Full-matrix handler probes `backtest-optimize --help`; omits the flag for **full-catalog** runs on legacy CLI, and **fails fast** with a clear rebuild message when a **subset** of strategies is selected but the CLI cannot filter families.
- **Fix (follow-up):** Probe result cache key includes the CLI binary **mtime**, so rebuilding `clmm-lp-cli` invalidates stale “unsupported” results without restarting `clmm-lp-api`.
- **Paths:** `crates/api/src/handlers/backtests.rs`

---

### BUG-20260417-01 — `data_alerts_loop.ps1` fails on empty `$PSScriptRoot`

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-04-17  
fixed_in: local  
keywords: windows, powershell, data_alerts_loop, PSScriptRoot, RepoRoot, Join-Path

- **Symptom:** Running `powershell -File .\tools\data_alerts_loop.ps1` failed immediately with `Join-Path : Cannot bind argument to parameter 'Path' because it is an empty string.` at param default for `RepoRoot`.
- **Root cause:** `RepoRoot` default evaluated `Join-Path $PSScriptRoot ".."` during parameter binding; in some hosts/sessions `$PSScriptRoot` was empty, so binding failed before script logic started.
- **Fix:** Changed `RepoRoot` default to empty and resolved it at runtime with fallbacks: `$PSScriptRoot` -> `Split-Path $MyInvocation.MyCommand.Path` -> `Get-Location`.
- **Symptom (follow-up):** The same path-empty failure appeared in `tools/snapshot_health_alert.ps1` (and could affect scheduled-task registration helper), because they used the same default-parameter pattern.
- **Fix (follow-up):** Applied the same runtime `RepoRoot` resolution hardening in `tools/snapshot_health_alert.ps1` and `tools/register_snapshot_health_scheduled_task.ps1`.
- **Guards/tests:** Re-ran `tools/data_alerts_loop.ps1` with explicit minimal intervals and `-SkipSlack` to verify startup no longer fails at parameter binding.
- **Guards/tests (follow-up):** Re-ran `powershell -File .\tools\snapshot_health_alert.ps1 -SkipSlack` and confirmed it reaches health-check execution instead of failing in parameter binding.
- **Paths:** `tools/data_alerts_loop.ps1`, `tools/snapshot_health_alert.ps1`, `tools/register_snapshot_health_scheduled_task.ps1`

---

### BUG-20260416-02 — Snapshot loops stopped collecting when release CLI binary was missing

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-16  
fixed_in: local  
keywords: snapshots, collector-loop, run-snapshot-loop, run-snapshot-loop-5m, clmm-lp-cli, release-binary, cargo-fallback

- **Symptom:** Snapshot freshness checks showed `rows_in_window~=0` for both `snapshots.jsonl` (10m) and `snapshots_5m.jsonl` (5m), while loop logs repeatedly reported `target\\release\\clmm-lp-cli.exe` not found.
- **Symptom (follow-up):** In service-like loop context, `cargo` was not in PATH, so the first fallback attempt also failed with `The term 'cargo' is not recognized...`.
- **Root cause:** Windows loop scripts hard-required `target/<configuration>/clmm-lp-cli.exe` and only logged errors when absent; no runtime fallback existed, so periodic collection silently stopped.
- **Fix:** Added runtime fallback in both loop scripts: when release binary is missing, execute collector via `cargo run -q -p clmm-lp-cli --bin clmm-lp-cli -- snapshot-run-curated-all` (and `--snapshots-suffix 5m` for 5m loop). Logs now explicitly record fallback mode on startup.
- **Fix (follow-up):** Added cargo path resolver (`CARGO_HOME`, `%USERPROFILE%\\.cargo\\bin\\cargo.exe`, then PATH lookup) so fallback works under service contexts where PATH is minimal.
- **Guards/tests:** Re-ran collector health scripts to confirm root cause and verify that loops have a valid execution path even without release artifact.
- **Guards/tests (follow-up):** Loop heartbeats (`data/snapshot_logs/snapshot-loop-heartbeat-{10m,5m}.json`) + `snapshot_health_check.ps1` stale-heartbeat issues; Windows: `tools/register_snapshot_health_scheduled_task.ps1` lub `tools/data_alerts_loop.ps1` pod Shawl/NSSM — automatyczne `snapshot_health_alert` bez ręcznego sprawdzania.
- **Paths:** `scripts/windows/run-snapshot-loop.ps1`, `scripts/windows/run-snapshot-loop-5m.ps1`, `tools/snapshot_health_check.ps1`, `tools/register_snapshot_health_scheduled_task.ps1`

---

### BUG-20260415-01 — Position Detail vs Positions list: inconsistent strategy-link badge

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-04-15  
fixed_in: local  
keywords: positions-ui, position-detail, monitored-positions, linked-strategy, react-query, stale-cache

- **Symptom:** Dla tej samej pozycji (`/positions/<PDA>`) w `Position Info` strategia była widoczna jako podłączona, ale na stronie `Positions` w tabeli `Monitored positions (API)` pojawiało się `Not linked`.
- **Root cause:** Widok `Positions` opierał się na mapowaniu `strategies -> position_addresses` bez wymuszonego refetch na mount oraz bez normalizacji klucza po stronie pozycji z monitora. Przy cache / odświeżeniach między podstronami mogło to dawać rozjazd statusu.
- **Fix:** W obu widokach (`PositionDetail`, `Positions`) zapytanie `['strategies']` wymusza odświeżenie na mount. W `Positions` dodano normalizację adresu (`trim`) po obu stronach mapowania (`position_addresses` i `position.address`) przed lookupem.
- **Fix (follow-up):** Dla `['strategies']` w tych widokach ustawiono też `staleTime: 0`, `refetchOnWindowFocus: true` i krótki `refetchInterval` (15s), aby zbić ryzyko utrzymywania starego link-statusu przy długiej sesji SPA i globalnym cache `staleTime=5m`.
- **Fix (follow-up 2):** `PositionDetail` filtruje listę linked strategii przez `diagnostics.linked_strategies` (backend source-of-truth), więc sekcja `Position Info` nie pokazuje linku, którego backend już nie widzi.
- **Fix (follow-up 3):** `Positions` (`Monitored positions (API)`) również opiera badge `Strategy` o `position-diagnostics` per-wiersz (a nie tylko `GET /strategies`), więc status `linked/not linked` jest zgodny z backend source-of-truth także na liście.
- **Symptom (2026-04-27, regression):** Dla pozycji z linked strategią sekcja `PositionDetail -> Diagnostics` pokazywała strategię poprawnie, ale `Positions -> Monitored positions (API)` wracało do `Not linked`.
- **Root cause (2026-04-27, regression):** W `Positions` status był nadal liczony jako przecięcie dwóch źródeł (`diagnostics.linked_strategies` AND `GET /strategies` mapowane po `position_addresses`). Gdy diagnostics miało link, a cache/config `/strategies` chwilowo nie zawierało tego PDA, UI pokazywał false-negative `Not linked`.
- **Fix (follow-up 4):** `Positions` renderuje status linku bezpośrednio z `position-diagnostics` (source-of-truth); `GET /strategies` służy tylko do wzbogacenia opisu parametrami, z fallbackiem do danych z diagnostics gdy szczegóły strategii nie są dostępne.
- **Symptom (2026-04-29, regression):** Po pending-open recovery nowy aktywny PDA potrafił pozostać `Not linked`, mimo że strategia była running; jednocześnie stary close/open session mógł dalej wisieć jako pending/stranded.
- **Root cause (2026-04-29, regression):** Pending-open queue jest globalna i item może zostać obsłużony przez dowolny executor. `reopen_hook` aktualizował wcześniej tylko `strategy_id` executora, który akurat podniósł item, zamiast strategii faktycznie trzymającej `old_position` w `position_addresses`.
- **Fix (follow-up 5):** `reopen_hook` rozwiązuje teraz właścicieli po `old_position` (`strategy_ids_holding_position_address`) i wykonuje replace `old -> new` dla każdej pasującej strategii, a następnie synchronizuje managed allowlist tych strategii. Fallback do pierwotnego `strategy_id` zostaje tylko gdy nie znaleziono właściciela.
- **Guards/tests:** Weryfikacja ręczna na tej samej pozycji: `Position Info` i `Monitored positions (API)` pokazują spójny status linku.
- **Paths:** `web/src/pages/Positions.tsx`, `web/src/pages/PositionDetail.tsx`, `crates/api/src/services/strategy_service.rs`

---

### BUG-20260414-08 — New position detail showed full prior rotation “history” (merged stream component)

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-04-14  
fixed_in: local  
keywords: stream-lineage, position_stream_edges, suppress_jsonl_rotation_stitch, compute_position_stream_lineage, fresh-open, BFS

- **Symptom:** After closing an old NFT and opening a new one, Position detail’s lineage/history table listed many unrelated prior PDAs (same pool) as if they belonged to the new mint.
- **Root cause:** JSONL/registry fallback respected `suppress_jsonl_rotation_stitch` (manual / unanchored opens), but the **DB path** always queried edges using the full undirected BFS component from `compute_position_stream_performance`, which can merge unrelated PDAs when the edge graph is noisy.
- **Root cause (follow-up):** `lifecycle_rotation_parent_before_open` treated **`close_kind=rotation` alone** on an older `bot_close_position` as rotation evidence. API opens are logged as **`bot_open_position`** with a **new** `rebalance_session_id` (`cost_session_id`); a recent unrelated rotation close in the same pool/payer window became a false “parent”, so `suppress_jsonl_rotation_stitch` stayed false and the long chain reappeared even after the DB isolation fix.
- **Fix:** When `suppress_jsonl_rotation_stitch` is true, DB lineage uses an **entry-only** stream member list for the edge query + `build_lineage_chain_from_db_edges`, and stream PnL totals use the same restriction via `compute_position_stream_pnl_for_stream_members`. **Parent inference** no longer uses `close_kind=rotation` without a **session id match** (close vs open) or **bot-tied** rows on the closed PDA in the pre-open window.
- **Fix (follow-up 2):** **Operator** semantics in lifecycle JSONL: `position_open` / `source:cli` / `details.open_origin=operator_api` ⇒ **always** suppress prior history; API open writes `open_origin`; **manual** closes (`position_close`, `close_kind=manual`, `close_source=api`) do not act as rotation parents and stop **forward** JSONL chain walks (commit `4836a2c` was the earlier “no false history” baseline).
- **Guards/tests:** `db_edges_entry_only_positions_ignore_external_rotation_neighbor`, `rotation_parent_ignores_ambient_close_kind_rotation_without_session_or_bot_tie`; `cargo test -p clmm-lp-api services::position_stream_lineage::tests`.
- **Paths:** `crates/api/src/services/position_stream_lineage.rs`, `crates/api/src/services/position_stream_pnl.rs`

### BUG-20260414-07 — Manual “Close position” reported success but sent no on-chain close (dry-run strategy executor)

status: fixed  
severity: critical  
reported_by: user  
first_seen: 2026-04-14  
fixed_in: local  
keywords: close-position, resolve_executor_for_position_ops, dry_run, StrategyExecutor, position ops

- **Symptom:** Operator clicked Close multiple times; UI/API returned success (“position closed”) but the Whirlpool position remained open on-chain.
- **Root cause:** `resolve_executor_for_position_ops` returned the **first** strategy executor in the map. If that strategy was `dry_run=true`, `RebalanceExecutor::execute_full_close_only` **no-oped** (returns `Ok(())` without submitting txs), while `PositionService` still returned `OperationResult::success()`.
- **Fix:** Resolve only executors with `!is_dry_run()` **and** `wallet_pubkey().is_some()`; prefer `__api_position_ops__`, then any qualifying strategy runner, then create the lazy ops executor from env keypair. Added `StrategyExecutor::is_dry_run()`.
- **Guards/tests:** `cargo build -p clmm-lp-api`.
- **Paths:** `crates/api/src/services/position_executor.rs`, `crates/execution/src/strategy/executor.rs`

### BUG-20260414-06 — `min_rebalance_interval_hours: 0` caused a close+open every eval tick (~5m) while in-range

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-14  
fixed_in: local  
keywords: periodic, min_rebalance_interval_hours, eval_interval_secs, rebalance-loop, strategy-parameters

- **Symptom:** Multiple rebalances within ~25 minutes (e.g. 5×) while the position stayed in range; cadence matched **`eval_interval_secs`** (default 300s), not “every N hours”.
- **Root cause:** `DecisionConfig` ties Periodic to `hours_since_rebalance >= periodic_interval_hours`. When **`min_rebalance_interval_hours` / periodic interval is `0`**, `hours_since >= 0` is always true → **Rebalance on every executor tick**. UI could send `0` via numeric field; persisted JSON could also contain `0`.
- **Fix:** Follow-up policy split: `periodic` still guards against `0` (frontend blocks `0`, backend clamps `0 -> 1` defensively if it arrives via API), while non-periodic strategies accept `0` as “no time gate”. Optional empty interval now stays optional (no implicit 1h/24h clamp in strategy parameter mapping).
- **Guards/tests:** `min_rebalance_interval_parses_json_number_and_string`; UI validation for `periodic` rejects `0` with explicit message.
- **Paths:** `crates/api/src/services/strategy_service.rs`, `crates/api/src/handlers/strategies.rs`, `web/src/lib/strategyFormShared.tsx`

### BUG-20260414-05 — Strategy UI stuck on first linked PDA after bot rotations; HTTP start / autostart used divergent executor wiring

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-14  
fixed_in: local  
keywords: strategies, reopen-hook, position_addresses, start-strategy, autostart, StrategyService, state.executors, periodic-rebalance

- **Symptom:** Bot closed and reopened positions (new NFT PDAs) while price stayed in range; dashboard strategy still showed the **original** `parameters.position_addresses` entry (e.g. first mint), not the active position.
- **Root cause (hook):** `start_strategy_executor_core` (used by `POST /strategies/{id}/start`, `ensure_executor_after_link`, and `PUT /strategies/{id}` restart) did **not** register `set_reopen_hook` or `set_managed_allowlist`, so close→open cycles did not call `replace_position_address_in_strategy`.
- **Root cause (map):** `StrategyService::start_strategy` (API boot autostart) stored executors in a **private** `HashMap` separate from `AppState.executors`, so stop/sync/heal paths that read `state.executors` could not see autostarted runners (and behavior diverged from HTTP-started strategies).
- **Fix:** Introduced shared `wire_executor_allowlist_and_reopen_hook`; `start_strategy_executor_core` calls it. `StrategyService` now uses `state.executors` only (removed duplicate map).
- **Guards/tests:** `cargo build -p clmm-lp-api`; existing `managed_allowlist_*` tests still pass.
- **Paths:** `crates/api/src/services/strategy_service.rs`, `crates/api/src/handlers/strategies.rs`

### BUG-20260414-04 — Clearing `position_addresses` via strategy update did not stick; empty list widened executor scope

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-14  
fixed_in: local  
keywords: strategies, put-strategies, position_addresses, linked-positions, update_strategy, managed-allowlist, executor

- **Symptom:** After editing `data/strategies.json` or calling `PUT /strategies/{id}` with `position_addresses: []`, the dashboard still showed long **Linked positions** lists; operators could not reset strategy links for a clean test cycle.
- **Root cause (persistence):** `update_strategy` always re-inserted the previous `position_addresses` JSON into the new config, so an explicit clear in the request body was overwritten.
- **Root cause (semantics):** When `position_addresses` was an empty array, `start_strategy` / `sync_managed_allowlist_from_registry_for_strategy` treated “no configured PDAs” as “use all registry-open PDAs”, which could make a “cleared” strategy still drive automation across unrelated positions after autostart.
- **Fix:** `update_strategy` only restores legacy `position_addresses` / `executor_disabled_position_addresses` when the request omits those fields (`None`); if the client sends them (including `[]`), the request wins. Centralized allowlist helper: explicit `[]` ⇒ empty managed set; missing or non-array field keeps legacy “registry-open” fallback.
- **Guards/tests:** Unit tests `missing_position_addresses_field_uses_registry_open`, `explicit_empty_position_addresses_yields_empty_allowlist`.
- **Paths:** `crates/api/src/handlers/strategies.rs`, `crates/api/src/services/strategy_service.rs`

### BUG-20260414-03 — Stranded list had no operator dismiss control

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-04-14  
fixed_in: local  
keywords: stranded-rebalances, pending-open-recovery, watchdog, ui-action, dismiss-session

- **Symptom:** `Closed by bot, waiting for reopen` accumulates stale/noisy entries with no way to remove them from UI. User could not prepare clean test runs and wanted removed sessions to stop influencing bot recovery flow.
- **Symptom (follow-up):** Removed sessions returned after some time.
- **Symptom (follow-up 2):** `dismissed_session_ids` remained empty in runtime `pending-open-recovery.json` despite UI `Remove`.
- **Symptom (follow-up 3):** After removing one stranded row, bot could still auto-open from another queued pending item with the same `pool + intended range`, so list clearing did not fully stop reopen attempts during manual tests.
- **Symptom (follow-up 4):** `pending-open-recovery.json` still contained reopen items while `Closed by bot, waiting for reopen` section was empty, so operator could not remove hidden queue entries from UI.
- **Symptom (follow-up 5):** Even after removing visible `pending:*` rows, entries could reappear from another queued/derived item in the same `pool + intended range` group (different old position/session id), so operator cleanup still felt non-deterministic.
- **Root cause:** Watchdog/API exposed only read/reconcile operations (`get`/`reconcile`), with no persistent denylist/dismiss mechanism. Pending-open queue could continue using previously queued rows.
- **Root cause (follow-up):** Execution-side pending-open store schema lacked `dismissed_session_ids`; when bot wrote pending-open file, dismiss metadata was dropped, so sessions reappeared.
- **Root cause (follow-up 2):** Dismiss persistence depended on one shared JSON file; if another process rewrote the file without dismiss metadata, hidden sessions resurfaced.
- **Fix:** Added persistent session dismiss flow: `POST /bot-activity/stranded-rebalances/{session_id}/dismiss`. Dismissed session ids are stored in pending-open JSON, excluded from stranded snapshot/reconcile, and matching pending-open item for that session's old position is removed. Execution pending-open schema now preserves `dismissed_session_ids` on load/save.
- **Fix (follow-up 2):** Added separate persisted denylist file for dismissed sessions (`data/stranded-dismissed-sessions.json`, env override `CLMM_STRANDED_DISMISSED_PATH`) and merged it into snapshot/reconcile filters.
- **Fix (follow-up 3):** Dismiss now prunes pending-open queue by both exact `closed_position_nft` and by `pool + intended_tick_lower + intended_tick_upper` group, so operator cleanup keeps stranded/pending views coherent and blocks same-range auto-reopen leftovers.
- **Fix (follow-up 4):** `stranded-rebalances` snapshot now includes synthetic `pending-only` rows for queued reopen items that have no visible lifecycle close row, so every reopen-capable queue item is visible/removable from UI.
- **Fix (follow-up 5):** Dismiss now stores an additional denylist marker per `pool + intended range` (`pending-group:<pool>:<lower>:<upper>`). Snapshot/hide and reconcile/auto-enqueue both honor this group marker, preventing reappearance from sibling sessions/items in the same reopen group.
- **Guards/tests:** Added regression test `dismissed_session_is_excluded_from_stranded_list`; watchdog suite passes.
- **Guards/tests:** Added regression test `pending_open_store_parses_and_keeps_dismissed_sessions`.
- **Guards/tests (follow-up 3):** Added regression test `dismiss_prunes_pending_by_old_position_and_pool_range`.
- **Guards/tests (follow-up 4):** Added regression test `pending_only_item_is_visible_in_stranded_output`.
- **Guards/tests (follow-up 5):** Added regression test `dismissed_pending_group_hides_all_matching_pending_rows`.
- **Paths:** `crates/api/src/services/stranded_rebalance_watchdog.rs`, `crates/api/src/handlers/bot_activity.rs`, `crates/api/src/routes.rs`, `web/src/pages/Positions.tsx`, `web/src/lib/api.ts`, `crates/execution/src/strategy/pending_open.rs`

### BUG-20260414-02 — Manual close appeared in “Closed by bot, waiting for reopen”

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-04-14  
fixed_in: local  
keywords: stranded-rebalances, manual-close, bot-close, close-kind, close-source, ui-section

- **Symptom:** Po ręcznym zamknięciu pozycji wpis potrafił pojawić się w sekcji `Closed by bot, waiting for reopen`, co sugerowało nieprawidłową klasyfikację.
- **Root cause:** Watchdog budujący `stranded-rebalances` traktował każdy `event=bot_close_position` jako close botowy, bez sprawdzenia `details.close_kind` / `details.close_source`.
- **Fix:** Dodano filtr: rekordy close z `details.close_kind=manual` lub `details.close_source=api` są wykluczane z listy stranded.
- **Guards/tests:** Dodany test `manual_close_event_is_excluded_from_stranded_list`.
- **Paths:** `crates/api/src/services/stranded_rebalance_watchdog.rs`

### BUG-20260414-01 — Manual close returns opaque Whirlpool custom 3007

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-14  
fixed_in: local  
keywords: close-position, whirlpool, custom-3007, account-owned-by-wrong-program, signer-wallet, position-nft

- **Symptom:** Manual close failed with `InstructionError(2, Custom(3007))` on Whirlpool ix (`whirLb...`) and API returned generic bad-request text without actionable ownership hint.
- **Root cause:** `classify_close_position_error` handled `6018`/slippage but had no dedicated branch for Whirlpool `3007`, so operators could not distinguish account-ownership mismatch from slippage/funding issues.
- **Fix:** Added explicit close error mapping for `custom 3007` (`custom(3007)` / `custom_code=3007`) with clear hint: verify API signer owns required position/token accounts (especially position NFT ownership).
- **Fix (follow-up):** `close_position` path now treats `3007` as idempotent success **when registry already marks the PDA as closed** (removes stale monitor entry instead of surfacing repeated failure).
- **Guards/tests:** Added regression test `close_position_error_3007_maps_to_bad_request_with_account_hint`.
- **Paths:** `crates/api/src/services/position_service.rs`

### BUG-20260413-06 — Stan portfela na „nowa pozycja” bywa nieaktualny / trzeba odświeżyć

status: open  
severity: medium  
reported_by: user  
first_seen: 2026-04-13  
fixed_in:   
keywords: position-create, wallet-balances, api-signer, stale-data, react-query, race, owner-pubkey

- **Symptom:** Przy wejściu na flow otwarcia nowej pozycji odczyt „Stan portfela” bywa niepełny lub wygląda na opóźniony względem tego, co widać na stronie Wallet; czasem dopiero pełne odświeżenie strony pokazuje oczekiwane salda.
- **Root cause (analiza kodu):** W `PositionCreate` `effectiveOwnerPk` jest liczone jako `apiSigner.pubkey ?? ownerPk` (`ownerPk` = wybrany portfel z listy keypairów w localStorage). Dopóki zapytanie `GET /wallets/api-signer` się nie zakończy, używane jest **tymczasowo** `ownerPk`, więc `useQuery(['wallet-balances', effectiveOwnerPk])` może najpierw pobrać salda **innego** klucza niż ten, którym realnie podpisuje open (API signer). Po dojściu odpowiedzi api-signer klucz query się zmienia i następuje drugi fetch — użytkownik może złapać „złą” chwilę UI lub porównywać z Wallet (tam **zawsze** `ownerPk`, bez api-signer). Dodatkowo `staleTime: 20_000` i globalne `refetchOnWindowFocus: false` sprzyjają pokazywaniu cache bez natychmiastowego ponownego odczytu przy nawigacji.
- **Fix:** Do wdrożenia: np. nie włączać `wallet-balances` na PositionCreate dopóki `api-signer` nie jest `isFetched` (gdy signer jest skonfigurowany), albo jedno zapytanie łączone; ewent. `refetchOnMount: 'always'` / krótszy `staleTime` tylko dla tej strony; jasny komunikat „ładowanie portfela podpisującego…” zamiast chwilowych sald z niewłaściwego ownera.
- **Guards/tests:** Test E2E lub jednostkowy: kolejność rozwiązań query (wallets vs api-signer) nie powinna wyświetlać sald dla złego pubkey; regresja: po swap/open invalidate trafia w ten sam `effectiveOwnerPk`.
- **Paths:** `web/src/pages/PositionCreate.tsx` (`effectiveOwnerPk`, `apiSignerQ`, `effectiveBalancesQ`), `web/src/pages/Wallet.tsx` (porównanie: samo `ownerPk`), `web/src/main.tsx` (domyślne opcje React Query), `crates/api/src/handlers/wallets.rs` (`GET /wallets/api-signer`)

### BUG-20260413-07 — Performance „Value” vs Position history (start/end): duża rozbieżność (~2×) na otwartej pozycji

status: partially fixed  
severity: medium  
reported_by: user  
first_seen: 2026-04-13  
fixed_in: local  
keywords: position-detail, performance, stream-lineage, value-usd, snapshots, valuation-drift, UI-consistency

- **Symptom:** Na `/positions/<PDA>` (otwarta pozycja) karta **Performance → Value** pokazuje np. **~$3.88**, a w **Position history (rotations)** kolumny **start value** i **end value** obie np. **~$1.80** z etykietą `exact` — użytkownik widzi pozornie „dwukrotnie mniej” w tabeli niż w Performance.
- **Symptom (2026-04-14):** Karta **Performance → Value** bywa „historyczna”/zaniżona po wejściu na detal pozycji (frontend cache), mimo że oczekiwana jest bieżąca wycena.
- **Symptom (2026-04-14, follow-up):** Po wdrożeniu odświeżania na mount, w części sesji `Performance -> Value` nadal wygląda na zaniżone (np. ~$2.278) względem oczekiwanej bieżącej wyceny.
- **Symptom (2026-04-14, UI semantics):** Użytkownik mieszał „live value” (single PDA) z agregatem historycznym streamu; etykieta `end value` dla open node była czytana jako „przyszła wartość”.
- **Symptom (2026-04-14, follow-up):** Świeżo otwarta pozycja mogła mieć `opened == closed/last` i widoczne `end value` już na starcie, mimo braku close.
- **Symptom (2026-04-14, runtime chain jump):** Dla jednego chainu (SOL/USDC) kolejne node’y pokazały sekwencję `~2.01 -> ~0.98 -> ~3.05 -> ~6.00` przy niewielkiej zmianie ceny rynkowej; użytkownik raportuje efekt „uciętej nogi” w jednym kroku i „dodanej nogi” w następnym. To wygląda jak niespójna wycena per-node (nieadekwatna do realnego market move), a nie zwykły wynik ceny.
- **Symptom (2026-04-14, close accounting):** `end value` dla części zamknięć bywało policzone tylko z jednej nogi puli, bo `fee_payer_token_deltas` z tx meta nie zawsze zawiera obie nogi (typowo brak jednej nogi przy WSOL/ATA flow).
- **Symptom (2026-04-15, recovery follow-up):** Po `rebalance_incomplete` i recovery-open, nowe `baseline_open` potrafiło być zawyżone (np. ~7 vs oczekiwane ~3) przez mixed-sign delty na open.
- **Root cause (analiza kodu):** Ta metryka „wartość USD” jest liczona w **dwóch miejscach** (nie jest jednym cache): karta Performance vs wiersz lineage. Karta Performance bierze `value_usd` z `GET /positions/{address}`: świeże `compute_position_usd_valuation` na stanie z monitora/RPC (`handlers/positions.rs`). Tabela lineage w `GET /positions/{address}/stream-lineage` dla węzła w ścieżce DB w `node_metrics` ustawia **baseline** i **current** z tabeli `position_stream_valuation_snapshots` (pierwszy vs ostatni wiersz wg sortowania SQL), a niekoniecznie z tą samą chwilą ani tą samą logiką co bieżąca karta. Dla **świeżo otwartej** pozycji często jest **jeden** (lub kilka) snapshotów — **start i end mogą wskazywać ten sam zapis**, z wartością zapisaną wcześniej (inny moment/ceny/jedna noga w delcie) podczas gdy karta już pokazuje nowszą wycenę. Etykieta `exact` w UI pochodzi z `raw_json.valuation_quality` snapshotu lub z ścieżki lifecycle — opisuje **jakość wejścia cen**, nie gwarancję zgodności z kartą Performance.
- **Fix:** (2026-04-13, uproszczenie) Zamiast wielu heurystyk w `node_metrics`, **`persist_event_valuation_snapshots_for_positions`** przy zapisie **`baseline_open`** uzupełnia **tylko brakującą nogę** (delta `amount_*_ui == 0`) z **`details.amount_*_cap`** (Orca max raw); druga noga zostaje z delt. **Pełne zastąpienie obu nóg capami** usunięte — cap to maksimum, nie faktyczny depozyt; dawało zawyżenie wobec **Performance** (np. ~$6.15 vs ~$5.60). `baseline_amounts_source: "open_caps"`. **`ON CONFLICT DO UPDATE`** tylko przy `open_caps`. `node_metrics` — prosty odczyt snapshotów + istniejący guardrail z ledgera.
- **Fix (2026-04-14):** `PositionDetail` wymusza świeży fetch `GET /positions/{address}` na mount (`staleTime: 0`, `refetchOnMount: 'always'`). API zwraca też `valuation_source` (`live_valuation` vs `fallback_monitor`) dla `value_usd`, więc UI odróżnia świeżą wycenę od fallbacku monitora.
- **Fix (2026-04-14, UI):** `PositionDetail` rozdziela semantykę sekcji: `Live value (this position, now)` + jawny opis źródła dla single-PDA endpointu; stream dostał nagłówek „history summary across rotated PDAs”; kolumna historii `end value` zmieniona na `current/end value`.
- **Fix (2026-04-14, follow-up):** W DB-path lineage `closed_ts_utc` jest ustawiane tylko dla snapshotów `raw_json.kind=end_close`; zwykły latest/current snapshot dla aktywnej pozycji nie oznacza zamknięcia, więc UI nie pokazuje `end value` dla świeżego open.
- **Fix (2026-04-14, accounting):** Rebalance executor zapisuje deterministycznie `details.close_amount_a_raw` i `details.close_amount_b_raw` na evencie `bot_close_position` (best-effort świeży odczyt pozycji+pool tuż przed close; fallback do obliczonych amountów „before”). Lineage `node_metrics_from_lifecycle_best_effort` preferuje te pola przy liczeniu `end value`; `fee_payer_token_deltas` zostają tylko jako fallback dla starszych wierszy bez nowych pól.
- **Fix (2026-04-15, follow-up):** `persist_event_valuation_snapshots_for_positions` dla `end_close` nie wymaga już obecności `fee_payer_token_deltas`; najpierw bierze `details.close_amount_*_raw` i dopiero fallbackuje do delt. Dla `baseline_open` gdy brakuje jednej nogi puli w deltach, używa pełnego koszyka z `details.amount_*_cap` (obie nogi), żeby uniknąć mieszanego źródła (delta+cap) i zaniżenia start value. Dodatkowo `ON CONFLICT DO UPDATE` pozwala nadpisywać snapshoty `kind=end_close` (wcześniej warunek blokował aktualizację części zamknięć).
- **Fix (2026-04-15, recovery follow-up 2):** Recovery-like open (bot open + `rebalance_session_id` + wcześniejsze `bot_swap_*` w tej samej sesji) z niespójnymi pool-leg deltami (nie oba wydatkowe) wymusza pełny koszyk caps (`open_caps_recovery`) zamiast mieszania delta+cap.
- **Guards/tests:** `cargo test -p clmm-lp-api position_stream_lineage`; dodane: `pool_legs_strict_spend_requires_both_negative`, `open_row_is_recovery_like_when_prior_session_swap_exists`; po deploy: odśwież stream-lineage (persist) dla danej PDA — stary zły wiersz `baseline_open` może się zaktualizować tylko gdy trafi `open_caps`.
- **Residual risk:** Otwarcie **bez** `amount_*_cap` w `details` (np. część ścieżek CLI) — wtedy tylko delty + guardrail `node_metrics`; nadal możliwy drift cen/RPC. Dodatkowo możliwy pozostaje drift live/fallback przy chwilowej niedostępności RPC/price feed.
- **Symptom (2026-04-16, regression?):** Pozycja `52PR84ugSnNiaWbUAy1jrmf5YL7RqyzwvQmZ5u3wWEoC` pokazuje `start value ~$5.627` w historii (baseline z `amount_a_cap/amount_b_cap`), ale `Performance -> Value` pokazuje `~$0.218` (live valuation).
- **Paths:** `web/src/pages/PositionDetail.tsx` (Performance vs tabela lineage), `crates/api/src/handlers/positions.rs` (`get_position`, zapis snapshotów), `crates/api/src/services/position_stream_lineage.rs` (`node_metrics`, zapytania `position_stream_valuation_snapshots`), `crates/api/src/services/position_valuation.rs`

### BUG-20260413-05 — Stream lineage chained manual opens to unrelated rotation history

status: regressed  
severity: medium  
reported_by: user  
first_seen: 2026-04-13  
fixed_in: local  
keywords: position-stream-lineage, rotation, registry.jsonl, lifecycle, rebalance_session_id, false-parent, manual-open

- **Symptom:** A newly opened position in the same pool as prior activity appeared in **Position history (rotations)** as continuing an old PDA chain instead of a standalone node.
- **Symptom (2026-04-14):** Część nowych botowych rebalance (`close -> open`) nie dopina się do historii i pojawia się jako nowa pozycja startowa; jednocześnie manual opens na tym samym poolu muszą pozostać oddzielnymi historiami.
- **Symptom (2026-04-14, follow-up):** `linked positions` w strategiach rosły nieoczekiwanie; samo wejście na listę pozycji potrafiło modyfikować linki strategii (auto-heal w endpointach odczytowych).
- **Symptom (2026-04-14, follow-up 2):** Po ręcznym `Close` pozycja pozostawała w `position_addresses` strategii, mimo że manual close ma oznaczać koniec historii i brak dalszego zarządzania/reopen dla tej pozycji.
- **Symptom (2026-04-15, follow-up 3):** Recovery-open po `rebalance_incomplete` mógł utworzyć nowy PDA bez `rebalance_session_id`, więc lineage pokazywał nową pozycję jako osobny start-chain (bez powiązania z zamkniętym parentem), a strategia nie miała stabilnej kotwicy sesyjnej do old->new continuity.
- **Symptom (2026-04-15, follow-up 4):** Recovery-open tworzył nowy aktywny PDA, ale strategia pozostawała przypięta do starego `closed_position_nft` (`linked_strategies=[]` na nowej pozycji), co dawało niespójny status linku w UI.
- **Symptom (2026-04-15, follow-up 5):** Strategia z `parameters.position_addresses: []` (zamierzone „zarządzaj niczym”) nadal podejmowała decyzje na monitorowanych pozycjach, powodując częste rebalance nawet in-range.
- **Symptom (2026-04-16, follow-up 6):** Po `bot_open_position` dla `52PR84ug...` z `rebalance_session_id=facce...` stream-lineage potrafi zwrócić chain długości 1 zamiast kontynuacji po `4NL...` (open nastąpił wiele godzin po close; heurystyki parent inference mają okno ~60 min).
- **Root cause:** Registry fallback linked `registry_open` → `registry_close` by time/pool/owner even when `rebalance_session_id` did not match (or was empty). Lifecycle fallback linked opens to the latest close in a short window without requiring rotation evidence; forward close→open used the first qualifying open, not the true successor; lifecycle parent inference treated loose swap rows as rotation.
- **Root cause (runtime `debug-c45ac3.log`, H-lineage):** With DB enabled, `db_chain_from_edges_len` stayed **1** while JSONL fallback inflated `chain_len` to **3–4** — `build_linear_chain` walked from a single graph root and **omitted `entry` on forked `position_stream_edges`** (`A→B` and `A→C`), fell back to `[entry]`, then registry/lifecycle re-stitched a long pool history.
- **Root cause (2026-04-15, follow-up 3):** `recover_open_after_incomplete` called `open_new_range_with_wallet_mix(..., ledger_session_id=None)`; pending-open queue also did not persist session id. Recovered open rows were emitted as unanchored bot opens.
- **Root cause (2026-04-15, follow-up 4):** Pending-open success path added new position to monitor, but skipped normal post-rebalance continuity hooks (`managed_allowlist old->new` replacement and `reopen_hook`), so strategy `position_addresses` were not updated.
- **Root cause (2026-04-15, follow-up 5):** `StrategyExecutor::set_managed_allowlist` converted empty allowlist to `None`, which means unrestricted evaluation. This inverted semantics of explicit empty `position_addresses`.
- **Fix:** Registry parent/chain only when both rows carry the same non-empty `rebalance_session_id`. Lifecycle uses `lifecycle_rotation_parent_before_open` (session match, `close_kind=rotation`, or bot activity tied to the closed PDA); forward links require that helper to return the closed PDA; `infer_parent_position_from_lifecycle_best_effort` delegates to the same helper. DB mode uses **`build_lineage_chain_from_db_edges`** (backward from `entry`, then forward); **JSONL/registry fallback is skipped when any persisted edge touches `entry`**. **Update 2026-04-14:** JSONL suppress now blocks only fresh manual roots; bot-open nodes remain stitchable when rotation parent is inferable even with session mismatch. Rebalance executor now stamps one generated `ledger_session_id` through collect/close/swap/open lifecycle rows to improve deterministic close->open continuity.
- **Fix (follow-up):** Removed implicit strategy-link healing from read endpoints (`GET /positions`, diagnostics). Healing is now explicit via `POST /positions/{address}/heal-strategy-link`. Also hardened `replace_position_address_in_strategy` to only add/sync `new_position` when `old_position` was actually found/replaced, preventing accidental list growth on stale parent calls.
- **Fix (follow-up 2):** `DELETE /positions/{address}` now performs best-effort unlink from all strategies after successful manual close (`remove_position_address_from_all_strategies`), so operator close explicitly ends strategy linkage for that PDA.
- **Fix (follow-up 3):** Pending-open items now persist optional `rebalance_session_id`, `RebalanceResult` exposes generated session id for incomplete rebalance handoff, and recovery open passes that id into `open_new_range_with_wallet_mix` so recovered open rows carry the original session anchor.
- **Fix (follow-up 4):** On successful pending-open recovery, executor now performs the same continuity steps as standard rebalance success: replace `managed_allowlist` entry `old->new` (without growth) and call `reopen_hook(old, new)` to update strategy links.
- **Fix (follow-up 5):** `set_managed_allowlist` now keeps explicit empty list as restrictive (`Some(empty)`, target `0`) instead of unrestricted; added regression test `empty_managed_allowlist_stays_restrictive`.
- **Guards/tests (2026-04-14):** Added `jsonl_stitch_allowed_when_rotation_parent_exists_without_session_match`; existing `jsonl_stitch_suppressed_when_open_session_not_on_prior_close` and `jsonl_stitch_allowed_when_session_matches_prior_close` still pass.
- **Guards/tests:** Unit tests `lifecycle_chain_*`, `db_edges_*`, `jsonl_stitch_*`.
- **Paths:** `crates/api/src/services/position_stream_lineage.rs`, `crates/execution/src/strategy/executor.rs`, `crates/execution/src/strategy/rebalance.rs`, `crates/execution/src/strategy/pending_open.rs`

### BUG-20260413-04 — Open target USD looked too low after success

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-13  
fixed_in: local  
keywords: open-position, target-usd, valuation, wsol, usdc, price-source, drift

- **Symptom:** User sets open target (e.g. `5 USD`), position opens successfully, but displayed post-open value appears noticeably lower.
- **Root cause:** Runtime logs showed quote/open were near target, but later UI value used a lower external SOL/USD feed than pool-implied SOL/USD for the same WSOL/USDC pool state. This created an avoidable valuation drift in display value.
- **Fix:** For WSOL/USDC valuation path, backend now prefers SOL/USD implied from the pool tick (same on-chain state used for token amounts) instead of external feed-only pricing.
- **Guards/tests:** Verified on user reproduction after fix; instrumentation removed post-confirmation.
- **Paths:** `crates/api/src/services/position_valuation.rs`, `crates/api/src/handlers/pools.rs`, `crates/api/src/services/position_service.rs`, `crates/api/src/handlers/positions.rs`

### BUG-20260413-03 — Close Position fails with Whirlpool custom 6018

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-13  
fixed_in: local  
keywords: close-position, whirlpool, custom-6018, tokenminsubceeded, slippage, min-out

- **Symptom:** `Close Position failed: ... InstructionError(2, Custom(6018)) ... TokenMinSubceeded` on `whirLb...` even for manual close flow.
- **Root cause:** Manual close API path mapped executor failures to generic `500 Internal error`, so users did not get actionable guidance for Whirlpool `6018` (`TokenMinSubceeded`) even after executor-side retry logic was added.
- **Fix:** `PositionService::close_position` now classifies close errors (like open path): `6018`/slippage returns `400` with explicit min-out/slippage hint and suggested close-specific knobs (`--slippage-bps`, `WHIRLPOOL_CLOSE_SLIPPAGE_BPS`); wallet misconfiguration is mapped to `503` with signer setup hint.
- **Guards/tests:** Added regression test `close_position_error_6018_maps_to_bad_request_with_hint` in `crates/api/src/services/position_service.rs`; verified with `cargo test -p clmm-lp-api position_service::tests::close_position_error_6018_maps_to_bad_request_with_hint -- --nocapture`.
- **Paths:** `crates/protocols/src/orca/executor.rs`, `crates/api/src/services/position_service.rs`

### BUG-20260413-02 — PositionCreate nie sugerował swapu na operacyjny SOL

status: partially fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-13  
fixed_in: local  
keywords: position-create, swap-suggestion, operational-sol, rent, fees, jupiter

- **Symptom:** UI nie proponował `swap to SOL` przed `Open Position`, a backend zwracał błąd `API wallet has insufficient SOL for rent/fees`.
- **Symptom:** API potrafiło zwrócić `Top up both native SOL balance and wrapped-SOL token amount` mimo że UI nie blokował formularza (WSOL noga wyglądała na pokrytą).
- **Symptom:** `Open position failed: SPL Token program error InsufficientFunds` z logiem `Transfer: insufficient lamports ..., need ...` (Instruction 2) mimo wcześniejszego `Swap potwierdzony`.
- **Symptom:** Kolejne przypadki `InsufficientFunds` (Instruction 0) mimo że UI pokazywał wystarczające „Stan portfela”.
- **Symptom:** Nadal pojawiał się błąd `Transfer: insufficient lamports 24922302, need 33006413` (Instruction 0) nawet po wcześniejszych poprawkach UI.
- **Symptom:** Preflight bywa blokowany mimo wystarczającego `current_wsol` (tokenowo), bo native guard dla WSOL open działał w trybie konserwatywnym `need + pad`.
- **Root cause:** `fundingCheck` w `PositionCreate` sprawdzał tylko deficyty tokenów A/B dla notionalu pozycji i dla WSOL traktował dostępne saldo jako `native SOL + WSOL token`. Brakowało rozdzielenia wymagań: (1) WSOL token na nogę pozycji oraz (2) native SOL >= `min_open_lamports` na rent/fee buffer.
- **Root cause:** Dodatkowo brakowało doliczenia kosztu utworzenia konta WSOL (ATA rent) przy pierwszym użyciu WSOL, więc projekcja native SOL mogła być zaniżona względem realnego `SystemProgram::Transfer`.
- **Root cause:** UI walidował salda na podstawie lokalnie wybranego walleta (`/wallets/balances?owner=...`), podczas gdy `open/swap` są wykonywane przez API signer wallet z backendu. Przy rozjechanych portfelach frontend przepuszczał formularz mimo realnego braku środków na signerze.
- **Root cause:** W `WhirlpoolExecutor::preflight_open_liquidity_balances` pad operacyjny dla open był zbyt niski (`2_500_000` lamportów), więc preflight mógł przepuścić przypadki, które później padały na `SystemProgram::Transfer` po stronie tx build/send.
- **Fix:** Dodano check `shortOperationalSol` oparty o projekcję native SOL po finansowaniu nogi WSOL oraz `GET /wallets/api-signer (min_open_lamports)`. Dodatkowo `getAvailableUiAmount` liczy teraz saldo tokenowe (bez native), więc deficyt nogi WSOL jest wykrywany zgodnie z walidacją backendu. Projekcja odejmuje też estymatę rent dla WSOL ATA, gdy konto WSOL jeszcze nie istnieje. `blocked` uwzględnia oba warunki, a UI pokazuje dedykowany przycisk `Jupiter: swap to SOL`.
- **Fix:** `PositionCreate` przełączył źródło sald na API signer pubkey (z `/wallets/api-signer`) jako główny owner dla `wallet-balances`, walidacji fundingu i odświeżania po swapie. Dodano też notkę UI, że walidacja używa portfela API signer.
- **Fix:** Na podstawie historycznych openów z `data/ledger/orca_position_lifecycle.jsonl` (13 próbek: p50 ~10.08M, p95 ~10.48M, max ~11.07M lamportów) ustawiono domyślny `CLMM_MIN_OPEN_SOL_LAMPORTS` na `12_000_000` (0.012 SOL) jako bufor operacyjny. Wyższe wymagania SOL wynikające z notionalu nogi WSOL są walidowane oddzielnie.
- **Fix:** `WhirlpoolExecutor` używa teraz wspólnego bufora `open_native_sol_pad_lamports()` (env `CLMM_MIN_OPEN_SOL_LAMPORTS`, default `12_000_000`) zamiast sztywnego `2_500_000` w preflight WSOL/open; ten sam bufor jest też używany przy clampowaniu swapu WSOL.
- **Fix:** Usunięto blokujący heurystyczny bail native-SOL na etapie `preflight_open_liquidity_balances` dla nóg WSOL. Zamiast tego przed wysyłką tx wykonywana jest symulacja finalnego planu instrukcji (`simulate_transaction`) i parsowany jest rzeczywisty log runtime `Transfer: insufficient lamports X, need Y`; guard używa teraz `need` z symulacji + margines 1% (`ceil(need*1.01)`).
- **Guards/tests:** `npx tsc --noEmit` w `web/` przechodzi. TODO: test UI regresyjny dla scenariusza „A/B OK, ale operacyjny SOL za niski”.
- **Guards/tests:** `cargo check -p clmm-lp-protocols` przechodzi po zmianie prechecka na exact-plan + 1% margin.
- **Paths:** `web/src/pages/PositionCreate.tsx`, `crates/protocols/src/orca/executor.rs`

### BUG-20260413-01 — Manual close could trigger unintended reopen

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-13  
fixed_in: local  
keywords: manual-close, stale-snapshot, strategy-executor, evaluate_position, reopen, race

- **Symptom:** Po ręcznym `Close Position` chwilę później powstawała nowa pozycja, mimo że użytkownik nie uruchamiał rebalance.
- **Root cause:** `StrategyExecutor::evaluate_position` po `refresh_position` fallbackował do starego cache. Gdy monitor usunął już zamkniętą pozycję, ewaluator nadal mógł wykonać decyzję na starym snapshotcie.
- **Fix:** W `evaluate_position` usunięto fallback do stale cache dla ścieżki po refresh. Teraz:
  - przy błędzie refresh: skip cyklu (bez akcji),
  - gdy po refresh pozycji nie ma już w monitorze: skip cyklu (bez akcji).
- **Guards/tests:** TODO dodać test regresyjny race-case: manual close w trakcie pętli `evaluate_all` nie może wywołać `Rebalance`/`open`.
- **Paths:** `crates/execution/src/strategy/executor.rs`

### BUG-20260410-06 — Rotacja zamyka pozycję bez otwarcia nowej (urwany rebalance flow)

status: open  
severity: high  
reported_by: user  
first_seen: 2026-04-10  
fixed_in:   
keywords: rebalance, close_without_open, swap_mix, recovery, strategy

- **Symptom:** Pozycja zamknięta jako `close_kind=rotation`, ale brak kolejnego `bot_open_position` dla tej sesji/łańcucha.
- **Symptom (2026-04-16):** Dla sesji `facce436-7913-4173-b954-17f403d15a9d` (old PDA `4NLjjVqBtV4CVeFL224UzVpSW4Ds7g16rxuTvir78Qh3`) i `323c4f01-0526-484a-b9c4-ce820e4fc1e6` (old PDA `D6tnfq94B3WnAGeqVX3JUri9AhKfkfHNcNHviyTaBrcV`) recovery ma `attempts=13` i kończy się `open_position failed ... InstructionError(3, Custom(6012))` na `OpenPositionWithTokenExtensions`; wpisy pozostają w `data/pending-open-recovery.json` i w UI `Closed by bot, waiting for reopen`.
- **Symptom (2026-04-16, follow-up):** Bot otworzył nową pozycję `52PR84ugSnNiaWbUAy1jrmf5YL7RqyzwvQmZ5u3wWEoC` z `rebalance_session_id=facce436-7913-4173-b954-17f403d15a9d`, ale w UI nadal potrafi pozostać wpis w `Closed by bot, waiting for reopen` (prawdopodobnie przez pozostawiony item w `data/pending-open-recovery.json`, który generuje synthetic pending-only row mimo że lifecycle ma `bot_open_position`).
- **Symptom (2026-04-17, follow-up):** Dla sesji `2c0ab4d9-dcc6-485d-8beb-ce4a5910365a` (old PDA `DfjqibKyfMtXqkZrfsfmWvbxZxdZTH6m6J1L5qKnv4Xq`) pending-open recovery rośnie do `attempts=68` z błędem `pool tick -24299 not in new range [-24264, -24160): cannot quote deposit for open`; `open_seen=false`, pozycja pozostaje w `Closed by bot, waiting for reopen`.
- **Symptom (2026-04-17, quality follow-up):** Przy długim recovery operator nie miał standaryzowanego pola `stuck_reason` i progu alertu `attempts > N`; diagnoza opierała się na ręcznym czytaniu `last_error`.
- **Root cause:** Rebalance flow urwał się po close + etapach swap (`bot_swap_mix_round` / `bot_swap_exact_in_attempt`) bez finalnego open; brak jawnego `rebalance_incomplete` wpisu dla tego przypadku.
- **Root cause (follow-up 2026-04-17):** Recovery używa sztywno `intended_tick_lower/upper` z momentu close; gdy rynek przesunie się poza ten zakres, `quote_deposit_budget_in_range` odrzuca open (`tick_current` poza nowym pasmem). Ścieżka `recover_open_after_incomplete` nie stosuje adaptacji zakresu (widen/recenter), więc ponawia ten sam niepoprawny zakres.
- **Fix:** Do wdrożenia: twardy marker `rebalance_incomplete` + trwały `pending-open` recovery gdy close zakończony, a open nie doszedł do skutku; UI powinno pokazywać taki status zamiast "po prostu closed".
- **Fix (2026-04-17, quality):** `pending-open` zapisuje telemetry per item (`last_attempt_at`, `stuck_reason`, `stuck_since`, `last_alert_attempts`) i klasyfikuje `stuck_reason` automatycznie z `last_error` (`tick_out_of_range`, `quote_failed`, `rpc_timeout`, `insufficient_balance`, `unknown`). Dodano próg `CLMM_PENDING_OPEN_ALERT_ATTEMPTS` (default 10): po przekroczeniu emitowany jest alert `Pending Open Stuck` (z deduplikacją per item).
- **Symptom (2026-04-27, follow-up):** Dla `retouch_shift` finalny range open po recovery mógł odbiegać od pierwotnego planu strategii po kilku minutach opóźnienia (stary plan z momentu close był wykonywany na nowym rynku).
- **Root cause (2026-04-27, follow-up):** `pending-open` nie trzymał metadanych świeżości planu (`planned_at`, `planned_price`) i recovery otwierał na starych `intended_tick_*` bez jawnej polityki stale/drift replan.
- **Fix (2026-04-27, follow-up):** `pending-open` zapisuje teraz `planned_at_utc` i `planned_price_ab`. Recovery dla `RetouchShift` sprawdza TTL (`CLMM_RECOVER_PLAN_TTL_SECS`, default 180s) i drift ceny (`CLMM_RECOVER_PLAN_MAX_DRIFT_PCT`, default 1%). Przy stale/drift replanuje zakres (zachowując szerokość) wokół bieżącego ticka, loguje `bot_recover_open_replanned`, i zapisuje `range_adjustment_reason` do zdarzenia rebalance.
- **Guards/tests:** test scenariusza: close success + swap rounds + open failure/abort => wpis `rebalance_incomplete` + recovery artifact; dodatkowo testy klasyfikacji `stuck_reason` i progowego alertowania attempts.
- **Paths:** `data/ledger/orca_position_lifecycle.jsonl`, `crates/execution/src/strategy/rebalance.rs`, `crates/execution/src/strategy/pending_open.rs`, `crates/execution/src/strategy/executor.rs`, `crates/execution/src/lifecycle/events.rs`, `crates/api/src/handlers/positions.rs`

### BUG-20260410-05 — Collect Fees: brak executora mimo aktywnego środowiska

status: partially fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-10  
fixed_in: local (pending user verification)  
keywords: collect_fees, executor, wallet, KEYPAIR_PATH, SOLANA_KEYPAIR, WALLET_KEYPAIR_BASE58, fee_owed, position-reader

- **Symptom:** (1) `Collect Fees failed: Service unavailable: Fee collection requires executor and wallet configuration`; (2) po naprawie wallet: `Collect Fees failed: Internal error: collect_fees: read position for fee_owed`.
- **Symptom:** (1) `Collect Fees failed: Service unavailable: Fee collection requires executor and wallet configuration`; (2) po naprawie wallet: `Collect Fees failed: Internal error: collect_fees: read position for fee_owed`; (3) po sukcesie collect brak informacji o kwocie/tokenach zebranych fee.
- **Symptom:** (1) `Collect Fees failed: Service unavailable: Fee collection requires executor and wallet configuration`; (2) po naprawie wallet: `Collect Fees failed: Internal error: collect_fees: read position for fee_owed`; (3) po sukcesie collect brak informacji o kwocie/tokenach zebranych fee; (4) w lineage widoczne `collect 1x` oraz `A=0/B=0` bez jasnej informacji czy to błąd czy faktycznie zero owed.
- **Symptom:** (1) `Collect Fees failed: Service unavailable: Fee collection requires executor and wallet configuration`; (2) po naprawie wallet: `Collect Fees failed: Internal error: collect_fees: read position for fee_owed`; (3) po sukcesie collect brak informacji o kwocie/tokenach zebranych fee; (4) w lineage widoczne `collect 1x` oraz `A=0/B=0` bez jasnej informacji czy to błąd czy faktycznie zero owed; (5) `LP Zebrane`: jedna noga miała wartość, druga pokazywała `-`.
- **Symptom:** (1) `Collect Fees failed: Service unavailable: Fee collection requires executor and wallet configuration`; (2) po naprawie wallet: `Collect Fees failed: Internal error: collect_fees: read position for fee_owed`; (3) po sukcesie collect brak informacji o kwocie/tokenach zebranych fee; (4) w lineage widoczne `collect 1x` oraz `A=0/B=0` bez jasnej informacji czy to błąd czy faktycznie zero owed; (5) `LP Zebrane`: jedna noga miała wartość, druga pokazywała `-`; (6) brak szybkiego wyjaśnienia „dlaczego 0” w UI.
- **Symptom:** (1) `Collect Fees failed: Service unavailable: Fee collection requires executor and wallet configuration`; (2) po naprawie wallet: `Collect Fees failed: Internal error: collect_fees: read position for fee_owed`; (3) po sukcesie collect brak informacji o kwocie/tokenach zebranych fee; (4) w lineage widoczne `collect 1x` oraz `A=0/B=0` bez jasnej informacji czy to błąd czy faktycznie zero owed; (5) `LP Zebrane`: jedna noga miała wartość, druga pokazywała `-`; (6) brak szybkiego wyjaśnienia „dlaczego 0” w UI; (7) komunikat collect bez czytelnego formatu oraz brak widoku collect leg values w tabeli sesji.
- **Symptom:** (8) `Internal error: stream lineage: collect fee rows: error returned from database: kolumna "lp_collected_token_a_raw" nie istnieje` na środowiskach z nieodpaloną migracją `005_ledger_lp_collected_raw.sql`.
- **Symptom (2026-04-17, follow-up):** `positions/{pda}/lifecycle-summary` i karty kosztów/prowizji potrafią pokazywać same zera (`tx=0`, `collect=0`) mimo że `bot-activity/ledger` dla tego samego PDA zawiera `bot_open_position`/`bot_collect_fees`/`bot_close_position` z dodatnimi `tx_fee_lamports` i legami collect.
- **Root cause:** Dwa etapy: (a) resolver executora ładował wallet tylko z części źródeł env i proces API nie dziedziczył signer vars; (b) collect path twardo failował gdy pre-read pozycji do `fee_owed_*` nie powiedzie się, mimo że sama transakcja collect może być wykonalna.
- **Root cause:** (c) zapytanie lineage zakładało fizyczne kolumny `lp_collected_token_*_raw` w `position_stream_ledger_rows`; na starszym schemacie były tylko w `raw_json`.
- **Root cause (follow-up 2026-04-17):** Dwa niezależne problemy regresyjne: (d) w `lifecycle-summary` matching był de facto `session OR ELSE position` (if/else), więc wiersze z obcym `rebalance_session_id` były odrzucane nawet gdy `position_pubkey` pasował do streamu; (e) ingest lifecycle->DB zakładał nowe kolumny (`fee_payer_token_deltas`, `lp_collected_token_*_raw`) i na starszym schemacie kończył się błędem, przez co agregacje DB pozostawały puste.
- **Fix:** Rozszerzono `load_wallet_from_env()` o fallback na `SOLANA_KEYPAIR`/`WALLET_KEYPAIR_BASE58`, dodano diagnostykę env/path oraz jawne przekazywanie signer vars w `Start-ClmmApi-8081.ps1`. Collect nie przerywa się już na błędzie odczytu `fee_owed_*`; wykonuje harvest i zapisuje authoritative leg values tylko gdy pre-read się powiedzie. API `collect_fees` zwraca teraz komunikat z kwotami obu nóg (A/B) wyliczony jako `pre_uncollected - post_uncollected` oraz dołącza szczegóły pre/post w `data`.
- **Fix:** Rozszerzono `load_wallet_from_env()` o fallback na `SOLANA_KEYPAIR`/`WALLET_KEYPAIR_BASE58`, dodano diagnostykę env/path oraz jawne przekazywanie signer vars w `Start-ClmmApi-8081.ps1`. Collect nie przerywa się już na błędzie odczytu `fee_owed_*`; wykonuje harvest i zapisuje authoritative leg values tylko gdy pre-read się powiedzie. API `collect_fees` zwraca teraz komunikat z kwotami obu nóg (A/B) wyliczony jako `pre_uncollected - post_uncollected` oraz dołącza szczegóły pre/post w `data`. Dla lineage dodano notę jakości danych: przy `collect_events > 0` i `A/B == 0` API jawnie komunikuje, że collect został wykonany przy `fee_owed_a/b == 0`.
- **Fix:** Rozszerzono `load_wallet_from_env()` o fallback na `SOLANA_KEYPAIR`/`WALLET_KEYPAIR_BASE58`, dodano diagnostykę env/path oraz jawne przekazywanie signer vars w `Start-ClmmApi-8081.ps1`. Collect nie przerywa się już na błędzie odczytu `fee_owed_*`; wykonuje harvest i zapisuje authoritative leg values tylko gdy pre-read się powiedzie. API `collect_fees` zwraca teraz komunikat z kwotami obu nóg (A/B) wyliczony jako `pre_uncollected - post_uncollected` oraz dołącza szczegóły pre/post w `data`. Dla lineage dodano notę jakości danych: przy `collect_events > 0` i `A/B == 0` API jawnie komunikuje, że collect został wykonany przy `fee_owed_a/b == 0`. Jeśli collect ma tylko jedną nogę zmapowaną, brakująca noga jest normalizowana do `0` (z notą), aby UI nie pokazywał `-`.
- **Fix:** Rozszerzono `load_wallet_from_env()` o fallback na `SOLANA_KEYPAIR`/`WALLET_KEYPAIR_BASE58`, dodano diagnostykę env/path oraz jawne przekazywanie signer vars w `Start-ClmmApi-8081.ps1`. Collect nie przerywa się już na błędzie odczytu `fee_owed_*`; wykonuje harvest i zapisuje authoritative leg values tylko gdy pre-read się powiedzie. API `collect_fees` zwraca teraz komunikat z kwotami obu nóg (A/B) wyliczony jako `pre_uncollected - post_uncollected` oraz dołącza szczegóły pre/post w `data`. Dla lineage dodano notę jakości danych: przy `collect_events > 0` i `A/B == 0` API jawnie komunikuje, że collect został wykonany przy `fee_owed_a/b == 0`. Jeśli collect ma tylko jedną nogę zmapowaną, brakująca noga jest normalizowana do `0` (z notą), aby UI nie pokazywał `-`. Dodano per-node `collect_zero_diagnostics` (in-range share est., swap count est., position share est.) i render w tabeli `LP Zebrane`. Dodatkowo LP legs dla collect są teraz brane priorytetowo z Orca `harvest_position_instructions.fees_quote` (obie nogi), a nie tylko z pre-read `PositionReader`.
- **Fix:** Rozszerzono `load_wallet_from_env()` o fallback na `SOLANA_KEYPAIR`/`WALLET_KEYPAIR_BASE58`, dodano diagnostykę env/path oraz jawne przekazywanie signer vars w `Start-ClmmApi-8081.ps1`. Collect nie przerywa się już na błędzie odczytu `fee_owed_*`; wykonuje harvest i zapisuje authoritative leg values tylko gdy pre-read się powiedzie. API `collect_fees` zwraca teraz komunikat z kwotami obu nóg (A/B) wyliczony jako `pre_uncollected - post_uncollected` oraz dołącza szczegóły pre/post w `data` (kwoty w komunikacie zaokrąglone do 3 miejsc). Dla lineage dodano notę jakości danych: przy `collect_events > 0` i `A/B == 0` API jawnie komunikuje, że collect został wykonany przy `fee_owed_a/b == 0`. Jeśli collect ma tylko jedną nogę zmapowaną, brakująca noga jest normalizowana do `0` (z notą), aby UI nie pokazywał `-`. Dodano per-node `collect_zero_diagnostics` (in-range share est., swap count est., position share est.) i render w tabeli `LP Zebrane`. Dodatkowo LP legs dla collect są teraz brane priorytetowo z Orca `harvest_position_instructions.fees_quote` (obie nogi), a nie tylko z pre-read `PositionReader`. W tabeli sesji (`Logs / rebalances`) dodano kolumnę `Collect values` z `A raw/B raw` dla collect tx.
- **Fix:** (2026-04-13) Query w `stream-lineage` został uodporniony na drift schematu: wartości `lp_collected_token_*_raw` są czytane z `raw_json` (aliasowane do tych samych nazw), bez bezpośredniego odwołania do brakujących kolumn.
- **Fix (2026-04-17):** `lifecycle-summary` używa teraz rzeczywistego OR (`session match` **lub** `position match`) i ma regresyjny test. Ingest `position_stream_ledger_rows` dostał detekcję kolumn `information_schema` i zapisuje fallback-variant SQL zgodny ze starszym schematem (bez optional columns), zamiast cicho tracić cały ingest.
- **Guards/tests:** dodać unit testy resolvera źródeł wallet (ścieżka vs env key material) i test collect_fees dla scenariusza „position pre-read fails but tx still executes”.
- **Paths:** `crates/api/src/services/position_executor.rs`, `crates/api/src/services/position_service.rs`, `crates/api/src/handlers/wallets.rs`, `tools/Start-ClmmApi-8081.ps1`, `crates/execution/src/strategy/rebalance.rs`, `crates/api/src/services/position_stream_lineage.rs`, `crates/api/src/handlers/positions.rs`, `crates/api/src/services/position_stream_performance.rs`

### BUG-20260410-04 — Brak regresyjnych testów UI dla feedbacku collect/swap

status: open  
severity: high  
reported_by: user  
first_seen: 2026-04-10  
fixed_in:   
keywords: ui-tests, collect_fees, swap-before-open, message-passthrough, regression

- **Symptom:** Zmiany w komunikatach UI dla `Collect Fees` i `Swap` mogą wracać do starego, mylącego zachowania bez szybkiego wykrycia.
- **Root cause:** Brak dedykowanych testów integracyjnych frontend dla scenariuszy „success bez signature” / „backend message passthrough”.
- **Fix:** Dodać testy UI/integration dla `PositionDetail` i `PositionCreate` weryfikujące prezentację `message` z API.
- **Guards/tests:** PR touching `web/src/pages/PositionDetail.tsx` lub `web/src/pages/PositionCreate.tsx` powinien zawierać test dla tych ścieżek.
- **Paths:** `web/src/pages/PositionDetail.tsx`, `web/src/pages/PositionCreate.tsx`

### BUG-20260410-01 — Lineage showed inconsistent start/end for same node

status: open  
severity: high  
reported_by: user  
first_seen: 2026-04-10  
fixed_in: local  
keywords: lineage, baseline_value_usd, current_value_usd, rebalance_session_id, rotation

- **Symptom:** same PDA showed different `start/end` across views; sometimes `start` wildly above `end`.
- **Symptom:** `Logs / rebalances` can show `—` in `start value` / `end value` for older rows (or intermittently for the same row) because API emits `0` for baseline/current in some runs.
- **Root cause:** mixed-source lineage valuation with fallback drift.
- **Root cause:** per-node valuation in `stream-lineage` still depends on short-timeout best-effort price fetch (`fetch_mint_prices_usd` with 2s timeout) and incomplete open/close leg deltas in lifecycle rows; when either is missing, baseline/current may remain zero and UI intentionally renders `—` for zero.
- **Fix:** session-first continuity (`close(old)->open(new)` by same `rebalance_session_id`) plus guardrails on cap/continuity fallbacks.
- **Fix:** Added stable mint-price merge in lineage (`live + last-good cache`, TTL 15m) so temporary free-price timeouts do not zero `start/end` for rows that had recent quotes; backfill snapshots now use the same stable price path.
- **Fix:** `stream-lineage` now persists and prefers event snapshots (`baseline_open` / `end_close`) for chain PDAs in DB mode, with token amounts + USD + `price_source` + `valuation_quality` in snapshot `raw_json`; baseline/current selection prioritizes these kinds over generic earliest/latest snapshots.
- **Guards/tests:** Added unit invariants in `position_stream_lineage.rs` (`continuity_from_session_carries_prev_end_to_next_baseline`, `baseline_fallback_guardrail_blocks_implausible_prev_end`).
- **Paths:** `crates/api/src/services/position_stream_lineage.rs`

### BUG-20260410-02 — Collect fees UI said requested, but no real effect

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-10  
fixed_in: local  
keywords: collect_fees, ui-message, dry_run, position_detail

- **Symptom:** UI displayed `Collect requested.` even when backend returned dry-run/no-op style response.
- **Root cause:** frontend success toast ignored backend message payload.
- **Fix:** `PositionDetail` now shows API `message` for collect, not hardcoded text.
- **Guards/tests:** still missing dedicated UI integration test for collect message passthrough (open action item).
- **Paths:** `web/src/pages/PositionDetail.tsx`

### BUG-20260410-03 — Swap step appeared to do nothing

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-10  
fixed_in: local  
keywords: swap-before-open, position-create, dry-run, ui-feedback

- **Symptom:** clicking `Swap` often looked like no action.
- **Root cause:** backend dry-run/info response was not surfaced when signature was missing.
- **Fix:** `PositionCreate` now renders `swapStepInfo` from API message regardless of signature presence.
- **Guards/tests:** still missing dedicated UI integration test for swap message without `swap_signature` (open action item).
- **Paths:** `web/src/pages/PositionCreate.tsx`

