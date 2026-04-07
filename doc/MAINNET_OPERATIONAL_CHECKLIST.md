# Mainnet — operational checklist (Orca-first)

**Purpose:** shorten the path from devnet to **observation / dry-run / limited live** on mainnet without mixing clusters or surprise live trades.

**Related:** [`doc/PRODUCTION_FAST_PATH.md`](PRODUCTION_FAST_PATH.md) (shortest path: build → env → dry-run → `--execute`), [`doc/BOT_OPERATIONS_MODEL_2026-03-23.md`](BOT_OPERATIONS_MODEL_2026-03-23.md) (Dry-Run → Limited Live → Standard Live), [`doc/RPC_SOLANA_BOT_NOTES.md`](RPC_SOLANA_BOT_NOTES.md) (RPC choice, priority fees, fallbacks), [`doc/OPERATIONAL_CONTINUITY.md`](OPERATIONAL_CONTINUITY.md) (supervisor, restarty, logi).

## 0. Data: minimum position sizing (Orca Whirlpool)

Goal: before any mainnet live action, determine **the smallest operationally safe amounts** to open a position in each target Orca pool **today**.

Important: there is **no single constant “minimum $”**. The effective minimum changes with:
- current price (`current_tick` / `sqrt_price_x64`),
- chosen range (`tick_lower/tick_upper`, in-range vs out-of-range),
- pool params (`tick_spacing`),
- token decimals + rounding.

Treat the result as “**measured at timestamp/slot**” and keep a **quick re-measure** step in your runbook.

**Procedure + table template:** see [`doc/MAINNET_MIN_POSITION_SIZING.md`](MAINNET_MIN_POSITION_SIZING.md).

## 1. Cluster and RPC (fail-fast)

- Set **`SOLANA_RPC_URL`** to a **mainnet** JSON-RPC endpoint (and optional **`SOLANA_RPC_FALLBACK_URLS`** — same cluster only; comma-separated).
- Set **`CLMM_EXPECTED_CLUSTER=mainnet-beta`** when you intend mainnet. The codebase validates **inferable** URLs (e.g. `api.devnet.solana.com` vs `api.mainnet-beta.solana.com`). Custom provider hostnames without keywords are **not** auto-checked — rely on operator discipline.
- If the process panics at startup with `cluster guard (CLMM_EXPECTED_CLUSTER)`, fix URLs or unset **`CLMM_EXPECTED_CLUSTER`**.

## 2. Dry-run (no signing / no state change)

- **`ExecutorConfig::dry_run`** in [`crates/execution/src/strategy/executor.rs`](crates/execution/src/strategy/executor.rs): decision loop and simulated paths without submitting transactions.
- **API** [`crates/api/src/services/position_service.rs`](crates/api/src/services/position_service.rs): defaults to **dry-run** for position operations unless configured otherwise — verify `dry_run` in your deployment before enabling live signing.

Use dry-run against **mainnet RPC** for real pool/price reads while keeping keys unused for txs.

## 3. Limited live (small scope)

Follow **Mode 2 — Limited Live** in [`doc/BOT_OPERATIONS_MODEL_2026-03-23.md`](BOT_OPERATIONS_MODEL_2026-03-23.md): **one pool**, **small capital**, operator available, clear stop procedure.

- Confirm **pool address** (Orca Whirlpool) and **tick/range** from your grid / runbook.
- Set explicit **capital / liquidity caps** in your strategy parameters (project-specific; see execution and API strategy config).
- **Priority fees / retries:** align with [`doc/RPC_SOLANA_BOT_NOTES.md`](RPC_SOLANA_BOT_NOTES.md) and your RPC provider’s limits.

## 4. Stop / circuit breaker

- Execution exposes a **circuit breaker** (see `StrategyExecutor` and `CircuitBreaker` in `clmm-lp-execution`). Use operational runbooks to define when to stop the process (critical alerts, repeated RPC failures, unexpected PnL drift).

## 5. What stays out of this checklist

- Full **fee truth** vs backtest proxy is still covered under [`doc/TODO_ONCHAIN_NEXT_STEPS.md`](TODO_ONCHAIN_NEXT_STEPS.md) — not a prerequisite for first mainnet **observation** or **tiny** live tests.
