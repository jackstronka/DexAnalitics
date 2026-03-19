# Orca Fees Data Plan (snapshot-fees, tier 2)

## Objective

Make `fee_source=snapshots` work reliably for Orca Whirlpools by collecting, per pool, the minimum on-chain data required to build “fee delta per time step” using snapshot differences:

- `fee_growth_global_a` / `fee_growth_global_b` (global fee growth accumulators)
- `protocol_fee_owed_a` / `protocol_fee_owed_b` (protocol fee counters)
- `liquidity_active` (active liquidity proxy from Whirlpool state)
- vault balances + token identity (to convert token deltas into USD)

This aligns with Orca’s public Whirlpool state model, where `feeGrowthGlobalA/B` and `protocolFeeOwedA/B` live on the Whirlpool account and represent cumulative values. See:
- Orca API schemas: `feeGrowthGlobalA/B`, `protocolFeeOwedA/B` (`PublicWhirlpool`)
- Orca “Understanding Whirlpool Fees” for fee components and meaning of `fee_rate` and `protocol_fee_rate`

## What to collect (per snapshot record)

For each snapshot you should append one JSON object to:

`data/pool-snapshots/orca/<pool_address>/snapshots.jsonl`

### Common identity + conversion fields
- `ts_utc`: snapshot timestamp (RFC3339)
- `slot`: Solana slot at snapshot time
- `pool_address`: whirlpool account address
- `token_mint_a`, `token_mint_b`: token mint addresses (from Whirlpool state)
- `token_vault_a`, `token_vault_b`: SPL token vault addresses (from Whirlpool state)
- `vault_amount_a`, `vault_amount_b`: SPL token account balances for those vaults (fetch 2 token accounts)
- `mint_decimals_a`, `mint_decimals_b`: decimals for USD conversion
  - recommended to include in the snapshot (or store in a local metadata cache and copy into snapshots during ETL)

### Fee proxy inputs (tier 2)
Collect the following directly from the Whirlpool account state:
- `fee_growth_global_a` (u128 string in snapshots)
- `fee_growth_global_b`
- `protocol_fee_owed_a` (u64)
- `protocol_fee_owed_b` (u64)
- `liquidity_active` (from `liquidity`, used to turn growth deltas into token-fee deltas)
- (recommended) `tick_current`, `fee_rate_raw`, `protocol_fee_rate_bps`

Why these fields:
- Orca Whirlpool state exposes these cumulative fields as part of `PublicWhirlpool`.
- Fee growth accumulators increase when swaps execute (so snapshot differences represent total fee flow between timestamps).
- `protocol_fee_owed_*` provides a fallback/counter-like proxy when growth deltas are unusable.

### Notes about fee growth updates
- Orca updates/logs `fee_growth_global_*` during swap instruction execution, so the Whirlpool account’s `feeGrowthGlobalA/B` values move over time.

## Snapshot cadence (time intervals)

### Key constraint from our current snapshot-fees implementation

Our fee-delta builder consumes consecutive snapshot pairs (`pts.windows(2)`), computes a delta from growth counters between the two snapshots, and assigns the resulting USD-fee to a single backtest step bucket:

- `idx = (mid_ts - start_ts) / step_seconds`

Therefore:
- If your snapshot interval is much larger than `step_seconds`, a single delta may span multiple backtest steps, but we currently attribute it to one bucket (not distributed across intermediate steps).

### Recommendation

Let `step_seconds` be the same resolution used by `backtest` / `optimize` for pricing/time slicing (commonly 3600 for 1h).

Recommended snapshot interval for Orca:
- `ORCA_SNAPSHOT_INTERVAL <= step_seconds` (ideally close: `step_seconds` itself)

Practical presets:
- If you backtest with 1h steps: collect Orca snapshots every 1 hour (safe baseline).
- If you want better “in/out of range” fidelity around tick crossings: collect every 15 minutes (or 5 minutes for high-volatility pools).

### Minimum for “readiness” (tier 2)

To pass tier 2 readiness checks, you need at least:
- `with_fee_growth >= 2` (>=2 snapshot rows that include fee growth fields)
- `with_protocol_fee_counter >= 2` (>=2 rows that include protocol fee owed fields)

Because Orca’s Whirlpool state contains these fields, the usual causes of NOT READY are:
- decoding/parsing failures
- missing vault/token identity fields

So ensure parsing succeeds (`parse_ok` in our newer diagnostic snapshots) and that vault balances can be fetched.

## Collection procedure (operational)

### Per pool (cron loop)
1. Fetch Whirlpool account state at time `t` (Solana RPC).
2. From state, get:
   - token mint A/B
   - token vault A/B
   - fee growth and protocol fee counters
   - liquidity_active and tick_current (if available)
3. Fetch SPL token balances for vault A and vault B (two token accounts).
4. Compute `ts_utc` and store `slot`.
5. Append the record to the pool’s JSONL file.

### Cadence scaling / compute budget

If RPC cost becomes an issue later:
- keep vault balance fetches to 2 accounts
- cache `mint_decimals` once per mint
- prefer collecting only the Orca pools you care about (curated list in `STARTUP.md`)

## Expected outputs (what “good” looks like)

After enough snapshots:
- `fee_growth_global_a/b` should change across rows
- `protocol_fee_owed_a/b` should also change across rows
- readiness for tier 2 should become `READY` (for at least the pools you target)

