//! Position service for executing position operations.

use crate::error::ApiError;
use crate::models::{OpenPositionRequest, RebalanceRequest, SwapBeforeOpenRequest};
use crate::state::{AlertUpdate, AppState, PositionUpdate};
use clmm_lp_execution::prelude::{RebalanceParams, RebalanceReason, StrategyExecutor};
use clmm_lp_protocols::ledger::position_registry::registry_path;
use clmm_lp_protocols::orca::position_reader::PositionReader;
use clmm_lp_protocols::prelude::WhirlpoolReader;
use solana_sdk::pubkey::Pubkey;
use std::env;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Result of a position operation.
#[derive(Debug, Clone)]
pub struct OperationResult {
    /// Whether the operation was successful.
    pub success: bool,
    /// Transaction signature if applicable.
    pub signature: Option<String>,
    /// Error message if failed.
    pub error: Option<String>,
    /// Additional data.
    pub data: Option<serde_json::Value>,
}

impl OperationResult {
    /// Creates a successful result.
    pub fn success() -> Self {
        Self {
            success: true,
            signature: None,
            error: None,
            data: None,
        }
    }

    /// Creates a successful result with signature.
    pub fn success_with_signature(signature: String) -> Self {
        Self {
            success: true,
            signature: Some(signature),
            error: None,
            data: None,
        }
    }

    /// Creates a failed result.
    pub fn failure(error: impl Into<String>) -> Self {
        Self {
            success: false,
            signature: None,
            error: Some(error.into()),
            data: None,
        }
    }

    /// Creates a dry-run result.
    pub fn dry_run(message: impl Into<String>) -> Self {
        Self {
            success: true,
            signature: None,
            error: None,
            data: Some(serde_json::json!({
                "dry_run": true,
                "message": message.into()
            })),
        }
    }

    /// Creates a successful result with additional data.
    pub fn success_with_data(data: serde_json::Value) -> Self {
        Self {
            success: true,
            signature: None,
            error: None,
            data: Some(data),
        }
    }
}

/// Service for position operations.
pub struct PositionService {
    /// Application state.
    state: AppState,
    /// Strategy executor for rebalancing.
    executor: Option<Arc<RwLock<StrategyExecutor>>>,
    /// Pool reader.
    pool_reader: WhirlpoolReader,
    /// Whether in dry-run mode.
    dry_run: bool,
}

impl PositionService {
    /// Creates a new position service.
    pub fn new(state: AppState) -> Self {
        let pool_reader = WhirlpoolReader::new(state.provider.clone());
        Self {
            state,
            executor: None,
            pool_reader,
            dry_run: true, // Default to dry-run for safety
        }
    }

    /// Sets the strategy executor.
    pub fn set_executor(&mut self, executor: Arc<RwLock<StrategyExecutor>>) {
        self.executor = Some(executor);
    }

    /// Enables or disables dry-run mode.
    pub fn set_dry_run(&mut self, dry_run: bool) {
        self.dry_run = dry_run;
    }

