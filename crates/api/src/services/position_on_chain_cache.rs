//! Short-lived negative cache: position PDA with no on-chain account (404), to avoid
//! re-querying stale `registry_open` / strategy links on every `GET /positions`.

use solana_sdk::pubkey::Pubkey;
use std::collections::HashMap;
use std::env;
use std::hash::{Hash, Hasher};
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

#[derive(Debug, Clone)]
pub struct CachedSupplementBatch {
    pub until: Instant,
    pub fingerprint: u64,
    pub by_pubkey: HashMap<Pubkey, clmm_lp_execution::monitor::MonitoredPosition>,
}

/// TTL for shared supplement RPC cache (list + close-all).
pub fn supplement_batch_cache_ttl() -> Duration {
    let secs = env::var("CLMM_SUPPLEMENT_BATCH_CACHE_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(25);
    Duration::from_secs(secs)
}

/// Fingerprint of supplement candidate PDAs (order-independent).
#[must_use]
pub fn supplement_candidates_fingerprint(candidates: &[Pubkey]) -> u64 {
    let mut sorted: Vec<[u8; 32]> = candidates.iter().map(|p| p.to_bytes()).collect();
    sorted.sort_unstable();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for bytes in sorted {
        bytes.hash(&mut h);
    }
    h.finish()
}

async fn read_supplement_batch_cache(
    state: &AppState,
    fingerprint: u64,
) -> Option<HashMap<Pubkey, clmm_lp_execution::monitor::MonitoredPosition>> {
    let guard = state.supplement_batch_cache.read().await;
    let cached = guard.as_ref()?;
    if cached.fingerprint != fingerprint || Instant::now() >= cached.until {
        return None;
    }
    Some(cached.by_pubkey.clone())
}

async fn write_supplement_batch_cache(
    state: &AppState,
    fingerprint: u64,
    mut by_pubkey: HashMap<Pubkey, clmm_lp_execution::monitor::MonitoredPosition>,
) {
    let until = Instant::now() + supplement_batch_cache_ttl();
    let mut guard = state.supplement_batch_cache.write().await;
    if let Some(existing) = guard.as_mut()
        && existing.fingerprint == fingerprint
        && Instant::now() < existing.until
    {
        existing.by_pubkey.extend(by_pubkey.drain());
        existing.until = until;
        return;
    }
    *guard = Some(CachedSupplementBatch {
        until,
        fingerprint,
        by_pubkey,
    });
}

#[derive(Debug, Default, Clone)]
pub struct SupplementFetchStats {
    pub skipped_absent_cached: u32,
    pub skipped_registry_closed: u32,
    pub skipped_already_listed: u32,
    pub skipped_supplement_cache: u32,
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

    let fingerprint = supplement_candidates_fingerprint(&candidates);
    let batch_cache = read_supplement_batch_cache(state, fingerprint).await;

    let mut stats = SupplementFetchStats::default();
    let mut merged = Vec::new();
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
        if let Some(cache) = &batch_cache
            && let Some(p) = cache.get(&pk)
        {
            stats.skipped_supplement_cache += 1;
            stats.merged_ok += 1;
            merged.push(p.clone());
            continue;
        }
        to_fetch.push(pk);
    }

    if to_fetch.is_empty() {
        return (merged, stats);
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

    let mut fresh_by_pubkey: HashMap<Pubkey, clmm_lp_execution::monitor::MonitoredPosition> =
        HashMap::new();
    for outcome in outcomes {
        match outcome {
            Ok(p) => {
                stats.merged_ok += 1;
                fresh_by_pubkey.insert(p.address, p.clone());
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

    if !fresh_by_pubkey.is_empty() {
        if let Some(cache) = batch_cache {
            let mut combined = cache;
            combined.extend(fresh_by_pubkey);
            write_supplement_batch_cache(state, fingerprint, combined).await;
        } else {
            write_supplement_batch_cache(state, fingerprint, fresh_by_pubkey).await;
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

    #[test]
    fn supplement_fingerprint_is_order_independent() {
        let a = Pubkey::new_unique();
        let b = Pubkey::new_unique();
        let f1 = supplement_candidates_fingerprint(&[a, b]);
        let f2 = supplement_candidates_fingerprint(&[b, a]);
        assert_eq!(f1, f2);
        assert_ne!(f1, supplement_candidates_fingerprint(&[a]));
    }

    #[tokio::test]
    async fn supplement_batch_cache_reuses_fingerprint_within_ttl() {
        let state = test_state();
        let pk = Pubkey::new_unique();
        let candidates = vec![pk];
        let fp = supplement_candidates_fingerprint(&candidates);
        let mut by = HashMap::new();
        let pool = Pubkey::new_unique();
        by.insert(
            pk,
            clmm_lp_execution::monitor::MonitoredPosition {
                address: pk,
                pool,
                on_chain: clmm_lp_protocols::prelude::OnChainPosition {
                    address: pk,
                    pool,
                    owner: Pubkey::new_unique(),
                    tick_lower: 0,
                    tick_upper: 0,
                    liquidity: 0,
                    fee_growth_inside_a: 0,
                    fee_growth_inside_b: 0,
                    fees_owed_a: 0,
                    fees_owed_b: 0,
                },
                pnl: clmm_lp_execution::monitor::PositionPnL::default(),
                in_range: true,
                last_updated: chrono::Utc::now(),
            },
        );
        write_supplement_batch_cache(&state, fp, by).await;
        let got = read_supplement_batch_cache(&state, fp).await;
        assert!(got.as_ref().is_some_and(|m| m.contains_key(&pk)));
        let wrong = read_supplement_batch_cache(&state, fp.wrapping_add(1)).await;
        assert!(wrong.is_none());
    }
}
