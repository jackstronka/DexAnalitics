# Mainnet — minimum position sizing (Orca Whirlpool)

Purpose: define and measure the **minimum viable deposit** to open an Orca Whirlpool position with **minimal cost and risk**, and record results in a way that remains useful even if the market moves tomorrow.

This is intended as a **pre-mainnet-live** checklist item.

---

## Why “minimum” is not constant

There is no single constant minimum USD amount. The effective minimum depends on:

- **pool parameters**: `tick_spacing`
- **current price**: `current_tick` / `sqrt_price_x64`
- **your range**: `tick_lower/tick_upper` (width), and whether the position is **in-range** or **out-of-range**
- **token decimals + rounding**: small deposits can round down to `0` in one leg → `liquidity_delta = 0` or validation failure
- **operational overhead**: creating missing ATAs (rent) + transaction fees (often dominates at tiny sizes)

So: record the result as “**measured at ts/slot**” and keep a **quick re-measure** procedure.

---

## Target pools (from docs)

Orca curated list (see `STARTUP.md`):

- SOL/USDC (0.04%): `Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE`
- whETH/SOL (0.05%): `HktfL7iwGKT5QHjywQkcDnZXScoh811k7akrMZJkCcEF`
- cbBTC/USDC (0.04%): `HxA6SKW5qA4o12fjVgTpXdq2YnZ5Zv1s7SB4FFomsyLM`

If more pools are added later, this procedure stays the same.

---

## What to measure (3 metrics per pool)

For each pool measure and record:

1) **`min_in_range`**: smallest `(amount_a, amount_b)` that successfully opens a position **in-range** with `liquidity_delta > 0`.

2) **`min_out_of_range_single_sided`**: smallest `amount_a` *or* `amount_b` that successfully opens a position **out-of-range** (single-sided deposit) with `liquidity_delta > 0`.

3) **`min_total_operational_cost`**: SOL spent on:
- ATA creation (rent) (if applicable),
- network/priority fee,
- any other fixed on-chain overhead.

Record this separately for:
- **fresh wallet** (ATAs missing),
- **warm wallet** (ATAs already exist).

---

## Range standards (so results are comparable)

Pick and keep these conventions:

- **In-range test range**: choose a deterministic range around the current tick, e.g. “±W ticks” where \(W\) is a fixed multiple of `tick_spacing` (same across runs for a given pool).
- **Out-of-range single-sided test range**: choose a deterministic range fully above or fully below `current_tick` to ensure single-sided deposit.

Always record the actual `tick_lower` and `tick_upper` used.

---

## Quick re-measure method (2-phase search)

For each (pool, scenario):

1) **Exponential ramp**: start from a very small amount (in base units), increase ×2 until you get the first success.
2) **Binary search** between last failure and first success to find the minimum amount(s) that still succeed.

Success criteria:
- open/increase tx succeeds (preflight + send),
- resulting `liquidity_delta > 0`,
- amounts do not round to zero unexpectedly for the scenario (for in-range expect both legs non-zero).

If the code path supports simulation-only, you can do phase (1) with simulate to reduce on-chain side effects, then confirm with a single real tx.

---

## Table template (fill per measurement run)

Record one row per pool + scenario, plus extra rows when you re-measure after market movement.

### Measurement header

- **cluster**: mainnet-beta
- **rpc**: (provider label)
- **wallet_mode**: fresh / warm
- **ts_utc**:
- **slot**:

### Results table

| pool | pair | scenario | fee_tier | tick_spacing | current_tick | tick_lower | tick_upper | amount_a_base | amount_b_base | liquidity_delta | tx_count | sol_spent_total | notes |
|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| `Czfq...44zE` | SOL/USDC | out_of_range_single_sided | 0.04% |  |  |  |  |  |  |  |  |  |  |
| `Czfq...44zE` | SOL/USDC | in_range | 0.04% |  |  |  |  |  |  |  |  |  |  |
| `Hktf...CcEF` | whETH/SOL | out_of_range_single_sided | 0.05% |  |  |  |  |  |  |  |  |  |  |
| `Hktf...CcEF` | whETH/SOL | in_range | 0.05% |  |  |  |  |  |  |  |  |  |  |
| `HxA6...syLM` | cbBTC/USDC | out_of_range_single_sided | 0.04% |  |  |  |  |  |  |  |  |  |  |
| `HxA6...syLM` | cbBTC/USDC | in_range | 0.04% |  |  |  |  |  |  |  |  |  |  |

Notes:
- Keep amounts in **base units** (integers) to avoid ambiguity.
- If you also want a human-friendly number, add a second column in decimals, but do not replace base units.

