//! Whirlpool executor for on-chain operations.
//!
//! Provides functionality to execute LP operations on Orca Whirlpools:
//! - Open positions
//! - Increase/decrease liquidity
//! - Collect fees
//! - Close positions

use crate::rpc::RpcProvider;
use anyhow::{Context, Result};
use borsh::BorshDeserialize;
use orca_whirlpools::{
    DecreaseLiquidityParam, IncreaseLiquidityParam, WhirlpoolsConfigInput,
    close_position_instructions, decrease_liquidity_instructions, harvest_position_instructions,
    increase_liquidity_instructions, open_position_instructions_with_tick_bounds,
    set_whirlpools_config_address,
};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{
    instruction::Instruction, pubkey::Pubkey, signature::Keypair, signature::Signature,
    signer::Signer, transaction::Transaction,
};
use std::str::FromStr;
use std::sync::Arc;
use tracing::{debug, info};

/// Orca Whirlpool program ID (mainnet).
pub const WHIRLPOOL_PROGRAM_ID: &str = "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc";

/// Token program ID.
pub const TOKEN_PROGRAM_ID: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";

/// Associated token program ID.
pub const ASSOCIATED_TOKEN_PROGRAM_ID: &str = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL";

/// System program ID.
pub const SYSTEM_PROGRAM_ID: &str = "11111111111111111111111111111111";

/// Derives the Whirlpool **position** PDA for `(pool, tick_lower, tick_upper)` (NFT metadata address).
#[must_use]
pub fn derive_whirlpool_position_address(
    pool: &Pubkey,
    tick_lower: i32,
    tick_upper: i32,
) -> Pubkey {
    let program_id = Pubkey::from_str(WHIRLPOOL_PROGRAM_ID).expect("valid whirlpool program id");
    let (position_mint, _) = Pubkey::find_program_address(
        &[
            b"position_mint",
            pool.as_ref(),
            &tick_lower.to_le_bytes(),
            &tick_upper.to_le_bytes(),
        ],
        &program_id,
    );
    let (position, _) =
        Pubkey::find_program_address(&[b"position", position_mint.as_ref()], &program_id);
    position
}

/// Parameters for opening a new position.
#[derive(Debug, Clone)]
pub struct OpenPositionParams {
    /// Pool address.
    pub pool: Pubkey,
    /// Lower tick bound.
    pub tick_lower: i32,
    /// Upper tick bound.
    pub tick_upper: i32,
    /// Amount of token A to deposit.
    pub amount_a: u64,
    /// Amount of token B to deposit.
    pub amount_b: u64,
    /// Slippage tolerance in basis points.
    pub slippage_bps: u16,
}

/// Parameters for increasing liquidity.
#[derive(Debug, Clone)]
pub struct IncreaseLiquidityParams {
    /// Position address.
    pub position: Pubkey,
    /// Pool address.
    pub pool: Pubkey,
    /// Liquidity amount to add.
    pub liquidity_amount: u128,
    /// Maximum token A amount.
    pub token_max_a: u64,
    /// Maximum token B amount.
    pub token_max_b: u64,
}

/// Parameters for decreasing liquidity.
#[derive(Debug, Clone)]
pub struct DecreaseLiquidityParams {
    /// Position address.
    pub position: Pubkey,
    /// Pool address.
    pub pool: Pubkey,
    /// Liquidity amount to remove.
    pub liquidity_amount: u128,
    /// Minimum token A amount.
    pub token_min_a: u64,
    /// Minimum token B amount.
    pub token_min_b: u64,
}

/// Result of an execution operation.
#[derive(Debug, Clone)]
pub struct ExecutionResult {
    /// Transaction signature.
    pub signature: Signature,
    /// Whether the transaction was successful.
    pub success: bool,
    /// Slot at which the transaction was confirmed.
    pub slot: Option<u64>,
    /// Error message if failed.
    pub error: Option<String>,
    /// Position PDA created by open-position flow (if applicable).
    pub created_position: Option<Pubkey>,
}