    /// Executes an Orca Whirlpool swap (ExactIn) **only** inside a pool.
    ///
    /// This is a building block for a 2-step UI flow: SWAP first, then OPEN.
    pub async fn swap_before_open_exact_in(
        &self,
        request: &SwapBeforeOpenRequest,
    ) -> Result<OperationResult, ApiError> {
        let pool_pubkey = Pubkey::from_str(&request.pool_address)
            .map_err(|_| ApiError::bad_request("Invalid pool address"))?;
        let amount_in = request.amount_in;
        if amount_in == 0 {
            return Err(ApiError::bad_request("amount_in must be greater than 0"));
        }

        let specified_mint = Pubkey::from_str(request.specified_mint.trim())
            .map_err(|_| ApiError::bad_request("Invalid specified_mint"))?;

        info!(
            pool = %request.pool_address,
            specified_mint = %request.specified_mint,
            amount_in = %amount_in,
            dry_run = self.dry_run,
            "Swap before open (ExactIn, swap only)"
        );

        let ledger_session = request
            .cost_session_id
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        if self.dry_run {
            return Ok(OperationResult::dry_run(format!(
                "Would swap_exact_in amount_in={} mint={} in pool {} (no open)",
                request.amount_in, request.specified_mint, request.pool_address
            )));
        }

        // Non-dry-run: fetch pool state to validate that mint is either token A or token B.
        let pool_state = self
            .pool_reader
            .get_pool_state(&request.pool_address)
            .await
            .map_err(|e| ApiError::not_found(format!("Pool not found: {e}")))?;
        if specified_mint != pool_state.token_mint_a && specified_mint != pool_state.token_mint_b {
            return Err(ApiError::Validation(
                "specified_mint must be the pool's token A or B mint".to_string(),
            ));
        }

        let Some(executor) = &self.executor else {
            return Ok(OperationResult::failure(
                "Swap requires executor and wallet configuration",
            ));
        };

        let guard = executor.read().await;
        let sig_opt = guard
            .execute_swap_exact_in(
                &pool_pubkey,
                &specified_mint,
                request.amount_in,
                request.slippage_tolerance_bps,
                ledger_session.clone(),
            )
            .await
            .map_err(|e| ApiError::internal(format!("swap before open failed: {e}")))?;

        let swap_signature = sig_opt.ok_or_else(|| {
            ApiError::internal(
                "swap_exact_in returned no signature (unexpected dry-run)".to_string(),
            )
        })?;

        let mut data = serde_json::json!({
            "swap_signature": swap_signature.to_string(),
        });
        if let Some(ref sid) = ledger_session {
            data["cost_session_id"] = serde_json::json!(sid);
        }

        Ok(OperationResult::success_with_data(data))
    }

