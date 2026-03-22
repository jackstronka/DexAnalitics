# On-Chain Fees Progress Log

Last update: 2026-03-20

## What was implemented now

- P1.1 decode data quality audit command:
  - `swaps-decode-audit`
  - computes per-pool `raw_rows`, `decoded_rows`, `ok_rows`, `ok_pct`, `status_counts`, `latest_block_time`
  - can save JSON report to `data/reports/decode_audit_*.json`

- P1.2 comparison report in backtest:
  - during `backtest`, collects LP-fee totals for 3 sources on the same in-range steps:
    - candles
    - swaps
    - snapshots
  - writes `data/reports/backtest_fee_compare_*.json`

- P1.1 decode schema extension:
  - added optional fields in `decoded_swaps.jsonl`:
    - `fee_amount_raw`
    - `tick_after`
    - `sqrt_price_x64_after`
    - `log_swap_mentions`
  - these are extracted heuristically from log text when available
  - schema is now **`schema_version = 3`** for rows produced after the decoder fix (older rows may still show `2` or `1`)

- Monitoring / alerting baseline:
  - `data-health-check` command:
    - stale file alerts (`swaps.jsonl`, `decoded_swaps.jsonl`, `snapshots.jsonl`)
    - decode quality alert (`decode_ok_pct` below threshold)
    - optional non-zero exit with `--fail-on-alert`
  - saves JSON alert report to `data/reports/health_alerts_*.json`

- P2 foundation in snapshots:
  - Orca snapshots now include:
    - `tick_spacing`
    - `tick_neighborhood` around `tick_current`
  - Raydium snapshots now include:
    - `tick_neighborhood` around `tick_current` (coarse)
  - Meteora snapshots now include:
    - `active_bin_neighborhood` around `active_id`

## Recommended timing for P1.1 quality check

- You can run audit now (it already works), but for stable conclusions:
  - minimum: ~2-3 scheduler cycles
  - preferred: at least 24h of collection
- Command:
  - `clmm-lp-cli swaps-decode-audit --save-report`

## Decoder fix (2026-03-19)

Root cause of `decode_status=partial` with no vault resolution: **`meta` was read from the wrong JSON path** (`transaction.meta` instead of top-level `meta` after serde flatten on `EncodedConfirmedTransactionWithStatusMeta`), and **static `accountKeys` omitted v0 ALT accounts** (must append `meta.loadedAddresses` writable+readonly).

- `decoded_swaps.jsonl` schema_version is now **3** for newly decoded rows.
- Rebuild local decoded files: `swaps-enrich-curated-all --refresh-decoded --max-decode <large-enough>`.

## P1.2 Hybrid fill (2026-03-20)

- Backtest and backtest-optimize now do a **bucket-level hybrid**:
  - use decoded swap fees for step buckets where decoded swaps exist,
  - fill missing buckets with the tx-count timing proxy from `swaps.jsonl`.

This prevents the common failure mode where decoded coverage is sparse and strategy fees become ~`$0`.

## Checklist prac (living doc)

Zobacz **`doc/TODO_ONCHAIN_NEXT_STEPS.md`** — pełna lista faz A–E + tech debt.

## B2 — Orca `Traded` event (2026-03-20)

- Implemented in `clmm-lp-protocols::events::whirlpool_traded`:
  - Parses `Program data: <base64>` lines with Anchor discriminator `sha256("event:Traded")[..8]` + Borsh payload matching on-chain `Traded` in `orca-so/whirlpools` `events.rs`.
- `swaps-enrich` / `decode_one_signature` (Orca pools):
  - Fills `fee_amount_raw` ← `lp_fee`, `sqrt_price_x64_after` ← `post_sqrt_price`, and optionally amounts/direction when vault deltas are ambiguous.
  - New decode status: **`ok_traded_event`** (counted as “ok” in `swaps-decode-audit` and accepted with `--fee-swap-decode-status ok` alongside strict `ok`).

## Next follow-ups from this checkpoint

- Raydium / Meteora: protocol-specific event or instruction parsing (same tier as Orca `Traded`).
- Replace Raydium coarse tick neighborhood with real tick-spacing-aware neighborhood.
- Extend P2 with account-level neighborhood liquidity snapshots (tick/bin arrays), not just index neighborhoods.
