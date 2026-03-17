//! Command Line Interface for the CLMM Liquidity Provider.

pub mod backtest_engine;
pub mod commands;
pub mod engine;
pub mod output;

use anyhow::Result;
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
#[command(about = "CLMM Liquidity Provider Strategy Optimizer CLI", long_about = None)]
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
    /// Maximize Sharpe ratio (risk-adjusted returns)
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
    /// Composite: fees - alpha*|IL|*capital - rebalance_cost (use --alpha)
    #[value(alias = "composite")]
    Composite,
    /// Risk-adjusted: PnL / (1 + max_drawdown)
    #[value(alias = "risk_adj")]
    RiskAdj,
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

        /// Optional Whirlpool pool address to use real Dune volume data
        #[arg(long)]
        whirlpool_address: Option<String>,

        /// Optional fixed LP share of the pool (e.g. 0.001 = 0.1%)
        #[arg(long)]
        lp_share: Option<f64>,
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
        /// Initial capital in USD
        #[arg(long, default_value_t = 7000.0)]
        capital: f64,
        /// Transaction cost per rebalance in USD
        #[arg(long, default_value_t = 0.1)]
        tx_cost: f64,
        /// Optional Whirlpool pool address for Dune TVL/volume
        #[arg(long)]
        whirlpool_address: Option<String>,
        /// Optional fixed LP share (e.g. 0.0001 = 0.01%)
        #[arg(long)]
        lp_share: Option<f64>,
        /// Objective to maximize: pnl or vs_hodl
        #[arg(long, value_enum, default_value_t = BacktestObjectiveArg::VsHodl)]
        objective: BacktestObjectiveArg,
        /// Number of range widths to try (from min to max). E.g. 10 → 1%, 2%, ..., 10%
        #[arg(long, default_value_t = 10)]
        range_steps: usize,
        /// Minimum range width in percent (e.g. 1 = 1%)
        #[arg(long, default_value_t = 1.0)]
        min_range_pct: f64,
        /// Maximum range width in percent (e.g. 15 = 15%)
        #[arg(long, default_value_t = 15.0)]
        max_range_pct: f64,
        /// Show top N results (0 = only best)
        #[arg(long, default_value_t = 5)]
        top_n: usize,
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
    DunePoolMetrics {
        /// Whirlpool pool address
        #[arg(long)]
        pool_address: String,
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

            info!("📡 Initializing Birdeye Provider...");
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
                "🔍 Fetching data for {}/USDC from {} to {}...",
                symbol_a, start_time, now
            );

            // Fetch 1-hour candles
            let candles = provider
                .get_price_history(
                    &token_a, &token_b, start_time, now, 3600, // 1h resolution
                )
                .await?;

            println!("✅ Fetched {} candles:", candles.len());
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
            lower,
            upper,
            capital,
            strategy,
            rebalance_interval,
            threshold_pct,
            tx_cost,
            whirlpool_address,
            lp_share,
        } => {
            let api_key = env::var("BIRDEYE_API_KEY")
                .expect("BIRDEYE_API_KEY must be set in .env or environment");

            println!("📡 Initializing Backtest Engine...");
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

            let (token_b, use_cross_pair) = if let (Some(sb), Some(mb)) = (symbol_b.as_ref(), mint_b.as_ref()) {
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

            let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
            let start_time = now - (days * 24 * 3600);

            if use_cross_pair {
                println!(
                    "🔍 Fetching historical data for {}/{} ({} days)...",
                    symbol_a,
                    symbol_b.as_deref().unwrap_or("UNKNOWN"),
                    days
                );
            } else {
                println!(
                    "🔍 Fetching historical data for {}/USDC ({} days)...",
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
                println!("❌ No data found for the specified period.");
                return Ok(());
            }

            // Optional: load real daily USD TVL & volume from Dune for a given Whirlpool pool.
            let (dune_daily_tvl, dune_daily_volume): (Option<HashMap<String, Decimal>>, Option<HashMap<String, Decimal>>) =
                if let Some(pool) = whirlpool_address {
                    use clmm_lp_data::providers::DuneClient;
                    println!("📡 Fetching daily TVL & volume from Dune for pool {pool}...");
                    let dune = DuneClient::from_env()?;
                    let (tvl_map, vol_map) = dune.fetch_tvl_volume_maps(pool).await?;
                    if tvl_map.is_empty() || vol_map.is_empty() {
                        println!("⚠️ Missing Dune TVL or volume data for this pool, falling back to synthetic model.");
                        (None, None)
                    } else {
                        (Some(tvl_map), Some(vol_map))
                    }
                } else {
                    (None, None)
                };

            // Prepare Price Path
            let prices: Vec<Price> = candles.iter().map(|c| c.close).collect();
            let entry_price = prices.first().cloned().unwrap_or(Price::new(Decimal::ONE));
            let final_price = prices.last().cloned().unwrap_or(entry_price);

            // Setup position tracker
            let initial_range = PriceRange::new(
                Price::new(Decimal::from_f64(*lower).unwrap()),
                Price::new(Decimal::from_f64(*upper).unwrap()),
            );
            let capital_dec = Decimal::from_f64(*capital).unwrap();
            let tx_cost_dec = Decimal::from_f64(*tx_cost).unwrap();

            let mut tracker =
                PositionTracker::new(capital_dec, entry_price, initial_range, tx_cost_dec);

            // Setup synthetic fallback model (when no Dune data is available)
            let mut volume_model = ConstantVolume::from_amount(
                Amount::new(U256::from(1_000_000_000_000u64), 6), // 1M USDC vol per step
            );
            let fee_rate = Decimal::from_f64(0.003).unwrap();
            let lp_share_override: Option<Decimal> = lp_share
                .and_then(|s| Decimal::from_f64(s))
                .filter(|s| *s > Decimal::ZERO && *s <= Decimal::ONE);

            println!(
                "🚀 Running backtest with {:?} strategy over {} steps...",
                strategy,
                prices.len()
            );

            // Run simulation with strategy
            let range_width_pct =
                Decimal::from_f64((*upper - *lower) / ((*upper + *lower) / 2.0)).unwrap();

            for (idx, price) in prices.iter().enumerate() {
                // Per-step USD volume and LP share.
                // If Dune data is available, we use daily TVL/volume for that date.
                // Otherwise we fall back to a synthetic 1% share & constant volume.
                let (step_volume_usd, lp_share_effective) = if let (Some(ref tvl_map), Some(ref vol_map)) =
                    (dune_daily_tvl.as_ref(), dune_daily_volume.as_ref())
                {
                    // Group by date (YYYY-MM-DD)
                    let candle = &candles[idx];
                    let datetime =
                        chrono::DateTime::from_timestamp(candle.start_timestamp as i64, 0)
                            .unwrap_or_default();
                    let date_key = datetime.format("%Y-%m-%d").to_string();

                    let daily_tvl = tvl_map.get(&date_key).cloned().unwrap_or(Decimal::ZERO);
                    let daily_vol = vol_map.get(&date_key).cloned().unwrap_or(Decimal::ZERO);

                    // Avoid division by zero; if TVL is missing, drop back to synthetic model.
                    if daily_tvl.is_zero() || daily_vol.is_zero() {
                        let vol = volume_model.next_volume().to_decimal();
                        let share = lp_share_override
                            .unwrap_or_else(|| Decimal::from_f64(0.01).unwrap()); // default 1%
                        (vol, share)
                    } else {
                        // Effective share: override from CLI if provided, otherwise capital / TVL.
                        let share = if let Some(s) = lp_share_override {
                            s
                        } else {
                            let capital_dec = Decimal::from_f64(*capital).unwrap();
                            (capital_dec / daily_tvl)
                                .min(Decimal::ONE)
                                .max(Decimal::ZERO)
                        };

                        // Per-step volume: distribute daily volume evenly over the day for now.
                        let steps_per_day = Decimal::from_f64(24.0).unwrap();
                        let step_vol = daily_vol / steps_per_day;
                        (step_vol, share)
                    }
                } else {
                    let vol = volume_model.next_volume().to_decimal();
                    let share = lp_share_override
                        .unwrap_or_else(|| Decimal::from_f64(0.01).unwrap()); // default 1%
                    (vol, share)
                };

                // Calculate fees for this step
                let in_range = price.value >= tracker.current_range.lower_price.value
                    && price.value <= tracker.current_range.upper_price.value;

                let step_fees = if in_range {
                    step_volume_usd * lp_share_effective * fee_rate
                } else {
                    Decimal::ZERO
                };

                // Apply strategy
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

            // Determine pair label for reporting (supports optional quote token)
            let pair_label = if let Some(sb) = symbol_b {
                format!("{}/{}", symbol_a, sb)
            } else {
                format!("{}/USDC", symbol_a)
            };

            // Print rich report
            print_backtest_report(
                &pair_label,
                *days,
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
            capital,
            tx_cost,
            whirlpool_address,
            lp_share,
            objective,
            range_steps,
            min_range_pct,
            max_range_pct,
            top_n,
            min_time_in_range,
            max_drawdown,
            alpha,
            static_only,
            windows,
        } => {
            let api_key = env::var("BIRDEYE_API_KEY")
                .expect("BIRDEYE_API_KEY must be set in .env or environment");
            let provider = BirdeyeProvider::new(api_key);
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
            let (token_b, use_cross_pair) = if let (Some(sb), Some(mb)) = (symbol_b.as_ref(), mint_b.as_ref()) {
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
            let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
            let fetch_days = *days * ((*windows).max(1) as u64);
            let start_time = now - (fetch_days * 24 * 3600);
            let pair_label = if use_cross_pair {
                format!("{}/{}", symbol_a, symbol_b.as_deref().unwrap_or("?"))
            } else {
                format!("{}/USDC", symbol_a)
            };
            println!("🔍 Fetching historical data for {} ({} days, {} window(s))...", pair_label, fetch_days, windows);
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
                println!("❌ No data found for the specified period.");
                return Ok(());
            }

            // For cross-pairs A/B, fetch B/USDC to convert USD capital to token B units (for liquidity math).
            let quote_usd_map: Option<HashMap<u64, Decimal>> = if use_cross_pair {
                let usdc = Token::new(
                    "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
                    "USDC",
                    6,
                    "USD Coin",
                );
                let quote_candles = provider
                    .get_price_history(&token_b, &usdc, start_time, now, 3600)
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
            let (dune_daily_tvl, dune_daily_volume): (Option<HashMap<String, Decimal>>, Option<HashMap<String, Decimal>>) =
                if let Some(pool) = whirlpool_address.as_ref() {
                    use clmm_lp_data::providers::DuneClient;
                    println!("📡 Fetching Dune TVL & volume for pool {}...", pool);
                    let dune = DuneClient::from_env()?;
                    let (tvl_map, vol_map) = dune.fetch_tvl_volume_maps(pool).await?;
                    if tvl_map.is_empty() || vol_map.is_empty() {
                        (None, None)
                    } else {
                        (Some(tvl_map), Some(vol_map))
                    }
                } else {
                    (None, None)
                };

            // Fetch on-chain active liquidity and effective fee rate (Orca Whirlpool).
            // Also fetch token decimals on-chain so base-unit conversions are correct.
            let (pool_active_liquidity, effective_fee_rate, token_a_decimals, token_b_decimals): (Option<u128>, Option<Decimal>, u8, u8) =
                if let Some(pool) = whirlpool_address.as_ref() {
                    use clmm_lp_domain::math::fee_math::calculate_effective_fee_rate;
                    use clmm_lp_protocols::orca::pool_reader::WhirlpoolReader;
                    use clmm_lp_protocols::rpc::RpcProvider;
                    use crate::engine::token_meta::fetch_mint_decimals;
                    use std::sync::Arc;
                    println!("⛓️  Fetching on-chain Whirlpool liquidity/fees for pool {}...", pool);
                    let rpc = Arc::new(RpcProvider::mainnet());
                    let reader = WhirlpoolReader::new(rpc.clone());
                    let state = reader.get_pool_state(pool).await?;
                    let base_fee = state.fee_rate();
                    let protocol_fee_pct =
                        Decimal::from(state.protocol_fee_rate_bps) / Decimal::from(10_000);
                    let eff = calculate_effective_fee_rate(base_fee, protocol_fee_pct);
                    let dec_a = fetch_mint_decimals(rpc.as_ref(), &state.token_mint_a.to_string()).await?;
                    let dec_b = fetch_mint_decimals(rpc.as_ref(), &state.token_mint_b.to_string()).await?;
                    (Some(state.liquidity), Some(eff), dec_a, dec_b)
                } else {
                    // fallback: assume 9/6 for A/USDC-like pairs
                    (None, None, token_a_decimals_guess, if use_cross_pair { token_b_decimals_guess } else { 6u8 })
                };

            let fee_rate = effective_fee_rate.unwrap_or_else(|| Decimal::from_f64(0.003).unwrap());
            let capital_dec = Decimal::from_f64(*capital).unwrap();
            let lp_share_override: Option<Decimal> = lp_share
                .and_then(|s| Decimal::from_f64(s))
                .filter(|s| *s > Decimal::ZERO && *s <= Decimal::ONE);
            let steps_per_day = Decimal::from_f64(24.0).unwrap();
            let _ = use_cross_pair;

            use backtest_engine::{build_step_data, fee_realism, run_grid, StratConfig};

            let steps_per_window = candles.len() / (*windows).max(1);
            let window_ranges: Vec<std::ops::Range<usize>> = (0..*windows)
                .map(|w| (w * steps_per_window)..((w + 1) * steps_per_window))
                .collect();

            let strategies: Vec<StratConfig> = if *static_only {
                vec![StratConfig::Static]
            } else {
                vec![
                    StratConfig::Static,
                    StratConfig::Threshold(0.02),
                    StratConfig::Threshold(0.03),
                    StratConfig::Threshold(0.05),
                    StratConfig::Threshold(0.07),
                    StratConfig::Threshold(0.10),
                    StratConfig::Threshold(0.15),
                    StratConfig::Periodic(12),
                    StratConfig::Periodic(24),
                    StratConfig::Periodic(48),
                    StratConfig::Periodic(72),
                ]
            };

            let min_frac = (*min_range_pct / 100.0).clamp(0.001, 1.0);
            let max_frac = (*max_range_pct / 100.0).clamp(min_frac + 0.001, 2.0);
            let width_pcts: Vec<f64> = if *range_steps <= 1 {
                vec![(min_frac + max_frac) / 2.0]
            } else {
                (0..*range_steps)
                    .map(|i| min_frac + (max_frac - min_frac) * (i as f64) / ((*range_steps - 1) as f64))
                    .collect()
            };
            let tx_cost_dec = Decimal::from_f64(*tx_cost).unwrap();

            let score_fn = |s: &TrackerSummary| -> Decimal {
                match objective {
                    BacktestObjectiveArg::Pnl => s.final_pnl,
                    BacktestObjectiveArg::VsHodl => s.vs_hodl,
                    BacktestObjectiveArg::Composite => {
                        let il_amt = (s.final_il_pct.abs() * capital_dec).min(capital_dec);
                        s.total_fees - (Decimal::from_f64(*alpha).unwrap() * il_amt) - s.total_rebalance_cost
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

            let (mut results, fee_check_vol, fee_check_expected_100, audit_step_data): (Vec<(f64, f64, String, TrackerSummary, Decimal)>, Decimal, Decimal, Option<Vec<backtest_engine::StepData>>) = if *windows <= 1 {
                let (step_data, entry_price, center) = build_step_data(
                    &candles,
                    dune_daily_tvl.as_ref(),
                    dune_daily_volume.as_ref(),
                    quote_usd_map.as_ref(),
                    capital_dec,
                    lp_share_override,
                    steps_per_day,
                );
                let (fv, fe100) = fee_realism(&step_data, fee_rate);
                let rows = run_grid(
                    &step_data,
                    entry_price,
                    center,
                    &width_pcts,
                    &strategies,
                    capital_dec,
                    tx_cost_dec,
                    fee_rate,
                    pool_active_liquidity,
                    token_a_decimals as u32,
                    token_b_decimals as u32,
                    None::<&[_]>,
                );
                let mut r: Vec<_> = rows
                    .into_iter()
                    .map(|(_, lower, upper, name, summary)| {
                    let sc = score_fn(&summary);
                    (lower, upper, name, summary, sc)
                })
                    .collect();
                r.sort_by(|a, b| b.4.partial_cmp(&a.4).unwrap_or(std::cmp::Ordering::Equal));
                (r, fv, fe100, Some(step_data))
            } else {
                type AggKey = (String, String);
                let mut agg: HashMap<AggKey, (Decimal, u32, f64, f64, TrackerSummary)> = HashMap::new();
                let mut fee_check_vol = Decimal::ZERO;
                let mut fee_check_expected_100 = Decimal::ZERO;
                let mut audit_step_data: Option<Vec<backtest_engine::StepData>> = None;
                for range in &window_ranges {
                    let slice = &candles[range.clone()];
                    if slice.is_empty() {
                        continue;
                    }
                    let (step_data, entry_price, center) = build_step_data(
                        slice,
                        dune_daily_tvl.as_ref(),
                        dune_daily_volume.as_ref(),
                        quote_usd_map.as_ref(),
                        capital_dec,
                        lp_share_override,
                        steps_per_day,
                    );
                    if fee_check_vol.is_zero() {
                        let (fv, fe100) = fee_realism(&step_data, fee_rate);
                        fee_check_vol = fv;
                        fee_check_expected_100 = fe100;
                        audit_step_data = Some(step_data.clone());
                    }
                    let rows = run_grid(
                        &step_data,
                        entry_price,
                        center,
                        &width_pcts,
                        &strategies,
                        capital_dec,
                        tx_cost_dec,
                        fee_rate,
                        pool_active_liquidity,
                        token_a_decimals as u32,
                        token_b_decimals as u32,
                        None::<&[_]>,
                    );
                    for (wp_frac, lower, upper, strat_name, summary) in rows {
                        let key = (format!("{:.6}", wp_frac), strat_name.clone());
                        let sc = score_fn(&summary);
                        agg.entry(key)
                            .and_modify(|e| {
                                e.0 += sc;
                                e.1 += 1;
                                e.2 = lower;
                                e.3 = upper;
                                e.4 = summary.clone();
                            })
                            .or_insert((sc, 1, lower, upper, summary));
                    }
                }
                let mut r: Vec<_> = agg
                    .into_iter()
                    .map(|((_, strat_name), (sum_score, count, lower, upper, summary))| {
                        let avg = if count > 0 {
                            sum_score / Decimal::from(count)
                        } else {
                            sum_score
                        };
                        (lower, upper, strat_name, summary, avg)
                    })
                    .collect();
                r.sort_by(|a, b| b.4.partial_cmp(&a.4).unwrap_or(std::cmp::Ordering::Equal));
                (r, fee_check_vol, fee_check_expected_100, audit_step_data)
            };

            let _round2 = |d: Decimal| (d * Decimal::from(100)).round() / Decimal::from(100);

            let min_tir = min_time_in_range.map(|x| Decimal::from_f64(x / 100.0).unwrap());
            let max_dd = max_drawdown.map(|x| Decimal::from_f64(x / 100.0).unwrap());
            results.retain(|(_, _, _, s, _)| {
                min_tir.map_or(true, |m| s.time_in_range_pct >= m)
                    && max_dd.map_or(true, |m| s.max_drawdown <= m)
            });

            let n = (*top_n).min(results.len());
            let best = match results.first() {
                Some(b) => b,
                None => {
                    println!("❌ No results after filters (min_time_in_range / max_drawdown). Relax or remove filters.");
                    return Ok(());
                }
            };
            use crate::output::optimize_report;
            optimize_report::print_best_block(
                &pair_label,
                days,
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
            if fee_check_expected_100 > Decimal::ZERO {
                let ratio_pct = best.3.total_fees / fee_check_expected_100 * Decimal::from(100);
                if *windows <= 1 {
                    println!("   Fee check: period volume ${:.0}, expected (100% TIR) ${:.2}, simulated ${:.2} (ratio {:.1}%)",
                        fee_check_vol, fee_check_expected_100, best.3.total_fees, ratio_pct);
                } else {
                    println!("   Fee check (first window only): period volume ${:.0}, expected (100% TIR) ${:.2}; BEST simulated ${:.2} is from last window (ratio {:.1}% vs first window)",
                        fee_check_vol, fee_check_expected_100, best.3.total_fees, ratio_pct);
                }
            }
            if n > 1 {
                println!();
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

            println!("📡 Initializing Optimizer...");
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

            let (token_b, use_cross_pair) = if let (Some(sb), Some(mb)) = (symbol_b.as_ref(), mint_b.as_ref()) {
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
                    "🔍 Fetching historical data for {}/{} ({} days) to estimate volatility...",
                    symbol_a,
                    symbol_b.as_deref().unwrap_or("UNKNOWN"),
                    days
                );
            } else {
                println!(
                    "🔍 Fetching historical data for {}/USDC ({} days) to estimate volatility...",
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
                println!("❌ No data found for the specified period.");
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

            println!("📊 Market Analysis:");
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
                "🔄 Running optimization with {:?} objective ({} iterations)...",
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
                    println!("🔧 Initializing database...");
                    let db = Database::connect(&database_url).await?;
                    db.migrate().await?;
                    println!("✅ Database initialized successfully!");
                }
                DbAction::Status => {
                    println!("🔍 Checking database connection...");
                    match Database::connect(&database_url).await {
                        Ok(_) => {
                            println!("✅ Connected to database: {}", database_url);
                        }
                        Err(e) => {
                            println!("❌ Failed to connect: {}", e);
                        }
                    }
                }
                DbAction::ListSimulations { limit } => {
                    let db = Database::connect(&database_url).await?;
                    let simulations = db.simulations().find_recent(*limit).await?;

                    if simulations.is_empty() {
                        println!("No simulations found.");
                    } else {
                        println!("📊 Recent Simulations:");
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
                        println!("🎯 Recent Optimizations:");
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
                println!("❌ For cross-pairs, pass both --symbol-b and --mint-b.");
                return Ok(());
            }
            let pair_label = if use_cross_pair {
                format!("{}/{}", symbol_a, symbol_b.as_deref().unwrap_or("?"))
            } else {
                format!("{}/USDC", symbol_a)
            };
            println!("📊 Analyzing {} over {} days...", pair_label, days);
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
                println!("❌ No data available for the specified period.");
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
            println!("🎯 ANALYSIS RESULTS: {}", pair_label);
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
                "Conservative (1σ daily)",
                format!(
                    "${:.2} - ${:.2}",
                    current_price - range_1x,
                    current_price + range_1x
                )
            ]);
            suggest_table.add_row(row![
                "Moderate (2σ daily)",
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
            println!("💡 Tip: Use these ranges with the backtest command:");
            println!(
                "   clmm-lp-cli backtest --lower {:.2} --upper {:.2} --days {}",
                current_price - range_2x,
                current_price + range_2x,
                days
            );
            println!();
        }
        Commands::DunePoolMetrics { pool_address } => {
            use clmm_lp_data::providers::{DuneClient, TvlPoint, VolumePoint};

            let dune = DuneClient::from_env()?;

            println!("📡 Fetching TVL series from Dune for pool {pool_address}...");
            let tvl_series: Vec<TvlPoint> = dune.fetch_tvl(pool_address).await?;
            println!("   TVL points: {}", tvl_series.len());

            println!("📡 Fetching volume/fees series from Dune for pool {pool_address}...");
            let vol_series: Vec<VolumePoint> = dune.fetch_volume_fees(pool_address).await?;
            println!("   Volume/fees points: {}", vol_series.len());

            println!();
            println!("date,tvl_usd,volume_usd,fees_usd");
            for tvl in &tvl_series {
                let vol = vol_series.iter().find(|v| v.date == tvl.date);
                let volume_usd = vol.map(|v| v.volume_usd.to_string()).unwrap_or_else(|| "0".into());
                let fees_usd = vol.map(|v| v.fees_usd.to_string()).unwrap_or_else(|| "0".into());

                println!("{},{},{},{}", tvl.date, tvl.tvl_usd, volume_usd, fees_usd);
            }
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
    println!("📊 BACKTEST RESULTS: {}", pair);
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
        "Impermanent Loss",
        format!("{:.2}%", summary.final_il_pct * Decimal::from(100))
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
    println!("🎯 OPTIMIZATION RESULTS: {}/USDC", symbol);
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
    println!("💡 Tip: Use these bounds with the backtest command:");
    println!(
        "   clmm-lp-cli backtest --lower {:.2} --upper {:.2}",
        lower, upper
    );
    println!();
}
