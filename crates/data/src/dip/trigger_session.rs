//! Per-wallet Jupiter Trigger JWT sessions (Faza E prep — attach only, no create/cancel).
//!
//! Stored under `data/dip/trigger_sessions/{wallet_id}.json` (gitignored via `/data/`).
//! Never log `access_token` / `refresh_token`.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::providers::TriggerAuthTokens;

/// On-disk session bound to a host wallet id + pubkey.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DipTriggerSession {
    pub wallet_id: String,
    pub wallet_pubkey: String,
    pub tokens: TriggerAuthTokens,
    /// RFC3339 when the session was written.
    pub attached_at: String,
}

#[must_use]
pub fn default_trigger_session_dir() -> PathBuf {
    PathBuf::from("data/dip/trigger_sessions")
}

#[must_use]
pub fn resolve_trigger_session_dir(override_dir: Option<&str>) -> PathBuf {
    if let Some(p) = override_dir.map(str::trim).filter(|s| !s.is_empty()) {
        return PathBuf::from(p);
    }
    if let Ok(p) = std::env::var("CLMM_DIP_TRIGGER_SESSION_DIR")
        && !p.trim().is_empty()
    {
        return PathBuf::from(p);
    }
    default_trigger_session_dir()
}

fn session_path(dir: &Path, wallet_id: &str) -> Result<PathBuf> {
    let id = wallet_id.trim();
    if id.is_empty() {
        bail!("wallet_id empty");
    }
    if id.contains(['/', '\\', '.', ':']) || id.contains("..") {
        bail!("wallet_id contains illegal characters");
    }
    Ok(dir.join(format!("{id}.json")))
}

pub fn save_trigger_session(dir: &Path, session: &DipTriggerSession) -> Result<PathBuf> {
    fs::create_dir_all(dir).with_context(|| format!("mkdir {}", dir.display()))?;
    let path = session_path(dir, &session.wallet_id)?;
    if session.tokens.wallet_pubkey.trim() != session.wallet_pubkey.trim() {
        bail!("tokens.wallet_pubkey mismatch vs session.wallet_pubkey");
    }
    let raw = serde_json::to_vec_pretty(session).context("serialize trigger session")?;
    fs::write(&path, raw).with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

pub fn load_trigger_session(dir: &Path, wallet_id: &str) -> Result<Option<DipTriggerSession>> {
    let path = session_path(dir, wallet_id)?;
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let session: DipTriggerSession =
        serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;
    Ok(Some(session))
}

pub fn delete_trigger_session(dir: &Path, wallet_id: &str) -> Result<bool> {
    let path = session_path(dir, wallet_id)?;
    if !path.exists() {
        return Ok(false);
    }
    fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;
    Ok(true)
}

/// Public status without JWT material.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DipTriggerSessionStatus {
    pub wallet_id: String,
    pub wallet_pubkey: String,
    pub attached: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attached_at: Option<String>,
}

impl DipTriggerSession {
    #[must_use]
    pub fn status(&self) -> DipTriggerSessionStatus {
        DipTriggerSessionStatus {
            wallet_id: self.wallet_id.clone(),
            wallet_pubkey: self.wallet_pubkey.clone(),
            attached: true,
            expires_at: self.tokens.expires_at.clone(),
            attached_at: Some(self.attached_at.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn sample(wallet_id: &str) -> DipTriggerSession {
        DipTriggerSession {
            wallet_id: wallet_id.into(),
            wallet_pubkey: "WalletPubkey111".into(),
            tokens: TriggerAuthTokens {
                access_token: "acc-secret".into(),
                refresh_token: Some("ref-secret".into()),
                expires_at: Some("2099-01-01T00:00:00Z".into()),
                wallet_pubkey: "WalletPubkey111".into(),
            },
            attached_at: "2026-11-09T22:00:00Z".into(),
        }
    }

    #[test]
    fn save_load_roundtrip() {
        let dir = tempdir().unwrap();
        let path = save_trigger_session(dir.path(), &sample("w1")).unwrap();
        assert!(path.exists());
        let loaded = load_trigger_session(dir.path(), "w1").unwrap().unwrap();
        assert_eq!(loaded.wallet_id, "w1");
        assert_eq!(loaded.tokens.access_token, "acc-secret");
        let st = loaded.status();
        assert!(st.attached);
        assert!(
            serde_json::to_string(&st)
                .unwrap()
                .contains("WalletPubkey111")
        );
        assert!(!serde_json::to_string(&st).unwrap().contains("acc-secret"));
    }

    #[test]
    fn rejects_path_traversal_wallet_id() {
        let dir = tempdir().unwrap();
        assert!(save_trigger_session(dir.path(), &sample("../x")).is_err());
    }

    #[test]
    fn delete_removes_file() {
        let dir = tempdir().unwrap();
        save_trigger_session(dir.path(), &sample("w2")).unwrap();
        assert!(delete_trigger_session(dir.path(), "w2").unwrap());
        assert!(load_trigger_session(dir.path(), "w2").unwrap().is_none());
    }
}
