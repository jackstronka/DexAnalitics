# Bot Operations Model (2026-03-23)

## Purpose

This document defines how the bot is operated day-to-day, not only how it is implemented.

It covers:
- operating modes,
- operator procedures,
- alert handling and escalation,
- manual override and emergency actions,
- post-session review and audit trail.

The model is Orca-first and aligned with hybrid architecture decisions.

## Design Inputs Borrowed From Reviewed Models

### From Hummingbot (controllers/executors)
- Clear split between decision policy and execution.
- Explicit lifecycle of actions (create/stop/monitor).
- Config-driven operation and multi-step status checks.

### From Orca Whirlpools repositioning examples
- Practical Solana transaction handling concerns:
  - retries,
  - priority fee policy,
  - simulation-aware send path,
  - edge cases like SOL/wSOL and transaction size.
- Operational emphasis on readable CLI output and failure transparency.

### From Gelato/G-UNI style automation
- Checker-first pattern: verify preconditions before execution.
- Guardrails as operational safety, not strategy replacement.
- Time-based/condition-based cadence for supervision.

## Operating Modes

### Mode 1: Dry-Run
Purpose:
- verify strategy logic, trigger behavior, and lifecycle events without on-chain risk.

Allowed actions:
- decision loop active,
- simulated transaction path,
- no on-chain state changes.

Exit criteria:
- no critical errors in N consecutive cycles,
- strategy triggers observed as expected,
- logs/lifecycle artifacts complete.

### Mode 2: Limited Live (Single-Market)
Purpose:
- real execution with small test capital and strict supervision.

Scope constraints:
- one pool only (`HktfL7iwGKT5QHjywQkcDnZXScoh811k7akrMZJkCcEF` by current preflight),
- small capital test profile,
- operator on-call during session.

Exit criteria:
- stable execution across planned window,
- no unresolved critical alerts,
- net behavior consistent with strategy semantics.

### Mode 3: Standard Live
Purpose:
- normal operation after successful limited-live validation.

Scope:
- still Orca-first in Stage 1,
- scale only after explicit approval.

## Daily Operator Procedure

### Pre-Start Checklist
- [ ] Confirm RPC health and latency baseline.
- [ ] Confirm env variables loaded (`PRIVATE_KEY`, RPC, mode flags).
- [ ] Confirm selected strategy config and pool address.
- [ ] Confirm operating mode (`dry-run` or `limited-live`).
- [ ] Confirm alert channel is active.

### Start Procedure
- [ ] Start services (API/CLI executor) using selected mode.
- [ ] Verify first monitor cycle completed.
- [ ] Verify lifecycle events are being recorded.
- [ ] Verify no immediate configuration errors.

### In-Session Supervision
- [ ] Review trigger decisions vs market moves.
- [ ] Confirm no repeated failure loop (same error across cycles).
- [ ] Confirm rebalance reason tags are correct (`Periodic`, `RangeExit`, `RetouchShift`, etc.).
- [ ] Confirm fee collection attempts happen before range migration.

### End-of-Session
- [ ] Stop strategy gracefully.
- [ ] Export/retain logs and lifecycle summary.
- [ ] Mark incidents and action items in worklog.

## Alert Model and Escalation

### Severity Levels

Info:
- normal lifecycle transitions, successful actions.

Warning:
- temporary RPC issues, transient simulation/send failures, skipped action due to guardrails.

Critical:
- repeated tx failures past retry budget,
- inconsistent strategy state,
- unexpected position state drift,
- key/wallet/config integrity issue.

### Response Targets

Critical:
- acknowledge in <= 5 minutes,
- decide continue/stop in <= 15 minutes.

Warning:
- triage in <= 30 minutes.

Info:
- review in routine post-session checks.

## Guardrails Policy (Minimal, Strategy-Respecting)

Principle:
- strategy remains primary driver for price reaction,
- guardrails only prevent pathological operational behavior.

Enabled minimal guardrails:
- retry budget for tx send failures,
- short anti-loop protection for repeated identical failures,
- emergency stop path.

Not used as hard blockers by default:
- aggressive expected-benefit gating that suppresses valid strategy triggers.

## Manual Override and Emergency Controls

### Manual Override Triggers
- repeated critical failures,
- suspected wrong config in live mode,
- unexpected token balance behavior,
- suspicious divergence from expected strategy behavior.

### Manual Actions
- soft stop: pause decision/execution loop safely,
- hard stop: disable strategy and stop tx attempts,
- emergency exit (if implemented for stage): close position path with operator confirmation.

### Recovery Procedure
- identify root cause from logs + lifecycle,
- apply fix,
- restart in dry-run first if cause was unclear,
- only then return to limited-live.

## Operational Metrics to Track

Core:
- decision cycles count,
- rebalance count by reason,
- fee collection attempts/success rate,
- tx success/failure rate,
- retry count distribution,
- mean time to recovery for incidents.

Quality:
- strategy-semantic parity checks (expected vs observed triggers),
- number of manual interventions,
- incident recurrence.

## Runbook Integration

This operations model complements:
- `doc/ORCA_RUNBOOK.md`
- `doc/BOT_HYBRID_DEFINITION_OF_READY_2026-03-23.md`
- `doc/BOT_WORKLOG_2026-03-23.md`

If procedures conflict, this file defines operator behavior and escalation precedence.

## Stage 1 Operational Constraints

- Orca-only live runtime.
- Single-market deployment for initial operations.
- Small-capital limited-live mode before any scale-up.
- Any scale-up requires explicit post-mortem approval.

