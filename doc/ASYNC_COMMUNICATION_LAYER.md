# Async communication layer (v2)

## Decision matrix: NATS vs Redis vs Kafka

| Criterion | NATS JetStream | Redis Streams | Kafka |
| --- | --- | --- | --- |
| Ops complexity | Low/medium | Low | High |
| Throughput at scale | High | Medium | Very high |
| Replay/retention | Good | Good | Excellent |
| Consumer groups | Yes | Yes | Yes |
| Multi-team/multi-service enterprise | Medium | Medium | High |
| Recommended project stage | Early-mid | Early-mid | Mid-late |

## Default recommendation for this repo

- Start with `EVENT_BUS_MODE=inprocess` (already wired in API).
- For first broker rollout, use `EVENT_BUS_MODE=broker` + `EVENT_BUS_BACKEND=nats`.
- Promote to Kafka only if all conditions are true:
  - 3+ independent long-running consumers,
  - long retention/replay becomes core requirement,
  - strict operational SLA and auditability exceed NATS/Redis needs.

## Event contract (v1)

All events use the same envelope:

- `event_id` (uuid)
- `event_type` (e.g. `position.updated`, `alert.raised`)
- `event_version` (u16)
- `occurred_at` (UTC)
- `source` (producer component)
- `correlation_id` (trace/correlation key)
- `payload` (JSON)

Idempotency key: `event_type:event_id`.

## Reliability model

- Retry with exponential backoff on publish (`EVENT_BUS_MAX_RETRIES`).
- Duplicate guard (idempotency store with TTL in in-process bus).
- DLQ buffer (in-memory for now) to avoid losing failing events while adapter matures.

## Rollout

1. `inprocess` baseline in all environments.
2. Enable `broker` in shadow mode (`EVENT_BUS_SHADOW_MODE=true`).
3. Switch selected event types first (`position.updated`, `alert.raised`).
4. Disable shadow mode after stability metrics are acceptable.

## Environment knobs

- `EVENT_BUS_MODE=inprocess|broker`
- `EVENT_BUS_BACKEND=nats|redis|kafka`
- `EVENT_BUS_SHADOW_MODE=true|false`
- `EVENT_BUS_MAX_RETRIES=<u8>`

