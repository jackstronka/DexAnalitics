use clap::{Parser, ValueEnum};

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
enum ProtocolArg {
    Orca,
    Raydium,
    Meteora,
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
    let position_truth_ready = false;

    println!("Snapshot readiness audit:");
    println!("  protocol: {:?}", args.protocol);
    println!("  pool: {}", args.pool_address);
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
