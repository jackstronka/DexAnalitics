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
    DecreaseLiquidityParam, IncreaseLiquidityParam, SwapInstructions, SwapType,
    WhirlpoolsConfigInput, close_position_instructions, decrease_liquidity_instructions,
    harvest_position_instructions, increase_liquidity_instructions,
    open_full_range_position_instructions, open_position_instructions_with_tick_bounds,
    set_whirlpools_config_address, swap_instructions,
};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_program_pack::Pack;
use solana_sdk::{
    instruction::Instruction, pubkey::Pubkey, signature::Keypair, signature::Signature,
    signer::Signer, transaction::Transaction,
};
use solana_system_interface::instruction as system_instruction;
use spl_associated_token_account::get_associated_token_address;
use spl_associated_token_account::instruction::create_associated_token_account;
use spl_token::state::Account as SplTokenAccount;
use std::str::FromStr;
use std::sync::Arc;
use tracing::{debug, info, warn};

fn is_whirlpool_custom_code(err: &str, code: u32) -> bool {
    // Errors are produced by `RpcProvider::send_and_confirm_transaction` and typically include:
    // `... InstructionError(N, Custom(X)) ... | ix_program=...whirL... | custom_code=X`
    // Be robust to formatting differences.
    let needle = format!("custom_code={code}");
    (err.contains(&needle) || err.contains(&format!("Custom({code})")))
        && err.contains(WHIRLPOOL_PROGRAM_ID)
}

fn env_retry_attempts(var: &str, fallback: u8) -> u8 {
    std::env::var(var)
        .ok()
        .and_then(|s| s.trim().parse::<u8>().ok())
        .filter(|&n| (1..=50).contains(&n))
        .unwrap_or(fallback)
}

/// Default min-out slippage for `close_position` when the caller passes `None` (basis points).
/// Prefer keeping this **low** to limit worse-than-quoted token amounts; if Whirlpool returns
/// **6018** (`TokenMinSubceeded`), retry with a higher value or set `WHIRLPOOL_CLOSE_SLIPPAGE_BPS`.
fn default_close_slippage_bps() -> u16 {
    const FALLBACK: u16 = 100;
    match std::env::var("WHIRLPOOL_CLOSE_SLIPPAGE_BPS") {
        Ok(s) => s
            .trim()
            .parse::<u16>()
            .ok()
            .filter(|&n| n <= 10_000)
            .unwrap_or(FALLBACK),
        Err(_) => FALLBACK,
    }
}

/// Orca Whirlpool program ID (mainnet).
pub const WHIRLPOOL_PROGRAM_ID: &str = "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc";

/// Token program ID.
pub const TOKEN_PROGRAM_ID: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";

/// Wrapped SOL mint (SPL).
pub const WSOL_MINT: &str = "So11111111111111111111111111111111111111112";

/// Extra lamports to wrap beyond `token_max` so rounding / on-chain paths do not hit `InsufficientFunds`
/// on the WSOL leg (Orca still debits SPL token balance, not “max - epsilon” in all CPI paths).
fn wsol_deposit_target_with_buffer(needed_amount: u64) -> u64 {
    if needed_amount == 0 {
        return 0;
    }
    needed_amount
        .saturating_add(needed_amount.saturating_mul(50) / 10_000)
        .saturating_add(50_000)
}

fn wsol_deposit_target_exact(needed_amount: u64) -> u64 {
    needed_amount
}

fn open_native_sol_pad_lamports() -> u64 {
    std::env::var("CLMM_MIN_OPEN_SOL_LAMPORTS")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(12_000_000)
}

fn sol_first_auto_unwrap_enabled() -> bool {
    std::env::var("CLMM_SOL_FIRST_AUTO_UNWRAP")
        .ok()
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "y"))
        .unwrap_or(true)
}

fn sol_first_keep_wsol_min_raw() -> u64 {
    std::env::var("CLMM_SOL_FIRST_KEEP_WSOL_MIN_RAW")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0)
}

fn parse_insufficient_lamports_from_log_line(line: &str) -> Option<(u64, u64)> {
    let marker = "Transfer: insufficient lamports ";
    let idx = line.find(marker)?;
    let tail = &line[idx + marker.len()..];
    let (have_s, need_part) = tail.split_once(", need ")?;
    let have = have_s.trim().parse::<u64>().ok()?;
    let need = need_part.trim().parse::<u64>().ok()?;
    Some((have, need))
}

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