    /// Opens a new position.
    pub async fn open_position(
        &self,
        request: &OpenPositionRequest,
    ) -> Result<OperationResult, ApiError> {
        let pool_pubkey = Pubkey::from_str(&request.pool_address)
            .map_err(|_| ApiError::bad_request("Invalid pool address"))?;

        info!(
            pool = %request.pool_address,
            tick_lower = request.tick_lower,
            tick_upper = request.tick_upper,
            full_range = request.full_range,
            "Opening position"
        );

        if !request.full_range {
            // Validate tick range
            if request.tick_lower >= request.tick_upper {
                return Err(ApiError::Validation(
                    "tick_lower must be less than tick_upper".to_string(),
                ));
            }
        }

        if self.dry_run {
            info!("Dry-run mode: would open position");
            if let Some(ref sw) = request.swap_before_open {
                if request.full_range {
                    return Ok(OperationResult::dry_run(format!(
                        "Would swap {} raw units (mint {}) then open full-range in pool {}",
                        sw.amount_in, sw.specified_mint, request.pool_address
                    )));
                }
                return Ok(OperationResult::dry_run(format!(
                    "Would swap {} raw units (mint {}) then open position in pool {} with range [{}, {}]",
                    sw.amount_in,
                    sw.specified_mint,
                    request.pool_address,
                    request.tick_lower,
                    request.tick_upper
                )));
            }
            if request.full_range {
                return Ok(OperationResult::dry_run(format!(
                    "Would open full-range position in pool {}",
                    request.pool_address
                )));
            }
            return Ok(OperationResult::dry_run(format!(
                "Would open position in pool {} with range [{}, {}]",
                request.pool_address, request.tick_lower, request.tick_upper
            )));
        }

        let need_pool_state = request.swap_before_open.is_some() || !request.full_range;
        let pool_state = if need_pool_state {
            Some(
                self.pool_reader
                    .get_pool_state(&request.pool_address)
                    .await
                    .map_err(|e| ApiError::not_found(format!("Pool not found: {}", e)))?,
            )
        } else {
            None
        };

        if let Some(ref sw) = request.swap_before_open {
            if sw.amount_in == 0 {
                return Err(ApiError::bad_request(
                    "swap_before_open.amount_in must be greater than 0",
                ));
            }
            let mint = Pubkey::from_str(sw.specified_mint.trim())
                .map_err(|_| ApiError::bad_request("Invalid swap_before_open.specified_mint"))?;
            let ps = pool_state.as_ref().ok_or_else(|| {
                ApiError::internal("pool state required for swap validation".to_string())
            })?;
            if mint != ps.token_mint_a && mint != ps.token_mint_b {
                return Err(ApiError::Validation(
                    "swap_before_open.specified_mint must be the pool's token A or B mint"
                        .to_string(),
                ));
            }
        }

        if !request.full_range {
            let pool_state = pool_state.as_ref().ok_or_else(|| {
                ApiError::internal("pool state required for tick validation".to_string())
            })?;
            let tick_spacing = pool_state.tick_spacing as i32;
            if request.tick_lower % tick_spacing != 0 || request.tick_upper % tick_spacing != 0 {
                return Err(ApiError::Validation(format!(
                    "Tick bounds must be multiples of tick spacing ({})",
                    tick_spacing
                )));
            }
        }

        // Idempotency: if `cost_session_id` is present, treat it as a request id.
        // If we already have a `registry_open` row with the same session id, return it
        // instead of opening a second position (covers client retries / double-clicks).
        if let Some(sid) = request
            .cost_session_id
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
        {
            if let Some(existing) = find_registry_open_position_by_session_id(&sid) {
                return Ok(OperationResult::success_with_data(serde_json::json!({
                    "message": "Position already opened for this request (idempotent replay)",
                    "position_pda": existing.position_pubkey,
                    "open_signature": existing.signature,
                    "opened_ts_utc": existing.ts_utc,
                    "cost_session_id": sid,
                })));
            }
        }

        let Some(executor) = &self.executor else {
            return Ok(OperationResult::failure(
                "Position opening requires executor and wallet configuration",
            ));
        };

        let ledger_session = request
            .cost_session_id
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let guard = executor.read().await;

        // Guardrail: fail fast if API wallet doesn't have enough SOL to cover rent/fees.
        // This avoids long opaque simulation errors when accounts must be created.
        if let Some(wallet_pk) = guard.wallet_pubkey() {
            let min_lamports = min_open_sol_lamports();
            if min_lamports > 0 {
                match self.state.provider.get_balance(&wallet_pk).await {
                    Ok(have) if have < min_lamports => {
                        let have_sol = (have as f64) / 1e9;
                        let need_sol = (min_lamports as f64) / 1e9;
                        return Err(ApiError::bad_request(format!(
                            "Open position blocked: API wallet has insufficient SOL for rent/fees. \
Have {have} lamports (~{have_sol:.6} SOL), require at least {min_lamports} lamports (~{need_sol:.6} SOL). \
Top up the API wallet and retry."
                        )));
                    }
                    Ok(_) => {}
                    Err(e) => {
                        warn!(error = %e, "Failed to precheck API wallet SOL balance; continuing with open");
                    }
                }
            }
        }

        let mut swap_signature: Option<String> = None;
        if let Some(ref sw) = request.swap_before_open {
            let mint = Pubkey::from_str(sw.specified_mint.trim())
                .map_err(|_| ApiError::bad_request("Invalid swap_before_open.specified_mint"))?;
            let sig_opt = guard
                .execute_swap_exact_in(
                    &pool_pubkey,
                    &mint,
                    sw.amount_in,
                    request.slippage_tolerance_bps,
                    ledger_session.clone(),
                )
                .await
                .map_err(|e| ApiError::internal(format!("swap before open failed: {e}")))?;
            if let Some(s) = sig_opt {
                swap_signature = Some(s.to_string());
            }
        }

        let opened_position = guard
            .execute_open_position(
                &pool_pubkey,
                request.tick_lower,
                request.tick_upper,
                request.amount_a,
                request.amount_b,
                request.slippage_tolerance_bps,
                request.full_range,
                ledger_session.clone(),
            )
            .await
            .map_err(classify_open_position_error)?;

        let mut data = serde_json::json!({
            "position_pda": opened_position.to_string(),
        });
        if let Some(ref sid) = ledger_session {
            data["cost_session_id"] = serde_json::json!(sid);
        }
        if let Some(ref s) = swap_signature {
            data["swap_signature"] = serde_json::json!(s);
        }

        Ok(OperationResult::success_with_data(data))
    }

