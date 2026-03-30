use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "position-truth-report",
    about = "MVP report for Tier3 position-truth checkpoints (JSONL)"
)]
struct Args {
    /// Pool address to filter checkpoints by.
    #[arg(long)]
    pool_address: String,
    /// Position address to filter checkpoints by.
    #[arg(long)]
    position_address: String,
    /// Optional JSONL ledger path (default: data/position-fee-checkpoints.jsonl).
    #[arg(long)]
    position_fee_ledger_path: Option<std::path::PathBuf>,
    /// Show last N checkpoint rows (default: 10).
    #[arg(long, default_value_t = 10)]
    tail: usize,
}

#[derive(Debug, serde::Deserialize)]
struct PositionFeeCheckpointRow {
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

fn main_inner() -> anyhow::Result<()> {
    let args = Args::parse();

    let ledger_path = args
        .position_fee_ledger_path
        .unwrap_or_else(|| std::path::PathBuf::from("data/position-fee-checkpoints.jsonl"));

    if !ledger_path.exists() {
        println!("No position-fee ledger found: {}", ledger_path.display());
        return Ok(());
    }

    let txt = std::fs::read_to_string(&ledger_path)?;
    let mut rows: Vec<PositionFeeCheckpointRow> = Vec::new();
    for line in txt.lines().filter(|l| !l.trim().is_empty()) {
        if let Ok(r) = serde_json::from_str::<PositionFeeCheckpointRow>(line) {
            if r.pool == args.pool_address && r.position == args.position_address {
                rows.push(r);
            }
        }
    }

    let total = rows.len();
    let sum_collected_a: u64 = rows.iter().map(|r| r.collected_a).sum();
    let sum_collected_b: u64 = rows.iter().map(|r| r.collected_b).sum();

    println!("Position-truth report (MVP):");
    println!("  ledger: {}", ledger_path.display());
    println!("  pool: {}", args.pool_address);
    println!("  position: {}", args.position_address);
    println!("  checkpoints: {}", total);
    println!("  collected_a_sum: {}", sum_collected_a);
    println!("  collected_b_sum: {}", sum_collected_b);

    if total == 0 {
        return Ok(());
    }

    let n = args.tail.min(total);
    println!();
    println!("Last {n} checkpoints:");
    for r in rows.iter().rev().take(n).rev() {
        println!(
            "  - ts={} event={} ticks=[{},{}] liq={} owed=({},{}) collected=({},{}) source={}",
            r.ts_utc,
            r.event_type,
            r.tick_lower,
            r.tick_upper,
            r.liquidity,
            r.fees_owed_a,
            r.fees_owed_b,
            r.collected_a,
            r.collected_b,
            r.source.as_deref().unwrap_or("unknown")
        );
    }

    Ok(())
}

fn main() -> anyhow::Result<()> {
    main_inner()
}

