//! Local wallets (keypairs on API host) + on-chain read-only balances.

use crate::error::{ApiError, ApiResult};
use crate::models::{
    ActiveSignerResponse, ApiSignerWalletResponse, ConvertSolDirection, ConvertSolRequest,
    ConvertSolResponse, CreateWalletRequest, CreateWalletResponse, SetActiveSignerRequest,
    WalletBalanceConfidence, WalletBalancesResponse, WalletConvertOpResponse, WalletEffectiveBalancesResponse,
    WalletEntry, WalletOpsStatsResponse, WalletReconciliationStatus, WalletReplicationStatus, WalletTokenBalance,
    WalletReconcileItem, WalletReconcileResponse, WalletTransferLogEntry, WalletTransferRequest, WalletWsStatusResponse,
    WalletTransferResponse, WalletTransfersListResponse, WalletsListResponse,
};
use crate::services::position_executor::load_wallet_from_env;
use crate::state::AppState;
use axum::{Json, extract::Query, extract::State};
use clmm_lp_protocols::orca::executor::WhirlpoolExecutor;
use serde::{Deserialize, Serialize};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_client::pubsub_client::PubsubClient;
use solana_client::rpc_config::{RpcProgramAccountsConfig, RpcTransactionLogsConfig};
use solana_client::rpc_filter::{Memcmp, RpcFilterType};
use solana_client::rpc_response::Response as RpcResponseEnvelope;
use solana_client::rpc_response::RpcLogsResponse;
use solana_client::rpc_request::TokenAccountsFilter;
use solana_client::rpc_response::RpcKeyedAccount;
use solana_sdk::{
    pubkey::Pubkey, signature::Keypair, signature::read_keypair_file, signer::Signer,
    transaction::Transaction,
};
use solana_system_interface::instruction as system_instruction;
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Mutex as StdMutex, OnceLock};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tokio::task::JoinSet;
use tokio::time::sleep;
use uuid::Uuid;

const SPL_TOKEN_PROGRAM_ID: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const TOKEN_2022_PROGRAM_ID: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";
const WSOL_MINT: &str = "So11111111111111111111111111111111111111112";
const HEDGE_LATENCY_WINDOW: usize = 64;

type WalletReplica = Option<(String, String)>;
type WalletReplicaPair = (WalletReplica, WalletReplica);

#[derive(Default)]
struct HedgeLatencyRegistry {
    by_key: BTreeMap<String, VecDeque<u64>>,
}

#[derive(Default)]
struct EndpointPenaltyRegistry {
    penalized_until: BTreeMap<String, Instant>,
}

#[derive(Debug, Clone)]
struct HedgeConfig {
    enabled: bool,
    max_attempts: usize,
    delay_ms: u64,
    budget_pct: f64,
}

#[derive(Debug)]
struct HedgeBudgetState {
    tokens: f64,
    last_refill: Instant,
    estimated_rps: f64,
}

impl Default for HedgeBudgetState {
    fn default() -> Self {
        Self {
            tokens: 20.0,
            last_refill: Instant::now(),
            estimated_rps: 20.0,
        }
    }
}

static HEDGE_LATENCIES: OnceLock<StdMutex<HedgeLatencyRegistry>> = OnceLock::new();
static HEDGE_BUDGET: OnceLock<StdMutex<HedgeBudgetState>> = OnceLock::new();
static TOKEN_ENDPOINT_PENALTIES: OnceLock<StdMutex<EndpointPenaltyRegistry>> = OnceLock::new();

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

fn hedge_config_from_env() -> HedgeConfig {
    let enabled = std::env::var("CLMM_WALLET_HEDGE_ENABLE")
        .ok()
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(true);
    let max_attempts = std::env::var("CLMM_WALLET_HEDGE_MAX_ATTEMPTS")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(2)
        .clamp(1, 4);
    let delay_ms = std::env::var("CLMM_WALLET_HEDGE_DELAY_MS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(120);
    let budget_pct = std::env::var("CLMM_WALLET_HEDGE_BUDGET_PCT")
        .ok()
        .and_then(|v| v.trim().parse::<f64>().ok())
        .unwrap_or(10.0)
        .clamp(1.0, 100.0);
    HedgeConfig {
        enabled,
        max_attempts,
        delay_ms,
        budget_pct,
    }
}

fn token_endpoint_penalty_secs() -> u64 {
    std::env::var("CLMM_WALLET_TOKEN_ENDPOINT_PENALTY_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(90)
}

fn should_penalize_token_endpoint(error_text: &str) -> bool {
    let s = error_text.to_ascii_lowercase();
    s.contains("403")
        || s.contains("429")
        || s.contains("timeout")
        || s.contains("forbidden")
        || s.contains("too many requests")
}

fn penalize_token_endpoint(endpoint: &str, error_text: &str) {
    if !should_penalize_token_endpoint(error_text) {
        return;
    }
    let lock = TOKEN_ENDPOINT_PENALTIES.get_or_init(|| StdMutex::new(EndpointPenaltyRegistry::default()));
    if let Ok(mut guard) = lock.lock() {
        let ttl = Duration::from_secs(token_endpoint_penalty_secs());
        guard
            .penalized_until
            .insert(endpoint.to_string(), Instant::now() + ttl);
    }
}

fn filter_penalized_token_endpoints(endpoints: &[String]) -> Vec<String> {
    let lock = TOKEN_ENDPOINT_PENALTIES.get_or_init(|| StdMutex::new(EndpointPenaltyRegistry::default()));
    let mut out = endpoints.to_vec();
    if let Ok(mut guard) = lock.lock() {
        let now = Instant::now();
        guard.penalized_until.retain(|_, until| *until > now);
        let healthy = out
            .iter()
            .filter(|ep| guard.penalized_until.get(*ep).is_none())
            .cloned()
            .collect::<Vec<_>>();
        if !healthy.is_empty() {
            out = healthy;
        }
    }
    out
}

fn record_hedge_latency_ms(key: &str, latency_ms: u64) {
    let lock = HEDGE_LATENCIES.get_or_init(|| StdMutex::new(HedgeLatencyRegistry::default()));
    if let Ok(mut guard) = lock.lock() {
        let entry = guard.by_key.entry(key.to_string()).or_default();
        entry.push_back(latency_ms);
        while entry.len() > HEDGE_LATENCY_WINDOW {
            let _ = entry.pop_front();
        }
    }
}

fn hedge_delay_for_key(key: &str, fallback_delay_ms: u64) -> u64 {
    let lock = HEDGE_LATENCIES.get_or_init(|| StdMutex::new(HedgeLatencyRegistry::default()));
    if let Ok(guard) = lock.lock()
        && let Some(values) = guard.by_key.get(key)
        && !values.is_empty()
    {
        let mut sorted = values.iter().copied().collect::<Vec<_>>();
        sorted.sort_unstable();
        let idx = ((sorted.len() as f64) * 0.95).floor() as usize;
        return sorted[idx.min(sorted.len().saturating_sub(1))].max(25);
    }
    fallback_delay_ms
}

fn try_consume_hedge_budget(cfg: &HedgeConfig) -> bool {
    let lock = HEDGE_BUDGET.get_or_init(|| StdMutex::new(HedgeBudgetState::default()));
    let Ok(mut guard) = lock.lock() else {
        return false;
    };
    let now = Instant::now();
    let dt = now.duration_since(guard.last_refill).as_secs_f64();
    let refill_rate = (guard.estimated_rps * cfg.budget_pct / 100.0).max(0.2);
    let cap = (guard.estimated_rps * 3.0).max(8.0);
    guard.tokens = (guard.tokens + dt * refill_rate).min(cap);
    guard.last_refill = now;
    if guard.tokens >= 1.0 {
        guard.tokens -= 1.0;
        true
    } else {
        false
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WalletConvertOpRow {
    op_id: String,
    owner_pubkey: String,
    direction: ConvertSolDirection,
    amount_raw: u64,
    reconciliation_status: WalletReconciliationStatus,
    reason_code: Option<String>,
    attempts: u32,
    created_at_utc: String,
    updated_at_utc: String,
    last_verified_at_utc: Option<String>,
    pre_native_lamports: u64,
    pre_wsol_raw: u64,
    post_native_lamports: Option<u64>,
    post_wsol_raw: Option<u64>,
    wrap_signature: Option<String>,
    unwrap_signature: Option<String>,
    rewrap_signature: Option<String>,
    tx_signature: Option<String>,
    last_error: Option<String>,
}

fn wallet_ops_store_path() -> PathBuf {
    std::env::var("CLMM_WALLET_OPS_STORE_PATH")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("data/agent/wallet_convert_ops.jsonl"))
}

fn read_wallet_convert_ops(path: &Path) -> anyhow::Result<Vec<WalletConvertOpRow>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(path)?;
    let mut out = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(row) = serde_json::from_str::<WalletConvertOpRow>(trimmed) {
            out.push(row);
        }
    }
    Ok(out)
}

fn write_wallet_convert_ops(path: &Path, rows: &[WalletConvertOpRow]) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut buf = String::new();
    for row in rows {
        buf.push_str(&serde_json::to_string(row)?);
        buf.push('\n');
    }
    fs::write(path, buf)?;
    Ok(())
}

