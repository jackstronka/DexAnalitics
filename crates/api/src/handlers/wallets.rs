//! Local wallets (keypairs on API host) + on-chain read-only balances.

use crate::error::{ApiError, ApiResult};
use crate::models::{
    ApiSignerWalletResponse, WalletBalancesResponse, WalletEntry, WalletTokenBalance,
    WalletsListResponse,
};
use crate::services::position_executor::load_wallet_from_env;
use crate::state::AppState;
use axum::{Json, extract::Query, extract::State};
use serde::Deserialize;
use solana_sdk::{pubkey::Pubkey, signature::read_keypair_file, signer::Signer};
use std::collections::HashSet;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::LazyLock;
use std::time::Duration;

static HTTP: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .expect("reqwest client for wallet rpc")
});

fn resolve_wallets_dir(state: &AppState) -> PathBuf {
    state
        .config
        .wallets_dir
        .as_ref()
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var("CLMM_WALLETS_DIR").ok().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("wallets"))
}

/// List wallet keypair files from the API host (directory).
#[utoipa::path(
    get,
    path = "/wallets",
    tag = "Wallets",
    responses(
        (status = 200, description = "Wallets discovered on disk", body = WalletsListResponse),
        (status = 500, description = "I/O error")
    )
)]
pub async fn list_wallets(State(state): State<AppState>) -> ApiResult<Json<WalletsListResponse>> {
    let dir = resolve_wallets_dir(&state);
    let dir_s = dir.to_string_lossy().to_string();

    let mut wallets = Vec::new();
    if dir.exists() {
        let rd = std::fs::read_dir(&dir)
            .map_err(|e| ApiError::internal(format!("read_dir {dir_s}: {e}")))?;
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            let filename = p
                .file_name()
                .and_then(|x| x.to_str())
                .unwrap_or("")
                .to_string();
            if filename.is_empty() {
                continue;
            }
            let id = p
                .file_stem()
                .and_then(|x| x.to_str())
                .unwrap_or("")
                .to_string();
            if id.is_empty() {
                continue;
            }

            let pubkey = read_keypair_file(&p)
                .map(|kp| kp.pubkey().to_string())
                .unwrap_or_else(|_| "".to_string());
            if pubkey.is_empty() {
                continue;
            }

            wallets.push(WalletEntry {
                id,
                filename,
                pubkey,
            });
        }
    }

    wallets.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(Json(WalletsListResponse {
        wallets_dir: dir_s,
        wallets,
    }))
}

#[derive(Debug, Deserialize)]
pub struct WalletBalancesQuery {
    pub owner: String,
}

