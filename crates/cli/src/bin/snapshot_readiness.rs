use clap::{Parser, ValueEnum};

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
enum ProtocolArg {
    Orca,
    Raydium,
    Meteora,
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
enum FeeModeArg {
    Heuristic,
    PositionTruth,
}

#[derive(Parser, Debug)]
#[command(
    name = "snapshot-readiness",
    about = "Audit snapshot sufficiency (tier 1/2/3)"
)]
struct Args {
    /// Protocol of the snapshot file
    #[arg(long, value_enum)]
    protocol: ProtocolArg,
    /// Pool address used in data/pool-snapshots/{protocol}/{pool}/snapshots.jsonl
    #[arg(long)]
    pool_address: String,
    /// Fee accounting mode (default: current heuristic path).
    #[arg(long, value_enum, default_value_t = FeeModeArg::Heuristic)]
    fee_mode: FeeModeArg,
    /// Optional JSONL ledger path for Tier3 position-fee checkpoints (used in `--fee-mode position-truth`).
    ///
    /// Expected format: JSONL rows emitted by `LifecycleTracker::record_fee_checkpoint`.
    #[arg(long)]
    position_fee_ledger_path: Option<std::path::PathBuf>,
    /// Optional position address for Tier3 readiness (position-truth is evaluated per position).
    ///
    /// When `--fee-mode position-truth` is selected and this arg is omitted, Tier3 will report NOT READY
    /// with a hint to pass `--position-address`.
    #[arg(long)]
    position_address: Option<String>,
}

fn protocol_dir(protocol: ProtocolArg) -> &'static str {
    match protocol {
        ProtocolArg::Orca => "orca",
        ProtocolArg::Raydium => "raydium",
        ProtocolArg::Meteora => "meteora",
    }
}