impl ExecutionResult {
    /// Creates a successful result.
    #[must_use]
    pub fn success(signature: Signature, slot: u64) -> Self {
        Self {
            signature,
            success: true,
            slot: Some(slot),
            error: None,
            created_position: None,
        }
    }

    /// Creates a failed result.
    #[must_use]
    pub fn failure(signature: Signature, error: String) -> Self {
        Self {
            signature,
            success: false,
            slot: None,
            error: Some(error),
            created_position: None,
        }
    }
}

/// Executor for Orca Whirlpool operations.
pub struct WhirlpoolExecutor {
    /// RPC provider for blockchain interaction.
    provider: Arc<RpcProvider>,
}

impl WhirlpoolExecutor {
    /// Creates a new WhirlpoolExecutor.
    pub fn new(provider: Arc<RpcProvider>) -> Self {
        Self { provider }
    }

    /// Opens a new position in a Whirlpool.
    ///
    /// # Arguments
    /// * `params` - Position parameters
    /// * `payer` - Transaction payer and position owner
    ///
    /// # Returns
    /// Execution result with transaction signature.
    pub async fn open_position(
        &self,
        params: &OpenPositionParams,
        payer: &Keypair,
    ) -> Result<ExecutionResult> {
        info!(
            pool = %params.pool,
            tick_lower = params.tick_lower,
            tick_upper = params.tick_upper,
            "Opening new position (orca_whirlpools SDK)"
        );

        let endpoint = self.provider.current_endpoint().await;
        let config = if endpoint.contains("devnet") {
            WhirlpoolsConfigInput::SolanaDevnet
        } else {
            WhirlpoolsConfigInput::SolanaMainnet
        };
        set_whirlpools_config_address(config)
            .map_err(|e| anyhow::anyhow!("orca set_whirlpools_config_address failed: {e}"))?;
        let rpc = RpcClient::new(endpoint);

        let opened = open_position_instructions_with_tick_bounds(
            &rpc,
            params.pool,
            params.tick_lower,
            params.tick_upper,
            IncreaseLiquidityParam {
                token_max_a: params.amount_a,
                token_max_b: params.amount_b,
            },
            Some(params.slippage_bps),
            Some(payer.pubkey()),
        )
        .await
        .map_err(|e| anyhow::anyhow!("orca open_position_instructions failed: {e}"))?;

        let whirlpool_program = Pubkey::from_str(WHIRLPOOL_PROGRAM_ID)
            .map_err(|e| anyhow::anyhow!("invalid whirlpool program id: {e}"))?;
        let (position_pda, _) = Pubkey::find_program_address(
            &[b"position", opened.position_mint.as_ref()],
            &whirlpool_program,
        );

        let mut res = self
            .send_transaction_with_signers(&opened.instructions, payer, &opened.additional_signers)
            .await?;
        if res.success {
            res.created_position = Some(position_pda);
        }
        Ok(res)
    }

    /// Increases liquidity in an existing position.
    pub async fn increase_liquidity(
        &self,
        params: &IncreaseLiquidityParams,
        payer: &Keypair,
    ) -> Result<ExecutionResult> {
        info!(
            position = %params.position,
            liquidity = params.liquidity_amount,
            "Increasing liquidity"
        );
        let endpoint = self.provider.current_endpoint().await;
        let config = if endpoint.contains("devnet") {
            WhirlpoolsConfigInput::SolanaDevnet
        } else {
            WhirlpoolsConfigInput::SolanaMainnet
        };
        set_whirlpools_config_address(config)
            .map_err(|e| anyhow::anyhow!("orca set_whirlpools_config_address failed: {e}"))?;
        let rpc = RpcClient::new(endpoint);

        // SDK requires position mint; fetch & deserialize position account to get it.
        let acct = self
            .provider
            .get_account(&params.position)
            .await
            .context("fetch position account")?;
        let parsed = crate::orca::position_reader::WhirlpoolPosition::try_from_slice(&acct.data)
            .context("parse WhirlpoolPosition (borsh)")?;

        let inc = increase_liquidity_instructions(
            &rpc,
            parsed.position_mint,
            IncreaseLiquidityParam {
                token_max_a: params.token_max_a,
                token_max_b: params.token_max_b,
            },
            Some(100),
            Some(payer.pubkey()),
        )
        .await
        .map_err(|e| anyhow::anyhow!("orca increase_liquidity_instructions failed: {e}"))?;

        self.send_transaction_with_signers(&inc.instructions, payer, &inc.additional_signers)
            .await
    }

