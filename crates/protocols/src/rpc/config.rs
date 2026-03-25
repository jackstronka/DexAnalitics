//! RPC configuration for Solana endpoints.

use std::time::Duration;

/// Configuration for RPC endpoints.
#[derive(Debug, Clone)]
pub struct RpcConfig {
    /// Primary RPC endpoint URL.
    pub primary_url: String,
    /// Fallback RPC endpoint URLs.
    pub fallback_urls: Vec<String>,
    /// Request timeout duration.
    pub timeout: Duration,
    /// Maximum retries per request.
    pub max_retries: u32,
    /// Base delay for exponential backoff in milliseconds.
    pub retry_base_delay_ms: u64,
    /// Maximum delay for exponential backoff in milliseconds.
    pub retry_max_delay_ms: u64,
    /// Health check interval in seconds.
    pub health_check_interval_secs: u64,
    /// Commitment level for requests.
    pub commitment: CommitmentLevel,
}

impl Default for RpcConfig {
    fn default() -> Self {
        // Allow overriding RPC endpoints via environment variables.
        // This is critical for `getTransaction`-heavy workflows (swap decoding) where public
        // endpoints may rate-limit, time out, or disable transaction history.
        let env_primary = std::env::var("SOLANA_RPC_URL").ok().filter(|s| !s.trim().is_empty());
        let env_fallbacks = std::env::var("SOLANA_RPC_FALLBACK_URLS")
            .ok()
            .unwrap_or_default();
        let mut fallback_urls: Vec<String> = env_fallbacks
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        // Default public fallbacks (still useful even when SOLANA_RPC_URL is set).
        fallback_urls.extend(vec![
            "https://solana-api.projectserum.com".to_string(),
            "https://rpc.ankr.com/solana".to_string(),
        ]);

        Self {
            primary_url: env_primary.unwrap_or_else(|| "https://api.mainnet-beta.solana.com".to_string()),
            fallback_urls,
            timeout: Duration::from_secs(30),
            max_retries: 3,
            retry_base_delay_ms: 100,
            retry_max_delay_ms: 5000,
            health_check_interval_secs: 60,
            commitment: CommitmentLevel::Confirmed,
        }
    }
}

impl RpcConfig {
    /// Creates a new RPC config with the given primary URL.
    #[must_use]
    pub fn new(primary_url: impl Into<String>) -> Self {
        Self {
            primary_url: primary_url.into(),
            ..Default::default()
        }
    }

    /// Adds a fallback URL.
    #[must_use]
    pub fn with_fallback(mut self, url: impl Into<String>) -> Self {
        self.fallback_urls.push(url.into());
        self
    }

    /// Sets the request timeout.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Sets the maximum retries.
    #[must_use]
    pub fn with_max_retries(mut self, retries: u32) -> Self {
        self.max_retries = retries;
        self
    }

    /// Sets the commitment level.
    #[must_use]
    pub fn with_commitment(mut self, commitment: CommitmentLevel) -> Self {
        self.commitment = commitment;
        self
    }

    /// Returns all endpoint URLs in priority order.
    #[must_use]
    pub fn all_endpoints(&self) -> Vec<&str> {
        let mut endpoints = vec![self.primary_url.as_str()];
        endpoints.extend(self.fallback_urls.iter().map(String::as_str));
        endpoints
    }

    /// Creates a devnet configuration.
    #[must_use]
    pub fn devnet() -> Self {
        Self {
            primary_url: "https://api.devnet.solana.com".to_string(),
            fallback_urls: vec![],
            ..Default::default()
        }
    }

    /// Creates a testnet configuration.
    #[must_use]
    pub fn testnet() -> Self {
        Self {
            primary_url: "https://api.testnet.solana.com".to_string(),
            fallback_urls: vec![],
            ..Default::default()
        }
    }

    /// Creates a localhost configuration.
    #[must_use]
    pub fn localhost() -> Self {
        Self {
            primary_url: "http://127.0.0.1:8899".to_string(),
            fallback_urls: vec![],
            ..Default::default()
        }
    }
}

/// Commitment level for RPC requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CommitmentLevel {
    /// Processed commitment (fastest, least reliable).
    Processed,
    /// Confirmed commitment (balanced).
    #[default]
    Confirmed,
    /// Finalized commitment (slowest, most reliable).
    Finalized,
}

impl CommitmentLevel {
    /// Returns the commitment level as a string for RPC calls.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Processed => "processed",
            Self::Confirmed => "confirmed",
            Self::Finalized => "finalized",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = RpcConfig::default();
        assert!(config.primary_url.contains("mainnet"));
        assert!(!config.fallback_urls.is_empty());
    }

    #[test]
    fn test_all_endpoints() {
        let config = RpcConfig::new("https://primary.com")
            .with_fallback("https://fallback1.com")
            .with_fallback("https://fallback2.com");

        let endpoints = config.all_endpoints();
        assert_eq!(endpoints.len(), 5); // primary + 2 default + 2 added
        assert_eq!(endpoints[0], "https://primary.com");
    }

    #[test]
    fn test_devnet_config() {
        let config = RpcConfig::devnet();
        assert!(config.primary_url.contains("devnet"));
        assert!(config.fallback_urls.is_empty());
    }
}