    /// Closes a position.
    pub async fn close_position(&self, address: &str) -> Result<OperationResult, ApiError> {
        let position_pubkey = Pubkey::from_str(address)
            .map_err(|_| ApiError::bad_request("Invalid position address"))?;

        info!(position = %address, "Closing position");

        // Verify position exists
        let positions = self.state.monitor.get_positions().await;
        let (pool_pubkey, liquidity_for_message) =
            if let Some(p) = positions.iter().find(|p| p.address == position_pubkey) {
                (p.pool, p.on_chain.liquidity)
            } else {
                // Fallback: position might not be in the in-memory monitor (e.g. after API restart).
                // Fetch on-chain position state to learn the pool, then proceed.
                let reader = PositionReader::new(self.state.provider.clone());
                let on_chain = reader
                    .get_position(address)
                    .await
                    .map_err(|e| ApiError::not_found(format!("Position not found: {e}")))?;
                (on_chain.pool, on_chain.liquidity)
            };

        if self.dry_run {
            info!("Dry-run mode: would close position");
            return Ok(OperationResult::dry_run(format!(
                "Would close position {} with liquidity {}",
                address, liquidity_for_message
            )));
        }

        let Some(executor) = &self.executor else {
            return Ok(OperationResult::failure(
                "Position closing requires executor and wallet configuration",
            ));
        };

        let guard = executor.read().await;
        guard
            .execute_full_close_only(&position_pubkey, &pool_pubkey)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;

        Ok(OperationResult::success())
    }

    /// Collects fees from a position.
    pub async fn collect_fees(&self, address: &str) -> Result<OperationResult, ApiError> {
        let position_pubkey = Pubkey::from_str(address)
            .map_err(|_| ApiError::bad_request("Invalid position address"))?;

        info!(position = %address, "Collecting fees");

        // Verify position exists
        let positions = self.state.monitor.get_positions().await;
        let position = positions
            .iter()
            .find(|p| p.address == position_pubkey)
            .ok_or_else(|| ApiError::not_found("Position not found"))?;

        if self.dry_run {
            info!("Dry-run mode: would collect fees");
            return Ok(OperationResult::dry_run(format!(
                "Would collect fees from position {}: {} token A, {} token B",
                address, position.pnl.fees_earned_a, position.pnl.fees_earned_b
            )));
        }

        let Some(executor) = &self.executor else {
            return Ok(OperationResult::failure(
                "Fee collection requires executor and wallet configuration",
            ));
        };

        let guard = executor.read().await;
        guard
            .execute_collect_fees_only(&position_pubkey, &position.pool)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;

        Ok(OperationResult::success())
    }

