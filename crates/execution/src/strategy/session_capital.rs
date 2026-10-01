//! Logical portfolio caps for reopen: SESSION (per rebalance) or CHAIN (full rotation cycle).

use clmm_lp_data::repositories::Database;
pub use clmm_lp_data::wallet_session::{SessionCapsSource, SessionMintCaps};
use clmm_lp_data::wallet_session::{resolve_chain_mint_caps, resolve_session_mint_caps};
use clmm_lp_protocols::orca::deposit_quote::DepositBudgetQuote;
use clmm_lp_protocols::rpc::RpcProvider;
use solana_sdk::pubkey::Pubkey;
use spl_token::solana_program::program_pack::Pack;
use spl_token::state::Account as SplTokenAccount;

/// Native SOL held back for fees/rent before wrap or in-pool swap (matches swap-mix).
const CHAIN_NATIVE_SOL_RESERVE_LAMPORTS: u64 = 10_000_000;

pub fn reopen_use_session_capital() -> bool {
    match std::env::var("CLMM_REOPEN_USE_SESSION_CAPITAL") {
        Ok(v) => matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => false,
    }
}

/// Explicit env override for CHAIN portfolio caps (`CLMM_REOPEN_USE_CHAIN_PORTFOLIO`).
/// `None` = unset → auto when [`chain_portfolio_enabled`] sees a non-empty `chain_session_id`.
pub fn reopen_use_chain_portfolio_env() -> Option<bool> {
    match std::env::var("CLMM_REOPEN_USE_CHAIN_PORTFOLIO") {
        Ok(v) => {
            let v = v.trim().to_ascii_lowercase();
            if matches!(v.as_str(), "0" | "false" | "no" | "off") {
                Some(false)
            } else {
                Some(matches!(v.as_str(), "1" | "true" | "yes" | "on"))
            }
        }
        Err(_) => None,
    }
}

/// Legacy name: true only when env explicitly enables CHAIN caps (ignores auto-by-id).
#[allow(dead_code)]
pub fn reopen_use_chain_portfolio() -> bool {
    reopen_use_chain_portfolio_env() == Some(true)
}

/// CHAIN-first funding: on when env is on, or env unset and `chain_session_id` is present.
pub fn chain_portfolio_enabled(chain_session_id: Option<&str>) -> bool {
    match reopen_use_chain_portfolio_env() {
        Some(true) => true,
        Some(false) => false,
        None => chain_session_id
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .is_some(),
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

pub fn reopen_chain_strict_empty() -> bool {
    match std::env::var("CLMM_REOPEN_CHAIN_STRICT_EMPTY") {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        ),
        Err(_) => reopen_session_strict_empty(),
    }
}

/// `min(RPC, portfolio cap)` when portfolio caps are loaded (flag already applied at load time).
pub fn cap_rpc_with_portfolio(
    rpc_raw: u64,
    mint: &Pubkey,
    portfolio: Option<&SessionMintCaps>,
) -> u64 {
    let Some(sc) = portfolio else {
        return rpc_raw;
    };
    let mint_s = mint.to_string();
    rpc_raw.min(sc.cap_u64_for_mint(&mint_s))
}

pub fn cap_rpc_with_session(rpc_raw: u64, mint: &Pubkey, session: Option<&SessionMintCaps>) -> u64 {
    let Some(sc) = session.filter(|_| reopen_use_session_capital()) else {
        return rpc_raw;
    };
    cap_rpc_with_portfolio(rpc_raw, mint, Some(sc))
}

#[must_use]
pub fn chain_native_spendable_lamports(native_lamports: u64) -> u64 {
    native_lamports.saturating_sub(CHAIN_NATIVE_SOL_RESERVE_LAMPORTS)
}

/// When opening on a WSOL pool leg, treat spendable native SOL as available on that leg.
pub fn apply_portfolio_caps_to_wallet_raw(
    balance_a_raw: u64,
    balance_b_raw: u64,
    spendable_lamports: u64,
    token_mint_a: &Pubkey,
    token_mint_b: &Pubkey,
    wsol_mint_pk: &Pubkey,
    portfolio: Option<&SessionMintCaps>,
) -> (u64, u64, u64) {
    let wa = cap_rpc_with_portfolio(balance_a_raw, token_mint_a, portfolio);
    let wb = cap_rpc_with_portfolio(balance_b_raw, token_mint_b, portfolio);
    let spend = if token_mint_a == wsol_mint_pk {
        cap_rpc_with_portfolio(spendable_lamports, token_mint_a, portfolio)
    } else if token_mint_b == wsol_mint_pk {
        cap_rpc_with_portfolio(spendable_lamports, token_mint_b, portfolio)
    } else {
        spendable_lamports
    };
    (wa, wb, spend)
}

#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn chain_wallet_notional_usd_sol_first(
    balance_a_raw: u64,
    balance_b_raw: u64,
    spendable_lamports: u64,
    token_mint_a: &Pubkey,
    token_mint_b: &Pubkey,
    wsol_mint_pk: &Pubkey,
    decimals_a: u8,
    decimals_b: u8,
    price_a_usd: f64,
    price_b_usd: f64,
) -> f64 {
    let mut a_ui = ui_from_raw(balance_a_raw, decimals_a);
    let mut b_ui = ui_from_raw(balance_b_raw, decimals_b);
    let spendable_ui = spendable_lamports as f64 / 1e9;
    if token_mint_a == wsol_mint_pk {
        a_ui = a_ui.max(spendable_ui);
    }
    if token_mint_b == wsol_mint_pk {
        b_ui = b_ui.max(spendable_ui);
    }
    a_ui * price_a_usd + b_ui * price_b_usd
}

