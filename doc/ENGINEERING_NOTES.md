# Engineering notes (code changes)

**Purpose:** short, **append-only** entries whenever someone (or AI) makes a **non-trivial** code change. Optimized for **grep and semantic search**: each entry has a **`keywords:`** line with comma-separated tokens (crates, domains, CLI flags, protocols).

**When to add an entry**

- New or removed public CLI subcommand / important flag.
- Behavioral change in backtest, optimization, execution, or protocol adapters.
- New dependency, breaking RPC/data format assumption, or migration of on-disk layout under `data/`.
- Anything you would explain to a teammate in standup — if it touches multiple files or user-visible behavior, log it here.

**Skip** for: typo fixes, pure refactors with no behavior change, one-line test-only edits.

**Order:** **newest first** (add new `##` sections at the **top**, right under this preamble).

---
## 2026-03-30 — Bot Tier3: position-fee ledger + feeGrowthInside (Whirlpool)

**keywords:** clmm-lp-execution, clmm-lp-protocols, clmm-lp-domain, PositionTruthMode, position_fee_ledger, PositionFeeCheckpoint, orca, whirlpool, tick_array, fee_growth_inside, fee_growth_outside, fee_growth_global
**paths:** `crates/execution/src/strategy/executor.rs`, `crates/execution/src/lifecycle/tracker.rs`, `crates/domain/src/position_fee_checkpoint.rs`, `crates/protocols/src/orca/tick_reader.rs`, `crates/protocols/src/orca/tick_array.rs`

Dodano rozszerzony JSONL ledger checkpointów pozycji (schema_version=2) oraz runtime capture w pętli bota: `event_type=poll` + pre/post dla collect/decrease/close (gdy `fee_mode=position_truth`). W `clmm-lp-protocols` dodano reader TickArray (PDA `tick_array`) i wyliczanie `feeGrowthInside` na podstawie `feeGrowthGlobal` i `feeGrowthOutside` dla granic ticków, zapisywane do ledger dla audytu i walidacji vs real fees.

## 2026-03-30 — Backtest: snapshot `liquidity_active` fee share + debug/tick modes

**keywords:** clmm-lp-cli, backtest_engine, run_single, fee-source snapshots, liquidity_active_raw, dynamic-liquidity-share, tick-aligned-inrange, CLMM_DEBUG_STEP_LIQ_SHARE, CLMM_IN_RANGE_TICK, orca, raydium
**paths:** `crates/cli/src/backtest_engine.rs`, `crates/cli/src/commands/snapshot_price_path.rs`, `crates/cli/src/local_swap_fees.rs`, `crates/cli/src/engine/tests.rs`

W trybie `--fee-source snapshots` backtest teraz przenosi `liquidity_active_raw` z snapshotów do `StepDataPoint` i atrybuuje pool fees per krok dynamicznie jako `position_liquidity / liquidity_active_at_step` (zamiast stałego `pool_active_liquidity` dla całego runu). Dodatkowo:
- env `CLMM_DEBUG_STEP_LIQ_SHARE` wypisuje mechanikę in-range i podział fees (pierwsze N kroków)
- env `CLMM_IN_RANGE_TICK=1` przełącza in-range z floatowych granic (`--lower/--upper`) na tickowo (`tick_current` vs wyznaczane ticki); działa gdy snapshot dostarcza `tick_current`.
---

## 2026-03-30 — CLI: `snapshot-run-curated-all --snapshots-suffix` + pętla 5m

**keywords:** clmm-lp-cli, snapshot-run-curated-all, snapshots-suffix, snapshot-jsonl-suffix, snapshot-backtest-prep, prepared-snapshot-window, powershell, snapshot-loop, orca, raydium, meteora
**paths:** `crates/cli/src/main.rs`, `crates/cli/src/commands/snapshot_backtest_prep.rs`, `crates/cli/src/commands/snapshot_price_path.rs`, `scripts/windows/run-snapshot-loop-5m.ps1`

`snapshot-run-curated-all` obsługuje teraz `--snapshots-suffix <SUFFIX>`: zapisuje snapshoty do `data/pool-snapshots/{protocol}/{pool}/snapshots_<SUFFIX>.jsonl` (zamiast `snapshots.jsonl`) oraz status do `data/snapshot_logs/snapshot-run-curated-all_<SUFFIX>.jsonl`. Dodano skrypt Windows `scripts/windows/run-snapshot-loop-5m.ps1`, który uruchamia zbieranie co 5 minut do wariantu `snapshots_5m.jsonl`.

W praktyce odpalasz oba skrypty równolegle: `scripts/windows/run-snapshot-loop.ps1` (wariant domyślny `snapshots.jsonl`, co 10 minut) oraz `scripts/windows/run-snapshot-loop-5m.ps1` (wariant `snapshots_5m.jsonl`, co 5 minut). Oba procesy zapisują do osobnych plików, więc nie nadpisują się.

Dodano też obsługę wariantu w backtestach: `backtest` / `backtest-optimize` mają flagę `--snapshot-jsonl-suffix 5m` (czytanie `snapshots_5m.jsonl`) oraz `snapshot-backtest-prep --snapshots-suffix 5m`, który zapisuje osobny cache pod `data/backtest-snapshot-cache/orca_5m/...` (manifest do `data/backtest-snapshot-cache/manifest_5m.json`).
---

## 2026-03-30 — CLI: accept datetime in `--start-date/--end-date` + end-exclusive snapshot filtering

**keywords:** clmm-lp-cli, backtest, backtest-optimize, start-date, end-date, datetime, RFC3339, snapshot_price_path, end-exclusive
**paths:** `crates/cli/src/main.rs`, `crates/cli/src/commands/snapshot_price_path.rs`, `crates/cli/src/commands/snapshot_backtest_prep.rs`

Extended snapshots-mode window parsing so `--start-date/--end-date` accept timestamps like `2026-03-24T11:00:00Z` (in addition to `YYYY-MM-DD`). Snapshot JSONL parsing and snapshot cache prep now treat `end_ts` as **exclusive** (`ts >= end_ts` filtered out) to match intended “withdraw at 10:00” semantics.

---

## 2026-03-30 — Snapshot-fee sanity-check override via env var

**keywords:** clmm-lp-cli, backtest, fee-source snapshots, snapshot fee sanity check, CLMM_SNAPSHOT_FEE_SANITY_MAX_RATIO
**paths:** `crates/cli/src/main.rs`

Guardrail „snapshot pool fees vs candle baseline” (ratio default `10x`) was causing `--fee-source snapshots` to fall back when `--price-path-source birdeye` runs without Dune volume scaling (different unit scale between Birdeye `step_volume_usd` and snapshot `fee_growth` deltas). Added env var `CLMM_SNAPSHOT_FEE_SANITY_MAX_RATIO` to override the threshold for experiments/debug runs.

---

## 2026-03-28 — Snapshot JSONL: `resolve_snapshot_jsonl_path` (nie zawsze `.repaired`)

**keywords:** clmm-lp-cli, snapshot_price_path, snapshots.jsonl.repaired, resolve_snapshot_jsonl_path, backtest, calendar window
**paths:** `crates/cli/src/commands/snapshot_price_path.rs`, `crates/cli/src/commands/snapshot_backtest_prep.rs`

Wcześniej przy istnieniu **`snapshots.jsonl.repaired`** wybierano go **zawsze** zamiast `snapshots.jsonl`. Plik naprawczy często **zostaje w tyle** względem append-only kolekcji → okna `--start-date` / `--hours` „ostatnie dni” były **puste** mimo świeżych wierszy w `snapshots.jsonl`. Teraz wybór: **`mtime` nowszy wygrywa** (remis → raw). To nie zastępuje ręcznego usunięcia przestarzałego `.repaired`, ale przy typowym flow collector + stary repair znów widać aktualne timestampy.

