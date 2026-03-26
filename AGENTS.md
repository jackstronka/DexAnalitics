# Agent / AI assistant map

Project: **Bociarz LP Strategy Lab** (derived from upstream CLMM Liquidity Provider, MIT).

Rust **workspace** (edition **2024**). Root: [`Cargo.toml`](Cargo.toml). Human-oriented README: [`README.md`](README.md). Documentation index: [`doc/README.md`](doc/README.md). **Architecture and pipelines:** [`doc/PROJECT_OVERVIEW.md`](doc/PROJECT_OVERVIEW.md) (single high-level source of truth). **Prioritized backlog (fees/RPC/snapshots):** [`doc/TODO_ONCHAIN_NEXT_STEPS.md`](doc/TODO_ONCHAIN_NEXT_STEPS.md) (*Od czego zacząć*, fazy A–F + M).

## Crates (one line each)

| Crate | Role |
| ----- | ---- |
| `clmm-lp-domain` | Domain math (ticks/prices, IL, liquidity), entities |
| `clmm-lp-simulation` | Backtest engine (positions, rebalances, time-in-range) |
| `clmm-lp-optimization` | Range/grid optimizers and objectives |
| `clmm-lp-protocols` | Solana protocol adapters (Orca, Raydium, Meteora) |
| `clmm-lp-data` | External providers and local repos (Birdeye/Jupiter/Dune/swap data) |
| `clmm-lp-execution` | Live monitoring, strategy execution, alerts, scheduler |
| `clmm-lp-api` | REST API (see crate `lib.rs` for endpoints overview) |
| `clmm-lp-cli` | Main CLI; subcommands and `Commands` enum |

## Code entry points

- **CLI:** [`crates/cli/src/main.rs`](crates/cli/src/main.rs) — `Commands` / `Subcommand` dispatch; command implementations under [`crates/cli/src/commands/`](crates/cli/src/commands/).
- **API server:** [`crates/api/src/main.rs`](crates/api/src/main.rs) and [`crates/api/src/lib.rs`](crates/api/src/lib.rs).
- **Execution / monitoring:** [`crates/execution/src/lib.rs`](crates/execution/src/lib.rs) and submodules (e.g. `strategy/`, `monitor/`).
- **Protocols / RPC:** [`crates/protocols/src/lib.rs`](crates/protocols/src/lib.rs), [`crates/protocols/src/rpc/`](crates/protocols/src/rpc/).

Each crate has a short module doc at `crates/<name>/src/lib.rs` (`//!`).

## Local data (do not treat as source to hand-edit for “truth”)

Append-only JSONL under `data/` (see `README.md` and `PROJECT_OVERVIEW.md` for paths): `pool-snapshots/`, `swaps/`, optional `dune-cache/`, etc. Prefer CLI pipelines to regenerate. Optional backup copies may exist under `data-backup/` or similar—confirm with the team before relying on them.

## Conventions

- **Errors:** `thiserror` for typed errors in libraries; `anyhow::Result` is common at CLI / application boundaries.
- **Tests:** `#[cfg(test)]` modules next to code; integration tests under `crates/<crate>/tests/` where present (e.g. CLI).
- **Secrets:** never commit `.env` or API keys; use env vars as in `README.md` (`BIRDEYE_API_KEY`, `DUNE_API_KEY`, etc.).

## When changing behavior users see

Update CLI help text in code, and if the change is user-facing across docs, add a line to [`doc/README.md`](doc/README.md) if a new doc file was added.

## Engineering notes (searchable history)

For **non-trivial** code changes (new behavior, CLI, formats, cross-crate contracts), append a short entry to [`doc/ENGINEERING_NOTES.md`](doc/ENGINEERING_NOTES.md) with a **`keywords:`** line so humans and AI can find it later (`grep`, `@ENGINEERING_NOTES`, codebase search). See the preamble in that file for when to skip.
