//! Local wallets (keypairs on API host) + on-chain read-only balances.

use crate::error::{ApiError, ApiResult};
use crate::models::{
    ActiveSignerResponse, ApiSignerWalletResponse, ConvertSolDirection, ConvertSolRequest,
    ConvertSolResponse, CreateWalletRequest, CreateWalletResponse, SetActiveSignerRequest,
    WalletBalancesResponse, WalletEntry, WalletReplicationStatus, WalletTokenBalance,
    WalletReconcileItem, WalletReconcileResponse, WalletTransferLogEntry, WalletTransferRequest,
    WalletTransferResponse, WalletTransfersListResponse, WalletsListResponse,
};
use crate::services::position_executor::load_wallet_from_env;
use crate::state::AppState;
use axum::{Json, extract::Query, extract::State};
use clmm_lp_protocols::orca::executor::WhirlpoolExecutor;
use serde::Deserialize;
use solana_sdk::{
    pubkey::Pubkey, signature::Keypair, signature::read_keypair_file, signer::Signer,
    transaction::Transaction,
};
use solana_system_interface::instruction as system_instruction;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

const SPL_TOKEN_PROGRAM_ID: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const TOKEN_2022_PROGRAM_ID: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";

type WalletReplica = Option<(String, String)>;
type WalletReplicaPair = (WalletReplica, WalletReplica);

#[derive(Debug, Clone)]
struct WalletStores {
    primary: PathBuf,
    secondary: Option<PathBuf>,
}

fn resolve_wallet_stores(state: &AppState) -> WalletStores {
    let primary = state
        .config
        .wallets_dir_primary
        .as_ref()
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            state
                .config
                .wallets_dir
                .as_ref()
                .filter(|s| !s.trim().is_empty())
                .map(PathBuf::from)
        })
        .or_else(|| std::env::var("CLMM_WALLETS_DIR_PRIMARY").ok().map(PathBuf::from))
        .or_else(|| std::env::var("CLMM_WALLETS_DIR").ok().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("wallets"));
    let secondary = state
        .config
        .wallets_dir_secondary
        .as_ref()
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var("CLMM_WALLETS_DIR_SECONDARY").ok().map(PathBuf::from))
        .filter(|p| p != &primary);
    WalletStores { primary, secondary }
}

fn wallet_file_path(dir: &Path, wallet_id: &str) -> PathBuf {
    dir.join(format!("{wallet_id}.json"))
}

fn wallet_fingerprint_bytes(bytes: &[u8]) -> String {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD as B64;
    // Keep dependencies minimal: SHA-256 via solana-sdk hash helper.
    let h = solana_sdk::hash::hash(bytes);
    B64.encode(h.to_bytes())
}

fn scan_wallet_dir(
    dir: &PathBuf,
    out: &mut BTreeMap<String, WalletReplicaPair>,
    is_primary: bool,
) {
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
        let pubkey = match read_keypair_file(&p) {
            Ok(kp) => kp.pubkey().to_string(),
            Err(_) => continue,
        };
        let bytes = match fs::read(&p) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let fp = wallet_fingerprint_bytes(&bytes);
        let ent = out.entry(id).or_insert((None, None));
        if is_primary {
            ent.0 = Some((pubkey, fp));
        } else {
            ent.1 = Some((pubkey, fp));
        }
    }
}

fn create_wallet_entry(
    id: String,
    primary: Option<(String, String)>,
    secondary: Option<(String, String)>,
) -> Option<WalletEntry> {
    let chosen_pubkey = primary
        .as_ref()
        .map(|v| v.0.clone())
        .or_else(|| secondary.as_ref().map(|v| v.0.clone()))?;
    let fingerprint = primary
        .as_ref()
        .map(|v| v.1.clone())
        .or_else(|| secondary.as_ref().map(|v| v.1.clone()));
    let present_in_primary = primary.is_some();
    let present_in_secondary = secondary.is_some();
    let conflict = match (&primary, &secondary) {
        (Some(a), Some(b)) => a.1 != b.1 || a.0 != b.0,
        _ => false,
    };
    let replication_status = if conflict {
        WalletReplicationStatus::Conflict
    } else if present_in_primary && (present_in_secondary || secondary.is_none()) {
        WalletReplicationStatus::Healthy
    } else {
        WalletReplicationStatus::Degraded
    };
    Some(WalletEntry {
        id: id.clone(),
        filename: format!("{id}.json"),
        pubkey: chosen_pubkey,
        present_in_primary,
        present_in_secondary,
        replication_status,
        fingerprint,
    })
}