fn main_inner() -> anyhow::Result<()> {
    let args = Args::parse();

    let path = std::path::Path::new("data")
        .join("pool-snapshots")
        .join(protocol_dir(args.protocol))
        .join(&args.pool_address)
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

    let mut parse_ok_rows = 0usize;
    let mut parse_error_rows = 0usize;

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

        // Optional diagnostics fields (only present in the newer collector output).
        if v.get("parse_ok").and_then(|x| x.as_bool()).unwrap_or(false) {
            parse_ok_rows += 1;
        }
        if v.get("parse_error").is_some() {
            parse_error_rows += 1;
        }

        let has_fee_growth = match args.protocol {
            ProtocolArg::Orca => {
                v.get("fee_growth_global_a").is_some() && v.get("fee_growth_global_b").is_some()
            }
            ProtocolArg::Raydium => {
                v.get("fee_growth_global_a_x64").is_some()
                    && v.get("fee_growth_global_b_x64").is_some()
            }
            ProtocolArg::Meteora => false,
        };
        if has_fee_growth {
            with_fee_growth += 1;
        }

        let has_protocol_fee_counter = match args.protocol {
            ProtocolArg::Orca => {
                v.get("protocol_fee_owed_a").is_some() && v.get("protocol_fee_owed_b").is_some()
            }
            ProtocolArg::Raydium => {
                v.get("protocol_fees_token_a").is_some() && v.get("protocol_fees_token_b").is_some()
            }
            ProtocolArg::Meteora => {
                v.get("protocol_fee_amount_a").is_some() && v.get("protocol_fee_amount_b").is_some()
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
    let snapshot_fee_heuristic_ready =
        with_ts >= 2 && with_mints >= 2 && (with_fee_growth >= 2 || with_protocol_fee_counter >= 2);

    #[derive(Debug, serde::Deserialize)]
    struct PositionFeeCheckpointRow {
        #[allow(dead_code)]
        #[serde(default)]
        schema_version: Option<u32>,
        ts_utc: String,
        position: String,
        pool: String,
        event_type: String,
        tick_lower: i32,
        tick_upper: i32,
        liquidity: String,
        fees_owed_a: u64,
        fees_owed_b: u64,
        collected_a: u64,
        collected_b: u64,
        #[serde(default)]
        source: Option<String>,
    }

    let mut tier3_missing: Vec<String> = Vec::new();
    let position_truth_ready = if matches!(args.fee_mode, FeeModeArg::PositionTruth) {
        let pos_arg = args.position_address.clone().map(|s| s.trim().to_string());
        let ledger_path = args
            .position_fee_ledger_path
            .clone()
            .unwrap_or_else(|| std::path::PathBuf::from("data/position-fee-checkpoints.jsonl"));

        if !ledger_path.exists() {
            tier3_missing.push(format!(
                "missing position-fee ledger file: {}",
                ledger_path.display()
            ));
            false
        } else {
            let txt = std::fs::read_to_string(&ledger_path).unwrap_or_default();
            let mut all_rows_for_pool: Vec<PositionFeeCheckpointRow> = Vec::new();
            let mut positions_for_pool: std::collections::BTreeSet<String> =
                std::collections::BTreeSet::new();
            for line in txt.lines().filter(|l| !l.trim().is_empty()) {
                if let Ok(r) = serde_json::from_str::<PositionFeeCheckpointRow>(line) {
                    if r.pool == args.pool_address {
                        positions_for_pool.insert(r.position.clone());
                        all_rows_for_pool.push(r);
                    }
                }
            }

            let pos = if let Some(p) = pos_arg.clone() && !p.is_empty() {
                Some(p)
            } else if positions_for_pool.len() == 1 {
                positions_for_pool.iter().next().cloned()
            } else {
                None
            };

            let rows_opt: Option<Vec<PositionFeeCheckpointRow>> = pos.map(|pos| {
                all_rows_for_pool
                    .into_iter()
                    .filter(|r| r.position == pos)
                    .collect::<Vec<_>>()
            });

            match rows_opt {
                None => {
                    tier3_missing.push(
                        "missing --position-address for per-position Tier3 readiness".to_string(),
                    );
                    if positions_for_pool.is_empty() {
                        tier3_missing.push("no positions found in ledger for this pool".to_string());
                    } else {
                        let positions_list = positions_for_pool
                            .iter()
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(", ");
                        tier3_missing.push(format!(
                            "positions in ledger for this pool: {}",
                            positions_list
                        ));
                        tier3_missing.push("suggested commands (pick one position):".to_string());
                        for p in positions_for_pool {
                            tier3_missing.push(format!(
                                "cargo run --bin clmm-lp-cli -- snapshot-readiness --protocol {:?} --pool-address {} --fee-mode position-truth --position-address {}",
                                args.protocol, args.pool_address, p
                            ));
                        }
                    }
                    false
                }
                Some(rows) => {
                    if rows.len() < 2 {
                        tier3_missing.push("need >=2 checkpoints for this pool+position".to_string());
                    }

                    let has_open = rows.iter().any(|r| r.event_type == "open_position");
                    if !has_open {
                        tier3_missing.push("missing open_position checkpoint".to_string());
                    }

                    let has_progress = rows.iter().any(|r| {
                        matches!(
                            r.event_type.as_str(),
                            "collect_fees" | "close_position" | "rebalance_out" | "rebalance_in"
                        )
                    });
                    if !has_progress {
                        tier3_missing.push(
                            "missing one of: collect_fees | close_position | rebalance_* checkpoint"
                                .to_string(),
                        );
                    }

                    tier3_missing.is_empty()
                }
            }
        }
    } else {
        false
    };

    println!("Snapshot readiness audit:");
    println!("  protocol: {:?}", args.protocol);
    println!("  pool: {}", args.pool_address);
    println!("  fee_mode: {:?}", args.fee_mode);
    println!("  file: {}", path.display());
    println!("  rows: {}", total);
    println!(
        "  coverage: ts={} ({:.1}%), vaults={} ({:.1}%), mints={} ({:.1}%), liquidity={} ({:.1}%), fee_growth={} ({:.1}%), protocol_fee_counter={} ({:.1}%), decimals={} ({:.1}%), parse_ok={} parse_error={}",
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
        pct(with_decimals),
        parse_ok_rows,
        parse_error_rows
    );
    println!();
    println!("Readiness tiers:");
    println!(
        "  2) Snapshot fee heuristic (experimental): {}",
        if snapshot_fee_heuristic_ready {
            "READY"
        } else {
            "NOT READY"
        }
    );
    println!(
        "  1) LP-share (capital/TVL proxy): {}",
        if lp_share_ready { "READY" } else { "NOT READY" }
    );
    println!(
        "  3) Position-truth fee model: {}",
        if position_truth_ready {
            "READY"
        } else {
            "NOT READY"
        }
    );
    if matches!(args.fee_mode, FeeModeArg::PositionTruth) && !position_truth_ready {
        if tier3_missing.is_empty() {
            println!("     Tier3: unknown reason (no missing reasons collected)");
        } else {
            println!("     Tier3 missing:");
            for m in tier3_missing {
                println!("       - {m}");
            }
        }
    }

    if args.protocol == ProtocolArg::Meteora && !snapshot_fee_heuristic_ready {
        println!(
            "     Missing: protocol_fee_amount_a/b coverage (tier 2 requires it at >=2 rows)."
        );
    }
    if args.protocol == ProtocolArg::Raydium && !snapshot_fee_heuristic_ready {
        println!(
            "     Missing: fee-growth and/or protocol-fees coverage (tier 2 requires >=2 rows with required fields)."
        );
    }

    Ok(())
}

fn main() -> anyhow::Result<()> {
    main_inner()
}