pub fn wallet_reconcile_interval_secs() -> u64 {
    std::env::var("CLMM_WALLET_RECONCILE_INTERVAL_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(20)
}

fn wallet_reconcile_timeout_secs() -> u64 {
    std::env::var("CLMM_WALLET_RECONCILE_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(180)
}

fn wallet_reconcile_wsol_tolerance_raw() -> u64 {
    std::env::var("CLMM_WALLET_RECONCILE_WSOL_TOLERANCE_RAW")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(5_000)
}

fn now_utc_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn is_reconciled(row: &WalletConvertOpRow, current_wsol_raw: u64, tolerance_raw: u64) -> bool {
    match row.direction {
        ConvertSolDirection::NativeToWsol => current_wsol_raw.saturating_add(tolerance_raw)
            >= row.pre_wsol_raw.saturating_add(row.amount_raw),
        ConvertSolDirection::WsolToNative => {
            let expected_max = row
                .pre_wsol_raw
                .saturating_sub(row.amount_raw)
                .saturating_add(tolerance_raw);
            current_wsol_raw <= expected_max
        }
    }
}

fn to_wallet_op_response(row: &WalletConvertOpRow) -> WalletConvertOpResponse {
    WalletConvertOpResponse {
        op_id: row.op_id.clone(),
        owner_pubkey: row.owner_pubkey.clone(),
        direction: row.direction.clone(),
        amount_raw: row.amount_raw,
        reconciliation_status: row.reconciliation_status.clone(),
        reason_code: row.reason_code.clone(),
        attempts: row.attempts,
        created_at_utc: row.created_at_utc.clone(),
        updated_at_utc: row.updated_at_utc.clone(),
        last_verified_at_utc: row.last_verified_at_utc.clone(),
        last_error: row.last_error.clone(),
        post_native_lamports: row.post_native_lamports,
        post_wsol_raw: row.post_wsol_raw,
    }
}

pub async fn reconcile_wallet_convert_ops_tick(state: &AppState) -> anyhow::Result<usize> {
    let _guard = state.wallet_ops_lock.lock().await;
    let path = wallet_ops_store_path();
    let mut rows = read_wallet_convert_ops(&path)?;
    if rows.is_empty() {
        return Ok(0);
    }
    let timeout_secs = wallet_reconcile_timeout_secs();
    let tolerance_raw = wallet_reconcile_wsol_tolerance_raw();
    let exec = WhirlpoolExecutor::new(state.provider.clone());
    let now = chrono::Utc::now();
    let mut touched = 0usize;
    for row in &mut rows {
        if !matches!(
            row.reconciliation_status,
            WalletReconciliationStatus::ConfirmedUnreconciled
        ) {
            continue;
        }
        row.attempts = row.attempts.saturating_add(1);
        let owner = match Pubkey::from_str(&row.owner_pubkey) {
            Ok(v) => v,
            Err(e) => {
                row.reconciliation_status = WalletReconciliationStatus::Mismatch;
                row.reason_code = Some("invalid_owner_pubkey".to_string());
                row.last_error = Some(format!("invalid owner pubkey: {e}"));
                row.updated_at_utc = now_utc_iso();
                touched += 1;
                continue;
            }
        };
        let elapsed_too_long = chrono::DateTime::parse_from_rfc3339(&row.created_at_utc)
            .ok()
            .map(|dt| now.signed_duration_since(dt.with_timezone(&chrono::Utc)).num_seconds())
            .unwrap_or_default()
            >= timeout_secs as i64;
        let native = match state.provider.get_balance(&owner).await {
            Ok(v) => v,
            Err(e) => {
                row.reason_code = Some("unverified_native_read".to_string());
                row.last_error = Some(format!("unverified read native balance: {e}"));
                if elapsed_too_long {
                    row.reconciliation_status = WalletReconciliationStatus::Mismatch;
                    row.reason_code = Some("timeout_unverified_native_read".to_string());
                }
                row.updated_at_utc = now_utc_iso();
                touched += 1;
                continue;
            }
        };
        let wsol = match exec.read_wsol_balance_raw(&owner).await {
            Ok(v) => v,
            Err(e) => {
                row.reason_code = Some("unverified_wsol_read".to_string());
                row.last_error = Some(format!("unverified read WSOL balance: {e}"));
                if elapsed_too_long {
                    row.reconciliation_status = WalletReconciliationStatus::Mismatch;
                    row.reason_code = Some("timeout_unverified_wsol_read".to_string());
                }
                row.updated_at_utc = now_utc_iso();
                touched += 1;
                continue;
            }
        };
        row.post_native_lamports = Some(native);
        row.post_wsol_raw = Some(wsol);
        row.updated_at_utc = now_utc_iso();
        row.last_verified_at_utc = Some(now_utc_iso());
        row.last_error = None;
        if is_reconciled(row, wsol, tolerance_raw) {
            row.reconciliation_status = WalletReconciliationStatus::Reconciled;
            row.reason_code = Some("delta_matched".to_string());
        } else if elapsed_too_long {
            row.reconciliation_status = WalletReconciliationStatus::Mismatch;
            row.reason_code = Some("timeout_delta_not_matched".to_string());
            row.last_error = Some("expected WSOL delta not observed before timeout".to_string());
        } else {
            row.reason_code = Some("awaiting_delta_match".to_string());
        }
        touched += 1;
    }
    if touched > 0 {
        write_wallet_convert_ops(&path, &rows)?;
    }
    Ok(touched)
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

fn pick_fanout_endpoints(endpoints: &[String], fanout: usize) -> Vec<String> {
    let take_n = fanout.max(1).min(endpoints.len().max(1));
    endpoints.iter().take(take_n).cloned().collect()
}

struct FanoutOutcome<T> {
    result: anyhow::Result<T>,
    attempts: Vec<String>,
    budget_limited: bool,
}

fn format_token_accounts_error(e: &anyhow::Error, attempts: &[String]) -> String {
    format_attempts_error("getTokenAccountsByOwner", e, attempts)
}

fn format_attempts_error(prefix: &str, e: &anyhow::Error, attempts: &[String]) -> String {
    if attempts.is_empty() {
        format!("{prefix}: {e}")
    } else {
        format!("{prefix}: {e}; attempts: {}", attempts.join(" | "))
    }
}

async fn get_token_accounts_by_owner_fanout(
    owner: Pubkey,
    program_id: Pubkey,
    endpoints: Vec<String>,
    fanout: usize,
    timeout: Duration,
) -> FanoutOutcome<Vec<RpcKeyedAccount>> {
    let mut errors: Vec<String> = Vec::new();
    let candidate_endpoints = filter_penalized_token_endpoints(&endpoints);
    let selected = pick_fanout_endpoints(&candidate_endpoints, fanout);
    if selected.is_empty() {
        return FanoutOutcome {
            result: Err(anyhow::anyhow!("no RPC endpoints configured")),
            attempts: errors,
            budget_limited: false,
        };
    }
    let cfg = hedge_config_from_env();
    let mut budget_limited = false;
    let attempts_n = cfg.max_attempts.min(selected.len()).max(1);
    let request_key = format!("token_accounts:{program_id}");
    let delay_ms = hedge_delay_for_key(&request_key, cfg.delay_ms).min(timeout.as_millis() as u64);
    let mut set = JoinSet::new();
    let first_endpoint = selected[0].clone();
    {
        let owner_pk = owner;
        let pid = program_id;
        set.spawn(async move {
            let started = Instant::now();
            let client = RpcClient::new_with_timeout(first_endpoint.clone(), timeout);
            let res = tokio::time::timeout(
                timeout,
                client.get_token_accounts_by_owner(&owner_pk, TokenAccountsFilter::ProgramId(pid)),
            )
            .await;
            let elapsed = started.elapsed().as_millis() as u64;
            match res {
                Ok(Ok(v)) => (first_endpoint, Ok(v), elapsed),
                Ok(Err(e)) => (first_endpoint, Err(format!("{e}")), elapsed),
                Err(_) => (
                    first_endpoint,
                    Err(format!("timeout after {}ms", timeout.as_millis())),
                    elapsed,
                ),
            }
        });
    }
    let mut pending_spawn_idx = 1usize;
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok((_endpoint, Ok(v), elapsed_ms)) => {
                record_hedge_latency_ms(&request_key, elapsed_ms);
                set.abort_all();
                return FanoutOutcome {
                    result: Ok(v),
                    attempts: errors,
                    budget_limited,
                };
            }
            Ok((endpoint, Err(e), elapsed_ms)) => {
                penalize_token_endpoint(&endpoint, &e);
                errors.push(format!("{endpoint}: {e}"));
                record_hedge_latency_ms(&request_key, elapsed_ms);
                if cfg.enabled
                    && pending_spawn_idx < attempts_n
                    && delay_ms > 0
                    && try_consume_hedge_budget(&cfg)
                {
                    let endpoint = selected[pending_spawn_idx].clone();
                    pending_spawn_idx += 1;
                    let owner_pk = owner;
                    let pid = program_id;
                    set.spawn(async move {
                        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                        let started = Instant::now();
                        let client = RpcClient::new_with_timeout(endpoint.clone(), timeout);
                        let res = tokio::time::timeout(
                            timeout,
                            client.get_token_accounts_by_owner(
                                &owner_pk,
                                TokenAccountsFilter::ProgramId(pid),
                            ),
                        )
                        .await;
                        let elapsed = started.elapsed().as_millis() as u64;
                        match res {
                            Ok(Ok(v)) => (endpoint, Ok(v), elapsed),
                            Ok(Err(e)) => (endpoint, Err(format!("{e}")), elapsed),
                            Err(_) => (
                                endpoint,
                                Err(format!("timeout after {}ms", timeout.as_millis())),
                                elapsed,
                            ),
                        }
                    });
                } else if cfg.enabled && pending_spawn_idx < attempts_n {
                    budget_limited = true;
                }
            }
            Err(e) => errors.push(format!("join error: {e}")),
        }
    }
    FanoutOutcome {
        result: Err(anyhow::anyhow!(
            "all fanout RPC attempts failed (program {program_id})"
        )),
        attempts: errors,
        budget_limited,
    }
}

async fn get_native_balance_hedged(
    owner: Pubkey,
    endpoints: Vec<String>,
    timeout: Duration,
) -> FanoutOutcome<u64> {
    let mut errors: Vec<String> = Vec::new();
    if endpoints.is_empty() {
        return FanoutOutcome {
            result: Err(anyhow::anyhow!("no RPC endpoints configured")),
            attempts: errors,
            budget_limited: false,
        };
    }
    let cfg = hedge_config_from_env();
    let attempts_n = cfg.max_attempts.min(endpoints.len()).max(1);
    let request_key = "native_balance".to_string();
    let delay_ms = hedge_delay_for_key(&request_key, cfg.delay_ms).min(timeout.as_millis() as u64);
    let mut budget_limited = false;
    let mut set = JoinSet::new();
    let first_endpoint = endpoints[0].clone();
    {
        let owner_pk = owner;
        set.spawn(async move {
            let started = Instant::now();
            let client = RpcClient::new_with_timeout(first_endpoint.clone(), timeout);
            let res = tokio::time::timeout(timeout, client.get_balance(&owner_pk)).await;
            let elapsed = started.elapsed().as_millis() as u64;
            match res {
                Ok(Ok(v)) => (first_endpoint, Ok(v), elapsed),
                Ok(Err(e)) => (first_endpoint, Err(format!("{e}")), elapsed),
                Err(_) => (
                    first_endpoint,
                    Err(format!("timeout after {}ms", timeout.as_millis())),
                    elapsed,
                ),
            }
        });
    }
    let mut pending_spawn_idx = 1usize;
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok((_endpoint, Ok(v), elapsed_ms)) => {
                record_hedge_latency_ms(&request_key, elapsed_ms);
                set.abort_all();
                return FanoutOutcome {
                    result: Ok(v),
                    attempts: errors,
                    budget_limited,
                };
            }
            Ok((endpoint, Err(e), elapsed_ms)) => {
                errors.push(format!("{endpoint}: {e}"));
                record_hedge_latency_ms(&request_key, elapsed_ms);
                if cfg.enabled
                    && pending_spawn_idx < attempts_n
                    && delay_ms > 0
                    && try_consume_hedge_budget(&cfg)
                {
                    let endpoint = endpoints[pending_spawn_idx].clone();
                    pending_spawn_idx += 1;
                    let owner_pk = owner;
                    set.spawn(async move {
                        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                        let started = Instant::now();
                        let client = RpcClient::new_with_timeout(endpoint.clone(), timeout);
                        let res = tokio::time::timeout(timeout, client.get_balance(&owner_pk)).await;
                        let elapsed = started.elapsed().as_millis() as u64;
                        match res {
                            Ok(Ok(v)) => (endpoint, Ok(v), elapsed),
                            Ok(Err(e)) => (endpoint, Err(format!("{e}")), elapsed),
                            Err(_) => (
                                endpoint,
                                Err(format!("timeout after {}ms", timeout.as_millis())),
                                elapsed,
                            ),
                        }
                    });
                } else if cfg.enabled && pending_spawn_idx < attempts_n {
                    budget_limited = true;
                }
            }
            Err(e) => errors.push(format!("join error: {e}")),
        }
    }
    FanoutOutcome {
        result: Err(anyhow::anyhow!("all hedged getBalance attempts failed")),
        attempts: errors,
        budget_limited,
    }
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
    let out = fetch_wallet_balances_chain(&state, owner_pk).await?;
    Ok(Json(out))
}