    /// Rebalances a position.
    pub async fn rebalance_position(
        &self,
        address: &str,
        request: &RebalanceRequest,
    ) -> Result<OperationResult, ApiError> {
        let position_pubkey = Pubkey::from_str(address)
            .map_err(|_| ApiError::bad_request("Invalid position address"))?;

        info!(
            position = %address,
            new_tick_lower = request.new_tick_lower,
            new_tick_upper = request.new_tick_upper,
            "Rebalancing position"
        );

        // Validate tick range
        if request.new_tick_lower >= request.new_tick_upper {
            return Err(ApiError::Validation(
                "new_tick_lower must be less than new_tick_upper".to_string(),
            ));
        }

        // Verify position exists
        let positions = self.state.monitor.get_positions().await;
        let position = positions
            .iter()
            .find(|p| p.address == position_pubkey)
            .ok_or_else(|| ApiError::not_found("Position not found"))?;

        if self.dry_run {
            info!("Dry-run mode: would rebalance position");

            // Broadcast update
            self.state
                .broadcast_position_update(PositionUpdate {
                    update_type: "rebalance_simulated".to_string(),
                    position_address: address.to_string(),
                    timestamp: chrono::Utc::now(),
                    data: serde_json::json!({
                        "old_range": [position.on_chain.tick_lower, position.on_chain.tick_upper],
                        "new_range": [request.new_tick_lower, request.new_tick_upper],
                        "dry_run": true
                    }),
                })
                .await;

            return Ok(OperationResult::dry_run(format!(
                "Would rebalance position {} from [{}, {}] to [{}, {}]",
                address,
                position.on_chain.tick_lower,
                position.on_chain.tick_upper,
                request.new_tick_lower,
                request.new_tick_upper
            )));
        }

        // Fetch pool state
        let pool_state = self
            .pool_reader
            .get_pool_state(&position.pool.to_string())
            .await
            .map_err(|e| ApiError::Internal(format!("Failed to fetch pool state: {}", e)))?;

        // Validate tick spacing
        let tick_spacing = pool_state.tick_spacing as i32;
        if request.new_tick_lower % tick_spacing != 0 || request.new_tick_upper % tick_spacing != 0
        {
            return Err(ApiError::Validation(format!(
                "Tick bounds must be multiples of tick spacing ({})",
                tick_spacing
            )));
        }

        // Execute rebalance if executor is available
        if let Some(executor) = &self.executor {
            let _params = RebalanceParams {
                position: position_pubkey,
                pool: position.pool,
                current_tick_lower: position.on_chain.tick_lower,
                current_tick_upper: position.on_chain.tick_upper,
                new_tick_lower: request.new_tick_lower,
                new_tick_upper: request.new_tick_upper,
                current_liquidity: position.on_chain.liquidity,
                pool_tick_current: pool_state.tick_current,
                pool_sqrt_price: pool_state.sqrt_price,
                reason: RebalanceReason::Manual,
                current_il_pct: position.pnl.il_pct,
                amount_a_before: None,
                amount_b_before: None,
                price_ab_before: None,
                amount_a_after: None,
                amount_b_after: None,
                price_ab_after: None,
                optimization_run_id: None,
            };

            let _executor_guard = executor.read().await;
            // Note: RebalanceExecutor is inside StrategyExecutor, we need to access it
            // For now, we'll use the lifecycle tracker to record the intent

            drop(_executor_guard);

            // Record the rebalance request
            self.state
                .lifecycle
                .record_rebalance(
                    position_pubkey,
                    position.pool,
                    clmm_lp_execution::prelude::RebalanceData {
                        old_tick_lower: position.on_chain.tick_lower,
                        old_tick_upper: position.on_chain.tick_upper,
                        new_tick_lower: request.new_tick_lower,
                        new_tick_upper: request.new_tick_upper,
                        old_liquidity: position.on_chain.liquidity,
                        new_liquidity: position.on_chain.liquidity, // Assuming same liquidity
                        tx_cost_lamports: 0,
                        il_at_rebalance: position.pnl.il_pct,
                        reason: RebalanceReason::Manual,
                        amount_a_before: None,
                        amount_b_before: None,
                        amount_a_after: None,
                        amount_b_after: None,
                        price_ab_before: None,
                        price_ab_after: None,
                        fees_a_collected: None,
                        fees_b_collected: None,
                        optimization_run_id: None,
                        old_position: Some(position_pubkey.to_string()),
                    },
                )
                .await;

            // Broadcast update
            self.state
                .broadcast_position_update(PositionUpdate {
                    update_type: "rebalance_initiated".to_string(),
                    position_address: address.to_string(),
                    timestamp: chrono::Utc::now(),
                    data: serde_json::json!({
                        "old_range": [position.on_chain.tick_lower, position.on_chain.tick_upper],
                        "new_range": [request.new_tick_lower, request.new_tick_upper]
                    }),
                })
                .await;

            // Broadcast alert
            self.state
                .broadcast_alert(AlertUpdate {
                    level: "info".to_string(),
                    message: format!("Rebalance initiated for position {}", address),
                    timestamp: chrono::Utc::now(),
                    position_address: Some(address.to_string()),
                })
                .await;

            info!("Rebalance recorded - actual execution pending wallet configuration");
            return Ok(OperationResult::success());
        }

        warn!("Rebalancing not yet fully implemented");
        Ok(OperationResult::failure(
            "Rebalancing requires executor configuration",
        ))
    }

