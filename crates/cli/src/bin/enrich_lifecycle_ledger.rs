use clap::Parser;
use clmm_lp_protocols::ledger::tx_lifecycle::{
    enrich_tx_costs, fee_payer_token_deltas_by_mint, ledger_path,
};
use clmm_lp_protocols::rpc::RpcProvider;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Signature;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;
use tokio::time::sleep;

#[derive(Parser, Debug)]
#[command(
    name = "enrich-lifecycle-ledger",
    about = "Recompute fee_payer_token_deltas from tx meta and write a new JSONL ledger (keeps original untouched)."
)]
struct Args {
    /// Input JSONL path (default: ledger_path() = data/ledger/orca_position_lifecycle.jsonl).
    #[arg(long)]
    input: Option<PathBuf>,

    /// Output JSONL path (default: <input>.enriched.jsonl).
    #[arg(long)]
    output: Option<PathBuf>,

    /// Max rows to process (default: all).
    #[arg(long)]
    limit: Option<usize>,

    /// Sleep between RPC calls in ms (default: 40).
    #[arg(long, default_value_t = 40)]
    sleep_ms: u64,

    /// Only rewrite rows that have `signature` and `fee_payer_pubkey` (default: true).
    #[arg(long, default_value_t = true)]
    only_signed_rows: bool,
}

fn default_output_path(input: &PathBuf) -> PathBuf {
    let s = input.to_string_lossy().to_string();
    if s.ends_with(".jsonl") {
        PathBuf::from(format!("{}.enriched.jsonl", s.trim_end_matches(".jsonl")))
    } else {
        PathBuf::from(format!("{s}.enriched.jsonl"))
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let input = args.input.unwrap_or_else(ledger_path);
    let output = args.output.unwrap_or_else(|| default_output_path(&input));

    if !input.exists() {
        anyhow::bail!("Input ledger does not exist: {}", input.display());
    }

    let rpc = RpcProvider::mainnet();
    let f_in = File::open(&input)?;
    let reader = BufReader::new(f_in);
    let f_out = File::create(&output)?;
    let mut writer = BufWriter::new(f_out);

    let mut total: usize = 0;
    let mut rewritten: usize = 0;
    let mut skipped: usize = 0;
    let mut failed: usize = 0;

    for line in reader.lines() {
        let line = line?;
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        total += 1;
        if let Some(limit) = args.limit {
            if total > limit {
                break;
            }
        }

        let mut v: serde_json::Value = match serde_json::from_str(t) {
            Ok(v) => v,
            Err(_) => {
                // Preserve unparseable lines as-is.
                writeln!(writer, "{t}")?;
                skipped += 1;
                continue;
            }
        };

        let Some(obj) = v.as_object_mut() else {
            writeln!(writer, "{t}")?;
            skipped += 1;
            continue;
        };

        let sig_s = obj
            .get("signature")
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let fee_payer_s = obj
            .get("fee_payer_pubkey")
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());

        if args.only_signed_rows && (sig_s.is_none() || fee_payer_s.is_none()) {
            writeln!(writer, "{}", serde_json::to_string(&v)?)?;
            skipped += 1;
            continue;
        }

        let Some(sig_s) = sig_s else {
            writeln!(writer, "{}", serde_json::to_string(&v)?)?;
            skipped += 1;
            continue;
        };
        let Some(fee_payer_s) = fee_payer_s else {
            writeln!(writer, "{}", serde_json::to_string(&v)?)?;
            skipped += 1;
            continue;
        };

        let sig = match Signature::from_str(sig_s) {
            Ok(s) => s,
            Err(_) => {
                writeln!(writer, "{}", serde_json::to_string(&v)?)?;
                failed += 1;
                continue;
            }
        };
        let fee_payer = match Pubkey::from_str(fee_payer_s) {
            Ok(p) => p,
            Err(_) => {
                writeln!(writer, "{}", serde_json::to_string(&v)?)?;
                failed += 1;
                continue;
            }
        };

        // Fetch parsed tx JSON and recompute per-mint deltas.
        let (_fee, _slot, _pre, _post, _delta, tx_json) = enrich_tx_costs(&rpc, &sig, &fee_payer).await;
        if let Some(tx) = tx_json.as_ref() {
            let deltas = fee_payer_token_deltas_by_mint(tx, &fee_payer);
            if deltas.as_object().is_some_and(|m| !m.is_empty()) {
                obj.insert("fee_payer_token_deltas".to_string(), deltas);
                obj.insert(
                    "fee_payer_token_deltas_enriched".to_string(),
                    serde_json::Value::Bool(true),
                );
                rewritten += 1;
            } else {
                // Remove stale field if it exists (make "missing" explicit).
                obj.remove("fee_payer_token_deltas");
                obj.insert(
                    "fee_payer_token_deltas_enriched".to_string(),
                    serde_json::Value::Bool(true),
                );
                rewritten += 1;
            }
        } else {
            failed += 1;
        }

        writeln!(writer, "{}", serde_json::to_string(&v)?)?;

        if args.sleep_ms > 0 {
            sleep(Duration::from_millis(args.sleep_ms)).await;
        }
    }

    writer.flush()?;

    println!("enrich-lifecycle-ledger done:");
    println!("  input:  {}", input.display());
    println!("  output: {}", output.display());
    println!("  total_lines_seen: {total}");
    println!("  rewritten: {rewritten}");
    println!("  skipped: {skipped}");
    println!("  failed: {failed}");
    Ok(())
}