async fn fetch_wallet_balances_chain(state: &AppState, owner_pk: Pubkey) -> ApiResult<WalletBalancesResponse> {
    let rpc_url_used = state.provider.current_endpoint().await;
    let rpc_endpoints = state.provider.all_endpoints();
    let token_timeout_ms = std::env::var("CLMM_WALLET_BALANCES_TOKEN_TIMEOUT_MS")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(4500);
    let native_timeout_ms = std::env::var("CLMM_WALLET_BALANCES_NATIVE_TIMEOUT_MS")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(token_timeout_ms);
    let fanout = std::env::var("CLMM_WALLET_BALANCES_FANOUT")
        .ok()
        .and_then(|s| s.trim().parse::<usize>().ok())
        .unwrap_or(3);
    let token_deadline = Duration::from_millis(token_timeout_ms);
    let native_deadline = Duration::from_millis(native_timeout_ms);
    let spl_pid = Pubkey::from_str(SPL_TOKEN_PROGRAM_ID).expect("SPL token program id");
    let t22_pid = Pubkey::from_str(TOKEN_2022_PROGRAM_ID).expect("token-2022 program id");

    let bal_fut = get_native_balance_hedged(owner_pk, rpc_endpoints.clone(), native_deadline);
    let legacy_fut = get_token_accounts_by_owner_fanout(
        owner_pk,
        spl_pid,
        rpc_endpoints.clone(),
        fanout,
        token_deadline,
    );
    let tok2022_fut =
        get_token_accounts_by_owner_fanout(owner_pk, t22_pid, rpc_endpoints, fanout, token_deadline);
    let (balance_outcome, legacy_outcome, tok2022_outcome) =
        tokio::join!(bal_fut, legacy_fut, tok2022_fut);

    let mut native_attempt_errors = balance_outcome.attempts;
    if balance_outcome.budget_limited {
        native_attempt_errors.push("degraded_budget_limited".to_string());
    }
    let lamports = balance_outcome
        .result
        .map_err(|e| ApiError::internal(format_attempts_error("getBalance", &e, &native_attempt_errors)))?;
    let sol = format!("{:.9}", (lamports as f64) / 1e9);

    let mut legacy_attempt_errors = legacy_outcome.attempts;
    let mut token2022_attempt_errors = tok2022_outcome.attempts;
    if legacy_outcome.budget_limited {
        legacy_attempt_errors.push("degraded_budget_limited".to_string());
    }
    if tok2022_outcome.budget_limited {
        token2022_attempt_errors.push("degraded_budget_limited".to_string());
    }
    let tok_legacy = legacy_outcome.result;
    let tok_2022 = tok2022_outcome.result;
    let legacy_ok = tok_legacy.is_ok();
    let token_2022_ok = tok_2022.is_ok();
    let legacy_err = tok_legacy
        .as_ref()
        .err()
        .map(|e| format_token_accounts_error(e, &legacy_attempt_errors));
    let token_2022_err = tok_2022
        .as_ref()
        .err()
        .map(|e| format_token_accounts_error(e, &token2022_attempt_errors));
    let mut token_rows = Vec::new();
    if let Ok(v) = tok_legacy {
        append_tokens_from_keyed_accounts(&v, &mut token_rows);
    }
    if let Ok(v) = tok_2022 {
        append_tokens_from_keyed_accounts(&v, &mut token_rows);
    }
    let token_accounts_total = token_rows.len() as u64;
    let tokens = merge_wallet_token_rows(token_rows);

    Ok(WalletBalancesResponse {
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
    })
}