---

## 2026-03-28 — CLI: `snapshot-backtest-prep` + `--prepared-snapshot-window` (cache pod szybkie backtesty)

**keywords:** clmm-lp-cli, snapshot-backtest-prep, backtest-snapshot-cache, prepared_snapshot_window, Orca, snapshots.jsonl, backtest, backtest-optimize
**paths:** `crates/cli/src/commands/snapshot_backtest_prep.rs`, `crates/cli/src/commands/snapshot_price_path.rs`, `crates/cli/src/main.rs`, `tools/run_snapshot_backtest_prep_loop.ps1`

Komenda **`snapshot-backtest-prep`** czyta `data/pool-snapshots/orca/<POOL>/snapshots.jsonl` i zapisuje przycięte okna czasowe do **`data/backtest-snapshot-cache/orca/<POOL>/window_h24.jsonl`** (oraz `h48`, `h96`, `d7`, `d30` wg flag) + **`data/backtest-snapshot-cache/manifest.json`**. Domyślne pool-e: SOL/USDC + whETH/SOL (lista jak w module). **`backtest`** / **`backtest-optimize`** przy **`--price-path-source snapshots`** mogą użyć **`--prepared-snapshot-window h24`** (tylko Orca) — wtedy `build_from_orca_snapshots` czyta plik cache zamiast pełnego JSONL (nadal przecięcie z `--hours` / datami). Uruchomienie z root workspace: **`cargo run -p clmm-lp-cli --bin clmm-lp-cli -- snapshot-backtest-prep`**. Skrypt **`tools/run_snapshot_backtest_prep_loop.ps1`**: opcjonalnie **`snapshot-run-curated-all`** + **`snapshot-backtest-prep`** w pętli / Task Scheduler.

---

## 2026-03-28 — `run_single` / snapshot path: human `price_ab` must map to raw before sqrt valuation

**keywords:** clmm-lp-cli, backtest_engine, run_single, price_ab_human_to_raw, price_to_sqrt_q64, token decimals, SOL/USDC, final_value, PnL, snapshot_price_path
**paths:** `crates/cli/src/backtest_engine.rs`, `crates/cli/src/engine/tests.rs`

`estimate_position_liquidity` już używa `price_ab_human_to_raw` dla widełek i ceny wejścia; **`run_single`** liczył `sqrt` dla lower/upper/spot **bez** tego kroku. Przy **różnych `dec_a` / `dec_b`** (np. 9/6) sqrt i kwoty tokenów były w złej przestrzeni względem **L** → absurdalne **final_value / PnL** na ścieżce snapshotów. Dodano **`sqrt_q64_from_price_ab_human`** i użyto go przy wycenie krok po kroku oraz na końcu runu. Test regresji: **`run_single_sol_usdc_decimals_position_value_sane_at_flat_price`**. Test **`birdeye_volume_fees_match_equivalent_snapshot_fee_index`**: przy identycznych krokach cenowych i **`pool_fees_usd` = `step_volume_usd * fee_rate`** wynik `run_single` jest taki sam co przy samym indeksie snapshotów (`step_volume_usd = 0`).

---

## 2026-03-28 — `backtest-optimize`: wire snapshot `fee_growth` index into `run_grid` / `total_fees`

**keywords:** clmm-lp-cli, backtest-optimize, snapshot_fee_index_full, run_grid, run_single, total_fees, snapshot_price_path, fee_growth
**paths:** `crates/cli/src/backtest_engine.rs`, `crates/cli/src/main.rs`, `crates/cli/src/engine/tests.rs`

Ścieżka `--price-path-source snapshots` budowała `per_step_fees_usd` (log „N step buckets”), ale symulacja brała opłaty wyłącznie z `step_volume_usd * fee_rate` albo z Dune swaps — snapshotowe kroki mają `step_volume_usd = 0`, więc **`total_fees` w `TrackerSummary` było zerem**. Dodano opcjonalny map `snapshot_pool_fees_usd` do `run_single` / `run_grid`; `backtest-optimize` przekazuje go gdy `prefer_snapshot_fee_idx` (to samo co dotychczasowy `Auto` + niepusty indeks lub `--fee-source snapshots`). Okna rolling: remap indeksów globalnych na lokalne slice. Test: `snapshot_pool_fee_index_accrues_lp_share_when_in_range`.

---

## 2026-03-27 — CLI: `orca-pool-read` (mainnet RPC, Whirlpool tick / price B/A / liquidity)

**keywords:** clmm-lp-cli, orca-pool-read, mainnet, RpcProvider, WhirlpoolReader, SOLANA_RPC_URL, read-only
**paths:** `crates/cli/src/main.rs`

Nowa subkomenda **`orca-pool-read --pool-address <WHIRLPOOL>`**: tylko odczyt przez `RpcProvider::mainnet()` + `WhirlpoolReader::get_pool_state` — wypisuje m.in. `tick_current`, `sqrt_price_x64`, `price_token_b_per_token_a` (surowy stosunek B/A, nie USD), `liquidity`, minty/vaulty oraz skrót opłat jak przy `orca-pool-fee`.

---

## 2026-03-27 — `swaps-enrich-curated-all`: fail-fast when `SOLANA_RPC_URL` is devnet

**keywords:** clmm-lp-cli, swap_sync, swaps-enrich-curated-all, SOLANA_RPC_URL, devnet, mainnet-beta, STARTUP.md
**paths:** `crates/cli/src/swap_sync.rs`

Curated pools in `STARTUP.md` are mainnet; `getTransaction` against devnet for mainnet signatures fails endlessly. **`swaps-enrich-curated-all`** now errors immediately if the resolved primary RPC URL looks like devnet, with a message to switch or unset `SOLANA_RPC_URL`.

---

## 2026-03-27 — Mainnet prep: `CLMM_EXPECTED_CLUSTER`, RPC cluster guard; CLI backtest-optimize sync; docs

**keywords:** clmm-lp-protocols, clmm-lp-cli, rpc, CLMM_EXPECTED_CLUSTER, mainnet, dry-run, backtest-optimize, run_grid, StratConfig, DuneClient, from_env_swaps_only, MAINNET_OPERATIONAL_CHECKLIST, ORCA_RUNBOOK
**paths:** `crates/protocols/src/rpc/cluster.rs`, `crates/protocols/src/rpc/provider.rs`, `crates/protocols/src/rpc/config.rs`, `crates/data/src/providers/dune.rs`, `crates/cli/src/main.rs`, `crates/cli/src/backtest_engine.rs`, `crates/cli/src/output/optimize_result_json.rs`, `doc/MAINNET_OPERATIONAL_CHECKLIST.md`, `doc/ORCA_RUNBOOK.md`, `doc/README.md`

- **`RpcProvider::new`** runs optional URL-vs-intent validation when **`CLMM_EXPECTED_CLUSTER`** is set (`mainnet-beta` \| `devnet` \| `testnet` \| `localnet`); custom RPC hostnames without keywords are skipped.
- **`clmm-lp-cli`:** zsynchronizowano `backtest-optimize` / `backtest` z aktualnym `run_grid` / `StratConfig` (tylko Static, Threshold, Periodic); `DuneClient::from_env_swaps_only`; usunięto przestarzałe `GridRunParams` / per-step `pool_liquidity_active` z `StepDataPoint`.
- **Docs:** [`doc/MAINNET_OPERATIONAL_CHECKLIST.md`](MAINNET_OPERATIONAL_CHECKLIST.md) + wpis w [`doc/ORCA_RUNBOOK.md`](ORCA_RUNBOOK.md) i indeks [`doc/README.md`](README.md).

---

