//! Resolve the close signer (owner) for a position PDA using registry → lifecycle → strategy → RPC.

use crate::services::position_wallet_store::wallet_id_for_pubkey;
use crate::services::registry_stale_reconcile::registry_last_open_snapshot;
use crate::state::AppState;
use borsh::BorshDeserialize;
use clmm_lp_protocols::ledger::tx_lifecycle::ledger_read_path;
use clmm_lp_protocols::orca::position_reader::WhirlpoolPosition;
use clmm_lp_protocols::prelude::RpcProvider;
use solana_sdk::program_pack::Pack;
use solana_sdk::pubkey::Pubkey;
use spl_token::state::Account as SplTokenAccount;
use std::str::FromStr;
use std::sync::Arc;

const OPEN_LIFECYCLE_EVENTS: &[&str] = &[
    "bot_open_position",
    "bot_open_position_full_range",
    "position_open",
];

/// Why a position could not be assigned an API-managed close signer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseSignerSkipReason {
    UnmanagedSigner,
    InvalidAddress,
}

/// Resolved close signer for one position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloseSignerResolution {
    pub address: String,
    pub owner_pubkey: String,
    pub close_signer_wallet_id: String,
    pub close_signer_pubkey: String,
}

/// Preview row when signer cannot be resolved to an API wallet file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloseSignerSkipped {
    pub address: String,
    pub reason: CloseSignerSkipReason,
    pub owner_pubkey: Option<String>,
}

#[must_use]
pub fn is_plausible_owner_pubkey(pk: &Pubkey) -> bool {
    *pk != Pubkey::default()
}

fn owner_from_registry(position: &Pubkey) -> Option<Pubkey> {
    registry_last_open_snapshot(position)
        .map(|s| s.owner)
        .filter(is_plausible_owner_pubkey)
}

fn owner_from_lifecycle_ledger(position: &Pubkey) -> Option<Pubkey> {
    let path = ledger_read_path();
    let txt = std::fs::read_to_string(&path).ok()?;
    let pos_s = position.to_string();
    let mut last: Option<Pubkey> = None;

    for line in txt.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(t) else {
            continue;
        };
        let Some(ev) = v.get("event").and_then(|x| x.as_str()) else {
            continue;
        };
        if !OPEN_LIFECYCLE_EVENTS.contains(&ev) {
            continue;
        }
        let row_pos = v
            .get("position_pubkey")
            .or_else(|| v.get("position_pda"))
            .and_then(|x| x.as_str())
            .map(str::trim);
        if row_pos != Some(pos_s.as_str()) {
            continue;
        }
        let from_fee_payer = v
            .get("fee_payer_pubkey")
            .and_then(|x| x.as_str())
            .and_then(|s| Pubkey::from_str(s.trim()).ok())
            .filter(is_plausible_owner_pubkey);
        let from_owner_field = v
            .get("owner_pubkey")
            .and_then(|x| x.as_str())
            .and_then(|s| Pubkey::from_str(s.trim()).ok())
            .filter(is_plausible_owner_pubkey);
        let from_details = v.get("details").and_then(|d| {
            d.get("owner_pubkey")
                .or_else(|| d.get("fee_payer_pubkey"))
                .and_then(|x| x.as_str())
                .and_then(|s| Pubkey::from_str(s.trim()).ok())
                .filter(is_plausible_owner_pubkey)
        });
        if let Some(pk) = from_fee_payer.or(from_owner_field).or(from_details) {
            last = Some(pk);
        }
    }
    last
}

async fn owner_from_running_strategy(state: &AppState, position: &Pubkey) -> Option<Pubkey> {
    let pos_s = position.to_string();
    let running_ids: Vec<String> = {
        let strategies = state.strategies.read().await;
        strategies
            .values()
            .filter(|s| s.running)
            .filter(|s| {
                s.config
                    .get("parameters")
                    .and_then(|p| p.get("position_addresses"))
                    .and_then(|v| v.as_array())
                    .is_some_and(|arr| {
                        arr.iter()
                            .any(|v| v.as_str().map(str::trim) == Some(pos_s.as_str()))
                    })
            })
            .map(|s| s.id.clone())
            .collect()
    };
    for sid in running_ids {
        let Some(exec) = state.executors.read().await.get(&sid).cloned() else {
            continue;
        };
        let guard = exec.read().await;
        if let Some(pk) = guard.wallet_pubkey().filter(is_plausible_owner_pubkey) {
            return Some(pk);
        }
    }
    None
}

async fn position_mint_from_rpc(provider: &Arc<RpcProvider>, position: &Pubkey) -> Option<Pubkey> {
    let account = provider.get_account(position).await.ok()?;
    let parsed = WhirlpoolPosition::try_from_slice(&account.data).ok()?;
    Some(parsed.position_mint)
}

