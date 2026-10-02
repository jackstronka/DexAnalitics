# B4 fixture — backtest mini (Orca SOL/USDC)

**Source:** local `data/pool-snapshots/orca/Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE/snapshots_5m.jsonl` (not committed; gitignored collector output).
**Pool:** `Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE` (Orca SOL/USDC, fee 0.04%).
**Window:** 2026-04-01T00:00:56Z → 2026-04-02T23:59:38Z (**572** consecutive 5m snapshots, two calendar days).
**Sliced:** 2026-10-02. Regeneration: take the same `ts_utc` window; DTO fields are `tick → price_ab` (`1.0001^tick * 10^(9-6)`), `quote_usd = 1`, `lp_share = capital/TVL` with the same TVL formula as `snapshot_price_path`, pool fees from `fee_growth` deltas (`(Δg * L) >> 64`).

| File | Role |
| ---- | ---- |
| `manifest.json` | capital, width, tx cost, decimals, objective |
| `steps.jsonl` | DTO → `StepDataPoint` (one step per line) |
| `fees_by_step.json` | step index → pool fees USD (snapshot `fee_growth` path) |

A snapshot diff is an `economic_regression`. Update only locally (`INSTA_UPDATE=always cargo test -p clmm-lp-cli golden_backtest_mini`) with a “Golden delta” section in the PR.
