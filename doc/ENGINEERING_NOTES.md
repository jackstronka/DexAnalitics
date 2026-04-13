## 2026-04-13 — Stream lineage: rotation-only linking (registry + lifecycle)

keywords: api, position-stream-lineage, registry.jsonl, lifecycle, rebalance_session_id, rotation, infer_parent, strategy_service, position_stream_edges

- **What:** Position stream chain and `infer_parent_position_from_*` no longer stitch PDAs on time/pool/payer alone. Registry requires matching non-empty `rebalance_session_id` on open and close; lifecycle requires rotation signals via `lifecycle_rotation_parent_before_open` (shared session, `close_kind=rotation`, or bot rows tied to the closed PDA). Forward lifecycle links only follow opens whose inferred parent is the PDA that just closed. With Postgres edges, lineage uses **`build_lineage_chain_from_db_edges`** (walk backward from the requested PDA, then forward) instead of root-forward `build_linear_chain`, and **skips JSONL/registry fallback when any `position_stream_edges` row touches the entry** (forked graphs no longer yield `[entry]` + inflated JSONL). **`suppress_jsonl_rotation_stitch`** blocks registry/JSONL extension when the latest open is CLI `position_open`, or when no prior close in the same pool/fee payer shares the open’s `rebalance_session_id` within 60m (UI `bot_open_position` + fresh `cost_session_id` no longer inherits unrelated bot history when IL edges are missing).
- **Why:** Manual opens in the same pool were incorrectly attached to unrelated prior closes/chains; runtime logs showed `db_chain_from_edges_len: 1` with `chain_len: 4` when the DB graph forked.
- **paths:** `crates/api/src/services/position_stream_lineage.rs`, `doc/BUGS.md`

## 2026-04-13 — WSOL/USDC valuation aligns with pool tick

keywords: api, valuation, wsol, usdc, position-value, price-source, open-position

- **What:** In position USD valuation, WSOL price for WSOL/USDC pools now prefers the pool tick-implied SOL/USD value (same on-chain state used to derive token amounts) instead of relying on external feed-only price.
- **Why:** Runtime evidence showed open notional was near target, but displayed value drifted down because external SOL/USD feed diverged from pool-implied price for the same moment.
- **paths:** `crates/api/src/services/position_valuation.rs`, `doc/BUGS.md`

## 2026-04-13 — Open precheck from exact runtime simulation (+1% margin)

keywords: orca, open, precheck, simulation, insufficient-lamports, exact-plan, margin

- **What:** Replaced blocking WSOL-native heuristic in preflight with an exact-plan precheck based on `simulate_transaction` of the final signed instruction stack. If simulation logs `Transfer: insufficient lamports X, need Y`, API now returns a deterministic precheck failure using `ceil(Y*1.01)` as required native with 1% safety margin.
- **Why:** Align decisioning with actual Solana/Orca runtime path (including CPI side-effects) instead of simplified native-SOL heuristics that could over/under-block.
- **paths:** `crates/protocols/src/orca/executor.rs`, `doc/BUGS.md`

## 2026-04-13 — PositionCreate: swap suggestion now includes operational SOL reserve

keywords: ui, position-create, swap, sol, rent, fees, min-open-lamports

- **What:** `PositionCreate` funding checks now include native SOL reserve needed for open-path rent/fees. The UI computes projected native SOL after funding a WSOL leg and compares it against API `min_open_lamports` from `/wallets/api-signer`.
- **Update:** WSOL leg sufficiency now uses token balance only (without adding native SOL), matching backend validation that requires both wrapped SOL for position notional and native SOL for operational rent/fees.
- **Update:** Funding validation and displayed balances now use the API signer wallet (from `/wallets/api-signer`) instead of the locally selected UI wallet, aligning frontend gating with the actual backend transaction signer for open/swap flows.
- **Update:** Tuned default `CLMM_MIN_OPEN_SOL_LAMPORTS` to `0.012 SOL` using local historical open costs (lifecycle ledger p95/max with buffer). This threshold is now explicitly treated as operational overhead guardrail, while WSOL leg notional is validated separately.
- **Update:** `crates/protocols` WSOL open preflight now uses the same operational pad (`CLMM_MIN_OPEN_SOL_LAMPORTS`, default `0.012 SOL`) instead of a stale fixed `2_500_000` lamports, preventing false positives where preflight passed but `SystemProgram::Transfer` failed at simulation.
- **Why:** Prevents false-positive readiness where token A/B balances are sufficient but `Open Position` still fails on missing native SOL for operational costs.
- **paths:** `web/src/pages/PositionCreate.tsx`, `doc/BUGS.md`

## 2026-04-13 — Strategy executor: no stale fallback after refresh

keywords: strategy-executor, manual-close, stale-snapshot, evaluate_position, reopen, race

- **What:** `evaluate_position` no longer uses stale cached `MonitoredPosition` after refresh problems. If refresh fails, or if refresh removed the position from monitor (e.g. manual close), the executor skips this cycle.
- **Why:** Prevents unintended close/open actions on stale snapshots right after operator-triggered manual close.
- **paths:** `crates/execution/src/strategy/executor.rs`, `doc/BUGS.md`

## 2026-04-13 — UI lineage valuation quality badges (`exact` / `fallback` / `missing`)

keywords: lineage, ui, valuation_quality, position-detail, closed-position-detail, start-value, end-value

- **What:** Exposed `baseline_valuation_quality` and `current_valuation_quality` in `PositionStreamLineageNode` and wired badges next to `start value` / `end value` in both active and closed position lineage tables.
- **Why:** Operators can now distinguish trustworthy event snapshots from fallback/missing valuations without inspecting raw notes.
- **paths:** `crates/api/src/models.rs`, `crates/api/src/services/position_stream_lineage.rs`, `web/src/lib/api.ts`, `web/src/pages/PositionDetail.tsx`, `web/src/pages/ClosedPositionDetail.tsx`

## 2026-04-13 — Lineage prefers event snapshots (`baseline_open` / `end_close`)

keywords: lineage, valuation-snapshots, baseline_open, end_close, valuation_quality, start-value, end-value

- **What:** During `stream-lineage` build (DB mode), API now persists per-node event snapshots from lifecycle rows for the current chain: `baseline_open` (first open) and `end_close` (last close), including token leg amounts, USD value, `price_source`, and `valuation_quality` in `raw_json`.
- **How:** Added `persist_event_valuation_snapshots_for_positions` (live+cached mint prices) and changed snapshot selection queries to prefer `raw_json.kind = baseline_open/end_close` before generic earliest/latest rows.
- **Why:** Makes `start value` / `end value` rely on explicit open/close event snapshots instead of drifting to generic/fallback snapshots.
- **paths:** `crates/api/src/services/position_stream_lineage.rs`

## 2026-04-13 — Backtest from open positions (`POST /backtests/from-open-position`)

keywords: backtest, open-position, position-detail, clmm-lp-cli, snapshots, strategy-validation

- **What:** Added `POST /api/v1/backtests/from-open-position` mirroring the closed-position flow but anchored on `registry_open` context. API derives start date from open timestamp (or request override), end date defaults to next UTC day from now (exclusive upper bound), range from open ticks, and capital from request or lifecycle/DB fallbacks.
- **UI:** Added `Backtest (from open position)` panel on active `PositionDetail` with one-click run, job polling (`/backtests/{id}`), and stdout/stderr display.
- **paths:** `crates/api/src/handlers/backtests.rs`, `crates/api/src/models.rs`, `crates/api/src/routes.rs`, `crates/api/src/openapi.rs`, `web/src/lib/api.ts`, `web/src/pages/PositionDetail.tsx`

## 2026-04-13 — Lineage: stable mint-price fallback cache for start/end valuation

keywords: lineage, valuation, start-value, end-value, mint-prices, cache, fallback, position-stream-lineage

- **Problem:** `Logs / rebalances` per-node `start value` / `end value` could intermittently drop to `—` when short price fetches timed out; repeated refreshes for the same chain could produce different values.
- **Fix:** Added in-process mint price cache (`last good` TTL 15 min) and merged live+cache pricing path in `position_stream_lineage`: missing live quotes now reuse recent cached quotes instead of zeroing node valuation. Backfill snapshots also use the same stable price fetch path.
- **Guards/tests:** Added unit test `merge_live_prices_uses_recent_cache_for_missing_mint`.
- **paths:** `crates/api/src/services/position_stream_lineage.rs`

## 2026-04-11 — API: Base RPC + Aerodrome Slipstream `slot0` read endpoint

keywords: api, BASE_RPC_URL, evm, eth_call, slipstream, slot0, aerodrome, openapi

- **What:** `GET /api/v1/evm/base/aerodrome-slipstream/pools/{pool}/slot0` — `eth_call` `slot0()` via `BASE_RPC_URL`; JSON `SlipstreamSlot0Response` (tick, sqrtPriceX96 string, observation fields). Service `evm_json_rpc` decodes standard v3 return layout. `503` when env unset; `502` on RPC/decode failure.
- **Update:** Plan `doc/AERODROME_SLIPSTREAM_BASE_LIVE_PLAN.md` dopisany o **§0 — najpierw komunikacja, potem cięższe rzeczy**; Etap A/B w zakresie produktu; fazy 0–1 przeformułowane pod ten priorytet; rustdoc w `aerodrome_slipstream` handlerze.
- **paths:** `crates/api/src/services/evm_json_rpc.rs`, `crates/api/src/handlers/aerodrome_slipstream.rs`, `crates/api/src/routes.rs`, `crates/api/src/openapi.rs`, `crates/api/src/models.rs`, `crates/api/src/state.rs`, `crates/api/src/main.rs`, `crates/api/Cargo.toml`, `.env.example`

## 2026-04-11 — Aerodrome Slipstream Base: live deployment plan doc

keywords: aerodrome, slipstream, base, live, deployment, phases, alloy, rpc, gauges-v3, doc

- **What:** Added `doc/AERODROME_SLIPSTREAM_BASE_LIVE_PLAN.md` — phased rollout (0–5), official doc links (Slipstream README/SPEC, Base docs, Uniswap v3 reference), multi-generation `PoolFactory` warning, security/ops checklist, scope fee-only unstaked; indexed from `doc/README.md`.
- **Update:** Plan dopisany o **referencyjne pary z UI** (WETH–cbBTC, WETH–USDC, USDC–cbBTC, badge `CL100`) oraz kanoniczne adresy **WETH / USDC / cbBTC** na Base do rozwiązywania puli; przypomnienie, że procent z badge to nie „prawda fee” bez odczytu on-chain.
- **paths:** `doc/AERODROME_SLIPSTREAM_BASE_LIVE_PLAN.md`, `doc/README.md`

## 2026-04-11 — Aerodrome Slipstream (Base): pinned Gauges V3 addresses in `protocols`

keywords: aerodrome, slipstream, base, evm, gauges-v3, PoolFactory, NonfungiblePositionManager, Quoter, protocols

- **What:** Added `crates/protocols/src/aerodrome_slipstream/` with Base mainnet chain id and **Gauges V3** contract addresses from upstream Slipstream `README.md`, plus a short read-path integration checklist in module docs (unstaked vs gauge called out).
- **Why:** Single in-repo source of truth before adding an EVM RPC client (`alloy` / etc.); avoids ad-hoc copy-paste from explorers.
- **Update:** Product scope clarified as **fee z handlu only** — **unstaked** LP (NPM + pool `collect`); gauge stake / AERO emissions explicitly **out of scope** for the first integration; note on unstaked fee module vs naive Uniswap-v3 fee math in module rustdoc.
- **paths:** `crates/protocols/src/aerodrome_slipstream/mod.rs`, `crates/protocols/src/lib.rs`

## 2026-04-10 — RPC: dedupe endpoints + no rotate on definitive AccountNotFound

keywords: RpcProvider, RpcConfig, all_endpoints, SOLANA_RPC_URL, AccountNotFound, rotation

- **Problem:** Duplicate URLs in `all_endpoints()` (e.g. primary equals a fallback) made rotation log `from` == `to`. Definitive missing-account RPC errors still rotated through every node and incremented failure counters as if nodes were bad.
- **Fix:** `all_endpoints()` dedupes by URL string (first wins). `execute_with_retry` returns immediately on chain-matched `AccountNotFound` / “could not find account” without rotate/retry; other failures log `error_full` (full chain) for diagnosis.
- **paths:** `crates/protocols/src/rpc/config.rs`, `crates/protocols/src/rpc/provider.rs`

## 2026-04-10 — GET /positions/:addr — RPC errors vs missing account (502 vs 404)

keywords: get_position, monitored_position_from_chain, RpcProvider, 502, 404, PositionDetail

- **Problem:** Any `get_account` failure was mapped to HTTP 404 `Position not found: Failed to get account`, so transient RPC issues looked like a missing position; UI could not distinguish proxy/API down from on-chain absence.
- **Fix:** `map_position_fetch_error` classifies: pool-vs-position and bad layout → 400; Solana account absent (`AccountNotFound` / “could not find account”) → 404 with clearer copy; other RPC failures → 502 `Bad gateway`. Frontend `PositionDetail` adds contextual hints; `encodeURIComponent` on the path segment.
- **Update:** `GET /positions/{address}` lived on the **30s** base router; slow RPC + valuation exceeded Tower’s limit → **HTTP 408** empty body. Route moved to **on-chain** router (`API_ONCHAIN_REQUEST_TIMEOUT_SECS`, default **120s**). UI `getPosition` timeout raised to **120s**; empty-body **408** hint in `messageFromErrorBody`.
- **paths:** `crates/api/src/services/position_valuation.rs`, `crates/api/src/services/position_service.rs`, `crates/api/src/handlers/positions.rs`, `crates/api/src/routes.rs`, `web/src/lib/api.ts`, `web/src/pages/PositionDetail.tsx`

## 2026-04-10 — watchdog for stranded rebalances + dedicated Logs section

keywords: bot-activity, watchdog, rebalance_incomplete, pending-open-recovery, logs-ui, close-without-open, CLMM_STRANDED_RECONCILE_INTERVAL_SECS

- **Problem:** Rebalance sessions could end with `bot_close_position` and no matching `bot_open_position`, but detection/recovery visibility in UI was fragmented and partially heuristic.
- **Fix:** Added watchdog API endpoints: `GET /bot-activity/stranded-rebalances` (detected close-without-open sessions) and `POST /bot-activity/stranded-rebalances/reconcile` (auto-enqueue eligible sessions to pending-open queue when IL `rebalance_incomplete` provides intended ticks). Added dedicated "Urwane pozycje (watchdog)" section in `Logs` with one-click reconcile and session drill-down filter.
- **Update:** Logic lives in `stranded_rebalance_watchdog` service; API can run periodic reconcile in background when `CLMM_STRANDED_RECONCILE_INTERVAL_SECS` > 0 (requires `CLMM_IL_LEDGER_PATH`; if unset the task is a no-op per tick).
- **paths:** `crates/api/src/services/stranded_rebalance_watchdog.rs`, `crates/api/src/server.rs`, `crates/api/src/handlers/bot_activity.rs`, `crates/api/src/models.rs`, `crates/api/src/routes.rs`, `crates/api/src/openapi.rs`, `web/src/lib/api.ts`, `web/src/pages/Logs.tsx`, `.env.example`, `tools/Start-ClmmApi-8081.ps1`
- **Update:** `Start-ClmmApi-8081.ps1` hydrates `CLMM_STRANDED_RECONCILE_INTERVAL_SECS`, `CLMM_IL_LEDGER_PATH`, `CLMM_PENDING_OPEN_RECOVERY_PATH` from `.env` (when not already set) and forwards them to the `cargo run` child like signer vars.

## 2026-04-10 — position ops executor: wallet env fallback for collect/swap/close

keywords: position-executor, collect_fees, wallet, KEYPAIR_PATH, SOLANA_KEYPAIR, WALLET_KEYPAIR_BASE58

- **Problem:** API position ops could fail with `requires executor and wallet configuration` even when key material was present in env (non-path form).
- **Fix:** `load_wallet_from_env()` now checks file-path vars first, then env key material fallback (`SOLANA_KEYPAIR`, `WALLET_KEYPAIR_BASE58`) for lazy executor creation.
- **Update:** Added wallet-env diagnostics in operation errors (collect/swap/open/close) including `path_exists` hints and updated `/wallets/api-signer` guidance text for all supported signer sources. `tools/Start-ClmmApi-8081.ps1` now loads signer env vars from `.env` and forwards them explicitly to the API process.
- **Update:** `collect_fees` execution no longer hard-fails when pre-reading position `fee_owed_*` fails; it logs warning and continues harvest tx, storing authoritative LP leg raws only when pre-read data is available.
- **Update:** API collect success response now includes both collected legs in message (`token A/B`) and exposes pre/post uncollected snapshots in response `data` to make harvested amounts auditable from UI.
- **Update:** Lineage node `note` now explicitly marks `collect 1x` with `A/B=0` as a valid zero-owed collect event, reducing ambiguity between “missing data” and “executed collect with zero available fees”.
- **Update:** Added per-node `collect_zero_diagnostics` for UI panel (`why 0`): estimated in-range time share, swap events in window, and estimated position share using local fee checkpoints + lifecycle rows.
- **Update:** Collect legs now prefer Orca harvest quote (`fees_quote.fee_owed_a/b`) captured at instruction-build time, improving both-leg fidelity vs stale pre-read-only `fee_owed_*`.
- **paths:** `crates/api/src/services/position_executor.rs`, `crates/api/src/services/position_service.rs`, `crates/api/src/services/position_stream_lineage.rs`, `crates/api/src/handlers/wallets.rs`, `tools/Start-ClmmApi-8081.ps1`, `web/src/pages/PositionDetail.tsx`, `web/src/lib/api.ts`, `doc/BUGS.md`

## 2026-04-10 — lineage shadow-diff gate + tagged data catalog

keywords: lineage, shadow-diff, golden-fixture, quality-gates, data-catalog, data-tags

- **Problem:** Needed deterministic regression detection for lineage metrics and a way to avoid duplicating snapshot pipelines when equivalent data already exists.
- **Fix:** Added golden-fixture shadow diff test (`lineage_shadow_diff_matches_golden_fixture`) with expected JSON fixture and wired it into CI quality gates. Added `doc/DATA_CATALOG.md` with tagged source metadata and integrated it into AI quality checklist/rule.
- **paths:** `crates/api/src/services/position_stream_lineage.rs`, `crates/api/tests/fixtures/lineage_shadow_expected.json`, `.github/workflows/quality_gates.yml`, `doc/DATA_CATALOG.md`, `doc/AI_MERGE_CHECKLIST.md`, `.cursor/rules/ai-quality-gates.mdc`

## 2026-04-10 — regression quality gates: bug->test, CI policy, merge checklist

keywords: quality-gates, bug-regression, ci-policy, lineage-invariants, checklist, cursor-rules

- **Problem:** High/critical regressions were reappearing because fixes were not consistently coupled with tests and merge-time checks.
- **Fix:** Added CI gate `scripts/ci/critical-area-test-gate.sh` + workflow `.github/workflows/quality_gates.yml` that fails PRs when critical areas change without tests. Added mandatory AI rule `.cursor/rules/ai-quality-gates.mdc` and operator checklist `doc/AI_MERGE_CHECKLIST.md`. Added lineage regression/invariant tests in `position_stream_lineage.rs`.
- **paths:** `.github/workflows/quality_gates.yml`, `scripts/ci/critical-area-test-gate.sh`, `.cursor/rules/ai-quality-gates.mdc`, `doc/AI_MERGE_CHECKLIST.md`, `crates/api/src/services/position_stream_lineage.rs`, `doc/BUGS.md`

## 2026-04-10 — bug memory system: `doc/BUGS.md` + always-on AI rule

keywords: bugs, regression, bug-registry, cursor-rules, ai-quality, context-retention

- **Problem:** Repeated regressions and context loss across sessions/models caused the same classes of bugs to recur.
- **Fix:** Added persistent bug registry `doc/BUGS.md` with searchable `keywords` and status fields, plus mandatory always-on rule `.cursor/rules/bug-registry.mdc` requiring AI to read/update bug entries for reported issues.
- **paths:** `doc/BUGS.md`, `.cursor/rules/bug-registry.mdc`

## 2026-04-10 — API default `DRY_RUN` changed to false

keywords: DRY_RUN, AppState, collect_fees, swap-before-open, manual ops

- **Problem:** Manual actions (`collect`, `swap-before-open`) often returned `Would ...` because API defaulted to dry-run when `DRY_RUN` env was missing.
- **Fix:** `AppState` now defaults `DRY_RUN=false` when env is unset. `.env.example` updated to `DRY_RUN=false` for local dashboard/manual operation expectations.
- **paths:** `crates/api/src/state.rs`, `.env.example`

## 2026-04-10 — local startup: `Start-ClmmApi-8081.ps1` defaults to `DRY_RUN=false`

keywords: DRY_RUN, Start-ClmmApi-8081.ps1, swap-before-open, collect_fees, local dashboard

- **Problem:** UI actions (swap/collect/open) looked “broken” while API was in dry-run, returning `Would ...` messages and no tx signature.
- **Fix:** Local API start script now sets `DRY_RUN=false` by default (unless user explicitly pre-sets `DRY_RUN`), and prints effective `DRY_RUN` value in launcher/API window.
- **paths:** `tools/Start-ClmmApi-8081.ps1`

## 2026-04-10 — UX truthfulness: collect/swap show real backend outcome

keywords: collect_fees, swap-before-open, dry-run, ui feedback, positions handler

- **Problem:** UI mogło pokazać „Collect requested” albo sprawiać wrażenie „nic się nie dzieje”, mimo że backend zwrócił dry-run/no-op/info (albo brak collectable fees).
- **Fix:** `PositionDetail` pokazuje rzeczywisty `message` z API dla collect. Backend collect robi pre-check on-chain `fees_owed_{a,b}` i gdy oba = 0 zwraca jawny komunikat „No collectable fees…”, bez wysyłania tx. `PositionCreate` pokazuje `swapStepInfo` z odpowiedzi API także gdy brak signature.
- **paths:** `web/src/pages/PositionDetail.tsx`, `web/src/pages/PositionCreate.tsx`, `crates/api/src/services/position_service.rs`, `crates/api/src/handlers/positions.rs`

## 2026-04-10 — stream-lineage: session-first continuity (`close(old) -> open(new)`)

keywords: stream-lineage, rebalance_session_id, baseline_value_usd, rotation, session-first

- **Problem:** Nawet przy świeżych danych baseline/current potrafił dryfować przez mieszanie źródeł i fallbacków.
- **Fix:** Dodana reguła session-first: z lifecycle budujemy linki `old_position -> new_position` po tym samym `rebalance_session_id` i przenosimy `end(old)` jako `start(new)` dla węzłów sąsiadujących w chainie. Fallbacky działają dopiero po tej regule.
- **paths:** `crates/api/src/services/position_stream_lineage.rs`

## 2026-04-10 — stream-lineage baseline: cap/fallback guardrail vs overstatement

keywords: stream-lineage, baseline_value_usd, amount_a_cap, amount_b_cap, rotation continuity

- **Problem:** `start value` mogło być zawyżone (np. ~2x `end value`) gdy fallback brał pełne open capy albo `prev_end` dla kolejnego PDA mimo braku wiarygodnej ciągłości.
- **Fix:** Przy fallbackach baseline dodany limit wiarygodności: cap/prev-end może podnieść baseline tylko gdy nie przekracza ~135% bieżącego `current_value_usd` dla node. Chroni to przed skokami typu „dane z dupy”.
- **paths:** `crates/api/src/services/position_stream_lineage.rs`

## 2026-04-10 — PositionCreate swap-before-open: anti-underestimate for `amount_in`

keywords: PositionCreate, swap-before-open, estimateSwapInputRawExactIn, Whirlpool price, USD prices

- **Problem:** W niektórych sesjach plan SWAP wyliczał zbyt mały `amount_in` (mikroswap), więc krok „Swap” praktycznie nie pokrywał deficytu i wyglądał jak „swap nie działa”.
- **Fix:** Plan `swap_before_open` bierze teraz **większą** z dwóch estymacji: (1) USD-price based i (2) fallback z ceny puli Whirlpool (UI B-per-A), obie z +5% buforem; nadal ograniczone do ~92% dostępnego salda nogi finansującej.
- **paths:** `web/src/pages/PositionCreate.tsx`

