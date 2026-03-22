# On-Chain Fees Truth Plan

## Goal

Move fee modeling from candle/snapshot heuristics toward swap-level, on-chain-informed accounting.

Target outcome:
- Backtests use real swap fee flow per pool.
- Fee allocation to LP position uses active liquidity (`L_pos / L_active(t)`), not constant share assumptions.
- Snapshot-only mode remains available, but no longer the primary fee truth source.

## Are these data sources free?

Short answer: mostly yes, with infrastructure caveats.

- Free:
  - Solana RPC public endpoints (rate-limited), self-hosted RPC, or low-cost RPC plans.
  - Local JSONL storage (`data/...`).
  - Dexscreener/Jupiter/CoinGecko public endpoints (limits apply).
  - Existing local snapshot collectors.
- Not inherently free:
  - High-throughput dedicated RPC under heavy historical backfills.
  - Managed indexers / data infra at scale (Subsquid cloud, dedicated Yellowstone infra, etc.).

Conclusion:
- MVP can be done on free/low-cost stack.
- Production-grade reliability usually needs paid infra eventually.

## Architecture split

To reduce heuristic drift, split data into two streams:

1) Swap stream (new, mandatory for better fee truth)
- `data/swaps/<protocol>/<pool>/swaps.jsonl`
- One row per swap with timestamp/slot/signature and fee-related fields.

2) State snapshots (existing, to be expanded)
- `data/pool-snapshots/<protocol>/<pool>/snapshots.jsonl`
- Pool header + tick/bin neighborhood metadata for active liquidity reconstruction.

## Priority roadmap

## P1 (highest value): Swap stream MVP

Why first:
- Biggest accuracy jump vs candle-volume fees.
- Independent from full tick reconstruction.

Scope:
- Add collectors for curated pools:
  - Orca Whirlpools swaps
  - Raydium CLMM swaps
  - Meteora DLMM swaps
- Persist append-only JSONL rows with dedupe by signature + log index.

Minimum swap schema:
- `ts_utc`, `slot`, `signature`, `protocol`, `pool_address`
- `mint_in`, `mint_out`, `amount_in_raw`, `amount_out_raw`
- `fee_tier` and/or `fee_amount_raw` if decodable
- `direction` (`a_to_b`/`b_to_a`) and price-after hint if available (`tick_after`, `sqrt_price_after`, `active_id_after`)

Backtest integration:
- New fee source preference:
  - use swaps when available
  - fallback to snapshots
  - fallback to candles
- P1.2 implemented baseline:
  - local raw swaps timing (`data/swaps/.../swaps.jsonl`) can be used to distribute
    total pool fees across steps by tx-count weights when decoded swap fees are absent.
  - This is a timing proxy, not a final fee-truth model.

## P2: Expand snapshots for active liquidity

Why:
- Swap totals alone still need position share at execution time.

Extend snapshot schema:
- Common:
  - `sqrt_price_x64` / `tick_current` / `active_id`
  - protocol-level liquidity fields
- Orca/Raydium:
  - tick neighborhood (around current and around strategy bounds):
    - `tick_index`, `liquidity_gross`, `liquidity_net`, initialized flag
- Meteora:
  - bin neighborhood:
    - `bin_id`, reserves, liquidity, fee accumulators where available

Cadence:
- Keep global run at 10 minutes.
- For volatile pools, optional 1-5 minute cadence later.

## P3: Per-swap active share model

Objective:
- For each swap, compute LP share from current active liquidity near swap price.

Model:
- `fees_position_swap = fee_swap * (L_pos / L_active_at_swap)`
- Aggregate over swaps in time window.

Data join:
- Swap row joins nearest snapshot state by timestamp (or interpolation for price/tick position).

## P4: Orca near-truth fee growth-inside reconstruction

Objective:
- Use position-range aware fee growth inside (`fee_growth_inside`) reconstruction from tick arrays.

Notes:
- Most accurate and most complex.
- Do after P1-P3 are stable and validated.

## Implementation checklist (concrete)

1. Add docs/schemas:
- Define `swaps.jsonl` schema per protocol in `doc/`.
- Define extended snapshot tick/bin schema and versioning key (`schema_version`).

2. Collectors:
- Add CLI commands:
  - `swaps-sync-curated-all`
  - optional protocol-specific variants.
- Keep idempotent append (dedupe by signature/log index).

3. Backtest engine:
- Add fee source order:
  - swaps -> snapshots -> candles.
- Add `share_model` option:
  - `constant_lp_share` (legacy)
  - `active_liquidity_share` (new default once stable).

4. Validation:
- Add regression tests over cached fixtures:
  - fee totals non-negative
  - no duplicate swap rows after repeated sync
  - stable results for fixed fixture windows
- Add periodic sanity report:
  - `% fees from swaps vs snapshots vs candles`
  - pool-level missing-data diagnostics.

5. Operations:
- Keep Task Scheduler/cron for snapshots (already done, 10 min).
- Add second scheduled task for swaps sync (same cadence or 2-5 min).
- Add a parallel redundant scheduler path for both snapshot and swap sync jobs
  (alternative trigger/task that does the same work with separate logs), so one
  task failure does not stop data collection.

## Risks and mitigation

- RPC limits / missing historical windows:
  - paginate with backoff; persist cursor by pool.
- Decoder drift due to program upgrades:
  - include `parse_ok` + `parse_error`; keep raw payload fallback fields.
- Overfitting to sparse state:
  - keep interpolation explicit and report confidence metrics.

## Definition of done (phase gates)

- P1 done when:
  - swap JSONL exists for curated pools with continuous updates and dedupe.
  - backtest can run with `fee-source swaps` without fallback in recent windows.
- P2 done when:
  - snapshots include neighborhood tick/bin data and pass readiness checks.
- P3 done when:
  - backtest reports fees via per-swap active share for at least Orca + one more venue.
- P4 done when:
  - Orca range-position fee reconstruction is validated against known references.