fn wallet_effective_cache_ttl_secs() -> u64 {
    std::env::var("CLMM_WALLET_EFFECTIVE_CACHE_TTL_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(5)
}

fn ws_url_from_rpc_url(rpc_url: &str) -> String {
    if let Some(rest) = rpc_url.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = rpc_url.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        rpc_url.to_string()
    }
}

async fn ensure_wallet_ws_owner_worker(state: AppState, owner: String) {
    {
        let mut started = state.wallet_effective_ws_started.write().await;
        if started.contains(&owner) {
            return;
        }
        started.insert(owner.clone());
    }
    tokio::spawn(async move {
        loop {
            let rpc_url = state.provider.current_endpoint().await;
            let ws_url = ws_url_from_rpc_url(&rpc_url);
            let owner_for_log = owner.clone();
            let state_for_block = state.clone();
            let run = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
                let owner_pk = Pubkey::from_str(&owner_for_log)
                    .map_err(|e| anyhow::anyhow!("invalid owner pubkey for ws: {e}"))?;
                let token_program = Pubkey::from_str(SPL_TOKEN_PROGRAM_ID)
                    .map_err(|e| anyhow::anyhow!("invalid token program id: {e}"))?;
                let token_2022_program = Pubkey::from_str(TOKEN_2022_PROGRAM_ID)
                    .map_err(|e| anyhow::anyhow!("invalid token2022 program id: {e}"))?;
                let logs_cfg = RpcTransactionLogsConfig { commitment: None };
                let filter = solana_client::rpc_config::RpcTransactionLogsFilter::Mentions(vec![owner_for_log.clone()]);
                let owner_filter = RpcFilterType::Memcmp(Memcmp::new_raw_bytes(
                    32,
                    owner_pk.to_bytes().to_vec(),
                ));
                let token_program_cfg = RpcProgramAccountsConfig {
                    filters: Some(vec![owner_filter.clone()]),
                    account_config: Default::default(),
                    with_context: Some(true),
                    sort_results: None,
                };
                let token_2022_cfg = RpcProgramAccountsConfig {
                    filters: Some(vec![owner_filter]),
                    account_config: Default::default(),
                    with_context: Some(true),
                    sort_results: None,
                };
                let (_owner_client, owner_receiver) =
                    PubsubClient::account_subscribe(&ws_url, &owner_pk, None)?;
                let (_legacy_client, token_receiver) = PubsubClient::program_subscribe(
                    &ws_url,
                    &token_program,
                    Some(token_program_cfg),
                )?;
                let (_token2022_client, token_2022_receiver) = PubsubClient::program_subscribe(
                    &ws_url,
                    &token_2022_program,
                    Some(token_2022_cfg),
                )?;
                let (_logs_client, logs_receiver) =
                    PubsubClient::logs_subscribe(&ws_url, filter, logs_cfg)?;
                let handle = tokio::runtime::Handle::current();
                loop {
                    let mut changed = false;
                    if owner_receiver.recv_timeout(Duration::from_secs(30)).is_ok() {
                        changed = true;
                    }
                    if token_receiver.recv_timeout(Duration::from_millis(200)).is_ok() {
                        changed = true;
                    }
                    if token_2022_receiver.recv_timeout(Duration::from_millis(200)).is_ok() {
                        changed = true;
                    }
                    let logs_msg: Result<RpcResponseEnvelope<RpcLogsResponse>, _> =
                        logs_receiver.recv_timeout(Duration::from_millis(200));
                    if logs_msg.is_ok() {
                        changed = true;
                    }
                    if changed {
                        state_for_block
                            .wallet_ws_events_total
                            .fetch_add(1, Ordering::Relaxed);
                        if handle.block_on(refresh_wallet_effective_owner(
                            &state_for_block,
                            &owner_for_log,
                        ))
                        .is_err()
                        {
                            state_for_block
                                .wallet_ws_refresh_failures_total
                                .fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            })
            .await;
            if let Err(e) = run {
                tracing::warn!(owner = %owner, error = %e, "wallet ws worker join error");
                state
                    .wallet_ws_reconnects_total
                    .fetch_add(1, Ordering::Relaxed);
            }
            sleep(Duration::from_secs(2)).await;
        }
    });
}

#[utoipa::path(
    get,
    path = "/wallets/ws-status",
    tag = "Wallets",
    responses(
        (status = 200, description = "Wallet WS monitor status", body = WalletWsStatusResponse)
    )
)]
pub async fn get_wallet_ws_status(
    State(state): State<AppState>,
) -> ApiResult<Json<WalletWsStatusResponse>> {
    let started = state.wallet_effective_ws_started.read().await;
    let owners = started.iter().cloned().collect::<Vec<_>>();
    Ok(Json(WalletWsStatusResponse {
        owners_monitored: owners.len() as u32,
        owners,
        events_total: state.wallet_ws_events_total.load(Ordering::Relaxed),
        reconnects_total: state.wallet_ws_reconnects_total.load(Ordering::Relaxed),
        refresh_failures_total: state.wallet_ws_refresh_failures_total.load(Ordering::Relaxed),
    }))
}

