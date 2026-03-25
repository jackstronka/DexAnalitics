# Documentation index

This file is the **table of contents** for `doc/`: use it like a book—**thematic sections** below (chapters), and at the **end** an **alphabetical index** of every linked file for quick jump-by-name.

**Canonical architecture and data-flow narrative:** [`PROJECT_OVERVIEW.md`](PROJECT_OVERVIEW.md).

## Architecture and product

| Document | Purpose |
| -------- | ------- |
| [`PROJECT_OVERVIEW.md`](PROJECT_OVERVIEW.md) | Crate layout, fee pipeline (mermaid), CLI command names, data paths, terminology |
| [`ENGINEERING_NOTES.md`](ENGINEERING_NOTES.md) | **Append-only log of non-trivial code changes** — each entry has `keywords:` for grep / AI search |

## Runbooks and operations

| Document | Purpose |
| -------- | ------- |
| [`ORCA_RUNBOOK.md`](ORCA_RUNBOOK.md) | Orca-specific operational steps and notes |

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
| [`TODO_ONCHAIN_NEXT_STEPS.md`](TODO_ONCHAIN_NEXT_STEPS.md) | Phased TODO (A–E) and execution log |
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
| [`BACKTEST_OPTIMIZE_STRATEGIES.md`](BACKTEST_OPTIMIZE_STRATEGIES.md) | strategies, `backtest`, `backtest-optimize`, semantics |
| [`BACKTEST_OPTIMIZE_WHETH_SOL_24_48_72_FEES.md`](BACKTEST_OPTIMIZE_WHETH_SOL_24_48_72_FEES.md) | whETH/SOL, fees, grid example |
| [`BOT_HYBRID_ARCHITECTURE_CONTRACT_2026-03-23.md`](BOT_HYBRID_ARCHITECTURE_CONTRACT_2026-03-23.md) | hybrid bot, scoring, contract (snapshot) |
| [`BOT_HYBRID_DEFINITION_OF_READY_2026-03-23.md`](BOT_HYBRID_DEFINITION_OF_READY_2026-03-23.md) | DoR, Go/No-Go (snapshot) |
| [`BOT_OPERATIONS_MODEL_2026-03-23.md`](BOT_OPERATIONS_MODEL_2026-03-23.md) | ops, alerts, modes (snapshot) |
| [`BOT_RESEARCH_DECISION_2026-03-23.md`](BOT_RESEARCH_DECISION_2026-03-23.md) | research, matrix, direction (snapshot) |
| [`BOT_WORKLOG_2026-03-23.md`](BOT_WORKLOG_2026-03-23.md) | worklog, rationale (snapshot) |
| [`ENGINEERING_NOTES.md`](ENGINEERING_NOTES.md) | code changes, keywords, changelog, AI-searchable |
| [`FEES_DATA_PLAN.md`](FEES_DATA_PLAN.md) | fees data |
| [`METEORA_DLMM_SWAP_EVENT.md`](METEORA_DLMM_SWAP_EVENT.md) | Meteora, swap event, DLMM |
| [`ONCHAIN_FEES_PROGRESS.md`](ONCHAIN_FEES_PROGRESS.md) | on-chain fees, progress |
| [`ONCHAIN_FEES_TRUTH_PLAN.md`](ONCHAIN_FEES_TRUTH_PLAN.md) | on-chain fees, plan |
| [`ORCA_FEES_DATA_PLAN.md`](ORCA_FEES_DATA_PLAN.md) | Orca, fees plan |
| [`ORCA_RUNBOOK.md`](ORCA_RUNBOOK.md) | Orca, operations |
| [`PROJECT_OVERVIEW.md`](PROJECT_OVERVIEW.md) | architecture, crates, pipeline, CLI names, data paths |
| [`README.md`](README.md) | *this file* — TOC + A–Z index |
| [`TODO_ONCHAIN_NEXT_STEPS.md`](TODO_ONCHAIN_NEXT_STEPS.md) | roadmap phases A–E, next steps |