## 2026-04-10 — strategie po restarcie: autostart bardziej niezawodny

keywords: strategies, autostart, CLMM_STRATEGY_AUTOSTART_ON_BOOT, server boot, auto_start

- **Problem:** Po restarcie API strategie z `auto_start` bywały nieaktywne, bo autostart był globalnie OFF bez env i czytał tylko `parameters.auto_start` jako strict bool.
- **Fix:** Domyślnie autostart ON (chyba że env explicite wyłącza), parsowanie `auto_start` jako bool-ish (`true/false`, `1/0`, `yes/no`) + fallback do legacy root `config.auto_start`; przy starcie strategii na boot dodany jeden retry z krótkim backoffem.
- **paths:** `crates/api/src/server.rs`

## 2026-04-10 — collect_fees lineage: prefer authoritative pair (`lp_collected_token_*_raw`)

keywords: stream-lineage, collect_fees, lp_collected_token_a_raw, lp_collected_token_b_raw, fee_owed

- **Problem:** Agregacja node LP legs łączyła źródła `max(map/meta, columns, raw)`, co mogło dawać mylący obraz dla jednej nogi przy niepełnych danych RPC.
- **Fix:** Gdy dla wiersza collect są dostępne **obie** wartości `lp_collected_token_{a,b}_raw`, lineage traktuje je jako źródło prawdy i używa tej pary bez mieszania z meta RPC; stare wiersze bez pary nadal działają fallbackowo.
- **paths:** `crates/api/src/services/position_stream_lineage.rs`

## 2026-04-10 — stream-lineage collect legs: `0` tylko gdy noga jest potwierdzona

keywords: stream-lineage, collect_fees, SOL, WSOL, unknown-vs-zero, lp_collected_token_raw

- **Problem:** Po poprzedniej zmianie UI dostawało `0` dla obu nóg nawet gdy jedna noga była nieznana (brak źródła), co wyglądało jak „wszędzie SOL=0”.
- **Fix:** Agregacja zapisuje `0` tylko gdy istnieje jawny sygnał nogi (`lp_collected_token_{a,b}_raw` obecne, także równe 0). Gdy noga nie ma żadnego dowodu w danych, API zostawia `null` i UI pokazuje `—`.
- **paths:** `crates/api/src/services/position_stream_lineage.rs`

## 2026-04-10 — stream-lineage: continuity fallback `prev end -> next baseline`

keywords: stream-lineage, baseline_value_usd, current_value_usd, rotation continuity, PositionDetail, ClosedPositionDetail

- **Problem:** Ten sam PDA mógł mieć różny `start value` między widokiem open i closed (`—` vs liczba), gdy baseline dla nowego węzła był pusty, a poprzedni zamknięty węzeł miał już sensowne `end value`.
- **Fix:** Po istniejącym fallbacku `next baseline -> prev end` dodany odwrotny fallback `prev end -> next baseline` (gdy baseline = 0), z przeliczeniem `net_pnl_*` i notą w node.
- **paths:** `crates/api/src/services/position_stream_lineage.rs`

## 2026-04-10 — stream-lineage: baseline guardrail w DB (`amount_*_cap` z open)

keywords: stream-lineage, baseline_open, position_stream_valuation_snapshots, amount_a_cap, amount_b_cap, WSOL, PositionDetail

- **Problem:** Dla aktywnych PDA `start value` potrafił być mocno zaniżony (np. 1.8 vs 4.0), gdy snapshot `baseline_open` powstał z `fee_payer_token_deltas` bez jednej nogi (często WSOL/SOL).
- **Fix:** W `node_metrics` (DB path) dodany fallback: przy podejrzanie niskim baseline czytamy pierwszy open row z `position_stream_ledger_rows.raw_json.details` i liczymy baseline z `amount_a_cap` + `amount_b_cap` po cenach free API; używamy większej z wartości.
- **paths:** `crates/api/src/services/position_stream_lineage.rs`

## 2026-04-10 — stream-lineage collect_fees: pokazuj też nogę 0 (SOL/WSOL)

keywords: stream-lineage, collect_fees, SOL, WSOL, zero-leg, PositionDetail

- **Problem:** Przy collectach jednostronnych jedna noga (często SOL/WSOL) miała realnie `0`, ale API zwracało `null` i UI pokazywał `—`, co wyglądało jak brak danych.
- **Fix:** Gdy `collect_events > 0` i znamy minty puli, API zwraca obie nogi LP (`fees_collected_token_{a,b}_ui`) także dla wartości `0` zamiast `null`.
- **paths:** `crates/api/src/services/position_stream_lineage.rs`

## 2026-04-09 — stream-lineage: stabilniejsze LP fees (cache mintów puli + `collect_events`)

keywords: stream-lineage, position_stream_lineage, get_pool_state, POOL_TOKEN_MINTS_CACHE, collect_events, public RPC

- **Cause:** Przy wielu PDA `join_all(node_metrics)` odpala wiele równoległych `get_pool_state` — publiczny RPC często timeoutuje (2s). Wiersz `bot_collect_fees` był wtedy **pomijany** w agregacji, ale licznik `collect_events` nadal brał pełną liczbę wierszy SQL → UI: „1×”, `$0`, „Brak sumy USD”, przy kolejnym odświeżeniu inne dane.
- **Fix:** Cache procesu `pool → (mint_a, mint_b)`, do 3 prób z backoffem, timeout 4s; `collect_events` = tylko wiersze po udanym rozwiązaniu puli (lifecycle + DB). Ceny mintów dla LP USD: timeout 5s. Frontend: lineage `refetchOnWindowFocus: false`, `staleTime` 60s.
- **paths:** `crates/api/src/services/position_stream_lineage.rs`, `web/src/pages/PositionDetail.tsx`

## 2026-04-09 — `collect_fees`: obie nogi LP w ledgerze (`fee_owed_*` → JSONL → Postgres ingest)

keywords: collect_fees, fee_owed, lp_collected_token_a_raw, lp_collected_token_b_raw, orca_position_lifecycle.jsonl, position_stream_ledger_rows, rebalance, tx_lifecycle, position_stream_lineage, migration 005

- **Why:** Po harvestcie meta RPC często nie pokazuje pełnej mapy tokenów (np. WSOL); potrzebne są **obie** kwoty zebrane z pozycji (token A/B puli), nie tylko `(0,0)` z executora.
- **Fix:** Przed tx odczyt `fees_owed_a` / `fees_owed_b` z konta pozycji; po sukcesie zapis do `orca_position_lifecycle.jsonl` jako `lp_collected_token_{a,b}_raw` (tylko `operation == collect_fees`). Migracja `005_ledger_lp_collected_raw.sql`; ingest `ingest_lifecycle_rows_best_effort` zapisuje kolumny w `position_stream_ledger_rows`; lineage/USD scala z tymi polami.
- **paths:** `crates/execution/src/strategy/rebalance.rs`, `crates/protocols/src/ledger/tx_lifecycle.rs`, `crates/data/migrations/005_ledger_lp_collected_raw.sql`, `crates/api/src/services/position_stream_performance.rs`, `crates/api/src/services/position_stream_lineage.rs`

## 2026-04-09 — UI lineage LP fees: rename “raw” → “baz. jedn.” + note on WSOL in `fee_payer_token_deltas`

keywords: ClosedPositionDetail, PositionDetail, stream-lineage, fee_payer_token_deltas, WSOL, UI

- **Why:** Users read `(raw …)` next to SOL as if “raw” were a token class; field is integer smallest on-chain units (lamports etc.). Orca collect credits both legs but RPC meta often omits WSOL in the mint map — one leg visible, one not.
- **Fix:** Show `(baz. jedn.: N)` only when `N` is present; tooltip explains; short copy under rotation tables about SPL-vs-WSOL in deltas.
- **paths:** `web/src/lib/utils.ts`, `web/src/pages/ClosedPositionDetail.tsx`, `web/src/pages/PositionDetail.tsx`

## 2026-04-09 — `POST /backtests/from-closed-position`: registry close day → CLI `--end-date` (+1 UTC day)

keywords: backtest, from-closed-position, snapshots.jsonl, registry, start-date, end-date, clmm-lp-api, clmm-lp-cli

- **Cause:** CLI filters Orca snapshots with `ts >= start && ts < end` where `YYYY-MM-DD` is 00:00 UTC. Registry supplied the same date for open and close → `start == end` → zero rows → “No snapshot rows in the requested time window” while the job exit code could still be 0.
- **Fix:** When `end_date` is **not** overridden in the JSON body, infer exclusive upper bound as **the calendar day after** the registry close date. Explicit `end_date` in the request is still passed through unchanged (already “exclusive” in CLI terms).
- **Also:** CLI prints a short hint when the window is empty (zero-width vs missing file data).
- **paths:** `crates/api/src/handlers/backtests.rs`, `crates/api/src/models.rs`, `crates/cli/src/main.rs`

## 2026-04-09 — `GET /positions/closed`: pagination slice aligned with newest-first sort

keywords: closed positions, registry.jsonl, list_closed_positions, pagination, clmm-lp-api

- **Bug:** After sorting closed rows newest-first, the slice used `total - offset - limit` … `total - offset`, so the default page (`offset=0`) returned the **oldest** window when `total > limit`, contradicting OpenAPI (“skip newest”).
- **Fix:** `items = closed[offset .. min(offset + limit, total)]` with `offset` clamped to `total`.
- **paths:** `crates/api/src/handlers/positions.rs`

## 2026-04-09 — strategy autostart: refuse `auto_execute` without signing keypair (align with HTTP start)

keywords: strategy, autostart, StrategyService, KEYPAIR_PATH, SOLANA_KEYPAIR_PATH, WALLET_KEYPAIR_PATH, RebalanceExecutor, clmm-lp-api

- **Bug:** `CLMM_STRATEGY_AUTOSTART_ON_BOOT` + `StrategyService::start_strategy` allowed `auto_execute=true` with no `KEYPAIR_PATH` / `SOLANA_KEYPAIR_PATH` / `WALLET_KEYPAIR_PATH` → executor started, then rebalance failed with `Wallet not set on RebalanceExecutor`. HTTP `POST /strategies/{id}/start` already returned 400 in that case.
- **Fix:** same validation in `strategy_service.rs`: `auto_execute=true` and `!dry_run` requires a loaded wallet or `bad_request` (strategy stays not running).
- **paths:** `crates/api/src/services/strategy_service.rs`

## 2026-04-09 — `POST /backtests/from-closed-position`: resolve `clmm-lp-cli` (PATH / `CLMM_LP_CLI_PATH` / `target*`)

keywords: backtest, from-closed-position, clmm-lp-cli, CLMM_LP_CLI_PATH, CLMM_REPO_ROOT, CLMM_API_TARGET_DIR, subprocess, clmm-lp-api

- **Issue:** `program not found` when API spawned `clmm-lp-cli` (Windows dev: binary not on PATH, or API uses custom `--target-dir` e.g. `target-dev-api`).
- **Fix:** resolve executable in order: `CLMM_LP_CLI_PATH` → sibling of `current_exe` → `CLMM_REPO_ROOT/target/{debug,release}` → `CLMM_REPO_ROOT/$CLMM_API_TARGET_DIR/{debug,release}` → `CARGO_TARGET_DIR/{debug,release}` → else clear stderr hint.
- **paths:** `crates/api/src/handlers/backtests.rs`, `.env.example`, `web/src/pages/ClosedPositionDetail.tsx`

## 2026-04-09 — `POST /backtests/from-closed-position`: kapitał — fallbacki (ledger open / DB snapshot) + UI `capital` z lineage

keywords: backtest, from-closed-position, capital, orca_position_lifecycle.jsonl, registry, position_stream_valuation_snapshots, ClosedPositionDetail, clmm-lp-api

- **Bug:** 400 gdy brak `rebalance_session_id` na registry albo delty w JSON jako liczby zamiast stringów — `capital` wychodził 0.
- **Fix:** parsowanie `fee_payer_token_deltas` jak string lub number; kolejność: sesja z registry → **pierwszy wiersz open** dla tego PDA → **`value_usd` pierwszego snapshotu** w DB; jeden `get_pool_state` przed wyliczeniami. Web: jeśli `stream-lineage` ma `baseline_value_usd` dla wpisu, wysyła `capital`.
- **paths:** `crates/api/src/handlers/backtests.rs`, `web/src/pages/ClosedPositionDetail.tsx`

## 2026-04-09 — stream-lineage: zebrane LP fees — noga SOL (`fee_payer_token_*_delta_ui` + ścieżka DB)

keywords: stream-lineage, bot_collect_fees, fee_payer_token_a_delta_ui, WSOL, position_stream_lineage, position_stream_ledger_rows, clmm-lp-api

- **Problem:** UI potrafiło pokazać `SOL: —` przy niezerowej sumie USD / drugiej nodze: agregacja brała tylko `fee_payer_token_deltas` (pomijała wiersze bez mapy), a **`node_metrics` z Postgresa** w ogóle nie wypełniał `fees_collected_token_*`.
- **Fix:** Na wiersz `bot_collect_fees` scalać **max(Δ z mapy po mincie, `fee_payer_token_a_delta_ui` / `b`)** w kolejności min puli; to samo z pól JSONL; zwracany `BTreeMap` napędza USD i UI; DB path ustawia token UI + raw jak ścieżka lifecycle.
- **paths:** `crates/api/src/services/position_stream_lineage.rs`

## 2026-04-09 — `stream-lineage`: avoid 120s UI timeout (ingest + SQL BFS + self-seed × chain)

keywords: stream-lineage, position_stream_edges, maybe_ingest_ledgers, node_metrics, clmm-lp-api

- **Cause:** `compute_position_stream_lineage` called `maybe_ingest_ledgers` (full JSONL scan + per-line INSERT), BFS did **one SQL query per graph node**, and each node without DB snapshots tried **on-chain self-seed** (~2s) sequentially → closed position × long chain → UI timeout.
- **Fix:** lineage calls `compute_position_stream_performance(..., skip_ledger_ingest=true)`; edges loaded **once** and BFS in memory; per-node metrics use **`skip_snapshot_self_seed`** → lifecycle JSONL when no snapshots; **parallel** `node_metrics` via `join_all`.
- **paths:** `crates/api/src/services/position_stream_performance.rs`, `crates/api/src/services/position_stream_lineage.rs`, `crates/api/src/handlers/positions.rs`, `crates/api/src/services/position_stream_pnl.rs`

## 2026-04-09 — diagnostics: auto-heal strategy link after rotation (registry parent walk)

keywords: strategy, position_addresses, reopen_hook, diagnostics, registry.jsonl, heal_rotated_strategy_link, clmm-lp-api

- **Problem:** After rebalance/rotate, `parameters.position_addresses` could still list the **closed** PDA when `reopen_hook` did not persist (bot outside API, spawn error, etc.) → UI showed no linked strategy.
- **Fix:** `GET /positions/:address/diagnostics` if no link: infer **parent** PDA via `registry.jsonl` (then lifecycle), find strategy(ies) still holding that parent, `replace_position_address_in_strategy`, sync **managed allowlist** + `executor_disabled` on running executors.
- **Also:** `replace_position_address_in_strategy` now updates `executor_disabled_position_addresses` when a PDA rotates; registry chain windows aligned to **60m**.
- **paths:** `crates/api/src/services/strategy_service.rs`, `crates/api/src/services/position_stream_lineage.rs`, `crates/api/src/handlers/positions.rs`

## 2026-04-09 — `GET /positions/closed`: fix N× RPC before pagination (UI 15s timeout)

keywords: closed-positions, registry.jsonl, list_closed_positions, WhirlpoolReader, web, api

- **Bug:** handler called `get_pool_state` for **every** closed row in registry *before* sorting/pagination, so hundreds of sequential RPCs could exceed the default **15s** browser timeout.
- **Fix:** build closed list from registry only, paginate, then resolve **unique pool addresses on that page** once each. Frontend `getClosedPositions` uses **60s** timeout as a safety margin.
- **More:** query `enrich_pools=false` skips pool RPC entirely (registry-only). Dashboard **staged** fetch (`fast` then enrich) + **prefetch** on sidebar idle/hover for instant paint.
- **paths:** `crates/api/src/handlers/positions.rs`, `web/src/lib/api.ts`, `web/src/pages/ClosedPositions.tsx`, `web/src/components/Layout.tsx`

## 2026-04-09 — backfill DB valuation snapshots from lifecycle (current free prices)

keywords: position_stream_valuation_snapshots, backfill, lifecycle, stream_pnl, closed-positions, clmm-lp-api, postgres

- Added `POST /positions/backfill-valuation-snapshots` to convert historical lifecycle open/close token deltas into **synthetic** rows in `position_stream_valuation_snapshots`.
- Rows are tagged with `price_source="lifecycle_current_prices"` and value is computed using **current free mint prices** (approximate-by-design; enables DB-backed `stream-pnl` even for old/closed positions).
- **paths:** `crates/api/src/models.rs`, `crates/api/src/handlers/positions.rs`, `crates/api/src/services/position_stream_lineage.rs`, `crates/api/src/routes.rs`

## 2026-04-09 — collected/uncollected fees: show token + raw units (avoid misleading $0)

keywords: fees, bot_collect_fees, stream_lineage, lifecycle_summary, lamports, base-units, clmm-lp-api, web

- `GET /positions/:address/stream-lineage` nodes + `chain_cost_summary` now include **collected LP fees** also as **token A/B UI units** and **raw smallest units** (`fees_collected_token_*_{ui,raw}`), in addition to `fees_collected_usd`.
- UI no longer prints `$0.0000` for collected fees when `collect_events>0` but USD valuation is unavailable (e.g. JSONL-only mode); shows `—` and the **token + raw** amounts instead.
- **Positions** list shows uncollected fees in token UI plus **raw** (`pnl.fees_earned_a/b`) for quick sanity checks.
- **paths:** `crates/api/src/models.rs`, `crates/api/src/services/position_stream_lineage.rs`, `web/src/lib/api.ts`, `web/src/pages/ClosedPositionDetail.tsx`, `web/src/pages/PositionDetail.tsx`, `web/src/pages/Positions.tsx`

## 2026-04-08 — backtest-optimize `composite`: one-sided IL drag + rebalance costs

keywords: backtest-optimize, composite, final_il_pct, total_rebalance_cost, clmm-lp-cli, main.rs

- **`--objective composite`** no longer uses `α·|IL|·capital` (which also penalized LP **better** than HODL on mark). Score is `total_fees − α·max(0, −(final_il_pct·capital)) − total_rebalance_cost` — same `final_il_pct` semantics as `run_single` (LP mark + rebalance paid vs HODL, ex fees).
- **paths:** `crates/cli/src/main.rs`, `doc/BACKTEST_OPTIMIZE_STRATEGIES.md`

## 2026-04-08 — Positions list: pair labels + mint USD instead of raw addresses

keywords: web, PositionResponse, OrcaOwnerPositionEntry, token_mint_a, token_price_a_usd, enrich_pool_ticks_for_display, clmm-lp-api

- `GET /positions` i `GET /positions/:address` zwracają opcjonalnie **`token_a_label` / `token_b_label`**, minty A/B oraz **`token_price_a_usd` / `token_price_b_usd`** (best-effort z waluacji).
- `GET /orca/positions-by-owner` uzupełnia te same pola jednym odczytem puli + `fetch_mint_prices_usd` (`enrich_pool_ticks_for_display`).
- Strona **Positions**: kolumna „Pair (mints · USD)” zamiast samego skrótu PDA; Whirlpool w osobnej kolumnie.
- **Position Details**: nagłówek + karta „Token pair” używają tych samych pól; tabele ledger (IL + lifecycle) łączą **λ + ~USD** w jednej kolumnie; skróty PDA w IL „old → new”.
- **Lifecycle timeline** (zakładka Logs): oś czasu scala **wszystkie PDA z `stream-lineage`** (`GET /bot-activity/ledger` per PDA + dedupe) oraz wiersze **IL ledger** per PDA (`il:` na osi, fiolet); sesje poniżej nadal tylko dla bieżącego adresu.
- **paths:** `crates/api/src/models.rs`, `crates/api/src/handlers/positions.rs`, `crates/api/src/handlers/orca_onchain.rs`, `crates/api/src/services/position_valuation.rs`, `web/src/pages/Positions.tsx`, `web/src/components/PoolPairLabels.tsx`, `web/src/pages/PositionDetail.tsx`, `web/src/components/PositionLifecycleTimeline.tsx`, `web/src/lib/api.ts`, `web/src/lib/utils.ts`

## 2026-04-08 — stream lineage: per-PDA vs chain totals (network tx vs LP fees collected)

keywords: stream_lineage, LineageChainCostSummary, tx_fee_lamports, fees_collected_usd, chain_cost_summary, bot_collect_fees, clmm-lp-api, web

- `GET /positions/:address/stream-lineage` nodes include **`tx_fee_lamports`**, **`fees_collected_usd`**, **`collect_events`** (LP z `bot_collect_fees`), obok istniejącego **`tx_fees_usd`**.
- Odpowiedź zawiera opcjonalnie **`chain_cost_summary`**: sumy lamportów/USD kosztów sieci oraz sumę LP fees i licznik collectów po całym łańcuchu rotacji.
- **Closed position** i zakładka **Logs / rebalances** (aktywna pozycja): dwie karty podsumowania (ten PDA vs cały łańcuch) oraz kolumny tabeli **Sieć (tx)** / **LP zebrane**.
- **paths:** `crates/api/src/models.rs`, `crates/api/src/services/position_stream_lineage.rs`, `web/src/pages/ClosedPositionDetail.tsx`, `web/src/pages/PositionDetail.tsx`, `web/src/lib/api.ts`

## 2026-04-08 — stream lineage: DB single-PDA fallback + session id for parent link

keywords: position_stream_lineage, chain_from_lifecycle_best_effort, position_stream_edges, rebalance_session_id, clmm-lp-api

- When PostgreSQL is enabled but **`position_stream_edges`** has no rows (IL ledger not ingested / empty), `GET /positions/:addr/stream-lineage` now **falls back** to the same lifecycle-JSONL chain as DB-off mode if that chain is longer.
- Lifecycle parsing records **`rebalance_session_id`** and **`pool_pubkey`** aliases; linking a child `bot_open_*` to a parent `bot_close` accepts **matching session id** (in addition to swap-mix / `bot_collect_fees` / `bot_decrease_liquidity` between close→open).
- **`position_open` / `position_close`** (CLI `orca-position-*`) are treated like **`bot_open_*` / `bot_close_position`** when building the chain and JSONL-only node metrics — previously only `bot_*` events were recognized, so mixed CLI+bot ledgers showed a single PDA.
- **Closed position** web view includes the same **Position history (rotations)** table as the active position ledger tab.
- **paths:** `crates/api/src/services/position_stream_lineage.rs`, `web/src/pages/ClosedPositionDetail.tsx`

## 2026-04-08 — swap-mix: tighter `amount_in` (fewer extra swap txs)

keywords: swap_mix, CLMM_SWAP_MIX_SPEND_CAP_PCT, CLMM_SWAP_MIX_AMOUNT_IN_BUFFER_PCT, rebalance, orca_bot, clmm-lp-execution

- `ensure_swap_mix_for_rebalance_open` no longer caps each leg at **92%** (`SPEND_CAP_PCT`); default **`CLMM_SWAP_MIX_SPEND_CAP_PCT=0.988`** with **`CLMM_SWAP_MIX_AMOUNT_IN_BUFFER_PCT=1.03`** so one `swap_exact_in` usually clears the deposit-quote deficit without a second network fee. Round **≥1** uses spend cap **≥0.998** to finish without a third tx when two steps are still needed.
- Lifecycle `bot_swap_mix_round` diagnostics include `spend_cap_pct` / `amount_in_buffer_pct`.
- **paths:** `crates/execution/src/strategy/rebalance.rs`, `.env.example`

## 2026-04-08 — lifecycle ledger: read path vs write path (enriched JSONL)

keywords: orca_position_lifecycle.jsonl, fee_payer_token_deltas, ledger_read_path, CLMM_POSITION_LIFECYCLE_USE_ENRICHED, CLMM_POSITION_LIFECYCLE_LEDGER_READ_PATH, enrich-lifecycle-ledger, clmm-lp-protocols, clmm-lp-api

