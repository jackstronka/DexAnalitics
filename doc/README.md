# Documentation index

This file is the **table of contents** for `doc/`: use it like a book—**thematic sections** below (chapters), and at the **end** an **alphabetical index** of every linked file for quick jump-by-name.

**Canonical architecture and data-flow narrative:** [`PROJECT_OVERVIEW.md`](PROJECT_OVERVIEW.md).

**Normative behavior (how features should work — one place to refine):** [`FUNCTIONAL_SPECIFICATION.md`](FUNCTIONAL_SPECIFICATION.md).

**Co robić dalej (kolejka prac):** [`TODO_ONCHAIN_NEXT_STEPS.md`](TODO_ONCHAIN_NEXT_STEPS.md) — sekcja *Od czego zacząć* + fazy A–F i **M** (M1 Meteora TVL, M2 kolejka RPC w enrich).

**Warstwa decyzyjna / orkiestrator LP (wizja + fazy, shadow, symulacje):** [`DECISION_LAYER.md`](DECISION_LAYER.md).

**Plan produktowy (osobno od fees):** [`TODO_CHART_AGENT_LAYER.md`](TODO_CHART_AGENT_LAYER.md) — **osobny profil/tryb** (`agent_layer_profile`), screenshot + agenci, konsensus, rulebook; backlog P1–P13.

**Roadmap produktowa (strategie ↔ pozycja, shadow / historia):** [`ROADMAP.md`](ROADMAP.md) — wiele strategii na jednej pozycji (1 live + N shadow), odświeżanie co kilka minut, **zachowanie historii przypisań** przy zmianach.

**Strategie Bollinger + ostatnia świeca (backtest, API, bot):** [`IMPLEMENTATION_PLAN_BOLLINGER_CANDLE_STRATEGIES.md`](IMPLEMENTATION_PLAN_BOLLINGER_CANDLE_STRATEGIES.md) — plan faz; [`ROADMAP.md`](ROADMAP.md) — opis produktowy.

**Roadmap strategii (Jupiter / multi-venue / CLMM):** [`ROADMAP_JUPITER_MULTI_VENUE_LP.md`](ROADMAP_JUPITER_MULTI_VENUE_LP.md) — hipoteza routingu i fee; przenoszenie zakresów pod wolumen skierowany przez agregator; co zweryfikować danymi.

## Architecture and product

| Document | Purpose |
| -------- | ------- |
| [`PROJECT_OVERVIEW.md`](PROJECT_OVERVIEW.md) | Crate layout, fee pipeline (mermaid), CLI command names, data paths, terminology |
| [`AI_AGENT_LAYER.md`](AI_AGENT_LAYER.md) | **Canonical:** „agent / AI” w repo — `AgentDecision` + apply-optimize, Position Agent (LLM opcjonalny), `DecisionEngine` live, `agent_decisions.jsonl`, linki do roadmap |
| [`DECISION_LAYER.md`](DECISION_LAYER.md) | **Wizja + kontrakt + audyt §11:** orkiestrator LP, fazy, shadow/symulacje; **tabela co w kodzie / czego brak** (dowody ścieżkami) |
| [`FUNCTIONAL_SPECIFICATION.md`](FUNCTIONAL_SPECIFICATION.md) | **Normative:** expected behavior per feature (open/close, rebalance, strategies, wallet, fees); refine here first |
| [`PROJECT_END_TO_END.md`](PROJECT_END_TO_END.md) | End-to-end: ingest danych -> analytics -> decyzje bota -> wykonanie i UI |
| [`ROADMAP.md`](ROADMAP.md) | Roadmap produktowy: shadow strategies per position, historia przypisań strategia ↔ pozycja |
| [`ASYNC_COMMUNICATION_LAYER.md`](ASYNC_COMMUNICATION_LAYER.md) | Async event bus v2: decision matrix, event contract, rollout |
| [`SOLANA_INDEXING.md`](SOLANA_INDEXING.md) | Solana indexing concepts (RPC vs WebSocket vs Geyser), “token” misconception, relation to swap sync |
| [`AERODROME_SLIPSTREAM_BASE_LIVE_PLAN.md`](AERODROME_SLIPSTREAM_BASE_LIVE_PLAN.md) | **Base + Aerodrome Slipstream (CLMM) live:** oficjalne źródła, fazy 0–5, wielość deployów, bezpieczeństwo, Go-live checklist; fee-only unstaked; endpoint `GET /api/v1/evm/base/aerodrome-slipstream/pools/{pool}/slot0` + `BASE_RPC_URL` |
| [`ENGINEERING_NOTES.md`](ENGINEERING_NOTES.md) | **Append-only log of non-trivial code changes** — each entry has `keywords:` for grep / AI search |
| [`AI_STREAM_AGENT.md`](AI_STREAM_AGENT.md) | Local-first MVP for an AI narrator / stream agent (YouTube) |

