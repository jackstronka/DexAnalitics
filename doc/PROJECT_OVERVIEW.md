# Bociarz LP Strategy Lab — Project Overview

## Decision and Worklog Docs

For bot direction and historical context of recent work, see:
- `doc/BOT_RESEARCH_DECISION_2026-03-23.md` (research-based decision and comparison matrix)
- `doc/BOT_HYBRID_ARCHITECTURE_CONTRACT_2026-03-23.md` (weighted scoring and hybrid implementation contract)
- `doc/BOT_OPERATIONS_MODEL_2026-03-23.md` (how we operate the bot in practice: modes, alerts, escalation, overrides)
- `doc/BOT_WORKLOG_2026-03-23.md` (what was done and why)

## What this project is

CLMM Liquidity Provider is a Solana strategy optimizer and execution engine for liquidity providers (LPs) operating on Concentrated Liquidity Market Makers (CLMMs).

Async communication layer v2 (event-bus contract, decision matrix NATS/Redis/Kafka, rollout modes) is documented in `doc/ASYNC_COMMUNICATION_LAYER.md`.

It supports multiple venues:
- Orca Whirlpool
- Raydium CLMM
- Meteora DLMM (bin-based)

## Why it exists

DeFi market making needs more than maximizing APY. This project aims to:
- Quantify risk (impermanent loss, drawdown, and path-consistent performance)
- Find good tick ranges for fee capture
- Simulate historical performance with the same strategies you plan to run live

## High-level architecture (crates)

The workspace is separated by responsibility:
- `clmm-lp-domain`: core math (tick/price conversions, IL, liquidity), entities, value objects
- `clmm-lp-simulation`: backtesting engine (position tracking, rebalances, time-in-range)
- `clmm-lp-optimization`: grid / range optimizer and objective functions
- `clmm-lp-protocols`: protocol adapters (pool readers + executors where applicable)
- `clmm-lp-data`: external data providers and local repositories (Birdeye/Jupiter/Dune/swap events)
- `clmm-lp-execution`: live monitoring, strategy execution, scheduler scaffolding
- `clmm-lp-api` and `clmm-lp-cli`: user interfaces (REST/Web + CLI commands)

## Periodic `backtest-optimize`, artifacts, and AI agent layer

Two complementary ways to refresh grid results and push them into a running bot:

1. **In-process (`StrategyService` in `clmm-lp-api`)** — If your integration calls `StrategyService::start_strategy` (not the minimal HTTP handler path), strategy `parameters` may include `optimize_interval_secs`, `optimize_command`, and `optimize_result_json_path`. That spawns `clmm-lp-cli backtest-optimize` on a timer and applies `--optimize-result-json` via `apply_optimize_result_json` (same code path as periodic optimize in `crates/api/src/services/strategy_service.rs`).

2. **External scheduler + HTTP** — Run `backtest-optimize` on a cron/Task Scheduler with `--optimize-result-json` (and optionally `--optimize-result-json-copy-dir` for timestamped `latest.json` + history). Then call **`POST /api/v1/strategies/{id}/apply-optimize-result`** with either a raw [`OptimizeResultFile`](crates/domain/src/optimize_result.rs) JSON or an agent envelope `{ "decision": AgentDecision, "baseline_optimize_result": ... }`. Optional risk cap: `parameters.agent_max_width_pct_delta` enforces `|Δ winner.width_pct|` vs baseline when using the envelope. Types: [`AgentDecision`](crates/domain/src/agent_decision.rs), validation in `clmm-lp-execution::agent_decision`.

The HTTP `start_strategy` handler in `crates/api/src/handlers/strategies.rs` is a lighter path; use `StrategyService` or the apply endpoint above when you need the full optimize/apply pipeline.

### Who may apply grid results (`optimize_apply_policy`)

Pick one policy per strategy so operators and agents do not fight over the executor:

| Policy (`parameters.optimize_apply_policy`) | Periodic subprocess (`optimize_interval_secs`) | `POST .../apply-optimize-result` |
| --- | --- | --- |
| `periodic_subprocess` | Yes (only this path applies) | **409** — use for subprocess-only deployments |
| `external_http` | Must be **0** when using [`StrategyService`](crates/api/src/services/strategy_service.rs) (validated at start) | Yes (cron/agent applies) |
| `combined` (default) | Optional | Yes — shares per-strategy `optimization_busy` with the subprocess so only one apply runs at a time |

**Recommendation-only agent output:** send `AgentDecision` with `approved: false`; the API returns 200 and leaves the executor unchanged (no busy lock).

**Locks:** `AppState.optimization_busy` serializes HTTP apply with the periodic subprocess cycle for the same strategy ID when using `combined`.