fn load_wallet_keypair_from_stores(stores: &WalletStores, wallet_id: &str) -> Result<Keypair, ApiError> {
    let p1 = wallet_file_path(&stores.primary, wallet_id);
    if p1.exists() {
        return read_keypair_file(&p1)
            .map_err(|e| ApiError::bad_request(format!("wallet `{wallet_id}` read failed from primary: {e}")));
    }
    if let Some(sec) = &stores.secondary {
        let p2 = wallet_file_path(sec, wallet_id);
        if p2.exists() {
            return read_keypair_file(&p2)
                .map_err(|e| ApiError::bad_request(format!("wallet `{wallet_id}` read failed from secondary: {e}")));
        }
    }
    Err(ApiError::bad_request(format!(
        "wallet `{wallet_id}` not found in configured stores"
    )))
}

fn load_signer_wallet_for_api(
    state: &AppState,
) -> Result<Option<std::sync::Arc<clmm_lp_execution::prelude::Wallet>>, ApiError> {
    if let Ok(guard) = state.active_signer_wallet_id.try_read()
        && let Some(wallet_id) = guard.as_ref()
    {
        let stores = resolve_wallet_stores(state);
        let p1 = wallet_file_path(&stores.primary, wallet_id);
        if p1.exists() {
            return clmm_lp_execution::prelude::Wallet::from_file(
                &p1,
                "api-active-wallet",
            )
            .map(|w| Some(std::sync::Arc::new(w)))
            .map_err(|e| ApiError::internal(format!("active signer load failed: {e}")));
        }
        if let Some(sec) = stores.secondary {
            let p2 = wallet_file_path(&sec, wallet_id);
            if p2.exists() {
                return clmm_lp_execution::prelude::Wallet::from_file(
                    &p2,
                    "api-active-wallet",
                )
                .map(|w| Some(std::sync::Arc::new(w)))
                .map_err(|e| ApiError::internal(format!("active signer load failed: {e}")));
            }
        }
    }
    load_wallet_from_env()
}

fn wallet_entries_from_stores(stores: &WalletStores) -> Vec<WalletEntry> {
    let mut merged: BTreeMap<String, WalletReplicaPair> = BTreeMap::new();
    scan_wallet_dir(&stores.primary, &mut merged, true);
    if let Some(sec) = &stores.secondary {
        scan_wallet_dir(sec, &mut merged, false);
    }
    let mut wallets: Vec<WalletEntry> = merged
        .into_iter()
        .filter_map(|(id, (primary, secondary))| create_wallet_entry(id, primary, secondary))
        .collect();
    wallets.sort_by(|a, b| a.id.cmp(&b.id));
    wallets
}

