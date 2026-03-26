//! Transaction manager for lifecycle handling.

use super::TransactionResult;
use anyhow::Result;
use clmm_lp_protocols::prelude::RpcProvider;
use clmm_lp_protocols::rpc::SignatureStatusInfo;
use solana_sdk::signature::Signature;
use solana_sdk::transaction::Transaction;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

/// Configuration for transaction management.
#[derive(Debug, Clone)]
pub struct TransactionConfig {
    /// Maximum retries for sending.
    pub max_retries: u32,
    /// Base delay for retry backoff in milliseconds.
    pub retry_base_delay_ms: u64,
    /// Confirmation timeout in seconds.
    pub confirmation_timeout_secs: u64,
    /// Whether to simulate before sending.
    pub simulate_before_send: bool,
}

impl Default for TransactionConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            retry_base_delay_ms: 500,
            confirmation_timeout_secs: 60,
            simulate_before_send: true,
        }
    }
}

/// Manages transaction lifecycle.
pub struct TransactionManager {
    /// RPC provider.
    provider: Arc<RpcProvider>,
    /// Configuration.
    config: TransactionConfig,
}

impl TransactionManager {
    /// Creates a new transaction manager.
    pub fn new(provider: Arc<RpcProvider>, config: TransactionConfig) -> Self {
        Self { provider, config }
    }

    /// Sends a transaction with retry logic.
    pub async fn send_transaction(&self, transaction: &Transaction) -> Result<Signature> {
        let mut last_error = None;

        for attempt in 0..=self.config.max_retries {
            if attempt > 0 {
                let delay = self.config.retry_base_delay_ms * 2u64.pow(attempt - 1);
                debug!(attempt = attempt, delay_ms = delay, "Retrying transaction");
                sleep(Duration::from_millis(delay)).await;
            }

            match self.try_send_transaction(transaction).await {
                Ok(signature) => {
                    info!(signature = %signature, "Transaction sent successfully");
                    return Ok(signature);
                }
                Err(e) => {
                    warn!(
                        attempt = attempt,
                        error = %e,
                        "Transaction send failed"
                    );
                    last_error = Some(e);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("Unknown error")))
    }

    /// Tries to send a transaction once.
    async fn try_send_transaction(&self, transaction: &Transaction) -> Result<Signature> {
        if self.config.simulate_before_send {
            let sim = self.simulate(transaction).await?;
            if !sim.success {
                let sim_err = sim
                    .error
                    .unwrap_or_else(|| "simulation failed without error details".to_string());
                return Err(anyhow::anyhow!(
                    "Transaction simulation failed: {}",
                    sim_err
                ));
            }
        }

        self.provider.send_transaction(transaction).await
    }

    /// Waits for transaction confirmation.
    pub async fn wait_for_confirmation(&self, signature: &Signature) -> Result<TransactionResult> {
        let start = Instant::now();
        let timeout = Duration::from_secs(self.config.confirmation_timeout_secs);

        info!(signature = %signature, "Waiting for confirmation");

        loop {
            if start.elapsed() > timeout {
                return Err(anyhow::anyhow!("Confirmation timeout"));
            }

            match self.check_confirmation(signature).await {
                Ok(Some(mut result)) => {
                    result.confirmation_time = start.elapsed();
                    info!(
                        signature = %signature,
                        slot = result.slot,
                        time_ms = result.confirmation_time.as_millis(),
                        "Transaction confirmed"
                    );
                    return Ok(result);
                }
                Ok(None) => {
                    // Not confirmed yet
                    sleep(Duration::from_millis(500)).await;
                }
                Err(e) => {
                    error!(signature = %signature, error = %e, "Confirmation check failed");
                    return Err(e);
                }
            }
        }
    }

    /// Checks if a transaction is confirmed.
    async fn check_confirmation(&self, signature: &Signature) -> Result<Option<TransactionResult>> {
        let status = self.provider.get_signature_status(signature).await?;
        map_signature_status(signature, status)
    }

    /// Sends and confirms a transaction.
    pub async fn send_and_confirm(&self, transaction: &Transaction) -> Result<TransactionResult> {
        // If requested, do an explicit preflight simulation before sending.
        if self.config.simulate_before_send {
            let sim = self.simulate(transaction).await?;
            if !sim.success {
                let sim_err = sim
                    .error
                    .unwrap_or_else(|| "simulation failed without error details".to_string());
                return Err(anyhow::anyhow!(
                    "Transaction simulation failed: {}",
                    sim_err
                ));
            }
        }

        let signature = self.send_transaction(transaction).await?;
        self.wait_for_confirmation(&signature).await
    }

    /// Simulates a transaction.
    pub async fn simulate(&self, transaction: &Transaction) -> Result<SimulationResult> {
        let result = self.provider.simulate_transaction(transaction).await?;
        let logs = result.logs.unwrap_or_default();
        let compute_units = result.units_consumed.unwrap_or(0);

        if let Some(err) = result.err {
            return Ok(SimulationResult {
                success: false,
                logs,
                compute_units,
                error: Some(format!("{:?}", err)),
            });
        }

        Ok(SimulationResult {
            success: true,
            logs,
            compute_units,
            error: None,
        })
    }
}

/// Result of transaction simulation.
#[derive(Debug, Clone)]
pub struct SimulationResult {
    /// Whether simulation succeeded.
    pub success: bool,
    /// Simulation logs.
    pub logs: Vec<String>,
    /// Compute units consumed.
    pub compute_units: u64,
    /// Error message if failed.
    pub error: Option<String>,
}

fn map_signature_status(
    signature: &Signature,
    status: Option<SignatureStatusInfo>,
) -> Result<Option<TransactionResult>> {
    match status {
        Some(status) => {
            // A present status with no error is considered confirmed for our lifecycle.
            if let Some(err) = status.err {
                return Err(anyhow::anyhow!("Transaction failed: {:?}", err));
            }

            Ok(Some(TransactionResult {
                signature: *signature,
                slot: status.slot,
                confirmation_time: Duration::from_millis(0),
                compute_units: None,
                fee: 0,
            }))
        }
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::instruction::InstructionError;
    use solana_sdk::transaction::TransactionError;

    #[test]
    fn map_signature_status_none_is_pending() {
        let sig = Signature::default();
        let out = map_signature_status(&sig, None).expect("mapping should succeed");
        assert!(out.is_none());
    }

    #[test]
    fn map_signature_status_ok_is_confirmed() {
        let sig = Signature::default();
        let out = map_signature_status(
            &sig,
            Some(SignatureStatusInfo {
                slot: 42,
                err: None,
            }),
        )
        .expect("mapping should succeed");
        let out = out.expect("status should be confirmed");
        assert_eq!(out.signature, sig);
        assert_eq!(out.slot, 42);
    }

    #[test]
    fn map_signature_status_err_is_failure() {
        let sig = Signature::default();
        let err = map_signature_status(
            &sig,
            Some(SignatureStatusInfo {
                slot: 42,
                err: Some(TransactionError::InstructionError(
                    0,
                    InstructionError::Custom(123),
                )),
            }),
        )
        .expect_err("should map to failure");
        assert!(err.to_string().contains("Transaction failed"));
    }
}