    /// Decreases liquidity from an existing position.
    pub async fn decrease_liquidity(
        &self,
        params: &DecreaseLiquidityParams,
        payer: &Keypair,
    ) -> Result<ExecutionResult> {
        info!(
            position = %params.position,
            liquidity = params.liquidity_amount,
            "Decreasing liquidity"
        );
        let endpoint = self.provider.current_endpoint().await;
        let config = if endpoint.contains("devnet") {
            WhirlpoolsConfigInput::SolanaDevnet
        } else {
            WhirlpoolsConfigInput::SolanaMainnet
        };
        set_whirlpools_config_address(config)
            .map_err(|e| anyhow::anyhow!("orca set_whirlpools_config_address failed: {e}"))?;
        let rpc = RpcClient::new(endpoint);

        let acct = self
            .provider
            .get_account(&params.position)
            .await
            .context("fetch position account")?;
        let parsed = crate::orca::position_reader::WhirlpoolPosition::try_from_slice(&acct.data)
            .context("parse WhirlpoolPosition (borsh)")?;

        let dec = decrease_liquidity_instructions(
            &rpc,
            parsed.position_mint,
            DecreaseLiquidityParam::Liquidity(params.liquidity_amount),
            Some(100),
            Some(payer.pubkey()),
        )
        .await
        .map_err(|e| anyhow::anyhow!("orca decrease_liquidity_instructions failed: {e}"))?;

        self.send_transaction_with_signers(&dec.instructions, payer, &dec.additional_signers)
            .await
    }

    /// Collects fees from a position.
    pub async fn collect_fees(
        &self,
        position: &Pubkey,
        _pool: &Pubkey,
        payer: &Keypair,
    ) -> Result<ExecutionResult> {
        info!(position = %position, "Collecting fees");
        let endpoint = self.provider.current_endpoint().await;
        let config = if endpoint.contains("devnet") {
            WhirlpoolsConfigInput::SolanaDevnet
        } else {
            WhirlpoolsConfigInput::SolanaMainnet
        };
        set_whirlpools_config_address(config)
            .map_err(|e| anyhow::anyhow!("orca set_whirlpools_config_address failed: {e}"))?;
        let rpc = RpcClient::new(endpoint);

        let acct = self
            .provider
            .get_account(position)
            .await
            .context("fetch position account")?;
        let parsed = crate::orca::position_reader::WhirlpoolPosition::try_from_slice(&acct.data)
            .context("parse WhirlpoolPosition (borsh)")?;

        let harvested =
            harvest_position_instructions(&rpc, parsed.position_mint, Some(payer.pubkey()))
                .await
                .map_err(|e| anyhow::anyhow!("orca harvest_position_instructions failed: {e}"))?;

        self.send_transaction_with_signers(
            &harvested.instructions,
            payer,
            &harvested.additional_signers,
        )
        .await
    }

