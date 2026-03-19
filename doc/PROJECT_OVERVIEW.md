# Project Overview

## What this project is

CLMM Liquidity Provider is a Solana strategy optimizer and execution engine for liquidity providers (LPs) operating on Concentrated Liquidity Market Makers (CLMMs).

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

## Data flow for fees (CRON -> snapshot-fees -> optimizer)

The “snapshot-fees” pipeline is intended to reduce reliance on paid analytics for fee modeling by building local, on-chain-fee proxies.

```mermaid
graph TD
  Cron[CRON / local scheduler] --> Collector[Snapshot collector CLI]
  Collector --> OrcaSnaps[orca snapshots.jsonl]
  Collector --> RaydiumSnaps[raydium snapshots.jsonl]
  Collector --> MeteoraSnaps[meteora snapshots.jsonl]

  OrcaSnaps --> Optimizer[backtest / backtest-optimize]
  RaydiumSnaps --> Optimizer
  MeteoraSnaps --> Optimizer
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
- Backtesting/optimization:
  - `Backtest`
  - `BacktestOptimize` (grid search over ranges + 3 strategy types)

The curated pool addresses are defined in `STARTUP.md`.

## Where data is stored

Local snapshot files (append-only JSONL):
- `data/pool-snapshots/orca/<pool_address>/snapshots.jsonl`
- `data/pool-snapshots/raydium/<pool_address>/snapshots.jsonl`
- `data/pool-snapshots/meteora/<pool_address>/snapshots.jsonl`

Swap-level cache (when used):
- `data/dune-cache/*`
- `data/dune-swaps/*` (if/when ETL output is used)

## Terminology

- Tier 2 (snapshot-fees): use snapshot-derived fee proxies to model pool fees over time.
- Tier 3 (inside-range truth): account fees per position using inside-growth accounting and position/range history.

