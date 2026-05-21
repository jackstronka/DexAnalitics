//! SESSION logical portfolio caps for reopen (policy 5a).

use clmm_lp_data::repositories::Database;
pub use clmm_lp_data::wallet_session::{SessionCapsSource, SessionMintCaps};
use clmm_lp_data::wallet_session::resolve_session_mint_caps;
use solana_sdk::pubkey::Pubkey;
pub fn reopen_use_session_capital() -> bool {
    match std::env::var("CLMM_REOPEN_USE_SESSION_CAPITAL") {
        Ok(v) => matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => false,
    }
}

pub fn reopen_session_strict_empty() -> bool {
    match std::env::var("CLMM_REOPEN_SESSION_STRICT_EMPTY") {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        ),
        Err(_) => true,
    }
}

pub fn cap_rpc_with_session(rpc_raw: u64, mint: &Pubkey, session: Option<&SessionMintCaps>) -> u64 {
    let Some(sc) = session.filter(|_| reopen_use_session_capital()) else {
        return rpc_raw;
    };
    let mint_s = mint.to_string();
    rpc_raw.min(sc.cap_u64_for_mint(&mint_s))
}

pub fn session_caps_source_label(source: SessionCapsSource) -> &'static str {
    match source {
        SessionCapsSource::Gl => "gl_session",
        SessionCapsSource::PslrFallback => "pslr_fallback",
        SessionCapsSource::ReconciledMin => "reconciled_min",
        SessionCapsSource::LifecycleFile => "lifecycle_file",
        SessionCapsSource::Empty => "empty",
    }
}

pub async fn load_session_mint_caps(
    db: Option<&Database>,
    session_id: &str,
    owner: Option<&str>,
) -> Option<SessionMintCaps> {
    if !reopen_use_session_capital() {
        return None;
    }
    let sid = session_id.trim();
    if sid.is_empty() {
        return None;
    }
    let caps = resolve_session_mint_caps(db, sid, owner).await;
    if caps.is_empty() && reopen_session_strict_empty() {
        return Some(caps);
    }
    if caps.is_empty() {
        return None;
    }
    Some(caps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clmm_lp_data::wallet_session::SessionCapsSource;

    fn write_empty_lifecycle_jsonl(dir: &tempfile::TempDir) -> String {
        let path = dir.path().join("lifecycle.jsonl");
        std::fs::write(&path, "").expect("write empty jsonl");
        path.to_string_lossy().into_owned()
    }

    #[tokio::test]
    async fn load_session_mint_caps_none_when_flag_off() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path_s = write_empty_lifecycle_jsonl(&dir);
        unsafe {
            std::env::set_var("CLMM_POSITION_LIFECYCLE_LEDGER_PATH", &path_s);
            std::env::set_var("CLMM_REOPEN_USE_SESSION_CAPITAL", "0");
        }
        let out = load_session_mint_caps(None, "any-session", None).await;
        assert!(out.is_none());
        unsafe {
            std::env::remove_var("CLMM_REOPEN_USE_SESSION_CAPITAL");
            std::env::remove_var("CLMM_POSITION_LIFECYCLE_LEDGER_PATH");
        }
    }

    #[tokio::test]
    async fn load_session_mint_caps_strict_empty_returns_empty_some() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path_s = write_empty_lifecycle_jsonl(&dir);
        unsafe {
            std::env::set_var("CLMM_POSITION_LIFECYCLE_LEDGER_PATH", &path_s);
            std::env::set_var("CLMM_REOPEN_USE_SESSION_CAPITAL", "1");
            std::env::set_var("CLMM_REOPEN_SESSION_STRICT_EMPTY", "1");
        }
        let out = load_session_mint_caps(None, "sess-no-rows", None)
            .await
            .expect("strict empty returns Some");
        assert!(out.is_empty());
        assert_eq!(out.source, SessionCapsSource::Empty);
        unsafe {
            std::env::remove_var("CLMM_REOPEN_USE_SESSION_CAPITAL");
            std::env::remove_var("CLMM_REOPEN_SESSION_STRICT_EMPTY");
            std::env::remove_var("CLMM_POSITION_LIFECYCLE_LEDGER_PATH");
        }
    }

    #[tokio::test]
    async fn load_session_mint_caps_non_strict_empty_returns_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path_s = write_empty_lifecycle_jsonl(&dir);
        unsafe {
            std::env::set_var("CLMM_POSITION_LIFECYCLE_LEDGER_PATH", &path_s);
            std::env::set_var("CLMM_REOPEN_USE_SESSION_CAPITAL", "1");
            std::env::set_var("CLMM_REOPEN_SESSION_STRICT_EMPTY", "0");
        }
        let out = load_session_mint_caps(None, "sess-no-rows", None).await;
        assert!(out.is_none());
        unsafe {
            std::env::remove_var("CLMM_REOPEN_USE_SESSION_CAPITAL");
            std::env::remove_var("CLMM_REOPEN_SESSION_STRICT_EMPTY");
            std::env::remove_var("CLMM_POSITION_LIFECYCLE_LEDGER_PATH");
        }
    }

    #[tokio::test]
    async fn load_session_mint_caps_reads_jsonl_inventory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("lifecycle.jsonl");
        let sid = "sess-load-1";
        let line = serde_json::json!({
            "event": "bot_close_position",
            "signature": "sig-1",
            "rebalance_session_id": sid,
            "details": {
                "token_mint_a": clmm_lp_data::wallet_session::WSOL_MINT,
                "token_mint_b": clmm_lp_data::wallet_session::USDC_MINT,
                "close_amount_a_raw": 1_500u64,
                "close_amount_b_raw": 250u64
            }
        });
        std::fs::write(&path, line.to_string()).expect("write jsonl");
        let path_s = path.to_string_lossy().to_string();
        unsafe {
            std::env::set_var("CLMM_POSITION_LIFECYCLE_LEDGER_PATH", &path_s);
            std::env::set_var("CLMM_REOPEN_USE_SESSION_CAPITAL", "1");
            std::env::remove_var("CLMM_REOPEN_SESSION_STRICT_EMPTY");
        }
        let out = load_session_mint_caps(None, sid, None)
            .await
            .expect("inventory");
        assert_eq!(out.cap_u64_for_mint(clmm_lp_data::wallet_session::WSOL_MINT), 1_500);
        assert_eq!(out.cap_u64_for_mint(clmm_lp_data::wallet_session::USDC_MINT), 250);
        unsafe {
            std::env::remove_var("CLMM_REOPEN_USE_SESSION_CAPITAL");
            std::env::remove_var("CLMM_POSITION_LIFECYCLE_LEDGER_PATH");
        }
    }
}