    /// Increases liquidity in a position.
    pub async fn increase_liquidity(
        &self,
        address: &str,
        amount_a: u64,
        amount_b: u64,
    ) -> Result<OperationResult, ApiError> {
        let position_pubkey = Pubkey::from_str(address)
            .map_err(|_| ApiError::bad_request("Invalid position address"))?;

        info!(
            position = %address,
            amount_a = amount_a,
            amount_b = amount_b,
            "Increasing liquidity"
        );

        // Verify position exists
        let positions = self.state.monitor.get_positions().await;
        let _position = positions
            .iter()
            .find(|p| p.address == position_pubkey)
            .ok_or_else(|| ApiError::not_found("Position not found"))?;

        if self.dry_run {
            return Ok(OperationResult::dry_run(format!(
                "Would increase liquidity in position {} by {} token A and {} token B",
                address, amount_a, amount_b
            )));
        }

        // TODO: Implement actual liquidity increase
        Ok(OperationResult::failure(
            "Liquidity increase requires wallet configuration",
        ))
    }

    /// Decreases liquidity from a position.
    pub async fn decrease_liquidity(
        &self,
        address: &str,
        liquidity_amount: u128,
    ) -> Result<OperationResult, ApiError> {
        let position_pubkey = Pubkey::from_str(address)
            .map_err(|_| ApiError::bad_request("Invalid position address"))?;

        info!(
            position = %address,
            liquidity = liquidity_amount,
            "Decreasing liquidity"
        );

        // Verify position exists
        let positions = self.state.monitor.get_positions().await;
        let position = positions
            .iter()
            .find(|p| p.address == position_pubkey)
            .ok_or_else(|| ApiError::not_found("Position not found"))?;

        if liquidity_amount > position.on_chain.liquidity {
            return Err(ApiError::Validation(
                "Cannot decrease more liquidity than available".to_string(),
            ));
        }

        if self.dry_run {
            return Ok(OperationResult::dry_run(format!(
                "Would decrease liquidity in position {} by {}",
                address, liquidity_amount
            )));
        }

        let Some(executor) = &self.executor else {
            return Ok(OperationResult::failure(
                "Liquidity decrease requires executor and wallet configuration",
            ));
        };

        let guard = executor.read().await;
        match guard
            .execute_partial_decrease_liquidity(&position_pubkey, &position.pool, liquidity_amount)
            .await
        {
            Ok(()) => Ok(OperationResult::success()),
            Err(e) => Ok(OperationResult::failure(e.to_string())),
        }
    }
}

fn min_open_sol_lamports() -> u64 {
    // 0 disables. Default is conservative: 0.01 SOL (covers rent + a few retries).
    const DEFAULT: u64 = 10_000_000; // 0.01 SOL
    env::var("CLMM_MIN_OPEN_SOL_LAMPORTS")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT)
}

