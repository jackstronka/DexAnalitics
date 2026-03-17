//! Simple pool info abstractions.
//!
//! For now this module does not talk directly to Orca's on-chain program or
//! any external HTTP API. Instead it defines a small `PoolInfo` struct and a
//! trait-based interface that other components (or future integrations) can
//! use. This allows the simulation layer to be wired to real pool statistics
//! without hard-coding a particular data source.

use anyhow::Result;
use rust_decimal::Decimal;

/// Basic information about a CLMM pool used by the simulator.
#[derive(Debug, Clone)]
pub struct PoolInfo {
    /// Human readable name or symbol, e.g. "whETH/SOL".
    pub name: String,
    /// Pool address (e.g. Whirlpool address on Solana).
    pub address: String,
    /// Fee tier in basis points (e.g. 16, 30, 100).
    pub fee_bps: u32,
    /// Current TVL in USD (approximate).
    pub tvl_usd: Decimal,
    /// 24h trading volume in USD (approximate).
    pub volume_24h_usd: Decimal,
    /// Optional 7d trading volume in USD.
    pub volume_7d_usd: Option<Decimal>,
}

/// Abstraction for anything that can provide pool information.
pub trait PoolInfoProvider: Send + Sync {
    fn get_pool_info(&self, address: &str) -> Result<PoolInfo>;
}