fn parse_env_allowlist_csv(var: &str) -> HashSet<String> {
    std::env::var(var)
        .ok()
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToString::to_string)
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default()
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
    let stores = resolve_wallet_stores(&state);
    let wallets = wallet_entries_from_stores(&stores);
    let transfer_min_lamports = std::env::var("CLMM_WALLET_TRANSFER_MIN_LAMPORTS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(1_000_000);
    let transfer_max_lamports = std::env::var("CLMM_WALLET_TRANSFER_MAX_LAMPORTS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok());
    Ok(Json(WalletsListResponse {
        wallets_dir_primary: stores.primary.to_string_lossy().to_string(),
        wallets_dir_secondary: stores
            .secondary
            .as_ref()
            .map(|p| p.to_string_lossy().to_string()),
        transfer_min_lamports,
        transfer_max_lamports,
        wallets,
    }))
}

#[utoipa::path(
    post,
    path = "/wallets/reconcile",
    tag = "Wallets",
    responses((status = 200, description = "Wallet stores reconciled", body = WalletReconcileResponse))
)]
pub async fn reconcile_wallet_stores(
    State(state): State<AppState>,
) -> ApiResult<Json<WalletReconcileResponse>> {
    let stores = resolve_wallet_stores(&state);
    let wallets = wallet_entries_from_stores(&stores);
    let mut items = Vec::new();
    let mut repaired = 0usize;
    let mut conflicts = 0usize;

    for w in wallets {
        let primary_path = wallet_file_path(&stores.primary, &w.id);
        let secondary_path = stores
            .secondary
            .as_ref()
            .map(|dir| wallet_file_path(dir, &w.id));
        let mut item = WalletReconcileItem {
            wallet_id: w.id.clone(),
            status: w.replication_status,
            repaired: false,
            note: None,
        };
        match w.replication_status {
            WalletReplicationStatus::Healthy => {}
            WalletReplicationStatus::Conflict => {
                conflicts += 1;
                item.note = Some("conflict detected; manual resolution required".to_string());
            }
            WalletReplicationStatus::Degraded => {
                if let Some(sec_path) = secondary_path {
                    let res = if !w.present_in_primary && w.present_in_secondary {
                        fs::copy(&sec_path, &primary_path)
                    } else if w.present_in_primary && !w.present_in_secondary {
                        fs::copy(&primary_path, &sec_path)
                    } else {
                        Err(std::io::Error::other("unknown degraded state"))
                    };
                    match res {
                        Ok(_) => {
                            item.repaired = true;
                            repaired += 1;
                            item.note = Some("repaired missing replica".to_string());
                        }
                        Err(e) => {
                            item.note = Some(format!("repair failed: {e}"));
                        }
                    }
                } else {
                    item.note = Some("secondary store is not configured".to_string());
                }
            }
        }
        items.push(item);
    }

    Ok(Json(WalletReconcileResponse {
        primary: stores.primary.to_string_lossy().to_string(),
        secondary: stores.secondary.map(|p| p.to_string_lossy().to_string()),
        scanned: items.len(),
        repaired,
        conflicts,
        items,
    }))
}

