# Documentation index

This file is the **table of contents** for `doc/`: use it like a book—**thematic sections** below (chapters), and at the **end** an **alphabetical index** of every linked file for quick jump-by-name.

**Canonical architecture and data-flow narrative:** [`PROJECT_OVERVIEW.md`](PROJECT_OVERVIEW.md).

**Co robić dalej (kolejka prac):** [`TODO_ONCHAIN_NEXT_STEPS.md`](TODO_ONCHAIN_NEXT_STEPS.md) — sekcja *Od czego zacząć* + fazy A–F i **M** (M1 Meteora TVL, M2 kolejka RPC w enrich).

**Plan produktowy (osobno od fees):** [`TODO_CHART_AGENT_LAYER.md`](TODO_CHART_AGENT_LAYER.md) — **osobny profil/tryb** (`agent_layer_profile`), screenshot + agenci, konsensus, rulebook; backlog P1–P13.

## Architecture and product

| Document | Purpose |
| -------- | ------- |
| [`PROJECT_OVERVIEW.md`](PROJECT_OVERVIEW.md) | Crate layout, fee pipeline (mermaid), CLI command names, data paths, terminology |
| [`ASYNC_COMMUNICATION_LAYER.md`](ASYNC_COMMUNICATION_LAYER.md) | Async event bus v2: decision matrix, event contract, rollout |
| [`SOLANA_INDEXING.md`](SOLANA_INDEXING.md) | Solana indexing concepts (RPC vs WebSocket vs Geyser), “token” misconception, relation to swap sync |
| [`ENGINEERING_NOTES.md`](ENGINEERING_NOTES.md) | **Append-only log of non-trivial code changes** — each entry has `keywords:` for grep / AI search |
| [`AI_STREAM_AGENT.md`](AI_STREAM_AGENT.md) | Local-first MVP for an AI narrator / stream agent (YouTube) |

## Runbooks and operations

| Document | Purpose |
| -------- | ------- |
| [`ORCA_RUNBOOK.md`](ORCA_RUNBOOK.md) | Orca-specific operational steps and notes |
| [`ORCA_API_SERVICE_CONTRACT.md`](ORCA_API_SERVICE_CONTRACT.md) | Contract: `OrcaReadService` (REST) + `OrcaTxService` (on-chain), endpoint/method map, implementation checklist |
| [`ORCA_EXTERNAL_IMPLEMENTATIONS.md`](ORCA_EXTERNAL_IMPLEMENTATIONS.md) | Patterns from Hummingbot/Orca for production-like Orca integrations |
| [`DEVNET_BOT_PRODUCTION_READINESS.md`](DEVNET_BOT_PRODUCTION_READINESS.md) | 3-phase checklist to move bot from devnet MVP to production-like readiness |

## Backtesting and strategies

| Document | Purpose |
| -------- | ------- |
| [`BACKTEST_OPTIMIZE_STRATEGIES.md`](BACKTEST_OPTIMIZE_STRATEGIES.md) | Strategy catalog semantics for `backtest` / `backtest-optimize` |
| [`BACKTEST_OPTIMIZE_WHETH_SOL_24_48_72_FEES.md`](BACKTEST_OPTIMIZE_WHETH_SOL_24_48_72_FEES.md) | Focused backtest-optimize notes (example pair / fees) |

## Fees, swaps, and on-chain data plans

| Document | Purpose |
| -------- | ------- |
| [`FEES_DATA_PLAN.md`](FEES_DATA_PLAN.md) | Fees data approach |
| [`ORCA_FEES_DATA_PLAN.md`](ORCA_FEES_DATA_PLAN.md) | Orca fees data plan |
| [`ONCHAIN_FEES_TRUTH_PLAN.md`](ONCHAIN_FEES_TRUTH_PLAN.md) | Path toward on-chain-aligned fee accounting |
| [`ONCHAIN_FEES_PROGRESS.md`](ONCHAIN_FEES_PROGRESS.md) | Progress log for on-chain fees work |
| [`TODO_ONCHAIN_NEXT_STEPS.md`](TODO_ONCHAIN_NEXT_STEPS.md) | Roadmap: priorytet startowy, fazy A–F + **M** (M1/M2), log wykonania |
| [`METEORA_DLMM_SWAP_EVENT.md`](METEORA_DLMM_SWAP_EVENT.md) | Meteora DLMM swap event notes |

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
| [`../STARTUP.md`](../STARTUP.md) | End-to-end startup procedures; curated pool addresses |
| [`../AGENTS.md`](../AGENTS.md) | Short map for AI assistants (crates, entrypoints, links) |

