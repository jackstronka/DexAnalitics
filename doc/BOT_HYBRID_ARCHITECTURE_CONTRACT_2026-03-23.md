# Bot Hybrid Architecture Contract (2026-03-23)

## Decision Summary

Selected approach: **Hybrid**
- Keep our repository as runtime and source of strategy truth.
- Import selected architecture patterns from external solutions.
- Do not adopt a full external framework as the production runtime.

This contract defines:
- why hybrid wins,
- what we import from each source,
- what we explicitly do not import,
- where each element maps into our codebase.

## Weighted Scoring (Decision Closure)

Scoring scale: 1 (poor) to 5 (excellent).

Criteria and weights:
- Strategy fit to our semantics (`StaticRange`, `Periodic`, `Threshold`, `RetouchShift`): **30%**
- Time-to-live for Orca-first deployment: **25%**
- Operational reliability (tx lifecycle, retries, failure handling): **20%**
- Integration cost with existing code: **15%**
- Extensibility to Raydium/Meteora: **10%**

### Option A: External framework as primary runtime (e.g., full Hummingbot runtime)
- Strategy fit: 3
- Time-to-live: 2
- Operational reliability: 4
- Integration cost: 2
- Extensibility: 4
- **Weighted score: 2.95 / 5**

### Option B: Internal-only (no external pattern adoption)
- Strategy fit: 5
- Time-to-live: 3
- Operational reliability: 2
- Integration cost: 5
- Extensibility: 3
- **Weighted score: 3.80 / 5**

### Option C: Hybrid (internal runtime + selected external patterns)
- Strategy fit: 5
- Time-to-live: 4
- Operational reliability: 4
- Integration cost: 4
- Extensibility: 4
- **Weighted score: 4.30 / 5**

Decision: **Option C (Hybrid)**.

## Pattern Import Matrix

### From Hummingbot V2
Import:
- Controller/decision layer separated from execution layer.
- Config-driven strategy behavior and orchestration.
- Clear control loop semantics.

Do not import:
- Full Hummingbot runtime as hard dependency.
- Connector stack replacement for our protocol adapters.

Code mapping:
- `crates/execution/src/strategy/decision.rs` (decision layer)
- `crates/execution/src/strategy/executor.rs` (orchestration loop)

### From Orca Whirlpools Repositioning Bot
Import:
- Orca-first live execution flow patterns.
- Practical tx lifecycle hardening (priority fee, retries, simulation-aware path).
- Failure-mode awareness from PR history (SOL/wSOL and tx-size operational constraints).

Do not import:
- One-to-one copy of their CLI/runtime.

Code mapping:
- `crates/execution/src/strategy/rebalance.rs`
- `crates/execution/src/transaction/manager.rs`
- `crates/protocols/src/orca/executor.rs`

### From Gelato/G-UNI style automation
Import:
- Checker/policy stage before execution.
- Gating primitives: cooldowns, once-until-back-in, risk gates.

Do not import:
- EVM-specific automation infrastructure.

Code mapping:
- `crates/execution/src/strategy/decision.rs`
- `crates/execution/src/strategy/executor.rs`
- `crates/execution/src/lifecycle/*`

### From our existing codebase
Keep as source of truth:
- Backtest semantics and strategy definitions.
- API/CLI contract.
- Domain math and protocol adapters.

Code mapping:
- `crates/cli/src/backtest_engine.rs`
- `crates/api/src/services/strategy_service.rs`
- `crates/domain/*`, `crates/protocols/*`

## Non-Negotiable Invariants

1. Backtest-live semantic parity:
- Live decisions must match backtest semantics for shared strategy modes.

2. RetouchShift invariant:
- shift exiting edge only,
- preserve width in price terms,
- allow at most one retouch per out-of-range episode (`once_until_back_in`).

3. Execution ordering invariant:
- fee collection attempted before range migration.

4. Orca-first invariant:
- production live path starts with Orca single-market;
- Raydium/Meteora remain adapter-ready follow-up.

## Delivery Policy

Phase 1 (Orca-first):
- Harden tx lifecycle and policy gating.
- Validate semantic parity in integration tests.

Phase 2 (Adapter expansion):
- Introduce execution adapters for Raydium/Meteora.
- Reuse same decision/policy abstractions.

## Why This Contract Exists

Without a contract, “hybrid” can drift into ambiguous implementation choices.
This document fixes scope, boundaries, and ownership so future changes can be evaluated consistently.