fn classify_open_position_error(err: anyhow::Error) -> ApiError {
    // Default conversion would be 500. For user-triggered open failures,
    // return 4xx with a concrete hint when we can.
    // Use alternate formatting to include the full error chain.
    let raw = format!("{err:#}");
    let s = raw.to_lowercase();

    if s.contains("wallet not set")
        || s.contains("requires executor")
        || s.contains("wallet/executor not configured")
    {
        return ApiError::service_unavailable(format!(
            "Open position failed: API host cannot sign transactions (missing wallet/executor). \
Set `KEYPAIR_PATH` (or `SOLANA_KEYPAIR_PATH`) on the server. Detail: {raw}"
        ));
    }

    if s.contains("insufficient funds")
        || s.contains("insufficient lamports")
        || s.contains("insufficient balance")
        || s.contains("insufficient token")
    {
        if let Some((have, need)) = parse_insufficient_lamports(&s) {
            let have_sol = (have as f64) / 1e9;
            let need_sol = (need as f64) / 1e9;
            return ApiError::bad_request(format!(
                "Open position failed: insufficient SOL (lamports) to create required accounts (rent/fees). \
Have {have} lamports (~{have_sol:.6} SOL), need {need} lamports (~{need_sol:.6} SOL). \
Top up SOL on the API wallet, then retry. Detail: {raw}"
            ));
        }
        return ApiError::bad_request(format!(
            "Open position failed: insufficient funds/tokens for requested caps. \
Top up the wallet or lower Amount A/B (or use swap-before-open). Detail: {raw}"
        ));
    }

    if s.contains("tokenminsubceeded")
        || s.contains("token_min_subceeded")
        || s.contains("slippage")
        || s.contains("6018")
        || s.contains("0x1782")
    {
        return ApiError::bad_request(format!(
            "Open position failed: slippage/min-out too tight vs pool move. \
Retry; if it keeps failing, raise `slippage_tolerance_bps` for this open. Detail: {raw}"
        ));
    }

    if s.contains("tick")
        && (s.contains("spacing") || s.contains("invalid") || s.contains("out of bounds"))
    {
        return ApiError::bad_request(format!(
            "Open position failed: invalid tick bounds for this pool (spacing/range). \
Align ticks to pool tick_spacing and keep tick_lower < tick_upper. Detail: {raw}"
        ));
    }

    ApiError::bad_request(format!("Open position failed: {raw}"))
}

fn parse_insufficient_lamports(s_lower: &str) -> Option<(u64, u64)> {
    // From Solana logs: `Transfer: insufficient lamports 313234, need 2770080`
    // Input is already lowercased; we parse digits after the keywords.
    let i = s_lower.find("insufficient lamports")?;
    let tail = &s_lower[i..];
    let have = extract_first_u64_after(tail, "insufficient lamports")?;
    let need = extract_first_u64_after(tail, "need")?;
    Some((have, need))
}

fn extract_first_u64_after(haystack: &str, marker: &str) -> Option<u64> {
    let i = haystack.find(marker)?;
    let mut j = i + marker.len();
    // Skip non-digits
    while j < haystack.len() && !haystack.as_bytes()[j].is_ascii_digit() {
        j += 1;
    }
    let start = j;
    while j < haystack.len() && haystack.as_bytes()[j].is_ascii_digit() {
        j += 1;
    }
    if start == j {
        return None;
    }
    haystack[start..j].parse::<u64>().ok()
}

#[derive(Debug, Clone)]
struct RegistryOpenMatch {
    position_pubkey: String,
    signature: String,
    ts_utc: String,
}

