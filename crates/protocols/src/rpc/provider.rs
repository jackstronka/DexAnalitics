//! RPC provider with automatic failover and retry logic.

use super::{HealthChecker, RpcConfig};
use anyhow::{Context, Result};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_client::rpc_client::GetConfirmedSignaturesForAddress2Config;
use solana_client::rpc_config::RpcTransactionConfig;
use solana_client::rpc_request::TokenAccountsFilter;
use solana_client::rpc_response::RpcConfirmedTransactionStatusWithSignature;
use solana_client::rpc_response::RpcKeyedAccount;
use solana_sdk::account::Account;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Signature;
use solana_sdk::transaction::Transaction;
use solana_transaction_status_client_types::{
    EncodedConfirmedTransactionWithStatusMeta, UiTransactionEncoding,
};
use solana_transaction_status_client_types::{TransactionConfirmationStatus, TransactionStatus};
use std::error::Error;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tokio::time::sleep;
use tracing::{debug, info, warn};

fn format_error_chain(err: &(dyn Error + 'static)) -> String {
    let mut s = err.to_string();
    let mut cur = err.source();
    while let Some(e) = cur {
        s.push_str(" | ");
        s.push_str(&e.to_string());
        cur = e.source();
    }
    s
}

/// Solana `getAccount` / JSON-RPC for a missing account — rotating RPC nodes will not fix this.
fn is_definitive_account_not_found(err: &anyhow::Error) -> bool {
    let chain = format_error_chain(err.as_ref());
    chain.contains("AccountNotFound")
        || chain.contains("could not find account")
        || chain.contains("Account does not exist")
        || chain.contains("Invalid param: could not find account")
}

/// Lightweight signature status snapshot returned by `get_signature_status`.
#[derive(Debug, Clone)]
pub struct SignatureStatusInfo {
    /// Slot where the status was observed.
    pub slot: u64,
    /// Transaction error if execution failed.
    pub err: Option<solana_sdk::transaction::TransactionError>,
}

/// RPC provider with automatic failover and health checking.
pub struct RpcProvider {
    /// Configuration.
    config: RpcConfig,
    /// Health checker.
    health: Arc<HealthChecker>,
    /// Current active endpoint index.
    current_endpoint_idx: Arc<RwLock<usize>>,
}

impl RpcProvider {
    /// Creates a new RPC provider with the given configuration.
    ///
    /// When **`CLMM_EXPECTED_CLUSTER`** is set, validates inferable RPC URLs against it (see [`crate::rpc::cluster`]).
    #[must_use]
    pub fn new(config: RpcConfig) -> Self {
        if let Err(e) = super::cluster::enforce_expected_cluster_for_rpc_config(&config) {
            panic!(
                "cluster guard (CLMM_EXPECTED_CLUSTER): {e:#}. \
                 Unset CLMM_EXPECTED_CLUSTER or fix SOLANA_RPC_URL / SOLANA_RPC_FALLBACK_URLS."
            );
        }
        let endpoints = config.all_endpoints();
        if endpoints.is_empty() {
            // Should never happen, but fail-fast to avoid silent loops.
            panic!("RpcConfig produced 0 endpoints (SOLANA_RPC_URL empty?)");
        }
        if endpoints.len() < 2 {
            warn!(
                primary = config.primary_url,
                "RPC config has only 1 endpoint. If it rate-limits or becomes unavailable, snapshot collection will gap. \
                 Consider setting SOLANA_RPC_FALLBACK_URLS and avoid paid/blocked endpoints (402/401/403)."
            );
        }
        Self {
            config,
            health: Arc::new(HealthChecker::new()),
            current_endpoint_idx: Arc::new(RwLock::new(0)),
        }
    }

    /// Creates a new RPC provider for mainnet with default settings.
    #[must_use]
    pub fn mainnet() -> Self {
        Self::new(RpcConfig::default())
    }

    /// Creates a new RPC provider for devnet.
    #[must_use]
    pub fn devnet() -> Self {
        Self::new(RpcConfig::devnet())
    }

    /// Creates a new RPC provider for localhost.
    #[must_use]
    pub fn localhost() -> Self {
        Self::new(RpcConfig::localhost())
    }

    /// Returns the current active endpoint.
    pub async fn current_endpoint(&self) -> String {
        let idx = *self.current_endpoint_idx.read().await;
        let endpoints = self.config.all_endpoints();
        endpoints.get(idx).unwrap_or(&endpoints[0]).to_string()
    }

    /// Returns all configured RPC endpoints in provider priority order.
    #[must_use]
    pub fn all_endpoints(&self) -> Vec<String> {
        self.config
            .all_endpoints()
            .into_iter()
            .map(ToString::to_string)
            .collect()
    }

    /// Gets an RPC client for the current endpoint.
    async fn get_client(&self) -> RpcClient {
        let endpoint = self.current_endpoint().await;
        RpcClient::new_with_timeout(endpoint, self.config.timeout)
    }

    /// Rotates to the next healthy endpoint.
    async fn rotate_endpoint(&self) {
        let endpoints = self.config.all_endpoints();
        let mut idx = self.current_endpoint_idx.write().await;

        for i in 1..=endpoints.len() {
            let next_idx = (*idx + i) % endpoints.len();
            let endpoint = endpoints[next_idx];

            if let Some(reason) = self.health.disabled_reason(endpoint).await {
                debug!(
                    endpoint = endpoint,
                    reason = reason,
                    "Skipping hard-disabled RPC endpoint"
                );
                continue;
            }

            if self.health.is_healthy(endpoint).await {
                info!(
                    from = endpoints[*idx],
                    to = endpoint,
                    "Rotating to new RPC endpoint"
                );
                *idx = next_idx;
                return;
            }
        }

        // All endpoints unhealthy, try the next one anyway
        *idx = (*idx + 1) % endpoints.len();
        warn!("All endpoints unhealthy, rotating anyway");
    }

    /// Executes a request with retry and failover logic.
    async fn execute_with_retry<T, F, Fut>(&self, operation: F) -> Result<T>
    where
        F: Fn(RpcClient) -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let mut last_error = None;
        let mut retry_count = 0;

        while retry_count <= self.config.max_retries {
            let endpoint = self.current_endpoint().await;
            let client = self.get_client().await;
            let start = Instant::now();

            match operation(client).await {
                Ok(result) => {
                    let elapsed = start.elapsed().as_millis() as f64;
                    self.health.record_success(&endpoint, elapsed).await;
                    return Ok(result);
                }
                Err(e) => {
                    if is_definitive_account_not_found(&e) {
                        debug!(
                            endpoint = endpoint,
                            chain = %format_error_chain(e.as_ref()),
                            "RPC error is definitive account missing; skip rotate/retries"
                        );
                        return Err(e);
                    }
                    if let Some(reason) = hard_disable_reason(&e) {
                        warn!(
                            endpoint = endpoint,
                            reason = reason,
                            "Hard-disabling RPC endpoint (will not retry)"
                        );
                        self.health.disable_endpoint(&endpoint, reason).await;
                    }
                    warn!(
                        endpoint = endpoint,
                        retry = retry_count,
                        error = %e,
                        error_full = %format_error_chain(e.as_ref()),
                        "RPC request failed"
                    );
                    self.health.record_failure(&endpoint).await;
                    last_error = Some(e);

                    // Rotate endpoint on failure
                    self.rotate_endpoint().await;

                    // Exponential backoff
                    if retry_count < self.config.max_retries {
                        let delay = calculate_backoff(
                            retry_count,
                            self.config.retry_base_delay_ms,
                            self.config.retry_max_delay_ms,
                        );
                        debug!(delay_ms = delay, "Waiting before retry");
                        sleep(Duration::from_millis(delay)).await;
                    }

                    retry_count += 1;
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("Unknown error")))
    }

    /// Gets the current slot.
    pub async fn get_slot(&self) -> Result<u64> {
        self.execute_with_retry(|client| async move {
            client.get_slot().await.context("Failed to get slot")
        })
        .await
    }

    /// Gets the current block height.
    pub async fn get_block_height(&self) -> Result<u64> {
        self.execute_with_retry(|client| async move {
            client
                .get_block_height()
                .await
                .context("Failed to get block height")
        })
        .await
    }

    /// Largest token accounts holding `mint` (for NFT owner resolution).
    pub async fn get_token_largest_accounts(
        &self,
        mint: &Pubkey,
    ) -> Result<Vec<solana_client::rpc_response::RpcTokenAccountBalance>> {
        let mint = *mint;
        self.execute_with_retry(|client| async move {
            client
                .get_token_largest_accounts(&mint)
                .await
                .context("Failed to get token largest accounts")
        })
        .await
    }

    /// Gets account data for a given address.
    pub async fn get_account(&self, address: &Pubkey) -> Result<Account> {
        let addr = *address;
        self.execute_with_retry(|client| async move {
            client
                .get_account(&addr)
                .await
                .context("Failed to get account")
        })
        .await
    }

    /// Gets token accounts for `owner` filtered by `program_id`, using `jsonParsed` encoding.
    ///
    /// This is used by API read-only wallet balances and benefits from the same retry/failover
    /// logic as other on-chain reads.
    pub async fn get_token_accounts_by_owner_json_parsed(
        &self,
        owner: &Pubkey,
        program_id: &Pubkey,
    ) -> Result<Vec<RpcKeyedAccount>> {
        let owner = *owner;
        let program_id = *program_id;
        self.execute_with_retry(|client| async move {
            client
                // Solana client internally uses `jsonParsed` encoding for this endpoint.
                .get_token_accounts_by_owner(&owner, TokenAccountsFilter::ProgramId(program_id))
                .await
                .context("Failed to get token accounts by owner")
        })
        .await
    }

    /// Gets account data by address string.
    pub async fn get_account_by_address(&self, address: &str) -> Result<Account> {
        let pubkey = Pubkey::from_str(address).context("Invalid pubkey")?;
        self.get_account(&pubkey).await
    }

    /// Gets multiple accounts.
    pub async fn get_multiple_accounts(
        &self,
        addresses: &[Pubkey],
    ) -> Result<Vec<Option<Account>>> {
        let addrs = addresses.to_vec();
        self.execute_with_retry(|client| {
            let addrs = addrs.clone();
            async move {
                client
                    .get_multiple_accounts(&addrs)
                    .await
                    .context("Failed to get multiple accounts")
            }
        })
        .await
    }

    /// Gets the balance of an account in lamports.
    pub async fn get_balance(&self, address: &Pubkey) -> Result<u64> {
        let addr = *address;
        self.execute_with_retry(|client| async move {
            client
                .get_balance(&addr)
                .await
                .context("Failed to get balance")
        })
        .await
    }

    /// Fetch recent signatures for an address (getSignaturesForAddress).
    pub async fn get_signatures_for_address_with_config(
        &self,
        address: &Pubkey,
        config: GetConfirmedSignaturesForAddress2Config,
    ) -> Result<Vec<RpcConfirmedTransactionStatusWithSignature>> {
        let addr = *address;
        // `GetConfirmedSignaturesForAddress2Config` is not `Clone` in some Solana versions,
        // so we decompose it and rebuild for each retry attempt.
        let before = config.before;
        let until = config.until;
        let limit = config.limit;
        let commitment = config.commitment;

        self.execute_with_retry(|client| {
            let cfg = GetConfirmedSignaturesForAddress2Config {
                before,
                until,
                limit,
                commitment,
            };
            async move {
                client
                    .get_signatures_for_address_with_config(&addr, cfg)
                    .await
                    .context("Failed to get signatures for address")
            }
        })
        .await
    }

    /// Fetch a transaction with config (getTransaction).
    pub async fn get_transaction_with_config(
        &self,
        signature: &Signature,
        config: RpcTransactionConfig,
    ) -> Result<EncodedConfirmedTransactionWithStatusMeta> {
        let sig = *signature;
        self.execute_with_retry(|client| async move {
            client
                .get_transaction_with_config(&sig, config)
                .await
                .context("Failed to get transaction")
        })
        .await
    }

    /// Convenience: getTransaction(jsonParsed) with safe defaults.
    pub async fn get_transaction_json_parsed(
        &self,
        signature: &Signature,
    ) -> Result<EncodedConfirmedTransactionWithStatusMeta> {
        self.get_transaction_with_config(
            signature,
            RpcTransactionConfig {
                encoding: Some(UiTransactionEncoding::JsonParsed),
                commitment: None,
                max_supported_transaction_version: Some(0),
            },
        )
        .await
    }

    /// Gets the latest blockhash.
    pub async fn get_latest_blockhash(&self) -> Result<solana_sdk::hash::Hash> {
        self.execute_with_retry(|client| async move {
            client
                .get_latest_blockhash()
                .await
                .context("Failed to get latest blockhash")
        })
        .await
    }

    /// Gets transaction status.
    pub async fn get_signature_status(
        &self,
        signature: &Signature,
    ) -> Result<Option<SignatureStatusInfo>> {
        let sig = *signature;
        self.execute_with_retry(|client| async move {
            let statuses = client
                .get_signature_statuses(&[sig])
                .await
                .context("Failed to get signature status")?;

            Ok(statuses.value.first().and_then(|s| {
                s.as_ref().map(|status| SignatureStatusInfo {
                    slot: status.slot,
                    err: status.err.clone(),
                })
            }))
        })
        .await
    }

    /// Gets the health status of all endpoints.
    pub async fn get_health_status(
        &self,
    ) -> std::collections::HashMap<String, super::EndpointHealth> {
        self.health.get_all_health().await
    }

    /// Performs a health check on all endpoints.
    pub async fn check_all_endpoints(&self) {
        let endpoints = self.config.all_endpoints();
        for endpoint in endpoints {
            let _ = self.health.check_endpoint(endpoint).await;
        }
    }

    /// Simulates a transaction without broadcasting.
    pub async fn simulate_transaction(
        &self,
        transaction: &solana_sdk::transaction::Transaction,
    ) -> Result<solana_client::rpc_response::RpcSimulateTransactionResult> {
        let tx = transaction.clone();
        self.execute_with_retry(|client| {
            let tx = tx.clone();
            async move {
                let response = client
                    .simulate_transaction(&tx)
                    .await
                    .context("Failed to simulate transaction")?;
                Ok(response.value)
            }
        })
        .await
    }

    /// Sends and confirms a transaction.
    pub async fn send_and_confirm_transaction(
        &self,
        transaction: &solana_sdk::transaction::Transaction,
    ) -> Result<Signature> {
        // Devnet/mainnet RPCs are often flaky. A common failure mode is:
        // - send succeeds on one endpoint
        // - confirm/status polling happens on another due to failover rotation
        // which makes the tx appear "not found" and ends as a send+confirm error.
        //
        // To avoid this, pin a single endpoint for the whole send+confirm lifecycle.
        let tx = transaction.clone();
        let mut last_err: Option<anyhow::Error> = None;

        // IMPORTANT: The transaction is already signed with a recent blockhash obtained via this
        // provider. If we "fan out" to other endpoints here, we can hit `BlockhashNotFound` when
        // the send endpoint does not know the blockhash (common with public RPC fleets).
        //
        // Therefore: pin to `current_endpoint()` for each whole send+confirm attempt. If it fails,
        // rotate endpoint (provider-level) and the caller will typically re-sign with a fresh
        // blockhash on retry.
        let max_endpoints = self.config.all_endpoints().len().max(1);
        for _ in 0..max_endpoints {
            let endpoint = self.current_endpoint().await;
            let client = RpcClient::new_with_timeout(endpoint.clone(), self.config.timeout);

            // 1) Send with retries on this endpoint.
            let mut send_attempt = 0u32;
            let sig = loop {
                match client.send_transaction(&tx).await {
                    Ok(sig) => break Ok(sig),
                    Err(e) => {
                        // `solana_client` errors are often opaque in Display; keep Debug for UI/ops.
                        // Also include endpoint to distinguish rate-limit / auth / cluster issues.
                        let mut err_s =
                            format!("send_transaction failed (endpoint={endpoint}): {e:?}");

                        // When preflight simulation fails, try to fetch logs for actionable diagnostics.
                        // This keeps the error useful for UI users without requiring server-side log access.
                        let e_dbg = format!("{e:?}");
                        if e_dbg.contains("Transaction simulation failed") {
                            if let Ok(sim) = client.simulate_transaction(&tx).await {
                                let v = sim.value;
                                if let Some(sim_err) = v.err {
                                    err_s.push_str(&format!(" | simulation_err={sim_err:?}"));
                                }
                                if let Some(logs) = v.logs {
                                    // Keep the message bounded; UI shows ~400 chars by default.
                                    let mut joined = logs.join(" | ");
                                    if joined.len() > 2500 {
                                        joined.truncate(2500);
                                        joined.push_str("...");
                                    }
                                    err_s.push_str(&format!(" | logs={joined}"));
                                }
                            }
                            // Also include program ids per instruction to map "Instruction N".
                            let progs = format_instruction_program_ids(&tx);
                            if !progs.is_empty() {
                                err_s.push_str(&format!(" | ix_programs={progs}"));
                            }
                        }

                        let err = anyhow::anyhow!("{err_s}");
                        last_err = Some(err);
                        if send_attempt >= self.config.max_retries {
                            break Err(());
                        }
                        let delay = calculate_backoff(
                            send_attempt,
                            self.config.retry_base_delay_ms,
                            self.config.retry_max_delay_ms,
                        );
                        warn!(
                            endpoint = endpoint,
                            attempt = send_attempt,
                            delay_ms = delay,
                            error = ?last_err.as_ref().unwrap(),
                            "send_transaction failed; retrying"
                        );
                        sleep(Duration::from_millis(delay)).await;
                        send_attempt += 1;
                    }
                }
            };

            let sig = match sig {
                Ok(sig) => sig,
                Err(()) => {
                    warn!(endpoint = endpoint, "send failed on endpoint; trying next");
                    self.rotate_endpoint().await;
                    continue;
                }
            };

            // 2) Confirm by polling signature status on the same endpoint.
            let deadline = Instant::now() + Duration::from_secs(90);
            loop {
                if Instant::now() >= deadline {
                    last_err = Some(anyhow::anyhow!(
                        "confirm timeout (endpoint={endpoint}, signature={sig})"
                    ));
                    warn!(endpoint = endpoint, signature = %sig, "confirm timed out; trying next endpoint");
                    self.rotate_endpoint().await;
                    break;
                }

                let statuses: solana_client::rpc_response::Response<
                    Vec<Option<TransactionStatus>>,
                > = client
                    .get_signature_statuses(&[sig])
                    .await
                    .context("get_signature_statuses")?;
                if let Some(Some(status)) = statuses.value.first() {
                    if let Some(err) = status.err.clone() {
                        let mut err_s = format!(
                            "transaction error (endpoint={endpoint}, signature={sig}): {err:?}"
                        );
                        // Add program context for InstructionError(N, Custom(X)) and similar.
                        if let solana_sdk::transaction::TransactionError::InstructionError(
                            ix_index,
                            ix_err,
                        ) = &err
                        {
                            let ix_index = *ix_index as usize;
                            let progs = format_instruction_program_ids(&tx);
                            if !progs.is_empty() {
                                err_s.push_str(&format!(" | ix_programs={progs}"));
                            }
                            if let Some(ci) = tx.message.instructions.get(ix_index) {
                                let pid = tx
                                    .message
                                    .account_keys
                                    .get(ci.program_id_index as usize)
                                    .map(|p| p.to_string());
                                if let Some(pid) = pid {
                                    err_s.push_str(&format!(" | ix_program={ix_index}:{pid}"));
                                }
                            }
                            if let solana_sdk::instruction::InstructionError::Custom(code) = ix_err
                            {
                                err_s.push_str(&format!(" | custom_code={code}"));
                            }
                        }
                        return Err(anyhow::anyhow!("{err_s}"));
                    }

                    let ok = match self.config.commitment {
                        super::config::CommitmentLevel::Processed => true,
                        super::config::CommitmentLevel::Confirmed => matches!(
                            status.confirmation_status,
                            Some(TransactionConfirmationStatus::Confirmed)
                                | Some(TransactionConfirmationStatus::Finalized)
                        ),
                        super::config::CommitmentLevel::Finalized => matches!(
                            status.confirmation_status,
                            Some(TransactionConfirmationStatus::Finalized)
                        ),
                    };
                    if ok {
                        return Ok(sig);
                    }
                }

                sleep(Duration::from_millis(800)).await;
            }
        }

        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("Failed to send and confirm transaction")))
    }

    /// Sends a transaction without waiting for confirmation.
    pub async fn send_transaction(
        &self,
        transaction: &solana_sdk::transaction::Transaction,
    ) -> Result<Signature> {
        let tx = transaction.clone();
        self.execute_with_retry(|client| {
            let tx = tx.clone();
            async move {
                client
                    .send_transaction(&tx)
                    .await
                    .context("Failed to send transaction")
            }
        })
        .await
    }
}

