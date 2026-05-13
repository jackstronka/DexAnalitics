# Data Catalog (tagged sources)

Purpose: quickly discover existing datasets before adding new ingestion/snapshot flows.

## Tag schema

- `domain:` lineage | valuation | swaps | lifecycle | backtest | strategy
- `source:` rpc | onchain-account | json-local | jsonl-local | postgres
- `freshness:` realtime | near-realtime | historical
- `quality:` authoritative | best-effort | fallback
- `cost:` free

## Sources

### `data/wallet-effective-cache.json`

tags: domain=wallet; source=json-local; freshness=near-realtime; quality=best-effort(read-model); cost=free

- Public read-model cache for API effective wallet balances, keyed by owner pubkey.
- Contains balances, token rows, confidence/staleness metadata, and cache write timestamps only; it must not contain seed phrases, private keys, or keypair bytes.
- Produced by API wallet refresh paths (`GET /wallets/effective-balances?force=true`, periodic resync, WS-triggered refresh). Path override: `CLMM_WALLET_EFFECTIVE_CACHE_PATH`.

### `data/orca_position_lifecycle.jsonl`

tags: domain=lifecycle,lineage; source=jsonl-local; freshness=historical; quality=authoritative(best available for tx lifecycle); cost=free

- Event log for open/close/collect/swap bot operations.
- Includes `rebalance_session_id` and collected LP raw legs (`lp_collected_token_a_raw`, `lp_collected_token_b_raw`) when available.
- **Open/close row `details` (event-time valuation):** on successful bot `open_position` / `open_full_range_position` / `close_position`, the executor best-effort merges:
  - `event_slot` (u64) — confirmation slot when known;
  - `event_price_a_usd`, `event_price_b_usd` — USD spot for pool **token A / B** (same mint order as the Whirlpool), from a fresh pool read + GeckoTerminal (with the same WSOL/USDC tick override heuristic as API Performance);
  - `event_price_source` — e.g. `gecko` or `gecko+pool_tick_wsol`.
  - `open_amount_a_raw`, `open_amount_b_raw` — **measured** token amounts for the opened position legs (raw units), derived post-open from on-chain position liquidity + pool state (best-effort; only for successful opens).
  - `open_amounts_source` — currently `onchain_after_open` when the measurement path succeeds.
  Missing/timeout on enrichment does not block the ledger append. API lineage `persist_event_valuation_snapshots` prefers these fields for `baseline_open` / `end_close` and tags `raw_json.price_time_kind` (`at_tx_event` vs `at_persist_fallback`).

### `position_stream_ledger_rows` (Postgres)

tags: domain=lineage,lifecycle,valuation; source=postgres; freshness=near-realtime; quality=authoritative+best-effort mix; cost=free

- Persisted lifecycle rows used by API lineage services.
- Query-first source for chain/node aggregates and session continuity.

### `position_stream_valuation_snapshots` (Postgres)

tags: domain=valuation,lineage; source=postgres; freshness=near-realtime; quality=best-effort/fallback depending on fill path; cost=free

- Baseline/current valuation snapshots.
- May include backfilled rows tagged with approximate price-source metadata.

### `data/snapshots.jsonl`

tags: domain=valuation,backtest; source=jsonl-local; freshness=near-realtime; quality=best-effort; cost=free

- Periodic pool/account snapshots used by offline analytics and backtests.

### `data/swaps.jsonl` / `data/decoded_swaps.jsonl`

tags: domain=swaps,backtest; source=jsonl-local; freshness=near-realtime; quality=best-effort(decode quality varies); cost=free

- Raw and decoded swap events for fee/proxy analytics and scenario comparisons.

### `data/orca-rest/pool_volume_history.jsonl`

tags: domain=backtest,strategy; source=jsonl-local; freshness=near-realtime; quality=best-effort; cost=free

- Snapshot history of Orca Public API pool stats (volumes for `5m/1h/24h/7d` + TVL).
- Produced by API collector endpoint: `POST /api/v1/pools/orca/volume-history/collect`.
- Designed for easy joins by `pool_address` and `ts_utc` with local snapshot/backtest datasets.

## Usage rule

Before adding new data collection paths:

1. Check this catalog for an existing source with the same `domain`.
2. Prefer reusing/transforming existing data over creating a new snapshot feed.
3. If a new source is necessary, add it here with tags and intended consumers.