/// Parameters for opening a **full-range** position (Orca Splash / max range for spacing).
#[derive(Debug, Clone)]
pub struct OpenFullRangeParams {
    /// Pool address.
    pub pool: Pubkey,
    /// Amount of token A to deposit (max).
    pub amount_a: u64,
    /// Amount of token B to deposit (max).
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
    /// For collect/harvest: quoted fee owed (pool token A raw) at instruction build time.
    pub collect_fee_owed_a_raw: Option<u64>,
    /// For collect/harvest: quoted fee owed (pool token B raw) at instruction build time.
    pub collect_fee_owed_b_raw: Option<u64>,
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
            collect_fee_owed_a_raw: None,
            collect_fee_owed_b_raw: None,
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
            collect_fee_owed_a_raw: None,
            collect_fee_owed_b_raw: None,
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

    async fn read_spl_token_amount_opt(&self, ata: &Pubkey) -> Result<u64> {
        let opt = self
            .provider
            .get_multiple_accounts(&[*ata])
            .await
            .context("fetch SPL token account")?
            .into_iter()
            .next()
            .flatten();
        let Some(acct) = opt else {
            return Ok(0);
        };
        let parsed = SplTokenAccount::unpack(&acct.data).context("unpack SPL token account")?;
        Ok(parsed.amount)
    }

    pub async fn read_wsol_balance_raw(&self, owner: &Pubkey) -> Result<u64> {
        let mint = Pubkey::from_str(WSOL_MINT).expect("valid WSOL mint");
        let ata = get_associated_token_address(owner, &mint);
        self.read_spl_token_amount_opt(&ata).await
    }

    fn compute_unwrap_rewrap_amount(current_wsol: u64, amount_wsol_lamports: u64) -> Result<u64> {
        if amount_wsol_lamports > current_wsol {
            anyhow::bail!(
                "wsol unwrap failed: insufficient WSOL balance (have {current_wsol} raw, need {amount_wsol_lamports} raw)"
            );
        }
        Ok(current_wsol.saturating_sub(amount_wsol_lamports))
    }

    fn compute_wrap_target_from_delta(current_wsol: u64, amount_wsol_lamports: u64) -> Result<u64> {
        current_wsol
            .checked_add(amount_wsol_lamports)
            .ok_or_else(|| anyhow::anyhow!("wsol wrap failed: target overflow"))
    }

    /// Fail fast with a clear message if the **signing wallet** cannot cover raw deposit caps.
    async fn preflight_open_liquidity_balances(
        &self,
        minimal: &crate::orca::whirlpool::WhirlpoolMinimal,
        amount_a: u64,
        amount_b: u64,
        payer: &Keypair,
    ) -> Result<()> {
        let wsol = Pubkey::from_str(WSOL_MINT).expect("valid WSOL mint");
        let owner = payer.pubkey();

        let check_non_wsol = |mint: Pubkey, need: u64, leg: &'static str| async move {
            if need == 0 {
                return Ok(());
            }
            let ata = get_associated_token_address(&owner, &mint);
            let bal = self.read_spl_token_amount_opt(&ata).await?;
            if bal < need {
                anyhow::bail!(
                    "open preflight: insufficient SPL balance on {leg} (mint {mint}): have {bal} raw, need {need} raw \
                     — fund the API wallet’s token account for this mint or lower Amount. (owner {owner})"
                );
            }
            Ok(())
        };