/// Create a new local wallet keypair JSON file in primary/secondary stores.
#[utoipa::path(
    post,
    path = "/wallets/create",
    tag = "Wallets",
    request_body = CreateWalletRequest,
    responses(
        (status = 200, description = "Wallet created", body = CreateWalletResponse),
        (status = 400, description = "Invalid request"),
        (status = 500, description = "I/O error")
    )
)]
pub async fn create_wallet(
    State(state): State<AppState>,
    Json(req): Json<CreateWalletRequest>,
) -> ApiResult<Json<CreateWalletResponse>> {
    let stores = resolve_wallet_stores(&state);
    let wallet_id = req
        .wallet_id
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("wallet_{}", chrono::Utc::now().format("%Y%m%d_%H%M%S")));
    if !wallet_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(ApiError::bad_request(
            "wallet_id may contain only: a-z, A-Z, 0-9, '_' and '-'",
        ));
    }
    fs::create_dir_all(&stores.primary)
        .map_err(|e| ApiError::internal(format!("create primary dir failed: {e}")))?;
    if let Some(sec) = &stores.secondary {
        fs::create_dir_all(sec)
            .map_err(|e| ApiError::internal(format!("create secondary dir failed: {e}")))?;
    }

    let p1 = wallet_file_path(&stores.primary, &wallet_id);
    let p2 = stores
        .secondary
        .as_ref()
        .map(|dir| wallet_file_path(dir, &wallet_id));
    if !req.force && (p1.exists() || p2.as_ref().map(|p| p.exists()).unwrap_or(false)) {
        return Err(ApiError::bad_request(format!(
            "wallet `{wallet_id}` already exists (use force=true to overwrite)"
        )));
    }
    let keypair = Keypair::new();
    let bytes = serde_json::to_vec(&keypair.to_bytes().to_vec())
        .map_err(|e| ApiError::internal(format!("serialize keypair failed: {e}")))?;

    let primary_written;
    let mut secondary_written = false;
    {
        let mut f = fs::File::create(&p1)
            .map_err(|e| ApiError::internal(format!("create primary wallet file failed: {e}")))?;
        f.write_all(&bytes)
            .map_err(|e| ApiError::internal(format!("write primary wallet file failed: {e}")))?;
        primary_written = true;
    }
    if let Some(path) = p2 {
        match fs::File::create(&path).and_then(|mut f| f.write_all(&bytes)) {
            Ok(_) => secondary_written = true,
            Err(e) => {
                tracing::warn!(error = %e, path = %path.to_string_lossy(), "secondary wallet write failed");
            }
        }
    } else {
        secondary_written = true;
    }
    let wallet = create_wallet_entry(
        wallet_id.clone(),
        Some((keypair.pubkey().to_string(), wallet_fingerprint_bytes(&bytes))),
        if secondary_written {
            Some((keypair.pubkey().to_string(), wallet_fingerprint_bytes(&bytes)))
        } else {
            None
        },
    )
    .ok_or_else(|| ApiError::internal("created wallet entry cannot be built"))?;
    let note = if primary_written && secondary_written {
        None
    } else {
        Some("wallet created, but secondary write failed".to_string())
    };
    Ok(Json(CreateWalletResponse {
        wallet,
        primary_written,
        secondary_written,
        note,
    }))
}

#[utoipa::path(
    get,
    path = "/wallets/active-signer",
    tag = "Wallets",
    responses((status = 200, description = "Active signer", body = ActiveSignerResponse))
)]
pub async fn get_active_signer(
    State(state): State<AppState>,
) -> ApiResult<Json<ActiveSignerResponse>> {
    let active = state.active_signer_wallet_id.read().await.clone();
    if let Some(wallet_id) = active {
        let stores = resolve_wallet_stores(&state);
        let kp = load_wallet_keypair_from_stores(&stores, &wallet_id)?;
        return Ok(Json(ActiveSignerResponse {
            wallet_id: Some(wallet_id),
            pubkey: Some(kp.pubkey().to_string()),
            source: "active_wallet".to_string(),
        }));
    }
    let env_signer = load_wallet_from_env()
        .map_err(|e| ApiError::internal(format!("load env signer failed: {e}")))?;
    Ok(Json(ActiveSignerResponse {
        wallet_id: None,
        pubkey: env_signer.as_ref().map(|w| w.pubkey().to_string()),
        source: "env_fallback".to_string(),
    }))
}

#[utoipa::path(
    post,
    path = "/wallets/active-signer",
    tag = "Wallets",
    request_body = SetActiveSignerRequest,
    responses((status = 200, description = "Active signer updated", body = ActiveSignerResponse))
)]
pub async fn set_active_signer(
    State(state): State<AppState>,
    Json(req): Json<SetActiveSignerRequest>,
) -> ApiResult<Json<ActiveSignerResponse>> {
    let wallet_id = req.wallet_id.trim();
    if wallet_id.is_empty() {
        return Err(ApiError::bad_request("wallet_id is required"));
    }
    let stores = resolve_wallet_stores(&state);
    let kp = load_wallet_keypair_from_stores(&stores, wallet_id)?;
    *state.active_signer_wallet_id.write().await = Some(wallet_id.to_string());
    Ok(Json(ActiveSignerResponse {
        wallet_id: Some(wallet_id.to_string()),
        pubkey: Some(kp.pubkey().to_string()),
        source: "active_wallet".to_string(),
    }))
}

