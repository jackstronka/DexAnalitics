//! CLI entry for Orca Whirlpool live bot: `PositionMonitor` + `StrategyExecutor` (same as `clmm-lp-api`).

use super::orca_wallet::load_signing_wallet;
use anyhow::{Context, Result};
use clmm_lp_execution::prelude::{
    DecisionConfig, ExecutorConfig, MonitorConfig, PositionMonitor, StrategyExecutor,
    TransactionConfig, TransactionManager, decision_config_from_optimize_result,
    parse_optimize_result_json,
};
use clmm_lp_protocols::prelude::{RpcConfig, RpcProvider};
use rust_decimal::Decimal;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::info;

/// Run Orca LP bot: poll on-chain position, evaluate strategy, optionally sign txs.
pub async fn run_orca_bot(
    position: String,
    keypair: Option<PathBuf>,
    execute: bool,
    eval_interval_secs: u64,
    poll_interval_secs: u64,
    optimize_result_json: Option<PathBuf>,
) -> Result<()> {
    let dry_run = !execute;
    let auto_execute = execute;

    let signing_wallet = if execute {
        Some(
            load_signing_wallet(keypair.clone()).context(
                "`--execute` needs a key: `--keypair`, `KEYPAIR_PATH` / `SOLANA_KEYPAIR_PATH`, or `SOLANA_KEYPAIR`",
            )?,
        )
    } else {
        None
    };

    let provider = Arc::new(RpcProvider::new(RpcConfig::default()));

    let monitor = Arc::new(PositionMonitor::new(
        provider.clone(),
        MonitorConfig {
            poll_interval_secs,
            ..MonitorConfig::default()
        },
    ));

    monitor
        .add_position(&position)
        .await
        .with_context(|| format!("add position {position} to monitor"))?;

    let tx_manager = Arc::new(TransactionManager::new(
        provider.clone(),
        TransactionConfig::default(),
    ));

    let executor_config = ExecutorConfig {
        eval_interval_secs,
        auto_execute,
        require_confirmation: !auto_execute,
        max_slippage_pct: Decimal::new(5, 3), // 0.5%
        dry_run,
    };

    let mut executor = StrategyExecutor::new(
        provider.clone(),
        monitor.clone(),
        tx_manager,
        executor_config,
    );

    if let Some(path) = optimize_result_json {
        let txt =
            std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        let file = parse_optimize_result_json(&txt)
            .map_err(|e| anyhow::anyhow!("optimize JSON parse: {e}"))?;
        let cfg: DecisionConfig = decision_config_from_optimize_result(&file)
            .map_err(|e| anyhow::anyhow!("optimize JSON → DecisionConfig: {e}"))?;
        executor.set_decision_config(cfg);
        info!(path = %path.display(), "Loaded DecisionConfig from optimize-result JSON");
    }

    if let Some(w) = signing_wallet {
        executor.set_wallet(Arc::new(w));
    }

    let executor = Arc::new(executor);

    info!(
        position = %position,
        execute = execute,
        eval_interval_secs = eval_interval_secs,
        poll_interval_secs = poll_interval_secs,
        "Orca bot: starting monitor + strategy loop (Ctrl+C to stop)"
    );

    let mon = monitor.clone();
    let exe = executor.clone();
    tokio::join!(async move { mon.start().await }, async move {
        exe.start().await
    });

    Ok(())
}
