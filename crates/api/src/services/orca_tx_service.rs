//! Orca tx service (on-chain writes).
//!
//! Skeleton facade around `WhirlpoolExecutor` so the rest of the codebase can
//! talk to Orca through one contract (`OrcaTxService`) with shared preflight policy.

use anyhow::{Context, Result};
use clmm_lp_execution::prelude::Wallet;
use clmm_lp_protocols::prelude::RpcProvider;
use clmm_lp_protocols::prelude::{
    DecreaseLiquidityParams, ExecutionResult, IncreaseLiquidityParams, OpenPositionParams,
    WhirlpoolExecutor,
};
use solana_sdk::pubkey::Pubkey;
use std::sync::Arc;

/// Request to open a new position.
#[derive(Debug, Clone)]
pub struct OpenPositionTxRequest {
    pub pool_address: String,
    pub tick_lower: i32,
    pub tick_upper: i32,
    pub amount_a: u64,
    pub amount_b: u64,
    pub slippage_bps: u16,
}

/// Request to increase liquidity.
#[derive(Debug, Clone)]
pub struct IncreaseLiquidityTxRequest {
    pub position_address: String,
    pub pool_address: String,
    pub liquidity_amount: u128,
    pub token_max_a: u64,
    pub token_max_b: u64,
}

/// Request to decrease liquidity.
#[derive(Debug, Clone)]
pub struct DecreaseLiquidityTxRequest {
    pub position_address: String,
    pub pool_address: String,
    pub liquidity_amount: u128,
    pub token_min_a: u64,
    pub token_min_b: u64,
}

/// Request to collect fees.
#[derive(Debug, Clone)]
pub struct CollectFeesTxRequest {
    pub position_address: String,
    pub pool_address: String,
}

/// Request to close position.
#[derive(Debug, Clone)]
pub struct ClosePositionTxRequest {
    pub position_address: String,
    pub pool_address: String,
}

pub struct OrcaTxService {
    executor: WhirlpoolExecutor,
    wallet: Option<Arc<Wallet>>,
}

impl OrcaTxService {
    pub fn new(provider: Arc<RpcProvider>) -> Self {
        Self {
            executor: WhirlpoolExecutor::new(provider),
            wallet: None,
        }
    }

    /// Set wallet used to sign transactions.
    pub fn set_wallet(&mut self, wallet: Arc<Wallet>) {
        self.wallet = Some(wallet);
    }

    fn wallet(&self) -> Result<&Arc<Wallet>> {
        self.wallet
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("OrcaTxService wallet not set"))
    }

    pub async fn open_position(&self, req: OpenPositionTxRequest) -> Result<ExecutionResult> {
        let wallet = self.wallet()?;
        let pool = req
            .pool_address
            .parse::<Pubkey>()
            .context("invalid pool_address pubkey")?;

        let params = OpenPositionParams {
            pool,
            tick_lower: req.tick_lower,
            tick_upper: req.tick_upper,
            amount_a: req.amount_a,
            amount_b: req.amount_b,
            slippage_bps: req.slippage_bps,
        };

        self.executor
            .open_position(&params, wallet.keypair())
            .await
            .context("orca open_position RPC failed")
    }

    pub async fn increase_liquidity(
        &self,
        req: IncreaseLiquidityTxRequest,
    ) -> Result<ExecutionResult> {
        let wallet = self.wallet()?;
        let position = req
            .position_address
            .parse::<Pubkey>()
            .context("invalid position_address pubkey")?;
        let pool = req
            .pool_address
            .parse::<Pubkey>()
            .context("invalid pool_address pubkey")?;

        let params = IncreaseLiquidityParams {
            position,
            pool,
            liquidity_amount: req.liquidity_amount,
            token_max_a: req.token_max_a,
            token_max_b: req.token_max_b,
        };

        self.executor
            .increase_liquidity(&params, wallet.keypair())
            .await
            .context("orca increase_liquidity RPC failed")
    }

    pub async fn decrease_liquidity(
        &self,
        req: DecreaseLiquidityTxRequest,
    ) -> Result<ExecutionResult> {
        let wallet = self.wallet()?;
        let position = req
            .position_address
            .parse::<Pubkey>()
            .context("invalid position_address pubkey")?;
        let pool = req
            .pool_address
            .parse::<Pubkey>()
            .context("invalid pool_address pubkey")?;

        let params = DecreaseLiquidityParams {
            position,
            pool,
            liquidity_amount: req.liquidity_amount,
            token_min_a: req.token_min_a,
            token_min_b: req.token_min_b,
        };

        self.executor
            .decrease_liquidity(&params, wallet.keypair())
            .await
            .context("orca decrease_liquidity RPC failed")
    }

    pub async fn collect_fees(&self, req: CollectFeesTxRequest) -> Result<ExecutionResult> {
        let wallet = self.wallet()?;
        let position = req
            .position_address
            .parse::<Pubkey>()
            .context("invalid position_address pubkey")?;
        let pool = req
            .pool_address
            .parse::<Pubkey>()
            .context("invalid pool_address pubkey")?;

        self.executor
            .collect_fees(&position, &pool, wallet.keypair())
            .await
            .context("orca collect_fees RPC failed")
    }

    pub async fn close_position(&self, req: ClosePositionTxRequest) -> Result<ExecutionResult> {
        let wallet = self.wallet()?;
        let position = req
            .position_address
            .parse::<Pubkey>()
            .context("invalid position_address pubkey")?;
        let pool = req
            .pool_address
            .parse::<Pubkey>()
            .context("invalid pool_address pubkey")?;

        self.executor
            .close_position(&position, &pool, wallet.keypair())
            .await
            .context("orca close_position RPC failed")
    }
}