## 2026-03-27 — CLI tests: no committed snapshot JSONL; inline bytes + temp JSONL

**keywords:** clmm-lp-cli, snapshot_readiness, decode_fixture_tests, snapshot_readiness_regression_test, pool-snapshots, ci, raydium, meteora
**paths:** `crates/cli/tests/decode_fixture_tests.rs`, `crates/cli/tests/snapshot_readiness_regression_test.rs`

Workspace `data/` stays gitignored — we **do not** commit `pool-snapshots/*.jsonl`. Parser regression tests embed one `data_b64` account sample per protocol in Rust source. `snapshot_readiness` regression writes **minimal synthetic JSONL** under a temp `data/pool-snapshots/...` tree (tier-2 fields only) and runs the binary with that cwd.

---

## 2026-03-27 — Orca: full-range (Splash) open, `fetch_positions_for_owner`, Splash pool lookup

**keywords:** clmm-lp-protocols, clmm-lp-execution, clmm-lp-api, clmm-lp-cli, orca, whirlpools, full_range, splash, open_full_range_position_instructions, fetch_positions_for_owner, fetch_splash_pool, BuildUnsignedTxRequest, OpenPositionRequest, ENGINEERING_NOTES
**paths:** `crates/protocols/src/orca/executor.rs`, `crates/protocols/src/orca/pool_reader.rs`, `crates/execution/src/strategy/rebalance.rs`, `crates/api/src/handlers/tx.rs`, `crates/api/src/models.rs`, `crates/api/src/services/orca_tx_service.rs`, `crates/cli/src/commands/orca_position.rs`, `crates/cli/src/main.rs`

- **Full-range open:** `WhirlpoolExecutor::open_full_range_position` + `OpenFullRangeParams`; `full_range` flag on `POST /positions` (`OpenPositionRequest`), `BuildUnsignedTxRequest` (`full_range: true` skips tick fields for `/tx/open/build`), and `OrcaTxService::OpenPositionTxRequest`. `StrategyExecutor::execute_open_position` takes `full_range` and records effective tick range in fee checkpoints.
- **Discovery CLI:** `orca-positions-list` (`fetch_positions_for_owner`) and `orca-splash-pool` (`fetch_splash_pool`). **CLI open:** `--full-range` on `orca-position-open` / `orca-position-open-and-close`.
- **Helper:** `full_range_tick_indexes` in `pool_reader` (uses `orca_whirlpools_core`).

---

## 2026-03-27 — CLI + PS: bot JSONL ledgers (`il` + position-fee) and default `data/bot-runs/devnet/`

**keywords:** clmm-lp-cli, orca-bot-run, orca-bot-open-and-run, il_ledger_path, position_fee_ledger_path, powershell, bot_run_devnet, bot_session_devnet, jsonl, backtest
**paths:** `crates/cli/src/commands/orca_bot.rs`, `crates/cli/src/main.rs`, `tools/bot_run_devnet.ps1`, `tools/bot_session_devnet.ps1`, `doc/ORCA_RUNBOOK.md`

Dodano flagi `--il-ledger-path` i `--position-fee-ledger-path` do `orca-bot-run` / `orca-bot-open-and-run` (podpięte pod `StrategyExecutor::set_il_ledger_path` / `set_position_fee_ledger_path`; katalogi nadrzędne tworzone przed startem). Skrypty `bot_run_devnet.ps1` i `bot_session_devnet.ps1` domyślnie zakładają run w `data/bot-runs/devnet/<timestamp>/` z plikami `il_ledger.jsonl` i `position_fee_ledger.jsonl`), z wyłączeniem przez `-SkipLedger`.

## 2026-03-27 — API: add unsigned tx `increase` + one-command devnet smokes

**keywords:** clmm-lp-api, tx-build, increase-liquidity, orca, whirlpools, devnet, e2e, powershell
**paths:** `crates/api/src/handlers/tx.rs`, `crates/api/src/routes.rs`, `crates/api/src/openapi.rs`, `crates/api/src/handlers/devnet_e2e_tests.rs`, `tools/run_devnet_smokes.ps1`

Dodano brakujący endpoint `POST /tx/increase/build` (unsigned tx flow) oparty o `orca_whirlpools::increase_liquidity_instructions` + smoke test `devnet_unsigned_increase_liquidity_smoke`. Dorzucono też skrypt `tools/run_devnet_smokes.ps1`, który pozwala odpalić cały pakiet `devnet_` ignored testów jedną komendą (z ustawieniem env).

---

## 2026-03-27 — Devnet testability: safer RPC defaults + bot action smoke

**keywords:** clmm-lp-protocols, rpc, devnet, fallback, ankr, unauthorized, clmm-lp-api, bot, soak, e2e
**paths:** `crates/protocols/src/rpc/config.rs`, `crates/api/src/handlers/devnet_e2e_tests.rs`, `tools/run_devnet_smokes.ps1`, `crates/execution/src/monitor/position_monitor.rs`

Zmieniono domyślne fallbacki dla devnet tak, aby **nie dodawać automatycznie** endpointów wymagających API key (np. Ankr) — fallbacki są teraz wyłącznie z env (`SOLANA_RPC_FALLBACK_URLS`). Dodano `PositionMonitor::refresh_position` oraz nowy smoke `devnet_bot_actions_smoke` (open → collect → close) jako szybki test akcji bota bez długiej pętli.

---

## 2026-03-27 — CLI: Orca Whirlpool swap (`orca-swap`) for devnet funding automation

**keywords:** cli, orca, swap, devnet, sol-usdc, automation, whirlpools, sdk
**paths:** `crates/cli/src/commands/orca_swap.rs`, `crates/cli/src/main.rs`, `doc/ORCA_RUNBOOK.md`

Dodano komendę `orca-swap`, która buduje i wysyła swap na Orca Whirlpool przez `orca_whirlpools::swap_instructions` (ExactIn/ExactOut, slippage bps). Pozwala to automatycznie uzyskać (dev)USDC z SOL na devnecie bez ręcznego korzystania z UI.

---

## 2026-03-27 — PowerShell automation: wallet/position rebalance to ~50/50 on devnet

**keywords:** powershell, devnet, automation, rebalance, 50-50, orca-swap, close-position, open-position
**paths:** `tools/devnet_rebalance_wallet_half.ps1`, `doc/ORCA_RUNBOOK.md`

Rozbudowano skrypt `devnet_rebalance_wallet_half.ps1`:
- obsługuje obie strony (SOL->devUSDC oraz devUSDC->SOL, zależnie od overweight),
- opcjonalny tryb pozycji: `close -> rebalance -> open` dla automatyzacji „rebalance po połowie” bez ręcznego przepisywania kroków.

---

## 2026-03-27 — Safer open defaults in CLI (`amount_a/b`) to avoid SDK overflow path

**keywords:** cli, orca, open-position, amount-cap, devnet, sdk, overflow
**paths:** `crates/cli/src/commands/orca_position.rs`, `crates/cli/src/main.rs`

W komendach open (`orca-position-open`, `orca-position-open-and-close`, `orca-bot-open-and-run`) dodano jawne limity `amount_a/amount_b` i bezpieczne domyślne wartości (1000/1000) zamiast `u64::MAX`, aby uniknąć ścieżki overflow po stronie SDK przy wyznaczaniu token amountów dla open.

---

## 2026-03-27 — CLI devnet convenience: `orca-position-open-and-close`

**keywords:** cli, devnet, orca, open-and-close, sol-usdc, automation, smoke-flow
**paths:** `crates/cli/src/commands/orca_position.rs`, `crates/cli/src/main.rs`, `doc/ORCA_RUNBOOK.md`