#[utoipa::path(
    post,
    path = "/wallets/transfer",
    tag = "Wallets",
    request_body = WalletTransferRequest,
    responses(
        (status = 200, description = "SOL transfer submitted", body = WalletTransferResponse),
        (status = 400, description = "Invalid request")
    )
)]
pub async fn transfer_sol_between_wallets(
    State(state): State<AppState>,
    Json(req): Json<WalletTransferRequest>,
) -> ApiResult<Json<WalletTransferResponse>> {
    if req.lamports == 0 {
        return Err(ApiError::bad_request("lamports must be > 0"));
    }
    let min_lamports = std::env::var("CLMM_WALLET_TRANSFER_MIN_LAMPORTS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(1_000_000);
    if req.lamports < min_lamports {
        return Err(ApiError::bad_request(format!(
            "lamports must be >= {min_lamports} (dust guard)"
        )));
    }
    let max_lamports = std::env::var("CLMM_WALLET_TRANSFER_MAX_LAMPORTS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok());
    if let Some(max_lamports) = max_lamports
        && req.lamports > max_lamports
    {
        return Err(ApiError::bad_request(format!(
            "lamports exceeds configured max ({max_lamports})"
        )));
    }
    let to_pubkey =
        Pubkey::from_str(req.to_pubkey.trim()).map_err(|_| ApiError::bad_request("invalid to_pubkey"))?;
    let stores = resolve_wallet_stores(&state);
    let source_wallet_id = req.from_wallet_id.trim().to_string();
    let allowed_sources = parse_env_allowlist_csv("CLMM_WALLET_TRANSFER_SOURCE_ALLOWLIST");
    if !allowed_sources.is_empty() && !allowed_sources.contains(&source_wallet_id) {
        return Err(ApiError::bad_request(
            "source wallet is not in CLMM_WALLET_TRANSFER_SOURCE_ALLOWLIST".to_string(),
        ));
    }
    let mut allowed_recipients: HashSet<String> = parse_env_allowlist_csv("CLMM_WALLET_TRANSFER_ALLOWLIST");
    for w in wallet_entries_from_stores(&stores) {
        allowed_recipients.insert(w.pubkey);
    }
    if !allowed_recipients.is_empty() && !allowed_recipients.contains(&to_pubkey.to_string()) {
        return Err(ApiError::bad_request(
            "recipient is not in transfer allowlist (wallet stores + CLMM_WALLET_TRANSFER_ALLOWLIST)"
                .to_string(),
        ));
    }
    let from_kp = load_wallet_keypair_from_stores(&stores, &source_wallet_id)?;
    let from_pubkey = from_kp.pubkey();
    let bal = state
        .provider
        .get_balance(&from_pubkey)
        .await
        .map_err(|e| ApiError::internal(format!("read sender balance failed: {e}")))?;
    let reserve = 10_000u64;
    if bal < req.lamports.saturating_add(reserve) {
        return Err(ApiError::bad_request(format!(
            "insufficient SOL: have {bal} lamports, need at least {} + fee reserve",
            req.lamports
        )));
    }
    let ix = system_instruction::transfer(&from_pubkey, &to_pubkey, req.lamports);
    let recent = state
        .provider
        .get_latest_blockhash()
        .await
        .map_err(|e| ApiError::internal(format!("latest blockhash failed: {e}")))?;
    let tx =
        Transaction::new_signed_with_payer(&[ix], Some(&from_pubkey), &[&from_kp], recent);
    let sig = state
        .provider
        .send_and_confirm_transaction(&tx)
        .await
        .map_err(|e| ApiError::bad_request(format!("transfer failed: {e}")))?;

    // Best-effort local log (append-only JSONL) for ops/audit.
    {
        let log_dir = PathBuf::from("data").join("wallet-transfers");
        let log_path = log_dir.join("sol_transfers.jsonl");
        if let Err(e) = fs::create_dir_all(&log_dir) {
            tracing::warn!(error = %e, path = %log_dir.to_string_lossy(), "wallet transfer log: create_dir_all failed");
        } else {
            let entry = WalletTransferLogEntry {
                ts_utc: chrono::Utc::now().to_rfc3339(),
                from_wallet_id: source_wallet_id.clone(),
                from_pubkey: from_pubkey.to_string(),
                to_pubkey: to_pubkey.to_string(),
                lamports: req.lamports,
                signature: sig.to_string(),
                rpc_url: Some(state.provider.current_endpoint().await),
            };
            match serde_json::to_string(&entry) {
                Ok(line) => {
                    if let Err(e) = fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&log_path)
                        .and_then(|mut f| writeln!(f, "{line}"))
                    {
                        tracing::warn!(error = %e, path = %log_path.to_string_lossy(), "wallet transfer log: append failed");
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "wallet transfer log: serialize failed");
                }
            }
        }
    }

    Ok(Json(WalletTransferResponse {
        from_wallet_id: source_wallet_id,
        from_pubkey: from_pubkey.to_string(),
        to_pubkey: to_pubkey.to_string(),
        lamports: req.lamports,
        signature: sig.to_string(),
    }))
}

#[derive(Debug, Deserialize)]
pub struct WalletTransfersQuery {
    #[serde(default)]
    pub limit: Option<usize>,
}

/// `GET /wallets/transfers` — recent local transfer log (best-effort).
#[utoipa::path(
    get,
    path = "/wallets/transfers",
    tag = "Wallets",
    params(
        ("limit" = Option<usize>, Query, description = "Max number of transfers to return (default 50, max 200)")
    ),
    responses((status = 200, description = "Recent transfers (local log)", body = WalletTransfersListResponse))
)]
pub async fn list_wallet_transfers(
    Query(q): Query<WalletTransfersQuery>,
) -> ApiResult<Json<WalletTransfersListResponse>> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let log_path = PathBuf::from("data")
        .join("wallet-transfers")
        .join("sol_transfers.jsonl");
    let text = match fs::read_to_string(&log_path) {
        Ok(t) => t,
        Err(_) => {
            return Ok(Json(WalletTransfersListResponse { transfers: vec![] }));
        }
    };
    let mut rows: Vec<WalletTransferLogEntry> = text
        .lines()
        .rev()
        .take(limit)
        .filter_map(|line| serde_json::from_str::<WalletTransferLogEntry>(line).ok())
        .collect();
    // We iterated newest->oldest; keep that order (newest first) in response.
    if rows.len() > limit {
        rows.truncate(limit);
    }
    Ok(Json(WalletTransfersListResponse { transfers: rows }))
}