## Data flow for fees (CRON -> snapshot-fees -> optimizer)

The “snapshot-fees” pipeline is intended to reduce reliance on paid analytics for fee modeling by building local, on-chain-fee proxies.

```mermaid
graph TD
  Cron[CRON / local scheduler] --> Collector[Snapshot collector CLI]
  Cron --> SwapSync[Swaps sync CLI]
  Collector --> OrcaSnaps[orca snapshots.jsonl]
  Collector --> RaydiumSnaps[raydium snapshots.jsonl]
  Collector --> MeteoraSnaps[meteora snapshots.jsonl]
  SwapSync --> OrcaSwaps[orca swaps.jsonl]
  SwapSync --> RaydiumSwaps[raydium swaps.jsonl]
  SwapSync --> MeteoraSwaps[meteora swaps.jsonl]

  OrcaSnaps --> Optimizer[backtest / backtest-optimize]
  RaydiumSnaps --> Optimizer
  MeteoraSnaps --> Optimizer
  OrcaSwaps --> Optimizer
  RaydiumSwaps --> Optimizer
  MeteoraSwaps --> Optimizer
  Optimizer --> BestRange[best range per strategy (fees - IL - costs)]
```

## CLI commands relevant to this pipeline

Key commands live in `crates/cli/src/main.rs`:
- Snapshot collection:
  - `OrcaSnapshot`
  - `OrcaSnapshotCurated`
  - `RaydiumSnapshotCurated`
  - `MeteoraSnapshotCurated`
  - `SnapshotRunCuratedAll`
  - `SnapshotReadiness` (audits if snapshot data covers fee tiers)
- Swap stream collection (P1):
  - `SwapsSyncCuratedAll` (raw signatures; optional paged pull via `--max-pages`)
  - `SwapsSubscribeMentions` (WebSocket `logsSubscribe` by `mentions`; supports `--mentions-preset orca|raydium|meteora`; near-real-time raw signatures)
  - `SwapsEnrichCuratedAll` → `decoded_swaps.jsonl` (vault deltas + direction)
  - `SwapsDecodeAudit` (quality report)
  - `DataHealthCheck` (staleness + decode OK%)
  - `OpsIngestCycle` (automation wrapper: snapshots → sync → enrich → audit → health-check; saves JSON report in `data/reports/`)
- Backtesting/optimization:
  - `Backtest`
  - `BacktestOptimize` (grid search over ranges + default strategy set; opcjonalnie lokalne `data/swaps` gdy brak Dune)

## Strategy coverage by layer

- Implemented **backtest / backtest-optimize** strategy catalog: `static`, `oor_recenter`, `periodic`, `threshold`, `il_limit`, `retouch_shift`. Szczegóły semantyki: **`doc/BACKTEST_OPTIMIZE_STRATEGIES.md`**.
- `backtest-optimize`: evaluates the default strategy set on historical grid runs (`commands/backtest_optimize.rs::default_strategies`).
- `optimize`: analytical layer (Monte Carlo / synthetic paths); liczba nazw strategii w warstwie analitycznej może różnić się od siatki CLI — patrz kod `clmm-lp-optimization`.
- `ParameterOptimizer`: parameter-search candidates (warstwa optymalizacji analitycznej).

The curated pool addresses are defined in `STARTUP.md`.

## Where data is stored

Local snapshot files (append-only JSONL):
- `data/pool-snapshots/orca/<pool_address>/snapshots.jsonl`
- `data/pool-snapshots/raydium/<pool_address>/snapshots.jsonl`
- `data/pool-snapshots/meteora/<pool_address>/snapshots.jsonl`

Swap-level cache:
- `data/swaps/orca/<pool_address>/swaps.jsonl` (raw chain stream, P1)
- `data/swaps/orca/<pool_address>/decoded_swaps.jsonl` (P1.1 decode)
- `data/swaps/raydium/<pool_address>/swaps.jsonl` + `decoded_swaps.jsonl`
- `data/swaps/meteora/<pool_address>/swaps.jsonl` + `decoded_swaps.jsonl`
- `data/dune-cache/*`
- `data/dune-swaps/*` (if/when ETL output is used)

Conceptual background on **how Solana indexing works** (why a fungible token is not a “network-wide sniffer”, and how HTTP RPC relates to streams such as Geyser) lives in [`SOLANA_INDEXING.md`](SOLANA_INDEXING.md).

## Terminology

- Tier 2 (snapshot-fees): use snapshot-derived fee proxies to model pool fees over time.
- Tier 3 (inside-range truth): account fees per position using inside-growth accounting and position/range history.

