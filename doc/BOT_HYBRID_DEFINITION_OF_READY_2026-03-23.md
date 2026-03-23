# Bot Hybrid Definition of Ready (2026-03-23)

## Purpose

This checklist defines when the team is ready to start implementation sprint(s) for the hybrid architecture decision.

Scope: Orca-first live deployment with strategy-semantic parity and documented operational controls.

## Preconditions (Must Be True Before Sprint Start)

- [ ] Decision documents accepted by stakeholders:
  - `doc/BOT_RESEARCH_DECISION_2026-03-23.md`
  - `doc/BOT_HYBRID_ARCHITECTURE_CONTRACT_2026-03-23.md`
- [ ] Worklog is current:
  - `doc/BOT_WORKLOG_2026-03-23.md`
- [ ] Team agrees that production runtime remains internal (no full external runtime adoption).
- [ ] Orca-first deployment scope is explicitly accepted.

## Technical Ready Criteria

### Strategy Semantics
- [ ] Live strategy modes explicitly mapped to backtest semantics:
  - `StaticRange`
  - `Periodic`
  - `Threshold`
  - `RetouchShift`
- [ ] `RetouchShift` invariants documented and test-covered:
  - edge-only shift,
  - width preservation,
  - `once_until_back_in` gating.

### Execution and Transactions
- [ ] Rebalance flow ordering accepted: fee collection attempted before range migration.
- [ ] Tx lifecycle plan exists for:
  - simulation policy,
  - retry/backoff policy,
  - priority-fee policy,
  - failure taxonomy and alert routing.
- [ ] Wallet handling policy confirmed for mainnet (`PRIVATE_KEY` source and run mode controls).

### API/CLI Contract
- [ ] API parameters aligned with strategy config (`range_width_pct`, thresholds, periodic interval, strategy mode).
- [ ] CLI/API naming consistency reviewed (no conflicting strategy semantics).

## Test Ready Criteria

- [ ] Unit tests exist (or are planned in sprint tasks) for:
  - decision semantics parity,
  - retouch gating behavior,
  - lifecycle reason tagging (`RetouchShift` included).
- [ ] Integration tests planned for Orca-first rebalance flow.
- [ ] Smoke test procedure defined for dry-run and live single-market mode.

## Observability and Operations

- [ ] Lifecycle events are sufficient for post-mortem analysis.
- [ ] Runbook draft exists for:
  - startup,
  - strategy creation,
  - sanity checks,
  - rollback/stop procedures.
- [ ] Alert thresholds and escalation path agreed.

## Data and Environment Readiness

- [ ] RPC endpoint policy confirmed (provider, rate limits, failover expectation).
- [ ] Required environment variables documented and validated.
- [ ] Target Orca market and deployment wallet preselected for first live run.

## Definition of Ready (Go/No-Go Gate)

Sprint can start only if all items below are true:
- [ ] Decision accepted.
- [ ] Scope frozen for Orca-first stage.
- [ ] Technical invariants documented.
- [ ] Test plan exists.
- [ ] Operational runbook baseline exists.

If any of the above is false, status is **No-Go**.

## Definition of Done for Stage 1 (Orca-First)

- [ ] Strategy-semantic parity verified for Stage 1 modes.
- [ ] Orca rebalance flow executes with documented tx policy and observability.
- [ ] Runbook validated in at least one controlled dry-run and one limited live check.
- [ ] Worklog updated with rationale and outcomes.