When adding a new standalone doc under `doc/`, **add one row to the appropriate thematic table above** and **one line to the alphabetical index below**.

## Alphabetical index (A–Z by filename)

| File | Keywords |
| ---- | -------- |
| [`AI_STREAM_AGENT.md`](AI_STREAM_AGENT.md) | stream, narrator, obs, studio, agent |
| [`ASYNC_COMMUNICATION_LAYER.md`](ASYNC_COMMUNICATION_LAYER.md) | async, event bus, kafka, nats, redis, rollout |
| [`BACKTEST_OPTIMIZE_STRATEGIES.md`](BACKTEST_OPTIMIZE_STRATEGIES.md) | strategies, `backtest`, `backtest-optimize`, semantics |
| [`BACKTEST_OPTIMIZE_WHETH_SOL_24_48_72_FEES.md`](BACKTEST_OPTIMIZE_WHETH_SOL_24_48_72_FEES.md) | whETH/SOL, fees, grid example |
| [`BOT_HYBRID_ARCHITECTURE_CONTRACT_2026-03-23.md`](BOT_HYBRID_ARCHITECTURE_CONTRACT_2026-03-23.md) | hybrid bot, scoring, contract (snapshot) |
| [`BOT_HYBRID_DEFINITION_OF_READY_2026-03-23.md`](BOT_HYBRID_DEFINITION_OF_READY_2026-03-23.md) | DoR, Go/No-Go (snapshot) |
| [`BOT_OPERATIONS_MODEL_2026-03-23.md`](BOT_OPERATIONS_MODEL_2026-03-23.md) | ops, alerts, modes (snapshot) |
| [`BOT_RESEARCH_DECISION_2026-03-23.md`](BOT_RESEARCH_DECISION_2026-03-23.md) | research, matrix, direction (snapshot) |
| [`BOT_WORKLOG_2026-03-23.md`](BOT_WORKLOG_2026-03-23.md) | worklog, rationale (snapshot) |
| [`DEVNET_BOT_PRODUCTION_READINESS.md`](DEVNET_BOT_PRODUCTION_READINESS.md) | devnet, bot, production readiness, checklist, go/no-go |
| [`ENGINEERING_NOTES.md`](ENGINEERING_NOTES.md) | code changes, keywords, changelog, AI-searchable |
| [`FEES_DATA_PLAN.md`](FEES_DATA_PLAN.md) | fees data |
| [`METEORA_DLMM_SWAP_EVENT.md`](METEORA_DLMM_SWAP_EVENT.md) | Meteora, swap event, DLMM |
| [`ONCHAIN_FEES_PROGRESS.md`](ONCHAIN_FEES_PROGRESS.md) | on-chain fees, progress |
| [`ONCHAIN_FEES_TRUTH_PLAN.md`](ONCHAIN_FEES_TRUTH_PLAN.md) | on-chain fees, plan |
| [`ORCA_FEES_DATA_PLAN.md`](ORCA_FEES_DATA_PLAN.md) | Orca, fees plan |
| [`ORCA_API_SERVICE_CONTRACT.md`](ORCA_API_SERVICE_CONTRACT.md) | Orca, service contract, read/write split, endpoint map |
| [`ORCA_RUNBOOK.md`](ORCA_RUNBOOK.md) | Orca, operations |
| [`ORCA_EXTERNAL_IMPLEMENTATIONS.md`](ORCA_EXTERNAL_IMPLEMENTATIONS.md) | orca, hummingbot, examples, rent, token-2022, tx-builders |
| [`PROJECT_OVERVIEW.md`](PROJECT_OVERVIEW.md) | architecture, crates, pipeline, CLI names, data paths |
| [`README.md`](README.md) | *this file* — TOC + A–Z index |
| [`SOLANA_INDEXING.md`](SOLANA_INDEXING.md) | solana, indexing, RPC, Geyser, swaps-sync, misconceptions |
| [`TODO_CHART_AGENT_LAYER.md`](TODO_CHART_AGENT_LAYER.md) | agent_layer_profile, osobny tryb, chart screenshot, rules-as-training, consensus, eval harness, `AgentDecision`, P1–P13 |
| [`TODO_ONCHAIN_NEXT_STEPS.md`](TODO_ONCHAIN_NEXT_STEPS.md) | roadmap, phases A–F, M1/M2 sprint, start-here queue |