- Bot/CLI **append** to the canonical file from [`ledger_path`] (`CLMM_POSITION_LIFECYCLE_LEDGER_PATH`, default `data/ledger/orca_position_lifecycle.jsonl`).
- API and other **readers** use [`ledger_read_path`]: optional explicit `CLMM_POSITION_LIFECYCLE_LEDGER_READ_PATH`, or — when `CLMM_POSITION_LIFECYCLE_USE_ENRICHED=true` — the sibling `*.enriched.jsonl` from `enrich-lifecycle-ledger` if it exists, else the canonical path.
- **paths:** `crates/protocols/src/ledger/tx_lifecycle.rs`, `crates/protocols/src/ledger/swap_cost_estimate.rs`, `crates/api/src/handlers/bot_activity.rs`, `crates/api/src/handlers/positions.rs`, `crates/api/src/services/position_stream_lineage.rs`, `crates/api/src/services/position_stream_performance.rs`, `crates/api/src/services/lifecycle_ledger_aggregates.rs`, `Makefile` target `enrich-lifecycle-ledger` → `cargo run -p clmm-lp-cli --bin enrich_lifecycle_ledger`

## 2026-04-08 — fees collected shown for closed positions

keywords: clmm-lp-api, web, lifecycle, collect_fees, pnl, ui

- `GET /positions/:address/lifecycle-summary` now returns best-effort **collected LP fees** derived from `bot_collect_fees` rows’ `fee_payer_token_deltas` (positive deltas for pool mints), plus optional USD conversion at current mint prices when the pool is unambiguous.
- Closed position detail UI displays a new **Fees collected** card (USD + A/B UI amounts).
- **paths:** `crates/api/src/models.rs`, `crates/api/src/handlers/positions.rs`, `web/src/lib/api.ts`, `web/src/pages/ClosedPositionDetail.tsx`

## 2026-04-08 — Position lineage history (rotated PDAs chain) in Position Detail

keywords: stream_lineage, position_history, position_stream_edges, rebalance, rotated_pdas, clmm-lp-api, web

- Added `GET /positions/:address/stream-lineage` returning an **ordered chain** (root → … → current) of position PDAs reconstructed from `position_stream_edges`, plus **per-node** best-effort aggregates (baseline/current valuation, tx fees, realized cashflow, net PnL).
- `Position Details → Logs / rebalances` now shows a **Position history (rotations)** table with per-PDA metrics and a totals summary (reusing stream PnL totals).
- **paths:** `crates/api/src/services/position_stream_lineage.rs`, `crates/api/src/models.rs`, `crates/api/src/handlers/positions.rs`, `crates/api/src/routes.rs`, `web/src/lib/api.ts`, `web/src/pages/PositionDetail.tsx`

## 2026-04-08 — strategy link preserved on rebalance + stable-aware sizing guard

keywords: clmm-lp-api, clmm-lp-execution, strategies, rebalance, swap-mix, sizing, usdc

- `POST /positions` now honors `strategy_id`: after a successful open, the new position PDA is appended to the strategy’s `parameters.position_addresses` so the UI does not lose the strategy link.
- Rebalance swap-mix sizing: stablecoin-leg heuristic now detects inverted UI price conventions when `A` is stable (e.g. USDC/SOL) to avoid under-sizing `target_usd` and producing tiny reopened positions.
- **paths:** `crates/api/src/services/position_service.rs`, `crates/execution/src/strategy/rebalance.rs`

## 2026-04-08 — always collect fees before close

keywords: clmm-lp-execution, orca, close, collect_fees, lifecycle, ledger

- `execute_full_close_only` now **always** submits a `collect_fees` transaction immediately before `close_position`. This makes fee earnings explicit in the lifecycle ledger and keeps “fees earned” reporting consistent for closed positions.
- **paths:** `crates/execution/src/strategy/rebalance.rs`

- Emergency close path (`emergency_close_position`) was aligned to the same policy by delegating to `execute_full_close_only`.

## 2026-04-08 — swap tx fees show up in lifecycle summaries

keywords: clmm-lp-execution, clmm-lp-api, ledger, lifecycle, swap, costs, ui

- Swap transactions executed during rebalance swap-mix now attach the current position PDA to the lifecycle JSONL row, so `/positions/:address/lifecycle-summary` can match them and the web UI shows swap `signature` + `tx_fee_lamports` instead of `—`.
- **paths:** `crates/execution/src/strategy/rebalance.rs`, `crates/execution/src/strategy/executor.rs`, `crates/api/src/services/position_service.rs`

## 2026-04-08 — strategy autostart toggle in UI

keywords: web, clmm-lp-api, strategies, autostart, ui

- Strategy Create/Edit forms now expose `parameters.auto_start` (opt-in autostart). The Strategies list shows an “auto-start on boot” badge when enabled.
- Note: autostart still requires server env `CLMM_STRATEGY_AUTOSTART_ON_BOOT=1`.
- **paths:** `web/src/pages/StrategyCreate.tsx`, `web/src/pages/StrategyEdit.tsx`, `web/src/pages/Strategies.tsx`, `web/src/lib/strategyFormShared.tsx`, `web/src/lib/api.ts`

## 2026-04-08 — Stream performance across rotated position PDAs (DB + ledger ingest)

keywords: stream_performance, position_stream_edges, position_stream_ledger_rows, rebalance_session_id, bot_collect_fees, tx_fee_lamports, clmm-lp-api, clmm-lp-data, web

- Strategies can close→open new PDAs on each rebalance, so per-PDA monitor baselines are not enough. Added a **DB-backed “stream performance”** view that stitches PDAs via IL-ledger edges and aggregates lifecycle costs/collect deltas across the connected component.
- `clmm-lp-api` now connects to Postgres via `DATABASE_URL` on boot (best-effort), runs idempotent migrations, ingests JSONL ledgers into tables, and exposes `GET /positions/:address/stream-performance`.
- Dashboard `Position Details` shows stream-level totals (known PDAs/sessions, tx fee lamports+USD, collect event counts and deltas) alongside the existing per-PDA valuation/PnL fields.
- **paths:** `crates/data/migrations/002_position_stream_performance.sql`, `crates/data/src/repositories/database.rs`, `crates/api/src/services/position_stream_performance.rs`, `crates/api/src/handlers/positions.rs`, `crates/api/src/routes.rs`, `web/src/pages/PositionDetail.tsx`, `web/src/lib/api.ts`

## 2026-04-08 — Stream Net PnL + IL (valuation snapshots + token deltas)

keywords: stream_pnl, stream_il, valuation_snapshots, fee_payer_token_deltas, preTokenBalances, postTokenBalances, clmm-lp-protocols, clmm-lp-api, clmm-lp-data, web

- Lifecycle ledger rows now include **`fee_payer_token_deltas`**: mint→Δ(ui) derived from Solana `meta.preTokenBalances/postTokenBalances` for `owner=fee_payer` (best-effort). This makes stream cashflow accounting possible without paid providers.
- API persists **valuation snapshots** (`position_stream_valuation_snapshots`) on `GET /positions/:address` so baselines survive restarts and rotated PDAs.
- Added `GET /positions/:address/stream-pnl` returning best-effort **Net PnL** and **IL (LP vs HODL)** across the stitched stream; dashboard shows these figures under Position Details.
- Valuation snapshots now persist pool leg **mints + USD prices used** (`token_mint_a/b`, `price_a_usd`, `price_b_usd`) so HODL/IL is computed deterministically without extra RPC reads.
- **paths:** `crates/protocols/src/ledger/tx_lifecycle.rs`, `crates/data/migrations/003_stream_pnl_snapshots.sql`, `crates/api/src/services/position_stream_pnl.rs`, `crates/api/src/services/position_valuation.rs`, `crates/api/src/handlers/positions.rs`, `web/src/pages/PositionDetail.tsx`

## 2026-04-08 — Orca open preflight closures compile fix

keywords: swap_exact_in, swap_mix, preflight, async closure, clmm-lp-protocols

- **`preflight_open_liquidity_balances`**: inner `async` blocks needed **`async move`** with **`Pubkey` by value** and **`&'static str`** legs so the crate compiles under Rust 2024 (previous `|...| async {` captured references incorrectly).
- **swap-mix dust**: `ensure_swap_mix_for_rebalance_open` treats tiny remaining deficits as converged using `CLMM_SWAP_MIX_DEFICIT_USD_EPS` (default **0.05 USD**) to avoid exhausting rounds on fee/rounding dust.

## 2026-04-07 — Rebalance swap-mix + `swap_exact_in`: native SOL → wSOL (parity with open position)

keywords: rebalance, swap_mix, swap_exact_in, wsol, wrap, sync_native, Orca, clmm-lp-protocols, clmm-lp-execution

- Orca **ExactIn** debits the **wSOL SPL** ATA; **native SOL** in the fee payer does not count. `open_position` already used `ensure_wsol_ata_funded`; **`swap_exact_in` did not**, so swaps that specify wSOL could fail while manual/UI paths wrapped.
- `WhirlpoolExecutor::swap_exact_in` now prepends the same WSOL ATA fund + `sync_native` ix when `specified_mint` is wSOL.
- `ensure_swap_mix_for_rebalance_open`: if token A is wSOL, SPL `wa == 0`, but native SOL can fund a wrap, the executor runs **`submit_wsol_wrap_if_needed`** then `continue` so the next round sees wSOL in `wa` before A→B sizing.
- **paths:** `crates/protocols/src/orca/executor.rs`, `crates/execution/src/strategy/rebalance.rs`

## 2026-04-07 — PositionCreate budget mode: leg-edit abort vs „cannot match”, immediate quote sync

keywords: PositionCreate, quote-open-budget, budget, AbortSignal, UX, web

- Edycja Amount A/B przerywa poprzednie `solveTargetUsdForLegAmount`; gdy solver zwracał `null` po **abort**, UI pokazywał mylący komunikat „Nie można dopasować…” przy jednocześnie **starym** polu docelowego USD — teraz abort jest cichy.
- Po udanym dopasowaniu kwoty SOL/USDC i **Docelowa wartość pozycji** ustawiane są od razu z tego samego `quote` (wraz z `budgetSubmitRaw`), żeby suma szacunków USD i pole USD pozostały zsynchronizowane.
- **paths:** `web/src/pages/PositionCreate.tsx`

## 2026-04-07 — Orca open: auto-wrap native SOL into WSOL ATA (fix Tokenkeg `InsufficientFunds`)

keywords: orca, open_position, wsol, wrap, associated-token-account, Tokenkeg, InsufficientFunds, clmm-lp-protocols

- When opening a position in a pool where one leg is **WSOL**, the Orca SDK debits the **WSOL token account**. Operators often have native SOL but **0 WSOL**, causing `SPL Token InsufficientFunds (custom 0x1)`.
- The executor now preprends instructions to create/fund the WSOL ATA (and `sync_native`) up to the required cap before calling the Orca open instructions.
- **paths:** `crates/protocols/src/orca/executor.rs`

## 2026-04-07 — Orca open: SPL preflight + WSOL wrap buffer (clearer than on-chain `InsufficientFunds`)

keywords: orca, open_position, preflight, USDC, WSOL, InsufficientFunds, clmm-lp-protocols

- **Preflight** (RPC, before building the tx): for each pool leg, verify the **API signing wallet** has enough **raw SPL balance** for `amount_a` / `amount_b` (USDC etc.); WSOL legs check wrapped balance + native SOL for wrap + fee pad.
- WSOL auto-wrap now targets **`token_max` + small buffer** (50 bps + 50k lamports) to reduce edge-case `InsufficientFunds` after `SyncNative`.
- **paths:** `crates/protocols/src/orca/executor.rs`

## 2026-04-08 — Guardrail: strategy manages fixed position set (no “2→10” explosion) + reopen auto-relink

keywords: StrategyExecutor, guardrail, position_addresses, reopen, auto-link, strategies.json, registry.jsonl, clmm-lp-api, clmm-lp-execution

- Strategy execution now supports a **managed allowlist**: on start, the API derives the set of managed PDAs from `registry.jsonl` (currently open) and `parameters.position_addresses` (if present). The executor **only evaluates** these PDAs.
- On successful close→open, the executor **replaces** `old_pda → new_pda` inside the allowlist, keeping the managed set size constant (prevents accidental growth from stale/historical PDAs in monitor).
- API also installs a best-effort reopen hook to update `data/strategies.json` by replacing `old_pda → new_pda` in the same strategy’s `position_addresses` so the UI stays linked without manual steps.
- **paths:** `crates/execution/src/strategy/executor.rs`, `crates/api/src/services/strategy_service.rs`

## 2026-04-07 — Logs: lifecycle table shows `position_pubkey`, newest-first per page, hint for `bot_close_position`

keywords: Logs, Lifecycle ledger, position_pubkey, bot_close_position, web, JsonlTable

- Kolumna pozycji używała klucza `position_pda`, podczas gdy ledger zapisuje `position_pubkey` — w UI było „—”. `JsonlTable` ma opcjonalny `getCellValue`; lifecycle łączy `position_pubkey ?? position_pda`.
- W obrębie zwróconej strony wiersze są wyświetlane **od najnowszych** (odwrócenie względem kolejności w pliku).
- Krótka podpowiedź w nagłówku: zamknięcie = `event: bot_close_position`.
- **paths:** `web/src/pages/Logs.tsx`

## 2026-04-07 — Open Position UI: budget mode — editing Amount A/B updates target USD + other leg

keywords: PositionCreate, quote-open-budget, budget, UX, web

- In **Wspólna kwota USD** mode, changing **Amount token A or B** (debounced) runs a **binary search on `target_usd`** against `POST /pools/:id/quote-open-budget` so the chosen leg matches the typed UI amount; then **Docelowa wartość pozycji** and the **other leg** refresh from the same Orca deposit quote.
- **paths:** `web/src/pages/PositionCreate.tsx`

## 2026-04-07 — Lifecycle ledger: `bot_swap_exact_in` rows include structured swap `details`

keywords: bot_swap_exact_in, lifecycle, orca_position_lifecycle.jsonl, swap, ledger, clmm-lp-protocols, clmm-lp-execution

- Confirmed swap rows (`event: bot_swap_exact_in`) now include optional JSON **`details`**: pool + `token_mint_a` / `token_mint_b`, `specified_mint`, `other_mint_expected_output`, `amount_in_raw`, `specified_mint_decimals`, `amount_in_ui`, `slippage_bps` (min/actual out still not on `ExecutionResult`; see `note` in payload).
- **`paths:**` `crates/protocols/src/ledger/tx_lifecycle.rs`, `crates/execution/src/strategy/rebalance.rs`

## 2026-04-07 — Pending-open recovery enabled by default + richer tx error context + slippage env override

keywords: pending_open, recovery, clmm-pending-open, swap_exact_in, transaction error, ix_programs, slippage, orca, clmm-lp-execution, clmm-lp-protocols

- `StrategyExecutor` now enables pending-open recovery by default using `data/pending-open-recovery.json` (unless overridden via `CLMM_PENDING_OPEN_RECOVERY_PATH`), so “close succeeded but reopen failed” can auto-retry on the next cycles without extra env wiring.
- `RpcProvider::send_and_confirm_transaction` now enriches **confirmed tx failures** (`TransactionError::InstructionError(...)`) with `ix_programs`, `ix_program`, and `custom_code` to make `Custom(1)`-style errors actionable without server logs.
- Added `CLMM_REBALANCE_MAX_SLIPPAGE_BPS` to override `RebalanceConfig.max_slippage_bps` (default remains 50 bps).

## 2026-04-07 — API: strategy autostart on boot (opt-in)

keywords: clmm-lp-api, strategy, executor, autostart, production

- Added opt-in strategy autostart on API boot: when `CLMM_STRATEGY_AUTOSTART_ON_BOOT=true`, the API will automatically start strategies whose stored `parameters.auto_start=true`.
- This keeps “running” behavior consistent after an API restart without auto-starting everything by default.

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
## 2026-04-07 — Guardrail: no-close unless reopen feasible + auto-widen ticks; richer swap-mix amount logs

**keywords:** no_close_unless_reopen_feasible, reopen_preflight, auto_widen_ticks, swap_mix, deposit_quote, bot_reopen_preflight_failed, bot_reopen_widen_ticks, bot_swap_mix_round, orca, clmm-lp-execution
**paths:** `crates/execution/src/strategy/rebalance.rs`

- Added a preflight check before `close_position`: if a deposit quote fails for the planned tick range with current wallet balances, the bot **skips closing** to avoid leaving the operator with no position.
- Optional auto-widen: on quote failure, the bot expands tick width around `tick_current` for a few steps and retries the quote; all attempts are logged.
- Swap-mix planning logs now include proposed swap amounts in UI + USD-estimates, plus wallet balances and deficits in UI.

## 2026-04-07 — Swap-mix diagnostics: log per-swap attempt/result and failure context

**keywords:** swap_mix, swap_exact_in, bot_swap_exact_in_attempt, bot_swap_exact_in_submitted, bot_swap_exact_in_failed, bot_swap_mix_failed, lifecycle ledger, orca, clmm-lp-execution, clmm-lp-protocols
**paths:** `crates/execution/src/strategy/rebalance.rs`

- Added diagnostic lifecycle rows so swap-mix failures can be explained without guessing:
  - `bot_swap_exact_in_attempt`: mint + amount_in + slippage + round/leg
  - `bot_swap_exact_in_submitted`: signature + same context
  - `bot_swap_exact_in_failed`: full error string + same context
- `bot_swap_mix_failed` now includes `last_round` snapshot (balances, deficits, target_usd) to distinguish “tx failed” vs “target not reachable with wallet notional”.

## 2026-04-07 — Perf note: scaling live execution to 100+ positions (cache/batch/RPC budgets) — TODO later

**keywords:** performance, scaling, rpc, caching, get_multiple_accounts, scheduler, StrategyExecutor, WhirlpoolReader, clmm-lp-execution, clmm-lp-protocols, solana
**paths:** `crates/execution/src/strategy/executor.rs`, `crates/protocols/src/orca/pool_reader.rs`, `crates/protocols/src/rpc/provider.rs`

- **Motivation:** we may run **100+ active positions/strategies** with decision cadence \(30–300s\). With public RPC limits, the risk is **duplicated per-position reads** (same pool state fetched N times) and bursty simulate/quote calls.
- **Current state (as of today):**
  - `RpcProvider` has retry/failover/backoff but **no TTL cache** for `get_account` / `get_multiple_accounts`.
  - `StrategyExecutor::evaluate_all` loops positions and calls `WhirlpoolReader::get_pool_state` **per position**; `WhirlpoolReader::get_multiple_pools` exists but is not used in the hot path.
  - Websocket-driven `AccountListener` exists but is currently **placeholder / not production feed**, so it does not serve as an event-driven cache.
- **Prep work we should do before scaling** (postponed):
  - Add **shared per-pool cache** (TTL / slot-aware) for `WhirlpoolState` and tick boundary reads; dedupe concurrent fetches.
  - Update executor hot path to **group positions by pool** and refresh each pool once per cycle (or use `get_multiple_pools`).
  - Introduce an explicit **RPC budget** (RPS cap + queue/backpressure) and prioritize evaluations (near range edge, high TVL, etc.).
  - Keep `simulate_transaction` / heavy quotes **alarm-driven**, not unconditional per loop.

## 2026-04-07 — Strategy executor: OOR rebalance is scheduled by default (no immediate close+open)

**keywords:** strategy executor, DecisionConfig, periodic, oor_recenter, threshold, retouch_shift, range_exit, rebalance, clmm-lp-api, clmm-lp-execution
**paths:** `crates/execution/src/strategy/decision.rs`, `crates/api/src/models.rs`, `crates/api/src/services/strategy_service.rs`, `crates/api/src/handlers/strategies.rs`, `doc/BACKTEST_OPTIMIZE_STRATEGIES.md`

- **Change:** out-of-range (OOR) no longer triggers an immediate rebalance by default for `OorRecenter` / OOR branch of `Threshold` / `RetouchShift`. Instead, it waits for `min_rebalance_interval_hours` unless `rebalance_on_range_exit_immediately=true`.
- **Periodic:** `Periodic` now defaults to running only when OOR (`periodic_requires_out_of_range=true`) to match “rebalance every N hours *if* position is outside range”; set it to `false` for the previous “always on the timer” behavior.

## 2026-04-07 — Revert defaults: restore legacy immediate OOR and periodic timer behavior

**keywords:** strategy executor, DecisionConfig, periodic, oor_recenter, threshold, retouch_shift, defaults, UI, web
**paths:** `crates/execution/src/strategy/decision.rs`, `web/src/pages/StrategyCreate.tsx`, `web/src/pages/StrategyEdit.tsx`, `web/src/lib/strategyFormShared.tsx`, `web/src/lib/api.ts`

- Restored **legacy defaults**: `rebalance_on_range_exit_immediately=true` and `periodic_requires_out_of_range=false`.
- Exposed both toggles in the Strategy UI (create/edit) so operators can opt into the “scheduled-only” semantics when desired.

## 2026-04-08 — `backtest-optimize`: siatka last-candle ze świecą 45m (@ 900s/krok)

**keywords:** backtest_optimize, LAST_CANDLE_OPTIMIZE_GRID, last_candle, resolution_seconds
**paths:** `crates/cli/src/commands/backtest_optimize.rs`

- Do `LAST_CANDLE_OPTIMIZE_GRID` dopisano wiersze **`(3, *)`** — przy `--resolution-seconds 900` to **45m** świeca (min–max po 3 krokach), z presetami rebalansu `1|2|3|4|16|48` kroków (jak dla 15m/30m).

## 2026-04-04 — `.env.example`: Orca CLI + `SOLANA_RPC_FALLBACK_URLS`; `make cli-release`

**keywords:** .env.example, SOLANA_RPC_FALLBACK_URLS, CLMM_EXPECTED_CLUSTER, cli-release, Makefile, orca-bot-run
**paths:** `.env.example`, `Makefile`, `doc/MAINNET_OPERATIONAL_CHECKLIST.md`, `doc/PRODUCTION_FAST_PATH.md`

- **RPC:** przykładowa zmienna **`SOLANA_RPC_BACKUP_URLS`** zastąpiona przez **`SOLANA_RPC_FALLBACK_URLS`** (zgodnie z `RpcConfig`); dopisany komentarz o legacy.
- **Orca bot:** sekcja z `CLMM_ALERT_WEBHOOK_URL`, pending recovery, profitability (szablon, zakomentowane).
- **Build:** target **`make cli-release`** = `cargo build --release -p clmm-lp-cli`. Checklist mainnet linkuje do fast path.

## 2026-04-08 — Backtest `run_single`: estymacja L po rebalance z aktualnym quote (cross-pair)

**keywords:** backtest_engine, estimate_position_liquidity, LiquidityEstimateOverrides, rebalance, snapshot fees, fee share
**paths:** `crates/cli/src/backtest_engine.rs`, `crates/cli/src/engine/fees.rs`, `crates/cli/src/engine/tests.rs`

- **Błąd wzoru:** po rebalance granice USD były liczone jako `lower_ab * p.quote_usd`, ale `estimate_position_liquidity` (bez override) dzieliło je przez **`quote_usd` z pierwszego kroku** → zniekształcone `lower_ab`/`upper_ab` i `L`, gdy zmienia się kurs quote (np. SOL/USD). To mogło dawać absurdalne `final_value`, `vs_hodl` i udział fee względem `liquidity_active_raw`.
- **Poprawka:** po rebalance wywołanie `estimate_position_liquidity_with_overrides` z `quote_usd`, `price_ab`, `price_a_usd` z **bieżącego** kroku.
- **Fee share:** `position_liquidity / pool_liquidity` ograniczone do **≤ 1** (dynamiczna gałąź snapshot + `FeeShareModel::LiquidityShare`), żeby nie przekraczać „pool fees” kroku przy błędach skali.
- Test `snapshot_pool_fee_dynamic_liquidity_active_scales_fees`: większe `liquidity_active_raw` w fixture (żeby `L_pos` < `L_pool` i nie wchodzić w clamp w teście).

## 2026-04-04 — Runbook: najkrótsza ścieżka do live bota (`PRODUCTION_FAST_PATH`)