        let check_wsol = |need: u64, _leg: &'static str| async move {
            if need == 0 {
                return Ok::<(), anyhow::Error>(());
            }
            Ok::<(), anyhow::Error>(())
        };

        if amount_a > 0 {
            if minimal.token_mint_a == wsol {
                check_wsol(amount_a, "token A").await?;
            } else {
                check_non_wsol(minimal.token_mint_a, amount_a, "token A").await?;
            }
        }
        if amount_b > 0 {
            if minimal.token_mint_b == wsol {
                check_wsol(amount_b, "token B").await?;
            } else {
                check_non_wsol(minimal.token_mint_b, amount_b, "token B").await?;
            }
        }
        Ok(())
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

        let pool_acct = self
            .provider
            .get_account(&params.pool)
            .await
            .context("fetch whirlpool account (open preflight / WSOL wrap)")?;
        let minimal = crate::orca::whirlpool::parse_whirlpool_minimal(&pool_acct.data)
            .context("parse whirlpool mints (open preflight / WSOL wrap)")?;

        self.preflight_open_liquidity_balances(&minimal, params.amount_a, params.amount_b, payer)
            .await?;

        // If a pool leg is WSOL, the operator often has enough native SOL but **0 WSOL tokens**.
        // Orca SDK will use WSOL ATA for transfers; without pre-wrapping this fails with Tokenkeg `InsufficientFunds`.
        let mut pre_ix: Vec<Instruction> = Vec::new();
        if params.amount_a > 0 || params.amount_b > 0 {
            let wsol = Pubkey::from_str(WSOL_MINT).expect("valid WSOL mint");
            if minimal.token_mint_a == wsol {
                pre_ix.extend(
                    self.ensure_wsol_ata_funded(params.amount_a, payer, true)
                        .await?,
                );
            }
            if minimal.token_mint_b == wsol {
                pre_ix.extend(
                    self.ensure_wsol_ata_funded(params.amount_b, payer, true)
                        .await?,
                );
            }
        }
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

        let mut all_ix = pre_ix;
        all_ix.extend(opened.instructions);
        let mut res = self
            .send_transaction_with_signers(&all_ix, payer, &opened.additional_signers)
            .await?;
        if res.success {
            res.created_position = Some(position_pda);
            let _ = self
                .maybe_auto_unwrap_wsol_to_native(payer, "open_position")
                .await;
        }
        Ok(res)
    }

    /// Opens a **full-range** position (tick bounds from pool spacing; Splash-compatible).
    pub async fn open_full_range_position(
        &self,
        params: &OpenFullRangeParams,
        payer: &Keypair,
    ) -> Result<ExecutionResult> {
        info!(
            pool = %params.pool,
            "Opening full-range position (orca_whirlpools SDK)"
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

        let pool_acct = self
            .provider
            .get_account(&params.pool)
            .await
            .context("fetch whirlpool account (open preflight / WSOL wrap)")?;
        let minimal = crate::orca::whirlpool::parse_whirlpool_minimal(&pool_acct.data)
            .context("parse whirlpool mints (open preflight / WSOL wrap)")?;

        self.preflight_open_liquidity_balances(&minimal, params.amount_a, params.amount_b, payer)
            .await?;

        let mut pre_ix: Vec<Instruction> = Vec::new();
        if params.amount_a > 0 || params.amount_b > 0 {
            let wsol = Pubkey::from_str(WSOL_MINT).expect("valid WSOL mint");
            if minimal.token_mint_a == wsol {
                pre_ix.extend(
                    self.ensure_wsol_ata_funded(params.amount_a, payer, true)
                        .await?,
                );
            }
            if minimal.token_mint_b == wsol {
                pre_ix.extend(
                    self.ensure_wsol_ata_funded(params.amount_b, payer, true)
                        .await?,
                );
            }
        }

        let opened = open_full_range_position_instructions(
            &rpc,
            params.pool,
            IncreaseLiquidityParam {
                token_max_a: params.amount_a,
                token_max_b: params.amount_b,
            },
            Some(params.slippage_bps),
            Some(payer.pubkey()),
        )
        .await
        .map_err(|e| anyhow::anyhow!("orca open_full_range_position_instructions failed: {e}"))?;

        let whirlpool_program = Pubkey::from_str(WHIRLPOOL_PROGRAM_ID)
            .map_err(|e| anyhow::anyhow!("invalid whirlpool program id: {e}"))?;
        let (position_pda, _) = Pubkey::find_program_address(
            &[b"position", opened.position_mint.as_ref()],
            &whirlpool_program,
        );

        let mut all_ix = pre_ix;
        all_ix.extend(opened.instructions);
        let mut res = self
            .send_transaction_with_signers(&all_ix, payer, &opened.additional_signers)
            .await?;
        if res.success {
            res.created_position = Some(position_pda);
            let _ = self
                .maybe_auto_unwrap_wsol_to_native(payer, "open_full_range_position")
                .await;
        }
        Ok(res)
    }

    async fn ensure_wsol_ata_funded(
        &self,
        needed_amount: u64,
        payer: &Keypair,
        with_buffer: bool,
    ) -> Result<Vec<Instruction>> {
        if needed_amount == 0 {
            return Ok(Vec::new());
        }

        let target_amount = if with_buffer {
            wsol_deposit_target_with_buffer(needed_amount)
        } else {
            wsol_deposit_target_exact(needed_amount)
        };
        if target_amount == 0 {
            return Ok(Vec::new());
        }

        let mint = Pubkey::from_str(WSOL_MINT).expect("valid WSOL mint");
        let owner = payer.pubkey();
        let ata = get_associated_token_address(&owner, &mint);

        let acct_opt = self
            .provider
            .get_multiple_accounts(&[ata])
            .await
            .context("fetch WSOL ATA account")?
            .into_iter()
            .next()
            .flatten();

        let mut ixs: Vec<Instruction> = Vec::new();

        if acct_opt.is_none() {
            ixs.push(create_associated_token_account(
                &payer.pubkey(),
                &owner,
                &mint,
                &spl_token::id(),
            ));
        }

        let current_amount = if let Some(acct) = acct_opt {
            let parsed =
                SplTokenAccount::unpack(&acct.data).context("unpack WSOL token account")?;
            parsed.amount
        } else {
            0
        };
        if current_amount >= target_amount {
            return Ok(ixs);
        }

        let topup = target_amount - current_amount;
        ixs.push(system_instruction::transfer(&payer.pubkey(), &ata, topup));
        ixs.push(
            spl_token::instruction::sync_native(&spl_token::id(), &ata)
                .context("build sync_native")?,
        );
        Ok(ixs)
    }

    async fn maybe_auto_unwrap_wsol_to_native(
        &self,
        payer: &Keypair,
        context: &'static str,
    ) -> Result<()> {
        if !sol_first_auto_unwrap_enabled() {
            return Ok(());
        }
        let keep_min = sol_first_keep_wsol_min_raw();
        let current = self.read_wsol_balance_raw(&payer.pubkey()).await.unwrap_or(0);
        if current <= keep_min {
            return Ok(());
        }
        let unwrap_raw = current.saturating_sub(keep_min);
        match self.submit_wsol_unwrap_with_signature(unwrap_raw, payer).await {
            Ok(unwrap_sig) => {
                info!(
                    context,
                    unwrap_raw,
                    keep_min,
                    unwrap_signature = %unwrap_sig,
                    "SOL-first auto-unwrap after successful operation"
                );
            }
            Err(e) => {
                warn!(context, unwrap_raw, keep_min, error = %e, "SOL-first auto-unwrap skipped");
            }
        }
        Ok(())
    }

    /// Transfer native SOL into the owner's wSOL ATA + `sync_native` so SPL balance is usable by Orca swaps.
    /// No-op if the ATA already holds `needed_wsol_lamports` (or `needed_wsol_lamports == 0`).
    pub async fn submit_wsol_wrap_if_needed(
        &self,
        needed_wsol_lamports: u64,
        payer: &Keypair,
    ) -> Result<()> {
        let _ = self
            .submit_wsol_wrap_with_signature_if_needed(needed_wsol_lamports, payer)
            .await?;
        Ok(())
    }

    pub async fn submit_wsol_wrap_with_signature_if_needed(
        &self,
        needed_wsol_lamports: u64,
        payer: &Keypair,
    ) -> Result<Option<Signature>> {
        if needed_wsol_lamports == 0 {
            return Ok(None);
        }
        let ixs = self
            .ensure_wsol_ata_funded(needed_wsol_lamports, payer, true)
            .await?;
        if ixs.is_empty() {
            return Ok(None);
        }
        let res = self
            .send_transaction_with_signers(&ixs, payer, &[])
            .await
            .map_err(|e| anyhow::anyhow!("wsol wrap send: {e}"))?;
        if !res.success {
            let msg = res
                .error
                .unwrap_or_else(|| "wsol wrap transaction failed".to_string());
            anyhow::bail!("wsol wrap failed: {msg}");
        }
        Ok(Some(res.signature))
    }

    /// Wrap exactly `amount_wsol_lamports` from native SOL into WSOL ATA (delta mode).
    pub async fn submit_wsol_wrap_with_signature_delta(
        &self,
        amount_wsol_lamports: u64,
        payer: &Keypair,
    ) -> Result<Option<Signature>> {
        if amount_wsol_lamports == 0 {
            return Ok(None);
        }
        let owner = payer.pubkey();
        let mint = Pubkey::from_str(WSOL_MINT).expect("valid WSOL mint");
        let ata = get_associated_token_address(&owner, &mint);
        let current_wsol = self.read_spl_token_amount_opt(&ata).await?;
        let target = Self::compute_wrap_target_from_delta(current_wsol, amount_wsol_lamports)?;
        self.submit_wsol_wrap_with_signature_if_needed(target, payer)
            .await
    }

    /// Convert WSOL -> native SOL.
    ///
    /// - full unwrap: close WSOL ATA (single tx)
    /// - partial unwrap: close WSOL ATA, then re-wrap the remainder (best-effort)
    pub async fn submit_wsol_unwrap_with_signature(
        &self,
        amount_wsol_lamports: u64,
        payer: &Keypair,
    ) -> Result<Signature> {
        if amount_wsol_lamports == 0 {
            anyhow::bail!("wsol unwrap amount must be > 0");
        }
        let owner = payer.pubkey();
        let mint = Pubkey::from_str(WSOL_MINT).expect("valid WSOL mint");
        let ata = get_associated_token_address(&owner, &mint);
        let current_wsol = self.read_spl_token_amount_opt(&ata).await?;
        if current_wsol == 0 {
            anyhow::bail!("wsol unwrap failed: WSOL balance is 0");
        }
        let rewrap = Self::compute_unwrap_rewrap_amount(current_wsol, amount_wsol_lamports)?;
        let ix = spl_token::instruction::close_account(&spl_token::id(), &ata, &owner, &owner, &[])
            .context("build close_account for WSOL ATA")?;
        let res = self
            .send_transaction_with_signers(&[ix], payer, &[])
            .await
            .map_err(|e| anyhow::anyhow!("wsol unwrap send: {e}"))?;
        if !res.success {
            let msg = res
                .error
                .unwrap_or_else(|| "wsol unwrap transaction failed".to_string());
            anyhow::bail!("wsol unwrap failed: {msg}");
        }
        if rewrap > 0 {
            let ixs = self
                .ensure_wsol_ata_funded(rewrap, payer, false)
                .await
                .context("prepare partial unwrap remainder re-wrap")?;
            if !ixs.is_empty() {
                let rewrap_res = self
                    .send_transaction_with_signers(&ixs, payer, &[])
                    .await
                    .map_err(|e| anyhow::anyhow!("wsol partial re-wrap send: {e}"))?;
                if !rewrap_res.success {
                    let msg = rewrap_res
                        .error
                        .unwrap_or_else(|| "wsol partial re-wrap transaction failed".to_string());
                    anyhow::bail!(
                        "wsol unwrap partial: close succeeded but remainder re-wrap failed: {msg}"
                    );
                }
            }
        }
        Ok(res.signature)
    }

    /// Single-pool Orca swap (**ExactIn**) — same pool you will add liquidity to (e.g. rebalance token mix before open).
    pub async fn swap_exact_in(
        &self,
        pool: Pubkey,
        specified_mint: Pubkey,
        mut amount: u64,
        slippage_bps: u16,
        payer: &Keypair,
    ) -> Result<ExecutionResult> {
        if amount == 0 {
            anyhow::bail!("swap amount must be > 0");
        }
        info!(
            pool = %pool,
            specified_mint = %specified_mint,
            amount = amount,
            slippage_bps = slippage_bps,
            "Orca swap ExactIn in pool"
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

        let wsol = Pubkey::from_str(WSOL_MINT).map_err(|e| anyhow::anyhow!("wsol mint: {e}"))?;
        let mut pre_ix: Vec<Instruction> = Vec::new();
        if specified_mint == wsol {
            // When the WSOL ATA is short, we wrap native SOL via a SystemProgram transfer into the ATA.
            // Runtime evidence shows this can fail with:
            // "Transfer: insufficient lamports X, need Y" (Instruction 0).
            // To avoid repeated failures in swap-mix, clamp ExactIn amount to what the wallet can actually fund.
            let owner = payer.pubkey();
            let ata = get_associated_token_address(&owner, &wsol);
            let current_wsol = self.read_spl_token_amount_opt(&ata).await.unwrap_or(0);
            let native = self.provider.get_balance(&owner).await.unwrap_or(0);
            // Leave headroom for fees/rent; this is intentionally conservative.
            let wrap_budget = native.saturating_sub(open_native_sol_pad_lamports());
            let max_possible_in = current_wsol.saturating_add(wrap_budget);
            if amount > max_possible_in {
                amount = max_possible_in.max(1);
            }
            // Match `open_position`: Orca swap pulls **SPL wSOL** from the ATA; native SOL in the wallet is invisible.
            pre_ix.extend(
                // For swaps, avoid buffer-based wrap overshooting native SOL by a few lamports.
                // We only need to cover the actual ExactIn amount.
                self.ensure_wsol_ata_funded(amount, payer, false)
                    .await
                    .context("ensure wSOL ATA before ExactIn (specified_mint = wSOL)")?,
            );
        }
        let _pre_ix_len = pre_ix.len();

        // Retry without raising slippage: rebuild instructions from fresh pool state.
        // This addresses transient `min-out` / price-move windows that can surface as Custom(1)
        // on the Whirlpool program for swap ix (runtime evidence).
        let max_attempts: u8 = env_retry_attempts("WHIRLPOOL_SWAP_RETRY_ATTEMPTS", 8);
        let mut last: Option<ExecutionResult> = None;
        for attempt in 1..=max_attempts {
            let swap_ix: SwapInstructions = swap_instructions(
                &rpc,
                pool,
                amount,
                specified_mint,
                SwapType::ExactIn,
                Some(slippage_bps),
                Some(payer.pubkey()),
            )
            .await
            .map_err(|e| anyhow::anyhow!("orca swap_instructions failed: {e}"))?;

            let _swap_ix_len = swap_ix.instructions.len();
            let mut all_ix = pre_ix.clone();
            all_ix.extend(swap_ix.instructions);

            let res = self
                .send_transaction_with_signers(&all_ix, payer, &swap_ix.additional_signers)
                .await?;

            if res.success {
                let _ = self
                    .maybe_auto_unwrap_wsol_to_native(payer, "swap_exact_in")
                    .await;
                return Ok(res);
            }

            let err = res.error.clone().unwrap_or_default();
            if is_whirlpool_custom_code(&err, 1) {
                info!(
                    attempt,
                    max_attempts = max_attempts,
                    error = %err,
                    "swap_exact_in: Whirlpool custom_code=1; retrying with rebuilt instructions (same slippage)"
                );
                last = Some(res);
                continue;
            }

            return Ok(res);
        }

        Ok(last.unwrap_or_else(|| {
            ExecutionResult::failure(Signature::default(), "swap failed".into())
        }))
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
        let mut exec = self
            .send_transaction_with_signers(
                &harvested.instructions,
                payer,
                &harvested.additional_signers,
            )
            .await?;
        exec.collect_fee_owed_a_raw = Some(harvested.fees_quote.fee_owed_a);
        exec.collect_fee_owed_b_raw = Some(harvested.fees_quote.fee_owed_b);
        Ok(exec)
    }

    /// Closes a position.
    ///
    /// `slippage_bps`: passed to Orca `close_position_instructions` (min token amounts out). `None`
    /// uses **`WHIRLPOOL_CLOSE_SLIPPAGE_BPS`** env if set (valid `0..=10000`), otherwise **100** bps (1%).
    /// Keep slippage **as low as confirms**; if you see **6018** (`TokenMinSubceeded`), raise only for
    /// that attempt (CLI `--slippage-bps`, or env on the API host).
    pub async fn close_position(
        &self,
        position: &Pubkey,
        _pool: &Pubkey,
        payer: &Keypair,
        slippage_bps: Option<u16>,
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

        let slip = slippage_bps.or(Some(default_close_slippage_bps()));

        // Retry without raising slippage: rebuild instructions from fresh pool state.
        // Whirlpool can return 6018 (`TokenMinSubceeded`) for close when the pool moved between
        // instruction construction and execution.
        let max_attempts: u8 = env_retry_attempts("WHIRLPOOL_CLOSE_RETRY_ATTEMPTS", 8);
        let mut last: Option<ExecutionResult> = None;
        for attempt in 1..=max_attempts {
            let closed =
                close_position_instructions(&rpc, parsed.position_mint, slip, Some(payer.pubkey()))
                    .await
                    .map_err(|e| anyhow::anyhow!("orca close_position_instructions failed: {e}"))?;

            let res = self
                .send_transaction_with_signers(
                    &closed.instructions,
                    payer,
                    &closed.additional_signers,
                )
                .await?;

            if res.success {
                let _ = self
                    .maybe_auto_unwrap_wsol_to_native(payer, "close_position")
                    .await;
                return Ok(res);
            }

            let err = res.error.clone().unwrap_or_default();
            if is_whirlpool_custom_code(&err, 6018) {
                info!(
                    attempt,
                    max_attempts = max_attempts,
                    error = %err,
                    "close_position: Whirlpool custom_code=6018; retrying with rebuilt instructions (same slippage)"
                );
                last = Some(res);
                continue;
            }
            return Ok(res);
        }

        Ok(last.unwrap_or_else(|| {
            ExecutionResult::failure(Signature::default(), "close failed".into())
        }))
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

        if let Ok(sim) = self.provider.simulate_transaction(&transaction).await
            && let Some(logs) = sim.logs
        {
            let parsed = logs
                .iter()
                .find_map(|line| parse_insufficient_lamports_from_log_line(line));
            if let Some((have_lamports, need_lamports)) = parsed {
                let required_with_margin =
                    need_lamports.saturating_mul(101).saturating_add(99) / 100;
                let payer_pubkey = payer.pubkey();
                let native_balance = self
                    .provider
                    .get_balance(&payer_pubkey)
                    .await
                    .unwrap_or(have_lamports);
                if native_balance < required_with_margin {
                    let signature = transaction.signatures.first().copied().unwrap_or_default();
                    return Ok(ExecutionResult::failure(
                        signature,
                        format!(
                            "open preflight exact-plan: insufficient native SOL. \
                             Runtime simulation requires {need_lamports} lamports; with 1% safety margin require {required_with_margin}. \
                             Current native balance {native_balance}. Top up SOL or lower Amount."
                        ),
                    ));
                }
            }
        }

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

    #[test]
    fn test_compute_unwrap_rewrap_amount_full() {
        let current = 301_550_000u64;
        let requested = current;
        let rewrap = WhirlpoolExecutor::compute_unwrap_rewrap_amount(current, requested).unwrap();
        assert_eq!(rewrap, 0);
    }

    #[test]
    fn test_compute_unwrap_rewrap_amount_partial() {
        let current = 301_550_000u64;
        let requested = 50_000_000u64;
        let rewrap = WhirlpoolExecutor::compute_unwrap_rewrap_amount(current, requested).unwrap();
        assert_eq!(rewrap, 251_550_000);
    }

    #[test]
    fn test_compute_unwrap_rewrap_amount_insufficient() {
        let current = 30_000_000u64;
        let requested = 50_000_000u64;
        let err = WhirlpoolExecutor::compute_unwrap_rewrap_amount(current, requested)
            .expect_err("expected insufficient WSOL error");
        let msg = err.to_string();
        assert!(msg.contains("insufficient WSOL balance"));
        assert!(msg.contains("have 30000000 raw"));
        assert!(msg.contains("need 50000000 raw"));
    }

    #[test]
    fn test_compute_wrap_target_from_delta() {
        let current = 200_000_000u64;
        let delta = 50_000_000u64;
        let target = WhirlpoolExecutor::compute_wrap_target_from_delta(current, delta).unwrap();
        assert_eq!(target, 250_000_000u64);
    }

    #[test]
    fn test_compute_wrap_target_from_delta_overflow() {
        let err = WhirlpoolExecutor::compute_wrap_target_from_delta(u64::MAX, 1)
            .expect_err("expected overflow error");
        assert!(err.to_string().contains("target overflow"));
    }
}
