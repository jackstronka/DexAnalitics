//! One-off Orca Whirlpool position actions (open, partial decrease).

use anyhow::{Context, Result};
use clmm_lp_execution::prelude::{
    LifecycleTracker, RebalanceConfig, RebalanceExecutor, TransactionConfig, TransactionManager,
};
use clmm_lp_protocols::prelude::*;
use rust_decimal::Decimal;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;
use std::sync::Arc;
use tracing::info;

use super::orca_wallet::load_signing_wallet;

/// Resolve liquidity to remove: exactly one of `liquidity_pct` or `liquidity` (raw).
pub(crate) fn resolve_decrease_liquidity_delta(
    on_chain_liquidity: u128,
    liquidity_pct: Option<f64>,
    liquidity: Option<u128>,
) -> Result<u128> {
    match (liquidity_pct, liquidity) {
        (Some(pct), None) => {
            if pct <= 0.0 || pct > 100.0 {
                anyhow::bail!("--liquidity-pct must be in (0, 100]");
            }
            let d = liquidity_amount_from_pct(on_chain_liquidity, pct);
            if d == 0 {
                anyhow::bail!("--liquidity-pct yields 0 liquidity at current precision");
            }
            Ok(d)
        }
        (None, Some(l)) => {
            if l == 0 || l > on_chain_liquidity {
                anyhow::bail!(
                    "--liquidity must be > 0 and <= on-chain liquidity {}",
                    on_chain_liquidity
                );
            }
            Ok(l)
        }
        _ => anyhow::bail!("provide exactly one of --liquidity-pct or --liquidity"),
    }
}

fn ensure_ticks_on_spacing(lower: i32, upper: i32, spacing: u16) -> Result<()> {
    let s = i64::from(spacing);
    if lower as i64 % s != 0 || upper as i64 % s != 0 {
        anyhow::bail!(
            "tick_lower ({lower}) and tick_upper ({upper}) must be multiples of tick_spacing ({spacing})"
        );
    }
    if lower >= upper {
        anyhow::bail!("tick_lower must be strictly less than tick_upper");
    }
    Ok(())
}

fn execution_ok(res: &ExecutionResult) -> Result<()> {
    if !res.success {
        let msg = res
            .error
            .clone()
            .unwrap_or_else(|| "unknown error".to_string());
        anyhow::bail!("transaction failed: {msg}");
    }
    Ok(())
}

/// Open a new Whirlpool position (open + increase with max token caps).
pub async fn run_position_open(
    pool_addr: String,
    keypair: Option<std::path::PathBuf>,
    dry_run: bool,
    tick_lower: Option<i32>,
    tick_upper: Option<i32>,
    range_width_pct: Option<f64>,
    slippage_bps: u16,
) -> Result<()> {
    let pool = Pubkey::from_str(pool_addr.trim()).context("invalid pool pubkey")?;
    let provider = Arc::new(RpcProvider::new(RpcConfig::default()));
    let reader = WhirlpoolReader::new(provider.clone());
    let state = reader
        .get_pool_state(pool_addr.trim())
        .await
        .context("fetch pool state")?;

    let (tl, tu) = match (tick_lower, tick_upper, range_width_pct) {
        (Some(l), Some(u), None) => {
            ensure_ticks_on_spacing(l, u, state.tick_spacing)?;
            (l, u)
        }
        (None, None, Some(w)) => {
            if w <= 0.0 || w > 100.0 {
                anyhow::bail!(
                    "--range-width-pct must be in (0, 100], e.g. 10 for ±~5% price band around spot"
                );
            }
            let width_dec =
                Decimal::from_f64_retain(w / 100.0).context("range width as decimal")?;
            let (l, u) = calculate_tick_range(state.tick_current, width_dec, state.tick_spacing);
            (l, u)
        }
        _ => anyhow::bail!(
            "provide either (--tick-lower AND --tick-upper) OR --range-width-pct (percent of price width, e.g. 10)"
        ),
    };

    let pos = derive_whirlpool_position_address(&pool, tl, tu);
    info!(
        pool = %pool,
        tick_lower = tl,
        tick_upper = tu,
        position = %pos,
        dry_run = dry_run,
        "Orca open position"
    );

    if dry_run {
        println!("dry-run: would open position {pos}");
        println!("  pool: {pool}");
        println!("  ticks: [{tl}, {tu}] (spacing {})", state.tick_spacing);
        println!(
            "note: Whirlpool instruction account lists in this repo may be incomplete; test on devnet / simulate first."
        );
        return Ok(());
    }

    let wallet = Arc::new(load_signing_wallet(keypair)?);
    let orca = WhirlpoolExecutor::new(provider);
    let params = OpenPositionParams {
        pool,
        tick_lower: tl,
        tick_upper: tu,
        amount_a: u64::MAX,
        amount_b: u64::MAX,
        slippage_bps,
    };
    let res = orca
        .open_position(&params, wallet.keypair())
        .await
        .context("open_position RPC")?;
    execution_ok(&res)?;
    println!("position PDA: {pos}");
    println!("signature: {}", res.signature);
    Ok(())
}