Dodano komendę `orca-position-open-and-close`, która otwiera pozycję, czeka `--sleep-secs`, a następnie zamyka pozycję (pełne `close`). Ułatwia to szybkie devnet smoke testy “open -> close” bez ręcznego kopiowania `position_address`.

---

## 2026-03-27 — CLI: `orca-position-close` and `orca-position-collect-fees`

**keywords:** cli, orca, devnet, close-position, collect-fees, lifecycle, execution
**paths:** `crates/cli/src/commands/orca_position.rs`, `crates/cli/src/main.rs`, `doc/ORCA_RUNBOOK.md`

Dodano brakujące komendy operacyjne CLI do domykania sesji na devnecie: `orca-position-collect-fees` oraz `orca-position-close`. Obie komendy biorą `--position` i (poza `--dry-run`) używają signing wallet do wykonania ścieżek `collect_fees` i pełnego `close`.

---

## 2026-03-27 — New CLI flow `orca-bot-open-and-run` for devnet operations

**keywords:** cli, orca, bot, devnet, open-and-run, position-address, automation, runbook
**paths:** `crates/cli/src/commands/orca_bot.rs`, `crates/cli/src/main.rs`, `doc/ORCA_RUNBOOK.md`

Dodano komendę `orca-bot-open-and-run`, która wykonuje on-chain `open_position` (SDK path), pobiera realny `created_position` i natychmiast uruchamia na nim `orca-bot-run`. To upraszcza devnetowy flow operatorski (open -> monitor/strategy) i eliminuje ręczne przenoszenie adresu pozycji między krokami.

---

## 2026-03-27 — Orca hardening handoff: real `created_position` + unsigned lifecycle smoke

**keywords:** orca, sdk, created-position, position-address, unsigned-tx, lifecycle, open-decrease-collect-close, devnet, powershell, runbook
**paths:** `crates/protocols/src/orca/executor.rs`, `crates/execution/src/strategy/rebalance.rs`, `crates/cli/src/commands/orca_position.rs`, `crates/api/src/services/position_service.rs`, `crates/api/src/handlers/devnet_e2e_tests.rs`, `tools/bot_run_devnet.ps1`, `tools/bot_session_devnet.ps1`, `doc/DEVNET_WALLET_BOT_LAUNCH_RUNBOOK_V1.md`, `doc/ORCA_RUNBOOK.md`

Dopięto handoff realnego adresu pozycji z Orca SDK do warstw konsumenckich: `WhirlpoolExecutor::open_position` zwraca teraz `created_position` (PDA liczone z faktycznego `position_mint`), a execution/CLI/API przestały polegać na zgadywaniu pozycji po `(pool,ticks)` dla ścieżek open. Dodano także ignored smoke test dla pełnego unsigned lifecycle (`open -> read/decode -> decrease-all -> collect -> close`) oraz wsparcie w skryptach botowych dla wejścia `-OpenBuildResponseJson` (czytanie `position_address` z odpowiedzi `/tx/open/build`), z aktualizacją runbooków operacyjnych.

---

## 2026-03-27 — Devnet e2e open/read coverage for Orca proxy pairs (Nebula pools)

**keywords:** devnet, e2e, orca, proxy-pairs, open-position, read-back, position-address, nebula, smoke-tests
**paths:** `crates/api/src/handlers/devnet_e2e_tests.rs`

Dodano ignored smoke test `devnet_open_and_read_position_proxy_pairs_smoke`, który przechodzi po trzech devnetowych parach proxy (SOL/devUSDC, devSAMO/devUSDC, devTMAC/devUSDC) i dla każdej wykonuje pełny flow: `tx/open/build` -> podpis walletem -> `tx/submit-signed` -> odczyt konta pozycji po `position_address` zwróconym przez API -> deserializacja `WhirlpoolPosition`. Adresy puli pochodzą z tabeli devToken Nebula (Orca Whirlpools, devnet).

---

## 2026-03-27 — `/tx/open/build` now returns `position_mint` and `position_address` + open/read smoke

**keywords:** api, tx-open-build, orca, whirlpools, position-mint, position-address, automation, devnet, smoke-test
**paths:** `crates/api/src/models.rs`, `crates/api/src/handlers/tx.rs`, `crates/api/src/handlers/devnet_e2e_tests.rs`

Rozszerzono kontrakt `BuildUnsignedTxResponse` o pola `position_mint` i `position_address` dla ścieżki `POST /tx/open/build`, aby automatyzacja nie musiała zgadywać adresu pozycji po open. Dla `open` adres pozycji jest liczony z rzeczywistego `position_mint` zwracanego przez Orca SDK (`position = PDA("position", position_mint)`), co eliminuje błędne założenie deterministycznego wyliczania tylko z `(pool,tick_lower,tick_upper)`. Dodano też devnet smoke test `devnet_open_and_read_position_smoke` pokrywający sekwencję open -> submit -> odczyt i deserializację konta pozycji.

---

## 2026-03-27 — Orca devnet bot: WhirlpoolPosition deserialization + tx policy fixes

**keywords:** bot, devnet, orca, whirlpools, position-reader, borsh, policy-gate, allowlist, aToken, token-2022, executor, signer
paths: `crates/protocols/src/orca/position_reader.rs`, `crates/api/src/handlers/tx.rs`, `crates/protocols/src/orca/executor.rs`

Naprawiono wczytywanie on-chain pozycji dla `orca-bot-run` (dodano brakujące `reward_infos` do modelu `WhirlpoolPosition`, żeby `BorshDeserialize` nie kończyło się błędem `Not all bytes read`). Dodatkowo skorygowano policy-gate allowlist w `/tx/submit-signed` (brakujący program-id dla wariantu ATA z `orca_whirlpools` SDK) oraz usunięto błędne wymaganie podpisu dla `position_mint` w `WhirlpoolExecutor::open_position` (fix panic `NotEnoughSigners`).

---

## 2026-03-27 — Session timeout control for devnet bot wrapper

**keywords:** bot, devnet, powershell, timeout, max-runtime, session-wrapper, operations
**paths:** `tools/bot_session_devnet.ps1`, `doc/DEVNET_WALLET_BOT_LAUNCH_RUNBOOK_V1.md`

`bot_session_devnet.ps1` dostal parametr `-MaxRuntimeMinutes`, ktory uruchamia `bot_run_devnet` w osobnym procesie i automatycznie zatrzymuje sesje po zadanym czasie. Skrypt nadal zapisuje raport post-run i oznacza status `run_status=timeout`, co pozwala bezpiecznie uruchamiac ograniczone czasowo sesje pod scheduler/ops.

---

## 2026-03-27 — Devnet bot ops scripts: preflight, run wrapper, post-run report

**keywords:** bot, devnet, runbook, powershell, preflight, orca-bot-run, operations, reports
**paths:** `tools/bot_preflight.ps1`, `tools/bot_run_devnet.ps1`, `tools/bot_postrun_report.ps1`, `doc/DEVNET_WALLET_BOT_LAUNCH_RUNBOOK_V1.md`

Dodano trzy skrypty operacyjne pod powtarzalne uruchamianie bota na devnecie: `bot_preflight.ps1` (fail-fast check env/RPC/keypair), `bot_run_devnet.ps1` (wrapper na `orca-bot-run` z trybem dry-run/execute i domyslnym preflight) oraz `bot_postrun_report.ps1` (raport sesji JSON do `data/reports/`). Runbook v1 uzupelniono o gotowe komendy dla tych skryptow.

---

## 2026-03-27 — One-command devnet bot session wrapper

**keywords:** bot, devnet, powershell, session-wrapper, automation, preflight, report
**paths:** `tools/bot_session_devnet.ps1`, `doc/DEVNET_WALLET_BOT_LAUNCH_RUNBOOK_V1.md`