**keywords:** PRODUCTION_FAST_PATH, orca-bot-run, SOLANA_RPC_URL, CLMM_ALERT_WEBHOOK_URL, CLMM_PENDING_OPEN_RECOVERY_PATH, doc
**paths:** `doc/PRODUCTION_FAST_PATH.md`, `doc/README.md`, `STARTUP.md`

- Nowy dokument: kolejność **dry-run (bez `--execute`) → limited live → `--execute`**, tabela env (RPC, klaster, webhook, pending recovery, profitability), linki do checklist mainnet i continuity. Indeks `doc/README.md` + jedna linia w `STARTUP.md`.

## 2026-04-08 — `last_candle`: zakres LP = min–max ceny w świecy (nie close ± width)

**keywords:** last_candle, last_closed_candle_step_range, backtest_engine, StratConfig, BACKTEST_OPTIMIZE_STRATEGIES
**paths:** `crates/cli/src/backtest_engine.rs`, `crates/cli/src/engine/indicators.rs`, `doc/BACKTEST_OPTIMIZE_STRATEGIES.md`

- Po rebalance granice A/B = **minimum i maksimum `price_ab`** na ostatniej zamkniętej świecy (`candle_steps` kroków). Przy min=max lub braku zamkniętej świecy: fallback **±`width_pct`** wokół ceny. `last_closed_candle_step_range` w `indicators.rs`; test `last_closed_candle_step_range_matches_candle`.

## 2026-04-04 — Bot LP: bramka opłacalności, recovery `open`, alerty, emergency, optimize→live (bollinger/last_candle)

**keywords:** RebalanceProfitabilityMode, CLMM_REBALANCE_PROFITABILITY, CLMM_PENDING_OPEN_RECOVERY_PATH, CLMM_ALERT_WEBHOOK_URL, pending_open, RecoverOpenParams, EmergencyExitManager, WebhookNotifier, optimize_profile, StrategyExecutor, orca_bot
**paths:** `crates/execution/src/strategy/rebalance.rs`, `crates/execution/src/strategy/executor.rs`, `crates/execution/src/strategy/pending_open.rs`, `crates/execution/src/monitor/position_monitor.rs`, `crates/execution/src/emergency/emergency_exit.rs`, `crates/execution/src/optimize_profile.rs`, `crates/cli/src/commands/orca_bot.rs`

- **Opłacalność:** `RebalanceConfig::from_env()` — `CLMM_REBALANCE_PROFITABILITY=off|warn|block`, `CLMM_REBALANCE_EST_TX_COST_LAMPORTS` (domyślnie 500_000). Po `dry_run` sprawdzana jest heurystyka `is_profitable`; `block` przerywa rebalance z komunikatem.
- **Recovery po incomplete:** plik JSON (`CLMM_PENDING_OPEN_RECOVERY_PATH` lub domyślnie `data/pending-open-recovery.json` w `orca-bot` z `--execute`), wpis po `rebalance_incomplete`, ponawianie `recover_open_after_incomplete` na początku `evaluate_all` (do `CLMM_PENDING_OPEN_MAX_ATTEMPTS`). `Arc<RebalanceExecutor>` + `open_new_range_with_wallet_mix` (wspólne z pełnym rebalance).
- **Alerty:** `CLMM_ALERT_WEBHOOK_URL` → `MultiNotifier` + `WebhookNotifier` (prawdziwy POST JSON); monitor: range exit + progi IL; nowy `AlertType::RebalanceIncomplete` przy incomplete.
- **Emergency:** `EmergencyExitManager::new_with_rebalance` wywołuje Orcę przez `RebalanceExecutor::{emergency_collect_fees, emergency_decrease_all_liquidity, emergency_close_position}`; `StrategyExecutor::rebalance_executor_handle()` do podpięcia.
- **Optimize JSON:** `bollinger` → `Threshold` (fallback `threshold_ratio` lub `width_pct*0.5`); `last_candle` → `OorRecenter`.
- **Async:** `tokio::sync::Mutex` dla `optimization_run_id` / pending path / alert notifier w executorze (Send w `tokio::spawn`).

---
## 2026-04-04 — Rebalance: ustrukturyzowane logi błędów (`op = orca_rebalance`)

**keywords:** tracing, orca_rebalance, RebalanceExecutor, StrategyExecutor, swap_mix, open_position, close_position
**paths:** `crates/execution/src/strategy/rebalance.rs`, `crates/execution/src/strategy/executor.rs`

- Pola **`op = "orca_rebalance"`** + **`stage`** (`start`, `close_position`, `swap_mix`, `open_position`, `collect_fees`) oraz kontekst (pool, ticks, caps, `reason`) ułatwiają **grep** w logach. Niepowodzenia **`swap_mix`** i **`open`** logują szczegóły przed `bail`; executor przy **`outcome: incomplete` / `failed`** dopisuje stare/nowe ticki i `tick_current`.

---
## 2026-04-07 — Rebalance: swap w puli przed `open` przy złym mixie (quote deposit)

**keywords:** RebalanceExecutor, ensure_swap_mix_for_rebalance_open, quote_deposit_budget_in_range, swap_exact_in, CLMM_REBALANCE_SWAP_MAX_ROUNDS, orca_bot
**paths:** `crates/execution/src/strategy/rebalance.rs`

- Po `close` wykonywane jest **wyrównanie mixu** do `quote_deposit_budget_in_range` (`crates/protocols/src/orca/deposit_quote.rs`): ceny względne z `pool.price` + decimals mintów (bez płatnego feedu), `target_usd` ≈ 99.5% notional portfela, pętla do **`CLMM_REBALANCE_SWAP_MAX_ROUNDS`** (domyślnie 6) z **ExactIn** na tej samej puli (B→A lub A→B, połowa nadwyżki na krok). Potem istniejące ponawianie **`open_position`**.
- Nadal możliwe błędy: tick poza nowym zakresem, całkowity brak jednej nogi do swapu, wyczerpanie rund.

---
## 2026-04-07 — IL JSONL: `rebalance_incomplete` gdy zamknięcie OK, open nie

**keywords:** LifecycleTracker, rebalance_incomplete, IL ledger, StrategyExecutor, orca_bot, CLMM_REBALANCE_OPEN_MAX_ATTEMPTS
**paths:** `crates/execution/src/lifecycle/tracker.rs`, `crates/execution/src/strategy/executor.rs`, `crates/execution/src/strategy/rebalance.rs`

- Po nieudanym `open_position` po udanym `close` wiersz `event: "rebalance"` **nie** powstaje (wcześniej tylko `tracing` + `result.error`). Teraz — jeśli ustawiono `--il-ledger-path` / `set_il_ledger_path` — dopisywany jest **`event: "rebalance_incomplete"`** z `intended_tick_*`, `error`, `reason`, `hint` (retry: swap + open / quote caps).
- **Automatyczne ponawianie open:** `RebalanceExecutor::execute` ponawia **`open_position`** do **`CLMM_REBALANCE_OPEN_MAX_ATTEMPTS`** (1..=20, domyślnie **5**) z krótkim backoffiem i **ponownym odczytem SPL** po każdej próbie (opóźnienia RPC po zamknięciu). Nie rozwiązuje to złego **stosunku tokenów** do nowego zakresu — wtedy nadal może być swap przed open.

---
## 2026-04-07 — Orca bot `IlLimit`: OOR recenter przed zamknięciem na IL (rebalans zamiast samego `Close`)

**keywords:** StrategyExecutor, DecisionEngine, IlLimit, il_close_threshold, OOR, rebalance, orca_bot
**paths:** `crates/execution/src/strategy/decision.rs`

- **Problem:** W `IlLimit` najpierw sprawdzany był `il_close_threshold`; po wyjściu z zakresu IL często przekracza próg → `Decision::Close` (`execute_full_close_only`) zamiast `Rebalance` → w logu `bot_close_position` **bez** `bot_open_position`.
- **Zmiana:** Kolejność: (1) **OOR** + `hours_since_rebalance >= min_rebalance_interval_hours` → **Rebalance**; (2) IL powyżej **close** → **Close**; (3) IL powyżej **rebalance** (in-range / bez spełnionego OOR+cooldown) → **Rebalance**.
- **Uwaga:** Jeśli **OOR** i min. odstępu **nie** minął, nadal możliwe **Close** przy `|IL| > il_close_threshold`. Gdy **Rebalance** zamknie starą pozycję, a **open** padnie (mix tokenów), log: `Rebalance incomplete: old position closed on-chain but new position was not opened`.

---
## 2026-04-07 — Tabela `backtest-optimize`: vsHODL% vs IL-like% (była „Drag%”)

**keywords:** backtest-optimize, optimize_report, TrackerSummary, vs_hodl, final_il_pct, TIR
**paths:** `crates/cli/src/output/optimize_report.rs`, `crates/cli/src/main.rs`

- Ostatnia kolumna rankingu nie była „dragiem” opłat — to **IL-like** (`final_il_pct`): (LP końcowe + koszty rebalance − HODL) / kapitał, **bez** zaliczenia fee do wartości pozycji. Dodano **`vsHODL%`** = `vs_hodl / capital`, spójne z kolumną „vs HODL” (USD). Legenda drukuje się pod tabelą.

## 2026-04-07 — `backtest-optimize`: rozszerzona siatka last-candle (świeca × rebalans)

**keywords:** backtest-optimize, indicator-strategies, last_candle, last_closed_candle, rebalance_steps, clmm-lp-cli
**paths:** `crates/cli/src/commands/backtest_optimize.rs`, `doc/BACKTEST_OPTIMIZE_STRATEGIES.md`

- Zamiast dwóch wariantów `LastCandle` jest **14** par `(candle_steps, rebalance_steps)` w `LAST_CANDLE_OPTIMIZE_GRID` (15m / 30m / 1h świeca w krokach przy założeniu 15 min/krok, oraz zestawy rebalansów 15m…12h jak w specyfikacji). Koszt transakcji rebalansu pozostaje w `run_single` (`tx_cost` na każdy rebalance).

## 2026-04-06 — `backtest-optimize --indicator-strategies`: trzy szerokości Bollingera (K)

**keywords:** backtest-optimize, indicator-strategies, bollinger, StratConfig, clmm-lp-cli
**paths:** `crates/cli/src/commands/backtest_optimize.rs`, `doc/BACKTEST_OPTIMIZE_STRATEGIES.md`

- Siatka z `--indicator-strategies` zawiera teraz **6** wariantów Bollingera zamiast 2: to samo `window=20`, **`k` ∈ {1.5, 2.0, 2.5}** (węższe / klasyczne / szersze pasma w jednostkach σ), każde z **`rebalance_steps` 24 i 48** (kroki symulacji — przy `--resolution-seconds 900` to odpowiednio co 6 h i 12 h między rebalansami).

## 2026-04-06 — API signer: osobny próg SOL dla swap vs open

**keywords:** api-signer, CLMM_MIN_SWAP_SOL_LAMPORTS, CLMM_MIN_OPEN_SOL_LAMPORTS, swap, lamports, clmm-lp-api, web
**paths:** `crates/api/src/handlers/wallets.rs`, `crates/api/src/models.rs`, `web/src/lib/api.ts`, `web/src/pages/Swap.tsx`

- `GET /wallets/api-signer` zwraca `min_open_lamports` (jak dotąd: domyślnie 0.01 SOL, env `CLMM_MIN_OPEN_SOL_LAMPORTS`) oraz `min_swap_lamports` (niższy próg tylko pod opłaty swapu; domyślnie 1_500_000 lamportów ≈ 0.0015 SOL, env `CLMM_MIN_SWAP_SOL_LAMPORTS`).
- UI `Swap` używa `min_swap_lamports` do ostrzeżenia i blokady „Swap now”; open nadal chroniony osobnym progiem po stronie API.

## 2026-04-06 — Open Position: guardrail na minimalne SOL + czytelniejsze błędy w UI

**keywords:** open_position, rent, lamports, insufficient, ui, clmm-lp-api, clmm-lp-protocols, web
**paths:** `crates/api/src/services/position_service.rs`, `crates/protocols/src/rpc/provider.rs`, `crates/execution/src/strategy/executor.rs`, `web/src/pages/PositionCreate.tsx`

- API: przed `open_position` sprawdzamy saldo SOL walleta podpisującego i blokujemy request, jeśli jest poniżej progu (domyślnie \(0.01\) SOL; env: `CLMM_MIN_OPEN_SOL_LAMPORTS`).
- RPC: przy błędach preflight symulacji doklejamy `logs` + mapę `ix_programs`, żeby jednoznacznie wskazać przyczynę (np. brak lamportów na rent).
- Web: błąd `Open Position` nie jest już “cichy” i jest skracany z opcją “Show details”, żeby nie zalewać UI ścianą tekstu.

## 2026-04-06 — Wycena pozycji: poprawka ilości token A (uniknięcie „~połowa wartości”)

**keywords:** valuation, calculate_token_amounts, amount_a, inverse floor, U256, half value, clmm-lp-protocols, clmm-lp-api
**paths:** `crates/protocols/src/orca/position_reader.rs`, `crates/api/src/services/position_valuation.rs`

- `PositionReader::calculate_token_amounts` używa teraz dokładnej formuły U256 dla token A (`L*(√Pu-√P)*2^64/(√P*√Pu)`), bo wcześniejsze `floor(2^64/√P)` potrafiło zaniżać amount A i w UI wyglądało jak ~połowa wartości pozycji.

## 2026-04-06 — Swap (Orca): pokaz stanu walleta API + blokada przy zbyt małym SOL

**keywords:** swap, orca, api-signer, KEYPAIR_PATH, wallet, lamports, ui, clmm-lp-api, web
**paths:** `crates/api/src/handlers/wallets.rs`, `crates/api/src/routes.rs`, `crates/api/src/models.rs`, `web/src/lib/api.ts`, `web/src/pages/Swap.tsx`

- Dodano `GET /wallets/api-signer` zwracające pubkey walleta podpisującego na hoście API (env `KEYPAIR_PATH`/`SOLANA_KEYPAIR_PATH`) + saldo SOL i próg `CLMM_MIN_OPEN_SOL_LAMPORTS`.
- UI `Swap` pokazuje ten stan i blokuje “Swap now (Orca pool)” gdy wallet nie jest skonfigurowany albo SOL < próg.

## 2026-04-06 — Rebalance: `open` po `close` z realnymi capami SPL + sensowne `hours_since_rebalance` w snapshot

**keywords:** rebalance, RebalanceExecutor, open_position, close_position, u64::MAX, ATA, diagnostics, hours_since_rebalance
**paths:** `crates/execution/src/strategy/rebalance.rs`, `crates/execution/src/strategy/executor.rs`, `crates/api/src/models.rs`, `web/src/pages/PositionDetail.tsx`, `web/src/lib/api.ts`

- Po udanym zamknięciu pozycji **nie** używamy już `token_max_a/b = u64::MAX` przy otwarciu nowej — zamiast tego bierzemy **salda SPL z ATA** właściciela po `close` (fallback: kwoty z LP sprzed close). To usuwa scenariusz „zamknięte on-chain, ale `open` nie przechodzi” oraz przygotowuje grunt pod przyszły swap (gdy nowy zakres wymaga innego mixu tokenów).
- Snapshot `last_eval.hours_since_rebalance`: `None` zamiast `u64::MAX` gdy brak wpisu w lifecycle (nie mylić z gigantyczną liczbą w UI).

## 2026-04-06 — Diagnostyka rebalansu per pozycja + świeże `in_range`

**keywords:** positions, diagnostics, in_range, strategy executor, auto_execute, periodic, oor_recenter, clmm-lp-api, clmm-lp-execution, web
**paths:** `crates/api/src/handlers/positions.rs`, `crates/api/src/models.rs`, `crates/api/src/routes.rs`, `crates/api/src/openapi.rs`, `crates/execution/src/strategy/executor.rs`, `web/src/pages/PositionDetail.tsx`, `web/src/lib/api.ts`

- `GET /positions/:address` i lista pozycji zwracają `in_range` liczone z aktualnego `pool_state.tick_current` (RPC), zamiast potencjalnie starego cache monitora.
- Dodano `GET /positions/:address/diagnostics`: pokazuje czy PDA jest w monitorze, podpięte strategie, flagi `running/auto_execute/dry_run`, czy pozycja jest wyłączona z automatyki, oraz ostatni snapshot decyzji executora (jeśli dostępny).
- UI `Position Details` pokazuje sekcję **Diagnostics** z tymi danymi.

## 2026-04-05 — Backtest: strategie Bollinger i „ostatnia świeca” (`StratConfig` + `--indicator-strategies`)

**keywords:** backtest-optimize, StratConfig, Bollinger, last_candle, run_single, indicators, parse_strategy_label, clmm-lp-cli
**paths:** `crates/cli/src/backtest_engine.rs`, `crates/cli/src/engine/indicators.rs`, `crates/cli/src/commands/backtest_optimize.rs`, `crates/cli/src/output/optimize_result_json.rs`, `doc/BACKTEST_OPTIMIZE_STRATEGIES.md`

- **`StratConfig::Bollinger { window, k, rebalance_steps }`:** co `rebalance_steps` kroków (min. `window` zamknięć) nowe granice = SMA±K·σ na ostatnich `window` zamknięciach A/B; przy degeneracji (σ=0, dolna ≤0, itd.) fallback do pasma `width_pct` wokół SMA.
- **`StratConfig::LastCandle { candle_steps, rebalance_steps }`:** kotwica = close ostatniej **pełnej** „świecy” o długości `candle_steps` kroków; rebalance co `rebalance_steps` kroków (wyśrodkowanie na kotwicy, szerokość z siatki).
- **CLI:** `backtest-optimize --indicator-strategies` dokłada kilka wariantów do siatki (domyślnie wyłączone).
- **Etykiety / JSON:** `bollinger_w20_k2_r12`, `last_candle_c4_r24`; `strategy_kind` w optimize result: `bollinger`, `last_candle`.
- **Live:** `decision_config_from_optimize_result` zwraca czytelny błąd dla `bollinger` / `last_candle` (mapowanie na `StrategyMode` — osobna faza).

---
## 2026-04-04 — `deposit_quote`: noga A (U256) + wymuszenie dwustronnego quote (Open Position)

**keywords:** deposit_quote, quote_open_budget, Open Position, U256, amount_a, inv floor, Whirlpool
**paths:** `crates/protocols/src/orca/deposit_quote.rs`

- **Problem:** `amount_a` z `(L * (⌊2^64/√Pc⌋ - ⌊2^64/√Pu⌋)) >> 64` bywało **0** przy realnej odległości ticków (oba inverse po floor takie same), więc UI pokazywało np. **SOL = 0** przy **USDC > 0** i transakcja otwarcia pozycji bywała **nie do złożenia** mimo potwierdzonego swapu.
- **Fix:** token A z `L * (√Pu - √Pc) * 2^64 / (√Pc * √Pu)` w **U256**; zachowane **wymuszenie obu nóg > 0** (podłoga `L`, max `L` przy `usd ≤ target`, do ~3% slip jeśli minimalny dwustronny depozyt minimalnie przekracza nominalny target).

---
## 2026-04-04 — `/positions`: zakres kolor + `in_range` w `GET /orca/positions-by-owner`

**keywords:** Positions.tsx, OrcaOwnerPositionEntry, range_usdc_and_in_range_for_pool_ticks, in_range
**paths:** `web/src/pages/Positions.tsx`, `crates/api/src/handlers/orca_onchain.rs`, `crates/api/src/models.rs`, `crates/api/src/services/position_valuation.rs`

- Tabela monitora i tabela RPC: kolumna zakresu **zielona / czerwona** + etykieta In / Out of range.
- `OrcaOwnerPositionEntry.in_range`: jeden odczyt puli przez `range_usdc_and_in_range_for_pool_ticks` (wcześniej tylko zakres USDC).

---
## 2026-04-04 — Open Position: budżet USD ≈ wartość w pozycji (`quote-open-budget`)

**keywords:** deposit_quote, quote_open_budget, PositionCreate, token_max, Whirlpool, POST /pools, budget mode
**paths:** `crates/protocols/src/orca/deposit_quote.rs`, `crates/api/src/handlers/pools.rs`, `crates/api/src/models.rs`, `web/src/pages/PositionCreate.tsx`, `web/src/lib/api.ts`

- Problem: tryb „wspólna kwota USD” dzielił wartość 50/50 między tokeny → **caps** na Orca często w **złym stosunku** do krzywej przy wąskim zakresie → **~połowa** budżetu zostawała poza faktyczną płynnością.
- **Rozwiązanie:** `quote_deposit_budget_in_range` (binary search po `L`) + `POST /api/v1/pools/{address}/quote-open-budget` (ceny mintów jak `price_fetch`, ticki + `sqrt_price` z RPC). UI w trybie budżetowym **ustawia Amount** i wysyła **`token_max_*` z odpowiedzi** (bez straty float).
- Warunek: sensowny quote gdy **cena puli jest w [tick_lower, tick_upper)**; inaczej 400 z komunikatem lub ostrzeżenie `in_range: false`.

---
## 2026-04-04 — Wycena USD pozycji SOL/USDC: brak ceny WSOL (~½ wartości) + fallback

**keywords:** position_valuation, price_fetch, WSOL, Jupiter, CoinGecko, compute_position_usd_valuation, Value dashboard
**paths:** `crates/api/src/services/position_valuation.rs`, `crates/api/src/services/price_fetch.rs`

- Gdy mapa `mint → USD` nie zawiera ceny dla **WSOL** (`So111…`), kod używał `unwrap_or(0.0)` dla nogi SOL — **wartość pozycji** na dashboardzie była wtedy ok. **o połowę za niska** (została tylko noga USDC).
- **Walidacja on-chain:** jeśli brak ceny z feedu a mint to WSOL w parze **USDC + WSOL**, uzupełnienie **USD/SOL** z **bieżącego ticka puli** (`b_per_a_ui_decimal` — ta sama konwencja co zakres USDC).
- **price_fetch:** jeśli WSOL nadal nie ma ceny po Gecko/Jupiter/v4/DexPaprika/Dexscreener — ostatni fallback **CoinGecko** `simple/price` dla `solana` (tag źródła `coingecko_solana`).
- **Poprawka (wartość spadała ~\$1.16):** feed potrafi zwrócić **małą dodatnią** cenę WSOL (błędna jednostka / zły id) — wtedy stary warunek tylko `<= 0` **nie** podmieniał na tick puli. Teraz: **sanity band** (~\$10–\$2500) → tick; oraz **USDC mint → \$1** jeśli brak wpisu w mapie (żeby nie gubić nogi USDC).

---
## 2026-04-04 — `PositionResponse.uncollected_fees`: opłaty per token jak w Orca

**keywords:** UncollectedFeesInfo, fee_owed, PositionDetail, GET /positions, position_valuation, uncollected_fees_info_for_position
**paths:** `crates/api/src/models.rs`, `crates/api/src/services/position_valuation.rs`, `crates/api/src/handlers/positions.rs`, `web/src/pages/PositionDetail.tsx`

- Z waluacji USD (`compute_position_usd_valuation`) wystawiane są etykiety mintów (SOL/USDC/skrót) oraz kwoty `fee_owed` w jednostkach UI; pole opcjonalne gdy waluacja się nie uda.
- **Poprawka:** gdy pełna waluacja zwraca błąd (RPC), a zakres USDC nadal pochodzi z `tick_range_usdc_for_position` (osobny fetch z `.ok()`), `uncollected_fees` było puste — dodano `uncollected_fees_info_for_position` (tylko pool + decymale + `fee_owed`, bez math liquidity / cen).
- `GET /positions/:address`: przed waluacją USD wywołanie `refresh_position_fees_from_chain` (świeże `fee_owed` z RPC; monitor mógł pokazywać `$0.000` względem Orca). Web: `formatUsdUncollectedFees` — 6 miejsc dla kwot &lt; $0.01.
- `fees_usd` w `compute_position_usd_valuation`: mnożenie **Decimal** (fee UI × cena), nie `f64`; log `warn` gdy `fee_owed` &gt; 0 a USD = 0 (brak ceny mintu). UI: surowe `fees_earned_a/b` pod linią USD.
- Dashboard: „Uncollected fees (USD)” na szczegółach pozycji (format sub-cent).