/// Read-only on-chain balances for a wallet owner (native SOL + SPL token accounts).
#[utoipa::path(
    get,
    path = "/wallets/balances",
    tag = "Wallets",
    params(
        ("owner" = String, Query, description = "Solana wallet pubkey (base58)")
    ),
    responses(
        (status = 200, description = "Balances from chain", body = WalletBalancesResponse),
        (status = 400, description = "Invalid owner pubkey")
    )
)]
pub async fn get_wallet_balances(
    State(state): State<AppState>,
    Query(q): Query<WalletBalancesQuery>,
) -> ApiResult<Json<WalletBalancesResponse>> {
    let owner_trim = q.owner.trim();
    if owner_trim.is_empty() {
        return Err(ApiError::bad_request("query parameter `owner` is required"));
    }
    let owner_pk =
        Pubkey::from_str(owner_trim).map_err(|_| ApiError::bad_request("invalid owner pubkey"))?;

    let primary = state.provider.current_endpoint().await;
    let mut urls: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut push = |u: String| {
        let t = u.trim().to_string();
        if t.is_empty() {
            return;
        }
        if seen.insert(t.clone()) {
            urls.push(t);
        }
    };
    push(primary.clone());
    if let Ok(fb) = std::env::var("SOLANA_RPC_URL_FALLBACK") {
        push(fb);
    }
    // Always include free public endpoints as last resort.
    push("https://api.mainnet-beta.solana.com".to_string());
    push("https://solana.publicnode.com".to_string());

    async fn rpc_call_try(
        urls: &[String],
        method: &str,
        params: serde_json::Value,
    ) -> Result<(String, serde_json::Value), String> {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params
        });
        let mut last_err: Option<String> = None;
        for url in urls {
            let resp = HTTP
                .post(url)
                .header("Content-Type", "application/json")
                .json(&body)
                // Keep this endpoint responsive; try next RPC on timeout.
                .timeout(Duration::from_secs(10))
                .send()
                .await;
            let resp = match resp {
                Ok(r) => r,
                Err(e) => {
                    last_err = Some(format!("{url}: request: {e}"));
                    continue;
                }
            };
            let status = resp.status();
            let text = match resp.text().await {
                Ok(t) => t,
                Err(e) => {
                    last_err = Some(format!("{url}: body: {e}"));
                    continue;
                }
            };
            // Some providers return plain text on failure (e.g. "Out of CU").
            if text.trim_start().starts_with("Out of CU") {
                last_err = Some(format!("{url}: {text}"));
                continue;
            }
            let v: serde_json::Value = match serde_json::from_str(&text) {
                Ok(v) => v,
                Err(e) => {
                    last_err = Some(format!(
                        "{url}: json: {e}; body={}",
                        text.chars().take(200).collect::<String>()
                    ));
                    continue;
                }
            };
            if !status.is_success() {
                last_err = Some(format!("{url}: http {status}: {v}"));
                continue;
            }
            if v.get("error").is_some() {
                last_err = Some(format!("{url}: rpc error: {v}"));
                continue;
            }
            return Ok((url.clone(), v));
        }
        Err(last_err.unwrap_or_else(|| "all rpc endpoints failed".to_string()))
    }

    let owner_s = owner_pk.to_string();
    let (rpc_url_used, bal_v) = rpc_call_try(&urls, "getBalance", serde_json::json!([owner_s]))
        .await
        .map_err(ApiError::internal)?;
    let lamports = bal_v["result"]["value"].as_u64().unwrap_or(0);
    let sol = format!("{:.9}", (lamports as f64) / 1e9);

    let tok_v = rpc_call_try(
        &urls,
        "getTokenAccountsByOwner",
        serde_json::json!([
            owner_pk.to_string(),
            { "programId": "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA" },
            { "encoding": "jsonParsed" }
        ]),
    )
    .await;

    let mut tokens = Vec::new();
    // If token RPC is slow/unavailable, we still return SOL (tokens empty).
    if let Ok((_u, v)) = tok_v
        && let Some(arr) = v["result"]["value"].as_array()
    {
        for entry in arr {
            let mint = entry["account"]["data"]["parsed"]["info"]["mint"]
                .as_str()
                .unwrap_or("")
                .to_string();
            if mint.is_empty() {
                continue;
            }
            // Prefer uiAmountString; fallback to uiAmount.
            let ui_amount =
                entry["account"]["data"]["parsed"]["info"]["tokenAmount"]["uiAmountString"]
                    .as_str()
                    .map(|s| s.to_string())
                    .or_else(|| {
                        entry["account"]["data"]["parsed"]["info"]["tokenAmount"]["uiAmount"]
                            .as_f64()
                            .map(|x| x.to_string())
                    })
                    .unwrap_or_else(|| "0".to_string());
            tokens.push(WalletTokenBalance { mint, ui_amount });
        }
    }
    tokens.sort_by(|a, b| a.mint.cmp(&b.mint));

    Ok(Json(WalletBalancesResponse {
        owner: owner_pk.to_string(),
        rpc_url: rpc_url_used,
        lamports,
        sol,
        tokens,
    }))
}

/// Returns the API signing wallet pubkey and SOL balance (wallet loaded from env).
#[utoipa::path(
    get,
    path = "/wallets/api-signer",
    tag = "Wallets",
    responses(
        (status = 200, description = "API signer wallet status", body = ApiSignerWalletResponse)
    )
)]
pub async fn get_api_signer_wallet(
    State(state): State<AppState>,
) -> ApiResult<Json<ApiSignerWalletResponse>> {
    let min_open_lamports = std::env::var("CLMM_MIN_OPEN_SOL_LAMPORTS")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(12_000_000);
    let min_swap_lamports = std::env::var("CLMM_MIN_SWAP_SOL_LAMPORTS")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(1_500_000);

    let rpc_url = state.provider.current_endpoint().await;

    let w = load_wallet_from_env()
        .map_err(|e| ApiError::internal(format!("api-signer wallet load: {e}")))?;
    let Some(w) = w else {
        return Ok(Json(ApiSignerWalletResponse {
            configured: false,
            pubkey: None,
            rpc_url,
            lamports: None,
            sol: None,
            min_open_lamports,
            min_swap_lamports,
            note: Some(
                "Set one signer source on API host: KEYPAIR_PATH / SOLANA_KEYPAIR_PATH / WALLET_KEYPAIR_PATH or SOLANA_KEYPAIR / WALLET_KEYPAIR_BASE58."
                    .to_string(),
            ),
        }));
    };

    let pk = w.pubkey();
    match state.provider.get_balance(&pk).await {
        Ok(l) => Ok(Json(ApiSignerWalletResponse {
            configured: true,
            pubkey: Some(pk.to_string()),
            rpc_url,
            lamports: Some(l),
            sol: Some(format!("{:.9}", (l as f64) / 1e9)),
            min_open_lamports,
            min_swap_lamports,
            note: None,
        })),
        Err(e) => Ok(Json(ApiSignerWalletResponse {
            configured: true,
            pubkey: Some(pk.to_string()),
            rpc_url,
            lamports: None,
            sol: None,
            min_open_lamports,
            min_swap_lamports,
            note: Some(format!("Wallet loaded but SOL balance RPC failed: {e}")),
        })),
    }
}
