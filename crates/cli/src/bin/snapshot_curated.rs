use clap::{Parser, Subcommand};

// Reuse the existing snapshot collection logic (RPC + JSONL appends)
// without going through the large `clmm-lp-cli` command enum.
#[path = "../snapshots/collector.rs"]
mod collector;

#[derive(Parser, Debug)]
#[command(
    name = "snapshot-curated",
    about = "Curated snapshot collectors (isolated CLI)"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    RaydiumSnapshotCurated {
        /// Optional: stop after N pools (useful for testing)
        #[arg(long)]
        limit: Option<usize>,
    },
    MeteoraSnapshotCurated {
        /// Optional: stop after N pools (useful for testing)
        #[arg(long)]
        limit: Option<usize>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::RaydiumSnapshotCurated { limit } => {
            collector::raydium_snapshot_curated(limit).await
        }
        Commands::MeteoraSnapshotCurated { limit } => {
            collector::meteora_snapshot_curated(limit).await
        }
    }
}