    /// Closes a position.
    pub async fn close_position(
        &self,
        position: &Pubkey,
        _pool: &Pubkey,
        payer: &Keypair,
    ) -> Result<ExecutionResult> {
        info!(position = %position, "Closing position");
        let endpoint = self.provider.current_endpoint().await;
        let config = if endpoint.contains("devnet") {
            WhirlpoolsConfigInput::SolanaDevnet
        } else {
            WhirlpoolsConfigInput::SolanaMainnet
        };
        set_whirlpools_config_address(config)
            .map_err(|e| anyhow::anyhow!("orca set_whirlpools_config_address failed: {e}"))?;
        let rpc = RpcClient::new(endpoint);

        let acct = self
            .provider
            .get_account(position)
            .await
            .context("fetch position account")?;
        let parsed = crate::orca::position_reader::WhirlpoolPosition::try_from_slice(&acct.data)
            .context("parse WhirlpoolPosition (borsh)")?;

        let closed = close_position_instructions(
            &rpc,
            parsed.position_mint,
            Some(100),
            Some(payer.pubkey()),
        )
        .await
        .map_err(|e| anyhow::anyhow!("orca close_position_instructions failed: {e}"))?;

        self.send_transaction_with_signers(&closed.instructions, payer, &closed.additional_signers)
            .await
    }

    /// Simulates a transaction without broadcasting.
    pub async fn simulate_transaction<S: Signer>(
        &self,
        instructions: &[Instruction],
        payer: &S,
    ) -> Result<bool> {
        debug!(
            "Simulating transaction with {} instructions",
            instructions.len()
        );

        let recent_blockhash = self
            .provider
            .get_latest_blockhash()
            .await
            .context("Failed to get recent blockhash")?;

        let transaction = Transaction::new_signed_with_payer(
            instructions,
            Some(&payer.pubkey()),
            &[payer],
            recent_blockhash,
        );

        let result = self
            .provider
            .simulate_transaction(&transaction)
            .await
            .context("Failed to simulate transaction")?;

        if let Some(err) = result.err {
            debug!("Simulation failed: {:?}", err);
            return Ok(false);
        }

        debug!("Simulation successful");
        Ok(true)
    }

    // NOTE: Instruction building is delegated to `orca_whirlpools` SDK.

    async fn send_transaction_with_signers(
        &self,
        instructions: &[Instruction],
        payer: &Keypair,
        additional_signers: &[Keypair],
    ) -> Result<ExecutionResult> {
        let recent_blockhash = self
            .provider
            .get_latest_blockhash()
            .await
            .context("Failed to get recent blockhash")?;

        let mut transaction = Transaction::new_with_payer(instructions, Some(&payer.pubkey()));
        let mut signers: Vec<&Keypair> = Vec::with_capacity(1 + additional_signers.len());
        signers.push(payer);
        for kp in additional_signers {
            signers.push(kp);
        }
        transaction.sign(&signers, recent_blockhash);

        debug!("Sending transaction...");

        match self
            .provider
            .send_and_confirm_transaction(&transaction)
            .await
        {
            Ok(signature) => {
                info!(signature = %signature, "Transaction confirmed");
                // Get slot from transaction status
                let slot = self.provider.get_slot().await.unwrap_or(0);
                Ok(ExecutionResult::success(signature, slot))
            }
            Err(e) => {
                let signature = transaction.signatures.first().copied().unwrap_or_default();
                Ok(ExecutionResult::failure(signature, e.to_string()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_program_ids() {
        assert!(Pubkey::from_str(WHIRLPOOL_PROGRAM_ID).is_ok());
        assert!(Pubkey::from_str(TOKEN_PROGRAM_ID).is_ok());
        assert!(Pubkey::from_str(ASSOCIATED_TOKEN_PROGRAM_ID).is_ok());
    }

    #[test]
    fn test_execution_result() {
        let sig = Signature::default();

        let success = ExecutionResult::success(sig, 12345);
        assert!(success.success);
        assert_eq!(success.slot, Some(12345));
        assert!(success.error.is_none());
        assert!(success.created_position.is_none());

        let failure = ExecutionResult::failure(sig, "test error".to_string());
        assert!(!failure.success);
        assert!(failure.slot.is_none());
        assert_eq!(failure.error, Some("test error".to_string()));
        assert!(failure.created_position.is_none());
    }
}