Dodano nadrzedny skrypt `bot_session_devnet.ps1`, ktory spina caly przebieg sesji w jednej komendzie: preflight (opcjonalnie), uruchomienie `orca-bot-run`, a nastepnie zapis raportu post-run. Przy bledzie uruchomienia skrypt nadal zapisuje raport z `run_status=failed`, co poprawia audyt i niezawodnosc operacyjna pod scheduler.

---

## 2026-03-27 — Tier3 usability: per-position readiness + MVP position-truth report CLI

**keywords:** tier3, position-truth, snapshot-readiness, position-address, position-truth-report, jsonl, clmm-lp-cli
**paths:** `crates/cli/src/bin/snapshot_readiness.rs`, `crates/cli/src/bin/position_truth_report.rs`, `crates/cli/tests/snapshot_readiness_regression_test.rs`

W trybie `--fee-mode position-truth` Tier3 readiness jest teraz liczone **per pozycja** (filtruje checkpointy po `pool+position`). Jeśli `--position-address` nie jest podany, narzędzie auto-wykrywa pozycje z ledgeru dla danego poola: gdy jest dokładnie jedna, używa jej automatycznie; gdy jest wiele, wypisuje listę i wymaga wyboru. Dodano nowy bin `position-truth-report` (MVP), który czyta `data/position-fee-checkpoints.jsonl` i wypisuje podsumowanie oraz tail checkpointów dla wskazanego `(pool, position)`. Dodano testy na fixture JSONL.

---

## 2026-03-27 — Tier3 wiring: default checkpoint ledger path enabled in CLI bot and API strategy start

**keywords:** tier3, position-truth, checkpoint-ledger, orca-bot, api-strategy, jsonl, clmm-lp-cli, clmm-lp-api
**paths:** `crates/cli/src/commands/orca_bot.rs`, `crates/api/src/handlers/strategies.rs`, `crates/api/src/services/strategy_service.rs`

Domyślnie włączono zapisywanie checkpointów fee pozycji do `data/position-fee-checkpoints.jsonl` podczas uruchamiania bota CLI (`orca_bot`) oraz startu strategii w API/StrategyService. Dzięki temu Tier3 `snapshot-readiness --fee-mode position-truth` ma z czego czytać bez dodatkowej konfiguracji ścieżki.

---

## 2026-03-27 — Tier3 (PR3 WIP): snapshot-readiness reads position-fee checkpoint ledger

**keywords:** tier3, position-truth, snapshot-readiness, checkpoint-ledger, jsonl, clmm-lp-cli
**paths:** `crates/cli/src/bin/snapshot_readiness.rs`, `crates/cli/tests/snapshot_readiness_regression_test.rs`

`snapshot-readiness` w trybie `--fee-mode position-truth` potrafi teraz czytać lokalny JSONL z checkpointami (`data/position-fee-checkpoints.jsonl` lub `--position-fee-ledger-path`) i na tej podstawie wylicza Tier3 READY/NOT READY wraz z listą braków (min. 2 checkpointy dla poola + `open_position` + postęp typu `collect/close/rebalance`). Dodano test integracyjny z tempowym ledgerem checkpointów.

---

## 2026-03-27 — Tier3 prep (PR2): position-fee checkpoint ledger wired into lifecycle/strategy flow

**keywords:** tier3, position-truth, lifecycle, strategy-executor, position-fee-checkpoint, jsonl, clmm-lp-execution
**paths:** `crates/execution/src/lifecycle/tracker.rs`, `crates/execution/src/strategy/executor.rs`

Dodano dedykowany ledger JSONL dla checkpointów fee pozycji (`set_position_fee_ledger_path` + `record_fee_checkpoint`) w `LifecycleTracker`. `StrategyExecutor` emituje teraz checkpointy dla kluczowych operacji (`open_position`, `decrease_liquidity`, `collect_fees`, `close_position`) oraz podczas udanego `rebalance` (checkpoint `rebalance_out` dla starej pozycji i `rebalance_in` dla nowej). Dzięki temu zaczyna powstawać timeline danych pod tryb `position_truth` bez zmiany domyślnego flow `heuristic`.

---

## 2026-03-27 — Tier3 prep (PR1): fee mode switch + domain checkpoint model skeleton

**keywords:** tier3, position-truth, heuristic, fee-mode, checkpoint, clmm-lp-domain, clmm-lp-execution, snapshot-readiness
**paths:** `crates/domain/src/position_fee_checkpoint.rs`, `crates/domain/src/lib.rs`, `crates/domain/src/prelude.rs`, `crates/execution/src/strategy/executor.rs`, `crates/cli/src/bin/snapshot_readiness.rs`

Dodano szkielet pod drugi tryb fee accounting: `PositionTruthMode` (`heuristic` vs `position_truth`) oraz minimalny model `PositionFeeCheckpoint` w crate `domain`. `ExecutorConfig` w `execution` dostał pole `fee_mode` (domyślnie `Heuristic`, więc brak regresji obecnego flow). CLI `snapshot-readiness` przyjmuje teraz `--fee-mode` i raportuje aktywny tryb; ścieżka `position_truth` jest jawnie oznaczona jako jeszcze niepodpięta do evaluatora Tier3.

---

## 2026-03-27 — Meteora snapshots: always emit vault_amount fields for Tier1 readiness

**keywords:** meteora, snapshot-collector, snapshot-readiness, tier1, vault-amount, token-account, clmm-lp-cli
**paths:** `crates/cli/src/snapshots/collector.rs`

W collectorze Meteora dopięto stabilne emitowanie `vault_amount_a` i `vault_amount_b` w każdym nowym wierszu snapshotu: gdy RPC decode reserve-account się powiedzie, zapisujemy realne wartości; gdy odczyt jest niedostępny, zapisujemy fallback `0` oraz `vault_amount_source="missing_fallback_zero"`. Dzięki temu `snapshot-readiness` ma komplet pól wymaganych przez Tier1 (`LP-share`) i po dosnapshotowaniu co najmniej 2 nowych wierszy zaczyna raportować `Tier1 READY`.

---

## 2026-03-26 — tx unsigned build: Orca SDK open_position instruction builder

**keywords:** tx-build, unsigned-tx, orca_whirlpools, open_position_instructions_with_tick_bounds, partial-sign, clmm-lp-api
**paths:** `crates/api/src/handlers/tx.rs`, `crates/api/src/handlers/devnet_e2e_tests.rs`

W `POST /tx/*/build` unsigned flow wdrożono realne instrukcje z `orca_whirlpools` SDK (dla `open` przez `open_position_instructions_with_tick_bounds`, a dla `decrease/collect/close` wyprowadzamy `position_mint` z on-chain `WhirlpoolPosition` i używamy odpowiednich `*_instructions`). Dodatkowo server pre-signuje wymagane `additional_signers` (partial signatures), a testy Phantom-emulacji ustawiają wyłącznie signature wallet w odpowiednim slocie.

---

## 2026-03-26 — Strategy-driven bot: wallet + monitor seeding on start

**keywords:** bot, strategy-executor, auto_execute, wallet, KEYPAIR_PATH, position-monitor, devnet-e2e, clmm-lp-api
**paths:** `crates/api/src/handlers/strategies.rs`, `crates/api/src/handlers/devnet_e2e_tests.rs`

`POST /strategies/{id}/start` może teraz zasilić `PositionMonitor` listą pozycji z `parameters.position_addresses`. Dodatkowo, gdy `auto_execute=true` i `dry_run=false`, API wymusza i ładuje signing wallet z `KEYPAIR_PATH`/`SOLANA_KEYPAIR_PATH` oraz podpina go do `StrategyExecutor`, dzięki czemu strategie realnie sterują rebalance na devnecie (patrz `devnet_strategy_driven_rebalance_smoke`).

---

