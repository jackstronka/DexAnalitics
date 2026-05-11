//! Shared helpers for ignored devnet E2E tests (`devnet_*` in `devnet_e2e_tests.rs`).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use clmm_lp_domain::prelude::PositionTruthMode;
use clmm_lp_execution::prelude::{DecisionConfig, ExecutorConfig, StrategyExecutor, StrategyMode};
use clmm_lp_protocols::prelude::{RpcProvider, WhirlpoolReader, WhirlpoolState};
use rust_decimal::Decimal;
use solana_sdk::pubkey::Pubkey;
use tokio::time::sleep;

/// Default Orca devnet SOL/devUSDC whirlpool (see `devnet_e2e_tests.rs` comments).
pub fn devnet_pool_address_string() -> String {
    std::env::var("DEVNET_POOL_ADDRESS")
        .unwrap_or_else(|_| "3KBZiL2g8C7tiJ32hTv5v3KM7aK9htpqTw4cTXz1HvPt".to_string())
}

pub fn devnet_open_amounts_ticks() -> (u64, u64, i32, i32) {
    let amount_a: u64 = std::env::var("DEVNET_OPEN_AMOUNT_A")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1_000_000);
    let amount_b: u64 = std::env::var("DEVNET_OPEN_AMOUNT_B")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1_000);
    let tick_lower: i32 = std::env::var("DEVNET_TICK_LOWER")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(-128);
    let tick_upper: i32 = std::env::var("DEVNET_TICK_UPPER")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(128);
    (amount_a, amount_b, tick_lower, tick_upper)
}

/// Ticks for a band **above** the current price so `tick_current < tick_lower` (out of range).
pub fn oob_ticks_below_band(pool: &WhirlpoolState) -> (i32, i32) {
    let tc = pool.tick_current;
    let spacing = (pool.tick_spacing as i32).max(1);
    let tick_lower = ((tc + 10 * spacing) / spacing) * spacing;
    let tick_upper = tick_lower + spacing * 2;
    (tick_lower, tick_upper)
}

pub async fn wait_position_account(
    provider: &RpcProvider,
    position: &Pubkey,
    attempts: usize,
) -> bool {
    for _ in 0..attempts {
        if provider.get_account(position).await.is_ok() {
            return true;
        }
        sleep(Duration::from_millis(500)).await;
    }
    false
}

pub fn decision_config_for_devnet_strategy(mode: StrategyMode) -> DecisionConfig {
    let mut c = DecisionConfig {
        strategy_mode: mode,
        min_rebalance_interval_minutes: 0,
        periodic_interval_minutes: 0,
        periodic_requires_out_of_range: false,
        rebalance_on_range_exit_immediately: true,
        threshold_pct: Decimal::ZERO,
        bollinger_window_points: 2,
        bollinger_k: Decimal::new(2, 0),
        range_width_pct: Decimal::new(10, 2),
        last_candle_seconds: 60,
        ..DecisionConfig::default()
    };
    match mode {
        StrategyMode::IlLimit => {
            c.il_rebalance_threshold = Decimal::new(80, 2);
            c.il_close_threshold = Decimal::new(99, 2);
        }
        StrategyMode::StaticRange => {}
        _ => {}
    }
    c
}

pub fn executor_config_devnet_aggressive() -> ExecutorConfig {
    ExecutorConfig {
        eval_interval_secs: 1,
        auto_execute: true,
        require_confirmation: false,
        max_slippage_pct: Decimal::new(5, 3),
        dry_run: false,
        fee_mode: PositionTruthMode::Heuristic,
    }
}

pub async fn fetch_devnet_pool_state(provider: Arc<RpcProvider>, pool_s: &str) -> WhirlpoolState {
    WhirlpoolReader::new(provider)
        .get_pool_state(pool_s)
        .await
        .expect("devnet get_pool_state")
}

/// True if lifecycle events include `Rebalanced` for `position`.
pub async fn lifecycle_has_rebalanced(
    exec: &StrategyExecutor,
    position: Pubkey,
    timeout_secs: u64,
) -> bool {
    let n = timeout_secs.max(1) as usize;
    for _ in 0..n {
        let events = exec.lifecycle().get_events(&position).await;
        if events
            .iter()
            .any(|e| e.event_type == clmm_lp_execution::lifecycle::LifecycleEventType::Rebalanced)
        {
            return true;
        }
        sleep(Duration::from_secs(1)).await;
    }
    false
}

/// Optional JSON path for pending-open recovery during devnet matrix runs.
pub fn devnet_pending_open_recovery_path() -> PathBuf {
    if let Ok(p) = std::env::var("DEVNET_PENDING_OPEN_RECOVERY_PATH") {
        PathBuf::from(p.trim())
    } else {
        std::env::temp_dir().join(format!(
            "clmm_lp_pending_open_devnet_{}.json",
            std::process::id()
        ))
    }
}
