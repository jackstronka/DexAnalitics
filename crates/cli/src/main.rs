//! Command Line Interface for the CLMM Liquidity Provider.

pub mod backtest_engine;
pub mod commands;
pub mod engine;
mod local_swap_fees;
pub mod output;
mod snapshots;
mod swap_sync;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use clmm_lp_data::prelude::*;
use clmm_lp_domain::prelude::*;
use clmm_lp_optimization::prelude::*;
use clmm_lp_simulation::prelude::*;
use dotenv::dotenv;
use prettytable::{Table, row};
use primitive_types::U256;
use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};
use std::collections::HashMap;
use std::env;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::info;
use uuid::Uuid;

#[derive(Parser)]
#[command(name = "clmm-lp-cli")]
#[command(about = "Bociarz LP Strategy Lab — Strategy Optimizer CLI", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

/// Optimization objective for range optimization.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum OptimizationObjectiveArg {
    /// Maximize net PnL (fees - IL)
    Pnl,
    /// Maximize fees earned
    Fees,
    /// Maximize simplified Sharpe-style objective in the portfolio `optimize` command (`MaximizeSharpeRatio`). Not used by `backtest-optimize --objective risk_adj`.
    Sharpe,
}

/// Rebalancing strategy for backtest.
#[derive(Debug, Clone, Copy, ValueEnum, Default)]
enum StrategyArg {
    /// No rebalancing - hold initial range
    #[default]
    Static,
    /// Rebalance at fixed intervals
    Periodic,
    /// Rebalance when price moves beyond threshold
    Threshold,
}

/// Objective for backtest-optimize: which metric to maximize.
#[derive(Debug, Clone, Copy, ValueEnum, Default)]
enum BacktestObjectiveArg {
    /// Maximize net PnL (final value - initial capital)
    Pnl,
    /// Maximize LP vs HODL (outperformance over hold)
    #[default]
    #[value(alias = "vs_hodl")]
    VsHodl,
    /// Maximize **gross fees** only (ignores IL / vs HODL). Often picks **narrower** ranges than `vs_hodl` when volume concentrates while in-range.
    #[value(alias = "max_fees")]
    Fees,
    /// Composite: fees - alpha*|IL|*capital - rebalance_cost (use --alpha)
    #[value(alias = "composite")]
    Composite,
    /// Risk-adjusted **score** (not Sharpe): `final_pnl / (1 + max_drawdown)` using equity path drawdown in the backtest.
    #[value(alias = "risk_adj")]
    RiskAdj,
}