fn parse_wsol_raw_from_tokens(tokens: &[WalletTokenBalance]) -> u64 {
    let ui = tokens
        .iter()
        .find(|t| t.mint == WSOL_MINT)
        .and_then(|t| t.ui_amount.parse::<f64>().ok())
        .unwrap_or(0.0);
    if ui <= 0.0 {
        0
    } else {
        (ui * 1e9).round() as u64
    }
}

fn build_effective_balances(
    chain: WalletBalancesResponse,
    op_rows: &[WalletConvertOpRow],
) -> WalletEffectiveBalancesResponse {
    let owner = chain.owner.clone();
    let mut native_effective = chain.lamports as i128;
    let mut wsol_effective = parse_wsol_raw_from_tokens(&chain.tokens) as i128;
    let pending = op_rows
        .iter()
        .filter(|r| {
            r.owner_pubkey == owner
                && matches!(
                    r.reconciliation_status,
                    WalletReconciliationStatus::PendingConfirmation
                        | WalletReconciliationStatus::ConfirmedUnreconciled
                )
        })
        .collect::<Vec<_>>();
    for row in &pending {
        let delta = row.amount_raw as i128;
        match row.direction {
            ConvertSolDirection::NativeToWsol => {
                native_effective -= delta;
                wsol_effective += delta;
            }
            ConvertSolDirection::WsolToNative => {
                native_effective += delta;
                wsol_effective -= delta;
            }
        }
    }
    let native_effective_lamports = native_effective.max(0) as u64;
    let wsol_effective_raw = wsol_effective.max(0) as u64;
    let mut tokens_effective = chain.tokens.clone();
    if let Some(row) = tokens_effective.iter_mut().find(|r| r.mint == WSOL_MINT) {
        row.ui_amount = format!("{:.9}", (wsol_effective_raw as f64) / 1e9);
    }
    let confidence = if chain.token_legacy_ok == Some(true)
        && chain.token_2022_ok == Some(true)
        && pending.is_empty()
    {
        WalletBalanceConfidence::Verified
    } else if !pending.is_empty() {
        WalletBalanceConfidence::Projected
    } else {
        WalletBalanceConfidence::Degraded
    };
    WalletEffectiveBalancesResponse {
        owner: owner.clone(),
        as_of_utc: now_utc_iso(),
        is_stale: false,
        stale_age_ms: 0,
        confidence,
        pending_ops_count: pending.len() as u64,
        native_onchain_lamports: chain.lamports,
        native_effective_lamports,
        wsol_onchain_raw: parse_wsol_raw_from_tokens(&chain.tokens),
        wsol_effective_raw,
        rpc_url: chain.rpc_url,
        lamports: native_effective_lamports,
        sol: format!("{:.9}", (native_effective_lamports as f64) / 1e9),
        tokens: tokens_effective,
        token_accounts_total: chain.token_accounts_total,
        token_legacy_ok: chain.token_legacy_ok,
        token_2022_ok: chain.token_2022_ok,
        token_legacy_error: chain.token_legacy_error,
        token_2022_error: chain.token_2022_error,
    }
}

fn stale_marked_response(
    mut resp: WalletEffectiveBalancesResponse,
    is_stale: bool,
    age_ms: u64,
) -> WalletEffectiveBalancesResponse {
    resp.is_stale = is_stale;
    resp.stale_age_ms = age_ms;
    resp
}

async fn spawn_effective_refresh_if_needed(state: AppState, owner: String, owner_pk: Pubkey) {
    let mut refreshing = state.wallet_effective_refreshing.write().await;
    if refreshing.contains(&owner) {
        return;
    }
    refreshing.insert(owner.clone());
    drop(refreshing);
    tokio::spawn(async move {
        if let Ok(resp) = compute_effective_balances(&state, owner_pk).await {
            let mut cache = state.wallet_effective_cache.write().await;
            cache.insert(
                owner.clone(),
                crate::state::CachedWalletEffective {
                    response: resp,
                    updated_at: Instant::now(),
                },
            );
        }
        let mut refreshing = state.wallet_effective_refreshing.write().await;
        refreshing.remove(&owner);
    });
}

async fn build_warmup_placeholder(state: &AppState, owner: &str) -> WalletEffectiveBalancesResponse {
    let rpc_url = state.provider.current_endpoint().await;
    WalletEffectiveBalancesResponse {
        owner: owner.to_string(),
        as_of_utc: now_utc_iso(),
        is_stale: true,
        stale_age_ms: 0,
        confidence: WalletBalanceConfidence::Degraded,
        pending_ops_count: 0,
        native_onchain_lamports: 0,
        native_effective_lamports: 0,
        wsol_onchain_raw: 0,
        wsol_effective_raw: 0,
        rpc_url,
        lamports: 0,
        sol: "0".to_string(),
        tokens: Vec::new(),
        token_accounts_total: Some(0),
        token_legacy_ok: Some(false),
        token_2022_ok: Some(false),
        token_legacy_error: Some("warmup: effective cache miss, refresh in progress".to_string()),
        token_2022_error: Some("warmup: effective cache miss, refresh in progress".to_string()),
    }
}

async fn apply_last_good_tokens_fallback(
    state: &AppState,
    owner: &str,
    mut chain: WalletBalancesResponse,
) -> WalletBalancesResponse {
    let both_failed = chain.token_legacy_ok == Some(false) && chain.token_2022_ok == Some(false);
    if !both_failed || !chain.tokens.is_empty() {
        return chain;
    }
    let cache = state.wallet_effective_cache.read().await;
    let Some(cached) = cache.get(owner) else {
        return chain;
    };
    let cached_tokens = cached.response.tokens.clone();
    if cached_tokens.is_empty() {
        return chain;
    }
    chain.tokens = cached_tokens;
    chain.token_accounts_total = Some(chain.tokens.len() as u64);
    chain.token_legacy_error = Some(match chain.token_legacy_error {
        Some(e) => format!("{e} | fallback: using last-good token snapshot"),
        None => "fallback: using last-good token snapshot".to_string(),
    });
    chain.token_2022_error = Some(match chain.token_2022_error {
        Some(e) => format!("{e} | fallback: using last-good token snapshot"),
        None => "fallback: using last-good token snapshot".to_string(),
    });
    chain
}

