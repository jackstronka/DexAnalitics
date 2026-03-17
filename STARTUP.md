# CLMM Liquidity Provider - Startup Guide

This guide explains how to start all services in the correct order to have the complete solution running.

## Prerequisites

Before starting, ensure you have the following installed:

- **Rust**: 1.75+ (`rustup update stable`)
- **Node.js**: 18+ (`node --version`)
- **PostgreSQL**: 14+ (running on port 5432)
- **Make**: Build automation tool

## Quick Start (TL;DR)

```bash
# 1. Setup environment
cp .env.example .env
# Edit .env with your values

# 2. Start PostgreSQL (if not running)
docker run -d --name clmm-postgres \
  -e POSTGRES_USER=clmm_user \
  -e POSTGRES_PASSWORD=clmm_password \
  -e POSTGRES_DB=clmm_lp \
  -p 5432:5432 postgres:14

# 3. Initialize database
cargo run --bin clmm-lp-cli -- db init

# 4. Start API server (Terminal 1)
cargo run --bin clmm-lp-api

# 5. Start Web Dashboard (Terminal 2)
cd web && npm install && npm run dev
```

---

## Detailed Startup Instructions

### Step 1: Environment Configuration

Copy the example environment file and configure it:

```bash
cp .env.example .env
```

Edit `.env` with your values. At minimum, configure:

```bash
# Required for data fetching
BIRDEYE_API_KEY=your_birdeye_api_key

# Required for database
DATABASE_URL=postgres://clmm_user:clmm_password@localhost:5432/clmm_lp

# Optional: Solana RPC (defaults to mainnet)
SOLANA_RPC_URL=https://api.mainnet-beta.solana.com
```

### Step 2: Start PostgreSQL Database

**Option A: Using Docker (Recommended)**

```bash
docker run -d \
  --name clmm-postgres \
  -e POSTGRES_USER=clmm_user \
  -e POSTGRES_PASSWORD=clmm_password \
  -e POSTGRES_DB=clmm_lp \
  -p 5432:5432 \
  postgres:14
```

**Option B: Using local PostgreSQL**

```bash
# Create database and user
psql -U postgres -c "CREATE USER clmm_user WITH PASSWORD 'clmm_password';"
psql -U postgres -c "CREATE DATABASE clmm_lp OWNER clmm_user;"
```

Verify connection:

```bash
psql postgres://clmm_user:clmm_password@localhost:5432/clmm_lp -c "SELECT 1;"
```

### Step 3: Build the Project

```bash
# Build all crates
make build

# Or with release optimizations
cargo build --release --workspace
```

### Step 4: Initialize Database Schema

Run migrations to create the required tables:

```bash
cargo run --bin clmm-lp-cli -- db init
```

Verify the database status:

```bash
cargo run --bin clmm-lp-cli -- db status
```

Expected output:
```
✅ Database connection successful
📊 Tables: pools, simulations, simulation_results, price_history, optimization_results
```

### Step 5: Start the API Server

Open a new terminal and start the API server:

```bash
# Development mode
cargo run --bin clmm-lp-api

# Or production mode
RUST_LOG=info cargo run --release --bin clmm-lp-api
```

The API server will start on `http://localhost:8080`.

Verify it's running:

```bash
curl http://localhost:8080/api/v1/health
```

Expected response:
```json
{"status":"healthy","version":"0.1.1-alpha.3"}
```

**Available endpoints:**
- REST API: `http://localhost:8080/api/v1`
- Swagger UI: `http://localhost:8080/docs`
- WebSocket: `ws://localhost:8080/ws`

### Step 6: Start the Web Dashboard

Open another terminal and start the web dashboard:

```bash
cd web

# Install dependencies (first time only)
npm install

# Start development server
npm run dev
```

The dashboard will be available at `http://localhost:3000`.

> **Note**: The dashboard requires the API server to be running on port 8080.

---

## Service Overview

| Service | Port | URL | Description |
|---------|------|-----|-------------|
| PostgreSQL | 5432 | `localhost:5432` | Database |
| API Server | 8080 | `http://localhost:8080` | REST API + WebSocket |
| Swagger UI | 8080 | `http://localhost:8080/docs` | API Documentation |
| Web Dashboard | 3000 | `http://localhost:3000` | React Frontend |

---

## Startup Order

The services must be started in this order:

```
1. PostgreSQL Database
       ↓
2. Database Initialization (one-time)
       ↓
3. API Server
       ↓
4. Web Dashboard
```

---

## Using the CLI

The CLI can be used independently without the API server:

```bash
# Analyze a trading pair
cargo run --bin clmm-lp-cli -- analyze \
  --symbol-a SOL \
  --symbol-b USDC \
  --days 30

# Run a backtest
cargo run --bin clmm-lp-cli -- backtest \
  --symbol-a SOL \
  --symbol-b USDC \
  --capital 10000 \
  --lower-price 80 \
  --upper-price 120 \
  --strategy periodic

# Optimize range parameters
cargo run --bin clmm-lp-cli -- optimize \
  --symbol-a SOL \
  --symbol-b USDC \
  --capital 10000 \
  --objective sharpe
```

---

## Docker Compose (Full Stack)

For convenience, you can use Docker Compose to start all services:

```yaml
# docker-compose.yml
version: '3.8'

services:
  postgres:
    image: postgres:18
    environment:
      POSTGRES_USER: clmm_user
      POSTGRES_PASSWORD: clmm_password
      POSTGRES_DB: clmm_lp
    ports:
      - "5432:5432"
    volumes:
      - postgres_data:/var/lib/postgresql/data

  api:
    build: .
    ports:
      - "8080:8080"
    environment:
      DATABASE_URL: postgres://clmm_user:clmm_password@postgres:5432/clmm_lp
      RUST_LOG: info
    depends_on:
      - postgres

  web:
    build: ./web
    ports:
      - "3000:3000"
    depends_on:
      - api

volumes:
  postgres_data:
```

Start with:

```bash
docker-compose up -d
```

---

## Troubleshooting

### Database Connection Failed

```
Error: password authentication failed for user "joaquin"
```

**Solution**: Ensure `DATABASE_URL` is set in your `.env` file and you're running from the project root:

```bash
cd /path/to/CLMM-Liquidity-Provider
cargo run --bin clmm-lp-cli -- db init
```

### API Server Port Already in Use

```
Error: Address already in use (os error 48)
```

**Solution**: Change the port or kill the existing process:

```bash
# Find process using port 8080
lsof -i :8080

# Kill it
kill -9 <PID>

# Or use a different port
API_PORT=8081 cargo run --bin clmm-lp-api
```

### Web Dashboard Proxy Errors

```
[vite] http proxy error: /api/v1/health
AggregateError [ECONNREFUSED]
```

**Solution**: The API server is not running. Start it first:

```bash
cargo run --bin clmm-lp-api
```

### Missing Birdeye API Key

```
Error: BIRDEYE_API_KEY not set
```

**Solution**: Add your Birdeye API key to `.env`:

```bash
BIRDEYE_API_KEY=your_api_key_here
```

Get an API key at: https://birdeye.so/

---

## Health Checks

Verify all services are running:

```bash
# Check PostgreSQL
pg_isready -h localhost -p 5432

# Check API Server
curl -s http://localhost:8080/api/v1/health | jq

# Check Web Dashboard
curl -s http://localhost:3000 | head -1
```

---

## Stopping Services

```bash
# Stop Web Dashboard
# Press Ctrl+C in the terminal running npm

# Stop API Server
# Press Ctrl+C in the terminal running cargo

# Stop PostgreSQL (Docker)
docker stop clmm-postgres

# Stop all (Docker Compose)
docker-compose down
```

---

## Next Steps

Once all services are running:

1. Open the **Web Dashboard** at `http://localhost:3000`
2. Explore the **Swagger UI** at `http://localhost:8080/docs`
3. Run your first **analysis** with the CLI
4. Configure a **strategy** and start monitoring positions

For more information, see the [README.md](./README.md).

---

## IL across rebalances (segment-based model)

Backtest and simulation use **segment-based** impermanent loss:

- **Single range**: IL is computed as usual (entry price, current price, range bounds).
- **After each rebalance**: the tracker starts a new “segment”: entry price = price at rebalance, capital = position value at rebalance (after paying rebalance cost). IL for the new range is then computed from this segment entry and current price.

So for a **sequence of ranges** (close → open new → repeat), each segment has its own entry and capital. That gives path-correct position value and makes the comparison **continuous rebalancing vs one wide range** meaningful: both are evaluated over the same price path, with correct per-segment IL and fees/costs.

Implementation: `crates/simulation/src/position_tracker.rs` (fields `segment_entry_price`, `segment_capital`, reset in `execute_rebalance`).

---

## Backtest optimize (auto range + strategy)

The **`backtest-optimize`** command finds a good range and strategy for a given pair and period by running many backtests on the same historical data:

