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

