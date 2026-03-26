//! Orca read service.
//!
//! - REST (orca public API) for discovery/ranking/metadata: **skeleton** (stubbed).
//! - On-chain reads fallback via `WhirlpoolReader` / `PositionReader`: implemented.

use anyhow::bail;
use clmm_lp_protocols::prelude::RpcProvider;
use clmm_lp_protocols::prelude::{
    OnChainPosition, PositionReader, WhirlpoolReader, WhirlpoolState,
};
use std::sync::Arc;

/// Query params for `GET /pools` (Orca REST).
#[derive(Debug, Clone, Default)]
pub struct ListPoolsQuery {
    pub sort_by: Option<String>,
    pub sort_direction: Option<String>,
    pub size: Option<u32>,
    pub next_cursor: Option<String>,
    pub previous_cursor: Option<String>,
    pub min_tvl: Option<f64>,
    pub min_volume: Option<f64>,
    pub token: Option<String>,
    pub tokens_both_of: Option<String>,
    pub addresses: Option<String>,
    pub stats: Option<String>,
    pub has_rewards: Option<bool>,
    pub has_adaptive_fee: Option<bool>,
    pub include_blocked: Option<bool>,
}

/// Query params for `GET /pools/search` (Orca REST).
#[derive(Debug, Clone, Default)]
pub struct SearchPoolsQuery {
    pub q: String,
    pub size: Option<u32>,
    pub next_cursor: Option<String>,
    pub min_tvl: Option<f64>,
    pub min_volume: Option<f64>,
    pub stats: Option<String>,
    pub verified_only: Option<bool>,
}

/// Paged REST response wrapper.
#[derive(Debug, Clone)]
pub struct Paged<T> {
    pub data: Vec<T>,
    pub next: Option<String>,
    pub previous: Option<String>,
}

/// Minimal pool details DTO (REST).
///
/// Note: REST response schema is richer; keep DTO minimal until we lock exact fields.
#[derive(Debug, Clone)]
pub struct OrcaPoolDetails {
    pub address: String,
    pub tick_spacing: u16,
    pub token_mint_a: String,
    pub token_mint_b: String,
    pub tick_current_index: i32,
    pub price: String,
    pub tvl_usdc: String,
}

/// Minimal Whirlpool lock info DTO (REST).
#[derive(Debug, Clone)]
pub struct OrcaLockInfo {
    pub name: String,
    pub locked_percentage: String,
}

/// Service facade: REST + on-chain reads.
pub struct OrcaReadService {
    provider: Arc<RpcProvider>,
    whirlpool_reader: WhirlpoolReader,
    position_reader: PositionReader,
    rest_base_url: String,
}

impl OrcaReadService {
    pub fn new(provider: Arc<RpcProvider>) -> Self {
        Self {
            whirlpool_reader: WhirlpoolReader::new(provider.clone()),
            position_reader: PositionReader::new(provider.clone()),
            provider,
            rest_base_url: "https://api.orca.so/v2/solana".to_string(),
        }
    }

    // ===== REST (skeleton) =====

    /// `GET /pools` (Orca REST).
    pub async fn list_pools(&self, _q: ListPoolsQuery) -> anyhow::Result<Paged<OrcaPoolDetails>> {
        bail!("Orca REST not implemented yet in skeleton");
    }

    /// `GET /pools/search` (Orca REST).
    pub async fn search_pools(
        &self,
        _q: SearchPoolsQuery,
    ) -> anyhow::Result<Paged<OrcaPoolDetails>> {
        bail!("Orca REST not implemented yet in skeleton");
    }

    /// `GET /pools/{address}` (Orca REST).
    pub async fn get_pool(&self, _address: &str) -> anyhow::Result<OrcaPoolDetails> {
        bail!("Orca REST not implemented yet in skeleton");
    }

    /// `GET /lock/{address}` (Orca REST).
    pub async fn get_pool_lock_info(&self, _address: &str) -> anyhow::Result<Vec<OrcaLockInfo>> {
        bail!("Orca REST not implemented yet in skeleton");
    }

    // ===== On-chain reads (implemented) =====

    /// On-chain state: Whirlpool data (source of truth for tx preflight).
    pub async fn get_pool_state_onchain(
        &self,
        pool_address: &str,
    ) -> anyhow::Result<WhirlpoolState> {
        self.whirlpool_reader.get_pool_state(pool_address).await
    }

    /// On-chain position state (Whirlpool position NFT metadata).
    pub async fn get_position_onchain(
        &self,
        position_address: &str,
    ) -> anyhow::Result<OnChainPosition> {
        self.position_reader.get_position(position_address).await
    }

    /// On-chain positions by owner.
    pub async fn get_positions_by_owner_onchain(
        &self,
        owner: &str,
    ) -> anyhow::Result<Vec<OnChainPosition>> {
        self.position_reader.get_positions_by_owner(owner).await
    }
}
