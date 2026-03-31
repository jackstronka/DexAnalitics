//! Health checking for RPC endpoints.

use solana_client::nonblocking::rpc_client::RpcClient;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, warn};

/// Health status of an RPC endpoint.
#[derive(Debug, Clone)]
pub struct EndpointHealth {
    /// Whether the endpoint is currently healthy.
    pub is_healthy: bool,
    /// Last successful request timestamp.
    pub last_success: Option<Instant>,
    /// Last failure timestamp.
    pub last_failure: Option<Instant>,
    /// Number of consecutive failures.
    pub consecutive_failures: u32,
    /// Average response time in milliseconds.
    pub avg_response_time_ms: f64,
    /// Total requests made.
    pub total_requests: u64,
    /// Total successful requests.
    pub successful_requests: u64,
}

impl Default for EndpointHealth {
    fn default() -> Self {
        Self {
            is_healthy: true,
            last_success: None,
            last_failure: None,
            consecutive_failures: 0,
            avg_response_time_ms: 0.0,
            total_requests: 0,
            successful_requests: 0,
        }
    }
}

impl EndpointHealth {
    /// Records a successful request.
    pub fn record_success(&mut self, response_time_ms: f64) {
        self.is_healthy = true;
        self.last_success = Some(Instant::now());
        self.consecutive_failures = 0;
        self.total_requests += 1;
        self.successful_requests += 1;

        // Update rolling average
        let n = self.successful_requests as f64;
        self.avg_response_time_ms =
            self.avg_response_time_ms * (n - 1.0) / n + response_time_ms / n;
    }

    /// Records a failed request.
    pub fn record_failure(&mut self) {
        self.last_failure = Some(Instant::now());
        self.consecutive_failures += 1;
        self.total_requests += 1;

        // Mark unhealthy after 3 consecutive failures
        if self.consecutive_failures >= 3 {
            self.is_healthy = false;
        }
    }

    /// Returns the success rate as a percentage.
    #[must_use]
    pub fn success_rate(&self) -> f64 {
        if self.total_requests == 0 {
            return 100.0;
        }
        (self.successful_requests as f64 / self.total_requests as f64) * 100.0
    }
}

/// Health checker for multiple RPC endpoints.
pub struct HealthChecker {
    /// Health status for each endpoint.
    health_map: Arc<RwLock<HashMap<String, EndpointHealth>>>,
    /// Endpoints that are hard-disabled for this process (e.g. "402 Payment Required").
    disabled_endpoints: Arc<RwLock<HashMap<String, String>>>,
    /// Maximum consecutive failures before marking unhealthy.
    max_consecutive_failures: u32,
    /// Time to wait before retrying an unhealthy endpoint.
    recovery_timeout: Duration,
}

impl HealthChecker {
    /// Creates a new health checker.
    #[must_use]
    pub fn new() -> Self {
        Self {
            health_map: Arc::new(RwLock::new(HashMap::new())),
            disabled_endpoints: Arc::new(RwLock::new(HashMap::new())),
            max_consecutive_failures: 3,
            recovery_timeout: Duration::from_secs(60),
        }
    }

    /// Sets the maximum consecutive failures threshold.
    #[must_use]
    pub fn with_max_failures(mut self, max: u32) -> Self {
        self.max_consecutive_failures = max;
        self
    }

    /// Sets the recovery timeout.
    #[must_use]
    pub fn with_recovery_timeout(mut self, timeout: Duration) -> Self {
        self.recovery_timeout = timeout;
        self
    }

    /// Records a successful request for an endpoint.
    pub async fn record_success(&self, endpoint: &str, response_time_ms: f64) {
        if self.is_disabled(endpoint).await {
            // Keep hard-disabled endpoints disabled even if a stray success is recorded.
            return;
        }
        let mut map = self.health_map.write().await;
        let health = map.entry(endpoint.to_string()).or_default();
        health.record_success(response_time_ms);
        debug!(
            endpoint = endpoint,
            response_time_ms = response_time_ms,
            "RPC request succeeded"
        );
    }

    /// Records a failed request for an endpoint.
    pub async fn record_failure(&self, endpoint: &str) {
        if self.is_disabled(endpoint).await {
            // Don't spam failure counters for a hard-disabled endpoint.
            return;
        }
        let mut map = self.health_map.write().await;
        let health = map.entry(endpoint.to_string()).or_default();
        health.record_failure();
        warn!(
            endpoint = endpoint,
            consecutive_failures = health.consecutive_failures,
            "RPC request failed"
        );
    }