## Runbooks and operations

| Document | Purpose |
| -------- | ------- |
| [`PRODUCTION_FAST_PATH.md`](PRODUCTION_FAST_PATH.md) | **Shortest path to live Orca bot CLI** — env, order dry-run → execute, links |
| [`ORCA_RUNBOOK.md`](ORCA_RUNBOOK.md) | Orca-specific operational steps and notes |
| [`DEVNET_WALLET_BOT_LAUNCH_RUNBOOK_V1.md`](DEVNET_WALLET_BOT_LAUNCH_RUNBOOK_V1.md) | Step-by-step wallet setup + bot launch flow on devnet (`dry-run` -> `limited-live`) |
| [`ORCA_API_SERVICE_CONTRACT.md`](ORCA_API_SERVICE_CONTRACT.md) | Contract: `OrcaReadService` (REST) + `OrcaTxService` (on-chain), endpoint/method map, implementation checklist |
| [`ORCA_EXTERNAL_IMPLEMENTATIONS.md`](ORCA_EXTERNAL_IMPLEMENTATIONS.md) | Patterns from Hummingbot/Orca for production-like Orca integrations |
| [`DEVNET_BOT_PRODUCTION_READINESS.md`](DEVNET_BOT_PRODUCTION_READINESS.md) | 3-phase checklist to move bot from devnet MVP to production-like readiness |
| [`RPC_SOLANA_BOT_NOTES.md`](RPC_SOLANA_BOT_NOTES.md) | Solana RPC vs Orca, public vs provider, free-tier pointers, dual fallback — notes for mainnet bot |
| [`MAINNET_OPERATIONAL_CHECKLIST.md`](MAINNET_OPERATIONAL_CHECKLIST.md) | `CLMM_EXPECTED_CLUSTER`, dry-run vs limited live, links to BOT ops + RPC notes |
| [`POSITION_REGISTRY.md`](POSITION_REGISTRY.md) | `registry.jsonl` open/close, **szybki podgląd aktywnych pozycji** (replay, API, `orca-positions-list`) |
| [`OPERATIONAL_CONTINUITY.md`](OPERATIONAL_CONTINUITY.md) | systemd / Task Scheduler / Docker restart, logi, alerty (haki), RPC i klucze |
| [`SCRIPTS_CATALOG.md`](SCRIPTS_CATALOG.md) | spis `tools/*.ps1`, snapshot P0, Slack, CLI powiązane, skrypty spoza git (`scripts/`) |
| [`UI_REQUIREMENTS_PHASE1.md`](UI_REQUIREMENTS_PHASE1.md) | Zakres dashboardu fazy 1 (skrypty, portfel, pozycje, ledger, akcje); **status implementacji** + wymagania środowiska |
| [`DOCKER.md`](DOCKER.md) | `docker compose` (web + API), `API_UPSTREAM`; **Docker Desktop musi działać** (Windows — troubleshooting pipe) |

## Backtesting and strategies

