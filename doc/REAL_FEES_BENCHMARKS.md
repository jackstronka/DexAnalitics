# Real fees benchmarks (validation)

Purpose: store **user-provided real fees** and the **corresponding backtest outputs** so we can
re-run after code/model changes and compare deltas over time.

---

## Bench 2026-03-30 — window 2026-03-25T12:30:00Z..2026-03-30T11:30:00Z (UTC, end-exclusive)

- **capital**: $100
- **strategy**: `static`
- **resolution_seconds**: 3600
- **fee_source**: `snapshots`
- **price_path_source**:
  - **A**: `snapshots` (full JSONL)
  - **B**: `snapshots` + `--prepared-snapshot-window d30` (prepared cache)

### SOL/USDC (Orca Whirlpool)

- **pool**: `Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE`
- **real fees (user)**:
  - range `89.289–96.763`: `$0.28`
  - range `90.910–94.62`: `$0.50`
  - range `91.824–93.679`: `$0.59`

| range (quote) | real_fees_usd | backtest_A_fees_usd | backtest_B_fees_usd |
|---|---:|---:|---:|
| 89.289–96.763 | 0.28 | 0.25 | 0.25 |
| 90.910–94.62  | 0.50 | 0.22 | 0.22 |
| 91.824–93.679 | 0.59 | 0.11 | 0.11 |

### whETH/SOL (Orca Whirlpool)

- **pool**: `HktfL7iwGKT5QHjywQkcDnZXScoh811k7akrMZJkCcEF`
- **real fees (user)**:
  - range `23.037–23.978`: `$0.69`
  - range `23.279–23.749`: `$0.74`
  - range `23.39–23.635`: `$0.70`
  - range `23.44–23.559`: `$0.59`

| range (quote) | real_fees_usd | backtest_A_fees_usd | backtest_B_fees_usd |
|---|---:|---:|---:|
| 23.037–23.978 | 0.69 | 0.78 | 0.78 |
| 23.279–23.749 | 0.74 | 0.36 | 0.36 |
| 23.39–23.635  | 0.70 | 0.12 | 0.12 |
| 23.44–23.559  | 0.59 | 0.03 | 0.03 |

### Commands (for rerun)

#### SOL/USDC (A)

```bash
cargo run --bin clmm-lp-cli -- backtest \
  --symbol-a SOL --symbol-b USDC --mint-b EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v \
  --snapshot-protocol orca --snapshot-pool-address Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE \
  --fee-source snapshots --price-path-source snapshots \
  --start-date 2026-03-25T12:30:00Z --end-date 2026-03-30T11:30:00Z \
  --capital 100 --resolution-seconds 3600 \
  --lower <LOWER> --upper <UPPER>
```

#### SOL/USDC (B)

```bash
cargo run --bin clmm-lp-cli -- backtest \
  --symbol-a SOL --symbol-b USDC --mint-b EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v \
  --snapshot-protocol orca --snapshot-pool-address Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE \
  --fee-source snapshots --price-path-source snapshots \
  --prepared-snapshot-window d30 \
  --start-date 2026-03-25T12:30:00Z --end-date 2026-03-30T11:30:00Z \
  --capital 100 --resolution-seconds 3600 \
  --lower <LOWER> --upper <UPPER>
```

#### whETH/SOL (A)

```bash
cargo run --bin clmm-lp-cli -- backtest \
  --symbol-a whETH --mint-a 7vfCXTUXx5WJV5JADk17DUJ4ksgau7utNKj4b963voxs \
  --symbol-b SOL --mint-b So11111111111111111111111111111111111111112 \
  --snapshot-protocol orca --snapshot-pool-address HktfL7iwGKT5QHjywQkcDnZXScoh811k7akrMZJkCcEF \
  --fee-source snapshots --price-path-source snapshots \
  --start-date 2026-03-25T12:30:00Z --end-date 2026-03-30T11:30:00Z \
  --capital 100 --resolution-seconds 3600 \
  --lower <LOWER> --upper <UPPER>
```

#### whETH/SOL (B)

```bash
cargo run --bin clmm-lp-cli -- backtest \
  --symbol-a whETH --mint-a 7vfCXTUXx5WJV5JADk17DUJ4ksgau7utNKj4b963voxs \
  --symbol-b SOL --mint-b So11111111111111111111111111111111111111112 \
  --snapshot-protocol orca --snapshot-pool-address HktfL7iwGKT5QHjywQkcDnZXScoh811k7akrMZJkCcEF \
  --fee-source snapshots --price-path-source snapshots \
  --prepared-snapshot-window d30 \
  --start-date 2026-03-25T12:30:00Z --end-date 2026-03-30T11:30:00Z \
  --capital 100 --resolution-seconds 3600 \
  --lower <LOWER> --upper <UPPER>
```