#[must_use]
pub fn clamp_target_usd_to_chain_wallet_notional(target_usd: f64, wallet_notional_usd: f64) -> f64 {
    let wallet_cap = (wallet_notional_usd * 0.995).max(0.0);
    if target_usd.is_finite() && target_usd > 0.0 {
        target_usd.min(wallet_cap)
    } else {
        wallet_cap
    }
}

/// Clamp deposit quote caps to CHAIN/SESSION inventory (never suggest global-wallet sizing).
#[must_use]
pub fn clamp_deposit_quote_to_portfolio(
    q: &DepositBudgetQuote,
    portfolio: &SessionMintCaps,
    token_mint_a: &Pubkey,
    token_mint_b: &Pubkey,
) -> DepositBudgetQuote {
    let mut out = q.clone();
    out.token_max_a = cap_rpc_with_portfolio(out.token_max_a, token_mint_a, Some(portfolio));
    out.token_max_b = cap_rpc_with_portfolio(out.token_max_b, token_mint_b, Some(portfolio));
    out.amount_a = out.amount_a.min(out.token_max_a);
    out.amount_b = out.amount_b.min(out.token_max_b);
    out
}

pub fn portfolio_scope_label(scope: ReopenPortfolioScope) -> &'static str {
    match scope {
        ReopenPortfolioScope::Chain => "CHAIN",
        ReopenPortfolioScope::Session => "SESSION",
    }
}

#[derive(Debug, Clone)]
pub struct ChainScopedPoolWallet {
    pub loaded: Option<LoadedReopenCaps>,
    pub balance_a_raw: u64,
    pub balance_b_raw: u64,
    pub spendable_lamports: u64,
}

/// RPC wallet balances capped to CHAIN/SESSION inventory (dedicated strategy account view).
pub async fn load_chain_scoped_pool_wallet(
    provider: &RpcProvider,
    db: Option<&Database>,
    owner: &Pubkey,
    token_mint_a: &Pubkey,
    token_mint_b: &Pubkey,
    rebalance_session_id: Option<&str>,
    chain_session_id: Option<&str>,
) -> Result<ChainScopedPoolWallet, String> {
    let loaded = load_reopen_portfolio_caps(
        db,
        rebalance_session_id,
        chain_session_id,
        Some(&owner.to_string()),
    )
    .await;
    if let Some(ref l) = loaded
        && let Some(err) = portfolio_capital_error_if_strict(l)
    {
        return Err(err);
    }
    let portfolio = loaded.as_ref().map(|l| &l.caps);
    let wa = spl_token_balance_raw(provider, owner, token_mint_a).await;
    let wb = spl_token_balance_raw(provider, owner, token_mint_b).await;
    let native = provider.get_balance(owner).await.unwrap_or(0);
    let wsol_mint_pk: Pubkey = clmm_lp_protocols::orca::executor::WSOL_MINT
        .parse()
        .map_err(|_| "WSOL mint parse".to_string())?;
    let native_spendable = chain_native_spendable_lamports(native);
    let (balance_a_raw, balance_b_raw, spendable_lamports) = apply_portfolio_caps_to_wallet_raw(
        wa,
        wb,
        native_spendable,
        token_mint_a,
        token_mint_b,
        &wsol_mint_pk,
        portfolio,
    );
    Ok(ChainScopedPoolWallet {
        loaded,
        balance_a_raw,
        balance_b_raw,
        spendable_lamports,
    })
}