async fn compute_effective_balances(
    state: &AppState,
    owner_pk: Pubkey,
) -> ApiResult<WalletEffectiveBalancesResponse> {
    let chain_raw = fetch_wallet_balances_chain(state, owner_pk).await?;
    let owner = owner_pk.to_string();
    let chain = apply_last_good_tokens_fallback(state, &owner, chain_raw).await;
    let op_rows = {
        let _guard = state.wallet_ops_lock.lock().await;
        read_wallet_convert_ops(&wallet_ops_store_path())
            .map_err(|e| ApiError::internal(format!("read wallet ops store: {e}")))?
    };
    Ok(build_effective_balances(chain, &op_rows))
}

#[utoipa::path(
    get,
    path = "/wallets/effective-balances",
    tag = "Wallets",
    params(
        ("owner" = String, Query, description = "Solana wallet pubkey (base58)")
    ),
    responses(
        (status = 200, description = "Fast effective balances", body = WalletEffectiveBalancesResponse),
        (status = 400, description = "Invalid owner pubkey")
    )
)]
pub async fn get_wallet_effective_balances(
    State(state): State<AppState>,
    Query(q): Query<WalletBalancesQuery>,
) -> ApiResult<Json<WalletEffectiveBalancesResponse>> {
    let owner_trim = q.owner.trim();
    if owner_trim.is_empty() {
        return Err(ApiError::bad_request("query parameter `owner` is required"));
    }
    let owner_pk =
        Pubkey::from_str(owner_trim).map_err(|_| ApiError::bad_request("invalid owner pubkey"))?;
    let owner = owner_pk.to_string();
    ensure_wallet_ws_owner_worker(state.clone(), owner.clone()).await;
    let ttl = Duration::from_secs(wallet_effective_cache_ttl_secs());
    {
        let cache = state.wallet_effective_cache.read().await;
        if let Some(cached) = cache.get(&owner) {
            let is_fresh = cached.updated_at.elapsed() <= ttl;
            if !is_fresh {
                spawn_effective_refresh_if_needed(state.clone(), owner.clone(), owner_pk).await;
            }
            let age_ms = cached.updated_at.elapsed().as_millis() as u64;
            return Ok(Json(stale_marked_response(
                cached.response.clone(),
                !is_fresh,
                if is_fresh { 0 } else { age_ms },
            )));
        }
    }
    spawn_effective_refresh_if_needed(state.clone(), owner.clone(), owner_pk).await;
    Ok(Json(build_warmup_placeholder(&state, &owner).await))
}

pub async fn refresh_wallet_effective_owner(state: &AppState, owner: &str) -> anyhow::Result<()> {
    let owner_pk = Pubkey::from_str(owner).map_err(|e| anyhow::anyhow!("invalid owner pubkey: {e}"))?;
    let resp = compute_effective_balances(state, owner_pk)
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let mut cache = state.wallet_effective_cache.write().await;
    cache.insert(
        owner.to_string(),
        crate::state::CachedWalletEffective {
            response: resp,
            updated_at: Instant::now(),
        },
    );
    Ok(())
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
        (status = 200, description = "SOL conversion confirmed", body = ConvertSolResponse),
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
    let pre_native_lamports = state
        .provider
        .get_balance(&owner)
        .await
        .map_err(|e| ApiError::internal(format!("read pre native SOL balance: {e}")))?;
    let pre_wsol_raw = exec
        .read_wsol_balance_raw(&owner)
        .await
        .map_err(|e| ApiError::internal(format!("read pre WSOL balance: {e}")))?;
    let (signature, wrap_signature, unwrap_signature, rewrap_signature, partial, message) =
        match req.direction {
        ConvertSolDirection::NativeToWsol => {
            if pre_native_lamports < req.amount_raw {
                return Err(ApiError::bad_request(format!(
                    "insufficient native SOL balance (have {pre_native_lamports} raw, need {} raw)",
                    req.amount_raw
                )));
            }
            let wrap_sig = exec
                .submit_wsol_wrap_with_signature_delta(req.amount_raw, signer.keypair())
                .await
                .map_err(|e| ApiError::bad_request(format!("native_to_wsol failed: {e}")))?
                .map(|s| s.to_string());
            let msg = if wrap_sig.is_some() {
                "SOL conversion confirmed".to_string()
            } else {
                "SOL->WSOL no-op".to_string()
            };
            (
                wrap_sig.clone(),
                wrap_sig,
                None,
                None,
                false,
                msg,
            )
        }
        ConvertSolDirection::WsolToNative => {
            if pre_wsol_raw < req.amount_raw {
                return Err(ApiError::bad_request(format!(
                    "insufficient WSOL balance (have {pre_wsol_raw} raw, need {} raw)",
                    req.amount_raw
                )));
            }
            let partial = req.amount_raw < pre_wsol_raw;
            let sig = exec
                .submit_wsol_unwrap_with_signature(req.amount_raw, signer.keypair())
                .await
                .map_err(|e| ApiError::bad_request(format!("wsol_to_native failed: {e}")))?;
            let unwrap_sig = sig.to_string();
            // For partial unwrap close+rewrap path, unwrap tx signature is still useful as primary.
            (
                Some(unwrap_sig.clone()),
                None,
                Some(unwrap_sig),
                None,
                partial,
                "SOL conversion confirmed".to_string(),
            )
        }
    };
    let post_native_lamports = state
        .provider
        .get_balance(&owner)
        .await
        .map_err(|e| ApiError::internal(format!("read post native SOL balance: {e}")))?;
    let post_wsol_raw = exec
        .read_wsol_balance_raw(&owner)
        .await
        .map_err(|e| ApiError::internal(format!("read post WSOL balance: {e}")))?;
    let op_id = Uuid::new_v4().to_string();
    let mut reconciliation_status = WalletReconciliationStatus::ConfirmedUnreconciled;
    if is_reconciled(
        &WalletConvertOpRow {
            op_id: op_id.clone(),
            owner_pubkey: owner.to_string(),
            direction: req.direction.clone(),
            amount_raw: req.amount_raw,
            reconciliation_status: WalletReconciliationStatus::ConfirmedUnreconciled,
            reason_code: Some("awaiting_reconcile".to_string()),
            attempts: 1,
            created_at_utc: now_utc_iso(),
            updated_at_utc: now_utc_iso(),
            last_verified_at_utc: Some(now_utc_iso()),
            pre_native_lamports,
            pre_wsol_raw,
            post_native_lamports: Some(post_native_lamports),
            post_wsol_raw: Some(post_wsol_raw),
            wrap_signature: wrap_signature.clone(),
            unwrap_signature: unwrap_signature.clone(),
            rewrap_signature: rewrap_signature.clone(),
            tx_signature: signature.clone(),
            last_error: None,
        },
        post_wsol_raw,
        wallet_reconcile_wsol_tolerance_raw(),
    ) {
        reconciliation_status = WalletReconciliationStatus::Reconciled;
    }
    let row = WalletConvertOpRow {
        op_id: op_id.clone(),
        owner_pubkey: owner.to_string(),
        direction: req.direction.clone(),
        amount_raw: req.amount_raw,
        reconciliation_status: reconciliation_status.clone(),
        reason_code: Some(if matches!(reconciliation_status, WalletReconciliationStatus::Reconciled) {
            "post_read_matched".to_string()
        } else {
            "awaiting_reconcile".to_string()
        }),
        attempts: 1,
        created_at_utc: now_utc_iso(),
        updated_at_utc: now_utc_iso(),
        last_verified_at_utc: Some(now_utc_iso()),
        pre_native_lamports,
        pre_wsol_raw,
        post_native_lamports: Some(post_native_lamports),
        post_wsol_raw: Some(post_wsol_raw),
        wrap_signature: wrap_signature.clone(),
        unwrap_signature: unwrap_signature.clone(),
        rewrap_signature: rewrap_signature.clone(),
        tx_signature: signature.clone(),
        last_error: None,
    };
    let response_reason_code = row.reason_code.clone();
    let response_attempts = row.attempts;
    let response_last_verified_at_utc = row.last_verified_at_utc.clone();
    {
        let _guard = state.wallet_ops_lock.lock().await;
        let path = wallet_ops_store_path();
        let mut rows = read_wallet_convert_ops(&path)
            .map_err(|e| ApiError::internal(format!("read wallet ops store: {e}")))?;
        rows.push(row);
        write_wallet_convert_ops(&path, &rows)
            .map_err(|e| ApiError::internal(format!("write wallet ops store: {e}")))?;
    }

    Ok(Json(ConvertSolResponse {
        message,
        signature,
        wrap_signature,
        unwrap_signature,
        rewrap_signature,
        confirmed: true,
        partial,
        op_id,
        reconciliation_status,
        reason_code: response_reason_code,
        attempts: response_attempts,
        last_verified_at_utc: response_last_verified_at_utc,
        direction: req.direction,
        amount_raw: req.amount_raw,
        owner_pubkey: owner.to_string(),
        post_native_lamports,
        post_wsol_raw,
    }))
}

