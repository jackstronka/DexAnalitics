//! CLI entry for Orca Whirlpool live bot: `PositionMonitor` + `StrategyExecutor` (same as `clmm-lp-api`).

use super::orca_wallet::load_signing_wallet;
use super::position_lifecycle_ledger::try_append_position_open_cost_ledger;
use anyhow::{Context, Result};
use clmm_lp_domain::prelude::PositionTruthMode;
use clmm_lp_execution::prelude::{
    DecisionConfig, ExecutorConfig, MonitorConfig, MultiNotifier, PositionMonitor,
    StrategyExecutor, TransactionConfig, TransactionManager, WebhookNotifier,
    decision_config_from_optimize_result, parse_optimize_result_json,
};
use clmm_lp_protocols::ledger::position_registry::try_append_registry_open;
use clmm_lp_protocols::prelude::{
    OpenPositionParams, RpcConfig, RpcProvider, WhirlpoolExecutor, WhirlpoolReader,
    calculate_tick_range,
};
use rust_decimal::Decimal;
use solana_sdk::pubkey::Pubkey;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use tracing::info;

fn resolve_position_arg(
    position: Option<String>,
    open_build_response_json: Option<PathBuf>,
) -> Result<String> {
    if let Some(p) = position {
        let p = p.trim().to_string();
        if !p.is_empty() {
            return Ok(p);
        }
    }
    let Some(path) = open_build_response_json else {
        anyhow::bail!("provide --position or --open-build-response-json");
    };
    let txt = std::fs::read_to_string(&path)
        .with_context(|| format!("read open-build response JSON {}", path.display()))?;
    let v: serde_json::Value =
        serde_json::from_str(&txt).context("parse open-build response JSON")?;
    let p = v
        .get("position_address")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing `position_address` in {}", path.display()))?;
    Ok(p.to_string())
}

fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(p) = path.parent() {
        if !p.as_os_str().is_empty() {
            std::fs::create_dir_all(p)
                .with_context(|| format!("create directory {}", p.display()))?;
        }
    }
    Ok(())
}

