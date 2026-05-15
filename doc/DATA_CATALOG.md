# Data Catalog (tagged sources)

Purpose: quickly discover existing datasets before adding new ingestion/snapshot flows.

## Tag schema

- `domain:` lineage | valuation | swaps | lifecycle | backtest | strategy
- `source:` rpc | onchain-account | json-local | jsonl-local | postgres
- `freshness:` realtime | near-realtime | historical
- `quality:` authoritative | best-effort | fallback
- `cost:` free

## Sources

### `data/wallet-ledger-events.jsonl`

tags: domain=wallet,lineage; source=jsonl-local; freshness=near-realtime; quality=best-effort(audit); cost=free

- Append-only **wallet journal** (GL-style): API-originated actions (`swap_before_open`, `open_position`, `transfer_sol`, `convert_sol`) with `pending` / `confirmed` / `failed` and `correlation_id` to tie in-flight rows to outcomes (`transfer_sol` / `convert_sol` use the same id for sender+recipient or for convert pending→outcome).
- **Not** a balance register; UI and automation must not treat it as authoritative SPL/native balances.
- Include in host **backup** / retention policy like other `data/*.jsonl` audit files (loss = weaker post-mortem, not consensus breakage).
- Written by `clmm-lp-api` (`wallet_ledger` service); path override: `CLMM_WALLET_LEDGER_PATH`. Read via **`GET /wallets/ledger-events`**.
- **Roadmap:** docelowo mirror lub wyłączny zapis w **PostgreSQL** (plan kont + wpisy journal / read model); zob. `doc/WALLET_GL.md` §2.1. Do migracji ten plik pozostaje źródłem append.

### `wallet_gl_token_account` (Postgres)

tags: domain=wallet; source=postgres; freshness=static(curated); quality=authoritative(config); cost=free

- Jedno wierszowe **konto księgowe per mint SPL** z listy tokenów występujących w **curated** parach (`mint`, `symbol`, `account_code` = `SPL:{mint}`, `decimals`). Seed: migracja `009_wallet_gl_curated_tokens_and_pools.sql` — **zsynchronizuj** z `crates/api/src/handlers/backtests.rs::curated_backtest_pools()`.

### `wallet_gl_curated_pool` (Postgres)

tags: domain=wallet; source=postgres; freshness=static(curated); quality=authoritative(config); cost=free

- **Pary / poolle** (Orca, Raydium, Meteora) z tej samej listy co backtest: `pair_id`, `protocol`, `pool_address`, minty A/B + symbole. FK do `wallet_gl_token_account`.

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

### `position_chain_history_nodes` (Postgres)

tags: domain=lineage,valuation; source=postgres; freshness=near-realtime; quality=best-effort(read-model); cost=free

- **Materialized read-model** for one resolved rotation chain (UI “Historia pozycji” / lineage table): one row per PDA with `chain_anchor_pubkey` (URL/session anchor), `chain_seq` (1 = oldest), `metrics_mode` (`live` | `settlement_v1`), USD marks, fee legs, cashflow / net PnL, `raw_snapshot` JSONB (pełny `PositionStreamLineageNode` dla szybkiego odczytu API).
- **API (równoległe do `stream-lineage`):** `POST /api/v1/positions/{address}/chain-history/refresh` zapisuje (opcjonalnie chronione env **`CLMM_CHAIN_HISTORY_REFRESH_SECRET`** + nagłówek Bearer lub `X-Chain-History-Refresh`); `GET /api/v1/positions/{address}/chain-history` czyta. Zobacz [`doc/POSITION_CHAIN_HISTORY_PLAN.md`](POSITION_CHAIN_HISTORY_PLAN.md).
- **Meta:** tabela `position_chain_history_meta` (migracja `008_position_chain_history_meta.sql`) trzyma `chain_json`, `totals_json`, `chain_cost_summary_json`, `note` dla pary (anchor, `metrics_mode`).
- **paths:** migracje `007`, `008`; `crates/api/src/services/position_chain_history.rs`; `handlers/positions.rs`; `web/src/lib/api.ts` (`getPositionChainHistory`, `refreshPositionChainHistory`).

### `position_chain_history_meta` (Postgres)

tags: domain=lineage; source=postgres; freshness=on-write; quality=same-as-stream-lineage; cost=free

- Jedna logiczna „koperta” odpowiedzi lineage na `(chain_anchor_pubkey, metrics_mode)`: łańcuch adresów, opcjonalne totals / `chain_cost_summary`, notatka. Uzupełnia wiersze w `position_chain_history_nodes`.

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