#[derive(Debug, Deserialize)]
pub struct WalletBalancesQuery {
    pub owner: String,
}

fn append_tokens_from_keyed_accounts(
    accounts: &[solana_client::rpc_response::RpcKeyedAccount],
    out: &mut Vec<WalletTokenBalance>,
) {
    // We expect `jsonParsed` encoding (Solana client does this internally for token accounts).
    // Instead of relying on a nested RPC JSON shape, we extract fields from the keyed accounts.
    for ka in accounts {
        // `UiAccountData` is an untagged enum; the Json variant serializes to:
        // { program: "...", parsed: { ... }, space: ... }
        let data_v = match serde_json::to_value(&ka.account.data) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let parsed = data_v.get("parsed").unwrap_or(&serde_json::Value::Null);
        let info = parsed.get("info").unwrap_or(&serde_json::Value::Null);
        let mint = info.get("mint").and_then(|v| v.as_str()).unwrap_or("").to_string();
        if mint.is_empty() {
            continue;
        }
        let token_amount = info.get("tokenAmount").unwrap_or(&serde_json::Value::Null);
        let ui_amount = token_amount
            .get("uiAmountString")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| token_amount.get("uiAmount").and_then(|v| v.as_f64()).map(|x| x.to_string()))
            .unwrap_or_else(|| "0".to_string());
        out.push(WalletTokenBalance { mint, ui_amount });
    }
}