## 2026-04-04 — Link pozycji ↔ strategia: `monitor.add_position` best-effort (RPC)

**keywords:** link_position_strategy, ensure_strategy_running_after_position_link, start_strategy_executor_core, PositionMonitor, add_position, Failed to get account
**paths:** `crates/api/src/handlers/strategies.rs`, `crates/api/src/handlers/positions.rs`

- Gdy publiczny RPC nie zwraca konta pozycji (`Failed to get account`), twarde **400** po zapisie `position_addresses` blokowało UI; teraz powiązanie zostaje zapisane, start executora kontynuowany, a komunikat HTTP **200** może zawierać ostrzeżenie zamiast błędu.
- Dotyczy: `POST /positions/{address}/strategy`, `POST /strategies/{id}/start`, oraz open z `strategy_id` (ścieżka z notatką zamiast wyłącznie „automation started”).

## 2026-04-04 — Vite: scalanie root `.env` dla `API_PORT` / proxy (mniej HTTP 502)

**keywords:** vite, API_UPSTREAM, API_PORT, loadEnv, dev proxy, 502
**paths:** `web/vite.config.ts`, `web/.env.example`

- `defineConfig` ładuje `loadEnv` z **repo root** i z `web/` (`web` nadpisuje root) — samo `npm run dev` widzi ten sam `API_PORT` co `cargo run --bin clmm-lp-api` z root `.env`, bez ręcznego `API_UPSTREAM`.
- Domyślny port proxy bez env: **8080** (zgodnie z `.env.example` i `start-dev-stack.mjs`), zamiast samego `web/` z hardcoded `8081`.

## 2026-04-04 — `POST /positions/{address}/strategy`: przypisz / zmień / odłącz strategię od PDA

**keywords:** link_position_strategy, LinkPositionStrategyRequest, position_addresses, executor_disabled_position_addresses, PositionDetail, append_position_address_to_strategy
**paths:** `crates/api/src/handlers/positions.rs`, `crates/api/src/services/strategy_service.rs`, `crates/api/src/routes.rs`, `web/src/lib/api.ts`, `web/src/pages/PositionDetail.tsx`

- Body: `{ "strategy_id": "<uuid>" | null }`. Ustawienie ID: usuwa PDA ze wszystkich strategii (także z list wyłączeń executora), dopisuje do wybranej, persystencja, `ensure_strategy_running_after_position_link`. `null`: tylko odłączenie (unlink).
- UI na szczegółach pozycji: lista powiązań lub „None linked”, select wszystkich strategii, **Apply** / **Remove link**; po sukcesie invalidacja `['strategies']`.

## 2026-04-04 — `GET /strategies`: zawsze scalaj `position_addresses` z surowego JSON

**keywords:** list_strategies, StrategyParameters, position_addresses, Open Position, strategy_id, PositionDetail
**paths:** `crates/api/src/handlers/strategies.rs`, `web/src/pages/PositionDetail.tsx`

- Lista i szczegóły strategii parsowały `parameters` przez `serde_json::from_value::<StrategyParameters>`; przy **nieudanym** deserializowaniu (np. starsze / nietypowe wartości liczbowe) używane było `unwrap_or_default()` — **tracone** `parameters.position_addresses`, więc UI pokazywało „None linked” mimo że po `POST /positions` z `strategy_id` adres był dopisany w pamięci/pliku.
- Dodano `strategy_parameters_from_stored_config`: najpierw best-effort `StrategyParameters`, potem **nadpisanie** `position_addresses` i `executor_disabled_position_addresses` z **surowego** obiektu `parameters` w `config`.
- Frontend: dopasowanie PDA z listą — `trim` + obsługa wartości nie-string; tooltip przy „None linked”.

## 2026-04-03 — `PositionResponse`: zakres w USDC (pary z jednym USDC)

**keywords:** PositionResponse, range_lower_usdc, range_upper_usdc, tick_range_usdc, PositionDetail, Dashboard
**paths:** `crates/api/src/services/position_valuation.rs`, `crates/api/src/handlers/positions.rs`, `crates/api/src/models.rs`, `web/src/lib/utils.ts`, `web/src/pages/PositionDetail.tsx`

- Dla puli **USDC + jeden inny token** API uzupełnia `range_lower_usdc` / `range_upper_usdc` (min/max granicy zakresu) oraz `range_usdc_quote` (np. `per 1 SOL`) — liczone z ticków + decymali mintów (mainnet/devnet USDC mint).
- Gdy para nie ma USDC, pola są `null` — UI pokazuje jak wcześniej same ticki.
- **Poprawka:** cena UI z ticka jest liczona w **log-domenie** (`exp(tick·ln(1.0001) + Δdec·ln(10))`), żeby uniknąć **underflow `f64`** na samym `1.0001^tick` przy głębokich ujemnych tickach (np. -25276) — wcześniej zakres USDC znikał mimo pary SOL/USDC.
- **`GET /orca/positions-by-owner`:** te same pola co w `PositionResponse` (po jednym fetchu stanu puli na wiersz), tabela RPC w UI pokazuje USDC jak monitor.

## 2026-04-03 — `GET /positions/:address`: fallback RPC gdy brak wpisu w monitorze

**keywords:** get_position, list_positions, monitored_position_from_chain, registry.jsonl, lifecycle_ledger_aggregates
**paths:** `crates/api/src/handlers/positions.rs`, `crates/api/src/services/position_valuation.rs`, `crates/api/src/position_registry_seed.rs`, `crates/api/src/services/lifecycle_ledger_aggregates.rs`, `crates/api/src/handlers/analytics.rs`

- Gdy adresu nie ma w `PositionMonitor`, API buduje stan z RPC (`PositionReader` + `WhirlpoolReader`) zamiast zwracać 404; w tle `tokio::spawn` próbuje `monitor.add_position` (healing listy).
- `GET /positions` (lista): dla każdego `registry_open` w `data/positions/registry.jsonl`, którego nie ma w monitorze, dokładany jest ten sam fallback RPC — **górna tabela** nie jest już pusta tylko przez „tylko RAM”.
- Przywrócono moduł `lifecycle_ledger_aggregates` + pole `fees_collected_from_ledger` w `GET /analytics/portfolio`.

## 2026-04-03 — `POST /positions`: `monitor.add_position` po open (szczegóły od razu)

**keywords:** open_position, PositionMonitor, add_position, GET /positions/:address, PositionCreate
**paths:** `crates/api/src/handlers/positions.rs`, `web/src/pages/PositionCreate.tsx`

- Po udanym open API woła `monitor.add_position(position_pda)`, żeby `GET /positions/:addr` działał od razu (wcześniej tylko rejestr / restart).
- Gałąź z samym `message` bez PDA bez zmian; idempotent replay (`message` + `position_pda`) nie jest już ucinany na początku.
- Web: po sukcesie nawigacja do `/positions/{position_pda}` gdy jest w odpowiedzi.

## 2026-04-03 — Whirlpool close: domyślnie niski slippage (100 bps) + opcjonalnie `WHIRLPOOL_CLOSE_SLIPPAGE_BPS`

**keywords:** close_position, WhirlpoolExecutor, TokenMinSubceeded, 6018, WHIRLPOOL_CLOSE_SLIPPAGE_BPS, RebalanceExecutor
**paths:** `crates/protocols/src/orca/executor.rs`, `crates/execution/src/strategy/rebalance.rs`, `.env.example`

- Domyślnie z powrotem **100 bps** (niski min-out); opcjonalnie env **`WHIRLPOOL_CLOSE_SLIPPAGE_BPS`** (`0..=10000`) na hoście API/CLI gdy trzeba obejść 6018 bez zmiany zasady „jak najniżej”.
- Hint przy `6018` doprecyzowany: podnieś slippage tylko przy retry / wyższy tymczasowy env.

## 2026-04-03 — API: ceny USD — GeckoTerminal przed Jupiterem (więcej źródeł bez klucza)

**keywords:** clmm-lp-api, price_fetch, GeckoTerminal, Jupiter, Dexscreener, DexPaprika
**paths:** `crates/api/src/services/price_fetch.rs`

- Do łańcucha cen dopisano **GeckoTerminal** `GET .../networks/solana/token_price/{mints}` (batch w chunkach), **przed** Jupiter v2 — darmowe, bez klucza, typowo wystarcza na dashboard bez `JUPITER_API_KEY`.
- Kolejność: stable → GeckoTerminal → Jupiter v2 → legacy v4 → DexPaprika → Dexscreener.

## 2026-04-03 — Rebalance API + UI: `strategy_range` / `price_band` / `ticks` (bez samego promptu na ticki)

**keywords:** RebalanceRequest, RebalanceInput, calculate_tick_range, PositionDetail, POST /positions/rebalance
**paths:** `crates/api/src/models.rs`, `crates/api/src/services/position_service.rs`, `web/src/lib/api.ts`, `web/src/pages/PositionDetail.tsx`

- `POST /positions/{address}/rebalance`: pole `input`: `ticks` (domyślnie), `strategy_range` (szerokość z `strategy_id` i/lub `range_width_pct`, środek = `tick_current`), `price_band` (`center_price` + `range_width_pct`).
- Dry-run z samymi tickami nadal bez odczytu puli z RPC (zgodność z testami offline).
- Web: panel wyboru trybu zamiast `prompt` na ticki.

## 2026-04-03 — Lifecycle ledger: kwoty tokenów przy `collect_fees` + sumy na szczegółach pozycji

**keywords:** bot_collect_fees, orca_position_lifecycle.jsonl, tx_lifecycle, PositionDetail, fee_payer_token_a_delta_ui, clmm-lp-protocols, clmm-lp-execution
**paths:** `crates/protocols/src/ledger/tx_lifecycle.rs`, `crates/execution/src/strategy/rebalance.rs`, `web/src/pages/PositionDetail.tsx`

- Wiersze `event: bot_collect_fees` dopisują opcjonalnie `pool_mint_*` oraz `fee_payer_token_*_delta_*` (post−pre z `meta.pre/postTokenBalances` dla fee payer + mintów puli), żeby UI mógł sumować realnie skredytowane fee w portfelu API po wielu collectach.
- `try_append_rebalance_executor_tx_cost` przyjmuje `Arc<RpcProvider>` (jedno pobranie tx na wiersz).
- Web: karta „Fees collected (cumulative, from ledger)” + kolumny ΔA/ΔB w tabeli ledgera; etykieta „Unclaimed fees (USD est.)” wyjaśnia różnicę względem niewypłaconych fee na pozycji.

## 2026-04-03 — API: USD dla pozycji / portfela — Jupiter v2 + fallback (koniec zer z powodu martwego `price.jup.ag/v4`)

**keywords:** clmm-lp-api, price_fetch, position_valuation, Jupiter, Dexscreener, DexPaprika, wallet, analytics
**paths:** `crates/api/src/services/price_fetch.rs`, `crates/api/src/services/position_valuation.rs`, `crates/api/src/handlers/prices.rs`, `web/src/lib/api.ts`, `.env.example`

- Publiczny endpoint `https://price.jup.ag/v4/price` bywa niedostępny; `https://api.jup.ag/price/v2` często wymaga `JUPITER_API_KEY` (`x-api-key`).
- Wspólne `fetch_mint_prices_usd`: stable USDC/USDT → Jupiter v2 → (best-effort) legacy v4 → DexPaprika SSE → Dexscreener `token-pairs` (jak w CLI snapshot).
- `GET /prices/jupiter` i wycena pozycji (`value_usd`, fees USD, dashboard) korzystają z tej samej ścieżki. Frontend: usunięto bezużyteczny fallback do v4 w przeglądarce.

## 2026-04-03 — Web: dłuższy timeout dla operacji on-chain (`/positions/*`)

**keywords:** web, timeout, fetch abort, open position, swap-before-open, api.ts
**paths:** `web/src/lib/api.ts`

- Domyślne `fetchJson` miało timeout 15s, co przy wolnym RPC powodowało abort requestu w UI mimo że transakcja mogła zostać wykonana po stronie backendu.
- Dodano `fetchJsonLong` (90s) i podpięto pod cięższe endpointy `POST/DELETE /positions/*` (swap/open/close/collect/rebalance/decrease).

## 2026-04-03 — Swap API: ograniczony czas post-confirmation (mniej abortów requestu)

**keywords:** clmm-lp-execution, swap-before-open, wait_for_confirmation, timeout, abort signal
**paths:** `crates/execution/src/strategy/rebalance.rs`

- Po potwierdzeniu tx swap endpoint potrafił długo wisieć na dodatkowym `wait_for_confirmation` (niestabilne `getTransaction`), co kończyło się abortem requestu po stronie UI.
- `ensure_execution_success` ma teraz timeout 15s dla post-confirmation check; przy timeout/err loguje ostrzeżenie i kontynuuje bez blokowania odpowiedzi API.

## 2026-04-03 — Runner restart: przed startem ubijamy stare instancje

**keywords:** script runner, Start-ClmmScriptRunner, Stop-ClmmScriptRunner, port reuse
**paths:** `tools/Start-ClmmScriptRunner.ps1`

- Wrapper runnera wywołuje `tools/Stop-ClmmScriptRunner.ps1` przed startem, żeby zwolnić “nasz” port przy restarcie zamiast mnożyć konflikty listenerów.

## 2026-04-03 — API: większy stack Tokio workerów (fix crash `swap-before-open`)

**keywords:** clmm-lp-api, tokio, stack overflow, swap-before-open, 502
**paths:** `crates/api/src/main.rs`

- `POST /positions/swap-before-open` potrafił crashować proces API (`thread 'tokio-rt-worker' has overflowed its stack`), a frontend widział `HTTP 502 (empty body)`.
- API uruchamia teraz runtime Tokio z konfigurowalnym większym stackiem workerów (`API_TOKIO_STACK_SIZE_BYTES`, domyślnie 8 MiB), co stabilizuje ścieżkę swapu.

## 2026-04-03 — Web dev proxy: domyślny API upstream na `:8081` (nie `:8080`)

**keywords:** web, vite, API_UPSTREAM, API_PORT, 8081, dev proxy
**paths:** `web/vite.config.ts`

- Domyślny port backendu dla Vite proxy zmieniony z `8080` na `8081`, bo lokalny workflow dashboardu/API używa `:8081`, a `:8080` bywa zajęte przez inne usługi.
- Ogranicza to przypadki, gdzie UI pokazuje “puste” dane po restarcie, mimo że właściwy API (`:8081`) ma już zapisane strategie.

## 2026-04-03 — Runner: wrapper `Start-ClmmScriptRunner.ps1` używa `/health` zamiast `netstat`

**keywords:** script runner, Start-ClmmScriptRunner, CLMM_SCRIPT_RUNNER_PORT, HTTP.sys, /health
**paths:** `tools/Start-ClmmScriptRunner.ps1`

- Poprawione wykrywanie zajętego portu: wrapper nie przełącza już na `9857`, jeśli `9847` jest zajęte przez HTTP.sys, ale runner odpowiada na `GET /health`.
- Dzięki temu API i UI mniej tracą spójność portów przy wielokrotnych restartach.

## 2026-04-03 — API: strategie persistują do `data/strategies.json` (do DELETE)

**keywords:** clmm-lp-api, strategies, persistence, strategy_store, JSON store, restart
**paths:** `crates/api/src/strategy_store.rs`, `crates/api/src/state.rs`, `crates/api/src/handlers/strategies.rs`, `crates/api/src/services/strategy_service.rs`

- Dodano persistencję strategii na dysk, aby strategie nie znikały po restarcie API — dopóki ich nie usuniesz przez `DELETE /strategies/{id}`.
- Zapis wykonuje się po `POST /strategies`, `PUT /strategies/{id}`, `DELETE /strategies/{id}`, po zmianie `executor_disabled_position_addresses` oraz po linkowaniu pozycji do strategii (`parameters.position_addresses`).

## 2026-04-03 — API: DRY_RUN env steruje trybem live/symulacji (swap/open) + runner port

**keywords:** clmm-lp-api, DRY_RUN, dry_run, swap-before-open, Start-Dashboard-Safe, script_runner
**paths:** `crates/api/src/state.rs`, `tools/Start-Dashboard-Safe.ps1`, `tools/script_runner/Start-ClmmScriptRunner.ps1`

- API przestaje mieć „zabetonowane” `dry_run=true` — teraz czyta `DRY_RUN` z `.env` (domyślnie nadal `true` dla bezpieczeństwa).
- Skrypty startowe przestają mylnie przełączać runnera na `:9857` gdy `:9847` wygląda na zajęte przez HTTP.sys; decyzja jest oparta o `GET /health`, a runner ma fallback portu przy niepowodzeniu bind.

## 2026-04-03 — API: obsługa WALLET_KEYPAIR_PATH dla operacji position (swap/open)

**keywords:** clmm-lp-api, KEYPAIR_PATH, SOLANA_KEYPAIR_PATH, WALLET_KEYPAIR_PATH, wallet loading, position_executor
**paths:** `crates/api/src/services/position_executor.rs`

- `swap-before-open`/`open position` po przełączeniu na live wymaga `StrategyExecutor` z podpisującym walletem.
- U Ciebie `.env` używa `WALLET_KEYPAIR_PATH`, a kod wcześniej czytał tylko `KEYPAIR_PATH`/`SOLANA_KEYPAIR_PATH` — dlatego executor nie był tworzony.
- Dodano alias `WALLET_KEYPAIR_PATH` oraz rozwijanie `~/...` na Windows.

## 2026-04-03 — UI: dwa kroki SWAP -> OPEN (endpoint `/positions/swap-before-open`)

**keywords:** PositionCreate, swap-before-open, two-step flow, swapBeforeOpen endpoint, cost_session_id
**paths:** `crates/api/src/models.rs`, `crates/api/src/handlers/positions.rs`, `crates/api/src/routes.rs`, `crates/api/src/openapi.rs`, `web/src/lib/api.ts`, `web/src/pages/PositionCreate.tsx`, `crates/api/src/services/position_service.rs`

- Dodano endpoint **`POST /positions/swap-before-open`**: wykonuje tylko Orca swap `ExactIn` (bez open).
- UI w `PositionCreate` teraz:
  - najpierw wykonuje SWAP i pokazuje `swap_signature`,
  - dopiero po powodzeniu umożliwia OPEN Position (bez `swap_before_open` w payload).
- `cost_session_id` (księgowanie) jest generowane na kroku SWAP i przekazywane na krok OPEN, żeby sumować koszty per otwarta pozycja.

## 2026-04-03 — UI: „Zakres według ceny” pokazuje złe wartości (raw vs UI decimals)

**keywords:** PositionCreate, uiPriceFromRawPriceRatio, rawPriceRatioFromUiPrice, decimals, tickToPriceRatio
**paths:** `web/src/lib/whirlpoolTicks.ts`, `web/src/pages/PositionCreate.tsx`

- Poprawione mapowanie `tick <-> price` w sekcji „Zakres według ceny”: `tick_to_price` daje raw ratio, więc do UI (np. USDC za 1 SOL) trzeba uwzględnić różnicę `decimals`.
- UI:
  - synchronizacja cen z tickami jest przeliczana raw -> UI,
  - przeliczenie „Ustaw ticki z tych cen” jest UI -> raw (przed wyrównaniem do `tick_spacing`).

## 2026-04-02 — Swap + Open / position ops: portfel z `KEYPAIR_PATH` (nie tylko `auto_execute`)

**keywords:** position_executor, resolve_executor_for_position_ops, set_wallet, KEYPAIR_PATH, start_strategy_executor_core, StrategyExecutor, swap_before_open
**paths:** `crates/api/src/services/position_executor.rs`, `crates/api/src/handlers/positions.rs`, `crates/api/src/handlers/strategies.rs`, `crates/api/src/services/strategy_service.rs`

- **Problem:** `set_wallet` było wywoływane tylko przy `auto_execute && !dry_run`, więc przy `auto_execute=false` executor nie miał klucza — **swap i open** kończyły się „Wallet not set”.
- **Fix:** przy `!dry_run` ładuj portfel z `KEYPAIR_PATH` / `SOLANA_KEYPAIR_PATH` zawsze, gdy plik jest ustawiony; `auto_execute=true` bez env nadal zwraca 400.
- **`resolve_executor_for_position_ops`:** jeśli nie ma uruchomionej strategii, tworzy leniwy executor pod `__api_position_ops__` (ten sam keypair), żeby **POST /positions** działało bez **POST /strategies/…/start**.

## 2026-04-02 — Open Position: zakres przez cenę (B/A) → ticki

**keywords:** PositionCreate, whirlpoolTicks, alignPriceRatioToTicks, price ratio, tick spacing
**paths:** `web/src/lib/whirlpoolTicks.ts`, `web/src/pages/PositionCreate.tsx`

- Obok pól **Tick Lower / Upper**: sekcja **ceny graniczne** (ten sam stosunek co `price` puli: mint B za 1 mint A). Przycisk **„Ustaw ticki z tych cen”** wylicza ticki wyrównane do `tick_spacing` (`floor` dolnej ceny, `ceil` górnej; zamiana jeśli użytkownik poda odwrotnie).
- Pola cenowe synchronizują się z tickami, gdy edytujesz ticki lub auto-sync strategii; ręczna edycja cen wyłącza sync do czasu „Ustaw ticki…”.

## 2026-04-02 — Koszt swapu: szacunek (ledger) + `cost_session_id` (księgowanie per pozycja)

**keywords:** swap cost, cost_session_id, rebalance_session_id, orca_position_lifecycle.jsonl, swap_cost_estimate, bot_swap_exact_in, PositionOpenResponse, GET estimate-swap-cost, position_registry, tx_lifecycle
**paths:** `crates/protocols/src/ledger/tx_lifecycle.rs`, `crates/protocols/src/ledger/swap_cost_estimate.rs`, `crates/protocols/src/ledger/position_registry.rs`, `crates/execution/src/strategy/rebalance.rs`, `crates/api/src/models.rs`, `crates/api/src/handlers/pools.rs`, `crates/api/src/handlers/positions.rs`, `web/src/pages/PositionCreate.tsx`

- **`GET /pools/{address}/estimate-swap-cost`**: mediana `tx_fee_lamports` z lokalnego JSONL dla `swap_exact_in` w tej puli (jeśli brak — `DEFAULT_ESTIMATED_SWAP_NETWORK_FEE_LAMPORTS` = 10_000); pokazuje **opłatę sieciową** (`meta.fee`), nie pełny delta SPL/SOL portfela.
- **`OpenPositionRequest.cost_session_id`**: opcjonalnie; **PositionCreate** generuje UUID przy „Swap + Open”, żeby wiersze swap + open miały ten sam `rebalance_session_id` w ledgerze (suma kosztów sesji → przypisanie do nowej pozycji po `position_pda` w tym samym id).
- **`try_append_rebalance_executor_tx_cost` / CLI swap / registry**: opcjonalny override sesji zamiast wyłącznie `CLMM_REBALANCE_SESSION_ID`; zdarzenie `swap_exact_in` → `event: bot_swap_exact_in`.
- **`POST /positions`**: odpowiedź **`PositionOpenResponse`** (`message`, `position_pda`, `swap_signature`, `cost_session_id`).

## 2026-04-02 — Open Position: swap w puli Orca przed `open` (`swap_before_open`)

**keywords:** open position, swap_before_open, SwapInPoolBeforeOpen, swap_exact_in, Orca ExactIn, PositionService, PositionCreate, RebalanceExecutor
**paths:** `crates/protocols/src/orca/executor.rs`, `crates/execution/src/strategy/rebalance.rs`, `crates/execution/src/strategy/executor.rs`, `crates/api/src/models.rs`, `crates/api/src/services/position_service.rs`, `web/src/lib/api.ts`, `web/src/pages/PositionCreate.tsx`

- **`OpenPositionRequest.swap_before_open`**: opcjonalnie `{ specified_mint, amount_in }` — walidacja mintu A/B puli; najpierw swap ExactIn w tej puli, potem open (portfel API / executor).
- **UI**: przy jednostronnym niedoborze — checkbox + szacunek `amount_in`; przycisk „Swap + Open Position”.
- **Executor**: `WhirlpoolExecutor::swap_exact_in` + `RebalanceExecutor::execute_swap_exact_in` — do ponownego użycia przy rebalansowaniu.