    /// Checks if an endpoint is healthy.
    pub async fn is_healthy(&self, endpoint: &str) -> bool {
        if self.is_disabled(endpoint).await {
            return false;
        }
        let map = self.health_map.read().await;
        match map.get(endpoint) {
            Some(health) => {
                if health.is_healthy {
                    return true;
                }
                // Check if recovery timeout has passed
                if let Some(last_failure) = health.last_failure
                    && last_failure.elapsed() > self.recovery_timeout
                {
                    return true; // Allow retry
                }
                false
            }
            None => true, // Unknown endpoint is assumed healthy
        }
    }

    /// Hard-disable an endpoint for the lifetime of this process.
    pub async fn disable_endpoint(&self, endpoint: &str, reason: impl Into<String>) {
        let reason = reason.into();
        let mut disabled = self.disabled_endpoints.write().await;
        disabled.insert(endpoint.to_string(), reason);
    }

    /// Returns the hard-disable reason if present.
    pub async fn disabled_reason(&self, endpoint: &str) -> Option<String> {
        let disabled = self.disabled_endpoints.read().await;
        disabled.get(endpoint).cloned()
    }

    async fn is_disabled(&self, endpoint: &str) -> bool {
        let disabled = self.disabled_endpoints.read().await;
        disabled.contains_key(endpoint)
    }

    /// Gets the health status for an endpoint.
    pub async fn get_health(&self, endpoint: &str) -> EndpointHealth {
        let map = self.health_map.read().await;
        map.get(endpoint).cloned().unwrap_or_default()
    }

    /// Gets all endpoint health statuses.
    pub async fn get_all_health(&self) -> HashMap<String, EndpointHealth> {
        let map = self.health_map.read().await;
        map.clone()
    }

    /// Performs a health check on an endpoint.
    pub async fn check_endpoint(&self, endpoint: &str) -> bool {
        let client = RpcClient::new(endpoint.to_string());
        let start = Instant::now();

        match client.get_slot().await {
            Ok(_slot) => {
                let elapsed = start.elapsed().as_millis() as f64;
                self.record_success(endpoint, elapsed).await;
                true
            }
            Err(e) => {
                warn!(endpoint = endpoint, error = %e, "Health check failed");
                self.record_failure(endpoint).await;
                false
            }
        }
    }

    /// Returns the best healthy endpoint from a list.
    pub async fn get_best_endpoint<'a>(&self, endpoints: &'a [&'a str]) -> Option<&'a str> {
        let map = self.health_map.read().await;
        let disabled = self.disabled_endpoints.read().await;

        let mut best: Option<(&str, f64)> = None;

        for &endpoint in endpoints {
            if disabled.contains_key(endpoint) {
                continue;
            }
            let health = map.get(endpoint);

            // Check if healthy
            let is_healthy = match health {
                Some(h) => h.is_healthy,
                None => true,
            };

            if !is_healthy {
                continue;
            }

            // Get response time (lower is better)
            let response_time = health.map(|h| h.avg_response_time_ms).unwrap_or(0.0);

            match best {
                None => best = Some((endpoint, response_time)),
                Some((_, best_time)) if response_time < best_time => {
                    best = Some((endpoint, response_time));
                }
                _ => {}
            }
        }

        best.map(|(endpoint, _)| endpoint)
    }
}

impl Default for HealthChecker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_endpoint_health_success() {
        let mut health = EndpointHealth::default();
        health.record_success(100.0);
        health.record_success(200.0);

        assert!(health.is_healthy);
        assert_eq!(health.total_requests, 2);
        assert_eq!(health.successful_requests, 2);
        assert_eq!(health.consecutive_failures, 0);
        assert!((health.avg_response_time_ms - 150.0).abs() < 0.01);
    }

    #[test]
    fn test_endpoint_health_failure() {
        let mut health = EndpointHealth::default();
        health.record_failure();
        health.record_failure();
        assert!(health.is_healthy); // Still healthy after 2 failures

        health.record_failure();
        assert!(!health.is_healthy); // Unhealthy after 3 failures
    }

    #[test]
    fn test_success_rate() {
        let mut health = EndpointHealth::default();
        health.record_success(100.0);
        health.record_success(100.0);
        health.record_failure();
        health.record_failure();

        assert!((health.success_rate() - 50.0).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_health_checker() {
        let checker = HealthChecker::new();

        checker.record_success("https://test.com", 100.0).await;
        assert!(checker.is_healthy("https://test.com").await);

        checker.record_failure("https://test.com").await;
        checker.record_failure("https://test.com").await;
        checker.record_failure("https://test.com").await;
        assert!(!checker.is_healthy("https://test.com").await);
    }
}