/// Partially decrease liquidity (`token_min_a/b = 0` — high slippage tolerance).
pub async fn run_position_decrease(
    position_addr: String,
    keypair: Option<std::path::PathBuf>,
    dry_run: bool,
    liquidity_pct: Option<f64>,
    liquidity: Option<u128>,
) -> Result<()> {
    let position_pk = Pubkey::from_str(position_addr.trim()).context("invalid position pubkey")?;
    let provider = Arc::new(RpcProvider::new(RpcConfig::default()));
    let pos_reader = PositionReader::new(provider.clone());
    let on_chain = pos_reader
        .get_position(position_addr.trim())
        .await
        .context("fetch position")?;
    let pool_pk = on_chain.pool;

    let delta = resolve_decrease_liquidity_delta(on_chain.liquidity, liquidity_pct, liquidity)?;

    let tx_manager = Arc::new(TransactionManager::new(
        provider.clone(),
        TransactionConfig::default(),
    ));
    let lifecycle = Arc::new(LifecycleTracker::new());
    let mut exec =
        RebalanceExecutor::new(provider, tx_manager, lifecycle, RebalanceConfig::default());
    exec.set_wallet(Arc::new(load_signing_wallet(keypair)?));
    exec.set_dry_run(dry_run);
    exec.execute_partial_decrease(&position_pk, &pool_pk, delta)
        .await?;

    if dry_run {
        println!("dry-run: would decrease liquidity by {delta}");
    } else {
        println!(
            "requested liquidity decrease: {delta} (min tokens = 0; adjust executor for production slippage)"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_ticks_on_spacing_accepts_aligned() {
        ensure_ticks_on_spacing(-128, 128, 64).unwrap();
    }

    #[test]
    fn ensure_ticks_on_spacing_rejects_misaligned() {
        assert!(ensure_ticks_on_spacing(-100, 128, 64).is_err());
    }

    #[test]
    fn ensure_ticks_on_spacing_rejects_inverted_range() {
        assert!(ensure_ticks_on_spacing(64, 64, 64).is_err());
        assert!(ensure_ticks_on_spacing(128, 64, 64).is_err());
    }

    #[test]
    fn resolve_decrease_delta_pct() {
        let d = resolve_decrease_liquidity_delta(1000, Some(10.0), None).unwrap();
        assert_eq!(d, liquidity_amount_from_pct(1000, 10.0));
        assert!(d > 0);
    }

    #[test]
    fn resolve_decrease_delta_raw() {
        assert_eq!(
            resolve_decrease_liquidity_delta(100, None, Some(50)).unwrap(),
            50
        );
    }

    #[test]
    fn resolve_decrease_rejects_both_or_neither() {
        assert!(resolve_decrease_liquidity_delta(100, None, None).is_err());
        assert!(resolve_decrease_liquidity_delta(100, Some(1.0), Some(1)).is_err());
    }

    #[test]
    fn resolve_decrease_rejects_bad_pct() {
        assert!(resolve_decrease_liquidity_delta(100, Some(0.0), None).is_err());
        assert!(resolve_decrease_liquidity_delta(100, Some(101.0), None).is_err());
    }
}
