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

### BUG-20260420-01 — Position Detail totals lost/zeroed chain-level LP collected summary

status: partially fixed  
severity: medium  
reported_by: user  
first_seen: 2026-04-20  
fixed_in: local  
keywords: position-detail, stream-lineage, chain_cost_summary, fees_collected_usd_total, ui-regression, lifecycle-ingest, pool_address, pool_pubkey

- **Symptom:** On `PositionDetail` (`Logs / rebalances`), the totals card still showed baseline/current/tx fees/cashflow/net PnL, but chain-level **LP collected** summary disappeared after the economic-vs-IL layout update.
- **Symptom (follow-up):** After restoring the field, some closed chains still showed zeros for tx/LP collected although lifecycle JSONL rows existed (`bot_collect_fees`, non-zero `tx_fee_lamports`).
- **Root cause:** (1) UI refactor replaced the old totals block and omitted the `chain_cost_summary.fees_collected_usd_total` + `collect_events_total` rendering branch. (2) DB ingest/aggregation path was brittle for lifecycle rows using `pool_address` (not `pool_pubkey`) and DB fallback triggered only when **all** node values were empty, so partial DB rows could still zero tx/collect aggregates.
- **Fix:** Restored `LP collected (sum)` in UI totals; lifecycle ingest now accepts `pool_address` as fallback for `pool_pubkey`; DB node metrics now bridge tx/collect aggregates from lifecycle JSONL when DB returns zeros for those fields.
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
- **Guards/tests:** Weryfikacja ręczna na tej samej pozycji: `Position Info` i `Monitored positions (API)` pokazują spójny status linku.
- **Paths:** `web/src/pages/Positions.tsx`, `web/src/pages/PositionDetail.tsx`

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
- **Guards/tests:** test scenariusza: close success + swap rounds + open failure/abort => wpis `rebalance_incomplete` + recovery artifact; dodatkowo testy klasyfikacji `stuck_reason` i progowego alertowania attempts.
- **Paths:** `data/ledger/orca_position_lifecycle.jsonl`, `crates/execution/src/strategy/rebalance.rs`, `crates/execution/src/strategy/pending_open.rs`, `crates/execution/src/strategy/executor.rs`, `crates/api/src/handlers/positions.rs`

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