/// Run Orca LP bot: poll on-chain position, evaluate strategy, optionally sign txs.
pub async fn run_orca_bot(
    position: Option<String>,
    open_build_response_json: Option<PathBuf>,
    keypair: Option<PathBuf>,
    execute: bool,
    fee_mode: String,
    eval_interval_secs: u64,
    poll_interval_secs: u64,
    optimize_result_json: Option<PathBuf>,
    il_ledger_path: Option<PathBuf>,
    position_fee_ledger_path: Option<PathBuf>,
) -> Result<()> {
    let position = resolve_position_arg(position, open_build_response_json)?;
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

    let fee_mode = match fee_mode.trim().to_ascii_lowercase().as_str() {
        "heuristic" => PositionTruthMode::Heuristic,
        "position-truth" | "position_truth" | "truth" => PositionTruthMode::PositionTruth,
        other => {
            anyhow::bail!("invalid --fee-mode {other:?} (expected: heuristic | position-truth)")
        }
    };

    let executor_config = ExecutorConfig {
        eval_interval_secs,
        auto_execute,
        require_confirmation: !auto_execute,
        max_slippage_pct: Decimal::new(5, 3), // 0.5%
        dry_run,
        fee_mode,
    };

    let executor = StrategyExecutor::new(
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

    if execute {
        let path = std::env::var("CLMM_PENDING_OPEN_RECOVERY_PATH")
            .ok()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("data/pending-open-recovery.json"));
        ensure_parent_dir(&path)?;
        executor.set_pending_open_recovery_path(Some(path));
    }

    if let Ok(url) = std::env::var("CLMM_ALERT_WEBHOOK_URL") {
        if !url.is_empty() {
            let mut m = MultiNotifier::new();
            m.add(WebhookNotifier::new(url));
            executor.set_alert_notifier(Some(Arc::new(m)));
        }
    }

    if let Some(ref p) = il_ledger_path {
        ensure_parent_dir(p)?;
    }
    let position_fee_ledger_path = position_fee_ledger_path
        .clone()
        .or_else(|| Some(PathBuf::from("data/position-fee-checkpoints.jsonl")));
    if let Some(ref p) = position_fee_ledger_path {
        ensure_parent_dir(p)?;
    }
    executor.set_il_ledger_path(il_ledger_path.clone());
    executor.set_position_fee_ledger_path(position_fee_ledger_path.clone());
    if il_ledger_path.is_some() || position_fee_ledger_path.is_some() {
        info!(
            il_ledger = ?il_ledger_path,
            position_fee_ledger = ?position_fee_ledger_path,
            "Orca bot: JSONL ledgers enabled"
        );
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

/// Open new Orca position and immediately start bot loop on the created position.
pub async fn run_orca_bot_open_and_run(
    pool: String,
    keypair: Option<PathBuf>,
    tick_lower: Option<i32>,
    tick_upper: Option<i32>,
    range_width_pct: Option<f64>,
    amount_a: u64,
    amount_b: u64,
    slippage_bps: u16,
    execute: bool,
    fee_mode: String,
    eval_interval_secs: u64,
    poll_interval_secs: u64,
    optimize_result_json: Option<PathBuf>,
    il_ledger_path: Option<PathBuf>,
    position_fee_ledger_path: Option<PathBuf>,
) -> Result<()> {
    let pool_pk = Pubkey::from_str(pool.trim()).context("invalid pool pubkey")?;
    let wallet = load_signing_wallet(keypair.clone()).context(
        "`orca-bot-open-and-run` needs signing key: `--keypair`, `KEYPAIR_PATH` / `SOLANA_KEYPAIR_PATH`, or `SOLANA_KEYPAIR`",
    )?;

    let provider = Arc::new(RpcProvider::new(RpcConfig::default()));
    let reader = WhirlpoolReader::new(provider.clone());
    let state = reader
        .get_pool_state(pool.trim())
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
            calculate_tick_range(state.tick_current, width_dec, state.tick_spacing)
        }
        _ => anyhow::bail!(
            "provide either (--tick-lower AND --tick-upper) OR --range-width-pct (percent of price width, e.g. 10)"
        ),
    };

    let orca = WhirlpoolExecutor::new(provider.clone());
    let open = orca
        .open_position(
            &OpenPositionParams {
                pool: pool_pk,
                tick_lower: tl,
                tick_upper: tu,
                amount_a,
                amount_b,
                slippage_bps,
            },
            wallet.keypair(),
        )
        .await
        .context("open_position RPC")?;

    if !open.success {
        let msg = open.error.unwrap_or_else(|| "unknown error".to_string());
        anyhow::bail!("open_position failed: {msg}");
    }
    let position = open
        .created_position
        .ok_or_else(|| anyhow::anyhow!("open_position succeeded but missing created_position"))?;

    info!(
        pool = %pool_pk,
        tick_lower = tl,
        tick_upper = tu,
        position = %position,
        open_signature = %open.signature,
        "Opened position for bot run"
    );

    let fee_payer = wallet.pubkey();
    let position_pda = Some(position.to_string());
    try_append_position_open_cost_ledger(
        provider.as_ref(),
        &state,
        amount_a,
        amount_b,
        slippage_bps,
        &open.signature,
        "tick_range",
        Some(tl),
        Some(tu),
        range_width_pct,
        position_pda,
        &fee_payer,
        "orca_bot",
    )
    .await;
    try_append_registry_open(
        provider.as_ref(),
        "orca_bot",
        &position,
        &pool_pk,
        &fee_payer,
        &open.signature,
        None,
    )
    .await;

    run_orca_bot(
        Some(position.to_string()),
        None,
        keypair,
        execute,
        fee_mode,
        eval_interval_secs,
        poll_interval_secs,
        optimize_result_json,
        il_ledger_path,
        position_fee_ledger_path,
    )
    .await
}
