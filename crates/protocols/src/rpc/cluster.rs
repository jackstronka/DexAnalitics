//! Optional cluster consistency checks (RPC URL vs operator intent).
//!
//! Set **`CLMM_EXPECTED_CLUSTER`** to `mainnet-beta`, `devnet`, `testnet`, or `localnet` to fail fast
//! when an RPC URL clearly points at a different cluster (e.g. devnet URL with mainnet intent).

use super::RpcConfig;
use anyhow::{Context, Result, bail};
use std::str::FromStr;

/// Target Solana cluster for an operator session (from env / config).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClusterKind {
    MainnetBeta,
    Devnet,
    Testnet,
    Localnet,
}

impl ClusterKind {
    #[must_use]
    pub fn as_cli_label(&self) -> &'static str {
        match self {
            Self::MainnetBeta => "mainnet-beta",
            Self::Devnet => "devnet",
            Self::Testnet => "testnet",
            Self::Localnet => "localnet",
        }
    }
}

impl FromStr for ClusterKind {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "mainnet" | "mainnet-beta" | "mainnet_beta" => Ok(Self::MainnetBeta),
            "devnet" => Ok(Self::Devnet),
            "testnet" => Ok(Self::Testnet),
            "local" | "localhost" | "localnet" => Ok(Self::Localnet),
            _ => Err(format!(
                "unknown cluster {s:?} (expected mainnet-beta|devnet|testnet|localnet)"
            )),
        }
    }
}

/// Best-effort cluster hint from an RPC URL. Custom hostnames without keywords return `None`.
#[must_use]
pub fn infer_cluster_hint_from_rpc_url(url: &str) -> Option<ClusterKind> {
    let u = url.to_ascii_lowercase();
    if u.contains("devnet") {
        return Some(ClusterKind::Devnet);
    }
    if u.contains("testnet") {
        return Some(ClusterKind::Testnet);
    }
    if u.contains("mainnet") {
        return Some(ClusterKind::MainnetBeta);
    }
    if u.contains("127.0.0.1") || u.contains("localhost") {
        return Some(ClusterKind::Localnet);
    }
    None
}

/// If **`CLMM_EXPECTED_CLUSTER`** is set, ensure every inferable RPC endpoint matches it.
///
/// URLs that do not match any heuristic are **skipped** (allows custom mainnet RPC domains).
pub fn enforce_expected_cluster_for_rpc_config(config: &RpcConfig) -> Result<()> {
    let endpoints: Vec<String> = config
        .all_endpoints()
        .into_iter()
        .map(ToString::to_string)
        .collect();
    enforce_expected_cluster_for_rpc_urls(&endpoints)
}

/// Same as [`enforce_expected_cluster_for_rpc_config`] but takes raw URL strings.
pub fn enforce_expected_cluster_for_rpc_urls(urls: &[String]) -> Result<()> {
    let Ok(raw) = std::env::var("CLMM_EXPECTED_CLUSTER") else {
        return Ok(());
    };
    if raw.trim().is_empty() {
        return Ok(());
    }
    let expected = ClusterKind::from_str(raw.trim())
        .map_err(|e| anyhow::anyhow!(e))
        .with_context(|| format!("parsing CLMM_EXPECTED_CLUSTER={raw:?}"))?;

    for url in urls {
        let Some(hint) = infer_cluster_hint_from_rpc_url(url) else {
            continue;
        };
        if hint != expected {
            bail!(
                "RPC URL cluster hint {:?} does not match CLMM_EXPECTED_CLUSTER={} (url={url}). \
                 Fix SOLANA_RPC_URL / fallbacks or unset CLMM_EXPECTED_CLUSTER.",
                hint.as_cli_label(),
                expected.as_cli_label()
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infers_devnet_from_url() {
        assert_eq!(
            infer_cluster_hint_from_rpc_url("https://api.devnet.solana.com"),
            Some(ClusterKind::Devnet)
        );
    }

    #[test]
    fn infers_mainnet_from_url() {
        assert_eq!(
            infer_cluster_hint_from_rpc_url("https://api.mainnet-beta.solana.com"),
            Some(ClusterKind::MainnetBeta)
        );
    }

    #[test]
    fn custom_provider_host_has_no_hint() {
        assert_eq!(
            infer_cluster_hint_from_rpc_url("https://example-rpc.internal/v1/proxy"),
            None
        );
    }
}
