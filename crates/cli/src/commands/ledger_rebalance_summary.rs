//! CLI: aggregate rebalance rows from IL JSONL + tx-cost rows from `orca_position_lifecycle.jsonl`.

use anyhow::{Context, Result};
use clmm_lp_protocols::ledger::tx_lifecycle::{il_ledger_path_from_env, ledger_path};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

fn resolve_il_path(cli: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(p) = cli {
        let t = p.to_string_lossy().trim().to_string();
        if !t.is_empty() {
            return Some(p);
        }
    }
    il_ledger_path_from_env()
}

fn read_jsonl(path: &PathBuf) -> Result<Vec<Value>> {
    let f = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut out = Vec::new();
    for (line_no, line) in BufReader::new(f).lines().enumerate() {
        let line = line.with_context(|| format!("read line {}", line_no + 1))?;
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let v: Value =
            serde_json::from_str(t).with_context(|| format!("parse JSON line {}", line_no + 1))?;
        out.push(v);
    }
    Ok(out)
}

/// Summarize IL ledger `event: "rebalance"` rows and lifecycle ledger costs (optionally by session).
pub fn run_ledger_rebalance_summary(
    il_ledger: Option<PathBuf>,
    lifecycle_ledger: Option<PathBuf>,
) -> Result<()> {
    let il_path = resolve_il_path(il_ledger);
    let life_path = lifecycle_ledger.unwrap_or_else(ledger_path);

    if il_path.is_none() && !life_path.exists() {
        anyhow::bail!(
            "nothing to read: pass --il-ledger or set CLMM_IL_LEDGER_PATH, and/or ensure lifecycle file exists ({}). Use --lifecycle-ledger to override.",
            life_path.display()
        );
    }

    if let Some(ref p) = il_path {
        let rows = read_jsonl(p)?;
        let mut rebalances: Vec<&Value> = rows
            .iter()
            .filter(|v| v.get("event").and_then(|e| e.as_str()) == Some("rebalance"))
            .collect();

        println!("=== IL ledger (rebalance events) ===");
        println!("file: {}", p.display());
        println!("count: {}", rebalances.len());

        let mut sum_tx: u128 = 0;
        for v in &rebalances {
            if let Some(n) = v.get("tx_cost_lamports").and_then(|x| x.as_u64()) {
                sum_tx += u128::from(n);
            }
        }
        println!("sum tx_cost_lamports (executor estimate): {sum_tx}");

        rebalances.sort_by_key(|v| {
            v.get("timestamp")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string()
        });

        for v in rebalances {
            let ts = v.get("timestamp").and_then(|s| s.as_str()).unwrap_or("?");
            let old_p = v
                .get("old_position")
                .and_then(|s| s.as_str())
                .unwrap_or("-");
            let new_p = v.get("position").and_then(|s| s.as_str()).unwrap_or("?");
            let pool = v.get("pool").and_then(|s| s.as_str()).unwrap_or("?");
            let reason = v.get("reason").and_then(|s| s.as_str()).unwrap_or("?");
            let txc = v
                .get("tx_cost_lamports")
                .and_then(|x| x.as_u64())
                .map(|n| n.to_string())
                .unwrap_or_else(|| "-".to_string());
            let sid = v
                .get("rebalance_session_id")
                .and_then(|s| s.as_str())
                .unwrap_or("-");
            println!("{ts} | {old_p} -> {new_p} | pool={pool} | {reason} | tx_lamports={txc} | session={sid}");
        }
        println!();
    } else {
        println!("=== IL ledger (rebalance events) ===");
        println!("(skipped — no --il-ledger and no CLMM_IL_LEDGER_PATH)\n");
    }

    if !life_path.exists() {
        println!("=== Lifecycle tx-cost ledger ===");
        println!("(skipped — file not found: {})\n", life_path.display());
        return Ok(());
    }

    let life_rows = read_jsonl(&life_path)?;
    println!("=== Lifecycle tx-cost ledger ===");
    println!("file: {}", life_path.display());
    println!("rows: {}", life_rows.len());

    #[derive(Default)]
    struct Agg {
        rows: usize,
        tx_fee: u128,
        net_delta: i128,
    }
    let mut by_session: BTreeMap<String, Agg> = BTreeMap::new();
    let mut ungrouped = Agg::default();

    for v in &life_rows {
        let sid = v
            .get("rebalance_session_id")
            .and_then(|s| s.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let fee = v.get("tx_fee_lamports").and_then(|x| x.as_u64()).unwrap_or(0);
        let net = v
            .get("fee_payer_net_lamports_delta")
            .and_then(|x| x.as_i64())
            .unwrap_or(0);

        let target = if let Some(s) = sid {
            by_session.entry(s.to_string()).or_default()
        } else {
            &mut ungrouped
        };
        target.rows += 1;
        target.tx_fee += u128::from(fee);
        target.net_delta += i128::from(net);
    }

    if !by_session.is_empty() {
        println!("\n-- By CLMM_REBALANCE_SESSION_ID (sum tx_fee_lamports, sum fee_payer_net_lamports_delta) --");
        for (sid, a) in &by_session {
            println!(
                "session={sid} | rows={} | tx_fee_lamports={} | net_lamports_delta={}",
                a.rows, a.tx_fee, a.net_delta
            );
        }
    }

    if ungrouped.rows > 0 {
        println!(
            "\n-- Rows without rebalance_session_id | rows={} | tx_fee_lamports={} | net_lamports_delta={}",
            ungrouped.rows, ungrouped.tx_fee, ungrouped.net_delta
        );
    }

    let mut bot_like = 0u64;
    let mut sum_fee: u128 = 0;
    for v in &life_rows {
        let ev = v.get("event").and_then(|e| e.as_str()).unwrap_or("");
        if ev.starts_with("bot_") || ev == "cli_swap" {
            bot_like += 1;
            if let Some(f) = v.get("tx_fee_lamports").and_then(|x| x.as_u64()) {
                sum_fee += u128::from(f);
            }
        }
    }
    println!(
        "\n-- Rough footprint: bot_* + cli_swap rows | count={bot_like} | sum tx_fee_lamports={sum_fee} --"
    );

    Ok(())
}