## 2026-04-02 — Branding: nazwa produktu „Bociarz LP” (UI, OpenAPI, logi, docs)

**keywords:** branding, Bociarz LP, web, openapi, phantom_auth, README
**paths:** `web/src/components/Layout.tsx`, `crates/api/src/openapi.rs`, `crates/api/src/main.rs`, `README.md`, `web/index.html`

- User-facing teksty **„CLMM LP”** / **„Bociarz LP Strategy Lab”** ujednolicono do **„Bociarz LP”** (nagłówek dashboardu, tytuł strony, OpenAPI, komunikat Phantom sign-in, start API/CLI, monitoring Docker). Nazwy crate’ów (`clmm-lp-*`) i zmienne env (`CLMM_*`) bez zmian.

## 2026-04-02 — Strategia: Range Width % wymagany; Open Position — ticki z puli + strategii

**keywords:** web, StrategyCreate, StrategyEdit, PositionCreate, range_width_pct, calculate_tick_range, whirlpoolTicks, getPoolState
**paths:** `web/src/lib/strategyFormShared.tsx`, `web/src/lib/whirlpoolTicks.ts`, `web/src/pages/StrategyCreate.tsx`, `web/src/pages/StrategyEdit.tsx`, `web/src/pages/PositionCreate.tsx`

- **`FIELD_ENABLED.static_range.rangeWidth`**: włączone — **Range Width %** jest wymagany dla wszystkich typów, które go używają (`isRangeWidthSatisfied` przy create/save).
- **`calculateTickRangeFromWidthPct`**: TS odpowiednik `orca::pool_reader::calculate_tick_range` (szerokość % całego pasma ceny, tick spacing).
- **`tickToPriceRatio` / `formatPriceRatio`**: pod polami ticków — **ceny mint B za 1 mint A** przy dolnym/górnym ticku + spot z puli (zgodnie z `1.0001^tick`).
- **Open Position**: przy wybranej strategii z `parameters.range_width_pct` — tick lower/upper wyliczane z **`getPoolState`** (refetch ~10 s) + checkbox **auto-sync**; ręczna edycja ticków wyłącza sync.

## 2026-04-02 — Open Position: walidacja sald vs kwoty + linki Jupiter (prefill)

**keywords:** web, PositionCreate, WalletBalancesResponse, Jupiter, WSOL, swap UX
**paths:** `web/src/pages/PositionCreate.tsx`

- Porównanie **wymaganych kwot** (token A/B) z **saldem** (dla WSOL: native SOL + konto WSOL); przy niedoborze — **blokada** „Open Position”, komunikat PL i CTA.
- **Jupiter** `https://jup.ag/swap?inputMint=&outputMint=` + opcjonalnie `amount` (szacunek ExactIn z cen USD API, +5% bufor). Portfel w przeglądarce zwykle łączy się z Jupiterem po otwarciu karty.
- Link **Orca** (strona główna) jako alternatywa bez prefilla URL.
- **Na później (nie wdrożone):** składanie swap tx w API / jedna transakcja swap+open; semi-auto z podpisem w aplikacji.

## 2026-04-02 — `CreateStrategyRequest`: opcjonalne `pool_address` (deserialize + PUT)

**keywords:** CreateStrategyRequest, pool_address, serde, PUT /strategies, StrategyEdit, clmm-lp-api
**paths:** `crates/api/src/models.rs`, `crates/api/src/handlers/strategies.rs`, `web/src/lib/api.ts`, `web/src/pages/StrategyEdit.tsx`

- Pole **`pool_address`** w body POST/PUT jest **opcjonalne** (`Option` + `serde(default)`), żeby klient bez tego pola nie dostawał błędu „missing field `pool_address`”.
- **POST**: jeśli podano niepusty string — trafia do `config.pool_address`.
- **PUT**: niepusty string ustawia pool; pusty string czyści legacy pool w configu; **brak pola / `null`** — merge: zostaje poprzedni `pool_address` z bazy.
- **Web (edycja)**: jeśli strategia ma legacy `pool_address`, pole jest wysyłane przy zapisie.

## 2026-04-02 — Open Position + strategia: auto-start executora; pauza per pozycja

**keywords:** open_position, ensure_strategy_running_after_position_link, executor_disabled_position_addresses, StrategyExecutor, position-executor, PositionDetail
**paths:** `crates/api/src/handlers/positions.rs`, `crates/api/src/handlers/strategies.rs`, `crates/execution/src/strategy/executor.rs`, `web/src/pages/PositionDetail.tsx`, `web/src/lib/api.ts`

- Po udanym `open` z `strategy_id`: dopisek PDA do strategii, potem **automatyczny start** executora (`ensure_strategy_running_after_position_link`); jeśli strategia już działała — tylko `monitor.add_position` + sync listy wyłączeń.
- **`parameters.executor_disabled_position_addresses`**: PDA pomijane w `evaluate_all` (automatyzacja wyłączona dla tej pozycji, bez zatrzymywania całej strategii).
- **`POST /strategies/{id}/position-executor`** body `{ position_address, enabled }` — usuwa/dodaje PDA na liście wyłączeń; UI na szczegółach pozycji.

## 2026-04-02 — Strategie: edycja, usuwanie, `auto_execute` w odpowiedzi API

**keywords:** web, StrategyEdit, StrategyDetail, delete_strategy, StrategyResponse, auto_execute, clmm-lp-api
**paths:** `crates/api/src/handlers/strategies.rs`, `web/src/pages/StrategyEdit.tsx`, `web/src/pages/StrategyDetail.tsx`, `web/src/App.tsx`, `web/src/lib/api.ts`

- **`GET/POST/PUT /strategies`**: `StrategyResponse` zawiera **`auto_execute`** (spójnie z `dry_run` i configiem).
- **`DELETE /strategies/{id}`**: przed usunięciem — zatrzymanie executora (`stop`), `running = false`, cleanup `executors` / `optimization_busy` (jak przy stop), potem usunięcie wpisu.
- **Web**: trasa **`/strategies/:id/edit`**, formularz edycji (PUT), przyciski **Edit / Delete** na szczegółach; na liście skrót **Edit**.

## 2026-04-02 — Strategia bez poolu: parametry zapis; pool przy Open Position

**keywords:** web, StrategyCreate, CreateStrategyRequest, StrategyParameters, position_addresses, OpenPositionRequest
**paths:** `crates/api/src/models.rs`, `crates/api/src/handlers/strategies.rs`, `crates/api/src/services/strategy_service.rs`, `crates/api/src/handlers/positions.rs`, `web/src/pages/StrategyCreate.tsx`, `web/src/pages/PositionCreate.tsx`, `web/src/lib/api.ts`

- **`CreateStrategyRequest`**: głównie parametry (`strategy_type`, `parameters`, dry_run, `auto_execute`); opcjonalne legacy **`pool_address`** (patrz wpis wyżej). Pool przy nowych flow wybierany przy **otwarciu pozycji**.
- **`StrategyResponse.pool_address`**: opcjonalne (legacy / stare wpisy); nowe strategie zwracają brak pola lub pusto.
- **`PUT /strategies/{id}`**: przy aktualizacji **zachowywane** jest `parameters.position_addresses` ze starej konfiguracji (merge), żeby nie gubić powiązań z pozycjami.
- **`append_position_address_to_strategy`**: bez walidacji poolu strategii vs pozycji — dopisek PDA po udanym `open`.
- **`StrategyParameters`** (Rust): pole `position_addresses`; frontend: lista powiązanych adresów na **Strategy detail**.

## 2026-04-02 — Open Position: opcjonalne przypisanie strategii (`strategy_id`)

**keywords:** web, PositionCreate.tsx, OpenPositionRequest, position_addresses, strategies, clmm-lp-api
**paths:** `crates/api/src/models.rs`, `crates/api/src/handlers/positions.rs`, `crates/api/src/services/strategy_service.rs`, `web/src/pages/PositionCreate.tsx`, `web/src/lib/api.ts`

- `POST /positions` przyjmuje opcjonalne **`strategy_id`**. Przed otwarciem: walidacja, że strategia istnieje i **`pool_address` strategii = pool pozycji**.
- Po sukcesie on-chain (pole `position_pda` w odpowiedzi serwisu) PDA jest dopisywane do **`parameters.position_addresses`** danej strategii (bez duplikatów).
- **Dry-run** nie zwraca PDA — linkowanie nie następuje (brak adresu).
- UI *Open Position*: lista strategii filtrowana po wybranym poolu.

## 2026-04-02 — Web Strategy Create: pola zależne od typu, opisy, tooltips

**keywords:** web, StrategyCreate.tsx, tooltip, StrategyType, StrategyParameters, execution, getPools
**paths:** `web/src/pages/StrategyCreate.tsx`, `web/src/components/ui/tooltip.tsx`

- Formularz tworzenia strategii **wyłącza** (disabled) parametry liczbowe nieużywane przez wybrany `strategy_type`; przy zmianie typu **czyści** wartości pól, które przestają obowiązywać.
- Pod wyborem typu: **krótki opis** zachowania trybu; przy każdej etykiecie — **tooltip** (ikona) z wyjaśnieniem pola i wpływu wartości na executor.
- Payload wysyła tylko **dozwolone** dla typu klucze w `parameters` (spójnie z backendem).
- **Pool:** zamiast ręcznego wpisywania adresu — `<select>` z `GET /pools` (sort wg TVL), przycisk odświeżenia, komunikaty przy błędzie / pustej liście; **etykiety par** z `GET /orca/tokens/{mint}` (symbole jak SOL/USDC, fallback skrót mintu).

## 2026-04-02 — Web Wallet: SPL jako token (Orca), nie sam skrót mintu

**keywords:** web, Wallet.tsx, getOrcaToken, orca/tokens, SPL
**paths:** `web/src/pages/Wallet.tsx`

- Tabela „Saldo on-chain” pokazuje **symbol/nazwę** z `GET /api/v1/orca/tokens/{mint}` (pierwsza linia), **pełny mint** pod spodem; kolumna „Mint” → „Token”.

## 2026-04-02 — Web: typy `Pool` / `PoolState` zgodne z API (`PoolResponse`)

**keywords:** web, api.ts, Pools.tsx, PoolDetail.tsx, PositionCreate.tsx, PoolResponse, PoolStateResponse, orca/tokens
**paths:** `web/src/lib/api.ts`, `web/src/pages/Pools.tsx`, `web/src/pages/PoolDetail.tsx`, `web/src/pages/PositionCreate.tsx`

- Frontend używał legacy pól (`token_a`/`token_b`, `fee_tier`, rezerwy w stanie), podczas gdy backend zwraca `token_mint_a`/`token_mint_b`, `fee_rate_bps`, `apy_estimate` oraz stan z `sqrt_price` i `fee_growth_*` — przez to `/pools/:address` mogło rzucać przy renderze (biały ekran).
- Dodano `getOrcaToken` (`GET /api/v1/orca/tokens/{mint}`) do symboli/decimals w szczegółach puli i w formularzu „Open Position”.

## 2026-04-02 — Runner skryptów: wygodniejszy start + stabilne HTTP/1.1

**keywords:** scripts, runner, web, api, http1, dotenv, tools
**paths:** `tools/script_runner/Start-ClmmScriptRunner.ps1`, `tools/Start-ClmmScriptRunner.ps1`, `web/src/pages/Scripts.tsx`, `crates/api/src/handlers/scripts.rs`

- Runner skryptów przy starcie importuje root `.env` (tylko brakujące zmienne), więc nie trzeba ręcznie ustawiać `CLMM_SCRIPT_RUNNER_TOKEN` ani `SOLANA_RPC_URL` w oknie PowerShell.
- API wymusza HTTP/1.1 do komunikacji z runnerem (HttpListener jest wrażliwy na nowsze tryby), co usuwa losowe 502/500 przy `POST /scripts/{id}/run`.
- UI `/scripts` pokazuje stan uruchamiania i blokuje wieloklik dla tego samego skryptu do czasu zakończenia.

## 2026-04-02 — Scripts: poprawione `tools/scripts-manifest.json` opisy

**keywords:** web, Scripts.tsx, tools, scripts-manifest, summary, when_to_use, runner
**paths:** `tools/scripts-manifest.json`, `doc/SCRIPTS_CATALOG.md`, `web/src/pages/Scripts.tsx`

- Uzupełniono/naprawiono `summary` w manifeście dla wszystkich `tools/*.ps1`, tak aby UI “Catalog” pokazywało opis faktycznej roli skryptu (nie generyczne “Operator script …”).
- Dodano brakujące wpisy dla `Start-Dashboard.ps1` i `Stop-ClmmApi.ps1` (wcześniej były tylko `auto_discovered`).

## 2026-04-02 — clmm-lp-cli: `pool_meta.json` cache dla snapshot backtestów (decimals + tick_spacing)

**keywords:** clmm-lp-cli, snapshot-backtest-prep, backtest-optimize, pool_meta, decimals, tick_spacing, rpc, snapshots
**paths:** `crates/cli/src/commands/snapshot_backtest_prep.rs`, `crates/cli/src/main.rs`, `crates/cli/src/commands/snapshot_price_path.rs`

- `snapshot-backtest-prep` zapisuje pod `data/backtest-snapshot-cache/` plik `pool_meta.json` (na pool) z `token_mint_a/b` + `*_decimals` oraz `tick_spacing` pobranymi z Whirlpool state.
- `backtest-optimize` w trybie `--price-path-source snapshots` próbuje czytać `pool_meta.json` i przekazuje `*_decimals` jako override do `snapshot_price_path::build_from_orca_snapshots`, dzięki czemu (przy gotowym cache) wyniki są powtarzalne nawet gdy RPC jest blokowane.
- `tick_spacing` jest cache’owane dla spójności metadanych i przyszłych kroków; obecnie snapshot-fees backtests nie wymagają go do wyliczeń ceny/fee z samego `snapshots.jsonl`.

## 2026-04-02 — clmm-lp-cli: `backtest-optimize` nie robi RPC `fetch_pool_state` przy snapshotach opłat

**keywords:** clmm-lp-cli, backtest-optimize, snapshots, fee-source snapshots, fetch_pool_state, rpc
**paths:** `crates/cli/src/main.rs`

- W trybie `--price-path-source snapshots` + `--fee-source snapshots` (snapshot-fees jako źródło prawdy) pominięto `crate::commands::backtest_optimize::fetch_pool_state()` — to usuwa zależność od on-chain Orca pool state w backtestach i pozwala działać nawet przy blokowanych/publicznych endpointach RPC.
- Fallback do on-chain RPC zostaje, gdy nie używamy snapshot fees (np. gdy `fee_source != snapshots` lub snapshot fee model jest wyłączony).

## 2026-04-02 — API: `dotenv()` + env dla runnera skryptów

**keywords:** clmm-lp-api, dotenv, scripts runner, SCRIPT_RUNNER_URL, SCRIPT_RUNNER_TOKEN
**paths:** `crates/api/src/main.rs`, `crates/api/Cargo.toml`, `.env.example`

- `clmm-lp-api` nie ładował wcześniej root `.env` — dodano `dotenv().ok()` i zależność `dotenv`, żeby `SCRIPT_RUNNER_URL` / `SCRIPT_RUNNER_TOKEN` działały zgodnie z dokumentacją.
- Uzupełniono `/.env.example` o zmienne dla lokalnego `tools/script_runner`.

## 2026-04-01 — web: `@vitejs/plugin-react` ^6 (Vite 8 peer)

**keywords:** web, vite, @vitejs/plugin-react, npm, ERESOLVE
**paths:** `web/package.json`

- `vite@^8` kolidował z `@vitejs/plugin-react@4.x` (peer tylko do Vite 4–7). Podniesiono plugin do **^6**, zgodnie z peer `vite@^8`.

## 2026-04-01 — UI §1 skrypty: audyt w `UI_REQUIREMENTS_PHASE1.md`, modal „Logi” (excerpt runu)

**keywords:** UI_REQUIREMENTS_PHASE1, Scripts.tsx, SCRIPTS_CATALOG, script_runs.jsonl
**paths:** `doc/UI_REQUIREMENTS_PHASE1.md`, `web/src/pages/Scripts.tsx`

- Tabela **mapowanie wymagań → implementacja** dla §1; strona Scripts: link do `doc/SCRIPTS_CATALOG.md`, przycisk **Logi** (stdout/stderr/error z `last_run`).

## 2026-04-01 — GET /scripts: `resolve_repo_root` — spacer w górę od cwd / exe (pusta lista skryptów)

**keywords:** clmm-lp-api, CLMM_REPO_ROOT, scripts-manifest, GET /scripts
**paths:** `crates/api/src/handlers/scripts.rs`, `web/src/pages/Scripts.tsx`

- Gdy API startuje z podkatalogu (np. `web/`), wcześniej `current_dir` nie zawierał `tools/` → **0 skryptów**. Teraz: szukanie root po `tools/scripts-manifest.json` albo `Cargo.toml` + `tools/` + `crates/`, potem spacer od `current_exe` (`target/debug/…`).
- UI: komunikat gdy katalog pusty + pole `repo_root` (diagnostyka).

## 2026-04-01 — Scripts: API + runner — pełna lista `tools/*.ps1` (merge z manifestem), pole `auto_discovered`

**keywords:** clmm-lp-api, GET /scripts, ScriptCatalogItem, scripts-manifest, Start-ClmmScriptRunner, UI_REQUIREMENTS_PHASE1
**paths:** `crates/api/src/handlers/scripts.rs`, `crates/api/src/models.rs`, `tools/script_runner/Start-ClmmScriptRunner.ps1`, `web/src/pages/Scripts.tsx`, `doc/UI_REQUIREMENTS_PHASE1.md`

- `list_scripts`: skan top-level `tools/*.ps1`, merge z manifestem (ścieżka już w manifeście → bez duplikatu); bez manifestu — tylko skan. `POST /scripts/{id}/run`: manifest lub `tools/{id}.ps1`.
- Runner PS: `Resolve-ScriptEntry` — ten sam fallback; brak twardego wymogu manifestu przy starcie.

## 2026-04-01 — doc: Docker Desktop (Windows) troubleshooting, `UI_REQUIREMENTS_PHASE1` + status implementacji, confirm przy zamknięciu pozycji

**keywords:** docker, Docker Desktop, UI_REQUIREMENTS_PHASE1, PositionDetail, DOCKER.md, STARTUP.md
**paths:** `doc/DOCKER.md`, `doc/UI_REQUIREMENTS_PHASE1.md`, `STARTUP.md`, `web/src/pages/PositionDetail.tsx`

- Błąd pipe `dockerDesktopLinuxEngine` → uruchomić Docker Desktop; rozszerzony `UI_REQUIREMENTS_PHASE1.md` (środowisko + tabela statusu fazy 1).
- **Close position:** `window.confirm` przed `closePosition` (§5 destruktywne akcje).

## 2026-04-01 — Docker Compose: `api` + `web`, `API_UPSTREAM`, `doc/DOCKER.md`

**keywords:** docker, docker-compose, vite, API_UPSTREAM, VITE_DOCKER, CHOKIDAR_USEPOLLING, Makefile docker-up
**paths:** `docker-compose.yml`, `docker/api/Dockerfile`, `docker/web/Dockerfile`, `.dockerignore`, `web/vite.config.ts`, `doc/DOCKER.md`, `Makefile`, `STARTUP.md`

- Dev: bind mount `./web`, wolumen na `node_modules`; proxy Vite do `http://api:8080` zamiast localhost w kontenerze.
- Prod: osobno (build statyczny + nginx/CDN) — opisane krótko w `doc/DOCKER.md`.

## 2026-04-01 — web: `npm run dev:stack` — jeden terminal (concurrently + kill-port), uproszczony Start-Dashboard

**keywords:** web, npm, dev:stack, concurrently, kill-port, Start-Dashboard, vite, clmm-lp-api
**paths:** `web/scripts/start-dev-stack.mjs`, `web/package.json`, `tools/Start-Dashboard.ps1`, `Start-Dashboard.bat`, `STARTUP.md`

- Domyślny start: **`npm start`** (= `dev:stack`) — równoległe zwolnienie portów + `taskkill`/`pkill` API, baner (picocolors), concurrently z **`prefix: name`**, **`padPrefix`**, **`timings`**, kolory hex; opcjonalnie **`CLMM_OPEN_BROWSER=true`** → `vite --open`.
- `Start-Dashboard.bat` / `tools/Start-Dashboard.ps1`: `npm install` (jeśli brak `node_modules`) → **`npm start`**.

## 2026-04-01 — Windows: `Stop-ClmmApi.ps1`, zatrzymanie starego API przed `Start-Dashboard` (blokada `clmm-lp-api.exe`)

**keywords:** windows, cargo, clmm-lp-api, Start-Dashboard, Stop-ClmmApi, orca_read_service
**paths:** `tools/Stop-ClmmApi.ps1`, `tools/Start-Dashboard.ps1`, `crates/api/src/services/orca_read_service.rs`, `STARTUP.md`

- `cargo run` nie może nadpisać `.exe`, gdy proces nadal działa — skrypt stop + auto-stop w launcherze.
- `OrcaReadService`: `#[allow(dead_code)]` na polach zarezerwowanych pod REST (usuwa warning przy buildzie API).

## 2026-04-01 — Windows: `Start-Dashboard.bat` + baner „brak API” w Layout

**keywords:** web, vite, Start-Dashboard, CLMM_REPO_ROOT, ApiBackendBanner, proxy
**paths:** `Start-Dashboard.bat`, `tools/Start-Dashboard.ps1`, `web/src/components/ApiBackendBanner.tsx`, `web/src/components/Layout.tsx`, `STARTUP.md`

- Jedno kliknięcie uruchamia API (:8080) i Vite (:3000); `CLMM_REPO_ROOT` ustawione dla Scripts.
- Gdy backend nie odpowiada, czerwony baner + status w sidebarze zamiast „pustych” zakładek bez wyjaśnienia.

## 2026-04-01 — API + web: `GET /orca/positions-by-owner` (skan NFT jak `orca-positions-list`)

**keywords:** clmm-lp-api, orca_whirlpools, fetch_positions_for_owner, OrcaOwnerPositionsResponse, web, Positions, Wallet, RpcProvider
**paths:** `crates/api/src/handlers/orca_onchain.rs`, `crates/api/src/models.rs`, `crates/api/src/routes.rs`, `crates/api/src/openapi.rs`, `web/src/lib/api.ts`, `web/src/pages/Positions.tsx`, `web/src/pages/Wallet.tsx`, `web/src/components/ApiDataHint.tsx`

- Endpoint RPC dla portfela (query `owner` base58); osobna karta na **Positions**; **Wallet** pokazuje licznik on-chain gdy ustawiony `VITE_DEV_WALLET_PUBKEY`.

## 2026-04-01 — web: wyjaśnienie pustych list (monitor vs Orca), proxy Vite→8080, IL w analytics

**keywords:** web, vite, PositionMonitor, ApiDataHint, PortfolioAnalytics, total_il_pct, React Query
**paths:** `web/src/components/ApiDataHint.tsx`, `web/vite.config.ts`, `web/src/main.tsx`, `web/src/lib/api.ts`, `STARTUP.md`

- Vite proxy domyślnie na **port 8080** (jak API). `PortfolioAnalytics` w UI: **`total_il_pct`** (zgodnie z API), nie `total_il_usd`.
- Cache React Query: **stale 5 min**, **gc 30 min**. Karta **ApiDataHint** na Dashboard / Wallet / Positions — dlaczego brak pozycji przy aktywnych NFT na Orca (monitor ≠ skan portfela).

## 2026-04-01 — web: pinned dev wallet (`VITE_DEV_WALLET_PUBKEY`)

**keywords:** web, vite, VITE_DEV_WALLET_PUBKEY, devWallet, Wallet, Layout
**paths:** `web/src/lib/devWallet.ts`, `web/src/components/DevWalletBar.tsx`, `web/.env.example`, `web/.env.development`, `STARTUP.md`

- UI pokazuje stały pubkey z env; domyślnie `web/.env.development` (Vite dev), nadpisanie przez `web/.env.local`.

## 2026-04-01 — flows: IL ledger + API + web + whETH script (`CLMM_IL_LEDGER_PATH`)