fn find_registry_open_position_by_session_id(session_id: &str) -> Option<RegistryOpenMatch> {
    let path = registry_path();
    let file = File::open(&path).ok()?;
    let reader = BufReader::new(file);

    // Scan from the end would be ideal, but keep it simple and safe:
    // the registry is append-only and typically small; this is only used on `Open Position`.
    for line in reader.lines().filter_map(Result::ok) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if v.get("event").and_then(|x| x.as_str()) != Some("registry_open") {
            continue;
        }
        if v.get("rebalance_session_id").and_then(|x| x.as_str()) != Some(session_id) {
            continue;
        }
        let position_pubkey = v
            .get("position_pubkey")
            .and_then(|x| x.as_str())?
            .to_string();
        let signature = v.get("signature").and_then(|x| x.as_str())?.to_string();
        let ts_utc = v
            .get("ts_utc")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        return Some(RegistryOpenMatch {
            position_pubkey,
            signature,
            ts_utc,
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{ApiConfig, AppState};
    use clmm_lp_protocols::prelude::RpcConfig;

    fn sample_open_position_request(pool: Pubkey) -> OpenPositionRequest {
        OpenPositionRequest {
            pool_address: pool.to_string(),
            tick_lower: 0,
            tick_upper: 64,
            amount_a: 1,
            amount_b: 2,
            slippage_tolerance_bps: 50,
            full_range: false,
            strategy_id: None,
            swap_before_open: None,
            cost_session_id: None,
        }
    }

    #[tokio::test]
    async fn open_position_invalid_range() {
        let state = AppState::new(RpcConfig::default(), ApiConfig::default());
        let svc = PositionService::new(state);
        let pool = Pubkey::new_unique();
        let mut req = sample_open_position_request(pool);
        req.tick_lower = 64;
        req.tick_upper = 64;

        let err = svc.open_position(&req).await.expect_err("must fail");
        assert!(matches!(err, ApiError::Validation(_)));
    }

    #[tokio::test]
    async fn open_position_dry_run_returns_dry_run_data() {
        let state = AppState::new(RpcConfig::default(), ApiConfig::default());
        let svc = PositionService::new(state);
        let pool = Pubkey::new_unique();

        let res = svc
            .open_position(&sample_open_position_request(pool))
            .await
            .expect("dry-run");
        assert!(res.success);
        assert!(
            res.data
                .as_ref()
                .is_some_and(|d| d.get("dry_run").and_then(|v| v.as_bool()) == Some(true))
        );
    }

    #[tokio::test]
    async fn open_position_non_dry_run_without_executor_fails_fast() {
        let state = AppState::new(RpcConfig::default(), ApiConfig::default());
        let mut svc = PositionService::new(state);
        svc.set_dry_run(false);
        let pool = Pubkey::new_unique();
        // `full_range: true` skips RPC pool fetch before the executor check; a random pool
        // would otherwise return NotFound and never reach the intended failure path.
        let mut req = sample_open_position_request(pool);
        req.full_range = true;

        let res = svc.open_position(&req).await.expect("op result");

        assert!(!res.success);
        assert!(
            res.error
                .as_deref()
                .unwrap_or("")
                .contains("executor and wallet configuration")
        );
    }

    #[tokio::test]
    async fn decrease_liquidity_invalid_address() {
        let state = AppState::new(RpcConfig::default(), ApiConfig::default());
        let svc = PositionService::new(state);
        let err = svc
            .decrease_liquidity("not-a-valid-pubkey", 1)
            .await
            .expect_err("bad pubkey");
        assert!(matches!(err, ApiError::BadRequest(_)));
    }

    #[tokio::test]
    async fn decrease_liquidity_position_not_found() {
        let state = AppState::new(RpcConfig::default(), ApiConfig::default());
        let svc = PositionService::new(state);
        let err = svc
            .decrease_liquidity(&Pubkey::new_unique().to_string(), 1)
            .await
            .expect_err("unknown position");
        assert!(matches!(err, ApiError::NotFound(_)));
    }

    #[tokio::test]
    async fn close_position_invalid_address() {
        let state = AppState::new(RpcConfig::default(), ApiConfig::default());
        let svc = PositionService::new(state);
        let err = svc
            .close_position("not-a-valid-pubkey")
            .await
            .expect_err("bad pubkey");
        assert!(matches!(err, ApiError::BadRequest(_)));
    }

    #[tokio::test]
    async fn collect_fees_invalid_address() {
        let state = AppState::new(RpcConfig::default(), ApiConfig::default());
        let svc = PositionService::new(state);
        let err = svc
            .collect_fees("not-a-valid-pubkey")
            .await
            .expect_err("bad pubkey");
        assert!(matches!(err, ApiError::BadRequest(_)));
    }
}