fn format_instruction_program_ids(tx: &Transaction) -> String {
    // Legacy transactions in this codebase use `tx.message.account_keys`.
    // If indices are out of bounds, skip rather than failing error formatting.
    let mut parts: Vec<String> = Vec::new();
    for (i, ix) in tx.message.instructions.iter().enumerate() {
        let idx = usize::from(ix.program_id_index);
        if let Some(pk) = tx.message.account_keys.get(idx) {
            parts.push(format!("{i}:{pk}"));
        }
    }
    parts.join(",")
}

fn hard_disable_reason(err: &anyhow::Error) -> Option<String> {
    // The Solana RPC client wraps HTTP failures into opaque errors; we match on their string forms.
    // Example (observed): "HTTP status client error (402 Payment Required) for url (...)"
    let mut chain_msgs: Vec<String> = Vec::new();
    for c in err.chain() {
        chain_msgs.push(c.to_string());
    }
    let msg_joined = chain_msgs.join(" | ");
    let msg_lc = msg_joined.to_ascii_lowercase();

    // Hard failures: endpoint requires payment or auth, or blocks the request. Retrying/rotating back
    // to it just creates repeated gaps in snapshot collection.
    if msg_lc.contains("(402")
        || msg_lc.contains("402 payment required")
        || msg_lc.contains("(401")
        || msg_lc.contains("401 unauthorized")
        || msg_lc.contains("(403")
        || msg_lc.contains("403 forbidden")
    {
        return Some(msg_joined);
    }
    None
}