**keywords:** bot-activity, il-ledger, CLMM_IL_LEDGER_PATH, il_ledger_path_from_env, get_bot_il_ledger, BotActivity, PositionDetail, wheth_sol_three_bots
**paths:** `crates/protocols/src/ledger/tx_lifecycle.rs`, `crates/api/src/handlers/bot_activity.rs`, `crates/api/src/routes.rs`, `web/src/pages/BotActivity.tsx`, `web/src/pages/PositionDetail.tsx`, `tools/wheth_sol_three_bots_manual_range_25_25p5.ps1`, `STARTUP.md`, `doc/ORCA_RUNBOOK.md`

- **`il_ledger_path_from_env()`** w protocols; **GET `/bot-activity/il-ledger`** — tail+filter jak lifecycle; brak env → odpowiedź „unset”.
- Web: karta **IL / rebalance ledger** na Bot activity; blok na szczegółach pozycji; `getBotIlLedger` w `api.ts`.
- Skrypt whETH 3× bot: `--il-ledger-path` per strategia (`il_ledger_bot_*.jsonl`).

## 2026-04-01 — IL ledger + CLI: wpisy `rebalance` (stary PDA, koszt tx, session id) + `ledger-rebalance-summary`

**keywords:** LifecycleTracker, RebalanceData, rebalance_session_id, tx_cost_lamports, old_position, ledger-rebalance-summary, CLMM_IL_LEDGER_PATH
**paths:** `crates/execution/src/lifecycle/tracker.rs`, `crates/execution/src/lifecycle/events.rs`, `crates/execution/src/strategy/rebalance.rs`, `crates/cli/src/commands/ledger_rebalance_summary.rs`, `crates/cli/src/main.rs`

- Wiersz JSONL `event: "rebalance"` (IL ledger): dopisane `old_position`, `tx_cost_lamports`, `rebalance_session_id` (z `CLMM_REBALANCE_SESSION_ID` jak w `orca_position_lifecycle.jsonl`).
- Nowa komenda CLI: **`ledger-rebalance-summary`** — zbiórka wierszy `rebalance` z `--il-ledger` / `CLMM_IL_LEDGER_PATH` oraz sumy po `rebalance_session_id` w lifecycle JSONL (`--lifecycle-ledger` lub domyślna ścieżka z env).

## 2026-04-01 — api + web: `POST /positions/{addr}/decrease` — `liquidity_amount` jako string (u128)

**keywords:** clmm-lp-api, DecreaseLiquidityRequest, decrease_liquidity, positions, web
**paths:** `crates/api/src/models.rs`, `crates/api/src/handlers/positions.rs`, `web/src/lib/api.ts`, `web/src/pages/PositionDetail.tsx`

- Ciało JSON: `liquidity_amount` jest **stringiem** dziesiętnym (pełny zakres u128 bez utraty precyzji w przeglądarce). UI: przycisk „Decrease liquidity” na szczegółach pozycji.

## 2026-04-01 — api + web + tools: manifest skryptów, `script_runs.jsonl`, runner localhost, strony Scripts / Wallet, ledger na pozycji

**keywords:** clmm-lp-api, web, scripts-manifest.json, script_runs.jsonl, SCRIPT_RUNNER_URL, tools/script_runner, UI_REQUIREMENTS_PHASE1, bot-activity ledger, rebalance_session_id
**paths:** `crates/api/src/handlers/scripts.rs`, `tools/scripts-manifest.json`, `tools/script_runner/Start-ClmmScriptRunner.ps1`, `web/src/pages/Scripts.tsx`, `web/src/pages/Wallet.tsx`, `doc/UI_REQUIREMENTS_PHASE1.md`

- REST: `GET /api/v1/scripts` (manifest + ostatni run z JSONL), `POST /api/v1/scripts/{id}/run` (proxy do runnera z `SCRIPT_RUNNER_*`). Konfiguracja: `CLMM_REPO_ROOT`, opcjonalnie runner URL/token.
- Runner PS: `tools/script_runner/Start-ClmmScriptRunner.ps1` — allowlista `tools/*.ps1`, zapis wierszy do `data/script_runs.jsonl`.
- Web: `/scripts`, `/wallet`; szczegół pozycji: zakładka timeline z `GET /bot-activity/ledger?filter=` i grupowanie po `rebalance_session_id`; akcje collect / rebalance / decrease / close.

## 2026-04-01 — doc + journal: historia rebalansów, koszty, `--il-ledger-path`, szablon tabeli

**keywords:** rebalance, il_ledger_path, CLMM_REBALANCE_SESSION_ID, orca_position_lifecycle.jsonl, OPERATIONS_JOURNAL, ORCA_RUNBOOK
**paths:** `doc/ORCA_RUNBOOK.md`, `data/experiments/wheth-sol-manual-range-25-25p5/OPERATIONS_JOURNAL.md`

- W `ORCA_RUNBOOK.md` dopisana podsekcja **„Historia rebalansów (liczenie zdarzeń) i kosztów tx”**: `--il-ledger-path` → wiersze `event: "rebalance"`; koszty nadal z lifecycle JSONL + suma po `rebalance_session_id`; eksplorator + opcjonalna tabela w dzienniku.
- W `OPERATIONS_JOURNAL.md` (eksperyment whETH/SOL) sekcja **„Rebalanse — historia i koszty”** z tabelą operatora i odnośnikami.

## 2026-04-01 — tools: `quick_verify_data.ps1` preferuje `target/release|debug/clmm-lp-cli.exe`

**keywords:** tools, quick_verify_data, Resolve-ClmmLpCliExe, cargo run, snapshot-readiness, data-health-check
**paths:** `tools/quick_verify_data.ps1`, `tools/clmm_rpc_tools_helpers.ps1`

- **Problem:** `cargo run` rebuilduje i nadpisuje `target/debug/clmm-lp-cli.exe` — przy uruchomionym procesie (blokada pliku) skrypt padał z „failed to remove … exe”.
- **Zmiana:** jeśli istnieje binarka z `Resolve-ClmmLpCliExe` (najpierw release), używamy jej zamiast `cargo run`; w logu widać linię `Using …clmm-lp-cli.exe`.

## 2026-04-01 — doc: szybki stan aktywnych pozycji + linki do wielu strategii / inspiracji zewnętrznych

**keywords:** POSITION_REGISTRY, OPERATIONAL_CONTINUITY, orca-positions-list, Bot activity, StrategyMode, WHETH_SOL
**paths:** `doc/POSITION_REGISTRY.md`, `doc/OPERATIONAL_CONTINUITY.md`

- W `POSITION_REGISTRY.md` dopisana tabela: **on-chain** (`orca-positions-list`) vs **registry/API** vs **monitor w `orca-bot-run`**; odsyłacze do `BACKTEST_OPTIMIZE_STRATEGIES`, `BOT_OPERATIONS_MODEL`, whETH multi-bot; krótki akapit o wzorcach spoza repo (Orca AI agents, ekosystemy z REST vaultów) przy zachowaniu priorytetu darmowego RPC.
- W `OPERATIONAL_CONTINUITY.md` w **Related** dodany link do `POSITION_REGISTRY.md`.

## 2026-04-01 — execution: rebalans częściowy (close OK, open fail) + monitor bez spamu po zamkniętym PDA

**keywords:** clmm-lp-execution, RebalanceResult, StrategyExecutor, PositionMonitor, rebalance, AccountNotFound
**paths:** `crates/execution/src/strategy/rebalance.rs`, `crates/execution/src/strategy/executor.rs`, `crates/execution/src/monitor/position_monitor.rs`

- **`RebalanceResult::old_position_closed_on_chain`:** ustawiane po udanym `close_position`; gdy `open` się nie uda, `success` pozostaje `false`, ale executor usuwa **stary** PDA z monitora (i czyści `retouch_armed` dla `RetouchShift`), żeby nie logować w kółko `Failed to get account`.
- **Monitor:** jeśli `get_position` zwraca błąd typu „brak konta” (łańcuch zawiera m.in. `AccountNotFound`), pozycja jest **usuwana** z monitora zamiast powtarzać `ERROR` co poll.

## 2026-03-31 — CLI: `orca-bot-open-and-run` zapisuje `registry_open` + `position_open` jak `orca-position-open`

**keywords:** orca-bot-open-and-run, registry.jsonl, orca_position_lifecycle.jsonl, position_registry, try_append_registry_open, WhirlpoolExecutor
**paths:** `crates/cli/src/commands/orca_bot.rs`, `crates/cli/src/commands/position_lifecycle_ledger.rs`

- **Problem:** Open wykonany wyłącznie przez SDK w `run_orca_bot_open_and_run` nie wywoływał ścieżek z `orca-position-open`, więc brakowało wierszy **`registry_open`** oraz **`position_open`** w ledgerze — kolektory oparte na `registry.jsonl` nie widziały NFT.
- **Zmiana:** Po udanym `open_position` dopisywane są te same rekordy co po CLI (`try_append_position_open_cost_ledger` z `source: "orca_bot"`, `try_append_registry_open(..., "orca_bot", ...)`). `try_append_position_open_cost_ledger` przyjmuje jawny parametr `source` (`"cli"` z `orca-position-open`).

## 2026-03-31 — doc: whETH/SOL — 2× bot ~10 USD, zakres 25–25,5, dziennik i koszty

**keywords:** WHETH_SOL, two-bots, 10USD, oor_recenter, periodic, ledger, ORCA_RUNBOOK, journal
**paths:** `doc/WHETH_SOL_TWO_BOTS_10USD.md`, `doc/WHETH_SOL_THREE_BOTS_FIRST_RUN.md`

- Plan operacyjny: **dwie** strategie (np. `winner-A` + `winner-B`), deploy **~10 USD** (ok. **5 USD/pozycja** heurystycznie), reszta salda na **fee rebalansów**; ticki **[-55416,-55216]**; monitoring/rebalans przez `orca-bot-run` z **`--execute`**. Dopisany przewodnik: tabele capów (przykład), ledger (`orca_position_lifecycle.jsonl`, registry, `CLMM_REBALANCE_SESSION_ID`), szablon dziennika.

## 2026-03-31 — doc: STARTUP — kolejność `KEYPAIR_PATH` / domyślny plik bota mainnet

**keywords:** KEYPAIR_PATH, SOLANA_KEYPAIR_PATH, clmm_lp_bot_mainnet.json, STARTUP, operations
**paths:** `STARTUP.md` (sekcja Bot keypair)

- Dopisane: **resolution order** dla skryptów/CLI, przypomnienie o braku commitu keypaira, **szybki health check** (istnienie pliku + JSON 64 elementów / opcjonalnie `solana-keygen pubkey`).

## 2026-03-31 — ops: whETH/SOL 3× bot — ręczny zakres 25–25.5 SOL/whETH + JSON strategii

**keywords:** orca-bot-open-and-run, optimize-result-json, WHETH_SOL, oor_recenter, periodic, threshold, tools
**paths:** `data/experiments/wheth-sol-manual-range-25-25p5/winner-*.json`, `tools/wheth_sol_three_bots_manual_range_25_25p5.ps1`, `doc/WHETH_SOL_THREE_BOTS_FIRST_RUN.md`

- Dla puli `Hktf…` (mint A = SOL, mint B = whETH) ticki **[-55416, -55216]** odpowiadają w przybliżeniu **25.0–25.5 SOL za 1 whETH** (weryfikacja: `orca-pool-read`). Trzy pliki `OptimizeResultFile` (schema v1): **oor_recenter**, **periodic 12h**, **threshold 5%**; `width_pct` = **0.02** (~2%) dla spójności z szerokością pasma. Skrypt PowerShell drukuje gotowe komendy `orca-bot-open-and-run` (osobne terminale).

## 2026-03-31 — API: `POST /analytics/simulate` — pełna symulacja (`clmm_lp_simulation`)

**keywords:** clmm-lp-api, analytics, simulate, clmm_lp_simulation, GBM, WhirlpoolReader
**paths:** `crates/api/src/services/simulation_analytics.rs`, `crates/api/src/handlers/analytics.rs`, `crates/api/src/models.rs`, `web/src/lib/api.ts`

- Endpoint **nie jest już placeholderem**: pobiera stan puli z RPC (`WhirlpoolReader`), buduje zakres z `tick_to_price`, generuje **syntetyczną ścieżkę dzienna GBM** od aktualnej ceny, uruchamia `simulate_with_strategy` (strategie z `StrategyType` + domyślne progi), zwraca m.in. `time_in_range_pct`, `vs_hodl_usd`, `methodology_note`.
- `SimulationRequest` rozszerzony o `gbm_volatility` / `gbm_drift` / parametry strategii; `SimulationResponse` o pola jawnej metodologii. Historyczny replay jak `backtest` CLI nadal osobno — opis w `methodology_note`.

## 2026-03-31 — doc: WHETH 3× bot — CLI vs API optimize + symulacja zakresu

**keywords:** backtest-optimize, clmm-lp-cli, clmm-lp-api, analytics simulate, WHETH_SOL_FIRST_RUN, optimize_profile
**paths:** `doc/WHETH_SOL_THREE_BOTS_FIRST_RUN.md`, `crates/api/src/handlers/analytics.rs`, `crates/api/src/services/optimization_runner.rs`

- Dopisane sekcje: **skąd jest optimize** (siatka tylko w CLI; API = subprocess `backtest-optimize` lub `apply-optimize-result`; `POST /analytics/simulate` = placeholder), oraz **procedura symulacji doboru `width_pct`/parametrów** przed openami.

## 2026-03-31 — API + web: historia bota (JSONL) + Slack digest

**keywords:** clmm-lp-api, web, bot-activity, Slack, SLACK_WEBHOOK_URL, orca_position_lifecycle.jsonl, registry.jsonl, OPERATIONAL_CONTINUITY
**paths:** `crates/api/src/handlers/bot_activity.rs`, `web/src/pages/BotActivity.tsx`, `web/src/lib/api.ts`, `doc/OPERATIONAL_CONTINUITY.md`

- **GET** `/api/v1/bot-activity/ledger` i `/registry` — ostatnie wiersze JSONL (opcjonalnie `?filter=` substring, `?limit=`), te same ścieżki co CLI (`CLMM_POSITION_LIFECYCLE_LEDGER_PATH`, `CLMM_POSITION_REGISTRY_PATH`).
- **POST** `/api/v1/bot-activity/slack-summary` — treść z ogonu ledgera → Incoming Webhook (`SLACK_WEBHOOK_URL` w env procesu API).
- **Web:** strona **Bot activity** (`/bot-activity`) — tabele + przycisk wysyłki na Slack.

## 2026-03-31 — doc: pierwsze uruchomienie 3× bot whETH/SOL (strategie + dziennik)

**keywords:** WHETH_SOL, orca-bot-run, StrategyMode, BOT_OPERATIONS_MODEL, ORCA_RUNBOOK, operations
**paths:** `doc/WHETH_SOL_THREE_BOTS_FIRST_RUN.md`, `doc/ORCA_RUNBOOK.md`

- Nowy przewodnik: propozycja zestawu strategii (`oor_recenter` / `periodic` / `threshold` + alternatywy), bezpieczna sekwencja (dry-run → jeden live na raz), szablon tabeli dziennika i checklist; link z `ORCA_RUNBOOK.md`.

## 2026-03-31 — tools: `orca_wheth_sol_three_bots_plan.ps1` (plan 3× pozycja WHETH/SOL)

**keywords:** tools, WHETH_SOL, orca_curated_rebalance, orca-bot-run, capital plan, SCRIPTS_CATALOG
**paths:** `tools/orca_wheth_sol_three_bots_plan.ps1`, `doc/SCRIPTS_CATALOG.md`

- Skrypt **nie wysyła tx**: czyta `solana_account_state.ps1 -Json`, ceny CoinGecko (SOL/ETH), liczy heurystyczne **`AmountA`/`AmountB` na bot** (DeployUsd/NumBots, split 50/50 USD na nogi), sprawdza braki względem **3×** caps + `ReserveSolLamports`; drukuje przykładowe komendy `orca_curated_rebalance -Action Open` i `orca-bot-run`.

## 2026-03-31 — tools: `solana_wallet_usd_estimate.ps1` (portfel w USD)

**keywords:** tools, solana_wallet_usd_estimate, solana_account_state, CoinGecko, USDC, portfolio, SCRIPTS_CATALOG
**paths:** `tools/solana_wallet_usd_estimate.ps1`, `doc/SCRIPTS_CATALOG.md`

- Skrypt woła **`solana_account_state.ps1 -Json`**, sumuje **native SOL + wSOL (ATA)** jako jedną linię, SPL po **mincie**; ceny: **USDC/USDT = 1 USD**, **SOL / cbBTC (jako BTC) / whETH (jako ETH)** z **CoinGecko** `simple/price`; minty bez mapowania → **0 USD** + `unpriced_mint`. Opcja **`-Json`** (jedna linia), **`-SkipPriceFetch`** tylko kwoty UI.

## 2026-03-31 — tools: `orca_curated_rebalance.ps1` + `-OpenOnly` w `orca_position_open_then_close_quick.ps1`

**keywords:** tools, orca_curated_rebalance, orca_swap_curated, orca_position_open_then_close_quick, OpenOnly, curated, ORCA_RUNBOOK, SCRIPTS_CATALOG
**paths:** `tools/orca_curated_rebalance.ps1`, `tools/orca_position_open_then_close_quick.ps1`, `doc/ORCA_RUNBOOK.md`, `doc/SCRIPTS_CATALOG.md`

- **Dispatcher** dla trzech par curated (`SOL_USDC`, `WHETH_SOL`, `CBBTC_USDC`): akcje Help / ListPairs / Preflight / Open (samo `orca-position-open`) / Close / Swap (forward do `orca_swap_curated.ps1`) / FundCbBtc (`orca_fund_cbbtc_usdc_open.ps1`) / Smoke (`orca_position_smoke_curated_pools.ps1`).
- **`orca_position_open_then_close_quick.ps1 -OpenOnly`:** po udanym open pomija sleep/close/verify — pozycja zostaje na łańcuchu (rebalans / produkcja).

## 2026-03-31 — tools: uniwersalny build CLI (`build_clmm_lp_cli.ps1`) + wrapper release

**keywords:** tools, powershell, build_clmm_lp_cli, build_clmm_lp_cli_release, clmm-lp-cli, cargo, SCRIPTS_CATALOG, ORCA_RUNBOOK
**paths:** `tools/build_clmm_lp_cli.ps1`, `tools/build_clmm_lp_cli_release.ps1`, `doc/SCRIPTS_CATALOG.md`, `doc/ORCA_RUNBOOK.md`

- **`build_clmm_lp_cli.ps1`:** `cargo build` dla **`clmm-lp-cli`** z **`-Configuration Release`** (domyślnie) lub **`Debug`**; wypisuje ścieżkę do `target\release|debug\clmm-lp-cli.exe`.
- **`build_clmm_lp_cli_release.ps1`:** cienki wrapper wołający Release — zachowane stare odwołania w skryptach i doku.

## 2026-03-31 — tools: `orca_fund_cbbtc_usdc_open.ps1` — dopłaty po swapach + wyższy domyślny bufor USDC

**keywords:** tools, orca_fund_cbbtc_usdc_open, UsdcHeadroomBps, post-swap, preflight, cbBTC, USDC, ORCA_RUNBOOK
**paths:** `tools/orca_fund_cbbtc_usdc_open.ps1`

- Po zaplanowanych swapach **SOL/USDC** i **cbBTC/USDC** skrypt w pętli (domyślnie do **6** rund, param **`PostSwapTopUpMaxRounds`**) ponownie liczy preflight; jeśli brakuje tylko **USDC**, robi **exact-out USDC** na puli SOL/USDC z buforem **`UsdcHeadroomBps`**; jeśli brakuje tylko **cbBTC**, robi **exact-out cbBTC** na puli cbBTC/USDC — żeby zamknąć lukę między dry-run quote a faktycznym kosztem on-chain.
- Domyślne **`UsdcHeadroomBps`** podniesione **300 → 600** (nadal nadpisywalne z CLI).

## 2026-03-31 — tools: `orca_swap.ps1` — `Start-Process` zamiast `& cargo 2>&1` (stderr / exit code)

**keywords:** tools, powershell, orca_swap, orca_fund_cbbtc_usdc_open, cargo, Start-Process, NativeCommandError, ORCA_RUNBOOK
**paths:** `tools/orca_swap.ps1`

- Wywołanie `clmm-lp-cli` (exe lub `cargo run`) idzie przez **Start-Process** z przekierowaniem stdout/stderr do plików tymczasowych; linie są potem drukowane przez **Write-Host**.
- Cel: komunikaty cargo/rust na stderr nie trafiają do strumienia błędów hosta jako **NativeCommandError**, a kod wyjścia pochodzi z **ExitCode** procesu (stabilniejsze niż `$LASTEXITCODE` po merge `2>&1` w zagnieżdżonych `-File`), m.in. dla `orca_fund_cbbtc_usdc_open.ps1 -Execute`.

## 2026-03-31 — tools + ops: `data_alerts_loop.ps1` + alternatywy dla Task Scheduler

**keywords:** tools, data_alerts_loop, Shawl, NSSM, systemd, snapshot_health_alert, quick_verify_alert, OPERATIONAL_CONTINUITY
**paths:** `tools/data_alerts_loop.ps1`, `doc/OPERATIONAL_CONTINUITY.md`, `tools/README.md`, `deploy/systemd/clmm-lp-data-alerts-loop.service.example`, `deploy/README.md`, `doc/SCRIPTS_CATALOG.md`

- **`data_alerts_loop.ps1`:** jeden długo żyjący proces z interwałami snapshot vs quick-verify; log `data/snapshot_logs/data-alerts-loop.log`; zamiennik harmonogramu Windows.
- **Dokumentacja:** Shawl/NSSM, opcjonalnie Task Scheduler; Linux: `systemd` + `pwsh` (przykładowa jednostka w `deploy/systemd/`).

## 2026-03-31 — tools: `quick_verify_alert.ps1` (GO/NO-GO → Slack + throttle)

**keywords:** tools, quick_verify_data, quick_verify_alert, slack, snapshot-readiness, data-health-check, SCRIPTS_CATALOG
**paths:** `tools/quick_verify_alert.ps1`, `doc/SCRIPTS_CATALOG.md`, `tools/README.md`, `doc/OPERATIONAL_CONTINUITY.md`

- Wrapper woła `quick_verify_data.ps1`; przy exit≠0 (w tym **exit 2** NO-GO) wysyła `notify_slack_webhook.ps1`; throttle `MinMinutesBetweenSameIssues` (domyślnie **60 min**) w `data/agent-alerts/quick-verify-slack-throttle/`; catch na throw z `quick_verify_data`.

## 2026-03-31 — doc + tools: katalog skryptów (`SCRIPTS_CATALOG`) + `snapshot_health_alert` → Slack

**keywords:** scripts, SCRIPTS_CATALOG, snapshot-health, snapshot_health_alert, notify_slack_webhook, tools/README, OPERATIONAL_CONTINUITY
**paths:** `doc/SCRIPTS_CATALOG.md`, `doc/README.md`, `doc/OPERATIONAL_CONTINUITY.md`, `tools/README.md`, `tools/snapshot_health_alert.ps1`

- **`doc/SCRIPTS_CATALOG.md`:** spis `tools/*.ps1` z kolumną keywords, sekcja P0 (snapshot/ciągłość/jakość), CLI powiązane, uwaga na `scripts/` w `.gitignore`.
- **`tools/snapshot_health_alert.ps1`:** woła `snapshot_health_check.ps1`; przy exit≠0 Slack przez `notify_slack_webhook.ps1`; throttle `MinMinutesBetweenSameIssues` (domyślnie 15 min); stan w `data/agent-alerts/snapshot-slack-throttle/`.
- **`tools/README.md`:** skrót + link do katalogu.

## 2026-03-31 — tools: `orca_fund_cbbtc_usdc_open.ps1` (quote dry-run + opcjonalne swapy pod open)

**keywords:** tools, powershell, orca_fund_cbbtc_usdc_open, orca-swap, dry-run, quote, cbBTC, USDC, SOL_USDC, ORCA_RUNBOOK
**paths:** `tools/orca_fund_cbbtc_usdc_open.ps1`, `doc/ORCA_RUNBOOK.md`

