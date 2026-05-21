//! Load API host wallet keypairs from configured stores (primary / secondary).

use crate::error::ApiError;
use crate::state::AppState;
use clmm_lp_execution::prelude::Wallet;
use solana_sdk::signature::{read_keypair_file, Signer};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct WalletStores {
    pub primary: PathBuf,
    pub secondary: Option<PathBuf>,
}

#[must_use]
pub fn resolve_wallet_stores(state: &AppState) -> WalletStores {
    let primary = state
        .config
        .wallets_dir_primary
        .as_ref()
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| state.config.wallets_dir.as_ref().map(PathBuf::from))
        .or_else(|| std::env::var("CLMM_WALLETS_DIR_PRIMARY").ok().map(PathBuf::from))
        .or_else(|| std::env::var("CLMM_WALLETS_DIR").ok().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("wallets"));
    let secondary = state
        .config
        .wallets_dir_secondary
        .as_ref()
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("CLMM_WALLETS_DIR_SECONDARY")
                .ok()
                .map(PathBuf::from)
        })
        .filter(|p| p != &primary);
    WalletStores { primary, secondary }
}

fn wallet_file_path(dir: &Path, wallet_id: &str) -> PathBuf {
    dir.join(format!("{wallet_id}.json"))
}

fn scan_wallet_dir(dir: &Path, out: &mut BTreeMap<String, String>, is_primary: bool) {
    if !dir.exists() {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let id = p
            .file_stem()
            .and_then(|x| x.to_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if id.is_empty() {
            continue;
        }
        let Ok(kp) = read_keypair_file(&p) else {
            continue;
        };
        let pubkey = kp.pubkey().to_string();
        if is_primary || !out.contains_key(&id) {
            out.insert(id, pubkey);
        }
    }
}

/// `(wallet_id, pubkey)` entries discovered on the API host.
#[must_use]
pub fn list_api_wallet_pubkeys(state: &AppState) -> Vec<(String, String)> {
    let stores = resolve_wallet_stores(state);
    let mut map: BTreeMap<String, String> = BTreeMap::new();
    scan_wallet_dir(&stores.primary, &mut map, true);
    if let Some(sec) = &stores.secondary {
        scan_wallet_dir(sec, &mut map, false);
    }
    map.into_iter().collect()
}

#[must_use]
pub fn wallet_id_for_pubkey(state: &AppState, pubkey: &str) -> Option<String> {
    let needle = pubkey.trim();
    if needle.is_empty() {
        return None;
    }
    list_api_wallet_pubkeys(state)
        .into_iter()
        .find(|(_, pk)| pk == needle)
        .map(|(id, _)| id)
}

pub fn load_api_wallet_by_id(state: &AppState, wallet_id: &str) -> Result<Arc<Wallet>, ApiError> {
    let wid = wallet_id.trim();
    if wid.is_empty() {
        return Err(ApiError::bad_request("wallet_id empty"));
    }
    let stores = resolve_wallet_stores(state);
    let p1 = wallet_file_path(&stores.primary, wid);
    if p1.exists() {
        let w = Wallet::from_file(&p1, "api-close-all").map_err(|e| {
            ApiError::bad_request(format!("wallet `{wid}` read failed from primary: {e}"))
        })?;
        return Ok(Arc::new(w));
    }
    if let Some(sec) = &stores.secondary {
        let p2 = wallet_file_path(sec, wid);
        if p2.exists() {
            let w = Wallet::from_file(&p2, "api-close-all").map_err(|e| {
                ApiError::bad_request(format!("wallet `{wid}` read failed from secondary: {e}"))
            })?;
            return Ok(Arc::new(w));
        }
    }
    Err(ApiError::bad_request(format!(
        "wallet `{wid}` not found in configured stores"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{ApiConfig, AppState};
    use clmm_lp_data::repositories::Database;
    use clmm_lp_protocols::prelude::RpcConfig;
    use solana_sdk::signature::{Keypair, Signer};
    use tempfile::TempDir;

    fn test_state_with_wallets_dir(dir: &Path) -> AppState {
        let mut cfg = ApiConfig::default();
        cfg.wallets_dir_primary = Some(dir.to_string_lossy().to_string());
        AppState::new(RpcConfig::default(), cfg, None::<Database>)
    }

    #[test]
    fn wallet_id_for_pubkey_finds_match() {
        let tmp = TempDir::new().expect("tempdir");
        let kp = Keypair::new();
        let path = tmp.path().join("alpha.json");
        solana_sdk::signature::write_keypair_file(&kp, &path).expect("write keypair");
        let state = test_state_with_wallets_dir(tmp.path());
        let pk = kp.pubkey().to_string();
        let id = wallet_id_for_pubkey(&state, &pk).expect("wallet id");
        assert_eq!(id, "alpha");
    }
}