## 2026-03-27 — Quick data verifier (snapshot + decode + health, GO/NO-GO)

**keywords:** operations, quick-verify, snapshot-readiness, decode-audit, data-health-check, go-no-go, powershell
**paths:** `tools/quick_verify_data.ps1`, `doc/ORCA_RUNBOOK.md`

Dodano jedno-komendowy verifier operacyjny (`tools/quick_verify_data.ps1`) łączący `snapshot-readiness`, `data-health-check` i `swaps-decode-audit` w raport GO/NO-GO (`data/reports/quick_verify_*.json`) z kodem wyjścia 2 przy FAIL (pod scheduler/CI). W runbooku dodano sekcję z szybkim uruchomieniem.

---

## 2026-03-26 — Devnet production-readiness checklist (3 phases)

**keywords:** devnet, bot, production-readiness, checklist, go-no-go, operations, tx-safety
**paths:** `doc/DEVNET_BOT_PRODUCTION_READINESS.md`, `doc/README.md`

Dodano dedykowany dokument z checklista przejscia z devnet MVP do trybu production-like: faza 1 (must-have, blokery), faza 2 (stabilnosc operacyjna), faza 3 (hardening/rollout), wraz z Definition of Ready i kolejnoscia wdrozenia.

---

## 2026-03-26 — tx unsigned build: real Whirlpool instructions (not empty shell)

**keywords:** tx-build, unsigned-tx, phantom-flow, whirlpool-instruction, clmm-lp-api
**paths:** `crates/api/src/handlers/tx.rs`, `crates/api/src/handlers/devnet_e2e_tests.rs`

W `POST /tx/*/build` unsigned flow przestał budować pusty shell tx i zamiast tego generuje transaction z instrukcjami programu Whirlpool (open/decrease/collect/close), tak aby policy-gate i client-signing działały na realnym program-id/strukturze. Nadal jest to MVP względem pełnych list wymaganych kont (tick arrays / vaults) i docelowo zostanie rozszerzone o produkcyjną poprawność kont.

---

## 2026-03-26 — BuildUnsignedTxRequest: tick bounds required for `open` unsigned build

**keywords:** tx-build, unsigned-tx, open, whirlpool, tick-lower, tick-upper, api-validation, clmm-lp-api
**paths:** `crates/api/src/models.rs`, `crates/api/src/handlers/tx.rs`, `crates/api/src/handlers/devnet_e2e_tests.rs`

Dodano do `BuildUnsignedTxRequest` pola `tick_lower`/`tick_upper` oraz zaostrzono walidacje `POST /tx/open/build`: teraz `open` wymaga tych pól i encoduje je w danych instrukcji Whirlpool `open_position` zamiast `0/0`.

---

## 2026-03-26 — tx build/submit API: fail-safe request validation

**keywords:** tx-build, unsigned-tx, submit-signed, api-validation, clmm-lp-api
**paths:** `crates/api/src/handlers/tx.rs`, `crates/api/src/handlers/tx_tests.rs`, `crates/api/src/handlers/devnet_e2e_tests.rs`

Dodano twarde walidacje w `POST /tx/*/build` (wymagane pola dla open/decrease/collect/close + sanity check slippage), aby uniknac budowania niekompletnych/ryzykownych transakcji w trybie unsigned flow. Zaktualizowano devnet E2E testy unsigned flow pod nowe wymagania requestu.

---

## 2026-03-26 — Devnet E2E hardening: fail-fast keypair + negative submit tests

**keywords:** devnet, e2e, hardening, keypair, fail-fast, unsigned-tx, api-validation, clmm-lp-api
**paths:** `crates/api/src/handlers/devnet_e2e_tests.rs`

Usunięto „ciche” przechodzenie testów bez portfela: testy lifecycle i unsigned flow wymagają teraz jawnie `KEYPAIR_PATH`/`SOLANA_KEYPAIR_PATH` (fail-fast). Dodano negatywne testy submit (`unsigned tx` oraz `invalid base64`) żeby walidować granice API i policy flow na devnecie.

---

## 2026-03-26 — Devnet bot E2E pack: lifecycle endpoint + unsigned tx API + policy gate

**keywords:** devnet, e2e, bot-simulation, positions-decrease, unsigned-tx, phantom-flow, submit-signed, policy-gate, clmm-lp-api
**paths:** `crates/api/src/handlers/positions.rs`, `crates/api/src/handlers/tx.rs`, `crates/api/src/handlers/devnet_e2e_tests.rs`, `crates/api/src/routes.rs`

Dodano endpoint `POST /positions/{address}/decrease` oraz nowy zestaw endpointów unsigned tx (`/tx/*/build`, `/tx/submit-signed`) z policy gate (allowlist programów + preflight simulate). Rozszerzono pakiet `#[ignore]` o testy devnet lifecycle keypair i flow build->sign->submit (emulator Phantom przez keypair).

---

## 2026-03-26 — Async communication layer v2 scaffold (`EventBus`, contract, broker mode, metrics)

**keywords:** async-communication, event-bus, inprocess, broker, kafka, nats, redis, event-contract, correlation-id, clmm-lp-api
**paths:** `crates/api/src/events.rs`, `crates/api/src/state.rs`, `crates/api/src/websocket.rs`, `crates/api/src/main.rs`, `doc/ASYNC_COMMUNICATION_LAYER.md`

Dodano podstawową warstwę komunikacji eventowej: wersjonowany `EventEnvelope`, `EventBus` trait, `InProcessEventBus`, scaffold `BrokerEventBus` (z `EVENT_BUS_MODE` i feature `broker-event-bus`), retry publish + DLQ oraz metryki busa podpinane do `/metrics`. WebSockety subskrybują teraz eventy (`position.updated`, `alert.raised`) z busa.

---

## 2026-03-26 — API coverage suite: wszystkie endpointy z `routes` (REST + WS) mają testy

**keywords:** api, test-coverage, axum-router, websocket, routes, clmm-lp-api, endpoint-tests
**paths:** `crates/api/src/handlers/endpoint_coverage_tests.rs`, `crates/api/src/handlers/mod.rs`

Dodano router-level test suite, która uderza we wszystkie endpointy z `create_router` (w tym `/ws/positions` i `/ws/alerts`) i weryfikuje reachability/statusy na poziomie HTTP/upgrade. Testy są stabilizowane przez mocki dla `/orca/*` i przez asercje akceptujące warianty statusów zależne od live RPC.

---

## 2026-03-26 — Devnet smoke pack rozszerzony: `/orca/pools`, `/orca/tokens`, `/orca/protocol`

**keywords:** devnet, smoke, orca, live-api, ignored-tests, clmm-lp-api
**paths:** `crates/api/src/handlers/devnet_e2e_tests.rs`

Rozszerzono ręczny pakiet smoke (`#[ignore]`) o testy live dla proxy Orca REST, tak aby jednym zestawem móc szybko sprawdzić ścieżkę API→Orca oraz API→RPC devnet po zmianach.

---

## 2026-03-26 — Orca REST proxy: `/orca/pools/*` + `/orca/lock/*` (client + API + tests)

**keywords:** orca, orca-rest, clmm-lp-data, clmm-lp-api, axum, openapi, pools-search, lock, httpmock
**paths:** `crates/data/src/providers/orca_rest.rs`, `crates/api/src/handlers/orca.rs`, `crates/api/src/routes.rs`, `crates/api/src/openapi.rs`

Rozszerzono `OrcaRestClient` o `GET /pools/search`, `GET /pools/{address}` i `GET /lock/{address}` oraz wystawiono je w naszym API jako proxy pod `/orca/...` (z OpenAPI i testami `httpmock`, bez wywołań sieci).

---

## 2026-03-26 — Phantom auth foundations: challenge/verify (`signMessage`) + nonce store

