# Fees Data Plan (Tier 2 snapshot-fees)

## Goal

Enable `fee_source=snapshots` to work as a stable modeling pipeline for all supported venues.

In practice this means:
- For each protocol, snapshot JSONL must include enough data to compute a “fee delta per time step” (fee proxy).
- The existing audit command (`snapshot-readiness`) should classify the snapshot file as READY for:
  - Tier 1: LP-share proxy (used by backtest/optimize fee allocation)
  - Tier 2: Snapshot fee heuristic (experimental fee modeling from fee accumulators / protocol fee counters)

Tier 3 (“inside-range truth”) is intentionally out of scope here.

## Where this is implemented in code

Fee sourcing and fee proxy building happen in:
- `crates/cli/src/main.rs` (fee source selection, snapshot fee index building)
- `crates/cli/src/backtest_engine.rs` and `crates/cli/src/engine/fees.rs` (how LP share is applied and which fee share model is used)

Snapshot collection commands and schema definitions also live in:
- `crates/cli/src/main.rs`

## Snapshot JSONL schema (required fields for Tier 2)

Snapshots are append-only. Each JSON object must contain:

Common fields (all protocols):
- `ts_utc`: ISO-8601 timestamp string (RFC3339)
- `token_mint_a`, `token_mint_b`: token mints for USD conversion
- Vault balances:
  - `vault_amount_a`, `vault_amount_b` (used to compute LP-share proxy in some flows)
- Pool state:
  - `liquidity_active` (used for liquidity-share fee allocation when enabled)

Protocol-specific fee proxy inputs (used for Tier 2 readiness):

### Orca Whirlpool
Tier 2 expects at least one of the following to be present consistently across records:
- `fee_growth_global_a` and `fee_growth_global_b`
- or `protocol_fee_owed_a` and `protocol_fee_owed_b`

Additionally, the fee model builder uses liquidity delta from:
- `liquidity_active`

### Raydium CLMM
Tier 2 expects:
- `fee_growth_global_a_x64` and `fee_growth_global_b_x64`
- or `protocol_fees_token_a` and `protocol_fees_token_b`

Additionally, the fee model builder uses liquidity delta from:
- `liquidity_active`

### Meteora DLMM
Tier 2 expects:
- `protocol_fee_amount_a` and `protocol_fee_amount_b`

In the current code, Meteora also benefits from the same common token/vault/liquidity fields for USD conversion and LP-share computations.

## Recommended snapshot diagnostics (Raydium + Meteora)

To make the system auditable and to fix Tier 2 readiness reliably, snapshot objects should include:
- `parse_ok: bool` (true if account bytes were decoded successfully)
- `parse_error: Option<String>` (details when parsing fails)

This prevents silent failure caused by using `.ok()` on decode results.

## Operational checklist (how to reach READY)

1. Run snapshot collection for at least one pool per protocol:
   - `data/pool-snapshots/orca/<pool>/snapshots.jsonl`
   - `data/pool-snapshots/raydium/<pool>/snapshots.jsonl`
   - `data/pool-snapshots/meteora/<pool>/snapshots.jsonl`

2. Ensure at least 2 records in each file contain:
   - Common identity fields (`ts_utc`, `token_mint_a`, `token_mint_b`)
   - Fee proxy inputs for tier 2 (protocol-specific fields listed above)

3. Run audit:
   - `snapshot-readiness` for each pool (when it is stable)
   - if the audit command fails, validate by inspecting JSON keys in the JSONL.

4. After Tier 2 readiness is achieved for all venues you care about, run:
   - `backtest-optimize` with `--fee-source snapshots` (or `--fee-source auto`)
   - verify “Fee source breakdown” in output (snapshots vs candles vs swaps)

## Notes on scope boundaries

- Tier 2 is about snapshot-derived fee proxies (experimental but aiming to be the default).
- Tier 3 requires additional position-level and range-history data and is not part of this plan.