| Document | Purpose |
| -------- | ------- |
| [`BACKTEST_OPTIMIZE_STRATEGIES.md`](BACKTEST_OPTIMIZE_STRATEGIES.md) | Strategy catalog semantics for `backtest` / `backtest-optimize` |
| [`IMPLEMENTATION_PLAN_BOLLINGER_CANDLE_STRATEGIES.md`](IMPLEMENTATION_PLAN_BOLLINGER_CANDLE_STRATEGIES.md) | Plan: strategie Bollinger i ostatnia świeca (symulacja, API, web, execution) |
| [`BACKTEST_OPTIMIZE_WHETH_SOL_24_48_72_FEES.md`](BACKTEST_OPTIMIZE_WHETH_SOL_24_48_72_FEES.md) | Focused backtest-optimize notes (example pair / fees) |
| [`ROADMAP_JUPITER_MULTI_VENUE_LP.md`](ROADMAP_JUPITER_MULTI_VENUE_LP.md) | Jupiter aggregation, routing vs fee tier, hypothesis for CLMM range placement across venues |

## Fees, swaps, and on-chain data plans

| Document | Purpose |
| -------- | ------- |
| [`FEES_DATA_PLAN.md`](FEES_DATA_PLAN.md) | Fees data approach |
| [`ORCA_FEES_DATA_PLAN.md`](ORCA_FEES_DATA_PLAN.md) | Orca fees data plan |
| [`ONCHAIN_FEES_TRUTH_PLAN.md`](ONCHAIN_FEES_TRUTH_PLAN.md) | Path toward on-chain-aligned fee accounting |
| [`ONCHAIN_FEES_PROGRESS.md`](ONCHAIN_FEES_PROGRESS.md) | Progress log for on-chain fees work |
| [`TODO_ONCHAIN_NEXT_STEPS.md`](TODO_ONCHAIN_NEXT_STEPS.md) | Roadmap: priorytet startowy, fazy A–F + **M** (M1/M2), log wykonania |
| [`METEORA_DLMM_SWAP_EVENT.md`](METEORA_DLMM_SWAP_EVENT.md) | Meteora DLMM swap event notes |

## Position economics (IL, HODL, USD)

| Document | Purpose |
| -------- | ------- |
| [`IMPERMANENT_LOSS_USD_AND_FEES.md`](IMPERMANENT_LOSS_USD_AND_FEES.md) | IL vs HODL w USD, wariant z/bez fees LP, łańcuch PDAs (lineage), mapa kodu (`stream-pnl`, domain, symulacja), ograniczenia `PnLTracker` |

## Bot direction and worklog (dated snapshot — 2026-03-23)

These files capture **research decisions and context at a point in time**. They are **not** day-to-day runbooks; prefer `PROJECT_OVERVIEW.md` and runbooks for current procedures.

| Document | Purpose |
| -------- | ------- |
| [`BOT_RESEARCH_DECISION_2026-03-23.md`](BOT_RESEARCH_DECISION_2026-03-23.md) | Options comparison and recommended direction |
| [`BOT_HYBRID_ARCHITECTURE_CONTRACT_2026-03-23.md`](BOT_HYBRID_ARCHITECTURE_CONTRACT_2026-03-23.md) | Weighted scoring and hybrid implementation contract |
| [`BOT_HYBRID_DEFINITION_OF_READY_2026-03-23.md`](BOT_HYBRID_DEFINITION_OF_READY_2026-03-23.md) | Definition of ready / Go–No-Go checklist |
| [`BOT_OPERATIONS_MODEL_2026-03-23.md`](BOT_OPERATIONS_MODEL_2026-03-23.md) | Modes, alerts, escalation, overrides |
| [`BOT_WORKLOG_2026-03-23.md`](BOT_WORKLOG_2026-03-23.md) | What was done and why |
| [`TODO_CHART_AGENT_LAYER.md`](TODO_CHART_AGENT_LAYER.md) | Plan: **osobny profil** (`agent_layer_profile`), publiczne UI DEX, konsensus, rulebook, ewaluacja; P1–P13 |

## Repository root (outside `doc/`)