#[derive(Debug, Deserialize)]
pub struct WalletOpsQuery {
    pub owner: Option<String>,
    pub status: Option<String>,
    pub reason_code: Option<String>,
    pub updated_after: Option<String>,
    pub limit: Option<usize>,
}

/// List/inspect wallet conversion operation reconciliation state.
#[utoipa::path(
    get,
    path = "/wallets/ops",
    tag = "Wallets",
    params(
        ("owner" = Option<String>, Query, description = "owner pubkey filter"),
        ("status" = Option<String>, Query, description = "reconciliation status filter"),
        ("reason_code" = Option<String>, Query, description = "reason code filter"),
        ("updated_after" = Option<String>, Query, description = "RFC3339 lower bound for updated_at_utc"),
        ("limit" = Option<usize>, Query, description = "maximum rows")
    ),
    responses(
        (status = 200, description = "Wallet conversion operations", body = [WalletConvertOpResponse]),
        (status = 500, description = "I/O error")
    )
)]
pub async fn list_wallet_ops(
    State(state): State<AppState>,
    Query(q): Query<WalletOpsQuery>,
) -> ApiResult<Json<Vec<WalletConvertOpResponse>>> {
    let _guard = state.wallet_ops_lock.lock().await;
    let mut rows = read_wallet_convert_ops(&wallet_ops_store_path())
        .map_err(|e| ApiError::internal(format!("read wallet ops store: {e}")))?;
    if let Some(owner) = q.owner.as_ref() {
        rows.retain(|r| r.owner_pubkey == *owner);
    }
    if let Some(status) = q.status.as_ref() {
        let status = status.trim().to_ascii_lowercase();
        rows.retain(|r| {
            serde_json::to_string(&r.reconciliation_status)
                .unwrap_or_default()
                .to_ascii_lowercase()
                .contains(&status)
        });
    }
    if let Some(reason_code) = q.reason_code.as_ref() {
        let rc = reason_code.trim().to_ascii_lowercase();
        rows.retain(|r| {
            r.reason_code
                .as_ref()
                .map(|s| s.to_ascii_lowercase().contains(&rc))
                .unwrap_or(false)
        });
    }
    if let Some(updated_after) = q.updated_after.as_ref()
        && let Ok(threshold) = chrono::DateTime::parse_from_rfc3339(updated_after.trim())
    {
        rows.retain(|r| {
            chrono::DateTime::parse_from_rfc3339(&r.updated_at_utc)
                .ok()
                .map(|ts| ts >= threshold)
                .unwrap_or(false)
        });
    }
    rows.sort_by(|a, b| b.updated_at_utc.cmp(&a.updated_at_utc));
    let take = q.limit.unwrap_or(50).min(500);
    let out = rows
        .into_iter()
        .take(take)
        .map(|r| to_wallet_op_response(&r))
        .collect::<Vec<_>>();
    Ok(Json(out))
}

#[utoipa::path(
    get,
    path = "/wallets/ops/stats",
    tag = "Wallets",
    responses(
        (status = 200, description = "Wallet conversion operation aggregate stats", body = WalletOpsStatsResponse),
        (status = 500, description = "I/O error")
    )
)]
pub async fn get_wallet_ops_stats(
    State(state): State<AppState>,
) -> ApiResult<Json<WalletOpsStatsResponse>> {
    let _guard = state.wallet_ops_lock.lock().await;
    let rows = read_wallet_convert_ops(&wallet_ops_store_path())
        .map_err(|e| ApiError::internal(format!("read wallet ops store: {e}")))?;
    let total = rows.len() as u64;
    let mut reconciled = 0u64;
    let mut confirmed_unreconciled = 0u64;
    let mut mismatch = 0u64;
    let mut failed = 0u64;
    let mut pending_confirmation = 0u64;
    let mut reconcile_secs: Vec<f64> = Vec::new();
    for row in rows {
        match row.reconciliation_status {
            WalletReconciliationStatus::Reconciled => {
                reconciled += 1;
                if let (Ok(created), Ok(updated)) = (
                    chrono::DateTime::parse_from_rfc3339(&row.created_at_utc),
                    chrono::DateTime::parse_from_rfc3339(&row.updated_at_utc),
                ) {
                    reconcile_secs.push((updated - created).num_seconds().max(0) as f64);
                }
            }
            WalletReconciliationStatus::ConfirmedUnreconciled => confirmed_unreconciled += 1,
            WalletReconciliationStatus::Mismatch => mismatch += 1,
            WalletReconciliationStatus::Failed => failed += 1,
            WalletReconciliationStatus::PendingConfirmation => pending_confirmation += 1,
        }
    }
    let mismatch_ratio = if total > 0 {
        Some((mismatch as f64) / (total as f64))
    } else {
        None
    };
    let avg_seconds_to_reconcile = if reconcile_secs.is_empty() {
        None
    } else {
        Some(reconcile_secs.iter().sum::<f64>() / reconcile_secs.len() as f64)
    };
    Ok(Json(WalletOpsStatsResponse {
        total,
        reconciled,
        confirmed_unreconciled,
        mismatch,
        failed,
        pending_confirmation,
        mismatch_ratio,
        avg_seconds_to_reconcile,
    }))
}