- Fetches **one** price path (Birdeye) and optional Dune TVL/volume.
- Builds a **grid**: several range widths (e.g. 1%–15%) × strategies (static, threshold 2%/3%/5%/7%/10%/15%, periodic 12h/24h/48h/72h), or **only static** with `--static-only` for a faster range-only search.
- Runs backtests in **parallel** (rayon); with `--windows N` (N>1), splits history into N rolling windows and ranks by **average score** across windows for robustness.
- Applies optional **filters**: `--min-time-in-range` (%), `--max-drawdown` (%) so low TIR or high drawdown configs are dropped.
- Ranks by **objective**: `pnl`, `vs_hodl`, `composite` (fees − α·|IL|·capital − cost, `--alpha`), or `risk_adj` (PnL / (1 + max_drawdown)).
- Prints the **best** (range + strategy) and a **table** with Score, PnL, vs HODL, **TIR%**, **IL%** (rounded to 2 decimals).

Example (whETH/SOL, 30 days, maximize vs HODL):

```bash
cargo run --bin clmm-lp-cli -- backtest-optimize \
  --symbol-a whETH --mint-a 7vfCXTUXx5WJV5JADk17DUJ4ksgau7utNKj4b963voxs \
  --symbol-b SOL --mint-b So11111111111111111111111111111111111111112 \
  --days 30 --capital 7000 --tx-cost 0.1 \
  --whirlpool-address HktfL7iwGKT5QHjywQkcDnZXScoh811k7akrMZJkCcEF \
  --lp-share 0.0001 --objective vs_hodl --top-n 5
```

Options: `--objective pnl | vs_hodl | composite | risk_adj`, `--alpha` (for composite), `--range-steps`, `--min-range-pct` / `--max-range-pct`, `--top-n`, `--min-time-in-range` (%), `--max-drawdown` (%), `--static-only`, `--windows` (1 = single period, >1 = rolling windows, score averaged).

**Fee realism:** The BEST block prints a **Fee check** line: period volume (USD), expected fees if 100% TIR (`volume × lp_share × fee_tier`), and simulated fees. With **one window** (`--windows 1`), the ratio (simulated / expected) should be ≤ 100% and close to your fee-weighted time-in-range. With **multiple windows** (`--windows 3` etc.), expected is from the first window and simulated is from the last window of the best config, so the ratio can exceed 100% and is for reference only; use `--windows 1` to compare like-with-like.

**Tuning suggestions:**

| Goal | `--min-range-pct` | `--max-range-pct` | `--min-time-in-range` | `--max-drawdown` | `--windows` |
|------|-------------------|-------------------|------------------------|------------------|------------|
| Default (broad search) | 1 | 15 | (none) | (none) | 1 |
| Narrower ranges only | 3 | 10 | — | — | 1 |
| Avoid low TIR / high DD | 1 | 15 | 20 | 10 | 1 |
| More robust (avg over time) | 1 | 15 | — | — | 3 |
| Conservative + robust | 5 | 12 | 30 | 8 | 3 |

- **Range:** Wider (e.g. 15%) → more time in range, less rebalancing, often less IL; narrower (e.g. 3%) → more fees when in range but more rebalancing and risk of being out of range.
- **min-time-in-range:** Drop configs that were in range &lt; X% of the time (e.g. 20 or 30).
- **max-drawdown:** Drop configs with drawdown &gt; X% (e.g. 10).
- **windows:** Use 3 (or 5) to split history into rolling windows and rank by **average** score; reduces overfitting to one period.

**Refactor (shared logic):** `backtest` and `backtest-optimize` use:
- **Data:** `DuneClient::fetch_tvl_volume_maps(pool)` in `crates/data/src/providers/dune.rs` for a single fetch of TVL + volume maps.
- **CLI:** `crates/cli/src/backtest_engine.rs` – `build_step_data()`, `run_grid()`, `StratConfig`, `run_single()` for shared step data and parallel grid execution.

**Volume (realistic intraday):** Step volume uses a **hybrid** model when Dune is provided: per-candle USD volume from Birdeye (`volume_token_a × close`) gives the **intraday distribution** (which hours had more volume); Dune daily volume gives the **scale** so the day total matches the pool. So `step_vol = dune_daily × (candle_vol_usd / birdeye_day_total)`. That way, hours with high volume (often when price moves a lot and may be out of range) get more volume assigned, and fees are only earned when price is in range—so results are closer to reality than spreading daily volume evenly.

**IL:** Yes. Each backtest run uses the same segment-based IL as the single `backtest` command (see "IL across rebalances" above). The BEST and table show final IL % and it is included in PnL / position value.

**Monte Carlo (symulowane ścieżki cen):** Użyj komendy `optimize`, nie `backtest-optimize`. Optymalizuje zakres na podstawie **wielu losowych ścieżek** (zmienność z historii), a nie jednej realnej ścieżki. Przykład:

```bash
cargo run --bin clmm-lp-cli -- optimize --symbol-a whETH --mint-a 7vfCXTUXx5WJV5JADk17DUJ4ksgau7utNKj4b963voxs --symbol-b SOL --mint-b So11111111111111111111111111111111111111112 --days 30 --capital 7000 --objective pnl --iterations 100
```

---

## Roadmap: Real Dune‑Powered LP Bot

This project also serves as the foundation for a real Orca/Raydium LP bot that uses on‑chain prices and Dune pool metrics (TVL / volume / fees). The high‑level roadmap is:

### Curated pools (seed list)

Store a small curated list of pool addresses we care about, so we can:
- run repeatable backtests/analytics
- build multi‑protocol ranking (same universe of pools)
- later let the bot choose where to deploy

**Orca (Whirlpool)**
- **SOL/USDC**: `Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE`
- **whETH/SOL**: `HktfL7iwGKT5QHjywQkcDnZXScoh811k7akrMZJkCcEF`
- **cbBTC/USDC**: `HxA6SKW5qA4o12fjVgTpXdq2YnZ5Zv1s7SB4FFomsyLM`

**Meteora**
- **SOL/USDC (BIN Step1)**: `HTvjzsfX3yU6BUodCjZ5vZkUrAxMDTrBs3CJaq43ashR`
- **SOL/USDC (BIN Step4)**: `5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6`
- **SOL/USDC (BIN Step10)**: `BGm1tav58oGcsQJehL9WXBFXF7D27vZsKefj4xJKD5Y`

**Raydium**
- **SOL/USDT**: `3nMFwZXwY1s1M5s8vYAHqd4wGs4iSxXE4LRoUMMYqEgF`

1. **Define objective**: choose a clear function to maximize (e.g. `score = α · fees − β · |IL|`), reflecting the desired trade‑off between yield and risk.
2. **Optimize ranges offline**: use historical Birdeye prices + Dune TVL/volume to backtest many candidate ranges and strategies, selecting those that maximize the objective.
3. **Multi‑protocol pool analytics (discovery & ranking)**: implement analytics for a curated set of pools across projects (**Raydium**, **Meteora**, and others). Collect comparable metrics (e.g. daily/weekly **volume**, **TVL**, fee tier, volatility, historical time‑in‑range, stability of volume), then **rank pools** to answer: *“where should the bot deploy capital now?”* (e.g. pick pools with better volume/fees for the same risk).
4. **Approach on‑chain reality (data + fees correctness)**: move fees from candle‑level heuristics toward swap/tick‑level accounting, **without paid APIs**:
   - **B (swap history MVP)**: ingest **historical swaps** from a free dataset (start with **Dune** Solana DEX trades; optionally also **Solana BigQuery / Solarchive** backfills). Use swap timestamps + token amounts to drive **fee estimation at swap granularity** (fees come from swaps, not from candles). Candles (e.g. 1h) can still be used for **strategy logic / valuation / reporting**, but they should no longer be the source of “how much volume paid fees”.
   - **C‑lite (if available in datasets)**: if swaps include decoded `tick` / `sqrt_price` / `liquidity`, upgrade fee share from a constant snapshot to per‑swap active‑liquidity share (much closer to CLMM reality).
   - **C (full CLMM / “truth”)**: build/operate a lightweight indexer (e.g. **SQD/Subsquid** or **Yellowstone/Vixen**) to stream and store **swaps + tick array updates**. Reconstruct tick crossings and implement feeGrowth‑style accounting (fees owed = ΔfeeGrowthInside × L).
   - **Data backup for Dune**: keep a compatible fallback path that can re‑generate the same `SwapEvent` JSONs from:
     - **Solana BigQuery / Solarchive** (transactions + `pre/postTokenBalances`), using a small ETL script to reconstruct swaps for our curated pools and write them to `data/dune-swaps/*.json` in the same format as Dune, and
     - (later) an optional **narrow custom indexer** (SQD/Triton) that streams swaps only for our curated pools and writes them to the same JSON format. Backtests and the bot always read from local JSON cache, never directly from Dune, so switching the upstream source is a matter of changing only the sync/ETL step.
5. **Implement bot loop**: run a periodic agent that reads pool + position state, calls the optimizer, and decides when to enter/exit pools, harvest, close, or reopen positions on‑chain based on the pool ranking + range optimizer.
6. **Hardening and risk controls**: add limits on gas/priority fees, max rebalance frequency, drawdown/IL guards, and monitoring/alerting for the production bot.
7. **Research Hummingbot (code reuse)**: evaluate whether [Hummingbot](https://github.com/hummingbot/hummingbot) can be leveraged for parts of the bot (connectors, strategy scaffolding, event loop/monitoring, risk controls). Decide whether to reuse components or only borrow patterns.