| Path | Purpose |
| ---- | ------- |
| [`../README.md`](../README.md) | Polish quick-start, CLI recipes, workspace list |
| [`../STARTUP.md`](../STARTUP.md) | End-to-end startup; curated pools; `snapshot-backtest-prep` + `tools/run_snapshot_backtest_prep_loop.ps1` for fast Orca snapshot backtests |
| [`../AGENTS.md`](../AGENTS.md) | Short map for AI assistants (crates, entrypoints, links) |

When adding a new standalone doc under `doc/`, **add one row to the appropriate thematic table above** and **one line to the alphabetical index below**.

## Alphabetical index (A–Z by filename)

| File | Keywords |
| ---- | -------- |
| [`AERODROME_SLIPSTREAM_BASE_LIVE_PLAN.md`](AERODROME_SLIPSTREAM_BASE_LIVE_PLAN.md) | aerodrome, slipstream, base, live, alloy, rpc, gauges-v3, WETH, USDC, cbBTC, CL100, deployment phases |
| [`AI_STREAM_AGENT.md`](AI_STREAM_AGENT.md) | stream, narrator, obs, studio, agent |
| [`AI_AGENT_LAYER.md`](AI_AGENT_LAYER.md) | AgentDecision, apply-optimize, position agent, DecisionEngine, agent_decisions.jsonl, orchestration, LLM optional |
| [`ASYNC_COMMUNICATION_LAYER.md`](ASYNC_COMMUNICATION_LAYER.md) | async, event bus, kafka, nats, redis, rollout |
| [`BACKTEST_OPTIMIZE_STRATEGIES.md`](BACKTEST_OPTIMIZE_STRATEGIES.md) | strategies, `backtest`, `backtest-optimize`, semantics |
| [`IMPLEMENTATION_PLAN_BOLLINGER_CANDLE_STRATEGIES.md`](IMPLEMENTATION_PLAN_BOLLINGER_CANDLE_STRATEGIES.md) | bollinger, candle, StratConfig, StrategyMode, backtest, roadmap |
| [`IMPERMANENT_LOSS_USD_AND_FEES.md`](IMPERMANENT_LOSS_USD_AND_FEES.md) | IL, HODL, USD, fees LP, stream-pnl, lineage, calculate_il_concentrated, segment IL |
| [`BACKTEST_OPTIMIZE_WHETH_SOL_24_48_72_FEES.md`](BACKTEST_OPTIMIZE_WHETH_SOL_24_48_72_FEES.md) | whETH/SOL, fees, grid example |
| [`BOT_HYBRID_ARCHITECTURE_CONTRACT_2026-03-23.md`](BOT_HYBRID_ARCHITECTURE_CONTRACT_2026-03-23.md) | hybrid bot, scoring, contract (snapshot) |
| [`BOT_HYBRID_DEFINITION_OF_READY_2026-03-23.md`](BOT_HYBRID_DEFINITION_OF_READY_2026-03-23.md) | DoR, Go/No-Go (snapshot) |
| [`BOT_OPERATIONS_MODEL_2026-03-23.md`](BOT_OPERATIONS_MODEL_2026-03-23.md) | ops, alerts, modes (snapshot) |
| [`BOT_RESEARCH_DECISION_2026-03-23.md`](BOT_RESEARCH_DECISION_2026-03-23.md) | research, matrix, direction (snapshot) |
| [`BOT_WORKLOG_2026-03-23.md`](BOT_WORKLOG_2026-03-23.md) | worklog, rationale (snapshot) |
| [`DECISION_LAYER.md`](DECISION_LAYER.md) | decision-layer, orchestrator, shadow, counterfactual, simulation, backtest, data-quality, phases, capital allocation, implementation-audit |
| [`DEVNET_WALLET_BOT_LAUNCH_RUNBOOK_V1.md`](DEVNET_WALLET_BOT_LAUNCH_RUNBOOK_V1.md) | devnet, wallet, runbook, dry-run, limited-live, preflight |
| [`DEVNET_BOT_PRODUCTION_READINESS.md`](DEVNET_BOT_PRODUCTION_READINESS.md) | devnet, bot, production readiness, checklist, go/no-go |
| [`DOCKER.md`](DOCKER.md) | docker compose, web+api, API_UPSTREAM, Docker Desktop, Windows pipe |
| [`ENGINEERING_NOTES.md`](ENGINEERING_NOTES.md) | code changes, keywords, changelog, AI-searchable |
| [`FEES_DATA_PLAN.md`](FEES_DATA_PLAN.md) | fees data |
| [`FUNCTIONAL_SPECIFICATION.md`](FUNCTIONAL_SPECIFICATION.md) | normative feature behavior, operator spec, single source of truth for “should” |
| [`METEORA_DLMM_SWAP_EVENT.md`](METEORA_DLMM_SWAP_EVENT.md) | Meteora, swap event, DLMM |
| [`ONCHAIN_FEES_PROGRESS.md`](ONCHAIN_FEES_PROGRESS.md) | on-chain fees, progress |
| [`ONCHAIN_FEES_TRUTH_PLAN.md`](ONCHAIN_FEES_TRUTH_PLAN.md) | on-chain fees, plan |
| [`POSITION_REGISTRY.md`](POSITION_REGISTRY.md) | registry.jsonl, active positions, orca-positions-list, collectors, API registry |
| [`OPERATIONAL_CONTINUITY.md`](OPERATIONAL_CONTINUITY.md) | bot supervision, systemd, Docker, Windows script, logs, alerts hooks |
| [`ORCA_FEES_DATA_PLAN.md`](ORCA_FEES_DATA_PLAN.md) | Orca, fees plan |
| [`ORCA_API_SERVICE_CONTRACT.md`](ORCA_API_SERVICE_CONTRACT.md) | Orca, service contract, read/write split, endpoint map |
| [`ORCA_RUNBOOK.md`](ORCA_RUNBOOK.md) | Orca, operations |
| [`ORCA_EXTERNAL_IMPLEMENTATIONS.md`](ORCA_EXTERNAL_IMPLEMENTATIONS.md) | orca, hummingbot, examples, rent, token-2022, tx-builders |
| [`PROJECT_OVERVIEW.md`](PROJECT_OVERVIEW.md) | architecture, crates, pipeline, CLI names, data paths |
| [`PROJECT_END_TO_END.md`](PROJECT_END_TO_END.md) | end-to-end pipeline: data -> analytics -> bot -> UI |
| [`ROADMAP.md`](ROADMAP.md) | roadmap, position, strategy, shadow, counterfactual, live, history, assignment |
| [`ROADMAP_JUPITER_MULTI_VENUE_LP.md`](ROADMAP_JUPITER_MULTI_VENUE_LP.md) | Jupiter, multi-venue, routing, fee hypothesis, CLMM roadmap |
| [`RPC_SOLANA_BOT_NOTES.md`](RPC_SOLANA_BOT_NOTES.md) | rpc, solana, mainnet, fallback, free tier, orca bot |
| [`SCRIPTS_CATALOG.md`](SCRIPTS_CATALOG.md) | tools, powershell, snapshot-health, slack, monitoring, keywords |
| [`README.md`](README.md) | *this file* — TOC + A–Z index |
| [`SOLANA_INDEXING.md`](SOLANA_INDEXING.md) | solana, indexing, RPC, Geyser, swaps-sync, misconceptions |
| [`TODO_CHART_AGENT_LAYER.md`](TODO_CHART_AGENT_LAYER.md) | agent_layer_profile, osobny tryb, chart screenshot, rules-as-training, consensus, eval harness, `AgentDecision`, P1–P13 |
| [`TODO_ONCHAIN_NEXT_STEPS.md`](TODO_ONCHAIN_NEXT_STEPS.md) | roadmap, phases A–F, M1/M2 sprint, start-here queue |
| [`UI_REQUIREMENTS_PHASE1.md`](UI_REQUIREMENTS_PHASE1.md) | dashboard phase 1, scripts, wallet, positions, ledger, implementation status |