#[utoipa::path(
    get,
    path = "/wallets/ops/{op_id}",
    tag = "Wallets",
    params(
        ("op_id" = String, Path, description = "operation id")
    ),
    responses(
        (status = 200, description = "Wallet conversion operation", body = WalletConvertOpResponse),
        (status = 404, description = "Operation not found"),
        (status = 500, description = "I/O error")
    )
)]
pub async fn get_wallet_op(
    State(state): State<AppState>,
    axum::extract::Path(op_id): axum::extract::Path<String>,
) -> ApiResult<Json<WalletConvertOpResponse>> {
    let _guard = state.wallet_ops_lock.lock().await;
    let rows = read_wallet_convert_ops(&wallet_ops_store_path())
        .map_err(|e| ApiError::internal(format!("read wallet ops store: {e}")))?;
    let row = rows
        .into_iter()
        .find(|r| r.op_id == op_id)
        .ok_or_else(|| ApiError::not_found("wallet operation not found"))?;
    Ok(Json(to_wallet_op_response(&row)))
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

    #[test]
    fn pick_fanout_endpoints_respects_bound_and_order() {
        let endpoints = vec![
            "https://rpc-a.example".to_string(),
            "https://rpc-b.example".to_string(),
            "https://rpc-c.example".to_string(),
        ];
        let picked = pick_fanout_endpoints(&endpoints, 2);
        assert_eq!(
            picked,
            vec![
                "https://rpc-a.example".to_string(),
                "https://rpc-b.example".to_string()
            ]
        );
    }

    #[test]
    fn pick_fanout_endpoints_uses_all_when_fanout_large() {
        let endpoints = vec!["a".to_string(), "b".to_string()];
        let picked = pick_fanout_endpoints(&endpoints, 10);
        assert_eq!(picked, endpoints);
    }

    #[test]
    fn pick_fanout_endpoints_minimum_one_when_zero_requested() {
        let endpoints = vec!["a".to_string(), "b".to_string()];
        let picked = pick_fanout_endpoints(&endpoints, 0);
        assert_eq!(picked, vec!["a".to_string()]);
    }

    #[test]
    fn format_token_accounts_error_includes_attempts() {
        let e = anyhow::anyhow!("timeout");
        let attempts = vec![
            "https://rpc-a.example: timeout".to_string(),
            "https://rpc-b.example: 429".to_string(),
        ];
        let msg = format_token_accounts_error(&e, &attempts);
        assert!(msg.contains("getTokenAccountsByOwner: timeout"));
        assert!(msg.contains("rpc-a.example"));
        assert!(msg.contains("rpc-b.example"));
    }

    #[test]
    fn format_attempts_error_without_attempts() {
        let e = anyhow::anyhow!("boom");
        let msg = format_attempts_error("getBalance", &e, &[]);
        assert_eq!(msg, "getBalance: boom");
    }

    #[test]
    fn penalize_token_endpoint_marks_and_filters() {
        let endpoints = vec![
            "https://rpc-a.example".to_string(),
            "https://rpc-b.example".to_string(),
        ];
        penalize_token_endpoint(&endpoints[0], "HTTP status client error (403 Forbidden)");
        let filtered = filter_penalized_token_endpoints(&endpoints);
        assert_eq!(filtered, vec!["https://rpc-b.example".to_string()]);
    }

    #[test]
    fn penalize_token_endpoint_ignores_non_penalty_errors() {
        let endpoints = vec!["https://rpc-c.example".to_string()];
        penalize_token_endpoint(&endpoints[0], "account not found");
        let filtered = filter_penalized_token_endpoints(&endpoints);
        assert_eq!(filtered, endpoints);
    }

    #[test]
    fn is_reconciled_native_to_wsol_checks_expected_delta() {
        let row = WalletConvertOpRow {
            op_id: "op-1".to_string(),
            owner_pubkey: "owner".to_string(),
            direction: ConvertSolDirection::NativeToWsol,
            amount_raw: 50,
            reconciliation_status: WalletReconciliationStatus::ConfirmedUnreconciled,
            reason_code: None,
            attempts: 1,
            created_at_utc: now_utc_iso(),
            updated_at_utc: now_utc_iso(),
            last_verified_at_utc: None,
            pre_native_lamports: 1_000,
            pre_wsol_raw: 100,
            post_native_lamports: None,
            post_wsol_raw: None,
            wrap_signature: None,
            unwrap_signature: None,
            rewrap_signature: None,
            tx_signature: None,
            last_error: None,
        };
        assert!(is_reconciled(&row, 149, 1));
        assert!(!is_reconciled(&row, 140, 1));
    }

    #[test]
    fn is_reconciled_wsol_to_native_checks_expected_delta() {
        let row = WalletConvertOpRow {
            op_id: "op-2".to_string(),
            owner_pubkey: "owner".to_string(),
            direction: ConvertSolDirection::WsolToNative,
            amount_raw: 40,
            reconciliation_status: WalletReconciliationStatus::ConfirmedUnreconciled,
            reason_code: None,
            attempts: 1,
            created_at_utc: now_utc_iso(),
            updated_at_utc: now_utc_iso(),
            last_verified_at_utc: None,
            pre_native_lamports: 1_000,
            pre_wsol_raw: 120,
            post_native_lamports: None,
            post_wsol_raw: None,
            wrap_signature: None,
            unwrap_signature: None,
            rewrap_signature: None,
            tx_signature: None,
            last_error: None,
        };
        assert!(is_reconciled(&row, 80, 0));
        assert!(!is_reconciled(&row, 100, 0));
    }

    #[test]
    fn build_effective_balances_projects_pending_native_to_wsol() {
        let chain = WalletBalancesResponse {
            owner: "owner-1".to_string(),
            rpc_url: "rpc".to_string(),
            lamports: 1_000_000_000,
            sol: "1".to_string(),
            tokens: vec![WalletTokenBalance {
                mint: WSOL_MINT.to_string(),
                ui_amount: "0".to_string(),
            }],
            token_accounts_total: Some(1),
            token_legacy_ok: Some(true),
            token_2022_ok: Some(true),
            token_legacy_error: None,
            token_2022_error: None,
        };
        let op = WalletConvertOpRow {
            op_id: "op".to_string(),
            owner_pubkey: "owner-1".to_string(),
            direction: ConvertSolDirection::NativeToWsol,
            amount_raw: 200_000_000,
            reconciliation_status: WalletReconciliationStatus::ConfirmedUnreconciled,
            reason_code: None,
            attempts: 1,
            created_at_utc: now_utc_iso(),
            updated_at_utc: now_utc_iso(),
            last_verified_at_utc: None,
            pre_native_lamports: 1_000_000_000,
            pre_wsol_raw: 0,
            post_native_lamports: None,
            post_wsol_raw: None,
            wrap_signature: None,
            unwrap_signature: None,
            rewrap_signature: None,
            tx_signature: None,
            last_error: None,
        };
        let eff = build_effective_balances(chain, &[op]);
        assert_eq!(eff.native_effective_lamports, 800_000_000);
        assert_eq!(eff.wsol_effective_raw, 200_000_000);
        assert!(matches!(eff.confidence, WalletBalanceConfidence::Projected));
    }

    #[test]
    fn build_effective_balances_ignores_mismatch_ops() {
        let chain = WalletBalancesResponse {
            owner: "owner-2".to_string(),
            rpc_url: "rpc".to_string(),
            lamports: 900,
            sol: "0.000000900".to_string(),
            tokens: vec![],
            token_accounts_total: Some(0),
            token_legacy_ok: Some(false),
            token_2022_ok: Some(false),
            token_legacy_error: Some("x".to_string()),
            token_2022_error: Some("y".to_string()),
        };
        let op = WalletConvertOpRow {
            op_id: "op".to_string(),
            owner_pubkey: "owner-2".to_string(),
            direction: ConvertSolDirection::WsolToNative,
            amount_raw: 100,
            reconciliation_status: WalletReconciliationStatus::Mismatch,
            reason_code: None,
            attempts: 2,
            created_at_utc: now_utc_iso(),
            updated_at_utc: now_utc_iso(),
            last_verified_at_utc: None,
            pre_native_lamports: 800,
            pre_wsol_raw: 100,
            post_native_lamports: None,
            post_wsol_raw: None,
            wrap_signature: None,
            unwrap_signature: None,
            rewrap_signature: None,
            tx_signature: None,
            last_error: None,
        };
        let eff = build_effective_balances(chain, &[op]);
        assert_eq!(eff.native_effective_lamports, 900);
        assert_eq!(eff.pending_ops_count, 0);
    }
}