fn ui_from_raw(raw: u64, decimals: u8) -> f64 {
    raw as f64 / 10f64.powi(i32::from(decimals))
}

fn associated_token_address(owner: &Pubkey, mint: &Pubkey) -> Pubkey {
    spl_associated_token_address::get_associated_token_address(owner, mint, &spl_token::id())
}

mod spl_associated_token_address {
    use solana_sdk::pubkey;
    use solana_sdk::pubkey::Pubkey;

    const ASSOCIATED_TOKEN_PROGRAM_ID: Pubkey =
        pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

    pub fn get_associated_token_address(
        wallet_address: &Pubkey,
        token_mint_address: &Pubkey,
        token_program_id: &Pubkey,
    ) -> Pubkey {
        Pubkey::find_program_address(
            &[
                wallet_address.as_ref(),
                token_program_id.as_ref(),
                token_mint_address.as_ref(),
            ],
            &ASSOCIATED_TOKEN_PROGRAM_ID,
        )
        .0
    }
}

async fn spl_token_balance_raw(provider: &RpcProvider, owner: &Pubkey, mint: &Pubkey) -> u64 {
    let ata = associated_token_address(owner, mint);
    match provider.get_account(&ata).await {
        Ok(acc) => SplTokenAccount::unpack(&acc.data)
            .map(|t| t.amount)
            .unwrap_or(0),
        Err(_) => 0,
    }
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

pub fn chain_caps_source_label(source: SessionCapsSource) -> &'static str {
    match source {
        SessionCapsSource::Gl => "gl_chain",
        SessionCapsSource::PslrFallback => "pslr_fallback",
        SessionCapsSource::ReconciledMin => "reconciled_min",
        SessionCapsSource::LifecycleFile => "lifecycle_file",
        SessionCapsSource::Empty => "empty",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReopenPortfolioScope {
    Chain,
    Session,
}

#[derive(Debug, Clone)]
pub struct LoadedReopenCaps {
    pub caps: SessionMintCaps,
    pub scope: ReopenPortfolioScope,
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

pub async fn load_chain_mint_caps(
    db: Option<&Database>,
    chain_session_id: &str,
    owner: Option<&str>,
) -> Option<SessionMintCaps> {
    if !chain_portfolio_enabled(Some(chain_session_id)) {
        return None;
    }
    let cid = chain_session_id.trim();
    if cid.is_empty() {
        return None;
    }
    let caps = resolve_chain_mint_caps(db, cid, owner).await;
    if !caps.is_empty() {
        return Some(caps);
    }
    if reopen_chain_strict_empty()
        && clmm_lp_data::wallet_session::chain_has_funding_lifecycle_rows(cid)
    {
        return Some(caps);
    }
    if caps.is_empty() {
        return None;
    }
    Some(caps)
}

/// Prefer CHAIN caps when [`chain_portfolio_enabled`], else SESSION per rebalance.
pub async fn load_reopen_portfolio_caps(
    db: Option<&Database>,
    rebalance_session_id: Option<&str>,
    chain_session_id: Option<&str>,
    owner: Option<&str>,
) -> Option<LoadedReopenCaps> {
    if chain_portfolio_enabled(chain_session_id)
        && let Some(cid) = chain_session_id.map(str::trim).filter(|s| !s.is_empty())
    {
        if let Some(caps) = load_chain_mint_caps(db, cid, owner).await {
            return Some(LoadedReopenCaps {
                caps,
                scope: ReopenPortfolioScope::Chain,
            });
        }
        if reopen_chain_strict_empty()
            && clmm_lp_data::wallet_session::chain_has_funding_lifecycle_rows(cid)
        {
            return Some(LoadedReopenCaps {
                caps: SessionMintCaps::empty(cid.to_string()),
                scope: ReopenPortfolioScope::Chain,
            });
        }
    }
    if let Some(sid) = rebalance_session_id
        .map(str::trim)
        .filter(|s| !s.is_empty())
        && let Some(caps) = load_session_mint_caps(db, sid, owner).await
    {
        return Some(LoadedReopenCaps {
            caps,
            scope: ReopenPortfolioScope::Session,
        });
    }
    None
}

pub fn portfolio_capital_error_if_strict(loaded: &LoadedReopenCaps) -> Option<String> {
    let (strict, use_flag, label, source_label) = match loaded.scope {
        ReopenPortfolioScope::Chain => (
            reopen_chain_strict_empty(),
            chain_portfolio_enabled(Some(loaded.caps.session_id.as_str())),
            "CHAIN",
            chain_caps_source_label(loaded.caps.source),
        ),
        ReopenPortfolioScope::Session => (
            reopen_session_strict_empty(),
            reopen_use_session_capital(),
            "SESSION",
            session_caps_source_label(loaded.caps.source),
        ),
    };
    if !use_flag || !strict {
        return None;
    }
    if loaded.caps.is_empty() {
        Some(format!(
            "portfolio_capital_unknown: no {label} inventory for {} (source={source_label})",
            loaded.caps.session_id
        ))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_env::EnvGuard;
    use clmm_lp_data::wallet_session::SessionCapsSource;

    fn write_empty_lifecycle_jsonl(dir: &tempfile::TempDir) -> String {
        let path = dir.path().join("lifecycle.jsonl");
        std::fs::write(&path, "").expect("write empty jsonl");
        path.to_string_lossy().into_owned()
    }

    #[tokio::test]
    async fn load_session_mint_caps_none_when_flag_off() {
        let mut env = EnvGuard::lock().await;
        let dir = tempfile::tempdir().expect("tempdir");
        let path_s = write_empty_lifecycle_jsonl(&dir);
        env.set("CLMM_POSITION_LIFECYCLE_LEDGER_PATH", &path_s);
        env.set("CLMM_REOPEN_USE_SESSION_CAPITAL", "0");
        env.set("CLMM_REOPEN_USE_CHAIN_PORTFOLIO", "0");
        let out = load_session_mint_caps(None, "any-session", None).await;
        assert!(out.is_none());
    }

    #[test]
    fn clamp_target_usd_to_chain_wallet_notional_caps_at_inventory() {
        assert!((clamp_target_usd_to_chain_wallet_notional(10.0, 9.0) - 8.955).abs() < 1e-9);
        assert!((clamp_target_usd_to_chain_wallet_notional(5.0, 20.0) - 5.0).abs() < 1e-9);
    }

    #[test]
    fn clamp_deposit_quote_to_portfolio_limits_token_max() {
        let mint_a = Pubkey::new_unique();
        let mint_b = Pubkey::new_unique();
        let mut caps = SessionMintCaps::empty("chain-1");
        caps.caps_by_mint.insert(mint_a.to_string(), 100);
        caps.caps_by_mint.insert(mint_b.to_string(), 50);
        let q = DepositBudgetQuote {
            amount_a: 500,
            amount_b: 400,
            token_max_a: 500,
            token_max_b: 400,
            estimated_value_usd: 10.0,
            liquidity: 1,
        };
        let out = clamp_deposit_quote_to_portfolio(&q, &caps, &mint_a, &mint_b);
        assert_eq!(out.token_max_a, 100);
        assert_eq!(out.token_max_b, 50);
        assert_eq!(out.amount_a, 100);
        assert_eq!(out.amount_b, 50);
    }

    #[tokio::test]
    async fn load_reopen_portfolio_auto_chain_when_id_present_without_env() {
        let mut env = EnvGuard::lock().await;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("lifecycle.jsonl");
        let chain_id = "chain-auto-1";
        let line = serde_json::json!({
            "event": "bot_close_position",
            "signature": "sig-auto-1",
            "chain_session_id": chain_id,
            "details": {
                "token_mint_a": clmm_lp_data::wallet_session::WSOL_MINT,
                "token_mint_b": clmm_lp_data::wallet_session::USDC_MINT,
                "close_amount_a_raw": 3_000u64,
                "close_amount_b_raw": 50u64
            }
        });
        std::fs::write(&path, line.to_string()).expect("write jsonl");
        env.set("CLMM_POSITION_LIFECYCLE_LEDGER_PATH", &path);
        env.remove("CLMM_REOPEN_USE_CHAIN_PORTFOLIO");
        env.remove("CLMM_REOPEN_USE_SESSION_CAPITAL");
        let loaded = load_reopen_portfolio_caps(None, None, Some(chain_id), None)
            .await
            .expect("auto chain caps");
        assert_eq!(loaded.scope, ReopenPortfolioScope::Chain);
        assert_eq!(
            loaded
                .caps
                .cap_u64_for_mint(clmm_lp_data::wallet_session::WSOL_MINT),
            3_000
        );
    }

    #[test]
    fn chain_portfolio_enabled_respects_explicit_off() {
        let mut env = EnvGuard::blocking_lock();
        env.set("CLMM_REOPEN_USE_CHAIN_PORTFOLIO", "0");
        assert!(!chain_portfolio_enabled(Some("some-chain-id")));
        env.remove("CLMM_REOPEN_USE_CHAIN_PORTFOLIO");
        assert!(chain_portfolio_enabled(Some("some-chain-id")));
        assert!(!chain_portfolio_enabled(None));
    }

    #[tokio::test]
    async fn load_reopen_portfolio_prefers_chain_when_both_flags() {
        let mut env = EnvGuard::lock().await;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("lifecycle.jsonl");
        let chain_id = "chain-pref-1";
        let rebalance_id = "rebalance-other";
        let line = serde_json::json!({
            "event": "bot_close_position",
            "signature": "sig-chain-1",
            "chain_session_id": chain_id,
            "rebalance_session_id": rebalance_id,
            "details": {
                "token_mint_a": clmm_lp_data::wallet_session::WSOL_MINT,
                "token_mint_b": clmm_lp_data::wallet_session::USDC_MINT,
                "close_amount_a_raw": 2_000u64,
                "close_amount_b_raw": 100u64
            }
        });
        std::fs::write(&path, line.to_string()).expect("write jsonl");
        env.set("CLMM_POSITION_LIFECYCLE_LEDGER_PATH", &path);
        env.set("CLMM_REOPEN_USE_CHAIN_PORTFOLIO", "1");
        env.set("CLMM_REOPEN_USE_SESSION_CAPITAL", "1");
        env.remove("CLMM_REOPEN_CHAIN_STRICT_EMPTY");
        let loaded = load_reopen_portfolio_caps(None, Some(rebalance_id), Some(chain_id), None)
            .await
            .expect("chain caps");
        assert_eq!(loaded.scope, ReopenPortfolioScope::Chain);
        assert_eq!(
            loaded
                .caps
                .cap_u64_for_mint(clmm_lp_data::wallet_session::WSOL_MINT),
            2_000
        );
    }

    #[tokio::test]
    async fn cap_rpc_with_portfolio_limits_without_session_flag() {
        let mut env = EnvGuard::lock().await;
        let mint = Pubkey::new_unique();
        let mut caps = SessionMintCaps::empty("chain-1");
        caps.caps_by_mint.insert(mint.to_string(), 42);
        env.set("CLMM_REOPEN_USE_SESSION_CAPITAL", "0");
        assert_eq!(cap_rpc_with_portfolio(100, &mint, Some(&caps)), 42);
        assert_eq!(cap_rpc_with_session(100, &mint, Some(&caps)), 100);
    }

    #[tokio::test]
    async fn load_session_mint_caps_strict_empty_returns_empty_some() {
        let mut env = EnvGuard::lock().await;
        let dir = tempfile::tempdir().expect("tempdir");
        let path_s = write_empty_lifecycle_jsonl(&dir);
        env.set("CLMM_POSITION_LIFECYCLE_LEDGER_PATH", &path_s);
        env.set("CLMM_REOPEN_USE_SESSION_CAPITAL", "1");
        env.set("CLMM_REOPEN_SESSION_STRICT_EMPTY", "1");
        let out = load_session_mint_caps(None, "sess-no-rows", None)
            .await
            .expect("strict empty returns Some");
        assert!(out.is_empty());
        assert_eq!(out.source, SessionCapsSource::Empty);
    }

    #[tokio::test]
    async fn load_session_mint_caps_reads_jsonl_inventory() {
        let mut env = EnvGuard::lock().await;
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
        env.set("CLMM_POSITION_LIFECYCLE_LEDGER_PATH", &path);
        env.set("CLMM_REOPEN_USE_SESSION_CAPITAL", "1");
        env.remove("CLMM_REOPEN_SESSION_STRICT_EMPTY");
        let out = load_session_mint_caps(None, sid, None)
            .await
            .expect("inventory");
        assert_eq!(
            out.cap_u64_for_mint(clmm_lp_data::wallet_session::WSOL_MINT),
            1_500
        );
        assert_eq!(
            out.cap_u64_for_mint(clmm_lp_data::wallet_session::USDC_MINT),
            250
        );
    }
}