**keywords:** phantom, auth, signMessage, ed25519, jwt, clmm-lp-api, axum, replay-protection
**paths:** `crates/api/src/handlers/phantom_auth.rs`, `crates/api/src/state.rs`, `crates/api/src/routes.rs`, `crates/api/src/models.rs`

Dodano minimalne, bezpieczne fundamenty pod komunikację Phantom ↔ bot: endpointy `POST /auth/phantom/challenge` i `POST /auth/phantom/verify` (challenge–response), in-memory nonce store z TTL oraz odrzucanie replay (nonce jednokrotnego użytku). To umożliwia model “bot układa tx, Phantom podpisuje”.

---

## 2026-03-26 — Orca REST proxy domknięty o tokeny/protocol + devnet API smoke test

**keywords:** orca, tokens, protocol, api-proxy, clmm-lp-data, clmm-lp-api, devnet, e2e-smoke, httpmock
**paths:** `crates/data/src/providers/orca_rest.rs`, `crates/api/src/handlers/orca.rs`, `crates/api/src/handlers/devnet_e2e_tests.rs`, `crates/api/src/routes.rs`

Dodano brakujące endpointy Orca Public API (`/tokens`, `/tokens/search`, `/tokens/{mint}`, `/protocol`) w kliencie i proxy `/orca/*` wraz z testami `httpmock`. Dodatkowo dodano ręczny test smoke `#[ignore]` pod devnet (`devnet_pool_state_smoke`) do szybkiej walidacji ścieżki API→RPC.

---

## 2026-03-26 — CLI: local-first `studio-stream-plan` (AI stream agent MVP)

**keywords:** clmm-lp-cli, studio-stream-plan, ai-narrator, stream, obs, youtube, local-first, jsonl
**paths:** `crates/cli/src/main.rs`, `crates/cli/src/commands/studio.rs`, `doc/AI_STREAM_AGENT.md`

Dodano minimalną komendę CLI `studio-stream-plan`, która czyta lokalny JSONL z “itemami do narracji” i generuje JSONL segmentów z szablonem narracji (PL/EN, `style`, `pause_secs`). To jest warstwa przygotowująca artefakty do późniejszego TTS/OBS bez wiązania projektu z konkretnym dostawcą i bez zależności od płatnych feedów.

---

## 2026-03-26 — Rebranding: “Bociarz LP Strategy Lab” (public-facing docs/UI)

**keywords:** rebrand, branding, README, openapi, cli-about, web-title, attribution, MIT
**paths:** `README.md`, `STARTUP.md`, `Cargo.toml`, `web/index.html`, `web/package.json`, `web/README.md`, `crates/api/src/openapi.rs`, `crates/api/src/main.rs`, `crates/cli/src/main.rs`, `crates/domain/src/lib.rs`, `ATTRIBUTION.md`

Wprowadzono rebranding repo na “Bociarz LP Strategy Lab” w user-facing tekstach (README, STARTUP, CLI/API/OpenAPI oraz web title). Dodano `ATTRIBUTION.md` i zachowano upstream `LICENSE` (MIT) zgodnie z wymogami licencyjnymi.

## 2026-03-26 — Orca integration: `OrcaReadService` + `OrcaTxService` skeleton contract

**keywords:** OrcaReadService, OrcaTxService, clmm-lp-api, REST, tx-service, WhirlpoolReader, PositionReader, WhirlpoolExecutor, endpoint-map
**paths:** `crates/api/src/services/orca_read_service.rs`, `crates/api/src/services/orca_tx_service.rs`, `doc/ORCA_API_SERVICE_CONTRACT.md`, `crates/api/src/services/mod.rs`, `crates/api/src/prelude.rs`

Dodano szkielety serwisów jako jednowymiarowy kontrakt integracyjny (read REST + on-chain fallback, write on-chain) z gotową mapą endpointów/metod w `doc/ORCA_API_SERVICE_CONTRACT.md`.

---

## 2026-03-26 — API: PositionService open/close/collect wykonuje tx przez executor (dry-run testowane)

**keywords:** clmm-lp-api, PositionService, open_position, close_position, collect_fees, OrcaTxService, RebalanceExecutor, execute_open_position, executor-delegation, dry-run-tests
**paths:** `crates/api/src/services/position_service.rs`, `crates/api/src/handlers/positions.rs`, `crates/execution/src/strategy/rebalance.rs`, `crates/execution/src/strategy/executor.rs`

Zrobiono kolejne domknięcie MVP: serwis pozycji ma realna delegacje do executor-a dla `open_position/close_position/collect_fees` (z dry-runem bez wymagania walleta), a endpointy pozycji w API korzystaja z PositionService zamiast placeholderow. Dodano testy jednostkowe dla ścieżek dry-run i walidacji.

---

## 2026-03-26 — Automation: `ops-ingest-cycle` wrapper command + JSON report

**keywords:** ops-ingest-cycle, automation, Task Scheduler, snapshots, swaps-sync, swaps-enrich, decode-audit, data-health-check, clmm-lp-cli
**paths:** `crates/cli/src/main.rs`, `doc/PROJECT_OVERVIEW.md`

Dodano komendę `ops-ingest-cycle` jako „one-shot” wrapper uruchamiający cykl ingestu i metryk (snapshots → sync → enrich → audit → health-check) w jednym procesie. Komenda zapisuje raport JSON w `data/reports/` oraz ma `--fail-on-alert` do integracji z schedulerem.

---

## 2026-03-26 — Automation: `ops-ingest-loop` long-lived runner (Windows Service friendly)

**keywords:** ops-ingest-loop, windows service, nssm, automation, long-lived, backoff, jitter, clmm-lp-cli
**paths:** `crates/cli/src/main.rs`, `doc/TODO_ONCHAIN_NEXT_STEPS.md`

Dodano `ops-ingest-loop`: ciągły runner wykonujący cykl ingestu w pętli z interwałem, jitterem oraz backoff po błędach. Docelowo uruchamiany jako Windows Service (np. przez NSSM) zamiast Task Scheduler.

---

## 2026-03-26 — `swaps-subscribe-mentions`: presety `--mentions-preset` (Orca/Raydium/Meteora)

**keywords:** swaps-subscribe-mentions, mentions-preset, websocket, logsSubscribe, program-id, orca, raydium, meteora, clmm-lp-cli
**paths:** `crates/cli/src/main.rs`, `crates/cli/src/swap_sync.rs`, `doc/PROJECT_OVERVIEW.md`

Dodano `--mentions-preset <orca|raydium|meteora>` jako wygodny skrót do gotowych Program ID (z możliwością ręcznego override przez `--mentions`). Dzięki temu uruchomienie subskrypcji nie wymaga każdorazowego wpisywania pubkey.

---

## 2026-03-26 — Robust pull sync: paged `getSignaturesForAddress` + retry/backoff

**keywords:** swaps-sync-curated-all, getSignaturesForAddress, pagination, retry, backoff, max-pages, clmm-lp-cli, swap_sync
**paths:** `crates/cli/src/swap_sync.rs`, `crates/cli/src/main.rs`, `doc/PROJECT_OVERVIEW.md`

`swaps-sync-curated-all` dostał ulepszenie ścieżki pull (Opcja 3): paginację po `before` (arg `--max-pages`) oraz retry z backoff dla każdej strony RPC. Dzięki temu przy publicznych endpointach można zbierać więcej historii na run i ograniczyć dropy przy transient timeout/rate-limit bez zmiany formatu `data/swaps/.../swaps.jsonl`.

---

## 2026-03-26 — `logsSubscribe` po `mentions` do lokalnego `swaps.jsonl`

