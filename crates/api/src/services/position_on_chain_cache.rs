//! Short-lived negative cache: position PDA with no on-chain account (404), to avoid
//! re-querying stale `registry_open` / strategy links on every `GET /positions`.

use solana_sdk::pubkey::Pubkey;
use std::collections::HashMap;
use std::env;
use std::time::{Duration, Instant};
use crate::error::ApiError;
use crate::state::AppState;

pub fn position_absent_cache_ttl() -> Duration {
    let secs = env::var("CLMM_POSITION_ABSENT_CACHE_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(600);
    Duration::from_secs(secs)
}

pub fn api_error_is_account_absent(err: &ApiError) -> bool {
    matches!(err, ApiError::NotFound(_))
}

pub async fn is_position_absent_cached(state: &AppState, pk: &Pubkey) -> bool {
    let key = pk.to_string();
    let guard = state.position_absent_cache.read().await;
    guard
        .get(&key)
        .is_some_and(|until| *until > Instant::now())
}

pub async fn record_position_absent(state: &AppState, pk: &Pubkey) {
    let key = pk.to_string();
    let until = Instant::now() + position_absent_cache_ttl();
    let mut guard = state.position_absent_cache.write().await;
    guard.insert(key, until);
}

pub async fn prune_expired_position_absent_cache(state: &AppState) {
    let now = Instant::now();
    let mut guard = state.position_absent_cache.write().await;
    guard.retain(|_, until| *until > now);
}

/// Running strategies' explicit `parameters.position_addresses` (valid base58 PDAs).
pub async fn running_strategy_position_pubkeys(state: &AppState) -> Vec<Pubkey> {
    use std::collections::HashSet;
    use std::str::FromStr;

    let strategies = state.strategies.read().await;
    let mut out = HashSet::new();
    for s in strategies.values() {
        if !s.running {
            continue;
        }
        let Some(arr) = s
            .config
            .get("parameters")
            .and_then(|p| p.get("position_addresses"))
            .and_then(|v| v.as_array())
        else {
            continue;
        };
        for v in arr {
            let Some(addr) = v.as_str() else {
                continue;
            };
            if let Ok(pk) = Pubkey::from_str(addr.trim()) {
                out.insert(pk);
            }
        }
    }
    out.into_iter().collect()
}

#[derive(Debug, Default, Clone)]
pub struct SupplementFetchStats {
    pub skipped_absent_cached: u32,
    pub skipped_registry_closed: u32,
    pub skipped_already_listed: u32,
    pub chain_error: u32,
    pub merged_ok: u32,
}

pub async fn fetch_supplement_positions_parallel(
    state: &AppState,
    already: &std::collections::HashSet<Pubkey>,
    candidates: Vec<Pubkey>,
    from_registry: &std::collections::HashSet<Pubkey>,
    from_strategies: &std::collections::HashSet<Pubkey>,
    reg_state: &HashMap<Pubkey, bool>,
    concurrency: usize,
) -> (Vec<clmm_lp_execution::monitor::MonitoredPosition>, SupplementFetchStats) {
    use crate::services::position_valuation::monitored_position_from_chain;
    use futures::{stream, StreamExt};

    prune_expired_position_absent_cache(state).await;

    let mut stats = SupplementFetchStats::default();
    let mut to_fetch = Vec::new();

    for pk in candidates {
        if already.contains(&pk) {
            stats.skipped_already_listed += 1;
            continue;
        }
        if reg_state.get(&pk) == Some(&false) {
            stats.skipped_registry_closed += 1;
            continue;
        }
        if is_position_absent_cached(state, &pk).await {
            stats.skipped_absent_cached += 1;
            continue;
        }
        to_fetch.push(pk);
    }

    if to_fetch.is_empty() {
        return (Vec::new(), stats);
    }

    let provider = state.provider.clone();
    let absent_cache = state.position_absent_cache.clone();
    let st = state.clone();
    let from_registry_set = from_registry.clone();
    let limit = concurrency.max(1);

    let outcomes = stream::iter(to_fetch)
        .map(|pk| {
            let provider = provider.clone();
            let absent_cache = absent_cache.clone();
            let st = st.clone();
            let from_registry_set = from_registry_set.clone();
            async move {
                match monitored_position_from_chain(provider, &pk).await {
                    Ok(p) => Ok(p),
                    Err(e) => {
                        if api_error_is_account_absent(&e) {
                            let key = pk.to_string();
                            let until = Instant::now() + position_absent_cache_ttl();
                            absent_cache.write().await.insert(key, until);
                            let from_reg = from_registry_set.contains(&pk);
                            tokio::spawn(async move {
                                crate::services::registry_stale_reconcile::on_position_account_absent(
                                    &st, &pk, from_reg,
                                )
                                .await;
                            });
                        }
                        Err((pk, e))
                    }
                }
            }
        })
        .buffer_unordered(limit)
        .collect::<Vec<_>>()
        .await;

    let mut merged = Vec::new();
    for outcome in outcomes {
        match outcome {
            Ok(p) => {
                stats.merged_ok += 1;
                merged.push(p);
            }
            Err((pk, e)) => {
                stats.chain_error += 1;
                let src = if from_strategies.contains(&pk) {
                    "strategy"
                } else if from_registry.contains(&pk) {
                    "registry"
                } else {
                    "unknown"
                };
                tracing::warn!(
                    position = %pk,
                    source = src,
                    error = %e,
                    "list_positions: supplement fetch failed; skipping"
                );
            }
        }
    }

    (merged, stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{ApiConfig, AppState, StrategyState};
    use clmm_lp_protocols::prelude::RpcConfig;
    use std::str::FromStr;

    fn test_state() -> AppState {
        AppState::new(RpcConfig::default(), ApiConfig::default(), None)
    }

    #[tokio::test]
    async fn running_strategy_position_pubkeys_only_when_running() {
        let state = test_state();
        let pk = Pubkey::from_str("HTtpWVsnoctjiZqrYjkhan2RcYEnpxW3ueqns3PFJJQK").unwrap();
        let now = chrono::Utc::now();
        state.strategies.write().await.insert(
            "s1".to_string(),
            StrategyState {
                id: "s1".to_string(),
                name: "t".to_string(),
                running: true,
                config: serde_json::json!({
                    "parameters": { "position_addresses": [pk.to_string()] }
                }),
                created_at: now,
                updated_at: now,
            },
        );
        state.strategies.write().await.insert(
            "s2".to_string(),
            StrategyState {
                id: "s2".to_string(),
                name: "off".to_string(),
                running: false,
                config: serde_json::json!({
                    "parameters": { "position_addresses": ["11111111111111111111111111111111"] }
                }),
                created_at: now,
                updated_at: now,
            },
        );

        let got = running_strategy_position_pubkeys(&state).await;
        assert_eq!(got.len(), 1);
        assert_eq!(got[0], pk);
    }

    #[test]
    fn api_error_is_account_absent_matches_not_found() {
        assert!(api_error_is_account_absent(&ApiError::not_found("gone")));
        assert!(!api_error_is_account_absent(&ApiError::bad_gateway("rpc")));
    }
}