Skrypt szacuje braki do `orca-position-open` na puli **cbBTC/USDC** (`HxA6…`): parsuje `token_est_in` / `token_est_out` z stdout `orca-swap --dry-run`, planuje **SOL/USDC** exact-out USDC potem **cbBTC/USDC** exact-out cbBTC; **`-Execute`** woła `orca_swap.ps1`.

## 2026-03-31 — tools: curated Orca swapy (3 pary, wszystkie nogi) + wspólna lista pul

**keywords:** tools, powershell, orca_swap_curated, orca_curated_mainnet_pools, orca_swap, CargoOnly, SOL_USDC, WHETH_SOL, CBBTC_USDC, ORCA_RUNBOOK
**paths:** `tools/orca_curated_mainnet_pools.ps1`, `tools/orca_swap_curated.ps1`, `tools/orca_swap.ps1`, `tools/orca_position_smoke_curated_pools.ps1`, `doc/ORCA_RUNBOOK.md`

- **`orca_curated_mainnet_pools.ps1`:** jedna definicja trzech pul (mint_a/b, symbole) zgodna z `orca-pool-read`.
- **`orca_swap_curated.ps1`:** `-From`/`-To` + `-SwapType` → `--specified-mint` / `exact-in|exact-out`; `-ListPairs`.
- **`orca_swap.ps1`:** `-CargoOnly`, preferencja `Resolve-ClmmLpCliExe`, błąd przy niezerowym `LASTEXITCODE`.
- **Smoke curated** buduje listę pul z `orca_curated_mainnet_pools.ps1`.

## 2026-03-31 — tools: Slack Incoming Webhook helper (`notify_slack_webhook.ps1`)

**keywords:** tools, powershell, slack, SLACK_WEBHOOK_URL, alerts, OPERATIONAL_CONTINUITY
**paths:** `tools/notify_slack_webhook.ps1`, `doc/OPERATIONAL_CONTINUITY.md`

- Skrypt wysyła `text` przez **Incoming Webhook** (kolejność: `-WebhookUrl`, env `SLACK_WEBHOOK_URL`, potem parsowanie **`SLACK_WEBHOOK_URL=` z repo-root `.env`**; opcjonalnie `-DotEnvPath`). Instrukcja Slack w `doc/OPERATIONAL_CONTINUITY.md` (sekcja Slack).

## 2026-03-31 — tools: `solana_account_state.ps1 -Json` jako jedna linia (parsowanie preflight)

**keywords:** tools, powershell, solana_account_state, ConvertTo-Json, orca_position_preflight_core, preflight
**paths:** `tools/solana_account_state.ps1`

Tryb `-Json` wypisywał wieloliniowy JSON; `orca_position_open_preflight.ps1` brał pierwszą linię zaczynającą się od `{` (sam `{`), więc `ConvertFrom-Json` padał. Na stdout **-Json** używa teraz `ConvertTo-Json -Compress` (jedna linia); zapis `-OutJson` nadal pretty-print.

## 2026-03-31 — tools: auto-fund (exact-out swap) przed Orca position open

**keywords:** tools, powershell, orca_position_preflight_core, Invoke-OrcaPositionAutoFundFromPool, AutoFund, orca-swap, exact-out, orca_position_auto_fund_for_open, orca_position_open_then_close_quick, orca_position_open_then_close_fast, orca_position_smoke_curated_pools, ORCA_RUNBOOK
**paths:** `tools/orca_position_preflight_core.ps1`, `tools/orca_position_open_preflight.ps1`, `tools/orca_position_auto_fund_for_open.ps1`, `tools/orca_position_open_then_close_quick.ps1`, `tools/orca_position_open_then_close_fast.ps1`, `tools/orca_position_smoke_curated_pools.ps1`, `doc/ORCA_RUNBOOK.md`

- **`orca_position_preflight_core.ps1`:** `Get-OrcaPositionOpenPreflightState`, `Test-OrcaPositionOpenPreflight`, `Invoke-OrcaPositionAutoFundFromPool` (pętla exact-out na tej samej puli do momentu OK preflightu).
- **`orca_position_open_preflight.ps1`:** tylko param + standalone; wspólna logika w core.
- **`orca_position_auto_fund_for_open.ps1`:** samo auto-fund + końcowy test preflight (bez `orca-position-open`).
- **Quick / fast / smoke:** opcjonalnie **`-AutoFund`** i parametry bufora/slippage/max rund.
- **`doc/ORCA_RUNBOOK.md`:** pod auto-fund dopisane **planowanie swapów** (stała kolejność A-then-B, zapas nogi płacącej, dual-deficit, SOL reserve).

## 2026-03-31 — tools: preflight open + cbBTC/USDC w smoke curated

**keywords:** tools, powershell, orca_position_open_preflight, orca_position_open_then_close_quick, orca_position_open_then_close_fast, orca_position_smoke_curated_pools, SkipPreflight, ReserveSolLamports, cbBTC, ORCA_RUNBOOK
**paths:** `tools/orca_position_open_preflight.ps1`, `tools/orca_position_open_then_close_quick.ps1`, `tools/orca_position_open_then_close_fast.ps1`, `tools/orca_position_smoke_curated_pools.ps1`, `doc/ORCA_RUNBOOK.md`

- `orca_position_open_preflight.ps1`: prawdziwy mint **cbBTC** w etykietach; blok standalone uruchamia się tylko gdy skrypt **nie** jest dot-sourced (`$MyInvocation.InvocationName -ne '.'`), żeby `. .\…preflight.ps1` nie robił `Set-Location`/`exit` w sesji nadrzędnej.
- **Quick** i **fast** open→close: przed `orca-position-open` domyślnie wywołanie preflightu; `-SkipPreflight`, `-ReserveSolLamports` (przekazywane także ze smoke).
- **Smoke curated:** trzeci pool `cbBTC/USDC` (`HxA6SKW5qA4o12fjVgTpXdq2YnZ5Zv1s7SB4FFomsyLM`) jak w `STARTUP.md`.
- **`doc/ORCA_RUNBOOK.md`:** smoke + preflight + usunięcie sprzecznej uwagi „HxA6 nie z STARTUP”.

## 2026-03-31 — ops: ciągłość operacyjna bota (dokument + systemd + Docker + supervised PS1)

**keywords:** operations, orca-bot-run, systemd, docker-compose, Task Scheduler, OPERATIONAL_CONTINUITY, orca_bot_run_supervised
**paths:** `doc/OPERATIONAL_CONTINUITY.md`, `doc/ORCA_RUNBOOK.md`, `doc/README.md`, `doc/MAINNET_OPERATIONAL_CHECKLIST.md`, `deploy/systemd/clmm-lp-orca-bot.service.example`, `deploy/README.md`, `Docker/orca-bot.compose.example.yml`, `Docker/README.md`, `tools/orca_bot_run_supervised.ps1`

- Nowy runbook: **`doc/OPERATIONAL_CONTINUITY.md`** (superwizja procesu, logi, haki alertów, RPC/klucze, checklist).
- **Linux:** szablon `deploy/systemd/clmm-lp-orca-bot.service.example`.
- **Docker:** przykład `Docker/orca-bot.compose.example.yml` (`restart: unless-stopped`, volume na keypair + ledgery).
- **Windows:** `tools/orca_bot_run_supervised.ps1` — pętla restartu po niezerowym exit code; opcjonalnie `-LogDir` (ostrzeżenie dla Windows PowerShell 5.x w kwestii `$LASTEXITCODE` po `Tee-Object`).

## 2026-03-31 — tools: Orca quick (release exe) + `orca_position_smoke_curated_pools` + helper w swap/snapshot verify

**keywords:** tools, powershell, Resolve-ClmmLpCliExe, Invoke-ClmmLpCliStream, orca_position_open_then_close_quick, orca_position_smoke_curated_pools, build_clmm_lp_cli_release, orca_swap, quick_verify_data, run_snapshot_backtest_prep_loop, ORCA_RUNBOOK
**paths:** `tools/clmm_rpc_tools_helpers.ps1`, `tools/orca_position_open_then_close_quick.ps1`, `tools/orca_position_smoke_curated_pools.ps1`, `tools/build_clmm_lp_cli_release.ps1`, `tools/orca_swap.ps1`, `tools/quick_verify_data.ps1`, `tools/run_snapshot_backtest_prep_loop.ps1`, `doc/ORCA_RUNBOOK.md`

- Rozszerzono `clmm_rpc_tools_helpers.ps1` o **Resolve-ClmmLpCliExe**, **Invoke-ClmmLpCliStream**, **Invoke-ClmmLpCliCapture**; `orca_position_open_then_close_quick.ps1` używa release/debug exe gdy istnieje (`-CargoOnly` → zawsze `cargo run`).
- Nowe: **`tools/orca_position_smoke_curated_pools.ps1`** (open+close dla pooli jak w `STARTUP.md` Orca), **`tools/build_clmm_lp_cli_release.ps1`**.
- **Initialize-ClmmToolsRpcEnv** także w `orca_swap.ps1`, `quick_verify_data.ps1`, `run_snapshot_backtest_prep_loop.ps1`.
- **`doc/ORCA_RUNBOOK.md`:** sekcja smoke + rozróżnienie poola z `quick_verify` vs curated.

## 2026-03-31 — tools: Orca ops — `clmm_rpc_tools_helpers.ps1` + close slippage w quick + hint 6018

**keywords:** tools, powershell, CLMM_RPC_DENYLIST, orca-position-close, slippage_bps, execution_ok, TokenMinSubceeded
**paths:** `tools/clmm_rpc_tools_helpers.ps1`, `tools/orca_position_close_quick.ps1`, `tools/orca_position_open_then_close_quick.ps1`, `tools/orca_position_open_then_close_fast.ps1`, `crates/cli/src/commands/orca_position.rs`

- `Initialize-ClmmToolsRpcEnv`: gdy `CLMM_RPC_DENYLIST` jest puste i `SOLANA_RPC_URL` wygląda na mainnet, ustawia `ankr,projectserum`, żeby domyślne fallbacki w `RpcConfig` omijały często blokowane URL-e.
- `orca_position_close_quick.ps1` / `orca_position_open_then_close_quick.ps1`: domyślnie wyższy slippage na close (`-SlippageBps` / `-CloseSlippageBps`, 500 bps).
- `execution_ok` dopina hint przy tekście błędu z **6018** / **0x1782**.

## 2026-03-31 — tools: `restart_snapshot_loop_10m.ps1` (pin RPC; nie dotyka 5m)

**keywords:** tools, powershell, snapshot-loop, run-snapshot-loop, SOLANA_RPC_URL, snapshot_logs
**paths:** `tools/restart_snapshot_loop_10m.ps1`, `scripts/windows/run-snapshot-loop.ps1`

Skrypt zatrzymuje proces PowerShell uruchomiony z `scripts/windows/run-snapshot-loop.ps1` (bez `run-snapshot-loop-5m.ps1`) i startuje pętlę ponownie z domyślnym pinem RPC jak w skrypcie. Jeśli stara pętla działa pod Task Scheduler/NSSM w innej sesji, wyłącz duplikat ręcznie.

## 2026-03-30 — RPC: hard-disable paid/auth endpoints + optional denylist guard

**keywords:** clmm-lp-protocols, rpc, failover, health, 402, Payment Required, denylist, SOLANA_RPC_URL, SOLANA_RPC_FALLBACK_URLS, CLMM_RPC_DENYLIST
**paths:** `crates/protocols/src/rpc/provider.rs`, `crates/protocols/src/rpc/health.rs`, `crates/protocols/src/rpc/config.rs`

RPC failover now **hard-disables** endpoints that return HTTP auth/paywall failures (402/401/403) to avoid repeated rotation into dead URLs causing snapshot gaps. Added optional env `CLMM_RPC_DENYLIST` (comma-separated substrings) to filter such endpoints up-front, plus a startup warning when only one endpoint remains after config/denylist.

## 2026-03-31 — CLI: `orca-position-close --slippage-bps` (Whirlpool 6018 / TokenMinSubceeded)

**keywords:** clmm-lp-cli, orca-position-close, WhirlpoolExecutor, close_position_instructions, slippage_bps, TokenMinSubceeded, 6018, tools, orca_position_open_then_close_fast.ps1
**paths:** `crates/protocols/src/orca/executor.rs`, `crates/cli/src/commands/orca_position.rs`, `crates/cli/src/main.rs`, `crates/api/src/services/orca_tx_service.rs`, `tools/orca_position_open_then_close_fast.ps1`

`WhirlpoolExecutor::close_position` przyjmuje opcjonalny `slippage_bps` (domyślnie jak wcześniej: **100** bps przez `None` / brak flagi). CLI `orca-position-close` ma `--slippage-bps`; `ClosePositionTxRequest` ma `slippage_bps: Option<u16>`. Skrypt `orca_position_open_then_close_fast.ps1` przekazuje domyślnie **500** bps na close (`-CloseSlippageBps`), żeby unikać błędu on-chain **6018** (*TokenMinSubceeded*) przy bardzo małej płynności / szybkim open→close.

## 2026-03-30 — tools: `orca_position_open_then_close_fast.ps1` (close bez czekania na ledger)

**keywords:** tools, powershell, orca-position-open, orca-position-close, timing, confirm->confirm, ledger, getTransaction
**paths:** `tools/orca_position_open_then_close_fast.ps1`

Dodano skrypt `tools/orca_position_open_then_close_fast.ps1`, który startuje `close` natychmiast po wypisaniu `position PDA:` z `open` (nie czeka na post-tx enrichment ledgera, który na public RPC potrafi lagować), i mierzy czas confirm->confirm z timestampów logów `Transaction confirmed signature=...`.

## 2026-03-30 — tools: `orca_position_open_then_close_quick.ps1` mierzy czas confirm->confirm

**keywords:** tools, powershell, orca-position-open, orca-position-close, automation, timing, confirm->confirm
**paths:** `tools/orca_position_open_then_close_quick.ps1`

Skrypt `tools/orca_position_open_then_close_quick.ps1` został rozszerzony o pomiar czasu pomiędzy momentem pojawienia się `signature:` dla open a dla close (na podstawie streamingowego odczytu stdout/stderr cargo), żeby nie mieszać tego z dodatkowym enrichmentem ledgera.

## 2026-03-30 — tools: `orca_position_close_quick.ps1` (szybkie zamknięcie z registry)

**keywords:** tools, powershell, orca-position-close, registry.jsonl, position_registry, automation
**paths:** `tools/orca_position_close_quick.ps1`, `crates/protocols/src/ledger/position_registry.rs`

Dodano skrypt `tools/orca_position_close_quick.ps1`: wybiera ostatnio aktywną pozycję (`registry_open` bez późniejszego `registry_close`) dla właściciela i odpala jedną komendę `clmm-lp-cli orca-position-close` z gotowym `--position` i `--keypair`.

## 2026-03-30 — tools: `orca_position_open_then_close_quick.ps1` (open→close jednym kliknięciem)

**keywords:** tools, powershell, orca-position-open, orca-position-close, automation, Whirlpool
**paths:** `tools/orca_position_open_then_close_quick.ps1`

Dopisano skrypt automatyzujący flow: `orca-position-open` (live, małe `--amount-a/--amount-b`) → parsowanie `position PDA` z outputu → krótki sleep → `orca-position-close` oraz opcjonalna weryfikacja `orca-positions-list entries=0`.

## 2026-03-30 — CLI: `orca-position-close` dopisuje token refund delty (A/B) do ledgera

**keywords:** clmm-lp-cli, orca-position-close, position_lifecycle_ledger, jsonl, token_delta, preTokenBalances, postTokenBalances
**paths:** `crates/cli/src/commands/position_lifecycle_ledger.rs`

Do wierszy `event=position_close` w `data/ledger/orca_position_lifecycle.jsonl` dopisano best-effort delty tokenów A/B (`token_a_net_delta_*`, `token_b_net_delta_*`) jako `post - pre` dla fee-payera (owner) liczonych z `meta.preTokenBalances`/`meta.postTokenBalances`. Dzięki temu “zwroty” są widoczne w ilościach (base units + UI), obok dotychczasowych kosztów SOL/fees.

## 2026-03-30 — Orca: rozróżnienie pool (653 B) vs position PDA (216 B) + komunikat w `PositionReader`

**keywords:** clmm-lp-protocols, clmm-lp-cli, orca-position-close, Whirlpool, OpenPositionWithTokenExtensions, position_reader, pool vs position
**paths:** `crates/protocols/src/orca/position_reader.rs`, `crates/cli/src/main.rs`, `doc/POSITION_REGISTRY.md`

`PositionReader::get_position` wykrywa podanie konta **puli** Whirlpool (653 B + discriminator puli) zamiast **PDA pozycji** (216 B) i zwraca czytelny błąd (kolejność kont w `OpenPositionWithTokenExtensions` na Solscan). `doc/POSITION_REGISTRY.md` — sekcja „Pula vs PDA”; help CLI `orca-position-close --position` doprecyzowany.

## 2026-03-30 — STARTUP: Shawl/NSSM — druga usługa dla pętli snapshotów 5m

**keywords:** STARTUP, Shawl, NSSM, Windows Service, run-snapshot-loop, run-snapshot-loop-5m, snapshots_5m, snapshot-loop-5m.log
**paths:** `STARTUP.md`

W sekcji *Alternatives to Task Scheduler* dopisano **drugą** usługę równoległą do `clmm-snapshot-loop`: **`clmm-snapshot-loop-5m`** (`run-snapshot-loop-5m.ps1` → `snapshots_5m.jsonl`, log `snapshot-loop-5m.log`). Tabela NSSM i osobne `shawl add` dla 10m vs 5m; ścieżki nadal wymagają dopasowania do lokalnego klonu.

## 2026-03-30 — `position_registry.jsonl`: otwarte/zamknięte pozycje + sygnał dla kolektorów

**keywords:** clmm-lp-protocols, clmm-lp-cli, clmm-lp-execution, position_registry, registry_open, registry_close, collectors, jsonl, CLMM_POSITION_REGISTRY_PATH
**paths:** `crates/protocols/src/ledger/position_registry.rs`, `crates/cli/src/commands/orca_position.rs`, `crates/execution/src/strategy/rebalance.rs`, `doc/POSITION_REGISTRY.md`

Dodano append-only **`data/positions/registry.jsonl`** (`CLMM_POSITION_REGISTRY_PATH`): `registry_open` / `registry_close`, `source` cli vs `orca_bot`, opcjonalnie `rebalance_session_id`. CLI `orca-position-open` / `close` oraz udane open/close w **`RebalanceExecutor`** dopisują wiersze — kolektory mogą wyliczać aktywne pozycje (ostatni event per `position_pubkey`) i **kończyć** zbieranie danych per pozycja po `registry_close`. Dokumentacja: `doc/POSITION_REGISTRY.md`.

## 2026-03-30 — ORCA_RUNBOOK: rebalance (ticki), swap vs `RebalanceExecutor`, `CLMM_REBALANCE_SESSION_ID`

**keywords:** ORCA_RUNBOOK, rebalance, Whirlpool, tick range, close position, open position, orca-swap, CLMM_REBALANCE_SESSION_ID, RebalanceExecutor
**paths:** `doc/ORCA_RUNBOOK.md`

Rozszerzono runbook: **immutable** zakres ticków na jednym NFT Whirlpool → typowy flow collect → decrease → close → open (nowy PDA); alternatywa dwóch pozycji; **`RebalanceExecutor`** bez wbudowanego swapu — swap przez CLI/skrypt + ledger `cli_swap`; **`CLMM_REBALANCE_SESSION_ID`** jako spójne sumowanie kosztów w jednej sesji; przyszłość: id sesji z konfiguracji/UUID w Rust zamiast wyłącznie env.

## 2026-03-30 — Ledger: `cli_swap` + `CLMM_REBALANCE_SESSION_ID` (pełny koszt swap + rebalans + open)

**keywords:** clmm-lp-protocols, clmm-lp-cli, orca-swap, tx_lifecycle, rebalance_session_id, jsonl, fee_payer_net_lamports_delta
**paths:** `crates/protocols/src/ledger/tx_lifecycle.rs`, `crates/cli/src/commands/orca_swap.rs`, `crates/cli/src/commands/position_lifecycle_ledger.rs`

Po udanym **`orca-swap`** dopisywany jest wiersz do tego samego pliku co lifecycle (`event=cli_swap`, `operation=orca_whirlpool_swap`, `source=cli`). Opcjonalnie **`CLMM_REBALANCE_SESSION_ID`** (to samo wartość w całej sekwencji: swap → close/open → bot) jest zapisywane do **`rebalance_session_id`** na wierszach: `cli_swap`, `position_open` / `position_close`, oraz `orca_bot` (rebalance executor) — suma **`tx_fee_lamports`** lub delt płatnika po tym samym id daje **całościowy** koszt operacji łączonej.

## 2026-03-30 — protocols + execution: rebalance tx lifecycle ledger (`orca_bot`)

**keywords:** clmm-lp-protocols, clmm-lp-execution, rebalance, tx_lifecycle, ledger, jsonl, orca_bot, position_lifecycle, enrich_tx_costs
**paths:** `crates/protocols/src/ledger/tx_lifecycle.rs`, `crates/cli/src/commands/position_lifecycle_ledger.rs`, `crates/execution/src/strategy/rebalance.rs`

Shared append-only JSONL path (`data/ledger/orca_position_lifecycle.jsonl`, same env vars as CLI) and **`enrich_tx_costs`** (RPC `getTransaction` + `meta.fee` + fee payer `preBalances`/`postBalances`) live in **`clmm_lp_protocols::ledger::tx_lifecycle`**. After successful Orca ops in **`RebalanceExecutor`** (`collect_fees`, `decrease_liquidity`, `close_position`, `open_full_range_position`, `open_position`), a row is appended with **`source=orca_bot`**, **`event=bot_*`**, **`operation`** (internal op name), optional **`pool_address`** / **`position_pubkey`** (open flows fill position from `created_position`). CLI lifecycle rows add **`source=cli`** on the same **schema_version=2** file.

## 2026-03-30 — CLI: ledger cyklu życia pozycji Orca (`orca_position_lifecycle.jsonl`)

**keywords:** clmm-lp-cli, orca-position-open, orca-position-close, position_lifecycle_ledger, jsonl, meta.fee, preBalances, postBalances, fee_payer_net_lamports_delta, mint
**paths:** `crates/cli/src/commands/position_lifecycle_ledger.rs`, `crates/cli/src/commands/orca_position.rs`

Po **udanym** `orca-position-open` i `orca-position-close` dopisywany jest wiersz JSONL (**schema_version=2**): domyślnie `data/ledger/orca_position_lifecycle.jsonl`; ścieżka: `CLMM_POSITION_LIFECYCLE_LEDGER_PATH` lub legacy `CLMM_POSITION_OPEN_LEDGER_PATH`. Pola: mint A/B, limity open (raw+UI), `tx_fee_lamports` (`meta.fee`), oraz **`fee_payer_pre/post` + `fee_payer_net_lamports_delta`** (dla płatnika z `preBalances`/`postBalances` w tej samej transakcji). Przy **open** delta jest zwykle ujemna (fee+rent+depozyt SOL do puli); przy **close** często dodatnia (zwrot rent + SOL z płynności) minus skutek fee — suma delt po obu tx daje przybliżony **bilans SOL** z tych operacji (nogi tokenowe USDC itd. osobno).

## 2026-03-30 — tools: `solana_account_state.ps1` (SOL + SPL snapshot via JSON-RPC)

**keywords:** tools, powershell, solana, rpc, getBalance, getTokenAccountsByOwner, account-state, spl-token, token-2022, mainnet
**paths:** `tools/solana_account_state.ps1`

Read-only skrypt Windows: zbiera **lamports + SPL** dla podanego ownera (`spl-token` i **Token-2022**), bez `solana`/`spl-token` CLI. Parametry RPC: `getTokenAccountsByOwner` z filtrem `{ programId }` i **osobnym** trzecim obiektem `{ encoding: jsonParsed }` (wymóg RPC). Kolejka URL: `SOLANA_RPC_URL` → `SOLANA_RPC_FALLBACK_URLS` → domyślne publiczne fallbacki (mniej 429 na pojedynczym hoście). Wyjście: konsola lub `-Json` / `-OutJson` pod kolejne kroki automatyzacji.

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