#[derive(Debug, Clone, Copy, ValueEnum, Default)]
enum FeeSourceArg {
    /// Default behavior: swaps when provided, otherwise candles.
    #[default]
    Auto,
    /// Candle-volume based fee model.
    Candles,
    /// Dune swap-level fee model.
    Swaps,
    /// Snapshot-derived fee proxy (orca/raydium).
    Snapshots,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum SnapshotProtocolArg {
    Orca,
    Raydium,
    Meteora,
}

/// Where the backtest takes its **price path** from.
#[derive(Debug, Clone, Copy, ValueEnum, Default)]
enum PricePathSourceArg {
    /// Birdeye OHLCV (and cross-pair from USD legs).
    #[default]
    Birdeye,
    /// One step per row in local Orca `snapshots.jsonl` inside the time window (no Birdeye).
    Snapshots,
}

/// How strict to be when reading local `decoded_swaps.jsonl` for fee indexing.
#[derive(Debug, Clone, Copy, ValueEnum, Default)]
enum FeeSwapDecodeStatusArg {
    /// Only rows with `decode_status` in `ok`, `ok_traded_event` (Orca), or `ok_swap_event` (Raydium).
    #[default]
    Ok,
    /// Any successful tx with `amount_in_raw` (older / looser; may mix non-swap txs).
    Loose,
}

type OptimizeGridRow = (f64, f64, f64, String, TrackerSummary, Decimal);

/// After primary `score`, break ties so rankings are not an arbitrary permutation (common when
/// `objective fees` and many strategies share the same range with **zero rebalances** → identical fees).
fn sort_backtest_optimize_grid(
    results: &mut Vec<OptimizeGridRow>,
    objective: BacktestObjectiveArg,
) {
    use std::cmp::Ordering;
    results.sort_by(|a, b| {
        let primary = b.5.partial_cmp(&a.5).unwrap_or(Ordering::Equal);
        if primary != Ordering::Equal {
            return primary;
        }
        match objective {
            BacktestObjectiveArg::Fees => {
                let t =
                    b.4.vs_hodl
                        .partial_cmp(&a.4.vs_hodl)
                        .unwrap_or(Ordering::Equal);
                if t != Ordering::Equal {
                    return t;
                }
                let t = a.4.rebalance_count.cmp(&b.4.rebalance_count);
                if t != Ordering::Equal {
                    return t;
                }
                let t =
                    a.4.total_rebalance_cost
                        .partial_cmp(&b.4.total_rebalance_cost)
                        .unwrap_or(Ordering::Equal);
                if t != Ordering::Equal {
                    return t;
                }
                a.3.cmp(&b.3)
            }
            BacktestObjectiveArg::VsHodl => {
                let t =
                    b.4.total_fees
                        .partial_cmp(&a.4.total_fees)
                        .unwrap_or(Ordering::Equal);
                if t != Ordering::Equal {
                    return t;
                }
                let t = a.4.rebalance_count.cmp(&b.4.rebalance_count);
                if t != Ordering::Equal {
                    return t;
                }
                a.3.cmp(&b.3)
            }
            BacktestObjectiveArg::Pnl
            | BacktestObjectiveArg::Composite
            | BacktestObjectiveArg::RiskAdj => {
                let t =
                    b.4.total_fees
                        .partial_cmp(&a.4.total_fees)
                        .unwrap_or(Ordering::Equal);
                if t != Ordering::Equal {
                    return t;
                }
                let t = a.4.rebalance_count.cmp(&b.4.rebalance_count);
                if t != Ordering::Equal {
                    return t;
                }
                a.3.cmp(&b.3)
            }
        }
    });
}

#[derive(Subcommand)]
enum Commands {
    /// Fetch recent market data
    MarketData {
        /// Token A Symbol (e.g., SOL)
        #[arg(short, long, default_value = "SOL")]
        symbol_a: String,

        /// Token A Mint Address
        #[arg(long, default_value = "So11111111111111111111111111111111111111112")]
        mint_a: String,

        /// Hours of history to fetch
        #[arg(short, long, default_value_t = 24)]
        hours: u64,
    },
    /// Run a backtest on historical data
    Backtest {
        /// Token A Symbol (e.g., SOL)
        #[arg(short, long, default_value = "SOL")]
        symbol_a: String,

        /// Token A Mint Address
        #[arg(long, default_value = "So11111111111111111111111111111111111111112")]
        mint_a: String,

        /// Optional Token B Symbol (e.g., SOL for cross-pair whETH/SOL).
        #[arg(long)]
        symbol_b: Option<String>,

        /// Optional Token B Mint Address (required if symbol_b is set).
        #[arg(long)]
        mint_b: Option<String>,

        /// Days of history to backtest
        #[arg(short, long, default_value_t = 30)]
        days: u64,
        /// Hours of history to backtest (overrides --days when set)
        #[arg(long)]
        hours: Option<u64>,

        /// Optional start date (UTC) in YYYY-MM-DD. Overrides --days.
        #[arg(long)]
        start_date: Option<String>,

        /// Optional end date (UTC) in YYYY-MM-DD (exclusive). Overrides --days.
        /// Example: start=2026-03-07 end=2026-03-15 covers 7 full days (7..14 inclusive).
        #[arg(long)]
        end_date: Option<String>,

        /// Lower price bound
        #[arg(long)]
        lower: f64,

        /// Upper price bound
        #[arg(long)]
        upper: f64,

        /// Initial capital in USD
        #[arg(long, default_value_t = 1000.0)]
        capital: f64,

        /// Rebalancing strategy
        #[arg(long, value_enum, default_value_t = StrategyArg::Static)]
        strategy: StrategyArg,

        /// Rebalance interval in hours (for periodic strategy)
        #[arg(long, default_value_t = 24)]
        rebalance_interval: u64,

        /// Price threshold percentage for rebalance (for threshold strategy)
        #[arg(long, default_value_t = 0.05)]
        threshold_pct: f64,

        /// Transaction cost per rebalance in USD
        #[arg(long, default_value_t = 1.0)]
        tx_cost: f64,
        /// Use realistic rebalance cost model:
        /// fixed(network+priority+jito+tx_cost) + slippage_bps * rebalanced_notional.
        #[arg(long, default_value_t = false)]
        use_realistic_rebalance_cost: bool,
        /// Base network fee in USD per rebalance (used when --use-realistic-rebalance-cost).
        #[arg(long, default_value_t = 0.08)]
        network_fee_usd: f64,
        /// Priority fee in USD per rebalance (used when --use-realistic-rebalance-cost).
        #[arg(long, default_value_t = 0.12)]
        priority_fee_usd: f64,
        /// Optional Jito tip in USD per rebalance (used when --use-realistic-rebalance-cost).
        #[arg(long, default_value_t = 0.0)]
        jito_tip_usd: f64,
        /// Slippage in basis points on rebalance notional (used when --use-realistic-rebalance-cost).
        #[arg(long, default_value_t = 5.0)]
        slippage_bps: f64,
        /// Multiplier `k` for range-aware LP share calibration.
        /// Final share uses: clamp(k * share_est, 0, min(1, legacy_share * share_cap_mult)).
        #[arg(long, default_value_t = 1.0)]
        range_share_k: f64,
        /// Upper cap multiplier vs legacy TVL-share proxy in range-aware mode.
        #[arg(long, default_value_t = 3.0)]
        range_share_cap_mult: f64,

        /// Optional Whirlpool pool address to use real Dune volume data
        #[arg(long)]
        whirlpool_address: Option<String>,

        /// Optional fixed LP share of the pool (e.g. 0.001 = 0.1%)
        #[arg(long)]
        lp_share: Option<f64>,

        /// Optional Dune swaps query id/name to compute fees from swaps (e.g. "orca" or "6848259").
        #[arg(long)]
        dune_swaps: Option<String>,

        /// Fee source model: auto (default), candles, swaps, or snapshots.
        #[arg(long, value_enum, default_value_t = FeeSourceArg::Auto)]
        fee_source: FeeSourceArg,

        /// Protocol for reading pool snapshots (required for --fee-source snapshots).
        #[arg(long, value_enum)]
        snapshot_protocol: Option<SnapshotProtocolArg>,

        /// Pool address to load snapshots from (required for --fee-source snapshots).
        #[arg(long)]
        snapshot_pool_address: Option<String>,

        /// Candle resolution in seconds (e.g. 3600=1h, 1800=30m, 300=5m)
        #[arg(long, default_value_t = 3600)]
        resolution_seconds: u64,

        /// Price path: Birdeye OHLCV (default) or local Orca snapshots only (requires `--snapshot-protocol orca`, `--snapshot-pool-address`, cross-pair mints; no `BIRDEYE_API_KEY`).
        #[arg(long, value_enum, default_value_t = PricePathSourceArg::Birdeye)]
        price_path_source: PricePathSourceArg,

        /// For local `decoded_swaps.jsonl` fee index: `ok` = strict swap rows only; `loose` = legacy filter (success + amount_in_raw).
        #[arg(long, value_enum, default_value_t = FeeSwapDecodeStatusArg::Ok)]
        fee_swap_decode_status: FeeSwapDecodeStatusArg,
    },
    /// Find best range and strategy on historical data (grid search over ranges + strategies)
    BacktestOptimize {
        /// Token A Symbol (e.g., whETH)
        #[arg(short, long)]
        symbol_a: String,
        /// Token A Mint Address
        #[arg(long)]
        mint_a: String,
        /// Token B Symbol (e.g., SOL)
        #[arg(long)]
        symbol_b: Option<String>,
        /// Token B Mint Address (required if symbol_b is set)
        #[arg(long)]
        mint_b: Option<String>,
        /// Days of history
        #[arg(short, long, default_value_t = 30)]
        days: u64,
        /// Hours of history (overrides --days when set)
        #[arg(long)]
        hours: Option<u64>,
        /// Optional start date (UTC) YYYY-MM-DD (with `--price-path-source snapshots`, same window rules as `backtest`).
        #[arg(long)]
        start_date: Option<String>,
        /// Optional end date (UTC) YYYY-MM-DD exclusive (with `--price-path-source snapshots`).
        #[arg(long)]
        end_date: Option<String>,
        /// Initial capital in USD
        #[arg(long, default_value_t = 7000.0)]
        capital: f64,
        /// Transaction cost per rebalance in USD
        #[arg(long, default_value_t = 0.1)]
        tx_cost: f64,
        /// Use realistic rebalance cost model:
        /// fixed(network+priority+jito+tx_cost) + slippage_bps * rebalanced_notional.
        #[arg(long, default_value_t = false)]
        use_realistic_rebalance_cost: bool,
        /// Base network fee in USD per rebalance (used when --use-realistic-rebalance-cost).
        #[arg(long, default_value_t = 0.08)]
        network_fee_usd: f64,
        /// Priority fee in USD per rebalance (used when --use-realistic-rebalance-cost).
        #[arg(long, default_value_t = 0.12)]
        priority_fee_usd: f64,
        /// Optional Jito tip in USD per rebalance (used when --use-realistic-rebalance-cost).
        #[arg(long, default_value_t = 0.0)]
        jito_tip_usd: f64,
        /// Slippage in basis points on rebalance notional (used when --use-realistic-rebalance-cost).
        #[arg(long, default_value_t = 5.0)]
        slippage_bps: f64,
        /// Multiplier `k` for range-aware LP share calibration.
        #[arg(long, default_value_t = 1.0)]
        range_share_k: f64,
        /// Upper cap multiplier vs legacy TVL-share proxy in range-aware mode.
        #[arg(long, default_value_t = 3.0)]
        range_share_cap_mult: f64,
        /// Optional Whirlpool pool address for Dune TVL/volume
        #[arg(long)]
        whirlpool_address: Option<String>,
        /// Optional fixed LP share (e.g. 0.0001 = 0.01%)
        #[arg(long)]
        lp_share: Option<f64>,
        /// What to maximize when ranking the grid. `vs_hodl` (default) often favors **wide** ranges (high TIR, stay near HODL). For **fee-seeking** ranges use `fees` or tune `composite`; cap width with `--max-range-pct`. `risk_adj` uses PnL/(1+max_drawdown), not Sharpe — see `doc/BACKTEST_OPTIMIZE_STRATEGIES.md`.
        #[arg(long, value_enum, default_value_t = BacktestObjectiveArg::VsHodl)]
        objective: BacktestObjectiveArg,
        /// Number of range widths to try (from min to max). E.g. 10 â†’ 1%, 2%, ..., 10%
        #[arg(long, default_value_t = 10)]
        range_steps: usize,
        /// Minimum range width in percent (e.g. 1 = 1%)
        #[arg(long, default_value_t = 1.0)]
        min_range_pct: f64,
        /// Maximum range width in percent (e.g. 15 = 15%)
        #[arg(long, default_value_t = 15.0)]
        max_range_pct: f64,
        /// Show top N rows in the main ranking table (`0` = skip that table; BEST block still prints).
        #[arg(long, default_value_t = 5)]
        top_n: usize,
        /// Print the full main ranking table (all grid rows). Overrides `--top-n` for table size.
        #[arg(long, visible_alias = "all-rows", default_value_t = false)]
        full_ranking: bool,
        /// Min time-in-range %% to keep (0-100, e.g. 20)
        #[arg(long)]
        min_time_in_range: Option<f64>,
        /// Max drawdown %% to keep (0-100, e.g. 10)
        #[arg(long)]
        max_drawdown: Option<f64>,
        /// Alpha for composite objective: score = fees - alpha*|IL|*capital - cost
        #[arg(long, default_value_t = 1.0)]
        alpha: f64,
        /// Only run static strategy (faster, range-only grid)
        #[arg(long, default_value_t = false)]
        static_only: bool,
        /// Number of rolling windows; 1 = single period, >1 = average score over windows
        #[arg(long, default_value_t = 1)]
        windows: usize,
        /// IL-limit max threshold in percent (e.g. 5 = 5%).
        #[arg(long, default_value_t = 5.0)]
        il_max_pct: f64,
        /// Optional IL-limit close threshold in percent (must be >= --il-max-pct).
        #[arg(long)]
        il_close_pct: Option<f64>,
        /// Grace period in steps before IL-limit can trigger.
        #[arg(long, default_value_t = 0)]
        il_grace_steps: u64,
        /// Use Dune swap-level fees: orca, meteora, raydium, or a Dune query ID (e.g. 6848259). Requires DUNE_API_KEY.
        #[arg(long)]
        dune_swaps: Option<String>,
        /// Fee source model: auto (default), candles, swaps, or snapshots.
        #[arg(long, value_enum, default_value_t = FeeSourceArg::Auto)]
        fee_source: FeeSourceArg,
        /// Candle resolution in seconds (e.g. 3600=1h, 900=15m, 300=5m, 60=1m).
        #[arg(long, default_value_t = 3600)]
        resolution_seconds: u64,

        /// Optional: use snapshot vault balances to compute time-varying lp share (capital/TVL proxy).
        #[arg(long, value_enum)]
        snapshot_protocol: Option<SnapshotProtocolArg>,

        /// Optional: pool address to load snapshots from (data/pool-snapshots/{protocol}/{pool}/snapshots.jsonl).
        #[arg(long)]
        snapshot_pool_address: Option<String>,

        /// For local `decoded_swaps.jsonl` when Dune swaps are missing/empty (same as `backtest`).
        #[arg(long, value_enum, default_value_t = FeeSwapDecodeStatusArg::Ok)]
        fee_swap_decode_status: FeeSwapDecodeStatusArg,

        /// Price path: Birdeye OHLCV (default) or local Orca snapshots only (no `BIRDEYE_API_KEY`; same requirements as `backtest`).
        #[arg(long, value_enum, default_value_t = PricePathSourceArg::Birdeye)]
        price_path_source: PricePathSourceArg,

        /// RetouchShift hybrid: min seconds between consecutive retouches (anti-spam). Default 300 s so the 0.3% branch can fire before the 1 h rearm window.
        #[arg(long, default_value_t = 300)]
        retouch_repeat_cooldown_secs: u64,
        /// RetouchShift hybrid: allow another retouch when still OOR for at least this many seconds since last retouch (default 3600 = 1 h).
        #[arg(long, default_value_t = 3600)]
        retouch_repeat_rearm_secs: u64,
        /// RetouchShift hybrid: extra adverse A/B move vs last retouch price (default 0.003 = 0.3%).
        #[arg(long, default_value_t = 0.003)]
        retouch_repeat_extra_move_pct: f64,
        /// Disable hybrid RetouchShift repeat (legacy: one retouch per OOR episode until back in range).
        #[arg(long, default_value_t = false)]
        retouch_repeat_off: bool,

        /// Write machine-readable grid winner JSON for bots / API (`clmm-lp-execution::optimize_profile`).
        #[arg(long, value_name = "PATH")]
        optimize_result_json: Option<std::path::PathBuf>,
        /// Also write the same JSON under this directory as `<UTC timestamp>.json` and `latest.json` (history for AI agents).
        #[arg(long, value_name = "DIR")]
        optimize_result_json_copy_dir: Option<std::path::PathBuf>,
    },
    /// Optimize price range for LP position
    Optimize {
        /// Token A Symbol (e.g., SOL)
        #[arg(short, long, default_value = "SOL")]
        symbol_a: String,

        /// Token A Mint Address
        #[arg(long, default_value = "So11111111111111111111111111111111111111112")]
        mint_a: String,

        /// Optional Token B Symbol (e.g., SOL for cross-pair whETH/SOL).
        #[arg(long)]
        symbol_b: Option<String>,

        /// Optional Token B Mint Address (required if symbol_b is set).
        #[arg(long)]
        mint_b: Option<String>,

        /// Days of history to analyze for volatility
        #[arg(short, long, default_value_t = 30)]
        days: u64,

        /// Initial capital in USD
        #[arg(long, default_value_t = 1000.0)]
        capital: f64,

        /// Optimization objective
        #[arg(long, value_enum, default_value_t = OptimizationObjectiveArg::Pnl)]
        objective: OptimizationObjectiveArg,

        /// Number of Monte Carlo iterations
        #[arg(long, default_value_t = 100)]
        iterations: usize,
    },
    /// Database management commands
    Db {
        #[command(subcommand)]
        action: DbAction,
    },
    /// Analyze a token pair's historical data
    Analyze {
        /// Token A Symbol (e.g., SOL)
        #[arg(short, long, default_value = "SOL")]
        symbol_a: String,

        /// Token A Mint Address
        #[arg(long, default_value = "So11111111111111111111111111111111111111112")]
        mint_a: String,

        /// Optional Token B Symbol (e.g., SOL for cross-pair whETH/SOL).
        #[arg(long)]
        symbol_b: Option<String>,

        /// Optional Token B Mint Address (required if symbol_b is set).
        #[arg(long)]
        mint_b: Option<String>,

        /// Days of history to analyze
        #[arg(short, long, default_value_t = 30)]
        days: u64,
    },
    /// Fetch pool metrics from Dune (TVL, volume, fees)
    /// Fetch Dune swap data from API and save to local cache (data/dune-cache/{query_id}.json).
    /// Use before backtest-optimize --dune-swaps so the backtest uses cached data.
    DuneSyncSwaps {
        /// Protocol preset: orca, meteora, or raydium (uses known query IDs).
        #[arg(long)]
        protocol: Option<String>,
        /// Dune query ID (e.g. 6848259). Overrides --protocol if set.
        #[arg(long)]
        query_id: Option<String>,
    },
    DunePoolMetrics {
        /// Whirlpool pool address
        #[arg(long)]
        pool_address: String,
    },
    /// Print Orca Whirlpool fee tier (on-chain)
    OrcaPoolFee {
        /// Whirlpool pool address
        #[arg(long)]
        pool_address: String,
    },
    /// Find DefiLlama yield pools (discovery helper)
    DefiLlamaFindPools {
        /// Search string (matched against symbol + project)
        #[arg(long)]
        query: String,
        /// Optional chain filter (default: Solana)
        #[arg(long, default_value = "Solana")]
        chain: String,
        /// Max results to print
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Sync DefiLlama daily TVL to local cache
    DefiLlamaSyncTvl {
        /// DefiLlama yield pool id (UUID from /pools)
        #[arg(long)]
        pool_id: String,
    },
    /// Append an on-chain snapshot for an Orca Whirlpool pool (for local "cron" collection)
    OrcaSnapshot {
        /// Whirlpool pool address
        #[arg(long)]
        pool_address: String,
    },
    /// Snapshot all curated Orca Whirlpool pools from `STARTUP.md`
    OrcaSnapshotCurated {
        /// Optional: stop after N pools (useful for testing)
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Append an on-chain raw account snapshot for Raydium pools from `STARTUP.md` (v1 minimal)
    RaydiumSnapshotCurated {
        /// Optional: stop after N pools (useful for testing)
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Append an on-chain raw account snapshot for Meteora pools from `STARTUP.md` (v1 minimal)
    MeteoraSnapshotCurated {
        /// Optional: stop after N pools (useful for testing)
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Snapshot curated pools for all supported protocols (Orca + Raydium + Meteora)
    SnapshotRunCuratedAll {
        /// Optional: stop after N pools per protocol (useful for testing)
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Sync raw on-chain transaction stream for curated pools (P1 swaps MVP).
    SwapsSyncCuratedAll {
        /// Optional: stop after N pools per protocol (useful for testing)
        #[arg(long)]
        limit: Option<usize>,
        /// Max signatures fetched per pool per run.
        #[arg(long, default_value_t = 300)]
        max_signatures: usize,
        /// Max paginated `getSignaturesForAddress` pages per pool (1 keeps previous behavior).
        #[arg(long, default_value_t = 1)]
        max_pages: usize,
    },
    /// Live subscribe on Solana logs by `mentions` and append raw signatures into `data/swaps/.../swaps.jsonl`.
    SwapsSubscribeMentions {
        /// Protocol namespace for output path: orca, raydium, meteora.
        #[arg(long)]
        protocol: String,
        /// Pool address used as output directory key under `data/swaps/<protocol>/<pool_address>/`.
        #[arg(long)]
        pool_address: String,
        /// Program/account pubkey to use in `logsSubscribe` mentions filter.
        /// If omitted, set `--mentions-preset`.
        #[arg(long)]
        mentions: Option<String>,
        /// Built-in mentions presets: `orca`, `raydium`, `meteora`.
        /// Used only when `--mentions` is not provided.
        #[arg(long)]
        mentions_preset: Option<String>,
        /// Stop after processing this many websocket events.
        #[arg(long, default_value_t = 300)]
        max_events: usize,
        /// Stop when no new event arrives within this timeout.
        #[arg(long, default_value_t = 60)]
        idle_timeout_secs: u64,
    },
    /// Decode raw swap stream into vault-delta swap rows (P1.1).
    SwapsEnrichCuratedAll {
        /// Optional: stop after N pools per protocol (useful for testing)
        #[arg(long)]
        limit: Option<usize>,
        /// Max raw signatures decoded per pool per run.
        #[arg(long, default_value_t = 120)]
        max_decode: usize,
        /// Timeout per decode attempt (seconds).
        #[arg(long, default_value_t = 20)]
        decode_timeout_secs: u64,
        /// Number of retries after first failed/timeout decode attempt.
        #[arg(long, default_value_t = 2)]
        decode_retries: usize,
        /// Max concurrent `getTransaction` calls per run (bounded to 32). Use 1 for strict sequential behavior.
        #[arg(long, default_value_t = 4)]
        decode_concurrency: usize,
        /// Random delay 0..jitter_ms before each decode attempt (spreads load on public RPC).
        #[arg(long, default_value_t = 0)]
        decode_jitter_ms: u64,
        /// Delete existing `decoded_swaps.jsonl` per pool and re-decode from raw (after decoder fixes).
        #[arg(long, default_value_t = false)]
        refresh_decoded: bool,
    },
    /// Audit decode quality for P1.1 (coverage + decode_status histogram).
    SwapsDecodeAudit {
        /// Optional: stop after N pools per protocol (useful for testing)
        #[arg(long)]
        limit: Option<usize>,
        /// Save JSON report in data/reports/.
        #[arg(long, default_value_t = true)]
        save_report: bool,
    },
    /// Health check for snapshots/swaps pipelines with staleness + decode quality alerts.
    DataHealthCheck {
        /// Alert if file age is above this threshold.
        #[arg(long, default_value_t = 30)]
        max_age_minutes: i64,
        /// Alert if decoded ok percentage drops below this threshold.
        #[arg(long, default_value_t = 65.0)]
        min_decode_ok_pct: f64,
        /// Exit with error when alerts exist (for scheduler/monitor integration).
        #[arg(long, default_value_t = false)]
        fail_on_alert: bool,
    },
    /// One-shot ops cycle: snapshots -> swaps sync -> enrich -> audit -> health-check.
    /// Intended for automation (Task Scheduler / cron).
    OpsIngestCycle {
        /// Optional: stop after N pools per protocol (useful for testing)
        #[arg(long)]
        limit: Option<usize>,
        /// Run snapshot collectors first (recommended for consistent meta for enrich).
        #[arg(long, default_value_t = true)]
        run_snapshots: bool,
        /// Max signatures fetched per pool for swaps sync.
        #[arg(long, default_value_t = 600)]
        swaps_max_signatures: usize,
        /// Max paginated pages per pool for swaps sync.
        #[arg(long, default_value_t = 2)]
        swaps_max_pages: usize,
        /// Max raw signatures decoded per pool per run.
        #[arg(long, default_value_t = 160)]
        enrich_max_decode: usize,
        /// Timeout per decode attempt (seconds).
        #[arg(long, default_value_t = 20)]
        enrich_decode_timeout_secs: u64,
        /// Number of retries after first failed/timeout decode attempt.
        #[arg(long, default_value_t = 2)]
        enrich_decode_retries: usize,
        /// Max concurrent `getTransaction` calls (bounded to 32).
        #[arg(long, default_value_t = 4)]
        enrich_decode_concurrency: usize,
        /// Random delay 0..jitter_ms before each decode attempt.
        #[arg(long, default_value_t = 0)]
        enrich_decode_jitter_ms: u64,
        /// Delete existing `decoded_swaps.jsonl` per pool and re-decode from raw.
        #[arg(long, default_value_t = false)]
        enrich_refresh_decoded: bool,
        /// Health check: alert if file age exceeds this threshold.
        #[arg(long, default_value_t = 30)]
        health_max_age_minutes: i64,
        /// Health check: alert if decoded ok percentage drops below this threshold.
        #[arg(long, default_value_t = 65.0)]
        health_min_decode_ok_pct: f64,
        /// Exit with error when health alerts exist (for scheduler integration).
        #[arg(long, default_value_t = true)]
        fail_on_alert: bool,
    },
    /// Continuous ops loop: run `ops-ingest-cycle`, then sleep for an interval (+ jitter).
    /// Intended to be run as a long-lived process (e.g. Windows Service via NSSM).
    OpsIngestLoop {
        /// Optional: stop after N pools per protocol (useful for testing)
        #[arg(long)]
        limit: Option<usize>,
        /// Run snapshot collectors each cycle.
        #[arg(long, default_value_t = true)]
        run_snapshots: bool,
        /// Max signatures fetched per pool for swaps sync.
        #[arg(long, default_value_t = 600)]
        swaps_max_signatures: usize,
        /// Max paginated pages per pool for swaps sync.
        #[arg(long, default_value_t = 2)]
        swaps_max_pages: usize,
        /// Max raw signatures decoded per pool per run.
        #[arg(long, default_value_t = 160)]
        enrich_max_decode: usize,
        /// Timeout per decode attempt (seconds).
        #[arg(long, default_value_t = 20)]
        enrich_decode_timeout_secs: u64,
        /// Number of retries after first failed/timeout decode attempt.
        #[arg(long, default_value_t = 2)]
        enrich_decode_retries: usize,
        /// Max concurrent `getTransaction` calls (bounded to 32).
        #[arg(long, default_value_t = 4)]
        enrich_decode_concurrency: usize,
        /// Random delay 0..jitter_ms before each decode attempt.
        #[arg(long, default_value_t = 0)]
        enrich_decode_jitter_ms: u64,
        /// Delete existing `decoded_swaps.jsonl` per pool and re-decode from raw.
        #[arg(long, default_value_t = false)]
        enrich_refresh_decoded: bool,
        /// Health check: alert if file age exceeds this threshold.
        #[arg(long, default_value_t = 30)]
        health_max_age_minutes: i64,
        /// Health check: alert if decoded ok percentage drops below this threshold.
        #[arg(long, default_value_t = 65.0)]
        health_min_decode_ok_pct: f64,
        /// If true: stop the loop on health alerts (exit non-zero).
        #[arg(long, default_value_t = true)]
        fail_on_alert: bool,
        /// Base interval between cycles.
        #[arg(long, default_value_t = 900)]
        interval_secs: u64,
        /// Add random jitter 0..jitter_secs to the sleep interval.
        #[arg(long, default_value_t = 60)]
        jitter_secs: u64,
        /// Backoff base used after a failed cycle (seconds), multiplied by (1 + consecutive_failures).
        #[arg(long, default_value_t = 30)]
        error_backoff_base_secs: u64,
        /// Optional: stop after N cycles (useful for smoke testing).
        #[arg(long)]
        max_cycles: Option<u64>,
    },
    /// Live Orca Whirlpool bot: poll LP position + run the same `StrategyExecutor` loop as `clmm-lp-api`.
    /// Default: dry-run (log only). Use `--execute` to sign transactions (`--keypair` or `SOLANA_KEYPAIR`).
    OrcaBotRun {
        /// Whirlpool **position** (NFT) address to monitor.
        #[arg(long)]
        position: String,
        /// Path to Solana keypair JSON. Not required in dry-run; for `--execute` use `--keypair`, `KEYPAIR_PATH`, or `SOLANA_KEYPAIR`.
        #[arg(long)]
        keypair: Option<std::path::PathBuf>,
        /// Submit on-chain txs (rebalance / close / …). Without this flag, decisions are logged only (`dry_run`).
        #[arg(long, default_value_t = false)]
        execute: bool,
        /// Strategy evaluation period (seconds).
        #[arg(long, default_value_t = 300)]
        eval_interval_secs: u64,
        /// On-chain position poll period (seconds).
        #[arg(long, default_value_t = 30)]
        poll_interval_secs: u64,
        /// Optional `backtest-optimize` winner JSON → live `DecisionConfig` (same as API `apply-optimize-result`).
        #[arg(long)]
        optimize_result_json: Option<std::path::PathBuf>,
    },
    /// Open a new Orca Whirlpool LP position (on-chain). Without `--dry-run` requires a signing key.
    OrcaPositionOpen {
        /// Whirlpool pool address.
        #[arg(long)]
        pool: String,
        #[arg(long)]
        keypair: Option<std::path::PathBuf>,
        /// Print ticks + position PDA only (no transaction).
        #[arg(long, default_value_t = false)]
        dry_run: bool,
        /// Lower tick (must align to pool `tick_spacing`; use with `--tick-upper`).
        #[arg(long)]
        tick_lower: Option<i32>,
        /// Upper tick.
        #[arg(long)]
        tick_upper: Option<i32>,
        /// Alternative: symmetric range width in percent of price (e.g. `10` = 10%% total band, like backtest width).
        #[arg(long)]
        range_width_pct: Option<f64>,
        /// Max slippage in basis points for open + increase.
        #[arg(long, default_value_t = 50)]
        slippage_bps: u16,
    },
    /// Partially remove liquidity from an existing Whirlpool position (signs `decrease_liquidity`).
    OrcaPositionDecrease {
        /// Position (NFT) address.
        #[arg(long)]
        position: String,
        #[arg(long)]
        keypair: Option<std::path::PathBuf>,
        #[arg(long, default_value_t = false)]
        dry_run: bool,
        /// Remove this percent of current liquidity (0–100; mutually exclusive with `--liquidity`).
        #[arg(long)]
        liquidity_pct: Option<f64>,
        /// Remove exactly this liquidity amount in pool units (mutually exclusive with `--liquidity-pct`).
        #[arg(long)]
        liquidity: Option<u128>,
    },
    /// Show last snapshot-run-curated-all status from local JSONL log
    SnapshotStatusLast,
    /// Audit whether local snapshots are sufficient for each fee model tier
    SnapshotReadiness {
        /// Protocol of the snapshot file
        #[arg(long, value_enum)]
        protocol: SnapshotProtocolArg,
        /// Pool address used in data/pool-snapshots/{protocol}/{pool}/snapshots.jsonl
        #[arg(long)]
        pool_address: String,
    },

    /// Dexscreener: search pairs by query (cached)
    DexscreenerSearch {
        /// Search query (symbol, token, etc.)
        #[arg(long)]
        query: String,
        /// Max rows to print
        #[arg(long, default_value_t = 25)]
        limit: usize,
    },

    /// Dexscreener: list all pairs for a Solana token mint (cached)
    DexscreenerTokenPairs {
        /// Token mint address (Solana)
        #[arg(long)]
        token_mint: String,
        /// Max rows to print
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },

    /// Dexscreener: fetch details for a specific pair address (cached)
    DexscreenerPair {
        /// Pair address (e.g. pool address)
        #[arg(long)]
        pair_address: String,
    },

    /// Dexscreener: compare venues for a given token pair on Solana (cached)
    DexscreenerComparePair {
        /// Token mint A (Solana)
        #[arg(long)]
        mint_a: String,
        /// Token mint B (Solana)
        #[arg(long)]
        mint_b: String,
        /// Sort by: liquidity_usd, volume_h24, volume_h6, volume_h1, volume_m5
        #[arg(long, default_value = "volume_h24")]
        sort_by: String,
        /// Max rows to print
        #[arg(long, default_value_t = 25)]
        limit: usize,
    },

    /// Studio: generate a local JSONL "stream plan" (segments with narration templates).
    ///
    /// This is a local-first MVP: it does **not** call external APIs or TTS. It produces JSONL
    /// that can later be spoken by TTS and rendered in OBS.
    StudioStreamPlan {
        /// Input JSONL with items to narrate (expects at least `title` or `headline`).
        #[arg(long, default_value = "data/studio/inputs/items.jsonl")]
        input_jsonl: std::path::PathBuf,
        /// Output JSONL with planned segments.
        #[arg(long, default_value = "data/studio/out/segments.jsonl")]
        output_jsonl: std::path::PathBuf,
        /// Language of the narrator template.
        #[arg(long, default_value = "pl")]
        lang: String,
        /// Style label (e.g. neutral, satirical). Used only inside the template.
        #[arg(long, default_value = "neutral")]
        style: String,
        /// Suggested pause before narration (viewer reads headline).
        #[arg(long, default_value_t = 10)]
        pause_secs: u64,
        /// Max number of items to turn into segments.
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
}

/// Database management actions.
#[derive(Subcommand)]
enum DbAction {
    /// Initialize the database with migrations
    Init,
    /// Show database connection status
    Status,
    /// List recent simulations
    ListSimulations {
        /// Maximum number of results
        #[arg(short, long, default_value_t = 10)]
        limit: i64,
    },
    /// List recent optimizations
    ListOptimizations {
        /// Maximum number of results
        #[arg(short, long, default_value_t = 10)]
        limit: i64,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match &cli.command {
        Commands::MarketData {
            symbol_a,
            mint_a,
            hours,
        } => {
            let api_key = env::var("BIRDEYE_API_KEY")
                .expect("BIRDEYE_API_KEY must be set in .env or environment");

            info!("đź“ˇ Initializing Birdeye Provider...");
            let provider = BirdeyeProvider::new(api_key);

            // Define Tokens (Token B assumed USDC for this demo)
            let token_a_decimals: u8 = {
                use crate::engine::token_meta::fetch_mint_decimals;
                use clmm_lp_protocols::rpc::RpcProvider;
                let rpc = RpcProvider::mainnet();
                fetch_mint_decimals(&rpc, mint_a).await.unwrap_or(9)
            };
            let token_a = Token::new(mint_a, symbol_a, token_a_decimals, symbol_a);
            let token_b = Token::new(
                "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
                "USDC",
                6,
                "USD Coin",
            );

            let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
            let start_time = now - (hours * 3600);

            info!(
                "đź”Ť Fetching data for {}/USDC from {} to {}...",
                symbol_a, start_time, now
            );

            // Fetch 1-hour candles
            let candles = provider
                .get_price_history(
                    &token_a, &token_b, start_time, now, 3600, // 1h resolution
                )
                .await?;

            println!("âś… Fetched {} candles:", candles.len());
            println!();

            let mut table = Table::new();
            table.add_row(row!["Time", "Open", "High", "Low", "Close"]);

            for candle in candles {
                let datetime = chrono::DateTime::from_timestamp(candle.start_timestamp as i64, 0)
                    .unwrap_or_default();
                table.add_row(row![
                    datetime.format("%Y-%m-%d %H:%M"),
                    format!("{:.4}", candle.open.value),
                    format!("{:.4}", candle.high.value),
                    format!("{:.4}", candle.low.value),
                    format!("{:.4}", candle.close.value)
                ]);
            }
            table.printstd();
        }
        Commands::Backtest {
            symbol_a,
            mint_a,
            symbol_b,
            mint_b,
            days,
            hours,
            start_date,
            end_date,
            lower,
            upper,
            capital,
            strategy,
            rebalance_interval,
            threshold_pct,
            tx_cost,
            use_realistic_rebalance_cost,
            network_fee_usd,
            priority_fee_usd,
            jito_tip_usd,
            slippage_bps,
            range_share_k,
            range_share_cap_mult,
            whirlpool_address,
            lp_share,
            dune_swaps,
            fee_source,
            snapshot_protocol,
            snapshot_pool_address,
            resolution_seconds,
            price_path_source,
            fee_swap_decode_status,
        } => {
            println!("đź“ˇ Initializing Backtest Engine...");

            // Define Tokens
            let (token_a_decimals, token_b_decimals): (u8, u8) = {
                use crate::engine::token_meta::fetch_mint_decimals;
                use clmm_lp_protocols::rpc::RpcProvider;
                let rpc = RpcProvider::mainnet();
                let da = fetch_mint_decimals(&rpc, mint_a).await.unwrap_or(9);
                let db = if let Some(mb) = mint_b.as_ref() {
                    fetch_mint_decimals(&rpc, mb).await.unwrap_or(9)
                } else {
                    6u8
                };
                (da, db)
            };
            let token_a = Token::new(mint_a, symbol_a, token_a_decimals, symbol_a);

            let (token_b, use_cross_pair) =
                if let (Some(sb), Some(mb)) = (symbol_b.as_ref(), mint_b.as_ref()) {
                    // User supplied an explicit quote token (e.g., SOL for whETH/SOL)
                    let tb = Token::new(mb, sb, token_b_decimals, sb);
                    (tb, true)
                } else {
                    // Default to USDC quote
                    let tb = Token::new(
                        "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
                        "USDC",
                        6,
                        "USD Coin",
                    );
                    (tb, false)
                };

            let snapshots_only = matches!(price_path_source, PricePathSourceArg::Snapshots);
            if snapshots_only {
                if !matches!(
                    snapshot_protocol,
                    Some(SnapshotProtocolArg::Orca)
                        | Some(SnapshotProtocolArg::Raydium)
                        | Some(SnapshotProtocolArg::Meteora)
                ) {
                    anyhow::bail!(
                        "--price-path-source snapshots requires --snapshot-protocol orca|raydium|meteora"
                    );
                }
                if snapshot_pool_address.is_none() {
                    anyhow::bail!("--price-path-source snapshots requires --snapshot-pool-address");
                }
                if !use_cross_pair {
                    anyhow::bail!(
                        "--price-path-source snapshots requires a cross pair (--symbol-b and --mint-b)"
                    );
                }
            } else {
                let _ = env::var("BIRDEYE_API_KEY")
                    .expect("BIRDEYE_API_KEY must be set in .env or environment (or use --price-path-source snapshots)");
            }

            let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
            let (start_time, end_time, display_label) =
                if let (Some(sd), Some(ed)) = (start_date.as_deref(), end_date.as_deref()) {
                    use chrono::{NaiveDate, TimeZone, Utc};
                    let s = NaiveDate::parse_from_str(sd, "%Y-%m-%d")?;
                    let e = NaiveDate::parse_from_str(ed, "%Y-%m-%d")?;
                    let s_ts = Utc
                        .from_utc_datetime(&s.and_hms_opt(0, 0, 0).unwrap())
                        .timestamp() as u64;
                    let e_ts = Utc
                        .from_utc_datetime(&e.and_hms_opt(0, 0, 0).unwrap())
                        .timestamp() as u64;
                    (s_ts, e_ts, format!("{}..{}", sd, ed))
                } else if let Some(h) = hours {
                    let s_ts = now.saturating_sub(h.saturating_mul(3600));
                    (s_ts, now, format!("last {}h", h))
                } else {
                    let s_ts = now - (days * 24 * 3600);
                    (s_ts, now, format!("last {}d", days))
                };
            let effective_days: u64 = (end_time.saturating_sub(start_time) / 86_400).max(1);

            let mut prebuilt_snapshot_fee_index: Option<
                std::collections::BTreeMap<usize, Decimal>,
            > = None;

            let mut candles: Vec<clmm_lp_domain::entities::price_candle::PriceCandle> = Vec::new();

            #[allow(clippy::type_complexity)]
            let (dune_daily_tvl, dune_daily_volume): (
                Option<HashMap<String, Decimal>>,
                Option<HashMap<String, Decimal>>,
            ) = if snapshots_only {
                (None, None)
            } else if let Some(pool) = whirlpool_address.as_ref() {
                crate::commands::backtest_optimize::fetch_dune_tvl_volume(pool).await?
            } else {
                (None, None)
            };

            let capital_dec = Decimal::from_f64(*capital).unwrap();
            let lp_share_override: Option<Decimal> = lp_share
                .and_then(Decimal::from_f64)
                .filter(|s| *s > Decimal::ZERO && *s <= Decimal::ONE);

            let mut step_data: Vec<crate::backtest_engine::StepDataPoint> = if snapshots_only {
                println!(
                    "đź”Ť Price path from Orca snapshots only (window {}): {}/{} â€” no Birdeye",
                    display_label,
                    symbol_a,
                    symbol_b.as_deref().unwrap_or("?")
                );
                let pool = snapshot_pool_address.as_ref().unwrap();
                let prep = crate::commands::snapshot_price_path::build_from_orca_snapshots(
                    pool,
                    start_time as i64,
                    end_time as i64,
                    &token_a,
                    &token_b,
                    *capital,
                    *lp_share,
                )
                .await?;
                prebuilt_snapshot_fee_index = prep.per_step_fees_usd;
                if prep.step_data.is_empty() {
                    println!("âťŚ No snapshot rows in the requested time window.");
                    return Ok(());
                }
                println!(
                    "âś… Loaded {} snapshot steps from data/pool-snapshots/orca/{}/snapshots.jsonl",
                    prep.step_data.len(),
                    pool
                );
                prep.step_data
            } else {
                if use_cross_pair {
                    if let Some(h) = hours {
                        println!(
                            "đź”Ť Fetching historical data for {}/{} ({} hours)...",
                            symbol_a,
                            symbol_b.as_deref().unwrap_or("UNKNOWN"),
                            h
                        );
                    } else {
                        println!(
                            "đź”Ť Fetching historical data for {}/{} ({} days)...",
                            symbol_a,
                            symbol_b.as_deref().unwrap_or("UNKNOWN"),
                            days
                        );
                    }
                } else if let Some(h) = hours {
                    println!(
                        "đź”Ť Fetching historical data for {}/USDC ({} hours)...",
                        symbol_a, h
                    );
                } else {
                    println!(
                        "đź”Ť Fetching historical data for {}/USDC ({} days)...",
                        symbol_a, days
                    );
                }

                let api_key = env::var("BIRDEYE_API_KEY")
                    .expect("BIRDEYE_API_KEY must be set in .env or environment");
                let provider = BirdeyeProvider::new(api_key);

                candles = if use_cross_pair {
                    provider
                        .get_cross_pair_price_history(
                            &token_a,
                            &token_b,
                            start_time,
                            end_time,
                            *resolution_seconds,
                        )
                        .await?
                } else {
                    provider
                        .get_price_history(
                            &token_a,
                            &token_b,
                            start_time,
                            end_time,
                            *resolution_seconds,
                        )
                        .await?
                };

                if candles.is_empty() {
                    println!("âťŚ No data found for the specified period.");
                    return Ok(());
                }

                let quote_usd_map: Option<HashMap<u64, Decimal>> = if use_cross_pair {
                    let usdc = Token::new(
                        "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
                        "USDC",
                        6,
                        "USD Coin",
                    );
                    let quote_candles = provider
                        .get_price_history(
                            &token_b,
                            &usdc,
                            start_time,
                            end_time,
                            *resolution_seconds,
                        )
                        .await?;
                    if quote_candles.is_empty() {
                        None
                    } else {
                        Some(
                            quote_candles
                                .into_iter()
                                .map(|c| (c.start_timestamp, c.close.value))
                                .collect(),
                        )
                    }
                } else {
                    None
                };

                let steps_per_day =
                    Decimal::from_f64(86_400f64 / (*resolution_seconds).max(1) as f64).unwrap();
                let (sd, _e, _c) = crate::backtest_engine::build_step_data(
                    &candles,
                    dune_daily_tvl.as_ref(),
                    dune_daily_volume.as_ref(),
                    quote_usd_map.as_ref(),
                    capital_dec,
                    lp_share_override,
                    steps_per_day,
                );
                sd
            };

            // If snapshot pool data is provided and user didn't force a fixed lp share,
            // update per-step `lp_share` using a TVL proxy from on-chain vault balances.
            // This keeps candle/swaps fee logic intact, but makes `share(t)` more realistic.
            if lp_share_override.is_none()
                && snapshot_protocol.is_some()
                && snapshot_pool_address.is_some()
            {
                let proto = snapshot_protocol.unwrap();
                let pool_addr = snapshot_pool_address.clone().unwrap();
                let base_dir =
                    std::path::Path::new("data")
                        .join("pool-snapshots")
                        .join(match proto {
                            SnapshotProtocolArg::Orca => "orca",
                            SnapshotProtocolArg::Raydium => "raydium",
                            SnapshotProtocolArg::Meteora => "meteora",
                        });
                let snap_path = base_dir.join(&pool_addr).join("snapshots.jsonl");
                if snap_path.exists() {
                    let txt = std::fs::read_to_string(&snap_path)?;
                    let mut snaps: Vec<(i64, u64, u64, Option<String>, Option<String>)> =
                        Vec::new();
                    for line in txt.lines().filter(|l| !l.trim().is_empty()) {
                        let v: serde_json::Value = match serde_json::from_str(line) {
                            Ok(v) => v,
                            Err(_) => continue,
                        };
                        let ts = v
                            .get("ts_utc")
                            .and_then(|x| x.as_str())
                            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                            .map(|dt| dt.timestamp());
                        let a = v.get("vault_amount_a").and_then(|x| x.as_u64());
                        let b = v.get("vault_amount_b").and_then(|x| x.as_u64());
                        let ma = v
                            .get("token_mint_a")
                            .and_then(|x| x.as_str())
                            .map(|s| s.to_string());
                        let mb = v
                            .get("token_mint_b")
                            .and_then(|x| x.as_str())
                            .map(|s| s.to_string());
                        if let (Some(ts), Some(a), Some(b)) = (ts, a, b) {
                            snaps.push((ts, a, b, ma, mb));
                        }
                    }
                    snaps.sort_by_key(|(t, _, _, _, _)| *t);
                    if snaps.len() >= 1 {
                        let pow10 = |d: u32| -> Decimal {
                            let mut v = Decimal::ONE;
                            for _ in 0..d {
                                v *= Decimal::from(10u32);
                            }
                            v
                        };
                        let mut j = 0usize;
                        let mut last = snaps[0].clone();
                        for p in step_data.iter_mut() {
                            let t = p.start_timestamp as i64;
                            while j + 1 < snaps.len() && snaps[j + 1].0 <= t {
                                j += 1;
                                last = snaps[j].clone();
                            }
                            let token_usd = |raw: u64, mint: Option<&str>| -> Option<Decimal> {
                                let mint = mint?;
                                if mint.eq_ignore_ascii_case(&token_a.mint_address) {
                                    let amt = Decimal::from(raw) / pow10(token_a_decimals as u32);
                                    Some(amt * p.price_usd.value)
                                } else if mint.eq_ignore_ascii_case(&token_b.mint_address) {
                                    let amt = Decimal::from(raw) / pow10(token_b_decimals as u32);
                                    Some(amt * p.quote_usd)
                                } else {
                                    None
                                }
                            };
                            let tvl_usd = token_usd(last.1, last.3.as_deref())
                                .unwrap_or(Decimal::ZERO)
                                + token_usd(last.2, last.4.as_deref()).unwrap_or(Decimal::ZERO);
                            if tvl_usd > Decimal::ZERO {
                                let mut share = capital_dec / tvl_usd;
                                if share < Decimal::ZERO {
                                    share = Decimal::ZERO;
                                }
                                if share > Decimal::ONE {
                                    share = Decimal::ONE;
                                }
                                p.lp_share = share;
                            }
                        }
                    } else {
                        println!(
                            "âš ď¸Ź No usable vault balances found in snapshots at {} (lp_share unchanged).",
                            snap_path.display()
                        );
                    }
                } else {
                    println!(
                        "âš ď¸Ź Snapshot file not found at {} (lp_share unchanged).",
                        snap_path.display()
                    );
                }
            }

            // Optional: fetch and filter swaps for this pool (swap-fees mode).
            let swaps: Option<Vec<clmm_lp_data::swaps::SwapEvent>> = if let Some(arg) =
                dune_swaps.as_ref()
            {
                match crate::commands::backtest_optimize::fetch_swaps_for_optimize(arg).await {
                    Ok(Some(s)) => {
                        let (_pool_l, _eff, _da, _db, vault_a, vault_b) =
                            if let Some(pool) = whirlpool_address.as_ref() {
                                crate::commands::backtest_optimize::fetch_pool_state(
                                    pool,
                                    token_a_decimals,
                                    token_b_decimals,
                                    use_cross_pair,
                                )
                                .await?
                            } else {
                                (None, None, token_a_decimals, token_b_decimals, None, None)
                            };
                        Some(crate::commands::backtest_optimize::filter_swaps_for_pool(
                            s,
                            vault_a.as_deref(),
                            vault_b.as_deref(),
                            &token_a.mint_address,
                            &token_b.mint_address,
                        ))
                    }
                    Ok(None) => None,
                    Err(e) => {
                        let msg = e.to_string();
                        if msg.contains("402")
                            || msg.contains("Payment Required")
                            || msg.contains("credits")
                        {
                            println!(
                                "âš ď¸Ź Dune API credit limit reached. Falling back to candle-based fees."
                            );
                        } else {
                            println!(
                                "âš ď¸Ź Could not fetch Dune swaps ({}). Falling back to candle-based fees.",
                                e
                            );
                        }
                        None
                    }
                }
            } else {
                None
            };
            if dune_swaps.is_none()
                && matches!(fee_source, FeeSourceArg::Swaps | FeeSourceArg::Auto)
            {
                println!("â„ąď¸Ź --dune-swaps not set: using local swaps cache when available.");
            }

            // Optional: read local raw swap stream (P1 MVP) to build tx-count timing per step.
            let raw_swap_counts_by_step: Option<std::collections::BTreeMap<usize, u64>> = {
                let proto = snapshot_protocol.as_ref();
                let pool = snapshot_pool_address
                    .as_ref()
                    .or(whirlpool_address.as_ref());
                if proto.is_none() || pool.is_none() || step_data.is_empty() {
                    None
                } else {
                    let pdir = match proto.unwrap() {
                        SnapshotProtocolArg::Orca => "orca",
                        SnapshotProtocolArg::Raydium => "raydium",
                        SnapshotProtocolArg::Meteora => "meteora",
                    };
                    let path = std::path::Path::new("data")
                        .join("swaps")
                        .join(pdir)
                        .join(pool.unwrap())
                        .join("swaps.jsonl");
                    if !path.exists() {
                        None
                    } else {
                        let txt = match std::fs::read_to_string(&path) {
                            Ok(t) => t,
                            Err(_) => String::new(),
                        };
                        if txt.is_empty() {
                            None
                        } else {
                            let step_seconds = (*resolution_seconds).max(1) as i64;
                            let start_ts = step_data[0].start_timestamp as i64;
                            let mut counts: std::collections::BTreeMap<usize, u64> =
                                std::collections::BTreeMap::new();
                            for line in txt.lines().filter(|l| !l.trim().is_empty()) {
                                let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                                    continue;
                                };
                                // Skip failed txs
                                if v.get("err")
                                    .and_then(|x| x.as_str())
                                    .map(|s| !s.trim().is_empty() && s != "None")
                                    .unwrap_or(false)
                                {
                                    continue;
                                }
                                let ts = v.get("block_time").and_then(|x| x.as_i64());
                                let Some(ts) = ts else {
                                    continue;
                                };
                                let delta = ts - start_ts;
                                if delta < 0 {
                                    continue;
                                }
                                let idx = (delta / step_seconds) as usize;
                                *counts.entry(idx).or_insert(0) += 1;
                            }
                            if counts.is_empty() {
                                None
                            } else {
                                Some(counts)
                            }
                        }
                    }
                }
            };

            // Optional: build snapshot fee index by step (snapshot-fees mode).
            use std::collections::BTreeMap;
            let use_prebuilt_snap_fees = snapshots_only
                && (matches!(fee_source, FeeSourceArg::Snapshots)
                    || matches!(fee_source, FeeSourceArg::Auto));
            let mut snapshot_fee_index: Option<BTreeMap<usize, Decimal>> = if use_prebuilt_snap_fees
            {
                prebuilt_snapshot_fee_index.take()
            } else {
                None
            };

            if snapshot_fee_index.is_none() {
                snapshot_fee_index = {
                    let want_snapshots = matches!(fee_source, FeeSourceArg::Snapshots)
                        || (matches!(fee_source, FeeSourceArg::Auto)
                            && snapshot_protocol.is_some()
                            && snapshot_pool_address.is_some());
                    if !want_snapshots {
                        None
                    } else {
                        let proto = snapshot_protocol.ok_or_else(|| {
                            anyhow::anyhow!(
                                "--snapshot-protocol is required when using --fee-source snapshots"
                            )
                        })?;
                        let pool_addr = snapshot_pool_address.clone().ok_or_else(|| {
                        anyhow::anyhow!("--snapshot-pool-address is required when using --fee-source snapshots")
                    })?;

                        let base_dir =
                            std::path::Path::new("data")
                                .join("pool-snapshots")
                                .join(match proto {
                                    SnapshotProtocolArg::Orca => "orca",
                                    SnapshotProtocolArg::Raydium => "raydium",
                                    SnapshotProtocolArg::Meteora => "meteora",
                                });
                        let snap_path = base_dir.join(&pool_addr).join("snapshots.jsonl");
                        if !snap_path.exists() {
                            println!(
                                "âš ď¸Ź Snapshot file not found at {}. Falling back to candle/swap fees.",
                                snap_path.display()
                            );
                            None
                        } else {
                            let txt = std::fs::read_to_string(&snap_path)?;

                            #[derive(Clone)]
                            struct SnapshotFeePoint {
                                ts: i64,
                                protocol_fee_a_raw: u128,
                                protocol_fee_b_raw: u128,
                                fee_growth_a_raw: Option<u128>,
                                fee_growth_b_raw: Option<u128>,
                                liquidity_active_raw: Option<u128>,
                                mint_a: Option<String>,
                                mint_b: Option<String>,
                                dec_a: Option<u8>,
                                dec_b: Option<u8>,
                            }

                            let val_to_u128 = |x: Option<&serde_json::Value>| -> Option<u128> {
                                match x {
                                    Some(v) if v.is_u64() => v.as_u64().map(|n| n as u128),
                                    Some(v) if v.is_string() => {
                                        v.as_str().and_then(|s| s.trim().parse::<u128>().ok())
                                    }
                                    _ => None,
                                }
                            };

                            let mut pts: Vec<SnapshotFeePoint> = Vec::new();
                            for line in txt.lines().filter(|l| !l.trim().is_empty()) {
                                let v: serde_json::Value = match serde_json::from_str(line) {
                                    Ok(v) => v,
                                    Err(_) => continue,
                                };
                                let ts = v
                                    .get("ts_utc")
                                    .and_then(|x| x.as_str())
                                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                                    .map(|dt| dt.timestamp());
                                let Some(ts) = ts else {
                                    continue;
                                };

                                let (
                                    protocol_fee_a_raw,
                                    protocol_fee_b_raw,
                                    fee_growth_a_raw,
                                    fee_growth_b_raw,
                                    liquidity_active_raw,
                                ) = match proto {
                                    SnapshotProtocolArg::Orca => (
                                        val_to_u128(v.get("protocol_fee_owed_a")).unwrap_or(0),
                                        val_to_u128(v.get("protocol_fee_owed_b")).unwrap_or(0),
                                        val_to_u128(v.get("fee_growth_global_a")),
                                        val_to_u128(v.get("fee_growth_global_b")),
                                        val_to_u128(v.get("liquidity_active")),
                                    ),
                                    SnapshotProtocolArg::Raydium => (
                                        val_to_u128(v.get("protocol_fees_token_a")).unwrap_or(0),
                                        val_to_u128(v.get("protocol_fees_token_b")).unwrap_or(0),
                                        val_to_u128(v.get("fee_growth_global_a_x64")),
                                        val_to_u128(v.get("fee_growth_global_b_x64")),
                                        val_to_u128(v.get("liquidity_active")),
                                    ),
                                    SnapshotProtocolArg::Meteora => (
                                        val_to_u128(v.get("protocol_fee_amount_a")).unwrap_or(0),
                                        val_to_u128(v.get("protocol_fee_amount_b")).unwrap_or(0),
                                        None,
                                        None,
                                        None,
                                    ),
                                };

                                pts.push(SnapshotFeePoint {
                                    ts,
                                    protocol_fee_a_raw,
                                    protocol_fee_b_raw,
                                    fee_growth_a_raw,
                                    fee_growth_b_raw,
                                    liquidity_active_raw,
                                    mint_a: v
                                        .get("token_mint_a")
                                        .and_then(|x| x.as_str())
                                        .map(|s| s.to_string()),
                                    mint_b: v
                                        .get("token_mint_b")
                                        .and_then(|x| x.as_str())
                                        .map(|s| s.to_string()),
                                    dec_a: v
                                        .get("mint_decimals_a")
                                        .and_then(|x| x.as_u64())
                                        .and_then(|n| u8::try_from(n).ok()),
                                    dec_b: v
                                        .get("mint_decimals_b")
                                        .and_then(|x| x.as_u64())
                                        .and_then(|n| u8::try_from(n).ok()),
                                });
                            }

                            if pts.len() < 2 {
                                println!(
                                    "âš ď¸Ź Snapshot fee fields are missing or insufficient in {} for {:?}. Falling back to candle/swap fees.",
                                    snap_path.display(),
                                    proto
                                );
                                None
                            } else {
                                let mut decimals_by_mint: HashMap<String, u32> = HashMap::new();
                                decimals_by_mint
                                    .insert(token_a.mint_address.clone(), token_a_decimals as u32);
                                decimals_by_mint
                                    .insert(token_b.mint_address.clone(), token_b_decimals as u32);
                                for p in &pts {
                                    if let (Some(m), Some(d)) = (&p.mint_a, p.dec_a) {
                                        decimals_by_mint.entry(m.clone()).or_insert(d as u32);
                                    }
                                    if let (Some(m), Some(d)) = (&p.mint_b, p.dec_b) {
                                        decimals_by_mint.entry(m.clone()).or_insert(d as u32);
                                    }
                                }

                                // Backfill missing decimals from RPC once (for protocols where snapshot doesn't include them).
                                let mut unresolved: Vec<String> = Vec::new();
                                for p in &pts {
                                    if let Some(m) = &p.mint_a {
                                        if !decimals_by_mint.contains_key(m) {
                                            unresolved.push(m.clone());
                                        }
                                    }
                                    if let Some(m) = &p.mint_b {
                                        if !decimals_by_mint.contains_key(m) {
                                            unresolved.push(m.clone());
                                        }
                                    }
                                }
                                unresolved.sort();
                                unresolved.dedup();
                                if !unresolved.is_empty() {
                                    use crate::engine::token_meta::fetch_mint_decimals;
                                    use clmm_lp_protocols::rpc::RpcProvider;
                                    let rpc_dec = RpcProvider::mainnet();
                                    for m in unresolved {
                                        if let Ok(d) = fetch_mint_decimals(&rpc_dec, &m).await {
                                            decimals_by_mint.insert(m, d as u32);
                                        }
                                    }
                                }

                                let pow10 = |d: u32| -> Decimal {
                                    let mut v = Decimal::ONE;
                                    for _ in 0..d {
                                        v *= Decimal::from(10u32);
                                    }
                                    v
                                };
                                let usd_for_mint =
                                    |mint: &str,
                                     p: &crate::backtest_engine::StepDataPoint|
                                     -> Option<Decimal> {
                                        if mint.eq_ignore_ascii_case(&token_a.mint_address) {
                                            Some(p.price_usd.value)
                                        } else if mint.eq_ignore_ascii_case(&token_b.mint_address) {
                                            Some(p.quote_usd)
                                        } else {
                                            None
                                        }
                                    };

                                pts.sort_by_key(|p| p.ts);
                                let start_ts = step_data
                                    .first()
                                    .map(|p| p.start_timestamp as i64)
                                    .unwrap_or(0);
                                let step_seconds = *resolution_seconds as i64;
                                let mut map: BTreeMap<usize, Decimal> = BTreeMap::new();
                                let q64: U256 = U256::from(1u128) << 64;
                                let delta_from_growth =
                                    |g0: Option<u128>,
                                     g1: Option<u128>,
                                     liq: Option<u128>|
                                     -> Option<u128> {
                                        let (Some(g0), Some(g1), Some(liq)) = (g0, g1, liq) else {
                                            return None;
                                        };
                                        if g1 <= g0 || liq == 0 {
                                            return Some(0);
                                        }
                                        let dg = g1 - g0;
                                        let prod = U256::from(dg).saturating_mul(U256::from(liq));
                                        let raw = prod / q64;
                                        Some(raw.low_u128())
                                    };

                                // Convert deltas of protocol fee accumulators into per-step USD fee proxy.
                                for w in pts.windows(2) {
                                    let p0 = &w[0];
                                    let p1 = &w[1];
                                    if p1.ts <= p0.ts {
                                        continue;
                                    }

                                    // Prefer fee-growth based deltas (Orca/Raydium) for smoother/realistic flow.
                                    // Fallback to protocol-fee counters (all protocols, incl. Meteora).
                                    let dv_a = delta_from_growth(
                                        p0.fee_growth_a_raw,
                                        p1.fee_growth_a_raw,
                                        p1.liquidity_active_raw.or(p0.liquidity_active_raw),
                                    )
                                    .unwrap_or_else(|| {
                                        p1.protocol_fee_a_raw.saturating_sub(p0.protocol_fee_a_raw)
                                    });
                                    let dv_b = delta_from_growth(
                                        p0.fee_growth_b_raw,
                                        p1.fee_growth_b_raw,
                                        p1.liquidity_active_raw.or(p0.liquidity_active_raw),
                                    )
                                    .unwrap_or_else(|| {
                                        p1.protocol_fee_b_raw.saturating_sub(p0.protocol_fee_b_raw)
                                    });
                                    if dv_a == 0 && dv_b == 0 {
                                        continue;
                                    }

                                    let mid = (p0.ts + p1.ts) / 2;
                                    let delta = mid - start_ts;
                                    if delta < 0 {
                                        continue;
                                    }
                                    let idx = (delta / step_seconds.max(1)) as usize;
                                    let Some(step) = step_data.get(idx) else {
                                        continue;
                                    };

                                    let mut usd = Decimal::ZERO;
                                    if let Some(mint) = p1.mint_a.as_deref() {
                                        if let (Some(dec), Some(px)) =
                                            (decimals_by_mint.get(mint), usd_for_mint(mint, step))
                                        {
                                            let amt = Decimal::from_u128(dv_a)
                                                .unwrap_or(Decimal::ZERO)
                                                / pow10(*dec);
                                            usd += amt * px;
                                        }
                                    }
                                    if let Some(mint) = p1.mint_b.as_deref() {
                                        if let (Some(dec), Some(px)) =
                                            (decimals_by_mint.get(mint), usd_for_mint(mint, step))
                                        {
                                            let amt = Decimal::from_u128(dv_b)
                                                .unwrap_or(Decimal::ZERO)
                                                / pow10(*dec);
                                            usd += amt * px;
                                        }
                                    }

                                    if usd > Decimal::ZERO {
                                        *map.entry(idx).or_insert(Decimal::ZERO) += usd;
                                    }
                                }

                                if map.is_empty() {
                                    println!(
                                        "âš ď¸Ź Snapshot fee deltas found in {}, but could not convert to USD for this pair. Falling back to candle/swap fees.",
                                        snap_path.display()
                                    );
                                    None
                                } else {
                                    Some(map)
                                }
                            }
                        }
                    }
                };
            }

            // Index swap fees by step from Dune data (when provided).
            let mut swap_index: Option<BTreeMap<usize, Decimal>> = swaps.as_ref().map(|swaps| {
                let mut map: BTreeMap<usize, Decimal> = BTreeMap::new();
                if step_data.is_empty() {
                    return map;
                }
                let step_seconds = *resolution_seconds as i64;
                let start_ts = step_data[0].start_timestamp as i64;
                for s in swaps {
                    if let Some(dt) = s.block_time_utc() {
                        let delta = dt.timestamp() - start_ts;
                        if delta >= 0 {
                            let idx = (delta / step_seconds.max(1)) as usize;
                            let f = if s.fee_usd != Decimal::ZERO {
                                s.fee_usd
                            } else {
                                s.amount_usd * s.fee_tier
                            };
                            *map.entry(idx).or_insert(Decimal::ZERO) += f;
                        }
                    }
                }
                map
            });
            let mut local_decoded_swap_index: Option<BTreeMap<usize, Decimal>> = None;

            // Prepare Price Path (A/B close prices)
            let prices: Vec<Price> = if candles.is_empty() {
                step_data.iter().map(|s| s.price_ab).collect()
            } else {
                candles.iter().map(|c| c.close).collect()
            };
            let entry_price = prices.first().cloned().unwrap_or(Price::new(Decimal::ONE));
            let final_price = prices.last().cloned().unwrap_or(entry_price);

            // Setup position tracker
            let initial_range = PriceRange::new(
                Price::new(Decimal::from_f64(*lower).unwrap()),
                Price::new(Decimal::from_f64(*upper).unwrap()),
            );
            let tx_cost_dec = Decimal::from_f64(*tx_cost).unwrap();
            let realistic_fixed_cost_dec =
                Decimal::from_f64(*tx_cost + *network_fee_usd + *priority_fee_usd + *jito_tip_usd)
                    .unwrap();
            let realistic_slippage_bps_dec = Decimal::from_f64(*slippage_bps).unwrap();

            let mut tracker =
                PositionTracker::new(capital_dec, entry_price, initial_range, tx_cost_dec);

            // Fee rate: prefer on-chain effective fee rate when pool is provided.
            // Fall back to 0.30% when we don't know the pool.
            let (pool_active_liquidity, fee_rate) = if let Some(pool) = whirlpool_address
                .as_ref()
                .or(snapshot_pool_address.as_ref())
            {
                let (pool_l, eff, _da, _db, _va, _vb) =
                    crate::commands::backtest_optimize::fetch_pool_state(
                        pool,
                        token_a_decimals,
                        token_b_decimals,
                        use_cross_pair,
                    )
                    .await?;
                (
                    pool_l,
                    eff.unwrap_or_else(|| Decimal::from_f64(0.003).unwrap()),
                )
            } else {
                (None, Decimal::from_f64(0.003).unwrap())
            };

            // Local decoded swaps (P1.1): prefer this when Dune swaps are not provided.
            {
                let proto = snapshot_protocol.as_ref();
                let pool = snapshot_pool_address
                    .as_ref()
                    .or(whirlpool_address.as_ref());
                if proto.is_some() && pool.is_some() && !step_data.is_empty() {
                    let pdir = match proto.unwrap() {
                        SnapshotProtocolArg::Orca => "orca",
                        SnapshotProtocolArg::Raydium => "raydium",
                        SnapshotProtocolArg::Meteora => "meteora",
                    };
                    let path = std::path::Path::new("data")
                        .join("swaps")
                        .join(pdir)
                        .join(pool.unwrap())
                        .join("decoded_swaps.jsonl");
                    if path.exists() {
                        let txt = std::fs::read_to_string(&path).unwrap_or_default();
                        if !txt.trim().is_empty() {
                            let step_seconds = (*resolution_seconds).max(1) as i64;
                            let start_ts = step_data[0].start_timestamp as i64;
                            let pow10 = |d: u32| -> Decimal {
                                let mut v = Decimal::ONE;
                                for _ in 0..d {
                                    v *= Decimal::from(10u32);
                                }
                                v
                            };
                            let mut map: BTreeMap<usize, Decimal> = BTreeMap::new();
                            for line in txt.lines().filter(|l| !l.trim().is_empty()) {
                                let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                                    continue;
                                };
                                if matches!(*fee_swap_decode_status, FeeSwapDecodeStatusArg::Ok) {
                                    let st = v.get("decode_status").and_then(|x| x.as_str());
                                    if !matches!(
                                        st,
                                        Some("ok")
                                            | Some("ok_traded_event")
                                            | Some("ok_swap_event")
                                    ) {
                                        continue;
                                    }
                                }
                                if !v.get("success").and_then(|x| x.as_bool()).unwrap_or(false) {
                                    continue;
                                }
                                let Some(ts) = v.get("block_time").and_then(|x| x.as_i64()) else {
                                    continue;
                                };
                                let delta = ts - start_ts;
                                if delta < 0 {
                                    continue;
                                }
                                let idx = (delta / step_seconds) as usize;
                                let Some(step) = step_data.get(idx) else {
                                    continue;
                                };
                                let amount_in_raw = v
                                    .get("amount_in_raw")
                                    .and_then(|x| x.as_u64())
                                    .map(Decimal::from)
                                    .or_else(|| {
                                        v.get("amount_in_raw")
                                            .and_then(|x| x.as_str())
                                            .and_then(|s| s.parse::<u128>().ok())
                                            .and_then(Decimal::from_u128)
                                    });
                                let Some(amount_in_raw) = amount_in_raw else {
                                    continue;
                                };
                                let direction =
                                    v.get("direction").and_then(|x| x.as_str()).unwrap_or("");
                                let input_is_a = direction == "a_to_b";
                                let (decimals_in, price_in_usd) = if input_is_a {
                                    (token_a_decimals as u32, step.price_usd.value)
                                } else {
                                    (token_b_decimals as u32, step.quote_usd)
                                };
                                let amount_in_h = amount_in_raw / pow10(decimals_in);
                                let fee_usd = amount_in_h * price_in_usd * fee_rate;
                                if fee_usd > Decimal::ZERO {
                                    *map.entry(idx).or_insert(Decimal::ZERO) += fee_usd;
                                }
                            }
                            if !map.is_empty() {
                                local_decoded_swap_index = Some(map);
                            }
                        }
                    }
                }
            }

            // P1.2: if no decoded Dune swaps are available, use local raw swap tx timing
            // to distribute total pool fees across steps (swaps-timing proxy).
            if swap_index.as_ref().map(|m| m.is_empty()).unwrap_or(true) {
                if let Some(local_map) = local_decoded_swap_index.as_ref() {
                    // If decoded swaps only cover a few step buckets, hybridize:
                    // - use decoded swaps on buckets where we have them,
                    // - fill missing buckets with raw tx-count timing proxy.
                    if let Some(counts) = raw_swap_counts_by_step.as_ref() {
                        let total_count: u64 = counts.values().copied().sum();
                        if total_count > 0 {
                            let total_pool_fees: Decimal =
                                step_data.iter().map(|p| p.step_volume_usd * fee_rate).sum();
                            if total_pool_fees > Decimal::ZERO {
                                let mut map: BTreeMap<usize, Decimal> = BTreeMap::new();
                                let denom = Decimal::from(total_count);
                                for (idx, c) in counts {
                                    if *c == 0 {
                                        continue;
                                    }
                                    let w = Decimal::from(*c) / denom;
                                    let v = total_pool_fees * w;
                                    if v > Decimal::ZERO {
                                        map.insert(*idx, v);
                                    }
                                }
                                // Overwrite buckets we have decoded values for.
                                for (idx, v) in local_map {
                                    map.insert(*idx, v.clone());
                                }
                                if !map.is_empty() {
                                    println!(
                                        "đź§Ş Using local decoded swaps + timing proxy (decoded={} merged={})",
                                        local_map.len(),
                                        map.len()
                                    );
                                    swap_index = Some(map);
                                } else {
                                    swap_index = Some(local_map.clone());
                                }
                            } else {
                                swap_index = Some(local_map.clone());
                            }
                        } else {
                            swap_index = Some(local_map.clone());
                        }
                    } else {
                        println!(
                            "đź§Ş Using local decoded swaps from data/swaps ({} steps).",
                            local_map.len()
                        );
                        swap_index = Some(local_map.clone());
                    }
                }
            }
            if swap_index.as_ref().map(|m| m.is_empty()).unwrap_or(true) {
                if let Some(counts) = raw_swap_counts_by_step.as_ref() {
                    let total_count: u64 = counts.values().copied().sum();
                    if total_count > 0 {
                        let total_pool_fees: Decimal =
                            step_data.iter().map(|p| p.step_volume_usd * fee_rate).sum();
                        if total_pool_fees > Decimal::ZERO {
                            let mut map: BTreeMap<usize, Decimal> = BTreeMap::new();
                            let denom = Decimal::from(total_count);
                            for (idx, c) in counts {
                                if *c == 0 {
                                    continue;
                                }
                                let w = Decimal::from(*c) / denom;
                                let v = total_pool_fees * w;
                                if v > Decimal::ZERO {
                                    map.insert(*idx, v);
                                }
                            }
                            if !map.is_empty() {
                                println!(
                                    "đź§Ş Using local raw swaps timing proxy from data/swaps ({} steps).",
                                    map.len()
                                );
                                swap_index = Some(map);
                            }
                        }
                    }
                }
            }

            // Guardrail: snapshot-fee model is experimental. If implied pool fees are
            // unrealistically high vs candle-based pool fee baseline, disable it and fallback.
            if !snapshots_only {
                if let Some(idx_map) = snapshot_fee_index.as_ref() {
                    let snapshot_pool_fees: Decimal = idx_map.values().cloned().sum();
                    let candle_pool_fees: Decimal =
                        step_data.iter().map(|p| p.step_volume_usd * fee_rate).sum();
                    if candle_pool_fees > Decimal::ZERO {
                        let ratio = snapshot_pool_fees / candle_pool_fees;
                        let max_ratio = Decimal::from(10u32);
                        if ratio > max_ratio {
                            println!(
                                "âš ď¸Ź Snapshot fee sanity check failed: snapshot pool fees {:.2} vs candle baseline {:.2} (ratio {:.2}x > {:.2}x). Falling back from snapshot fees.",
                                snapshot_pool_fees, candle_pool_fees, ratio, max_ratio
                            );
                            snapshot_fee_index = None;
                        }
                    }
                }
            }

            println!(
                "đźš€ Running backtest ({}, res={}s, swaps={}) with {:?} strategy over {} steps...",
                display_label,
                *resolution_seconds,
                dune_swaps.as_deref().unwrap_or("none"),
                strategy,
                prices.len()
            );

            // Run simulation with strategy
            let range_width_pct =
                Decimal::from_f64((*upper - *lower) / ((*upper + *lower) / 2.0)).unwrap();
            let mut fee_steps_snapshots: usize = 0;
            let mut fee_steps_swaps: usize = 0;
            let mut fee_steps_candles: usize = 0;
            let mut fee_steps_zero: usize = 0;
            let mut fee_total_candles_lp = Decimal::ZERO;
            let mut fee_total_swaps_lp = Decimal::ZERO;
            let mut fee_total_snapshots_lp = Decimal::ZERO;
            let mut fee_cmp_steps = 0usize;
            let range_share_k_dec = Decimal::from_f64(*range_share_k).unwrap_or(Decimal::ONE);
            let range_share_cap_mult_dec = Decimal::from_f64(*range_share_cap_mult)
                .unwrap_or(Decimal::from(3u32))
                .max(Decimal::ONE);

            for (idx, price) in prices.iter().enumerate() {
                let p =
                    step_data
                        .get(idx)
                        .copied()
                        .unwrap_or(crate::backtest_engine::StepDataPoint {
                            price_usd: *price,
                            price_ab: *price,
                            step_volume_usd: Decimal::ZERO,
                            quote_usd: Decimal::ONE,
                            lp_share: lp_share_override
                                .unwrap_or_else(|| Decimal::from_f64(0.01).unwrap()),
                            pool_liquidity_active: None,
                            start_timestamp: step_data
                                .get(idx)
                                .map(|p| p.start_timestamp)
                                .or_else(|| candles.get(idx).map(|c| c.start_timestamp))
                                .unwrap_or(0),
                        });

                // Calculate fees for this step
                let in_range = price.value >= tracker.current_range.lower_price.value
                    && price.value <= tracker.current_range.upper_price.value;

                let step_lp_share = if let Some(share) = lp_share_override {
                    share
                } else if p.pool_liquidity_active.is_some() || pool_active_liquidity.is_some() {
                    // Range-aware LP share: narrower/wider ranges change position liquidity,
                    // which changes effective fee share even when TIR is 100%.
                    let lower_usd = tracker.current_range.lower_price.value * p.quote_usd;
                    let upper_usd = tracker.current_range.upper_price.value * p.quote_usd;
                    let capital_now = tracker
                        .snapshots
                        .last()
                        .map(|s| s.position_value_usd.max(Decimal::ZERO))
                        .unwrap_or(capital_dec);
                    let pos_l =
                        crate::engine::liquidity::estimate_position_liquidity_with_overrides(
                            &step_data,
                            lower_usd,
                            upper_usd,
                            capital_now,
                            token_a_decimals as u32,
                            token_b_decimals as u32,
                            crate::engine::liquidity::LiquidityEstimateOverrides {
                                quote_usd: Some(p.quote_usd),
                                price_ab: Some(p.price_ab.value),
                                price_a_usd: Some(p.price_usd.value),
                            },
                        );
                    let pos_l_dec = Decimal::from_u128(pos_l).unwrap_or(Decimal::ZERO);
                    let pool_l_dec_opt = p
                        .pool_liquidity_active
                        .filter(|v| *v > 0)
                        .or(pool_active_liquidity.filter(|v| *v > 0));
                    let pool_l_dec = pool_l_dec_opt
                        .map(|v| Decimal::from_u128(v).unwrap_or(Decimal::ONE))
                        .unwrap_or(Decimal::ONE);
                    if pool_l_dec > Decimal::ZERO {
                        let est = (pos_l_dec / pool_l_dec).clamp(Decimal::ZERO, Decimal::ONE);
                        let upper_cap = (p.lp_share * range_share_cap_mult_dec)
                            .clamp(Decimal::ZERO, Decimal::ONE);
                        (est * range_share_k_dec).clamp(Decimal::ZERO, upper_cap)
                    } else {
                        p.lp_share
                    }
                } else {
                    p.lp_share
                };

                let step_fees = if in_range {
                    fee_cmp_steps += 1;
                    let candles_lp = p.step_volume_usd * fee_rate * step_lp_share;
                    let swaps_lp = swap_index
                        .as_ref()
                        .and_then(|idx_map| idx_map.get(&idx).cloned())
                        .map(|v| v * step_lp_share)
                        .unwrap_or(Decimal::ZERO);
                    let snapshots_lp = snapshot_fee_index
                        .as_ref()
                        .and_then(|idx_map| idx_map.get(&idx).cloned())
                        .map(|v| v * step_lp_share)
                        .unwrap_or(Decimal::ZERO);
                    fee_total_candles_lp += candles_lp;
                    fee_total_swaps_lp += swaps_lp;
                    fee_total_snapshots_lp += snapshots_lp;
                    match fee_source {
                        FeeSourceArg::Candles => {
                            fee_steps_candles += 1;
                            p.step_volume_usd * fee_rate * step_lp_share
                        }
                        FeeSourceArg::Swaps => {
                            if let Some(v) = swap_index
                                .as_ref()
                                .and_then(|idx_map| idx_map.get(&idx).cloned())
                            {
                                fee_steps_swaps += 1;
                                v * step_lp_share
                            } else {
                                fee_steps_zero += 1;
                                Decimal::ZERO
                            }
                        }
                        FeeSourceArg::Snapshots => {
                            if let Some(v) = snapshot_fee_index
                                .as_ref()
                                .and_then(|idx_map| idx_map.get(&idx).cloned())
                            {
                                fee_steps_snapshots += 1;
                                v * step_lp_share
                            } else if let Some(ref idx_map) = swap_index {
                                if let Some(v) = idx_map.get(&idx).cloned() {
                                    fee_steps_swaps += 1;
                                    v * step_lp_share
                                } else {
                                    fee_steps_candles += 1;
                                    p.step_volume_usd * fee_rate * step_lp_share
                                }
                            } else {
                                fee_steps_candles += 1;
                                p.step_volume_usd * fee_rate * step_lp_share
                            }
                        }
                        FeeSourceArg::Auto => {
                            if let Some(ref idx_map) = swap_index {
                                if let Some(v) = idx_map.get(&idx).cloned() {
                                    fee_steps_swaps += 1;
                                    v * step_lp_share
                                } else if let Some(ref sidx) = snapshot_fee_index {
                                    if let Some(vs) = sidx.get(&idx).cloned() {
                                        fee_steps_snapshots += 1;
                                        vs * step_lp_share
                                    } else {
                                        fee_steps_candles += 1;
                                        p.step_volume_usd * fee_rate * step_lp_share
                                    }
                                } else {
                                    fee_steps_candles += 1;
                                    p.step_volume_usd * fee_rate * step_lp_share
                                }
                            } else if let Some(ref idx_map) = snapshot_fee_index {
                                if let Some(v) = idx_map.get(&idx).cloned() {
                                    fee_steps_snapshots += 1;
                                    v * step_lp_share
                                } else {
                                    fee_steps_candles += 1;
                                    p.step_volume_usd * fee_rate * step_lp_share
                                }
                            } else {
                                fee_steps_candles += 1;
                                p.step_volume_usd * fee_rate * step_lp_share
                            }
                        }
                    }
                } else {
                    Decimal::ZERO
                };

                // Apply strategy
                if *use_realistic_rebalance_cost {
                    // Dynamic (non-constant) rebalance cost per step:
                    // fixed infra cost + slippage% * current notional.
                    let notional_now = tracker
                        .snapshots
                        .last()
                        .map(|s| s.position_value_usd.max(Decimal::ZERO))
                        .unwrap_or(capital_dec);
                    let slippage_cost = if realistic_slippage_bps_dec > Decimal::ZERO {
                        notional_now * realistic_slippage_bps_dec / Decimal::from(10_000u32)
                    } else {
                        Decimal::ZERO
                    };
                    tracker.rebalance_cost = realistic_fixed_cost_dec + slippage_cost;
                } else {
                    tracker.rebalance_cost = tx_cost_dec;
                }

                match strategy {
                    StrategyArg::Static => {
                        let strat = StaticRange::new();
                        tracker.record_step(*price, step_fees, Some(&strat));
                    }
                    StrategyArg::Periodic => {
                        let strat = PeriodicRebalance::new(*rebalance_interval, range_width_pct);
                        tracker.record_step(*price, step_fees, Some(&strat));
                    }
                    StrategyArg::Threshold => {
                        let strat = ThresholdRebalance::new(
                            Decimal::from_f64(*threshold_pct).unwrap(),
                            range_width_pct,
                        );
                        tracker.record_step(*price, step_fees, Some(&strat));
                    }
                }
            }

            // Get summary
            let summary = tracker.summary();
            let total_steps = prices.len();
            println!(
                "đź§ľ Fee source breakdown (in-range steps): snapshots={} swaps={} candles={} zero={} | total steps={}",
                fee_steps_snapshots,
                fee_steps_swaps,
                fee_steps_candles,
                fee_steps_zero,
                total_steps
            );
            println!(
                "đź§Ş Fee compare totals (LP, in-range): candles=${:.4} swaps=${:.4} snapshots=${:.4} steps={}",
                fee_total_candles_lp, fee_total_swaps_lp, fee_total_snapshots_lp, fee_cmp_steps
            );
            {
                let out_dir = std::path::Path::new("data").join("reports");
                std::fs::create_dir_all(&out_dir)?;
                let ts = chrono::Utc::now().format("%Y%m%d_%H%M%S");
                let out = out_dir.join(format!("backtest_fee_compare_{}.json", ts));
                let payload = serde_json::json!({
                    "ts_utc": chrono::Utc::now().to_rfc3339(),
                    "pair": display_label,
                    "fee_source_selected": format!("{:?}", fee_source),
                    "in_range_steps": fee_cmp_steps,
                    "totals_lp_usd": {
                        "candles": fee_total_candles_lp,
                        "swaps": fee_total_swaps_lp,
                        "snapshots": fee_total_snapshots_lp
                    },
                    "step_breakdown": {
                        "snapshots_steps": fee_steps_snapshots,
                        "swaps_steps": fee_steps_swaps,
                        "candles_steps": fee_steps_candles,
                        "zero_steps": fee_steps_zero
                    }
                });
                std::fs::write(&out, serde_json::to_string_pretty(&payload)?)?;
                println!("đź“ť Fee compare report saved: {}", out.display());
            }

            // Determine pair label for reporting (supports optional quote token)
            let pair_label = if let Some(sb) = symbol_b {
                format!("{}/{}", symbol_a, sb)
            } else {
                format!("{}/USDC", symbol_a)
            };

            // Print rich report
            print_backtest_report(
                &pair_label,
                effective_days,
                *capital,
                entry_price.value,
                final_price.value,
                *lower,
                *upper,
                &summary,
                *strategy,
            );
        }
        Commands::BacktestOptimize {
            symbol_a,
            mint_a,
            symbol_b,
            mint_b,
            days,
            hours,
            start_date,
            end_date,
            capital,
            tx_cost,
            use_realistic_rebalance_cost,
            network_fee_usd,
            priority_fee_usd,
            jito_tip_usd,
            slippage_bps,
            range_share_k,
            range_share_cap_mult,
            whirlpool_address,
            lp_share,
            objective,
            range_steps,
            min_range_pct,
            max_range_pct,
            top_n,
            full_ranking,
            min_time_in_range,
            max_drawdown,
            alpha,
            static_only,
            windows,
            il_max_pct,
            il_close_pct,
            il_grace_steps,
            dune_swaps,
            fee_source,
            resolution_seconds,
            snapshot_protocol,
            snapshot_pool_address,
            fee_swap_decode_status,
            price_path_source,
            retouch_repeat_cooldown_secs,
            retouch_repeat_rearm_secs,
            retouch_repeat_extra_move_pct,
            retouch_repeat_off,
            optimize_result_json,
            optimize_result_json_copy_dir,
        } => {
            // TODO(E2.6): wire these calibration params into optimize path as well.
            let _ = (range_share_k, range_share_cap_mult);
            let (token_a_decimals_guess, token_b_decimals_guess): (u8, u8) = {
                use crate::engine::token_meta::fetch_mint_decimals;
                use clmm_lp_protocols::rpc::RpcProvider;
                let rpc = RpcProvider::mainnet();
                let da = fetch_mint_decimals(&rpc, mint_a).await.unwrap_or(9);
                let db = if let Some(mb) = mint_b.as_ref() {
                    fetch_mint_decimals(&rpc, mb).await.unwrap_or(9)
                } else {
                    6u8
                };
                (da, db)
            };
            let token_a = Token::new(mint_a, symbol_a, token_a_decimals_guess, symbol_a);
            let (token_b, use_cross_pair) =
                if let (Some(sb), Some(mb)) = (symbol_b.as_ref(), mint_b.as_ref()) {
                    (Token::new(mb, sb, token_b_decimals_guess, sb), true)
                } else {
                    (
                        Token::new(
                            "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
                            "USDC",
                            6,
                            "USD Coin",
                        ),
                        false,
                    )
                };

            let snapshots_only = matches!(price_path_source, PricePathSourceArg::Snapshots);
            if matches!(fee_source, FeeSourceArg::Snapshots) && !snapshots_only {
                anyhow::bail!(
                    "--fee-source snapshots in backtest-optimize requires --price-path-source snapshots"
                );
            }
            if snapshots_only {
                if !matches!(
                    snapshot_protocol,
                    Some(SnapshotProtocolArg::Orca)
                        | Some(SnapshotProtocolArg::Raydium)
                        | Some(SnapshotProtocolArg::Meteora)
                ) {
                    anyhow::bail!(
                        "--price-path-source snapshots requires --snapshot-protocol orca|raydium|meteora"
                    );
                }
                if snapshot_pool_address.is_none() {
                    anyhow::bail!("--price-path-source snapshots requires --snapshot-pool-address");
                }
                if !use_cross_pair {
                    anyhow::bail!(
                        "--price-path-source snapshots requires a cross pair (--symbol-b and --mint-b)"
                    );
                }
            }

            let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
            let windows_u64 = (*windows).max(1) as u64;
            let fetch_span_secs = if let Some(h) = hours {
                h.saturating_mul(3600).saturating_mul(windows_u64)
            } else {
                days.saturating_mul(24 * 3600).saturating_mul(windows_u64)
            };

            let (start_time, end_time, display_label) = if snapshots_only {
                if let (Some(sd), Some(ed)) = (start_date.as_deref(), end_date.as_deref()) {
                    use chrono::{NaiveDate, TimeZone, Utc};
                    let s = NaiveDate::parse_from_str(sd, "%Y-%m-%d")?;
                    let e = NaiveDate::parse_from_str(ed, "%Y-%m-%d")?;
                    let s_ts = Utc
                        .from_utc_datetime(&s.and_hms_opt(0, 0, 0).unwrap())
                        .timestamp() as u64;
                    let e_ts = Utc
                        .from_utc_datetime(&e.and_hms_opt(0, 0, 0).unwrap())
                        .timestamp() as u64;
                    (s_ts, e_ts, format!("{}..{}", sd, ed))
                } else {
                    // Same total span as Birdeye path: `hours` or `days` per rolling window, times `windows`.
                    let s_ts = now.saturating_sub(fetch_span_secs);
                    let label = if let Some(h) = hours {
                        format!("last {}h x {} window(s)", h, windows)
                    } else {
                        format!("last {}d x {} window(s)", days, windows)
                    };
                    (s_ts, now, label)
                }
            } else {
                let s_ts = now.saturating_sub(fetch_span_secs);
                let label = if let Some(h) = hours {
                    format!("last {}h", h)
                } else {
                    format!("last {}d", days)
                };
                (s_ts, now, label)
            };

            let pair_label = if use_cross_pair {
                format!("{}/{}", symbol_a, symbol_b.as_deref().unwrap_or("?"))
            } else {
                format!("{}/USDC", symbol_a)
            };

            let res = *resolution_seconds;
            let mut candles: Vec<clmm_lp_domain::entities::price_candle::PriceCandle> = Vec::new();
            let mut snapshot_step_data_full: Option<Vec<crate::backtest_engine::StepData>> = None;
            let mut snapshot_fee_index_full: Option<BTreeMap<usize, Decimal>> = None;

            let quote_usd_map: Option<HashMap<u64, Decimal>> = if snapshots_only {
                None
            } else {
                let api_key = env::var("BIRDEYE_API_KEY")
                    .expect("BIRDEYE_API_KEY must be set in .env or environment (or use --price-path-source snapshots)");
                let provider = BirdeyeProvider::new(api_key);
                if let Some(h) = hours {
                    println!(
                        "đź”Ť Fetching historical data for {} ({} hour(s), {} window(s))...",
                        pair_label, h, windows
                    );
                } else {
                    println!(
                        "đź”Ť Fetching historical data for {} ({} day(s), {} window(s))...",
                        pair_label, days, windows
                    );
                }
                candles = if use_cross_pair {
                    provider
                        .get_cross_pair_price_history(&token_a, &token_b, start_time, end_time, res)
                        .await?
                } else {
                    provider
                        .get_price_history(&token_a, &token_b, start_time, end_time, res)
                        .await?
                };
                if candles.is_empty() {
                    println!("âťŚ No data found for the specified period.");
                    return Ok(());
                }
                if use_cross_pair {
                    let usdc = Token::new(
                        "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
                        "USDC",
                        6,
                        "USD Coin",
                    );
                    let quote_candles = provider
                        .get_price_history(&token_b, &usdc, start_time, end_time, res)
                        .await?;
                    if quote_candles.is_empty() {
                        None
                    } else {
                        Some(
                            quote_candles
                                .into_iter()
                                .map(|c| (c.start_timestamp, c.close.value))
                                .collect(),
                        )
                    }
                } else {
                    None
                }
            };

            if snapshots_only {
                println!(
                    "đź”Ť backtest-optimize: price path from snapshots only (window {}): {}/{} â€” no Birdeye",
                    display_label,
                    symbol_a,
                    symbol_b.as_deref().unwrap_or("?")
                );
                let pool = snapshot_pool_address.as_ref().unwrap();
                let prep = match snapshot_protocol.unwrap() {
                    SnapshotProtocolArg::Orca => {
                        crate::commands::snapshot_price_path::build_from_orca_snapshots(
                            pool,
                            start_time as i64,
                            end_time as i64,
                            &token_a,
                            &token_b,
                            *capital,
                            *lp_share,
                        )
                        .await?
                    }
                    SnapshotProtocolArg::Raydium => {
                        crate::commands::snapshot_price_path::build_from_raydium_snapshots(
                            pool,
                            start_time as i64,
                            end_time as i64,
                            &token_a,
                            &token_b,
                            *capital,
                            *lp_share,
                        )
                        .await?
                    }
                    SnapshotProtocolArg::Meteora => {
                        crate::commands::snapshot_price_path::build_from_meteora_snapshots(
                            pool,
                            start_time as i64,
                            end_time as i64,
                            &token_a,
                            &token_b,
                            *capital,
                            *lp_share,
                        )
                        .await?
                    }
                };
                if prep.step_data.is_empty() {
                    println!("âťŚ No snapshot rows in the requested time window.");
                    return Ok(());
                }
                let protocol_dir = match snapshot_protocol.unwrap() {
                    SnapshotProtocolArg::Orca => "orca",
                    SnapshotProtocolArg::Raydium => "raydium",
                    SnapshotProtocolArg::Meteora => "meteora",
                };
                println!(
                    "âś… Loaded {} snapshot steps from data/pool-snapshots/{}/{}/snapshots.jsonl",
                    prep.step_data.len(),
                    protocol_dir,
                    pool
                );
                snapshot_fee_index_full = prep.per_step_fees_usd;
                snapshot_step_data_full = Some(prep.step_data);
            }

            #[allow(clippy::type_complexity)]
            let (dune_daily_tvl, dune_daily_volume): (
                Option<HashMap<String, Decimal>>,
                Option<HashMap<String, Decimal>>,
            ) = if snapshots_only {
                (None, None)
            } else if let Some(pool) = whirlpool_address.as_ref() {
                crate::commands::backtest_optimize::fetch_dune_tvl_volume(pool).await?
            } else {
                (None, None)
            };

            let pool_for_onchain_state = whirlpool_address
                .as_ref()
                .or(snapshot_pool_address.as_ref());
            let (
                pool_active_liquidity,
                effective_fee_rate,
                token_a_decimals,
                token_b_decimals,
                pool_vault_a,
                pool_vault_b,
            ): (
                Option<u128>,
                Option<Decimal>,
                u8,
                u8,
                Option<String>,
                Option<String>,
            ) = if let Some(pool) = pool_for_onchain_state {
                // Snapshot-only simulation for Raydium/Meteora should not depend on Orca-specific
                // on-chain layout parsing. For Orca we still prefer on-chain pool state.
                if snapshots_only
                    && matches!(
                        snapshot_protocol,
                        Some(SnapshotProtocolArg::Raydium) | Some(SnapshotProtocolArg::Meteora)
                    )
                {
                    let liq = snapshot_step_data_full
                        .as_ref()
                        .and_then(|sd| sd.first())
                        .and_then(|p| p.pool_liquidity_active);
                    (
                        liq,
                        None,
                        token_a_decimals_guess,
                        if use_cross_pair {
                            token_b_decimals_guess
                        } else {
                            6u8
                        },
                        None,
                        None,
                    )
                } else {
                    crate::commands::backtest_optimize::fetch_pool_state(
                        pool,
                        token_a_decimals_guess,
                        token_b_decimals_guess,
                        use_cross_pair,
                    )
                    .await?
                }
            } else {
                (
                    None,
                    None,
                    token_a_decimals_guess,
                    if use_cross_pair {
                        token_b_decimals_guess
                    } else {
                        6u8
                    },
                    None,
                    None,
                )
            };

            let fee_rate = effective_fee_rate.unwrap_or_else(|| Decimal::from_f64(0.003).unwrap());
            let capital_dec = Decimal::from_f64(*capital).unwrap();
            let lp_share_override: Option<Decimal> = lp_share
                .and_then(Decimal::from_f64)
                .filter(|s| *s > Decimal::ZERO && *s <= Decimal::ONE);
            let steps_per_day = Decimal::from_f64(86_400f64 / res.max(1) as f64).unwrap();
            let _ = use_cross_pair;

            let swaps: Option<Vec<clmm_lp_data::swaps::SwapEvent>> = if let Some(arg) =
                dune_swaps.as_ref()
            {
                match crate::commands::backtest_optimize::fetch_swaps_for_optimize(arg).await {
                    Ok(Some(s)) => Some(s),
                    Ok(None) => None,
                    Err(e) => {
                        let msg = e.to_string();
                        if msg.contains("402")
                            || msg.contains("Payment Required")
                            || msg.contains("credits")
                        {
                            println!(
                                "âš ď¸Ź Dune API credit limit reached. Using volume-based fees instead of swap-level fees."
                            );
                        } else {
                            println!(
                                "âš ď¸Ź Could not fetch Dune swaps ({}). Using volume-based fees.",
                                e
                            );
                        }
                        None
                    }
                }
            } else {
                None
            };
            let swaps = swaps.map(|s| {
                crate::commands::backtest_optimize::filter_swaps_for_pool(
                    s,
                    pool_vault_a.as_deref(),
                    pool_vault_b.as_deref(),
                    &token_a.mint_address,
                    &token_b.mint_address,
                )
            });
            let swaps_ref: Option<&[clmm_lp_data::swaps::SwapEvent]> = swaps.as_deref();
            let require_decode_ok = matches!(*fee_swap_decode_status, FeeSwapDecodeStatusArg::Ok);

            use backtest_engine::{
                GridRunParams, PeriodicTimeBasis, RetouchRepeatConfig, StratConfig,
                build_step_data, fee_realism, run_grid,
            };
            use std::collections::BTreeMap;
            use std::sync::Arc;

            let series_len = if snapshots_only {
                snapshot_step_data_full
                    .as_ref()
                    .map(|v| v.len())
                    .unwrap_or(0)
            } else {
                candles.len()
            };
            let steps_per_window = series_len / (*windows).max(1);
            let window_ranges: Vec<std::ops::Range<usize>> = (0..*windows)
                .map(|w| (w * steps_per_window)..((w + 1) * steps_per_window))
                .collect();

            let strategies: Vec<StratConfig> =
                crate::commands::backtest_optimize::default_strategies(
                    *static_only,
                    *il_max_pct,
                    *il_close_pct,
                    *il_grace_steps,
                );

            let min_frac = (*min_range_pct / 100.0).clamp(0.001, 1.0);
            let max_frac = (*max_range_pct / 100.0).clamp(min_frac + 0.001, 2.0);
            let width_pcts: Vec<f64> = if *range_steps <= 1 {
                vec![(min_frac + max_frac) / 2.0]
            } else {
                (0..*range_steps)
                    .map(|i| {
                        min_frac + (max_frac - min_frac) * (i as f64) / ((*range_steps - 1) as f64)
                    })
                    .collect()
            };
            let tx_cost_dec = Decimal::from_f64(*tx_cost).unwrap();
            let rebalance_cost_model = if *use_realistic_rebalance_cost {
                let fixed = Decimal::from_f64(
                    *tx_cost + *network_fee_usd + *priority_fee_usd + *jito_tip_usd,
                )
                .unwrap();
                let slippage = Decimal::from_f64(*slippage_bps).unwrap();
                Some(backtest_engine::RebalanceCostModel {
                    fixed_cost_usd: fixed,
                    slippage_bps: slippage,
                })
            } else {
                None
            };
            let retouch_repeat = if *retouch_repeat_off {
                None
            } else {
                Some(RetouchRepeatConfig {
                    cooldown_secs: *retouch_repeat_cooldown_secs,
                    rearm_after_secs: *retouch_repeat_rearm_secs,
                    extra_move_pct: *retouch_repeat_extra_move_pct,
                })
            };
            let grid_params = GridRunParams {
                capital_dec,
                tx_cost_dec,
                rebalance_cost_model,
                fee_rate,
                pool_active_liquidity,
                token_a_decimals: token_a_decimals as u32,
                token_b_decimals: token_b_decimals as u32,
                step_seconds: res as i64,
                periodic_time_basis: PeriodicTimeBasis::WallClockSeconds,
                retouch_repeat,
                use_liquidity_share: lp_share_override.is_none(),
            };

            let score_fn = |s: &TrackerSummary| -> Decimal {
                match objective {
                    BacktestObjectiveArg::Pnl => s.final_pnl,
                    BacktestObjectiveArg::VsHodl => s.vs_hodl,
                    BacktestObjectiveArg::Fees => s.total_fees,
                    BacktestObjectiveArg::Composite => {
                        let il_amt = (s.final_il_pct.abs() * capital_dec).min(capital_dec);
                        s.total_fees
                            - (Decimal::from_f64(*alpha).unwrap() * il_amt)
                            - s.total_rebalance_cost
                    }
                    BacktestObjectiveArg::RiskAdj => {
                        let denom = Decimal::ONE + s.max_drawdown;
                        if denom.is_zero() {
                            s.final_pnl
                        } else {
                            s.final_pnl / denom
                        }
                    }
                }
            };

            #[allow(clippy::type_complexity)]
            let (mut results, fee_check_vol, fee_check_expected_100, audit_step_data): (Vec<OptimizeGridRow>, Decimal, Decimal, Option<Vec<backtest_engine::StepData>>) = if *windows <= 1 {
                let (mut step_data, entry_price, center) = if snapshots_only {
                    let sd = snapshot_step_data_full.clone().expect("snapshot price path");
                    let entry = sd
                        .first()
                        .map(|s| s.price_usd)
                        .unwrap_or_else(|| Price::new(Decimal::ONE));
                    let center = entry.value.to_f64().unwrap_or(1.0);
                    (sd, entry, center)
                } else {
                    build_step_data(
                        &candles,
                        dune_daily_tvl.as_ref(),
                        dune_daily_volume.as_ref(),
                        quote_usd_map.as_ref(),
                        capital_dec,
                        lp_share_override,
                        steps_per_day,
                    )
                };
                if lp_share_override.is_none()
                    && snapshot_protocol.is_some()
                    && snapshot_pool_address.is_some()
                {
                    let proto = snapshot_protocol.unwrap();
                    let pool_addr = snapshot_pool_address.clone().unwrap();
                    let base_dir = std::path::Path::new("data")
                        .join("pool-snapshots")
                        .join(match proto {
                            SnapshotProtocolArg::Orca => "orca",
                            SnapshotProtocolArg::Raydium => "raydium",
                            SnapshotProtocolArg::Meteora => "meteora",
                        });
                    let snap_path = base_dir.join(&pool_addr).join("snapshots.jsonl");
                    if snap_path.exists() {
                        let txt = std::fs::read_to_string(&snap_path)?;
                        let mut snaps: Vec<(i64, u64, u64, Option<String>, Option<String>)> = Vec::new();
                        for line in txt.lines().filter(|l| !l.trim().is_empty()) {
                            let v: serde_json::Value = match serde_json::from_str(line) {
                                Ok(v) => v,
                                Err(_) => continue,
                            };
                            let ts = v
                                .get("ts_utc")
                                .and_then(|x| x.as_str())
                                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                                .map(|dt| dt.timestamp());
                            let a = v.get("vault_amount_a").and_then(|x| x.as_u64());
                            let b = v.get("vault_amount_b").and_then(|x| x.as_u64());
                            let ma = v.get("token_mint_a").and_then(|x| x.as_str()).map(|s| s.to_string());
                            let mb = v.get("token_mint_b").and_then(|x| x.as_str()).map(|s| s.to_string());
                            if let (Some(ts), Some(a), Some(b)) = (ts, a, b) {
                                snaps.push((ts, a, b, ma, mb));
                            }
                        }
                        snaps.sort_by_key(|(t, _, _, _, _)| *t);
                        if !snaps.is_empty() {
                            let pow10 = |d: u32| -> Decimal {
                                let mut v = Decimal::ONE;
                                for _ in 0..d {
                                    v *= Decimal::from(10u32);
                                }
                                v
                            };
                            let mut j = 0usize;
                            let mut last = snaps[0].clone();
                            for p in step_data.iter_mut() {
                                let t = p.start_timestamp as i64;
                                while j + 1 < snaps.len() && snaps[j + 1].0 <= t {
                                    j += 1;
                                    last = snaps[j].clone();
                                }
                                let token_usd = |raw: u64, mint: Option<&str>| -> Option<Decimal> {
                                    let mint = mint?;
                                    if mint.eq_ignore_ascii_case(&token_a.mint_address) {
                                        let amt = Decimal::from(raw) / pow10(token_a_decimals as u32);
                                        Some(amt * p.price_usd.value)
                                    } else if mint.eq_ignore_ascii_case(&token_b.mint_address) {
                                        let amt = Decimal::from(raw) / pow10(token_b_decimals as u32);
                                        Some(amt * p.quote_usd)
                                    } else {
                                        None
                                    }
                                };
                                let tvl_usd = token_usd(last.1, last.3.as_deref()).unwrap_or(Decimal::ZERO)
                                    + token_usd(last.2, last.4.as_deref()).unwrap_or(Decimal::ZERO);
                                if tvl_usd > Decimal::ZERO {
                                    let mut share = capital_dec / tvl_usd;
                                    if share < Decimal::ZERO {
                                        share = Decimal::ZERO;
                                    }
                                    if share > Decimal::ONE {
                                        share = Decimal::ONE;
                                    }
                                    p.lp_share = share;
                                }
                            }
                        }
                    }
                }
                let local_pool_fees_arc: Option<Arc<BTreeMap<usize, Decimal>>> = {
                    let use_snapshot_fees = matches!(fee_source, FeeSourceArg::Snapshots)
                        || (matches!(fee_source, FeeSourceArg::Auto)
                            && snapshots_only
                            && snapshot_fee_index_full
                                .as_ref()
                                .map(|m| !m.is_empty())
                                .unwrap_or(false));
                    if use_snapshot_fees {
                        match snapshot_fee_index_full.as_ref() {
                            Some(m) if !m.is_empty() => {
                                println!(
                                    "🧪 backtest-optimize: snapshot pool fees from snapshots index ({} step buckets).",
                                    m.len()
                                );
                                Some(Arc::new(m.clone()))
                            }
                            _ => {
                                println!(
                                    "⚠️ backtest-optimize: snapshot fee index unavailable/empty; fees may fall back to candles."
                                );
                                None
                            }
                        }
                    } else {
                        let use_local =
                            dune_swaps.is_none() || swaps_ref.map(|s| s.is_empty()).unwrap_or(true);
                        if !use_local {
                            None
                        } else if let (Some(proto), Some(pool)) = (
                            snapshot_protocol.as_ref(),
                            snapshot_pool_address
                                .as_deref()
                                .or(whirlpool_address.as_deref()),
                        ) {
                            let pdir = match proto {
                                SnapshotProtocolArg::Orca => "orca",
                                SnapshotProtocolArg::Raydium => "raydium",
                                SnapshotProtocolArg::Meteora => "meteora",
                            };
                            match crate::local_swap_fees::build_local_pool_fees_usd(
                                pdir,
                                pool,
                                &step_data,
                                res,
                                token_a_decimals,
                                token_b_decimals,
                                fee_rate,
                                require_decode_ok,
                            ) {
                                Some(m) if !m.is_empty() => {
                                    println!(
                                        "🧪 backtest-optimize: local pool fees from data/swaps/{} ({} step buckets).",
                                        pdir,
                                        m.len()
                                    );
                                    Some(Arc::new(m))
                                }
                                _ => None,
                            }
                        } else {
                            if dune_swaps.is_none() {
                                println!(
                                    "ℹ️ backtest-optimize: no Dune swaps; set --snapshot-protocol and --snapshot-pool-address (or --whirlpool-address) to use local data/swaps fees."
                                );
                            }
                            None
                        }
                    }
                };
                let (fv, fe100) = fee_realism(&step_data, fee_rate);
                let rows = run_grid(
                    &step_data,
                    entry_price,
                    center,
                    &width_pcts,
                    &strategies,
                    &grid_params,
                    swaps_ref,
                    local_pool_fees_arc,
                );
                let mut r: Vec<_> = rows
                    .into_iter()
                    .map(|(wp, lower, upper, name, summary)| {
                    let sc = score_fn(&summary);
                    (wp, lower, upper, name, summary, sc)
                })
                    .collect();
                sort_backtest_optimize_grid(&mut r, *objective);
                (r, fv, fe100, Some(step_data))
            } else {
                type AggKey = (String, String);
                let mut agg: HashMap<AggKey, (Decimal, u32, f64, f64, f64, TrackerSummary)> =
                    HashMap::new();
                let mut fee_check_vol = Decimal::ZERO;
                let mut fee_check_expected_100 = Decimal::ZERO;
                let mut audit_step_data: Option<Vec<backtest_engine::StepData>> = None;
                for range in &window_ranges {
                    let (mut step_data, entry_price, center) = if snapshots_only {
                        let full = snapshot_step_data_full.as_ref().expect("snapshot price path");
                        let end = range.end.min(full.len());
                        let start = range.start.min(end);
                        if start >= end {
                            continue;
                        }
                        let slice = &full[start..end];
                        let entry = slice
                            .first()
                            .map(|s| s.price_usd)
                            .unwrap_or_else(|| Price::new(Decimal::ONE));
                        let center = entry.value.to_f64().unwrap_or(1.0);
                        (slice.to_vec(), entry, center)
                    } else {
                        let slice = &candles[range.clone()];
                        if slice.is_empty() {
                            continue;
                        }
                        build_step_data(
                            slice,
                            dune_daily_tvl.as_ref(),
                            dune_daily_volume.as_ref(),
                            quote_usd_map.as_ref(),
                            capital_dec,
                            lp_share_override,
                            steps_per_day,
                        )
                    };
                    if lp_share_override.is_none()
                        && snapshot_protocol.is_some()
                        && snapshot_pool_address.is_some()
                    {
                        let proto = snapshot_protocol.unwrap();
                        let pool_addr = snapshot_pool_address.clone().unwrap();
                        let base_dir = std::path::Path::new("data")
                            .join("pool-snapshots")
                            .join(match proto {
                                SnapshotProtocolArg::Orca => "orca",
                                SnapshotProtocolArg::Raydium => "raydium",
                                SnapshotProtocolArg::Meteora => "meteora",
                            });
                        let snap_path = base_dir.join(&pool_addr).join("snapshots.jsonl");
                        if snap_path.exists() {
                            let txt = std::fs::read_to_string(&snap_path)?;
                            let mut snaps: Vec<(i64, u64, u64, Option<String>, Option<String>)> = Vec::new();
                            for line in txt.lines().filter(|l| !l.trim().is_empty()) {
                                let v: serde_json::Value = match serde_json::from_str(line) {
                                    Ok(v) => v,
                                    Err(_) => continue,
                                };
                                let ts = v
                                    .get("ts_utc")
                                    .and_then(|x| x.as_str())
                                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                                    .map(|dt| dt.timestamp());
                                let a = v.get("vault_amount_a").and_then(|x| x.as_u64());
                                let b = v.get("vault_amount_b").and_then(|x| x.as_u64());
                                let ma = v.get("token_mint_a").and_then(|x| x.as_str()).map(|s| s.to_string());
                                let mb = v.get("token_mint_b").and_then(|x| x.as_str()).map(|s| s.to_string());
                                if let (Some(ts), Some(a), Some(b)) = (ts, a, b) {
                                    snaps.push((ts, a, b, ma, mb));
                                }
                            }
                            snaps.sort_by_key(|(t, _, _, _, _)| *t);
                            if !snaps.is_empty() {
                                let pow10 = |d: u32| -> Decimal {
                                    let mut v = Decimal::ONE;
                                    for _ in 0..d {
                                        v *= Decimal::from(10u32);
                                    }
                                    v
                                };
                                let mut j = 0usize;
                                let mut last = snaps[0].clone();
                                for p in step_data.iter_mut() {
                                    let t = p.start_timestamp as i64;
                                    while j + 1 < snaps.len() && snaps[j + 1].0 <= t {
                                        j += 1;
                                        last = snaps[j].clone();
                                    }
                                    let token_usd = |raw: u64, mint: Option<&str>| -> Option<Decimal> {
                                        let mint = mint?;
                                        if mint.eq_ignore_ascii_case(&token_a.mint_address) {
                                            let amt = Decimal::from(raw) / pow10(token_a_decimals as u32);
                                            Some(amt * p.price_usd.value)
                                        } else if mint.eq_ignore_ascii_case(&token_b.mint_address) {
                                            let amt = Decimal::from(raw) / pow10(token_b_decimals as u32);
                                            Some(amt * p.quote_usd)
                                        } else {
                                            None
                                        }
                                    };
                                    let tvl_usd = token_usd(last.1, last.3.as_deref()).unwrap_or(Decimal::ZERO)
                                        + token_usd(last.2, last.4.as_deref()).unwrap_or(Decimal::ZERO);
                                    if tvl_usd > Decimal::ZERO {
                                        let mut share = capital_dec / tvl_usd;
                                        if share < Decimal::ZERO {
                                            share = Decimal::ZERO;
                                        }
                                        if share > Decimal::ONE {
                                            share = Decimal::ONE;
                                        }
                                        p.lp_share = share;
                                    }
                                }
                            }
                        }
                    }
                    if fee_check_vol.is_zero() {
                        let (fv, fe100) = fee_realism(&step_data, fee_rate);
                        fee_check_vol = fv;
                        fee_check_expected_100 = fe100;
                        audit_step_data = Some(step_data.clone());
                    }
                    let local_pool_fees_arc: Option<Arc<BTreeMap<usize, Decimal>>> = {
                        let use_snapshot_fees = matches!(fee_source, FeeSourceArg::Snapshots)
                            || (matches!(fee_source, FeeSourceArg::Auto)
                                && snapshots_only
                                && snapshot_fee_index_full
                                    .as_ref()
                                    .map(|m| !m.is_empty())
                                    .unwrap_or(false));
                        if use_snapshot_fees {
                            if let Some(full) = snapshot_fee_index_full.as_ref() {
                                let mut win_map: BTreeMap<usize, Decimal> = BTreeMap::new();
                                let win_start = range.start;
                                let win_end = range.end;
                                for (idx, v) in full.iter() {
                                    if *idx >= win_start && *idx < win_end {
                                        win_map.insert(*idx - win_start, *v);
                                    }
                                }
                                if !win_map.is_empty() {
                                    Some(Arc::new(win_map))
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        } else {
                            let use_local = dune_swaps.is_none()
                                || swaps_ref.map(|s| s.is_empty()).unwrap_or(true);
                            if !use_local {
                                None
                            } else if let (Some(proto), Some(pool)) = (
                                snapshot_protocol.as_ref(),
                                snapshot_pool_address
                                    .as_deref()
                                    .or(whirlpool_address.as_deref()),
                            ) {
                                let pdir = match proto {
                                    SnapshotProtocolArg::Orca => "orca",
                                    SnapshotProtocolArg::Raydium => "raydium",
                                    SnapshotProtocolArg::Meteora => "meteora",
                                };
                                match crate::local_swap_fees::build_local_pool_fees_usd(
                                    pdir,
                                    pool,
                                    &step_data,
                                    res,
                                    token_a_decimals,
                                    token_b_decimals,
                                    fee_rate,
                                    require_decode_ok,
                                ) {
                                    Some(m) if !m.is_empty() => Some(Arc::new(m)),
                                    _ => None,
                                }
                            } else {
                                None
                            }
                        }
                    };
                    let rows = run_grid(
                        &step_data,
                        entry_price,
                        center,
                        &width_pcts,
                        &strategies,
                        &grid_params,
                        swaps_ref,
                        local_pool_fees_arc,
                    );
                    for (wp_frac, lower, upper, strat_name, summary) in rows {
                        let key = (format!("{:.6}", wp_frac), strat_name.clone());
                        let sc = score_fn(&summary);
                        agg.entry(key)
                            .and_modify(|e| {
                                e.0 += sc;
                                e.1 += 1;
                                e.2 = wp_frac;
                                e.3 = lower;
                                e.4 = upper;
                                e.5 = summary.clone();
                            })
                            .or_insert((sc, 1, wp_frac, lower, upper, summary));
                    }
                }
                let mut r: Vec<_> = agg
                    .into_iter()
                    .map(|((_, strat_name), (sum_score, count, wp, lower, upper, summary))| {
                        let avg = if count > 0 {
                            sum_score / Decimal::from(count)
                        } else {
                            sum_score
                        };
                        (wp, lower, upper, strat_name, summary, avg)
                    })
                    .collect();
                sort_backtest_optimize_grid(&mut r, *objective);
                (r, fee_check_vol, fee_check_expected_100, audit_step_data)
            };

            let _round2 = |d: Decimal| (d * Decimal::from(100)).round() / Decimal::from(100);

            let min_tir = min_time_in_range.map(|x| Decimal::from_f64(x / 100.0).unwrap());
            let max_dd = max_drawdown.map(|x| Decimal::from_f64(x / 100.0).unwrap());
            results.retain(|(_, _, _, _, s, _)| {
                min_tir.is_none_or(|m| s.time_in_range_pct >= m)
                    && max_dd.is_none_or(|m| s.max_drawdown <= m)
            });

            let n = if *full_ranking {
                results.len()
            } else {
                (*top_n).min(results.len())
            };
            let best = match results.first() {
                Some(b) => b,
                None => {
                    println!(
                        "âťŚ No results after filters (min_time_in_range / max_drawdown). Relax or remove filters."
                    );
                    return Ok(());
                }
            };
            let optimize_period_label = if let Some(h) = hours.as_ref() {
                if *windows <= 1 {
                    format!("last {h} hour(s)")
                } else {
                    format!("last {h} hour(s) × {} rolling windows", *windows)
                }
            } else if *windows <= 1 {
                format!("last {days} day(s)")
            } else {
                format!("last {days} day(s) × {} rolling windows", *windows)
            };
            use crate::output::optimize_report;
            optimize_report::print_best_block(
                &pair_label,
                &optimize_period_label,
                capital,
                windows,
                &format!("{:?}", objective),
                min_range_pct,
                max_range_pct,
                strategies.len(),
                min_time_in_range,
                max_drawdown,
                best,
                capital_dec,
                fee_rate,
                pool_active_liquidity,
                audit_step_data.as_ref(),
                token_a_decimals as u32,
                token_b_decimals as u32,
                symbol_a,
                symbol_b.as_deref(),
            );
            if matches!(objective, BacktestObjectiveArg::VsHodl) {
                println!(
                    "   ℹ️  Objective `vs_hodl` rewards staying close to a HODL benchmark → **wide** ranges often win (high TIR). For **narrower / fee-focused** optimization try `--objective fees` or `--objective composite`, and/or lower `--max-range-pct`."
                );
            }
            if matches!(objective, BacktestObjectiveArg::Fees) {
                println!(
                    "   ℹ️  Objective `fees`: many strategies **tie** on the same `total_fees` for a fixed range when they **do not rebalance** (identical path). Ranking uses tie-breakers: **better vs HODL**, then **fewer rebalances / lower rebalance cost**, then strategy name. Identical Score+Fees rows are economically redundant picks among strategies that behaved the same."
                );
            }
            if let Some(h) = hours.as_ref() {
                if *h < 72 {
                    println!(
                        "   ℹ️  Short horizon ({h}h): `periodic_48h` / `periodic_72h` cannot fire; they match static if nothing else triggers. Many identical rows usually mean **0 rebalances** (price in range, no time/IL triggers)."
                    );
                }
            }
            if fee_check_expected_100 > Decimal::ZERO {
                let ratio_pct = best.4.total_fees / fee_check_expected_100 * Decimal::from(100);
                if *windows <= 1 {
                    println!(
                        "   Fee check: period volume ${:.0}, expected (100% TIR) ${:.2}, simulated ${:.2} (ratio {:.1}%)",
                        fee_check_vol, fee_check_expected_100, best.4.total_fees, ratio_pct
                    );
                } else {
                    println!(
                        "   Fee check (first window only): period volume ${:.0}, expected (100% TIR) ${:.2}; BEST simulated ${:.2} is from last window (ratio {:.1}% vs first window)",
                        fee_check_vol, fee_check_expected_100, best.4.total_fees, ratio_pct
                    );
                }
            }
            // Main ranking table (same columns as `optimize_report::build_results_table`); show for any
            // `top_n >= 1` so `--top-n 1` still gets a table (not only `print_candidate_sets`).
            if n > 0 {
                println!();
                if *full_ranking {
                    let win_label = hours
                        .map(|h| format!("{h}h"))
                        .unwrap_or_else(|| format!("{days}d"));
                    println!(
                        "### BACKTEST OPTIMIZE — MAIN RANKING TABLE (fee-source {:?}, objective {:?})",
                        fee_source, objective
                    );
                    println!("{}", "=".repeat(80));
                    println!("WINDOW: {win_label}");
                    println!("{}", "=".repeat(80));
                }
                let quote_usd_for_table: Option<Decimal> = if use_cross_pair {
                    audit_step_data
                        .as_ref()
                        .and_then(|v| v.first().map(|p| p.quote_usd))
                        .filter(|q| *q > Decimal::ZERO)
                } else {
                    None
                };
                let table = optimize_report::build_results_table(
                    &results,
                    n,
                    use_cross_pair,
                    quote_usd_for_table,
                    capital_dec,
                );
                table.printstd();
            }
            // Print diverse candidate sets to support manual/bot selection
            optimize_report::print_candidate_sets(
                &results,
                n,
                use_cross_pair,
                audit_step_data.as_ref(),
                capital_dec,
            );
            println!();

            if let Some(out_path) = optimize_result_json.as_ref() {
                let pool_addr = whirlpool_address
                    .clone()
                    .or_else(|| snapshot_pool_address.clone());
                let file = crate::output::optimize_result_json::build_optimize_result_file(
                    chrono::Utc::now().to_rfc3339(),
                    &format!("{:?}", objective),
                    pair_label.clone(),
                    token_a.mint_address.clone(),
                    token_b.mint_address.clone(),
                    token_a_decimals,
                    token_b_decimals,
                    pool_addr,
                    &format!("{:?}", price_path_source),
                    &format!("{:?}", fee_source),
                    *windows,
                    best.0,
                    best.1,
                    best.2,
                    best.3.as_str(),
                    &best.4,
                    best.5,
                    retouch_repeat,
                )?;
                crate::output::optimize_result_json::write_optimize_result_json(out_path, &file)?;
                println!("Optimize result JSON written to {}", out_path.display());
                if let Some(copy_dir) = optimize_result_json_copy_dir.as_ref() {
                    crate::output::optimize_result_json::write_optimize_result_copy_dir(
                        copy_dir, &file,
                    )?;
                    println!(
                        "Optimize result copies written under {} (timestamped + latest.json)",
                        copy_dir.display()
                    );
                }
            }
        }
        Commands::Optimize {
            symbol_a,
            mint_a,
            symbol_b,
            mint_b,
            days,
            capital,
            objective,
            iterations,
        } => {
            let api_key = env::var("BIRDEYE_API_KEY")
                .expect("BIRDEYE_API_KEY must be set in .env or environment");

            println!("đź“ˇ Initializing Optimizer...");
            let provider = BirdeyeProvider::new(api_key);

            // Define Tokens
            let (token_a_decimals, token_b_decimals): (u8, u8) = {
                use crate::engine::token_meta::fetch_mint_decimals;
                use clmm_lp_protocols::rpc::RpcProvider;
                let rpc = RpcProvider::mainnet();
                let da = fetch_mint_decimals(&rpc, mint_a).await.unwrap_or(9);
                let db = if let Some(mb) = mint_b.as_ref() {
                    fetch_mint_decimals(&rpc, mb).await.unwrap_or(9)
                } else {
                    6u8
                };
                (da, db)
            };
            let token_a = Token::new(mint_a, symbol_a, token_a_decimals, symbol_a);

            let (token_b, use_cross_pair) =
                if let (Some(sb), Some(mb)) = (symbol_b.as_ref(), mint_b.as_ref()) {
                    let tb = Token::new(mb, sb, token_b_decimals, sb);
                    (tb, true)
                } else {
                    let tb = Token::new(
                        "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
                        "USDC",
                        6,
                        "USD Coin",
                    );
                    (tb, false)
                };

            let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
            let start_time = now - (days * 24 * 3600);

            if use_cross_pair {
                println!(
                    "đź”Ť Fetching historical data for {}/{} ({} days) to estimate volatility...",
                    symbol_a,
                    symbol_b.as_deref().unwrap_or("UNKNOWN"),
                    days
                );
            } else {
                println!(
                    "đź”Ť Fetching historical data for {}/USDC ({} days) to estimate volatility...",
                    symbol_a, days
                );
            }

            let candles = if use_cross_pair {
                provider
                    .get_cross_pair_price_history(&token_a, &token_b, start_time, now, 3600)
                    .await?
            } else {
                provider
                    .get_price_history(&token_a, &token_b, start_time, now, 3600)
                    .await?
            };

            if candles.is_empty() {
                println!("âťŚ No data found for the specified period.");
                return Ok(());
            }

            // Calculate volatility from historical data
            let prices: Vec<f64> = candles
                .iter()
                .map(|c| c.close.value.to_f64().unwrap_or(0.0))
                .collect();

            let volatility = calculate_volatility(&prices);
            let current_price = *prices.last().unwrap_or(&100.0);
            let current_price_dec = Decimal::from_f64(current_price).unwrap();

            println!("đź“Š Market Analysis:");
            println!("   Current Price: ${:.4}", current_price);
            println!("   Volatility (annualized): {:.1}%", volatility * 100.0);
            println!();

            // Setup optimizer
            let optimizer = RangeOptimizer::new(*iterations, 30, 1.0 / 365.0);

            let base_position = Position {
                id: clmm_lp_domain::entities::position::PositionId(Uuid::new_v4()),
                pool_address: "opt-pool".to_string(),
                owner_address: "user".to_string(),
                liquidity_amount: 0,
                deposited_amount_a: Amount::new(U256::zero(), 9),
                deposited_amount_b: Amount::new(U256::zero(), 6),
                current_amount_a: Amount::new(U256::zero(), 9),
                current_amount_b: Amount::new(U256::zero(), 6),
                unclaimed_fees_a: Amount::new(U256::zero(), 9),
                unclaimed_fees_b: Amount::new(U256::zero(), 6),
                range: None,
                opened_at: now,
                status: PositionStatus::Open,
            };

            let volume =
                ConstantVolume::from_amount(Amount::new(U256::from(1_000_000_000_000u64), 6));
            let pool_liquidity = (*capital as u128) * 1000;
            let fee_rate = Decimal::from_f64(0.003).unwrap();

            println!(
                "đź”„ Running optimization with {:?} objective ({} iterations)...",
                objective, iterations
            );

            let result = match objective {
                OptimizationObjectiveArg::Pnl => optimizer.optimize(
                    base_position,
                    current_price_dec,
                    volatility,
                    0.0,
                    volume,
                    pool_liquidity,
                    fee_rate,
                    MaximizeNetPnL,
                ),
                OptimizationObjectiveArg::Fees => optimizer.optimize(
                    base_position,
                    current_price_dec,
                    volatility,
                    0.0,
                    volume,
                    pool_liquidity,
                    fee_rate,
                    MaximizeFees,
                ),
                OptimizationObjectiveArg::Sharpe => optimizer.optimize(
                    base_position,
                    current_price_dec,
                    volatility,
                    0.0,
                    volume,
                    pool_liquidity,
                    fee_rate,
                    MaximizeSharpeRatio::new(Decimal::from_f64(0.05).unwrap()),
                ),
            };

            // Print optimization results
            print_optimization_report(symbol_a, current_price, volatility, *capital, &result);
        }
        Commands::Db { action } => {
            let database_url = env::var("DATABASE_URL")
                .unwrap_or_else(|_| "postgres://localhost/clmm_lp".to_string());

            match action {
                DbAction::Init => {
                    println!("đź”§ Initializing database...");
                    let db = Database::connect(&database_url).await?;
                    db.migrate().await?;
                    println!("âś… Database initialized successfully!");
                }
                DbAction::Status => {
                    println!("đź”Ť Checking database connection...");
                    match Database::connect(&database_url).await {
                        Ok(_) => {
                            println!("âś… Connected to database: {}", database_url);
                        }
                        Err(e) => {
                            println!("âťŚ Failed to connect: {}", e);
                        }
                    }
                }
                DbAction::ListSimulations { limit } => {
                    let db = Database::connect(&database_url).await?;
                    let simulations = db.simulations().find_recent(*limit).await?;

                    if simulations.is_empty() {
                        println!("No simulations found.");
                    } else {
                        println!("đź“Š Recent Simulations:");
                        println!();
                        let mut table = Table::new();
                        table.add_row(row!["ID", "Strategy", "Capital", "Range", "Created"]);
                        for sim in simulations {
                            table.add_row(row![
                                sim.id.to_string()[..8].to_string(),
                                sim.strategy_type,
                                format!("${:.2}", sim.initial_capital),
                                format!("${:.2} - ${:.2}", sim.lower_price, sim.upper_price),
                                sim.created_at.format("%Y-%m-%d %H:%M")
                            ]);
                        }
                        table.printstd();
                    }
                }
                DbAction::ListOptimizations { limit } => {
                    let db = Database::connect(&database_url).await?;
                    let optimizations = db.simulations().find_recent_optimizations(*limit).await?;

                    if optimizations.is_empty() {
                        println!("No optimizations found.");
                    } else {
                        println!("đźŽŻ Recent Optimizations:");
                        println!();
                        let mut table = Table::new();
                        table.add_row(row!["ID", "Objective", "Range", "Expected PnL", "Created"]);
                        for opt in optimizations {
                            table.add_row(row![
                                opt.id.to_string()[..8].to_string(),
                                opt.objective_type,
                                format!(
                                    "${:.2} - ${:.2}",
                                    opt.recommended_lower, opt.recommended_upper
                                ),
                                format!("${:+.4}", opt.expected_pnl),
                                opt.created_at.format("%Y-%m-%d %H:%M")
                            ]);
                        }
                        table.printstd();
                    }
                }
            }
        }
        Commands::Analyze {
            symbol_a,
            mint_a,
            symbol_b,
            mint_b,
            days,
        } => {
            let api_key = env::var("BIRDEYE_API_KEY")
                .expect("BIRDEYE_API_KEY must be set in .env or environment");

            let use_cross_pair = symbol_b.is_some() || mint_b.is_some();
            if use_cross_pair && (symbol_b.is_none() || mint_b.is_none()) {
                println!("âťŚ For cross-pairs, pass both --symbol-b and --mint-b.");
                return Ok(());
            }
            let pair_label = if use_cross_pair {
                format!("{}/{}", symbol_a, symbol_b.as_deref().unwrap_or("?"))
            } else {
                format!("{}/USDC", symbol_a)
            };
            println!("đź“Š Analyzing {} over {} days...", pair_label, days);
            println!();

            let provider = BirdeyeProvider::new(api_key);

            let (token_a_decimals, token_b_decimals): (u8, u8) = {
                use crate::engine::token_meta::fetch_mint_decimals;
                use clmm_lp_protocols::rpc::RpcProvider;
                let rpc = RpcProvider::mainnet();
                let da = fetch_mint_decimals(&rpc, mint_a).await.unwrap_or(9);
                let db = if let Some(mb) = mint_b.as_ref() {
                    fetch_mint_decimals(&rpc, mb).await.unwrap_or(9)
                } else {
                    6u8
                };
                (da, db)
            };
            let token_a = Token::new(mint_a, symbol_a, token_a_decimals, symbol_a);
            let token_b = if use_cross_pair {
                Token::new(
                    mint_b.as_ref().expect("validated above"),
                    symbol_b.as_ref().expect("validated above"),
                    token_b_decimals,
                    symbol_b.as_ref().expect("validated above"),
                )
            } else {
                Token::new(
                    "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
                    "USDC",
                    6,
                    "USD Coin",
                )
            };

            let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
            let start_time = now - (days * 24 * 3600);

            let candles = if use_cross_pair {
                provider
                    .get_cross_pair_price_history(&token_a, &token_b, start_time, now, 3600)
                    .await?
            } else {
                provider
                    .get_price_history(&token_a, &token_b, start_time, now, 3600)
                    .await?
            };

            if candles.is_empty() {
                println!("âťŚ No data available for the specified period.");
                return Ok(());
            }

            // Calculate statistics
            let prices: Vec<f64> = candles
                .iter()
                .filter_map(|c| c.close.value.to_f64())
                .collect();

            let current_price = prices.last().copied().unwrap_or(0.0);
            let first_price = prices.first().copied().unwrap_or(0.0);
            let max_price = prices.iter().copied().fold(f64::MIN, f64::max);
            let min_price = prices.iter().copied().fold(f64::MAX, f64::min);
            let avg_price = prices.iter().sum::<f64>() / prices.len() as f64;

            let price_change = if first_price > 0.0 {
                (current_price - first_price) / first_price * 100.0
            } else {
                0.0
            };

            let volatility = calculate_volatility(&prices);
            let volatility_daily = volatility / (365.0_f64).sqrt();

            // Calculate volume stats
            let total_volume: f64 = candles
                .iter()
                .map(|c| c.volume_token_a.to_decimal().to_f64().unwrap_or(0.0))
                .sum();
            let avg_hourly_volume = total_volume / candles.len() as f64;

            // Print analysis report
            println!("đźŽŻ ANALYSIS RESULTS: {}", pair_label);
            println!();

            // Price Statistics Table
            let mut price_table = Table::new();
            price_table.add_row(row!["PRICE STATISTICS", ""]);
            price_table.add_row(row!["Current Price", format!("${:.4}", current_price)]);
            price_table.add_row(row!["Period Start", format!("${:.4}", first_price)]);
            price_table.add_row(row!["Period High", format!("${:.4}", max_price)]);
            price_table.add_row(row!["Period Low", format!("${:.4}", min_price)]);
            price_table.add_row(row!["Average Price", format!("${:.4}", avg_price)]);
            price_table.add_row(row!["Price Change", format!("{:+.2}%", price_change)]);
            price_table.add_row(row![
                "Price Range",
                format!("${:.4} - ${:.4}", min_price, max_price)
            ]);
            price_table.printstd();

            println!();

            // Volatility Table
            let mut vol_table = Table::new();
            vol_table.add_row(row!["VOLATILITY METRICS", ""]);
            vol_table.add_row(row![
                "Annualized Volatility",
                format!("{:.1}%", volatility * 100.0)
            ]);
            vol_table.add_row(row![
                "Daily Volatility",
                format!("{:.2}%", volatility_daily * 100.0)
            ]);
            vol_table.add_row(row!["Data Points", format!("{} candles", candles.len())]);
            vol_table.printstd();

            println!();

            // Volume Table
            let mut volume_table = Table::new();
            volume_table.add_row(row!["VOLUME METRICS", ""]);
            volume_table.add_row(row![
                "Total Volume",
                format!("{:.2} {}", total_volume, symbol_a)
            ]);
            volume_table.add_row(row![
                "Avg Hourly Volume",
                format!("{:.2} {}", avg_hourly_volume, symbol_a)
            ]);
            volume_table.add_row(row![
                "Avg Daily Volume",
                format!("{:.2} {}", avg_hourly_volume * 24.0, symbol_a)
            ]);
            volume_table.printstd();

            println!();

            // Suggested ranges based on volatility
            let range_1x = current_price * volatility_daily;
            let range_2x = current_price * volatility_daily * 2.0;

            let mut suggest_table = Table::new();
            suggest_table.add_row(row!["SUGGESTED LP RANGES", ""]);
            suggest_table.add_row(row![
                "Conservative (1Ď daily)",
                format!(
                    "${:.2} - ${:.2}",
                    current_price - range_1x,
                    current_price + range_1x
                )
            ]);
            suggest_table.add_row(row![
                "Moderate (2Ď daily)",
                format!(
                    "${:.2} - ${:.2}",
                    current_price - range_2x,
                    current_price + range_2x
                )
            ]);
            suggest_table.add_row(row![
                "Wide (period range)",
                format!("${:.2} - ${:.2}", min_price * 0.95, max_price * 1.05)
            ]);
            suggest_table.printstd();

            println!();
            println!("đź’ˇ Tip: Use these ranges with the backtest command:");
            println!(
                "   clmm-lp-cli backtest --lower {:.2} --upper {:.2} --days {}",
                current_price - range_2x,
                current_price + range_2x,
                days
            );
            println!();
        }
        Commands::DuneSyncSwaps { protocol, query_id } => {
            let query_id = query_id
                .clone()
                .or_else(|| {
                    protocol.as_ref().map(|p| {
                        crate::commands::backtest_optimize::dune_swaps_query_id(p).to_string()
                    })
                })
                .ok_or_else(|| {
                    anyhow::anyhow!("Provide --protocol (orca|meteora|raydium) or --query-id <id>")
                })?;
            let dune = clmm_lp_data::providers::DuneClient::from_env_swaps_only()?;
            println!(
                "đź“ˇ Fetching Dune swaps (query {}) and saving to cache...",
                query_id
            );
            let swaps = dune.fetch_swaps_force(&query_id).await?;
            let path = std::path::Path::new("data")
                .join("dune-cache")
                .join(format!("{}.json", query_id));
            println!(
                "âś… Cached {} swap events to {}",
                swaps.len(),
                path.display()
            );
        }
        Commands::DunePoolMetrics { pool_address } => {
            use clmm_lp_data::providers::{DuneClient, TvlPoint, VolumePoint};

            let dune = DuneClient::from_env()?;

            println!("đź“ˇ Fetching TVL series from Dune for pool {pool_address}...");
            let tvl_series: Vec<TvlPoint> = dune.fetch_tvl(pool_address).await?;
            println!("   TVL points: {}", tvl_series.len());

            println!("đź“ˇ Fetching volume/fees series from Dune for pool {pool_address}...");
            let vol_series: Vec<VolumePoint> = dune.fetch_volume_fees(pool_address).await?;
            println!("   Volume/fees points: {}", vol_series.len());

            println!();
            println!("date,tvl_usd,volume_usd,fees_usd");
            for tvl in &tvl_series {
                let vol = vol_series.iter().find(|v| v.date == tvl.date);
                let volume_usd = vol
                    .map(|v| v.volume_usd.to_string())
                    .unwrap_or_else(|| "0".into());
                let fees_usd = vol
                    .map(|v| v.fees_usd.to_string())
                    .unwrap_or_else(|| "0".into());

                println!("{},{},{},{}", tvl.date, tvl.tvl_usd, volume_usd, fees_usd);
            }
        }
        Commands::OrcaPoolFee { pool_address } => {
            let rpc = std::sync::Arc::new(clmm_lp_protocols::rpc::RpcProvider::mainnet());
            let reader = clmm_lp_protocols::orca::pool_reader::WhirlpoolReader::new(rpc);
            let state = reader.get_pool_state(&pool_address).await?;

            let base_fee = state.fee_rate();
            let proto = Decimal::from(state.protocol_fee_rate_bps) / Decimal::from(10_000);
            let eff = clmm_lp_domain::prelude::calculate_effective_fee_rate(base_fee, proto);
            let base_pct = base_fee.to_f64().unwrap_or(0.0) * 100.0;
            let proto_pct = proto.to_f64().unwrap_or(0.0) * 100.0;
            let eff_pct = eff.to_f64().unwrap_or(0.0) * 100.0;

            println!("Orca Whirlpool: {}", pool_address);
            println!(
                "  fee_rate_raw: {} (hundredths-of-bp)  -> {:.4}%",
                state.fee_rate_bps, base_pct
            );
            println!(
                "  protocol_fee_rate_bps: {} ({:.2}% of fees)",
                state.protocol_fee_rate_bps, proto_pct
            );
            println!(
                "  effective_fee_rate: {:.4}% (LP share after protocol cut)",
                eff_pct
            );
        }
        Commands::DefiLlamaFindPools {
            query,
            chain,
            limit,
        } => {
            use clmm_lp_data::providers::DefiLlamaClient;
            let client = DefiLlamaClient::new();
            let pools = client.list_pools().await?;
            let q = query.to_lowercase();
            let chain_lc = chain.to_lowercase();

            let mut matches: Vec<_> = pools
                .into_iter()
                .filter(|p| p.chain.to_lowercase() == chain_lc)
                .filter(|p| {
                    let hay = format!("{} {}", p.symbol, p.project).to_lowercase();
                    hay.contains(&q)
                })
                .collect();

            // Sort by TVL desc (rough relevance).
            matches.sort_by(|a, b| {
                b.tvl_usd
                    .partial_cmp(&a.tvl_usd)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

            println!("pool_id,chain,project,symbol,tvl_usd");
            for p in matches.into_iter().take(*limit) {
                println!(
                    "{},{},{},{},{:.2}",
                    p.pool, p.chain, p.project, p.symbol, p.tvl_usd
                );
            }
        }
        Commands::DefiLlamaSyncTvl { pool_id } => {
            use clmm_lp_data::providers::DefiLlamaClient;
            let client = DefiLlamaClient::new();
            let daily = client.fetch_daily_tvl(pool_id).await?;
            let path = std::path::Path::new("data")
                .join("defillama-cache")
                .join(format!("daily_tvl_{}.json", pool_id));
            println!(
                "âś… Cached {} daily TVL points to {}",
                daily.len(),
                path.display()
            );
        }
        Commands::OrcaSnapshot { pool_address } => {
            snapshots::collector::orca_snapshot(pool_address).await?
        }
        Commands::OrcaSnapshotCurated { limit } => {
            snapshots::collector::orca_snapshot_curated(*limit).await?
        }
        Commands::RaydiumSnapshotCurated { limit } => {
            snapshots::collector::raydium_snapshot_curated(*limit).await?
        }
        Commands::MeteoraSnapshotCurated { limit } => {
            snapshots::collector::meteora_snapshot_curated(*limit).await?
        }
        Commands::SnapshotRunCuratedAll { limit } => {
            use base64::{Engine, engine::general_purpose::STANDARD as BASE64_STANDARD};
            use clmm_lp_domain::prelude::calculate_effective_fee_rate;
            use rust_decimal::prelude::ToPrimitive;
            use serde::Serialize;
            use spl_token::solana_program::program_pack::Pack;
            use spl_token::state::Account as SplTokenAccount;
            use std::time::Instant;

            #[derive(Debug, Serialize)]
            struct ProtocolRunStats {
                target: usize,
                success: usize,
                failed: usize,
            }

            #[derive(Debug, Serialize)]
            struct RunErrorEntry {
                protocol: String,
                pool_address: String,
                error: String,
            }

            #[derive(Debug, Serialize)]
            struct SnapshotRunStatus {
                ts_utc: String,
                elapsed_ms: u128,
                rpc_slot: u64,
                ok: bool,
                orca: ProtocolRunStats,
                raydium: ProtocolRunStats,
                meteora: ProtocolRunStats,
                errors: Vec<RunErrorEntry>,
            }

            let startup_path = std::path::Path::new("STARTUP.md");
            let content = std::fs::read_to_string(startup_path).map_err(|e| {
                anyhow::anyhow!(
                    "Failed to read STARTUP.md at {}: {}",
                    startup_path.display(),
                    e
                )
            })?;

            let is_solana_pubkey = |s: &str| {
                if s.len() < 32 || s.len() > 44 {
                    return false;
                }
                if s.contains('-') {
                    return false;
                }
                s.chars()
                    .all(|c| matches!(c, '1'..='9' | 'A'..='Z' | 'a'..='z'))
            };

            // --- Extract Orca pools ---
            let mut in_section = false;
            let mut done = false;
            let mut orca_pool_addrs: Vec<String> = Vec::new();
            for line in content.lines() {
                if line.contains("**Orca (Whirlpool)**")
                    || line.trim_start().starts_with("**Orca (Whirlpool)**")
                {
                    in_section = true;
                    continue;
                }
                if in_section && (line.contains("**Meteora**") || line.contains("**Raydium**")) {
                    done = true;
                }
                if done {
                    break;
                }
                if in_section {
                    let chars = line.chars().collect::<Vec<_>>();
                    let mut i = 0usize;
                    while i < chars.len() {
                        if chars[i] == '`' {
                            if let Some(j) = (i + 1..chars.len()).find(|&k| chars[k] == '`') {
                                let addr: String = chars[i + 1..j].iter().collect();
                                if is_solana_pubkey(&addr) && !orca_pool_addrs.contains(&addr) {
                                    orca_pool_addrs.push(addr.clone());
                                    if limit.map(|l| orca_pool_addrs.len() >= l).unwrap_or(false) {
                                        done = true;
                                    }
                                }
                                i = j + 1;
                                continue;
                            }
                        }
                        i += 1;
                    }
                }
            }

            // --- Extract Meteora pools ---
            in_section = false;
            done = false;
            let mut meteora_pool_addrs: Vec<String> = Vec::new();
            for line in content.lines() {
                if line.trim_start().starts_with("**Meteora**") {
                    in_section = true;
                    continue;
                }
                if in_section && line.contains("**Raydium**") {
                    done = true;
                }
                if done {
                    break;
                }
                if in_section {
                    let chars = line.chars().collect::<Vec<_>>();
                    let mut i = 0usize;
                    while i < chars.len() {
                        if chars[i] == '`' {
                            if let Some(j) = (i + 1..chars.len()).find(|&k| chars[k] == '`') {
                                let addr: String = chars[i + 1..j].iter().collect();
                                if is_solana_pubkey(&addr) && !meteora_pool_addrs.contains(&addr) {
                                    meteora_pool_addrs.push(addr.clone());
                                    if limit
                                        .map(|l| meteora_pool_addrs.len() >= l)
                                        .unwrap_or(false)
                                    {
                                        done = true;
                                    }
                                }
                                i = j + 1;
                                continue;
                            }
                        }
                        i += 1;
                    }
                }
            }

            // --- Extract Raydium pools ---
            in_section = false;
            done = false;
            let mut raydium_pool_addrs: Vec<String> = Vec::new();
            for line in content.lines() {
                if line.trim_start().starts_with("**Raydium**") {
                    in_section = true;
                    continue;
                }
                if in_section && line.trim_start().starts_with("1.") {
                    done = true;
                }
                if done {
                    break;
                }
                if in_section {
                    let chars = line.chars().collect::<Vec<_>>();
                    let mut i = 0usize;
                    while i < chars.len() {
                        if chars[i] == '`' {
                            if let Some(j) = (i + 1..chars.len()).find(|&k| chars[k] == '`') {
                                let addr: String = chars[i + 1..j].iter().collect();
                                if is_solana_pubkey(&addr) && !raydium_pool_addrs.contains(&addr) {
                                    raydium_pool_addrs.push(addr.clone());
                                    if limit
                                        .map(|l| raydium_pool_addrs.len() >= l)
                                        .unwrap_or(false)
                                    {
                                        done = true;
                                    }
                                }
                                i = j + 1;
                                continue;
                            }
                        }
                        i += 1;
                    }
                }
            }

            if orca_pool_addrs.is_empty() {
                return Err(anyhow::anyhow!("No Orca pools found in STARTUP.md"));
            }

            let run_started_at = chrono::Utc::now();
            let run_started_instant = Instant::now();
            let mut run_errors: Vec<RunErrorEntry> = Vec::new();
            let mut orca_success = 0usize;
            let mut raydium_success = 0usize;
            let mut meteora_success = 0usize;

            let orca_target = orca_pool_addrs.len();
            let raydium_target = raydium_pool_addrs.len();
            let meteora_target = meteora_pool_addrs.len();

            let rpc = std::sync::Arc::new(clmm_lp_protocols::rpc::RpcProvider::mainnet());
            let slot_now = rpc.get_slot().await.unwrap_or(0);

            // ---- Orca snapshots (proper fields) ----
            #[derive(Debug, Serialize)]
            struct OrcaWhirlpoolSnapshot {
                ts_utc: String,
                slot: u64,
                pool_address: String,
                token_mint_a: String,
                token_mint_b: String,
                token_vault_a: String,
                token_vault_b: String,
                vault_amount_a: u64,
                vault_amount_b: u64,
                liquidity_active: String,
                tick_current: i32,
                fee_rate_raw: u16,
                protocol_fee_rate_bps: u16,
                fee_growth_global_a: String,
                fee_growth_global_b: String,
                protocol_fee_owed_a: u64,
                protocol_fee_owed_b: u64,
                effective_fee_rate_pct: f64,
            }

            let orca_reader =
                clmm_lp_protocols::orca::pool_reader::WhirlpoolReader::new(rpc.clone());
            for pool_address in orca_pool_addrs.into_iter() {
                let result: anyhow::Result<()> = async {
                    let state = orca_reader.get_pool_state(&pool_address).await?;
                    let accounts = rpc
                        .get_multiple_accounts(&[state.token_vault_a, state.token_vault_b])
                        .await?;

                    let vault_amount_a = accounts
                        .get(0)
                        .and_then(|a| a.as_ref())
                        .and_then(|a| SplTokenAccount::unpack(&a.data).ok())
                        .map(|a| a.amount)
                        .unwrap_or(0);
                    let vault_amount_b = accounts
                        .get(1)
                        .and_then(|a| a.as_ref())
                        .and_then(|a| SplTokenAccount::unpack(&a.data).ok())
                        .map(|a| a.amount)
                        .unwrap_or(0);

                    let base_fee = state.fee_rate();
                    let proto = rust_decimal::Decimal::from(state.protocol_fee_rate_bps)
                        / rust_decimal::Decimal::from(10_000);
                    let eff = calculate_effective_fee_rate(base_fee, proto);
                    let eff_pct = eff.to_f64().unwrap_or(0.0) * 100.0;

                    let snap = OrcaWhirlpoolSnapshot {
                        ts_utc: chrono::Utc::now().to_rfc3339(),
                        slot: slot_now,
                        pool_address: pool_address.to_string(),
                        token_mint_a: state.token_mint_a.to_string(),
                        token_mint_b: state.token_mint_b.to_string(),
                        token_vault_a: state.token_vault_a.to_string(),
                        token_vault_b: state.token_vault_b.to_string(),
                        vault_amount_a,
                        vault_amount_b,
                        liquidity_active: state.liquidity.to_string(),
                        tick_current: state.tick_current,
                        fee_rate_raw: state.fee_rate_bps,
                        protocol_fee_rate_bps: state.protocol_fee_rate_bps,
                        fee_growth_global_a: state.fee_growth_global_a.to_string(),
                        fee_growth_global_b: state.fee_growth_global_b.to_string(),
                        protocol_fee_owed_a: state.protocol_fee_owed_a,
                        protocol_fee_owed_b: state.protocol_fee_owed_b,
                        effective_fee_rate_pct: eff_pct,
                    };

                    let mut dir = std::path::PathBuf::from("data");
                    dir.push("pool-snapshots");
                    dir.push("orca");
                    dir.push(&pool_address);
                    std::fs::create_dir_all(&dir)?;
                    let mut path = dir;
                    path.push("snapshots.jsonl");

                    let line = serde_json::to_string(&snap)?;
                    let mut f = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&path)?;
                    use std::io::Write;
                    f.write_all(line.as_bytes())?;
                    f.write_all(b"\n")?;

                    println!("âś… Snapshot appended: {}", path.display());
                    Ok(())
                }
                .await;

                match result {
                    Ok(()) => orca_success += 1,
                    Err(e) => {
                        eprintln!("âťŚ Orca snapshot failed for {}: {}", pool_address, e);
                        run_errors.push(RunErrorEntry {
                            protocol: "orca".to_string(),
                            pool_address: pool_address.clone(),
                            error: e.to_string(),
                        });
                    }
                }
            }

            // ---- Raydium raw snapshots ----
            #[derive(Debug, Serialize)]
            struct RaydiumClmmSnapshot {
                ts_utc: String,
                slot: u64,
                protocol: String,
                pool_address: String,
                owner: String,
                lamports: u64,

                data_len: usize,
                data_b64: String,

                /// Whether the Raydium pool account bytes were decoded successfully.
                parse_ok: bool,
                /// Detailed decode error (only set when `parse_ok=false`).
                #[serde(skip_serializing_if = "Option::is_none")]
                parse_error: Option<String>,

                #[serde(skip_serializing_if = "Option::is_none")]
                token_mint_a: Option<String>,
                #[serde(skip_serializing_if = "Option::is_none")]
                token_mint_b: Option<String>,
                #[serde(skip_serializing_if = "Option::is_none")]
                token_vault_a: Option<String>,
                #[serde(skip_serializing_if = "Option::is_none")]
                token_vault_b: Option<String>,

                #[serde(skip_serializing_if = "Option::is_none")]
                vault_amount_a: Option<u64>,
                #[serde(skip_serializing_if = "Option::is_none")]
                vault_amount_b: Option<u64>,

                #[serde(skip_serializing_if = "Option::is_none")]
                mint_decimals_a: Option<u8>,
                #[serde(skip_serializing_if = "Option::is_none")]
                mint_decimals_b: Option<u8>,

                #[serde(skip_serializing_if = "Option::is_none")]
                liquidity_active: Option<String>,
                #[serde(skip_serializing_if = "Option::is_none")]
                tick_current: Option<i32>,
                #[serde(skip_serializing_if = "Option::is_none")]
                sqrt_price_x64: Option<String>,

                #[serde(skip_serializing_if = "Option::is_none")]
                fee_growth_global_a_x64: Option<String>,
                #[serde(skip_serializing_if = "Option::is_none")]
                fee_growth_global_b_x64: Option<String>,

                #[serde(skip_serializing_if = "Option::is_none")]
                protocol_fees_token_a: Option<u64>,
                #[serde(skip_serializing_if = "Option::is_none")]
                protocol_fees_token_b: Option<u64>,
            }
            for pool_address in raydium_pool_addrs.into_iter() {
                let result: anyhow::Result<()> = async {
                    let acct = rpc.get_account_by_address(&pool_address).await?;
                    let (parsed, parse_ok, parse_error) =
                        match clmm_lp_protocols::raydium::pool_reader::parse_pool_state(&acct.data)
                        {
                            Ok(p) => (Some(p), true, None),
                            Err(e) => (None, false, Some(e.to_string())),
                        };
                    let (vault_amount_a, vault_amount_b) = if let Some(ref p) = parsed {
                        use spl_token::solana_program::program_pack::Pack;
                        use spl_token::state::Account as SplTokenAccount;
                        use std::str::FromStr;
                        let va = solana_sdk::pubkey::Pubkey::from_str(&p.token_vault0).ok();
                        let vb = solana_sdk::pubkey::Pubkey::from_str(&p.token_vault1).ok();
                        if let (Some(va), Some(vb)) = (va, vb) {
                            match rpc.get_multiple_accounts(&[va, vb]).await {
                                Ok(accounts) => {
                                    let a = accounts
                                        .get(0)
                                        .and_then(|a| a.as_ref())
                                        .and_then(|a| SplTokenAccount::unpack(&a.data).ok())
                                        .map(|a| a.amount);
                                    let b = accounts
                                        .get(1)
                                        .and_then(|a| a.as_ref())
                                        .and_then(|a| SplTokenAccount::unpack(&a.data).ok())
                                        .map(|a| a.amount);
                                    (a, b)
                                }
                                Err(_) => (None, None),
                            }
                        } else {
                            (None, None)
                        }
                    } else {
                        (None, None)
                    };
                    let snap = RaydiumClmmSnapshot {
                        ts_utc: chrono::Utc::now().to_rfc3339(),
                        slot: slot_now,
                        protocol: "raydium".to_string(),
                        pool_address: pool_address.clone(),
                        owner: acct.owner.to_string(),
                        lamports: acct.lamports,
                        data_len: acct.data.len(),
                        data_b64: BASE64_STANDARD.encode(&acct.data),
                        parse_ok,
                        parse_error,
                        token_mint_a: parsed.as_ref().map(|p| p.token_mint0.to_string()),
                        token_mint_b: parsed.as_ref().map(|p| p.token_mint1.to_string()),
                        token_vault_a: parsed.as_ref().map(|p| p.token_vault0.to_string()),
                        token_vault_b: parsed.as_ref().map(|p| p.token_vault1.to_string()),
                        vault_amount_a,
                        vault_amount_b,
                        mint_decimals_a: parsed.as_ref().map(|p| p.mint_decimals0),
                        mint_decimals_b: parsed.as_ref().map(|p| p.mint_decimals1),
                        liquidity_active: parsed.as_ref().map(|p| p.liquidity_active.to_string()),
                        tick_current: parsed.as_ref().map(|p| p.tick_current),
                        sqrt_price_x64: parsed.as_ref().map(|p| p.sqrt_price_x64.to_string()),
                        fee_growth_global_a_x64: parsed
                            .as_ref()
                            .map(|p| p.fee_growth_global0_x64.to_string()),
                        fee_growth_global_b_x64: parsed
                            .as_ref()
                            .map(|p| p.fee_growth_global1_x64.to_string()),
                        protocol_fees_token_a: parsed.as_ref().map(|p| p.protocol_fees_token0),
                        protocol_fees_token_b: parsed.as_ref().map(|p| p.protocol_fees_token1),
                    };

                    let mut dir = std::path::PathBuf::from("data");
                    dir.push("pool-snapshots");
                    dir.push("raydium");
                    dir.push(&pool_address);
                    std::fs::create_dir_all(&dir)?;
                    let mut path = dir;
                    path.push("snapshots.jsonl");

                    let line = serde_json::to_string(&snap)?;
                    let mut f = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&path)?;
                    use std::io::Write;
                    f.write_all(line.as_bytes())?;
                    f.write_all(b"\n")?;

                    println!("âś… Snapshot appended: {}", path.display());
                    Ok(())
                }
                .await;

                match result {
                    Ok(()) => raydium_success += 1,
                    Err(e) => {
                        eprintln!("âťŚ Raydium snapshot failed for {}: {}", pool_address, e);
                        run_errors.push(RunErrorEntry {
                            protocol: "raydium".to_string(),
                            pool_address: pool_address.clone(),
                            error: e.to_string(),
                        });
                    }
                }
            }

            // ---- Meteora raw snapshots ----
            #[derive(Debug, Serialize)]
            struct MeteoraLbPairSnapshot {
                ts_utc: String,
                slot: u64,
                protocol: String,
                pool_address: String,
                owner: String,
                lamports: u64,

                data_len: usize,
                data_b64: String,

                /// Whether the Meteora lb_pair account bytes were decoded successfully.
                parse_ok: bool,
                /// Detailed decode error (only set when `parse_ok=false`).
                #[serde(skip_serializing_if = "Option::is_none")]
                parse_error: Option<String>,

                #[serde(skip_serializing_if = "Option::is_none")]
                active_id: Option<i32>,
                #[serde(skip_serializing_if = "Option::is_none")]
                bin_step: Option<u16>,

                #[serde(skip_serializing_if = "Option::is_none")]
                token_mint_a: Option<String>,
                #[serde(skip_serializing_if = "Option::is_none")]
                token_mint_b: Option<String>,
                #[serde(skip_serializing_if = "Option::is_none")]
                token_vault_a: Option<String>,
                #[serde(skip_serializing_if = "Option::is_none")]
                token_vault_b: Option<String>,

                #[serde(skip_serializing_if = "Option::is_none")]
                vault_amount_a: Option<u64>,
                #[serde(skip_serializing_if = "Option::is_none")]
                vault_amount_b: Option<u64>,

                #[serde(skip_serializing_if = "Option::is_none")]
                protocol_fee_amount_a: Option<u64>,
                #[serde(skip_serializing_if = "Option::is_none")]
                protocol_fee_amount_b: Option<u64>,
            }
            for pool_address in meteora_pool_addrs.into_iter() {
                let result: anyhow::Result<()> = async {
                    let acct = rpc.get_account_by_address(&pool_address).await?;
                    let (parsed, parse_ok, parse_error) =
                        match clmm_lp_protocols::meteora::pool_reader::parse_lb_pair(&acct.data) {
                            Ok(p) => (Some(p), true, None),
                            Err(e) => (None, false, Some(e.to_string())),
                        };
                    let (vault_amount_a, vault_amount_b) = if let Some(ref p) = parsed {
                        use spl_token::solana_program::program_pack::Pack;
                        use spl_token::state::Account as SplTokenAccount;
                        let accounts = rpc
                            .get_multiple_accounts(&[p.reserve_x, p.reserve_y])
                            .await
                            .ok();
                        if let Some(accounts) = accounts {
                            let a = accounts
                                .get(0)
                                .and_then(|a| a.as_ref())
                                .and_then(|a| SplTokenAccount::unpack(&a.data).ok())
                                .map(|a| a.amount);
                            let b = accounts
                                .get(1)
                                .and_then(|a| a.as_ref())
                                .and_then(|a| SplTokenAccount::unpack(&a.data).ok())
                                .map(|a| a.amount);
                            (a, b)
                        } else {
                            (None, None)
                        }
                    } else {
                        (None, None)
                    };
                    let snap = MeteoraLbPairSnapshot {
                        ts_utc: chrono::Utc::now().to_rfc3339(),
                        slot: slot_now,
                        protocol: "meteora".to_string(),
                        pool_address: pool_address.clone(),
                        owner: acct.owner.to_string(),
                        lamports: acct.lamports,
                        data_len: acct.data.len(),
                        data_b64: BASE64_STANDARD.encode(&acct.data),
                        parse_ok,
                        parse_error,
                        active_id: parsed.as_ref().map(|p| p.active_id),
                        bin_step: parsed.as_ref().map(|p| p.bin_step),
                        token_mint_a: parsed.as_ref().map(|p| p.token_mint_x.to_string()),
                        token_mint_b: parsed.as_ref().map(|p| p.token_mint_y.to_string()),
                        token_vault_a: parsed.as_ref().map(|p| p.reserve_x.to_string()),
                        token_vault_b: parsed.as_ref().map(|p| p.reserve_y.to_string()),
                        vault_amount_a,
                        vault_amount_b,
                        protocol_fee_amount_a: parsed.as_ref().map(|p| p.protocol_fee_amount_x),
                        protocol_fee_amount_b: parsed.as_ref().map(|p| p.protocol_fee_amount_y),
                    };

                    let mut dir = std::path::PathBuf::from("data");
                    dir.push("pool-snapshots");
                    dir.push("meteora");
                    dir.push(&pool_address);
                    std::fs::create_dir_all(&dir)?;
                    let mut path = dir;
                    path.push("snapshots.jsonl");

                    let line = serde_json::to_string(&snap)?;
                    let mut f = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&path)?;
                    use std::io::Write;
                    f.write_all(line.as_bytes())?;
                    f.write_all(b"\n")?;

                    println!("âś… Snapshot appended: {}", path.display());
                    Ok(())
                }
                .await;

                match result {
                    Ok(()) => meteora_success += 1,
                    Err(e) => {
                        eprintln!("âťŚ Meteora snapshot failed for {}: {}", pool_address, e);
                        run_errors.push(RunErrorEntry {
                            protocol: "meteora".to_string(),
                            pool_address: pool_address.clone(),
                            error: e.to_string(),
                        });
                    }
                }
            }

            let status = SnapshotRunStatus {
                ts_utc: run_started_at.to_rfc3339(),
                elapsed_ms: run_started_instant.elapsed().as_millis(),
                rpc_slot: slot_now,
                ok: run_errors.is_empty(),
                orca: ProtocolRunStats {
                    target: orca_target,
                    success: orca_success,
                    failed: orca_target.saturating_sub(orca_success),
                },
                raydium: ProtocolRunStats {
                    target: raydium_target,
                    success: raydium_success,
                    failed: raydium_target.saturating_sub(raydium_success),
                },
                meteora: ProtocolRunStats {
                    target: meteora_target,
                    success: meteora_success,
                    failed: meteora_target.saturating_sub(meteora_success),
                },
                errors: run_errors,
            };

            let mut status_dir = std::path::PathBuf::from("data");
            status_dir.push("snapshot_logs");
            std::fs::create_dir_all(&status_dir)?;
            let mut status_path = status_dir;
            status_path.push("snapshot-run-curated-all.jsonl");
            let status_line = serde_json::to_string(&status)?;
            let mut sf = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&status_path)?;
            use std::io::Write;
            sf.write_all(status_line.as_bytes())?;
            sf.write_all(b"\n")?;

            println!(
                "đź“Ś Snapshot run summary: Orca {}/{} | Raydium {}/{} | Meteora {}/{} | errors: {}",
                status.orca.success,
                status.orca.target,
                status.raydium.success,
                status.raydium.target,
                status.meteora.success,
                status.meteora.target,
                status.errors.len()
            );
            println!("đź“ť Run status appended: {}", status_path.display());
        }
        Commands::SwapsSyncCuratedAll {
            limit,
            max_signatures,
            max_pages,
        } => {
            crate::swap_sync::sync_curated_all_robust(*limit, *max_signatures, *max_pages).await?;
        }
        Commands::SwapsSubscribeMentions {
            protocol,
            pool_address,
            mentions,
            mentions_preset,
            max_events,
            idle_timeout_secs,
        } => {
            crate::swap_sync::subscribe_mentions_to_raw(
                protocol,
                pool_address,
                mentions,
                mentions_preset,
                *max_events,
                *idle_timeout_secs,
            )?;
        }
        Commands::SwapsEnrichCuratedAll {
            limit,
            max_decode,
            decode_timeout_secs,
            decode_retries,
            decode_concurrency,
            decode_jitter_ms,
            refresh_decoded,
        } => {
            crate::swap_sync::enrich_curated_all(
                *limit,
                *max_decode,
                *decode_timeout_secs,
                *decode_retries,
                *decode_concurrency,
                *decode_jitter_ms,
                *refresh_decoded,
            )
            .await?;
        }
        Commands::SwapsDecodeAudit { limit, save_report } => {
            crate::swap_sync::decode_audit_curated_all(*limit, *save_report)?;
        }
        Commands::DataHealthCheck {
            max_age_minutes,
            min_decode_ok_pct,
            fail_on_alert,
        } => {
            crate::swap_sync::health_check_curated_all(
                *max_age_minutes,
                *min_decode_ok_pct,
                *fail_on_alert,
            )?;
        }
        Commands::OpsIngestCycle {
            limit,
            run_snapshots,
            swaps_max_signatures,
            swaps_max_pages,
            enrich_max_decode,
            enrich_decode_timeout_secs,
            enrich_decode_retries,
            enrich_decode_concurrency,
            enrich_decode_jitter_ms,
            enrich_refresh_decoded,
            health_max_age_minutes,
            health_min_decode_ok_pct,
            fail_on_alert,
        } => {
            use chrono::Utc;
            use serde::Serialize;
            use std::time::Instant;

            #[derive(Debug, Serialize)]
            struct OpsCycleReport {
                ts_utc: String,
                elapsed_ms: u128,
                limit: Option<usize>,
                run_snapshots: bool,
                swaps_max_signatures: usize,
                swaps_max_pages: usize,
                enrich_max_decode: usize,
                enrich_decode_timeout_secs: u64,
                enrich_decode_retries: usize,
                enrich_decode_concurrency: usize,
                enrich_decode_jitter_ms: u64,
                enrich_refresh_decoded: bool,
                health_max_age_minutes: i64,
                health_min_decode_ok_pct: f64,
                fail_on_alert: bool,
                ok: bool,
            }

            let start = Instant::now();
            let mut ok = true;

            if *run_snapshots {
                println!("🧭 ops: snapshots (curated all)");
                if let Err(e) = snapshots::collector::orca_snapshot_curated(*limit).await {
                    ok = false;
                    println!("⚠️ ops: orca snapshots failed: {}", e);
                }
                if let Err(e) = snapshots::collector::raydium_snapshot_curated(*limit).await {
                    ok = false;
                    println!("⚠️ ops: raydium snapshots failed: {}", e);
                }
                if let Err(e) = snapshots::collector::meteora_snapshot_curated(*limit).await {
                    ok = false;
                    println!("⚠️ ops: meteora snapshots failed: {}", e);
                }
            }

            println!("🧭 ops: swaps sync (curated all)");
            crate::swap_sync::sync_curated_all_robust(
                *limit,
                *swaps_max_signatures,
                *swaps_max_pages,
            )
            .await?;

            println!("🧭 ops: swaps enrich (curated all)");
            crate::swap_sync::enrich_curated_all(
                *limit,
                *enrich_max_decode,
                *enrich_decode_timeout_secs,
                *enrich_decode_retries,
                *enrich_decode_concurrency,
                *enrich_decode_jitter_ms,
                *enrich_refresh_decoded,
            )
            .await?;

            println!("🧭 ops: decode audit (curated all)");
            crate::swap_sync::decode_audit_curated_all(*limit, true)?;

            println!("🧭 ops: health check");
            // This can bail when `fail_on_alert=true`.
            crate::swap_sync::health_check_curated_all(
                *health_max_age_minutes,
                *health_min_decode_ok_pct,
                *fail_on_alert,
            )?;

            let elapsed_ms = start.elapsed().as_millis();
            let report = OpsCycleReport {
                ts_utc: Utc::now().to_rfc3339(),
                elapsed_ms,
                limit: *limit,
                run_snapshots: *run_snapshots,
                swaps_max_signatures: *swaps_max_signatures,
                swaps_max_pages: *swaps_max_pages,
                enrich_max_decode: *enrich_max_decode,
                enrich_decode_timeout_secs: *enrich_decode_timeout_secs,
                enrich_decode_retries: *enrich_decode_retries,
                enrich_decode_concurrency: *enrich_decode_concurrency,
                enrich_decode_jitter_ms: *enrich_decode_jitter_ms,
                enrich_refresh_decoded: *enrich_refresh_decoded,
                health_max_age_minutes: *health_max_age_minutes,
                health_min_decode_ok_pct: *health_min_decode_ok_pct,
                fail_on_alert: *fail_on_alert,
                ok,
            };

            let ts = Utc::now().format("%Y%m%d_%H%M%S");
            let out_dir = std::path::Path::new("data").join("reports");
            std::fs::create_dir_all(&out_dir)?;
            let out = out_dir.join(format!("ops_ingest_cycle_{}.json", ts));
            std::fs::write(&out, serde_json::to_string_pretty(&report)?)?;
            println!("📝 ops cycle report saved: {}", out.display());

            if !report.ok {
                // Snapshot failures are non-fatal by default (we still ran the pipeline),
                // but mark a non-zero exit for automation visibility.
                anyhow::bail!("ops cycle finished with snapshot errors (see report)");
            }
        }
        Commands::OpsIngestLoop {
            limit,
            run_snapshots,
            swaps_max_signatures,
            swaps_max_pages,
            enrich_max_decode,
            enrich_decode_timeout_secs,
            enrich_decode_retries,
            enrich_decode_concurrency,
            enrich_decode_jitter_ms,
            enrich_refresh_decoded,
            health_max_age_minutes,
            health_min_decode_ok_pct,
            fail_on_alert,
            interval_secs,
            jitter_secs,
            error_backoff_base_secs,
            max_cycles,
        } => {
            use std::time::Duration;
            use tokio::time::sleep;

            println!(
                "🔁 ops-ingest-loop: interval={}s jitter=0..{}s max_cycles={:?}",
                interval_secs, jitter_secs, max_cycles
            );

            let mut cycles: u64 = 0;
            let mut consecutive_failures: u64 = 0;
            loop {
                if let Some(m) = *max_cycles {
                    if cycles >= m {
                        println!("✅ ops-ingest-loop: reached max_cycles={}", m);
                        break;
                    }
                }
                cycles += 1;
                println!("🧭 ops-ingest-loop: cycle {} start", cycles);

                let res = async {
                    // Inline the same sequence as OpsIngestCycle (but keep loop behavior here).
                    if *run_snapshots {
                        println!("🧭 ops: snapshots (curated all)");
                        snapshots::collector::orca_snapshot_curated(*limit).await?;
                        snapshots::collector::raydium_snapshot_curated(*limit).await?;
                        snapshots::collector::meteora_snapshot_curated(*limit).await?;
                    }
                    println!("🧭 ops: swaps sync (curated all)");
                    crate::swap_sync::sync_curated_all_robust(
                        *limit,
                        *swaps_max_signatures,
                        *swaps_max_pages,
                    )
                    .await?;
                    println!("🧭 ops: swaps enrich (curated all)");
                    crate::swap_sync::enrich_curated_all(
                        *limit,
                        *enrich_max_decode,
                        *enrich_decode_timeout_secs,
                        *enrich_decode_retries,
                        *enrich_decode_concurrency,
                        *enrich_decode_jitter_ms,
                        *enrich_refresh_decoded,
                    )
                    .await?;
                    println!("🧭 ops: decode audit (curated all)");
                    crate::swap_sync::decode_audit_curated_all(*limit, true)?;
                    println!("🧭 ops: health check");
                    crate::swap_sync::health_check_curated_all(
                        *health_max_age_minutes,
                        *health_min_decode_ok_pct,
                        *fail_on_alert,
                    )?;
                    Ok::<(), anyhow::Error>(())
                }
                .await;

                match res {
                    Ok(()) => {
                        consecutive_failures = 0;
                        println!("✅ ops-ingest-loop: cycle {} ok", cycles);
                    }
                    Err(e) => {
                        consecutive_failures = consecutive_failures.saturating_add(1);
                        println!(
                            "⚠️ ops-ingest-loop: cycle {} failed (failures={}): {}",
                            cycles, consecutive_failures, e
                        );
                        // If health-check is strict, failing is expected behavior for automation.
                        // Otherwise, continue with backoff.
                    }
                }

                let jitter = if *jitter_secs == 0 {
                    0u64
                } else {
                    rand::random::<u64>() % (*jitter_secs + 1)
                };
                let base_sleep = interval_secs.saturating_add(jitter);
                let backoff = if consecutive_failures == 0 {
                    0
                } else {
                    error_backoff_base_secs.saturating_mul(1 + consecutive_failures)
                };
                let sleep_secs = base_sleep.saturating_add(backoff);
                println!(
                    "⏳ ops-ingest-loop: sleeping {}s (base={}s jitter={}s backoff={}s)",
                    sleep_secs, interval_secs, jitter, backoff
                );
                sleep(Duration::from_secs(sleep_secs.max(1))).await;
            }
        }
        Commands::OrcaBotRun {
            position,
            keypair,
            execute,
            eval_interval_secs,
            poll_interval_secs,
            optimize_result_json,
        } => {
            crate::commands::orca_bot::run_orca_bot(
                position.clone(),
                keypair.clone(),
                *execute,
                *eval_interval_secs,
                *poll_interval_secs,
                optimize_result_json.clone(),
            )
            .await?;
        }
        Commands::OrcaPositionOpen {
            pool,
            keypair,
            dry_run,
            tick_lower,
            tick_upper,
            range_width_pct,
            slippage_bps,
        } => {
            if !*dry_run {
                let _ = crate::commands::orca_wallet::load_signing_wallet(keypair.clone())
                    .context("signing key: --keypair, KEYPAIR_PATH, or SOLANA_KEYPAIR")?;
            }
            crate::commands::orca_position::run_position_open(
                pool.clone(),
                keypair.clone(),
                *dry_run,
                *tick_lower,
                *tick_upper,
                *range_width_pct,
                *slippage_bps,
            )
            .await?;
        }
        Commands::OrcaPositionDecrease {
            position,
            keypair,
            dry_run,
            liquidity_pct,
            liquidity,
        } => {
            if !*dry_run {
                let _ = crate::commands::orca_wallet::load_signing_wallet(keypair.clone())
                    .context("signing key: --keypair, KEYPAIR_PATH, or SOLANA_KEYPAIR")?;
            }
            crate::commands::orca_position::run_position_decrease(
                position.clone(),
                keypair.clone(),
                *dry_run,
                *liquidity_pct,
                *liquidity,
            )
            .await?;
        }
        Commands::SnapshotStatusLast => {
            use serde_json::Value;

            let path = std::path::Path::new("data")
                .join("snapshot_logs")
                .join("snapshot-run-curated-all.jsonl");
            if !path.exists() {
                println!("No snapshot status log found at {}", path.display());
                println!("Run `snapshot-run-curated-all` first.");
                return Ok(());
            }

            let content = std::fs::read_to_string(&path)?;
            let Some(last_line) = content.lines().rev().find(|l| !l.trim().is_empty()) else {
                println!("Status log exists but is empty: {}", path.display());
                return Ok(());
            };

            let v: Value = serde_json::from_str(last_line)?;

            let ts = v.get("ts_utc").and_then(|x| x.as_str()).unwrap_or("n/a");
            let elapsed = v.get("elapsed_ms").and_then(|x| x.as_u64()).unwrap_or(0);
            let slot = v.get("rpc_slot").and_then(|x| x.as_u64()).unwrap_or(0);
            let ok = v.get("ok").and_then(|x| x.as_bool()).unwrap_or(false);

            let stat = |name: &str| -> (u64, u64, u64) {
                let obj = v.get(name).and_then(|x| x.as_object());
                let target = obj
                    .and_then(|o| o.get("target"))
                    .and_then(|x| x.as_u64())
                    .unwrap_or(0);
                let success = obj
                    .and_then(|o| o.get("success"))
                    .and_then(|x| x.as_u64())
                    .unwrap_or(0);
                let failed = obj
                    .and_then(|o| o.get("failed"))
                    .and_then(|x| x.as_u64())
                    .unwrap_or(0);
                (target, success, failed)
            };

            let (orca_t, orca_s, orca_f) = stat("orca");
            let (ray_t, ray_s, ray_f) = stat("raydium");
            let (met_t, met_s, met_f) = stat("meteora");

            let errors = v
                .get("errors")
                .and_then(|x| x.as_array())
                .cloned()
                .unwrap_or_default();

            println!("Snapshot run status (last):");
            println!("  ts_utc: {}", ts);
            println!(
                "  ok: {}   elapsed_ms: {}   rpc_slot: {}",
                ok, elapsed, slot
            );
            println!(
                "  orca:    success {}/{}  failed {}",
                orca_s, orca_t, orca_f
            );
            println!("  raydium: success {}/{}  failed {}", ray_s, ray_t, ray_f);
            println!("  meteora: success {}/{}  failed {}", met_s, met_t, met_f);
            println!("  errors: {}", errors.len());

            for e in errors.iter().take(5) {
                let protocol = e.get("protocol").and_then(|x| x.as_str()).unwrap_or("?");
                let pool = e
                    .get("pool_address")
                    .and_then(|x| x.as_str())
                    .unwrap_or("?");
                let err = e.get("error").and_then(|x| x.as_str()).unwrap_or("?");
                println!("    - [{}] {} -> {}", protocol, pool, err);
            }
            if errors.len() > 5 {
                println!("    ... and {} more errors", errors.len() - 5);
            }
        }
        Commands::SnapshotReadiness {
            protocol,
            pool_address,
        } => {
            let proto_dir = match protocol {
                SnapshotProtocolArg::Orca => "orca",
                SnapshotProtocolArg::Raydium => "raydium",
                SnapshotProtocolArg::Meteora => "meteora",
            };
            let path = std::path::Path::new("data")
                .join("pool-snapshots")
                .join(proto_dir)
                .join(pool_address)
                .join("snapshots.jsonl");
            if !path.exists() {
                println!("No snapshot file found: {}", path.display());
                return Ok(());
            }

            let txt = std::fs::read_to_string(&path)?;
            let lines: Vec<&str> = txt.lines().filter(|l| !l.trim().is_empty()).collect();
            if lines.is_empty() {
                println!("Snapshot file is empty: {}", path.display());
                return Ok(());
            }

            let mut with_ts = 0usize;
            let mut with_vaults = 0usize;
            let mut with_mints = 0usize;
            let mut with_liquidity = 0usize;
            let mut with_fee_growth = 0usize;
            let mut with_protocol_fee_counter = 0usize;
            let mut with_decimals = 0usize;
            let mut with_parse_ok = 0usize;
            let mut with_parse_error = 0usize;

            for line in &lines {
                let v: serde_json::Value = match serde_json::from_str(line) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if v.get("ts_utc").and_then(|x| x.as_str()).is_some() {
                    with_ts += 1;
                }
                if v.get("vault_amount_a").and_then(|x| x.as_u64()).is_some()
                    && v.get("vault_amount_b").and_then(|x| x.as_u64()).is_some()
                {
                    with_vaults += 1;
                }
                if v.get("token_mint_a").and_then(|x| x.as_str()).is_some()
                    && v.get("token_mint_b").and_then(|x| x.as_str()).is_some()
                {
                    with_mints += 1;
                }
                if v.get("liquidity_active").is_some() {
                    with_liquidity += 1;
                }

                if v.get("parse_ok").and_then(|x| x.as_bool()).unwrap_or(false) {
                    with_parse_ok += 1;
                }
                if v.get("parse_error").is_some() {
                    with_parse_error += 1;
                }
                let has_fee_growth = match protocol {
                    SnapshotProtocolArg::Orca => {
                        v.get("fee_growth_global_a").is_some()
                            && v.get("fee_growth_global_b").is_some()
                    }
                    SnapshotProtocolArg::Raydium => {
                        v.get("fee_growth_global_a_x64").is_some()
                            && v.get("fee_growth_global_b_x64").is_some()
                    }
                    SnapshotProtocolArg::Meteora => false,
                };
                if has_fee_growth {
                    with_fee_growth += 1;
                }
                let has_protocol_fee_counter = match protocol {
                    SnapshotProtocolArg::Orca => {
                        v.get("protocol_fee_owed_a").is_some()
                            && v.get("protocol_fee_owed_b").is_some()
                    }
                    SnapshotProtocolArg::Raydium => {
                        v.get("protocol_fees_token_a").is_some()
                            && v.get("protocol_fees_token_b").is_some()
                    }
                    SnapshotProtocolArg::Meteora => {
                        v.get("protocol_fee_amount_a").is_some()
                            && v.get("protocol_fee_amount_b").is_some()
                    }
                };
                if has_protocol_fee_counter {
                    with_protocol_fee_counter += 1;
                }
                if v.get("mint_decimals_a").is_some() && v.get("mint_decimals_b").is_some() {
                    with_decimals += 1;
                }
            }

            let total = lines.len();
            let pct = |n: usize| -> f64 { (n as f64) * 100.0 / (total as f64) };

            let lp_share_ready = with_ts >= 2 && with_vaults >= 2 && with_mints >= 2;
            let snapshot_fee_heuristic_ready = with_ts >= 2
                && with_mints >= 2
                && (with_fee_growth >= 2 || with_protocol_fee_counter >= 2);
            let position_truth_ready = false; // requires position-level state (inside-growth checkpoints + position liquidity/range history)

            println!("Snapshot readiness audit:");
            println!("  protocol: {:?}", protocol);
            println!("  pool: {}", pool_address);
            println!("  file: {}", path.display());
            println!("  rows: {}", total);
            println!(
                "  coverage: ts={} ({:.1}%), vaults={} ({:.1}%), mints={} ({:.1}%), liquidity={} ({:.1}%), fee_growth={} ({:.1}%), protocol_fee_counter={} ({:.1}%), decimals={} ({:.1}%)",
                with_ts,
                pct(with_ts),
                with_vaults,
                pct(with_vaults),
                with_mints,
                pct(with_mints),
                with_liquidity,
                pct(with_liquidity),
                with_fee_growth,
                pct(with_fee_growth),
                with_protocol_fee_counter,
                pct(with_protocol_fee_counter),
                with_decimals,
                pct(with_decimals)
            );
            println!(
                "  parse: parse_ok={} ({:.1}%), parse_error={} ({:.1}%)",
                with_parse_ok,
                pct(with_parse_ok),
                with_parse_error,
                pct(with_parse_error)
            );
            println!();
            println!("Readiness tiers:");
            println!(
                "  1) LP-share (capital/TVL proxy): {}",
                if lp_share_ready { "READY" } else { "NOT READY" }
            );
            println!(
                "  2) Snapshot fee heuristic (experimental): {}",
                if snapshot_fee_heuristic_ready {
                    "READY"
                } else {
                    "NOT READY"
                }
            );
            println!(
                "  3) Position-truth fee model (fee_growth_inside + position history): {}",
                if position_truth_ready {
                    "READY"
                } else {
                    "NOT READY"
                }
            );
            if !position_truth_ready {
                println!(
                    "     Missing: position range+liquidity timeline and inside-growth accounting checkpoints."
                );
            }
        }
        Commands::DexscreenerSearch { query, limit } => {
            use clmm_lp_data::providers::{DexChain, DexscreenerClient};

            let client = DexscreenerClient::new();
            let mut pairs = client.search(query).await?;
            pairs.retain(|p| p.chain_id.eq_ignore_ascii_case(DexChain::Solana.as_str()));

            // Sort by 24h volume, then liquidity
            pairs.sort_by(|a, b| {
                b.volume
                    .h24
                    .partial_cmp(&a.volume.h24)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| {
                        b.liquidity
                            .usd
                            .partial_cmp(&a.liquidity.usd)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
            });

            let mut t = Table::new();
            t.add_row(row![
                "chain",
                "dex",
                "pair_address",
                "base",
                "quote",
                "liq_usd",
                "vol_24h",
                "tx_24h"
            ]);
            for p in pairs.into_iter().take(*limit) {
                let tx24 = p.txns.h24.buys + p.txns.h24.sells;
                t.add_row(row![
                    p.chain_id,
                    p.dex_id,
                    p.pair_address,
                    format!("{} ({})", p.base_token.symbol, p.base_token.address),
                    format!("{} ({})", p.quote_token.symbol, p.quote_token.address),
                    format!("{:.0}", p.liquidity.usd),
                    format!("{:.0}", p.volume.h24),
                    tx24
                ]);
            }
            t.printstd();
        }
        Commands::DexscreenerTokenPairs { token_mint, limit } => {
            use clmm_lp_data::providers::{DexChain, DexscreenerClient};

            let client = DexscreenerClient::new();
            let mut pairs = client.token_pairs(DexChain::Solana, token_mint).await?;
            pairs.sort_by(|a, b| {
                b.volume
                    .h24
                    .partial_cmp(&a.volume.h24)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| {
                        b.liquidity
                            .usd
                            .partial_cmp(&a.liquidity.usd)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
            });

            let mut t = Table::new();
            t.add_row(row![
                "dex",
                "pair_address",
                "base",
                "quote",
                "liq_usd",
                "vol_24h",
                "tx_24h"
            ]);
            for p in pairs.into_iter().take(*limit) {
                let tx24 = p.txns.h24.buys + p.txns.h24.sells;
                t.add_row(row![
                    p.dex_id,
                    p.pair_address,
                    format!("{} ({})", p.base_token.symbol, p.base_token.address),
                    format!("{} ({})", p.quote_token.symbol, p.quote_token.address),
                    format!("{:.0}", p.liquidity.usd),
                    format!("{:.0}", p.volume.h24),
                    tx24
                ]);
            }
            t.printstd();
        }
        Commands::DexscreenerPair { pair_address } => {
            use clmm_lp_data::providers::{DexChain, DexscreenerClient};

            let client = DexscreenerClient::new();
            let pairs = client.pair(DexChain::Solana, pair_address).await?;
            let mut t = Table::new();
            t.add_row(row![
                "dex",
                "pair_address",
                "base",
                "quote",
                "liq_usd",
                "vol_24h",
                "vol_6h",
                "vol_1h",
                "vol_5m",
                "tx_24h"
            ]);
            for p in pairs {
                let tx24 = p.txns.h24.buys + p.txns.h24.sells;
                t.add_row(row![
                    p.dex_id,
                    p.pair_address,
                    format!("{} ({})", p.base_token.symbol, p.base_token.address),
                    format!("{} ({})", p.quote_token.symbol, p.quote_token.address),
                    format!("{:.0}", p.liquidity.usd),
                    format!("{:.0}", p.volume.h24),
                    format!("{:.0}", p.volume.h6),
                    format!("{:.0}", p.volume.h1),
                    format!("{:.0}", p.volume.m5),
                    tx24
                ]);
            }
            t.printstd();
        }
        Commands::DexscreenerComparePair {
            mint_a,
            mint_b,
            sort_by,
            limit,
        } => {
            use clmm_lp_data::providers::{DexChain, DexPair, DexscreenerClient};

            fn matches_pair(p: &DexPair, a: &str, b: &str) -> bool {
                let ba = p.base_token.address.eq_ignore_ascii_case(a)
                    && p.quote_token.address.eq_ignore_ascii_case(b);
                let ab = p.base_token.address.eq_ignore_ascii_case(b)
                    && p.quote_token.address.eq_ignore_ascii_case(a);
                ba || ab
            }

            let client = DexscreenerClient::new();
            let mut pairs = client.token_pairs(DexChain::Solana, mint_a).await?;
            pairs.retain(|p| matches_pair(p, mint_a, mint_b));

            let key = sort_by.trim().to_lowercase();
            pairs.sort_by(|a, b| {
                let f = |p: &DexPair| -> f64 {
                    match key.as_str() {
                        "liquidity_usd" => p.liquidity.usd,
                        "volume_h6" => p.volume.h6,
                        "volume_h1" => p.volume.h1,
                        "volume_m5" => p.volume.m5,
                        _ => p.volume.h24, // default volume_h24
                    }
                };
                f(b).partial_cmp(&f(a)).unwrap_or(std::cmp::Ordering::Equal)
            });

            let mut t = Table::new();
            t.add_row(row![
                "dex",
                "pair_address",
                "base",
                "quote",
                "liq_usd",
                "vol_24h",
                "vol_6h",
                "vol_1h",
                "vol_5m",
                "tx_24h"
            ]);
            for p in pairs.into_iter().take(*limit) {
                let tx24 = p.txns.h24.buys + p.txns.h24.sells;
                t.add_row(row![
                    p.dex_id,
                    p.pair_address,
                    format!("{} ({})", p.base_token.symbol, p.base_token.address),
                    format!("{} ({})", p.quote_token.symbol, p.quote_token.address),
                    format!("{:.0}", p.liquidity.usd),
                    format!("{:.0}", p.volume.h24),
                    format!("{:.0}", p.volume.h6),
                    format!("{:.0}", p.volume.h1),
                    format!("{:.0}", p.volume.m5),
                    tx24
                ]);
            }
            t.printstd();
        }
        Commands::StudioStreamPlan {
            input_jsonl,
            output_jsonl,
            lang,
            style,
            pause_secs,
            limit,
        } => {
            let lang = match lang.as_str() {
                "pl" | "PL" => crate::commands::studio::StudioLang::Pl,
                "en" | "EN" => crate::commands::studio::StudioLang::En,
                other => anyhow::bail!("unsupported --lang {other} (use pl|en)"),
            };
            crate::commands::studio::run_studio_stream_plan(
                input_jsonl.clone(),
                output_jsonl.clone(),
                lang,
                style.clone(),
                *pause_secs,
                *limit,
            )?;
        }
    }

    Ok(())
}

/// Calculates annualized volatility from price series.
fn calculate_volatility(prices: &[f64]) -> f64 {
    if prices.len() < 2 {
        return 0.0;
    }

    // Calculate log returns
    let returns: Vec<f64> = prices.windows(2).map(|w| (w[1] / w[0]).ln()).collect();

    if returns.is_empty() {
        return 0.0;
    }

    // Calculate standard deviation
    let mean = returns.iter().sum::<f64>() / returns.len() as f64;
    let variance = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / returns.len() as f64;
    let std_dev = variance.sqrt();

    // Annualize (assuming hourly data, ~8760 hours/year)
    std_dev * (8760.0_f64).sqrt()
}

/// Prints a rich backtest report using prettytable.
#[allow(clippy::too_many_arguments)]
fn print_backtest_report(
    pair: &str,
    days: u64,
    capital: f64,
    entry_price: Decimal,
    final_price: Decimal,
    lower: f64,
    upper: f64,
    summary: &TrackerSummary,
    strategy: StrategyArg,
) {
    let price_change_pct =
        ((final_price - entry_price) / entry_price * Decimal::from(100)).round_dp(2);
    let return_pct =
        (summary.final_pnl / Decimal::from_f64(capital).unwrap() * Decimal::from(100)).round_dp(2);
    let vs_hodl_pct = if summary.hodl_value != Decimal::ZERO {
        (summary.vs_hodl / summary.hodl_value * Decimal::from(100)).round_dp(2)
    } else {
        Decimal::ZERO
    };

    println!();
    println!("đź“Š BACKTEST RESULTS: {}", pair);
    println!("Period: {} days | Strategy: {:?}", days, strategy);
    println!();

    // Position Configuration Table
    let mut config_table = Table::new();
    config_table.add_row(row!["POSITION CONFIGURATION", ""]);
    config_table.add_row(row![
        "Price Range",
        format!("${:.2} - ${:.2}", lower, upper)
    ]);
    config_table.add_row(row!["Entry Price", format!("${:.4}", entry_price)]);
    config_table.add_row(row![
        "Final Price",
        format!("${:.4} ({:+.2}%)", final_price, price_change_pct)
    ]);
    config_table.add_row(row!["Initial Capital", format!("${:.2}", capital)]);
    config_table.printstd();

    println!();

    // Performance Metrics Table
    let mut perf_table = Table::new();
    perf_table.add_row(row!["PERFORMANCE METRICS", ""]);
    perf_table.add_row(row!["Final Value", format!("${:.2}", summary.final_value)]);
    perf_table.add_row(row![
        "Net PnL",
        format!("${:+.2} ({:+.2}%)", summary.final_pnl, return_pct)
    ]);
    perf_table.add_row(row!["Fees Earned", format!("${:.2}", summary.total_fees)]);
    perf_table.add_row(row![
        "IL vs HODL (ex-fees)",
        format!(
            "{:.2}%",
            summary.final_il_vs_hodl_ex_fees_pct * Decimal::from(100)
        )
    ]);
    perf_table.add_row(row![
        "IL Segment (last)",
        summary
            .final_il_segment_pct
            .map(|v| format!("{:.2}%", v * Decimal::from(100)))
            .unwrap_or_else(|| "n/a".to_string())
    ]);
    perf_table.printstd();

    println!();

    // Risk Metrics Table
    let mut risk_table = Table::new();
    risk_table.add_row(row!["RISK METRICS", ""]);
    risk_table.add_row(row![
        "Time in Range",
        format!("{:.1}%", summary.time_in_range_pct * Decimal::from(100))
    ]);
    risk_table.add_row(row![
        "Max Drawdown",
        format!("{:.2}%", summary.max_drawdown * Decimal::from(100))
    ]);
    risk_table.add_row(row![
        "Rebalances",
        format!(
            "{} (cost: ${:.2})",
            summary.rebalance_count, summary.total_rebalance_cost
        )
    ]);
    risk_table.printstd();

    println!();

    // Comparison Table
    let mut comp_table = Table::new();
    comp_table.add_row(row!["COMPARISON vs HODL", ""]);
    comp_table.add_row(row!["HODL Value", format!("${:.2}", summary.hodl_value)]);
    comp_table.add_row(row![
        "LP vs HODL",
        format!("${:+.2} ({:+.2}%)", summary.vs_hodl, vs_hodl_pct)
    ]);
    comp_table.printstd();

    println!();
}

/// Prints optimization results using prettytable.
fn print_optimization_report(
    symbol: &str,
    current_price: f64,
    volatility: f64,
    capital: f64,
    result: &OptimizationResult,
) {
    let lower = result.recommended_range.lower_price.value;
    let upper = result.recommended_range.upper_price.value;
    let width_pct = ((upper - lower) / Decimal::from_f64(current_price).unwrap()
        * Decimal::from(100))
    .round_dp(1);

    println!();
    println!("đźŽŻ OPTIMIZATION RESULTS: {}/USDC", symbol);
    println!();

    // Market Conditions Table
    let mut market_table = Table::new();
    market_table.add_row(row!["MARKET CONDITIONS", ""]);
    market_table.add_row(row!["Current Price", format!("${:.4}", current_price)]);
    market_table.add_row(row![
        "Volatility (annualized)",
        format!("{:.1}%", volatility * 100.0)
    ]);
    market_table.add_row(row!["Capital", format!("${:.2}", capital)]);
    market_table.printstd();

    println!();

    // Recommended Range Table
    let mut range_table = Table::new();
    range_table.add_row(row!["RECOMMENDED RANGE", ""]);
    range_table.add_row(row!["Lower Bound", format!("${:.4}", lower)]);
    range_table.add_row(row!["Upper Bound", format!("${:.4}", upper)]);
    range_table.add_row(row!["Range Width", format!("{}%", width_pct)]);
    range_table.printstd();

    println!();

    // Expected Performance Table
    let mut perf_table = Table::new();
    perf_table.add_row(row!["EXPECTED PERFORMANCE", ""]);
    perf_table.add_row(row![
        "Expected PnL",
        format!("${:+.4}", result.expected_pnl)
    ]);
    perf_table.add_row(row![
        "Expected Fees",
        format!("${:.4}", result.expected_fees)
    ]);
    perf_table.add_row(row!["Expected IL", format!("${:.4}", result.expected_il)]);
    if let Some(sharpe) = result.sharpe_ratio {
        perf_table.add_row(row!["Sharpe Ratio", format!("{:.2}", sharpe)]);
    }
    perf_table.printstd();

    println!();
    println!("đź’ˇ Tip: Use these bounds with the backtest command:");
    println!(
        "   clmm-lp-cli backtest --lower {:.2} --upper {:.2}",
        lower, upper
    );
    println!();
}
