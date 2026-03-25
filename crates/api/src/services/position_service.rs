//! Position service for executing position operations.

use crate::error::ApiError;
use crate::models::{OpenPositionRequest, RebalanceRequest};
use crate::state::{AlertUpdate, AppState, PositionUpdate};
use clmm_lp_execution::prelude::{RebalanceParams, RebalanceReason, StrategyExecutor};
use clmm_lp_protocols::prelude::WhirlpoolReader;
use solana_sdk::pubkey::Pubkey;
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

    /// Opens a new position.
    pub async fn open_position(
        &self,
        request: &OpenPositionRequest,
    ) -> Result<OperationResult, ApiError> {
        let _pool_pubkey = Pubkey::from_str(&request.pool_address)
            .map_err(|_| ApiError::bad_request("Invalid pool address"))?;

        info!(
            pool = %request.pool_address,
            tick_lower = request.tick_lower,
            tick_upper = request.tick_upper,
            "Opening position"
        );

        // Validate tick range
        if request.tick_lower >= request.tick_upper {
            return Err(ApiError::Validation(
                "tick_lower must be less than tick_upper".to_string(),
            ));
        }

        // Fetch pool state to validate
        let pool_state = self
            .pool_reader
            .get_pool_state(&request.pool_address)
            .await
            .map_err(|e| ApiError::not_found(format!("Pool not found: {}", e)))?;

        // Validate tick spacing
        let tick_spacing = pool_state.tick_spacing as i32;
        if request.tick_lower % tick_spacing != 0 || request.tick_upper % tick_spacing != 0 {
            return Err(ApiError::Validation(format!(
                "Tick bounds must be multiples of tick spacing ({})",
                tick_spacing
            )));
        }

        if self.dry_run {
            info!("Dry-run mode: would open position");
            return Ok(OperationResult::dry_run(format!(
                "Would open position in pool {} with range [{}, {}]",
                request.pool_address, request.tick_lower, request.tick_upper
            )));
        }

        // TODO: Implement actual position opening via WhirlpoolExecutor
        // This would require:
        // 1. Create position NFT
        // 2. Increase liquidity
        // 3. Track in lifecycle

        // For now, return a placeholder
        warn!("Position opening not yet fully implemented");
        Ok(OperationResult::failure(
            "Position opening requires wallet configuration",
        ))
    }

    /// Closes a position.
    pub async fn close_position(&self, address: &str) -> Result<OperationResult, ApiError> {
        let position_pubkey = Pubkey::from_str(address)
            .map_err(|_| ApiError::bad_request("Invalid position address"))?;

        info!(position = %address, "Closing position");

        // Verify position exists
        let positions = self.state.monitor.get_positions().await;
        let position = positions
            .iter()
            .find(|p| p.address == position_pubkey)
            .ok_or_else(|| ApiError::not_found("Position not found"))?;

        if self.dry_run {
            info!("Dry-run mode: would close position");
            return Ok(OperationResult::dry_run(format!(
                "Would close position {} with liquidity {}",
                address, position.on_chain.liquidity
            )));
        }

        // TODO: Implement actual position closing
        // This would require:
        // 1. Collect fees
        // 2. Decrease liquidity to 0
        // 3. Close position NFT
        // 4. Record in lifecycle

        warn!("Position closing not yet fully implemented");
        Ok(OperationResult::failure(
            "Position closing requires wallet configuration",
        ))
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

        // TODO: Implement actual fee collection
        // This would use the Whirlpool collect_fees instruction

        warn!("Fee collection not yet fully implemented");
        Ok(OperationResult::failure(
            "Fee collection requires wallet configuration",
        ))
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

        if self.dry_run {
            info!("Dry-run mode: would rebalance position");

            // Broadcast update
            self.state.broadcast_position_update(PositionUpdate {
                update_type: "rebalance_simulated".to_string(),
                position_address: address.to_string(),
                timestamp: chrono::Utc::now(),
                data: serde_json::json!({
                    "old_range": [position.on_chain.tick_lower, position.on_chain.tick_upper],
                    "new_range": [request.new_tick_lower, request.new_tick_upper],
                    "dry_run": true
                }),
            });

            return Ok(OperationResult::dry_run(format!(
                "Would rebalance position {} from [{}, {}] to [{}, {}]",
                address,
                position.on_chain.tick_lower,
                position.on_chain.tick_upper,
                request.new_tick_lower,
                request.new_tick_upper
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
                    },
                )
                .await;

            // Broadcast update
            self.state.broadcast_position_update(PositionUpdate {
                update_type: "rebalance_initiated".to_string(),
                position_address: address.to_string(),
                timestamp: chrono::Utc::now(),
                data: serde_json::json!({
                    "old_range": [position.on_chain.tick_lower, position.on_chain.tick_upper],
                    "new_range": [request.new_tick_lower, request.new_tick_upper]
                }),
            });

            // Broadcast alert
            self.state.broadcast_alert(AlertUpdate {
                level: "info".to_string(),
                message: format!("Rebalance initiated for position {}", address),
                timestamp: chrono::Utc::now(),
                position_address: Some(address.to_string()),
            });

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

        // TODO: Implement actual liquidity decrease
        Ok(OperationResult::failure(
            "Liquidity decrease requires wallet configuration",
        ))
    }
}