fn merge_wallet_token_rows(rows: Vec<WalletTokenBalance>) -> Vec<WalletTokenBalance> {
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<String, f64> = BTreeMap::new();
    for row in rows {
        let v = row.ui_amount.parse::<f64>().unwrap_or(0.0);
        let e = acc.entry(row.mint).or_insert(0.0);
        *e += v;
    }
    acc.into_iter()
        .map(|(mint, amount)| WalletTokenBalance {
            mint,
            ui_amount: amount.to_string(),
        })
        .collect()
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

    let rpc_url_used = state.provider.current_endpoint().await;
    let token_timeout_ms = std::env::var("CLMM_WALLET_BALANCES_TOKEN_TIMEOUT_MS")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(4500);
    let token_deadline = Duration::from_millis(token_timeout_ms);
    let spl_pid = Pubkey::from_str(SPL_TOKEN_PROGRAM_ID).expect("SPL token program id");
    let t22_pid = Pubkey::from_str(TOKEN_2022_PROGRAM_ID).expect("token-2022 program id");

    let bal_fut = state.provider.get_balance(&owner_pk);
    let legacy_fut = state
        .provider
        .get_token_accounts_by_owner_json_parsed(&owner_pk, &spl_pid);
    let tok2022_fut = state
        .provider
        .get_token_accounts_by_owner_json_parsed(&owner_pk, &t22_pid);

    // Run the heavy token reads concurrently, but keep a hard deadline so the endpoint stays responsive
    // even when public RPCs 429/403/timeout.
    let (bal_res, legacy_res, tok2022_res) = tokio::join!(
        bal_fut,
        tokio::time::timeout(token_deadline, legacy_fut),
        tokio::time::timeout(token_deadline, tok2022_fut)
    );

    let lamports = bal_res.map_err(|e| ApiError::internal(format!("getBalance failed: {e}")))?;
    let sol = format!("{:.9}", (lamports as f64) / 1e9);

    let tok_legacy: anyhow::Result<Vec<solana_client::rpc_response::RpcKeyedAccount>> = match legacy_res {
        Ok(r) => r,
        Err(_) => Err(anyhow::anyhow!(
            "timeout after {token_timeout_ms}ms (token accounts legacy)"
        )),
    };
    let tok_2022: anyhow::Result<Vec<solana_client::rpc_response::RpcKeyedAccount>> = match tok2022_res {
        Ok(r) => r,
        Err(_) => Err(anyhow::anyhow!(
            "timeout after {token_timeout_ms}ms (token accounts token-2022)"
        )),
    };

    // If one token RPC path is slow/unavailable, keep partial token list from the other one.
    // If both fail, we still return SOL with tokens empty (existing behavior).
    let legacy_ok = tok_legacy.is_ok();
    let token_2022_ok = tok_2022.is_ok();
    let legacy_err = tok_legacy
        .as_ref()
        .err()
        .map(|e| format!("getTokenAccountsByOwner: {e}"));
    let token_2022_err = tok_2022
        .as_ref()
        .err()
        .map(|e| format!("getTokenAccountsByOwner: {e}"));

    let mut token_rows = Vec::new();
    if let Ok(v) = tok_legacy {
        append_tokens_from_keyed_accounts(&v, &mut token_rows);
    }
    if let Ok(v) = tok_2022 {
        append_tokens_from_keyed_accounts(&v, &mut token_rows);
    }
    let token_accounts_total = token_rows.len() as u64;
    let tokens = merge_wallet_token_rows(token_rows);

    Ok(Json(WalletBalancesResponse {
        owner: owner_pk.to_string(),
        rpc_url: rpc_url_used,
        lamports,
        sol,
        tokens,
        token_accounts_total: Some(token_accounts_total),
        token_legacy_ok: Some(legacy_ok),
        token_2022_ok: Some(token_2022_ok),
        token_legacy_error: legacy_err,
        token_2022_error: token_2022_err,
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

    let w = load_signer_wallet_for_api(&state)
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

/// Convert native SOL <-> WSOL in the API signer wallet (1:1, no pool swap).
#[utoipa::path(
    post,
    path = "/wallets/convert-sol",
    tag = "Wallets",
    request_body = ConvertSolRequest,
    responses(
        (status = 200, description = "SOL conversion submitted", body = ConvertSolResponse),
        (status = 400, description = "Invalid request / insufficient source balance"),
        (status = 500, description = "RPC or execution error")
    )
)]
pub async fn convert_sol(
    State(state): State<AppState>,
    Json(req): Json<ConvertSolRequest>,
) -> ApiResult<Json<ConvertSolResponse>> {
    if req.amount_raw == 0 {
        return Err(ApiError::bad_request("amount_raw must be > 0"));
    }
    let signer = load_signer_wallet_for_api(&state)
        .map_err(|e| ApiError::internal(format!("api-signer wallet load: {e}")))?
        .ok_or_else(|| {
            ApiError::bad_request(
                "API signer wallet is not configured (set KEYPAIR_PATH / SOLANA_KEYPAIR_PATH / WALLET_KEYPAIR_PATH)",
            )
        })?;
    let owner = signer.pubkey();
    let exec = WhirlpoolExecutor::new(state.provider.clone());
    let signature = match req.direction {
        ConvertSolDirection::NativeToWsol => {
            let native = state
                .provider
                .get_balance(&owner)
                .await
                .map_err(|e| ApiError::internal(format!("read native SOL balance: {e}")))?;
            if native < req.amount_raw {
                return Err(ApiError::bad_request(format!(
                    "insufficient native SOL balance (have {native} raw, need {} raw)",
                    req.amount_raw
                )));
            }
            exec.submit_wsol_wrap_with_signature_if_needed(req.amount_raw, signer.keypair())
                .await
                .map_err(|e| ApiError::bad_request(format!("native_to_wsol failed: {e}")))?
                .map(|s| s.to_string())
        }
        ConvertSolDirection::WsolToNative => {
            let wsol = exec
                .read_wsol_balance_raw(&owner)
                .await
                .map_err(|e| ApiError::internal(format!("read WSOL balance: {e}")))?;
            if wsol < req.amount_raw {
                return Err(ApiError::bad_request(format!(
                    "insufficient WSOL balance (have {wsol} raw, need {} raw)",
                    req.amount_raw
                )));
            }
            let sig = exec
                .submit_wsol_unwrap_with_signature(req.amount_raw, signer.keypair())
                .await
                .map_err(|e| ApiError::bad_request(format!("wsol_to_native failed: {e}")))?;
            Some(sig.to_string())
        }
    };

    Ok(Json(ConvertSolResponse {
        message: "SOL conversion submitted".to_string(),
        signature,
        direction: req.direction,
        amount_raw: req.amount_raw,
        owner_pubkey: owner.to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_wallet_token_rows_sums_same_mint() {
        let merged = merge_wallet_token_rows(vec![
            WalletTokenBalance {
                mint: "So11111111111111111111111111111111111111112".to_string(),
                ui_amount: "1.25".to_string(),
            },
            WalletTokenBalance {
                mint: "So11111111111111111111111111111111111111112".to_string(),
                ui_amount: "0.75".to_string(),
            },
            WalletTokenBalance {
                mint: "Es9vMFrzaCERmJfrF4H2XfNwS7TfGsDz3jAC5vVsQt1z".to_string(),
                ui_amount: "2".to_string(),
            },
        ]);

        assert_eq!(merged.len(), 2);
        let sol_row = merged
            .iter()
            .find(|r| r.mint == "So11111111111111111111111111111111111111112")
            .expect("sol row");
        assert_eq!(sol_row.ui_amount.parse::<f64>().unwrap_or(0.0), 2.0);
    }
}