async fn nft_owner_via_rpc(provider: &Arc<RpcProvider>, position: &Pubkey) -> Option<Pubkey> {
    let mint = position_mint_from_rpc(provider, position).await?;
    let holders = provider.get_token_largest_accounts(&mint).await.ok()?;
    let token_account = holders
        .iter()
        .max_by_key(|h| h.amount.amount.parse::<u64>().unwrap_or(0))
        .and_then(|h| Pubkey::from_str(h.address.as_str()).ok())?;
    let account = provider.get_account(&token_account).await.ok()?;
    let token = SplTokenAccount::unpack(&account.data).ok()?;
    let owner = token.owner;
    is_plausible_owner_pubkey(&owner).then_some(owner)
}

async fn resolve_owner_effective(state: &AppState, position: &Pubkey) -> Option<Pubkey> {
    if let Some(pk) = owner_from_registry(position) {
        return Some(pk);
    }
    if let Some(pk) = owner_from_lifecycle_ledger(position) {
        return Some(pk);
    }
    if let Some(pk) = owner_from_running_strategy(state, position).await {
        return Some(pk);
    }
    nft_owner_via_rpc(&state.provider, position).await
}

/// Resolve close signer for one position; returns skip row when no API wallet matches owner.
pub async fn resolve_close_signer_for_position(
    state: &AppState,
    address: &str,
) -> Result<Result<CloseSignerResolution, CloseSignerSkipped>, String> {
    let position = Pubkey::from_str(address.trim())
        .map_err(|_| "invalid position address".to_string())?;
    let addr = position.to_string();

    let Some(owner) = resolve_owner_effective(state, &position).await else {
        return Ok(Err(CloseSignerSkipped {
            address: addr,
            reason: CloseSignerSkipReason::UnmanagedSigner,
            owner_pubkey: None,
        }));
    };
    let owner_s = owner.to_string();
    let Some(wallet_id) = wallet_id_for_pubkey(state, &owner_s) else {
        return Ok(Err(CloseSignerSkipped {
            address: addr,
            reason: CloseSignerSkipReason::UnmanagedSigner,
            owner_pubkey: Some(owner_s),
        }));
    };
    Ok(Ok(CloseSignerResolution {
        address: addr,
        owner_pubkey: owner_s.clone(),
        close_signer_wallet_id: wallet_id,
        close_signer_pubkey: owner_s,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{ApiConfig, AppState};
    use clmm_lp_data::repositories::Database;
    use clmm_lp_protocols::prelude::RpcConfig;
    use solana_sdk::signature::{Keypair, Signer};
    use std::io::Write;
    use tempfile::TempDir;

    fn test_state(wallets_dir: &std::path::Path, registry_path: &std::path::Path) -> AppState {
        let mut cfg = ApiConfig::default();
        cfg.wallets_dir_primary = Some(wallets_dir.to_string_lossy().to_string());
        unsafe {
            std::env::set_var(
                "CLMM_POSITION_REGISTRY_PATH",
                registry_path.to_string_lossy().to_string(),
            );
        }
        AppState::new(RpcConfig::default(), cfg, None::<Database>)
    }

    #[test]
    fn registry_owner_preferred_over_default() {
        let tmp = TempDir::new().expect("tempdir");
        let kp = Keypair::new();
        let owner = kp.pubkey();
        solana_sdk::signature::write_keypair_file(&kp, tmp.path().join("main.json"))
            .expect("write kp");

        let reg_path = tmp.path().join("registry.jsonl");
        let pos = Pubkey::new_unique();
        let pool = Pubkey::new_unique();
        let line = serde_json::json!({
            "event": "registry_open",
            "position_pubkey": pos.to_string(),
            "pool_address": pool.to_string(),
            "owner_pubkey": owner.to_string(),
        });
        let mut f = std::fs::File::create(&reg_path).expect("create reg");
        writeln!(f, "{line}").expect("write reg");

        unsafe {
            std::env::set_var("CLMM_POSITION_REGISTRY_PATH", reg_path.to_string_lossy().to_string());
        }
        let snap = registry_last_open_snapshot(&pos).expect("snapshot");
        assert_eq!(snap.owner, owner);
    }

    #[tokio::test]
    async fn resolves_wallet_id_from_registry_owner() {
        let tmp = TempDir::new().expect("tempdir");
        let kp = Keypair::new();
        let owner = kp.pubkey();
        solana_sdk::signature::write_keypair_file(&kp, tmp.path().join("alpha.json"))
            .expect("write kp");

        let reg_path = tmp.path().join("registry.jsonl");
        let pos = Pubkey::new_unique();
        let pool = Pubkey::new_unique();
        let line = serde_json::json!({
            "event": "registry_open",
            "position_pubkey": pos.to_string(),
            "pool_address": pool.to_string(),
            "owner_pubkey": owner.to_string(),
        });
        let mut f = std::fs::File::create(&reg_path).expect("create reg");
        writeln!(f, "{line}").expect("write reg");

        let state = test_state(tmp.path(), &reg_path);
        let resolved = resolve_close_signer_for_position(&state, &pos.to_string())
            .await
            .expect("resolve")
            .expect("ok");
        assert_eq!(resolved.close_signer_wallet_id, "alpha");
        assert_eq!(resolved.close_signer_pubkey, owner.to_string());
    }
}