/// Calculates exponential backoff delay.
fn calculate_backoff(retry: u32, base_ms: u64, max_ms: u64) -> u64 {
    let delay = base_ms * 2u64.pow(retry);
    delay.min(max_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_backoff() {
        assert_eq!(calculate_backoff(0, 100, 5000), 100);
        assert_eq!(calculate_backoff(1, 100, 5000), 200);
        assert_eq!(calculate_backoff(2, 100, 5000), 400);
        assert_eq!(calculate_backoff(3, 100, 5000), 800);
        assert_eq!(calculate_backoff(10, 100, 5000), 5000); // Capped at max
    }

    #[tokio::test]
    async fn test_provider_creation() {
        let provider = RpcProvider::new(RpcConfig::new("https://api.mainnet-beta.solana.com"));
        let endpoint = provider.current_endpoint().await;
        assert!(endpoint.contains("mainnet"));
    }

    #[tokio::test]
    async fn test_devnet_provider() {
        let provider = RpcProvider::devnet();
        let endpoint = provider.current_endpoint().await;
        assert!(endpoint.contains("devnet"));
    }

    #[test]
    fn account_not_found_detected_in_chain() {
        let e = anyhow::anyhow!("outer").context("could not find account");
        assert!(is_definitive_account_not_found(&e));
    }
}
