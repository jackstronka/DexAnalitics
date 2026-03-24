//! Run `clmm-lp-cli backtest-optimize` as a subprocess and apply `--optimize-result-json` to the executor.

use crate::error::ApiError;
use clmm_lp_execution::prelude::{
    decision_config_from_optimize_result, parse_optimize_result_json, StrategyExecutor,
};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Ensure CLI args include `--optimize-result-json <path>`.
pub fn merge_optimize_result_json_arg(mut argv: Vec<String>, result_path: &str) -> Vec<String> {
    if argv.iter().any(|a| a == "--optimize-result-json") {
        return argv;
    }
    argv.push("--optimize-result-json".to_string());
    argv.push(result_path.to_string());
    argv
}

/// Run optimize CLI (blocking). Returns stdout/stderr capture for logs on failure.
pub async fn run_optimize_subprocess(argv: &[String]) -> Result<(), ApiError> {
    if argv.is_empty() {
        return Err(ApiError::bad_request(
            "optimize_command must be non-empty (first element is the program path)",
        ));
    }
    let argv = argv.to_vec();
    let out = tokio::task::spawn_blocking(move || {
        let mut c = std::process::Command::new(&argv[0]);
        c.args(&argv[1..]);
        c.output()
    })
    .await
    .map_err(|e| ApiError::internal(format!("optimize join error: {e}")))?;

    let output = out.map_err(|e| ApiError::internal(format!("optimize spawn error: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        warn!(%stderr, %stdout, "backtest-optimize subprocess failed");
        return Err(ApiError::internal(format!(
            "backtest-optimize exited with {:?}: {}",
            output.status.code(),
            stderr
        )));
    }
    Ok(())
}

/// Read JSON result and apply to the running executor's decision config.
pub async fn apply_optimize_result_json(
    path: &Path,
    executor: &RwLock<StrategyExecutor>,
) -> Result<(), ApiError> {
    let text = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| ApiError::internal(format!("read optimize result: {e}")))?;
    let file = parse_optimize_result_json(&text)
        .map_err(|e| ApiError::internal(format!("parse optimize JSON: {e}")))?;
    let cfg = decision_config_from_optimize_result(&file)
        .map_err(|e| ApiError::internal(format!("optimize profile → DecisionConfig: {e}")))?;
    let mut g = executor.write().await;
    g.set_decision_config(cfg);
    info!(
        strategy_kind = %file.winner.strategy_kind,
        width_pct = file.winner.width_pct,
        "Applied optimize-result JSON to StrategyExecutor"
    );
    Ok(())
}

/// Try to start an optimize cycle. Returns `false` if another cycle still holds the lock.
#[must_use]
pub fn try_begin_optimize_busy(busy: &AtomicBool) -> bool {
    !busy.swap(true, Ordering::SeqCst)
}

/// Release the optimize lock after a cycle finishes (success or failure).
pub fn end_optimize_busy(busy: &AtomicBool) {
    busy.store(false, Ordering::SeqCst);
}

/// Run subprocess (if not busy) then apply JSON. `busy` prevents overlapping runs.
pub async fn run_optimize_cycle(
    argv: &[String],
    result_path: &Path,
    executor: &Arc<RwLock<StrategyExecutor>>,
    busy: &Arc<AtomicBool>,
) -> Result<(), ApiError> {
    if !try_begin_optimize_busy(busy) {
        warn!("optimization skipped: previous run still marked busy");
        return Ok(());
    }
    let res = async {
        run_optimize_subprocess(argv).await?;
        apply_optimize_result_json(result_path, executor.as_ref()).await
    }
    .await;
    end_optimize_busy(busy);
    res
}

#[cfg(test)]
mod tests {
    use super::{
        end_optimize_busy, merge_optimize_result_json_arg, try_begin_optimize_busy,
    };
    use std::sync::atomic::AtomicBool;

    #[test]
    fn merges_json_flag() {
        let v = vec!["bin".to_string(), "backtest-optimize".to_string()];
        let m = merge_optimize_result_json_arg(v, "/tmp/out.json");
        assert_eq!(
            m,
            vec![
                "bin",
                "backtest-optimize",
                "--optimize-result-json",
                "/tmp/out.json"
            ]
        );
    }

    #[test]
    fn does_not_duplicate_flag() {
        let v = vec![
            "bin".to_string(),
            "--optimize-result-json".to_string(),
            "x.json".to_string(),
        ];
        let m = merge_optimize_result_json_arg(v.clone(), "/tmp/out.json");
        assert_eq!(m, v);
    }

    #[test]
    fn optimize_busy_prevents_second_acquire_until_released() {
        let busy = AtomicBool::new(false);
        assert!(try_begin_optimize_busy(&busy));
        assert!(!try_begin_optimize_busy(&busy));
        end_optimize_busy(&busy);
        assert!(try_begin_optimize_busy(&busy));
        end_optimize_busy(&busy);
    }
}
