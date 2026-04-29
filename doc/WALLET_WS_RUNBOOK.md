# Wallet WS Runbook

Quick operational checklist for near-real-time wallet effective balances.

## Scope

- Backend cache endpoint: `GET /api/v1/wallets/effective-balances?owner=<PUBKEY>`
- Backend diagnostics endpoint: `GET /api/v1/wallets/ws-status`
- Service metrics endpoint: `GET /api/v1/metrics`

## What "healthy" looks like

1. Call `effective-balances` at least once for a wallet owner.
2. Call `ws-status`:
   - `owners_monitored` grows to at least `1`
   - `owners` contains the requested owner pubkey
3. Execute a wallet action (for example `SOL <-> WSOL` convert).
4. Re-check:
   - `metrics.wallet_ws.events_total` increases
   - `effective-balances.as_of_utc` advances quickly (seconds, not minutes)
   - `effective-balances` reflects new projected/verified amounts

## Key counters

- `metrics.wallet_ws.owners_monitored`
  - Number of owner-scoped WS workers currently registered.
- `metrics.wallet_ws.events_total`
  - Number of WS events observed (native account, token program updates, log mentions).
- `metrics.wallet_ws.reconnects_total`
  - Number of reconnect loops after WS worker failures.
- `metrics.wallet_ws.refresh_failures_total`
  - Number of WS-triggered refresh attempts that failed.

## First-line troubleshooting

If balance refresh is slow:

1. Verify owner registration:
   - `GET /api/v1/wallets/ws-status`
   - Ensure owner exists in `owners`.
2. Verify event flow:
   - `GET /api/v1/metrics`
   - Ensure `wallet_ws.events_total` increases during wallet activity.
3. Verify refresh health:
   - Check `wallet_ws.refresh_failures_total`.
   - If increasing, inspect API logs around `refresh_wallet_effective_owner`.
4. Verify fallback path:
   - Ensure periodic resync is configured (`CLMM_WALLET_EFFECTIVE_RESYNC_SECS`) so cache still heals even with WS issues.

## Notes

- WS path is event-driven fast path; periodic resync remains a safety net.
- Public RPC WS quality varies by endpoint. Reconnects may appear even in healthy operation on unstable providers.