**keywords:** swaps, logsSubscribe, mentions, websocket, Solana RPC, clmm-lp-cli, swap_sync, ingest
**paths:** `crates/cli/src/swap_sync.rs`, `crates/cli/src/main.rs`, `doc/PROJECT_OVERVIEW.md`

Dodano komendę CLI `swaps-subscribe-mentions`, która otwiera websocket do RPC (`logsSubscribe` z filtrem `mentions`) i dopisuje nowe sygnatury do `data/swaps/<protocol>/<pool>/swaps.jsonl` z deduplikacją po `signature`. To jest opcjonalna ścieżka near-real-time obok istniejącego pull (`getSignaturesForAddress`) i utrzymuje ten sam format artefaktów wejściowych dla dalszego enrich/decode.

---

## 2026-03-26 — Strategy loop: `CollectFees` / `Close` on-chain + kolejność decyzji

**keywords:** StrategyExecutor, DecisionEngine, CollectFees, Close, RebalanceExecutor, execute_collect_fees_only, execute_full_close_only, auto_collect_fees, clmm-lp-execution
**paths:** `crates/execution/src/strategy/decision.rs`, `crates/execution/src/strategy/rebalance.rs`, `crates/execution/src/strategy/executor.rs`

`decide()` najpierw liczy decyzję strategii (`StaticRange` … `IlLimit`); `CollectFees` tylko gdy wynik to `Hold` i `fees_usd > min_fees_to_collect` — wcześniejszy wczesny return nie zagłusza już Periodic/OorRecenter/Threshold/RetouchShift. `execute_decision` woła `RebalanceExecutor::execute_collect_fees_only` / `execute_full_close_only` (Orca), po sukcesie lifecycle + monitor (`remove_position` po close).

---

## 2026-03-26 — Cursor rule: priorytet darmowych danych on-chain (bez płatnych zewnętrznych API)

**keywords:** cursor rules, free-onchain-data-priority, RPC, snapshots, decoded_swaps, data quality, product philosophy, no paid APIs
**paths:** `.cursor/rules/free-onchain-data-priority.mdc`

New **always-apply** rule: default design assumes **no paid external data/RPC vendors**; maximize signal from chain + local artifacts; document noise/incompleteness; prefer engineering on free inputs over buying feeds.

---

## 2026-03-26 — `swaps-enrich-curated-all`: bounded parallel `getTransaction` (M2)

**keywords:** swaps-enrich-curated-all, swap_sync, getTransaction, decode-concurrency, decode-jitter-ms, CLMM_ENRICH_DECODE_INFLIGHT, CLMM_ENRICH_DECODE_JITTER_MS, M2, B4, clmm-lp-cli, futures buffer_unordered
**paths:** `crates/cli/src/swap_sync.rs`, `crates/cli/src/main.rs`, `crates/cli/Cargo.toml`, `doc/ORCA_RUNBOOK.md`

Enrich decodes signatures with `futures::stream::buffer_unordered(decode_concurrency)` (cap 32) instead of ad-hoc `JoinSet`/`Semaphore`. New CLI flags: `--decode-concurrency` (default 4), `--decode-jitter-ms` (default 0; random delay before each decode attempt). Environment variables `CLMM_ENRICH_DECODE_INFLIGHT` and `CLMM_ENRICH_DECODE_JITTER_MS` still override when set. `decode_one_signature_with_retry` takes jitter for all paths.

---

## 2026-03-25 — Doc: work queue + phase M (M1 Meteora TVL, M2 RPC enrich queue)

**keywords:** TODO_ONCHAIN_NEXT_STEPS, ORCA_RUNBOOK, doc README, roadmap, M1, M2, B4, SOLANA_RPC_URL, Meteora, swap_sync, documentation
**paths:** `doc/TODO_ONCHAIN_NEXT_STEPS.md`, `doc/README.md`, `doc/ORCA_RUNBOOK.md`

Added *Od czego zacząć* (RPC → A1/A2 → M2 → M1 → D/E2), explicit **Faza M** checkboxes aligned with implementation plan, B4↔M2 cross-link, execution log row. README TOC points to TODO as the canonical “what to do next”. ORCA_RUNBOOK: env vars + pointer to M2 before decode params.

---

## 2026-03-25 — `optimize_apply_policy`, shared `optimization_busy`, agent JSON contract

**keywords:** optimize_apply_policy, optimization_busy, apply-optimize-result, StrategyService, AgentDecision, AgentApplyEnvelope, serde deny_unknown_fields, clmm-lp-api, clmm-lp-domain, PROJECT_OVERVIEW
**paths:** `crates/api/src/models.rs`, `crates/api/src/state.rs`, `crates/api/src/handlers/strategies.rs`, `crates/api/src/services/strategy_service.rs`, `crates/domain/src/agent_decision.rs`, `doc/PROJECT_OVERVIEW.md`

Introduced `OptimizeApplyPolicy` on `StrategyParameters` (`periodic_subprocess` | `external_http` | `combined` default): HTTP apply returns 409 when policy is subprocess-only; `external_http` + `optimize_interval_secs > 0` is rejected in `StrategyService::start_strategy`. Moved per-strategy optimize locks to `AppState.optimization_busy` so `POST /apply-optimize-result` and periodic subprocess cycles share the same `AtomicBool`; cleanup on stop/delete. `AgentDecision` and `AgentApplyEnvelope` use `#[serde(deny_unknown_fields)]` for a strict agent contract. Documented operator matrix in `PROJECT_OVERVIEW.md`.

---

## 2026-03-25 — Agent decision layer + apply-optimize HTTP + optimize JSON history

**keywords:** agent, AgentDecision, apply-optimize-result, backtest-optimize, optimize-result-json, optimize-result-json-copy-dir, StrategyExecutor, clmm-lp-api, clmm-lp-cli, clmm-lp-domain, clmm-lp-execution
**paths:** `crates/domain/src/agent_decision.rs`, `crates/execution/src/agent_decision.rs`, `crates/api/src/services/optimization_runner.rs`, `crates/api/src/handlers/strategies.rs`, `crates/cli/src/output/optimize_result_json.rs`, `crates/cli/src/main.rs`, `doc/PROJECT_OVERVIEW.md`

Added `AgentDecision` (approve/reject + optional `OptimizeResultFile`), `validate_agent_decision` with optional `agent_max_width_pct_delta` vs baseline, `POST /strategies/{id}/apply-optimize-result` applying parsed JSON without subprocess, `apply_optimize_result_parsed` shared helper, and CLI `--optimize-result-json-copy-dir` for timestamped + `latest.json` copies. Documented `StrategyService` vs HTTP + external scheduler in `PROJECT_OVERVIEW.md`.

---

## 2026-03-25 — Doc: Solana indexing concepts (`SOLANA_INDEXING.md`)

**keywords:** solana, indexing, RPC, WebSocket, Geyser, swaps-sync, clmm-lp-cli, documentation
**paths:** `doc/SOLANA_INDEXING.md`, `doc/README.md`, `doc/PROJECT_OVERVIEW.md`

Added a standalone doc describing why an SPL token does not “replicate to collect txs”, trade-offs of JSON-RPC vs subscriptions vs Geyser/providers, filtering strategies, and how that maps to the existing pull pipeline (`swaps-sync-curated-all`, `swap_sync.rs`, RPC env vars). Linked from `doc/README.md` and `PROJECT_OVERVIEW.md`.

---

<!--
Template — copy, fill, paste above the line "---" that follows the newest entry.

## YYYY-MM-DD — Short title (what you did)

**keywords:** crate-name, domain, orca|raydium|meteora, cli-flag, topic
**crates:** clmm-lp-cli, …
**paths:** `crates/.../file.rs` (optional; main touch points)

2–4 sentences: what changed, why, impact. If breaking: say **BREAKING:** explicitly.
-->

