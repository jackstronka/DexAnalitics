//! Load Solana keypair for Orca CLI commands (`--keypair`, `KEYPAIR_PATH`, `SOLANA_KEYPAIR`).

use anyhow::{Context, Result};
use clmm_lp_execution::prelude::Wallet;
use std::path::PathBuf;

/// Resolve signing wallet: `--keypair`, then `KEYPAIR_PATH` / `SOLANA_KEYPAIR_PATH` (file), then `SOLANA_KEYPAIR` (env JSON or base58).
pub fn load_signing_wallet(keypair: Option<PathBuf>) -> Result<Wallet> {
    if let Some(p) = keypair {
        return Wallet::from_file(&p, "clmm-lp-cli").context("load --keypair");
    }
    for var in ["KEYPAIR_PATH", "SOLANA_KEYPAIR_PATH"] {
        if let Ok(p) = std::env::var(var) {
            let p = p.trim();
            if !p.is_empty() && std::path::Path::new(p).exists() {
                return Wallet::from_file(p, "clmm-lp-cli").with_context(|| format!("load {var}"));
            }
        }
    }
    Wallet::from_env("SOLANA_KEYPAIR", "clmm-lp-cli").context("load SOLANA_KEYPAIR")
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::signature::{Keypair, Signer};
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn load_signing_wallet_prefers_explicit_path() {
        let kp = Keypair::new();
        let path = std::env::temp_dir().join(format!(
            "clmm_orca_kp_{}_{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let bytes = kp.to_bytes();
        fs::write(&path, serde_json::to_string(&bytes.to_vec()).unwrap()).unwrap();

        let w = load_signing_wallet(Some(PathBuf::from(&path))).expect("load file");
        assert_eq!(w.pubkey(), kp.pubkey());

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn load_signing_wallet_reads_keypair_path_env() {
        let kp = Keypair::new();
        let path = std::env::temp_dir().join(format!(
            "clmm_orca_kp_env_{}_{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let bytes = kp.to_bytes();
        fs::write(&path, serde_json::to_string(&bytes.to_vec()).unwrap()).unwrap();

        let old = std::env::var("KEYPAIR_PATH").ok();
        unsafe { std::env::set_var("KEYPAIR_PATH", path.to_str().unwrap()) };
        let w = load_signing_wallet(None).expect("env path");
        assert_eq!(w.pubkey(), kp.pubkey());
        match old {
            Some(v) => unsafe { std::env::set_var("KEYPAIR_PATH", v) },
            None => unsafe { std::env::remove_var("KEYPAIR_PATH") },
        }
        let _ = fs::remove_file(&path);
    }
}
