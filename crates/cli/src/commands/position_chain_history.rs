//! `chain-history-refresh` — call the running API to materialize Postgres `position_chain_history_*`.

use anyhow::{Context, Result, bail};
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;

use crate::orchestrator_api_full::normalize_api_base;

#[derive(Debug, Deserialize)]
struct MaterializeChainHistoryResponse {
    ok: bool,
    chain_anchor_pubkey: String,
    metrics_mode: String,
    nodes_written: u32,
}

/// `POST /api/v1/positions/{address}/chain-history/refresh?mode=…`
pub async fn run_chain_history_refresh(
    api_base_url: &str,
    address: &str,
    mode: &str,
    refresh_secret: Option<&str>,
    x_api_key: Option<&str>,
    timeout_secs: u64,
) -> Result<()> {
    let base = normalize_api_base(api_base_url);
    let addr = address.trim();
    if addr.is_empty() {
        bail!("address must not be empty");
    }
    let url = format!("{base}/api/v1/positions/{addr}/chain-history/refresh?mode={mode}");

    let client = Client::builder()
        .user_agent("clmm-lp-cli/chain-history-refresh")
        .timeout(Duration::from_secs(timeout_secs.max(1)))
        .build()
        .context("build HTTP client")?;

    let mut req = client.post(&url);
    if let Some(key) = x_api_key.map(str::trim).filter(|s| !s.is_empty()) {
        req = req.header("X-API-Key", key);
    }
    if let Some(sec) = refresh_secret.map(str::trim).filter(|s| !s.is_empty()) {
        req = req.header("Authorization", format!("Bearer {sec}"));
    }

    let resp = req.send().await.with_context(|| format!("POST {url}"))?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("POST chain-history/refresh failed HTTP {status}: {body}");
    }
    let parsed: MaterializeChainHistoryResponse =
        serde_json::from_str(&body).with_context(|| {
            format!(
                "decode JSON response (first 400 chars): {}",
                body.chars().take(400).collect::<String>()
            )
        })?;
    tracing::info!(
        ok = parsed.ok,
        anchor = %parsed.chain_anchor_pubkey,
        mode = %parsed.metrics_mode,
        nodes = parsed.nodes_written,
        "chain-history materialized via API"
    );
    println!(
        "ok={} anchor={} metrics_mode={} nodes_written={}",
        parsed.ok, parsed.chain_anchor_pubkey, parsed.metrics_mode, parsed.nodes_written
    );
    Ok(())
}
