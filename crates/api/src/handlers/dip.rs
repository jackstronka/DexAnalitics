//! Dip Agent endpoints: snapshot (B) + propose (C) + runtime (U2) + Trigger attach/vault/orders (E0–E5).

use crate::error::{ApiError, ApiResult};
use crate::handlers::wallets::load_wallet_keypair_for_id;
use crate::models::{
    AgentDecisionRow, DipBuyDipOrder, DipLadderDefaultsDto, DipProposeRequest, DipProposeResponse,
    DipRuntimeResponse, DipSnapshotResponse, DipTriggerAttachRequest, DipTriggerAttachResponse,
    DipTriggerDcaHistoryResponse, DipTriggerDcaOrder, DipTriggerDcaOrderCancelRequest,
    DipTriggerDcaOrderCancelResponse, DipTriggerDcaOrderCreateRequest,
    DipTriggerDcaOrderCreateResponse, DipTriggerDepositCraftRequest,
    DipTriggerDepositCraftResponse, DipTriggerOpenOrder, DipTriggerPriceOrderCancelRequest,
    DipTriggerPriceOrderCancelResponse, DipTriggerPriceOrderCreateRequest,
    DipTriggerPriceOrderCreateResponse, DipTriggerPriceOrderHistoryResponse,
    DipTriggerPriceOrderPatchRequest, DipTriggerPriceOrderPatchResponse,
    DipTriggerSessionStatusDto, DipTriggerVaultResponse, DipZoneBandDto, DipZonesDto,
};
use crate::services::price_fetch::fetch_mint_prices_usd;
use crate::state::AppState;
use axum::Json;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::Utc;
use clmm_lp_data::dip::{
    DipAuditDecisionRow, DipLadderDefaults, DipProposeAction, DipProposeDecision, DipProposeMode,
    DipRuntimeFile, DipTriggerSession, DipZoneLabel, append_dip_propose, classify_zone,
    default_agent_decisions_path, default_zones_file, delete_trigger_session,
    load_dip_runtime_file, load_dip_zones_file, load_trigger_session, normalize_dip_mode,
    resolve_runtime_path, resolve_trigger_session_dir, save_trigger_session,
};
use clmm_lp_data::providers::{
    JupiterTriggerClient, OrderHistoryQuery, TriggerChallenge, TriggerDcaOrderCreateRequest,
    TriggerDepositCraftRequest, TriggerOrderHistoryItem, TriggerPriceOrderConfirmCancelRequest,
    TriggerPriceOrderCreateRequest, TriggerPriceOrderPatchRequest, TriggerPriceOrderRole,
    classify_history_item_role, classify_price_order_role, filter_open_buy_dips,
    filter_open_price_orders,
};
use serde::Deserialize;
use solana_sdk::signature::{Keypair, Signer};
use solana_sdk::transaction::VersionedTransaction;
use std::collections::BTreeSet;
use std::path::PathBuf;

const SOL_MINT: &str = "So11111111111111111111111111111111111111112";

#[derive(Debug, Deserialize)]
pub struct DipSnapshotQuery {
    /// When true (default), attempt Trigger order history if JWT is available.
    #[serde(default = "default_true")]
    pub include_orders: bool,
    /// Override path to `config/dip/zones.json`.
    #[serde(default)]
    pub zones_path: Option<String>,
    /// Prefer Trigger session JWT bound to this host `wallet_id` (after `/dip/trigger/attach`).
    #[serde(default)]
    pub wallet_id: Option<String>,
}

fn default_true() -> bool {
    true
}

fn bearer_from_headers(headers: &HeaderMap) -> Option<String> {
    let raw = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    let token = raw
        .strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))?;
    let t = token.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

fn env_trigger_jwt() -> Option<String> {
    std::env::var("CLMM_JUPITER_TRIGGER_JWT")
        .ok()
        .filter(|s| !s.trim().is_empty())
}

/// Resolve Trigger access token: Authorization → per-wallet session → env.
/// Returns `(jwt, source_label)`.
fn resolve_trigger_jwt(
    headers: &HeaderMap,
    wallet_id: Option<&str>,
) -> (Option<String>, &'static str) {
    if let Some(j) = bearer_from_headers(headers) {
        return (Some(j), "authorization_header");
    }
    if let Some(wid) = wallet_id.map(str::trim).filter(|s| !s.is_empty()) {
        let dir = resolve_trigger_session_dir(None);
        if let Ok(Some(session)) = load_trigger_session(&dir, wid) {
            return (Some(session.tokens.access_token), "wallet_session");
        }
    }
    if let Some(j) = env_trigger_jwt() {
        return (Some(j), "env_CLMM_JUPITER_TRIGGER_JWT");
    }
    (None, "none")
}

fn session_status_dto(wallet_id: &str) -> Option<DipTriggerSessionStatusDto> {
    let dir = resolve_trigger_session_dir(None);
    match load_trigger_session(&dir, wallet_id) {
        Ok(Some(s)) => Some(DipTriggerSessionStatusDto {
            wallet_id: s.wallet_id,
            wallet_pubkey: s.wallet_pubkey,
            attached: true,
            expires_at: s.tokens.expires_at,
            attached_at: Some(s.attached_at),
        }),
        _ => None,
    }
}

fn trigger_jwt_present(headers: &HeaderMap, wallet_id: Option<&str>) -> bool {
    resolve_trigger_jwt(headers, wallet_id).0.is_some()
}

fn jupiter_api_key(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .filter(|s| !s.is_empty())
        .or_else(|| {
            std::env::var("JUPITER_API_KEY")
                .ok()
                .filter(|s| !s.trim().is_empty())
        })
}

fn resolve_zones_path(override_path: Option<&str>) -> PathBuf {
    if let Some(p) = override_path {
        return PathBuf::from(p);
    }
    if let Ok(p) = std::env::var("CLMM_DIP_ZONES_PATH")
        && !p.trim().is_empty()
    {
        return PathBuf::from(p);
    }
    PathBuf::from("config/dip/zones.json")
}

fn map_trigger_open_order(o: TriggerOrderHistoryItem) -> DipTriggerOpenOrder {
    let role = classify_history_item_role(&o).as_str().to_string();
    DipTriggerOpenOrder {
        id: o.id,
        role,
        order_state: o.order_state,
        trigger_condition: o.trigger_condition,
        trigger_price_usd: o.trigger_price_usd,
        input_mint: o.input_mint,
        output_mint: o.output_mint,
        trigger_mint: o.trigger_mint,
        remaining_input_amount: o.remaining_input_amount,
        expires_at: o.expires_at,
    }
}

fn map_buy_dip_order(o: &DipTriggerOpenOrder) -> DipBuyDipOrder {
    DipBuyDipOrder {
        id: o.id.clone(),
        order_state: o.order_state.clone(),
        trigger_price_usd: o.trigger_price_usd,
        input_mint: o.input_mint.clone(),
        output_mint: o.output_mint.clone(),
        remaining_input_amount: o.remaining_input_amount.clone(),
        expires_at: o.expires_at,
    }
}

/// `GET /dip/snapshot` — SOL zone + optional open Trigger price orders (read-only).
#[utoipa::path(
    get,
    path = "/dip/snapshot",
    tag = "Dip",
    params(
        ("include_orders" = Option<bool>, Query, description = "Fetch Trigger active orders when JWT present"),
        ("zones_path" = Option<String>, Query, description = "Override path to zones.json"),
        ("wallet_id" = Option<String>, Query, description = "Use Trigger session for this host wallet id")
    ),
    responses(
        (status = 200, description = "Dip snapshot", body = DipSnapshotResponse)
    )
)]
pub async fn get_dip_snapshot(
    Query(q): Query<DipSnapshotQuery>,
    headers: HeaderMap,
) -> ApiResult<Json<DipSnapshotResponse>> {
    let zones_path = resolve_zones_path(q.zones_path.as_deref());
    let zones_file = load_dip_zones_file(&zones_path).unwrap_or_else(|_| default_zones_file());

    let mut mints = BTreeSet::new();
    mints.insert(SOL_MINT.to_string());
    let (prices, price_source) = fetch_mint_prices_usd(&mints).await;
    let sol_price = prices.get(SOL_MINT).copied();
    let zone = match sol_price {
        Some(px) => classify_zone(&zones_file, "SOL", px),
        None => DipZoneLabel::None,
    };

    let sol_zones = zones_file
        .symbols
        .get("SOL")
        .cloned()
        .unwrap_or_else(clmm_lp_data::dip::default_sol_zones);

    let mut notes = vec![
        "propose-only: no Trigger create/cancel from this endpoint".to_string(),
        format!("zones_path={}", zones_path.display()),
    ];

    let mut open_orders: Vec<DipTriggerOpenOrder> = Vec::new();
    let mut buy_dip_orders: Vec<DipBuyDipOrder> = Vec::new();
    let mut orders_source = "skipped".to_string();

    if q.include_orders {
        let wallet_id = q.wallet_id.as_deref();
        let (jwt, jwt_source) = resolve_trigger_jwt(&headers, wallet_id);
        match (jupiter_api_key(&headers), jwt) {
            (Some(api_key), Some(jwt)) => {
                notes.push(format!("trigger_jwt_source={jwt_source}"));
                let client = JupiterTriggerClient::new(Some(api_key));
                match client
                    .get_order_history(
                        &jwt,
                        &OrderHistoryQuery {
                            state: Some("active".into()),
                            limit: Some(50),
                            ..Default::default()
                        },
                    )
                    .await
                {
                    Ok(hist) => {
                        open_orders = filter_open_price_orders(&hist.orders)
                            .into_iter()
                            .map(map_trigger_open_order)
                            .collect();
                        buy_dip_orders = open_orders
                            .iter()
                            .filter(|o| o.role == TriggerPriceOrderRole::BuyDip.as_str())
                            .map(map_buy_dip_order)
                            .collect();
                        // Keep T13 helper path exercised for regressions.
                        let _ = filter_open_buy_dips(&hist.orders);
                        orders_source = "trigger_v2_orders_history".to_string();
                    }
                    Err(e) => {
                        orders_source = "error".to_string();
                        notes.push(format!("trigger orders/history failed: {e}"));
                    }
                }
            }
            (None, _) => {
                notes.push(
                    "JUPITER_API_KEY missing — zones/price only; set env or x-api-key header"
                        .into(),
                );
                orders_source = "no_api_key".into();
            }
            (_, None) => {
                notes.push(
                    "Trigger JWT missing — POST /dip/trigger/attach {wallet_id}, or Authorization Bearer, or CLMM_JUPITER_TRIGGER_JWT"
                        .into(),
                );
                orders_source = "no_jwt".into();
            }
        }
    } else {
        notes.push("include_orders=false".into());
    }

    if sol_price.is_none() {
        notes.push("SOL USD price unavailable from price fetch".into());
    }

    Ok(Json(DipSnapshotResponse {
        fetched_at: chrono::Utc::now().to_rfc3339(),
        symbol: "SOL".into(),
        mint: SOL_MINT.into(),
        price_usd: sol_price,
        price_source,
        zone: zone.as_str().to_string(),
        zones: DipZonesDto {
            a: DipZoneBandDto {
                min: sol_zones.a.min,
                max: sol_zones.a.max,
            },
            b: DipZoneBandDto {
                min: sol_zones.b.min,
                max: sol_zones.b.max,
            },
            c: DipZoneBandDto {
                min: sol_zones.c.min,
                max: sol_zones.c.max,
            },
        },
        open_orders,
        buy_dip_orders,
        orders_source,
        notes,
    }))
}

/// `GET /dip/runtime` — mode + ladder defaults + whether Jupiter key / Trigger JWT are set (no secrets).
#[utoipa::path(
    get,
    path = "/dip/runtime",
    tag = "Dip",
    params(
        ("runtime_path" = Option<String>, Query, description = "Override path to runtime.yaml"),
        ("wallet_id" = Option<String>, Query, description = "Include Trigger session status for this wallet")
    ),
    responses(
        (status = 200, description = "Dip runtime (public)", body = DipRuntimeResponse)
    )
)]
pub async fn get_dip_runtime(
    Query(q): Query<DipRuntimeQuery>,
    headers: HeaderMap,
) -> ApiResult<Json<DipRuntimeResponse>> {
    let path = resolve_runtime_path(q.runtime_path.as_deref());
    let mut notes = Vec::new();
    let (file, file_missing) = match load_dip_runtime_file(&path) {
        Ok(f) => (f, false),
        Err(e) => {
            notes.push(format!("runtime load failed ({e}); using defaults"));
            (DipRuntimeFile::default(), !path.exists())
        }
    };

    let wallet_id = q
        .wallet_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let trigger_session = wallet_id.and_then(session_status_dto);
    let ladder = ladder_dto(&file.ladder);
    Ok(Json(DipRuntimeResponse {
        path: path.to_string_lossy().to_string(),
        file_missing,
        mode: normalize_dip_mode(&file.mode),
        dca_allowed: file.jupiter.dca_allowed,
        jupiter_api_key_set: jupiter_api_key(&headers).is_some(),
        trigger_jwt_set: trigger_jwt_present(&headers, wallet_id),
        trigger_session,
        ladder,
        notes,
    }))
}

#[derive(Debug, Deserialize)]
pub struct DipRuntimeQuery {
    #[serde(default)]
    pub runtime_path: Option<String>,
    #[serde(default)]
    pub wallet_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DipTriggerSessionQuery {
    pub wallet_id: String,
}

/// `POST /dip/trigger/attach` — sign Trigger auth challenge with host keypair for `wallet_id`.
#[utoipa::path(
    post,
    path = "/dip/trigger/attach",
    tag = "Dip",
    request_body = DipTriggerAttachRequest,
    responses(
        (status = 200, description = "Trigger session attached (tokens stored server-side)", body = DipTriggerAttachResponse)
    )
)]
pub async fn post_dip_trigger_attach(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<DipTriggerAttachRequest>,
) -> ApiResult<Json<DipTriggerAttachResponse>> {
    let wallet_id = req.wallet_id.trim();
    if wallet_id.is_empty() {
        return Err(ApiError::bad_request("wallet_id required"));
    }
    let api_key = jupiter_api_key(&headers).ok_or_else(|| {
        ApiError::bad_request("JUPITER_API_KEY missing — set env or x-api-key header")
    })?;

    let kp = load_wallet_keypair_for_id(&state, wallet_id)?;
    let pubkey = kp.pubkey().to_string();
    let client = JupiterTriggerClient::new(Some(api_key));

    let challenge = client
        .request_challenge(&pubkey, "message")
        .await
        .map_err(|e| ApiError::bad_request(format!("trigger auth/challenge failed: {e}")))?;

    let signature = match challenge {
        TriggerChallenge::Message { challenge } => {
            kp.sign_message(challenge.as_bytes()).to_string()
        }
        TriggerChallenge::Transaction { .. } => {
            return Err(ApiError::bad_request(
                "transaction challenge not supported for host attach v1 — use type=message",
            ));
        }
    };

    let tokens = client
        .verify_challenge(&pubkey, &signature, "message")
        .await
        .map_err(|e| ApiError::bad_request(format!("trigger auth/verify failed: {e}")))?;

    if tokens.wallet_pubkey.trim() != pubkey {
        return Err(ApiError::internal(
            "verify returned wallet_pubkey mismatch vs host keypair",
        ));
    }

    let session = DipTriggerSession {
        wallet_id: wallet_id.to_string(),
        wallet_pubkey: pubkey,
        tokens,
        attached_at: Utc::now().to_rfc3339(),
    };
    let dir = resolve_trigger_session_dir(None);
    save_trigger_session(&dir, &session)
        .map_err(|e| ApiError::internal(format!("save trigger session: {e}")))?;

    Ok(Json(DipTriggerAttachResponse {
        session: DipTriggerSessionStatusDto {
            wallet_id: session.wallet_id,
            wallet_pubkey: session.wallet_pubkey,
            attached: true,
            expires_at: session.tokens.expires_at,
            attached_at: Some(session.attached_at),
        },
        notes: vec![
            "Trigger JWT stored under data/dip/trigger_sessions (not returned)".into(),
            "No create/cancel — attach is auth-only (E0)".into(),
        ],
    }))
}

/// `GET /dip/trigger/session` — public status for a wallet (no JWT).
#[utoipa::path(
    get,
    path = "/dip/trigger/session",
    tag = "Dip",
    params(
        ("wallet_id" = String, Query, description = "Host wallet id")
    ),
    responses(
        (status = 200, description = "Trigger session status", body = DipTriggerSessionStatusDto)
    )
)]
pub async fn get_dip_trigger_session(
    Query(q): Query<DipTriggerSessionQuery>,
) -> ApiResult<Json<DipTriggerSessionStatusDto>> {
    let wallet_id = q.wallet_id.trim();
    if wallet_id.is_empty() {
        return Err(ApiError::bad_request("wallet_id required"));
    }
    if let Some(dto) = session_status_dto(wallet_id) {
        return Ok(Json(dto));
    }
    Ok(Json(DipTriggerSessionStatusDto {
        wallet_id: wallet_id.to_string(),
        wallet_pubkey: String::new(),
        attached: false,
        expires_at: None,
        attached_at: None,
    }))
}

/// `DELETE /dip/trigger/session` — detach Trigger session for wallet.
#[utoipa::path(
    delete,
    path = "/dip/trigger/session",
    tag = "Dip",
    params(
        ("wallet_id" = String, Query, description = "Host wallet id")
    ),
    responses(
        (status = 200, description = "Detach result", body = DipTriggerSessionStatusDto)
    )
)]
pub async fn delete_dip_trigger_session(
    Query(q): Query<DipTriggerSessionQuery>,
) -> ApiResult<Json<DipTriggerSessionStatusDto>> {
    let wallet_id = q.wallet_id.trim();
    if wallet_id.is_empty() {
        return Err(ApiError::bad_request("wallet_id required"));
    }
    let dir = resolve_trigger_session_dir(None);
    let _ = delete_trigger_session(&dir, wallet_id)
        .map_err(|e| ApiError::internal(format!("delete trigger session: {e}")))?;
    Ok(Json(DipTriggerSessionStatusDto {
        wallet_id: wallet_id.to_string(),
        wallet_pubkey: String::new(),
        attached: false,
        expires_at: None,
        attached_at: None,
    }))
}

fn ladder_dto(l: &DipLadderDefaults) -> DipLadderDefaultsDto {
    DipLadderDefaultsDto {
        base_order_usd: l.base_order_usd,
        safety_order_usd: l.safety_order_usd,
        deviation_pct: l.deviation_pct,
        deviation_step_scale: l.deviation_step_scale,
        volume_scale: l.volume_scale,
        max_safety_orders: l.max_safety_orders,
        take_profit_pct: l.take_profit_pct,
        min_order_usd: l.min_order_usd,
    }
}

fn require_attached_session(wallet_id: &str) -> ApiResult<DipTriggerSession> {
    let wid = wallet_id.trim();
    if wid.is_empty() {
        return Err(ApiError::bad_request("wallet_id required"));
    }
    let dir = resolve_trigger_session_dir(None);
    load_trigger_session(&dir, wid)
        .map_err(|e| ApiError::internal(format!("load trigger session: {e}")))?
        .ok_or_else(|| {
            ApiError::bad_request(format!(
                "no Trigger session for wallet_id={wid} — POST /dip/trigger/attach first"
            ))
        })
}

#[derive(Debug, Deserialize)]
pub struct DipTriggerVaultQuery {
    pub wallet_id: String,
    /// When true, call `GET /vault/register` if `GET /vault` fails (T5).
    #[serde(default)]
    pub register_if_missing: bool,
}

/// `GET /dip/trigger/vault` — T4 vault read (+ optional T5 register).
#[utoipa::path(
    get,
    path = "/dip/trigger/vault",
    tag = "Dip",
    params(
        ("wallet_id" = String, Query, description = "Host wallet id with attached Trigger session"),
        ("register_if_missing" = Option<bool>, Query, description = "Register vault if GET /vault fails")
    ),
    responses(
        (status = 200, description = "Trigger vault", body = DipTriggerVaultResponse)
    )
)]
pub async fn get_dip_trigger_vault(
    Query(q): Query<DipTriggerVaultQuery>,
    headers: HeaderMap,
) -> ApiResult<Json<DipTriggerVaultResponse>> {
    let session = require_attached_session(&q.wallet_id)?;
    let api_key = jupiter_api_key(&headers).ok_or_else(|| {
        ApiError::bad_request("JUPITER_API_KEY missing — set env or x-api-key header")
    })?;
    let client = JupiterTriggerClient::new(Some(api_key));
    let mut notes = vec!["no create/cancel — vault read only (E1)".into()];
    let mut registered_now = false;

    let vault = if q.register_if_missing {
        match client.get_vault(&session.tokens.access_token).await {
            Ok(v) => v,
            Err(e) => {
                notes.push(format!("GET /vault failed ({e}); trying /vault/register"));
                registered_now = true;
                client
                    .register_vault(&session.tokens.access_token)
                    .await
                    .map_err(|reg| ApiError::bad_request(format!("vault/register failed: {reg}")))?
            }
        }
    } else {
        client
            .get_vault(&session.tokens.access_token)
            .await
            .map_err(|e| ApiError::bad_request(format!("vault failed: {e}")))?
    };

    Ok(Json(DipTriggerVaultResponse {
        wallet_id: session.wallet_id,
        wallet_pubkey: session.wallet_pubkey,
        vault_pubkey: vault.vault_pubkey,
        user_pubkey: vault.user_pubkey,
        privy_vault_id: vault.privy_vault_id,
        registered_now,
        notes,
    }))
}

/// `POST /dip/trigger/deposit/craft` — T6 propose unsigned deposit (no order create).
#[utoipa::path(
    post,
    path = "/dip/trigger/deposit/craft",
    tag = "Dip",
    request_body = DipTriggerDepositCraftRequest,
    responses(
        (status = 200, description = "Unsigned deposit craft", body = DipTriggerDepositCraftResponse)
    )
)]
pub async fn post_dip_trigger_deposit_craft(
    headers: HeaderMap,
    Json(req): Json<DipTriggerDepositCraftRequest>,
) -> ApiResult<Json<DipTriggerDepositCraftResponse>> {
    let session = require_attached_session(&req.wallet_id)?;
    let api_key = jupiter_api_key(&headers).ok_or_else(|| {
        ApiError::bad_request("JUPITER_API_KEY missing — set env or x-api-key header")
    })?;

    let order_type = req.order_type.trim();
    if order_type.is_empty() {
        return Err(ApiError::bad_request("order_type required"));
    }
    let order_sub_type = if order_type.eq_ignore_ascii_case("price") {
        Some(
            req.order_sub_type
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("single")
                .to_string(),
        )
    } else {
        req.order_sub_type
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };

    if req.input_mint.trim().is_empty() || req.output_mint.trim().is_empty() {
        return Err(ApiError::bad_request("input_mint and output_mint required"));
    }
    if req.amount.trim().is_empty() {
        return Err(ApiError::bad_request("amount required (smallest units)"));
    }

    let client = JupiterTriggerClient::new(Some(api_key));
    let body = TriggerDepositCraftRequest {
        input_mint: req.input_mint.trim().to_string(),
        output_mint: req.output_mint.trim().to_string(),
        user_address: session.wallet_pubkey.clone(),
        amount: req.amount.trim().to_string(),
        order_type: order_type.to_string(),
        order_sub_type,
    };
    let craft = client
        .craft_deposit(&session.tokens.access_token, &body)
        .await
        .map_err(|e| ApiError::bad_request(format!("deposit/craft failed: {e}")))?;

    Ok(Json(DipTriggerDepositCraftResponse {
        wallet_id: session.wallet_id,
        wallet_pubkey: session.wallet_pubkey,
        request_id: craft.request_id,
        transaction_base64: craft.transaction,
        receiver_address: craft.receiver_address,
        mint: craft.mint,
        amount: craft.amount,
        token_decimals: craft.token_decimals,
        notes: vec![
            "propose-only: unsigned deposit; sign + create via POST /dip/trigger/orders/price"
                .into(),
            "request_id is single-use as depositRequestId on create".into(),
        ],
    }))
}

/// Sign a Jupiter Trigger `VersionedTransaction` (deposit or cancel withdraw; base64 ↔ base64).
fn sign_trigger_versioned_tx_base64(tx_b64: &str, keypair: &Keypair) -> ApiResult<String> {
    let raw = BASE64
        .decode(tx_b64.trim().as_bytes())
        .map_err(|e| ApiError::bad_request(format!("trigger tx base64 decode: {e}")))?;
    let tx: VersionedTransaction = bincode::deserialize(&raw).map_err(|e| {
        ApiError::bad_request(format!(
            "trigger tx deserialize (expected VersionedTransaction): {e}"
        ))
    })?;
    let signed = VersionedTransaction::try_new(tx.message, &[keypair])
        .map_err(|e| ApiError::bad_request(format!("trigger tx sign failed: {e}")))?;
    let bytes = bincode::serialize(&signed)
        .map_err(|e| ApiError::internal(format!("trigger tx serialize: {e}")))?;
    Ok(BASE64.encode(bytes))
}

fn load_runtime_or_bad_request() -> ApiResult<DipRuntimeFile> {
    let path = resolve_runtime_path(None);
    load_dip_runtime_file(&path)
        .map_err(|e| ApiError::bad_request(format!("load dip runtime {}: {e}", path.display())))
}

fn require_live_execute_gate(confirm_execute: bool, action: &str) -> ApiResult<()> {
    if !confirm_execute {
        return Err(ApiError::bad_request(format!(
            "confirm_execute=true required for Trigger order {action}"
        )));
    }
    let rt = load_runtime_or_bad_request()?;
    let mode = normalize_dip_mode(&rt.mode);
    if mode != "live" {
        return Err(ApiError::bad_request(format!(
            "dip runtime mode is '{mode}' — set config/dip/runtime.yaml mode: live for {action}"
        )));
    }
    Ok(())
}

fn require_dca_allowed() -> ApiResult<()> {
    let rt = load_runtime_or_bad_request()?;
    if !rt.jupiter.dca_allowed {
        return Err(ApiError::bad_request(
            "jupiter.dca_allowed is false in config/dip/runtime.yaml",
        ));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct DipTriggerPriceOrderHistoryQuery {
    pub wallet_id: String,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    /// When true (default), keep only open/pending rows.
    #[serde(default = "default_true")]
    pub open_only: bool,
}

/// `GET /dip/trigger/orders/price` — T12 price order history with roles (read).
#[utoipa::path(
    get,
    path = "/dip/trigger/orders/price",
    tag = "Dip",
    params(
        ("wallet_id" = String, Query, description = "Host wallet id with attached Trigger session"),
        ("state" = Option<String>, Query, description = "Jupiter history state filter (default active)"),
        ("limit" = Option<u32>, Query, description = "Page size"),
        ("open_only" = Option<bool>, Query, description = "If true (default), return open/pending only")
    ),
    responses(
        (status = 200, description = "Trigger price order history", body = DipTriggerPriceOrderHistoryResponse)
    )
)]
pub async fn get_dip_trigger_orders_price(
    Query(q): Query<DipTriggerPriceOrderHistoryQuery>,
    headers: HeaderMap,
) -> ApiResult<Json<DipTriggerPriceOrderHistoryResponse>> {
    let session = require_attached_session(&q.wallet_id)?;
    let api_key = jupiter_api_key(&headers).ok_or_else(|| {
        ApiError::bad_request("JUPITER_API_KEY missing — set env or x-api-key header")
    })?;
    let client = JupiterTriggerClient::new(Some(api_key));
    let hist = client
        .get_order_history(
            &session.tokens.access_token,
            &OrderHistoryQuery {
                state: q.state.or_else(|| Some("active".into())),
                limit: q.limit.or(Some(50)),
                ..Default::default()
            },
        )
        .await
        .map_err(|e| ApiError::bad_request(format!("orders/history failed: {e}")))?;

    let rows = if q.open_only {
        filter_open_price_orders(&hist.orders)
    } else {
        hist.orders
    };

    Ok(Json(DipTriggerPriceOrderHistoryResponse {
        wallet_id: session.wallet_id,
        wallet_pubkey: session.wallet_pubkey,
        orders: rows.into_iter().map(map_trigger_open_order).collect(),
        notes: vec![
            "T12 read: roles from mint orientation (buy_dip/take_profit/stop_loss/buy_above)"
                .into(),
        ],
    }))
}

/// `POST /dip/trigger/orders/price` — E2 T7/T8/T9: craft + sign deposit + create single price order.
#[utoipa::path(
    post,
    path = "/dip/trigger/orders/price",
    tag = "Dip",
    request_body = DipTriggerPriceOrderCreateRequest,
    responses(
        (status = 200, description = "Created Trigger price order", body = DipTriggerPriceOrderCreateResponse)
    )
)]
pub async fn post_dip_trigger_orders_price(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<DipTriggerPriceOrderCreateRequest>,
) -> ApiResult<Json<DipTriggerPriceOrderCreateResponse>> {
    require_live_execute_gate(req.confirm_execute, "create")?;
    let session = require_attached_session(&req.wallet_id)?;
    let api_key = jupiter_api_key(&headers).ok_or_else(|| {
        ApiError::bad_request("JUPITER_API_KEY missing — set env or x-api-key header")
    })?;

    let cond = req.trigger_condition.trim().to_ascii_lowercase();
    if cond != "below" && cond != "above" {
        return Err(ApiError::bad_request(
            "trigger_condition must be 'below' (buy_dip/stop_loss) or 'above' (take_profit/buy_above)",
        ));
    }
    if req.input_mint.trim().is_empty()
        || req.output_mint.trim().is_empty()
        || req.trigger_mint.trim().is_empty()
    {
        return Err(ApiError::bad_request(
            "input_mint, output_mint, trigger_mint required",
        ));
    }
    let input_mint = req.input_mint.trim().to_string();
    let output_mint = req.output_mint.trim().to_string();
    let trigger_mint = req.trigger_mint.trim().to_string();
    if trigger_mint != input_mint && trigger_mint != output_mint {
        return Err(ApiError::bad_request(
            "trigger_mint must equal input_mint (sell-side TP/SL) or output_mint (buy_dip/buy_above)",
        ));
    }
    let role = classify_price_order_role(&cond, &input_mint, &output_mint, &trigger_mint);
    if req.amount.trim().is_empty() {
        return Err(ApiError::bad_request("amount required (smallest units)"));
    }
    if !req.trigger_price_usd.is_finite() || req.trigger_price_usd <= 0.0 {
        return Err(ApiError::bad_request("trigger_price_usd must be > 0"));
    }

    let expires_at = req
        .expires_at_ms
        .unwrap_or_else(|| Utc::now().timestamp_millis() + 7 * 24 * 60 * 60 * 1000);
    if expires_at <= Utc::now().timestamp_millis() {
        return Err(ApiError::bad_request("expires_at_ms must be in the future"));
    }

    let kp = load_wallet_keypair_for_id(&state, req.wallet_id.trim())?;
    if kp.pubkey().to_string() != session.wallet_pubkey {
        return Err(ApiError::bad_request(
            "host keypair pubkey mismatch vs Trigger session wallet_pubkey",
        ));
    }

    let client = JupiterTriggerClient::new(Some(api_key));
    let craft = client
        .craft_deposit(
            &session.tokens.access_token,
            &TriggerDepositCraftRequest {
                input_mint: input_mint.clone(),
                output_mint: output_mint.clone(),
                user_address: session.wallet_pubkey.clone(),
                amount: req.amount.trim().to_string(),
                order_type: "price".into(),
                order_sub_type: Some("single".into()),
            },
        )
        .await
        .map_err(|e| ApiError::bad_request(format!("deposit/craft failed: {e}")))?;

    let signed_tx = sign_trigger_versioned_tx_base64(&craft.transaction, &kp)?;
    let created = client
        .create_price_order(
            &session.tokens.access_token,
            &TriggerPriceOrderCreateRequest {
                order_type: "single".into(),
                deposit_request_id: craft.request_id.clone(),
                deposit_signed_tx: signed_tx,
                user_pubkey: session.wallet_pubkey.clone(),
                input_mint: input_mint.clone(),
                output_mint: output_mint.clone(),
                input_amount: req.amount.trim().to_string(),
                trigger_mint: trigger_mint.clone(),
                trigger_condition: cond.clone(),
                trigger_price_usd: req.trigger_price_usd,
                expires_at,
                slippage_bps: req.slippage_bps,
            },
        )
        .await
        .map_err(|e| ApiError::bad_request(format!("orders/price create failed: {e}")))?;

    Ok(Json(DipTriggerPriceOrderCreateResponse {
        wallet_id: session.wallet_id,
        wallet_pubkey: session.wallet_pubkey,
        order_id: created.id,
        tx_signature: created.tx_signature,
        deposit_confirmed: created.deposit_confirmed,
        deposit_request_id: craft.request_id,
        trigger_condition: cond,
        trigger_price_usd: req.trigger_price_usd,
        role: role.as_str().to_string(),
        notes: vec![
            format!(
                "live create role={}: deposit signed with host wallet_id keypair",
                role.as_str()
            ),
            "static stop_loss = below + trigger_mint==input_mint (trailingBps deny)".into(),
            "cancel via POST /dip/trigger/orders/price/cancel".into(),
        ],
    }))
}

/// `POST /dip/trigger/orders/price/cancel` — E3 T15–T16: cancel start + sign withdraw + confirm.
#[utoipa::path(
    post,
    path = "/dip/trigger/orders/price/cancel",
    tag = "Dip",
    request_body = DipTriggerPriceOrderCancelRequest,
    responses(
        (status = 200, description = "Cancelled Trigger price order + withdraw", body = DipTriggerPriceOrderCancelResponse)
    )
)]
pub async fn post_dip_trigger_orders_price_cancel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<DipTriggerPriceOrderCancelRequest>,
) -> ApiResult<Json<DipTriggerPriceOrderCancelResponse>> {
    require_live_execute_gate(req.confirm_execute, "cancel")?;
    let session = require_attached_session(&req.wallet_id)?;
    let api_key = jupiter_api_key(&headers).ok_or_else(|| {
        ApiError::bad_request("JUPITER_API_KEY missing — set env or x-api-key header")
    })?;
    let order_id = req.order_id.trim();
    if order_id.is_empty() {
        return Err(ApiError::bad_request("order_id required"));
    }

    let kp = load_wallet_keypair_for_id(&state, req.wallet_id.trim())?;
    if kp.pubkey().to_string() != session.wallet_pubkey {
        return Err(ApiError::bad_request(
            "host keypair pubkey mismatch vs Trigger session wallet_pubkey",
        ));
    }

    let client = JupiterTriggerClient::new(Some(api_key));
    let cancel = client
        .cancel_price_order(&session.tokens.access_token, order_id)
        .await
        .map_err(|e| ApiError::bad_request(format!("orders/price/cancel failed: {e}")))?;

    let signed_tx = sign_trigger_versioned_tx_base64(&cancel.transaction, &kp)?;
    let confirmed = client
        .confirm_cancel_price_order(
            &session.tokens.access_token,
            order_id,
            &TriggerPriceOrderConfirmCancelRequest {
                signed_transaction: signed_tx,
                cancel_request_id: cancel.request_id.clone(),
            },
        )
        .await
        .map_err(|e| ApiError::bad_request(format!("orders/price/confirm-cancel failed: {e}")))?;

    Ok(Json(DipTriggerPriceOrderCancelResponse {
        wallet_id: session.wallet_id,
        wallet_pubkey: session.wallet_pubkey,
        order_id: confirmed.id,
        cancel_request_id: cancel.request_id,
        tx_signature: confirmed.tx_signature,
        notes: vec![
            "E3 live cancel: withdraw signed with host wallet_id keypair".into(),
            "order moved ready_to_cancel then cancelled after withdraw lands".into(),
        ],
    }))
}

/// `POST /dip/trigger/orders/price/patch` — E4 T14: patch single order trigger/slippage.
#[utoipa::path(
    post,
    path = "/dip/trigger/orders/price/patch",
    tag = "Dip",
    request_body = DipTriggerPriceOrderPatchRequest,
    responses(
        (status = 200, description = "Patched Trigger price order", body = DipTriggerPriceOrderPatchResponse)
    )
)]
pub async fn post_dip_trigger_orders_price_patch(
    headers: HeaderMap,
    Json(req): Json<DipTriggerPriceOrderPatchRequest>,
) -> ApiResult<Json<DipTriggerPriceOrderPatchResponse>> {
    require_live_execute_gate(req.confirm_execute, "patch")?;
    let session = require_attached_session(&req.wallet_id)?;
    let api_key = jupiter_api_key(&headers).ok_or_else(|| {
        ApiError::bad_request("JUPITER_API_KEY missing — set env or x-api-key header")
    })?;
    let order_id = req.order_id.trim();
    if order_id.is_empty() {
        return Err(ApiError::bad_request("order_id required"));
    }
    if req.trigger_price_usd.is_none() && req.slippage_bps.is_none() {
        return Err(ApiError::bad_request(
            "at least one of trigger_price_usd or slippage_bps required",
        ));
    }
    if let Some(px) = req.trigger_price_usd
        && (!px.is_finite() || px <= 0.0)
    {
        return Err(ApiError::bad_request("trigger_price_usd must be > 0"));
    }
    if let Some(bps) = req.slippage_bps
        && bps > 10_000
    {
        return Err(ApiError::bad_request("slippage_bps must be 0..=10000"));
    }

    let client = JupiterTriggerClient::new(Some(api_key));
    let patched = client
        .patch_price_order(
            &session.tokens.access_token,
            order_id,
            &TriggerPriceOrderPatchRequest {
                order_type: "single".into(),
                trigger_price_usd: req.trigger_price_usd,
                slippage_bps: req.slippage_bps,
            },
        )
        .await
        .map_err(|e| ApiError::bad_request(format!("orders/price PATCH failed: {e}")))?;

    Ok(Json(DipTriggerPriceOrderPatchResponse {
        wallet_id: session.wallet_id,
        wallet_pubkey: session.wallet_pubkey,
        order_id: patched.id,
        trigger_price_usd: req.trigger_price_usd,
        slippage_bps: req.slippage_bps,
        notes: vec![
            "E4 live patch: single order only (no OCO/trailing)".into(),
            "JWT from attached wallet_id session".into(),
        ],
    }))
}

#[derive(Debug, Deserialize)]
pub struct DipTriggerDcaHistoryQuery {
    pub wallet_id: String,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

/// `GET /dip/trigger/orders/dca` — E5 T18 DCA history for attached wallet.
#[utoipa::path(
    get,
    path = "/dip/trigger/orders/dca",
    tag = "Dip",
    params(
        ("wallet_id" = String, Query, description = "Host wallet id with attached Trigger session"),
        ("state" = Option<String>, Query, description = "Optional Jupiter state filter"),
        ("limit" = Option<u32>, Query, description = "Page size")
    ),
    responses(
        (status = 200, description = "DCA order history", body = DipTriggerDcaHistoryResponse)
    )
)]
pub async fn get_dip_trigger_orders_dca(
    Query(q): Query<DipTriggerDcaHistoryQuery>,
    headers: HeaderMap,
) -> ApiResult<Json<DipTriggerDcaHistoryResponse>> {
    require_dca_allowed()?;
    let session = require_attached_session(&q.wallet_id)?;
    let api_key = jupiter_api_key(&headers).ok_or_else(|| {
        ApiError::bad_request("JUPITER_API_KEY missing — set env or x-api-key header")
    })?;
    let client = JupiterTriggerClient::new(Some(api_key));
    let hist = client
        .get_dca_order_history(
            &session.tokens.access_token,
            &OrderHistoryQuery {
                state: q.state,
                limit: q.limit,
                ..Default::default()
            },
        )
        .await
        .map_err(|e| ApiError::bad_request(format!("orders/history/dca failed: {e}")))?;

    Ok(Json(DipTriggerDcaHistoryResponse {
        wallet_id: session.wallet_id,
        wallet_pubkey: session.wallet_pubkey,
        orders: hist
            .orders
            .into_iter()
            .map(|o| DipTriggerDcaOrder {
                id: o.id,
                order_state: o.order_state,
                order_type: o.order_type,
                input_mint: o.input_mint,
                output_mint: o.output_mint,
                input_amount: o.input_amount,
                order_count: o.order_count,
                interval_seconds: o.interval_seconds,
                rounds_remaining: o.rounds_remaining,
            })
            .collect(),
        notes: vec!["E5 read: DCA history (T18)".into()],
    }))
}

/// `POST /dip/trigger/orders/dca` — E5 T17: craft+sign+create time_based DCA.
#[utoipa::path(
    post,
    path = "/dip/trigger/orders/dca",
    tag = "Dip",
    request_body = DipTriggerDcaOrderCreateRequest,
    responses(
        (status = 200, description = "Created DCA order", body = DipTriggerDcaOrderCreateResponse)
    )
)]
pub async fn post_dip_trigger_orders_dca(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<DipTriggerDcaOrderCreateRequest>,
) -> ApiResult<Json<DipTriggerDcaOrderCreateResponse>> {
    require_dca_allowed()?;
    require_live_execute_gate(req.confirm_execute, "dca create")?;
    let session = require_attached_session(&req.wallet_id)?;
    let api_key = jupiter_api_key(&headers).ok_or_else(|| {
        ApiError::bad_request("JUPITER_API_KEY missing — set env or x-api-key header")
    })?;
    if req.input_mint.trim().is_empty() || req.output_mint.trim().is_empty() {
        return Err(ApiError::bad_request("input_mint and output_mint required"));
    }
    if req.amount.trim().is_empty() {
        return Err(ApiError::bad_request("amount required (smallest units)"));
    }
    if req.order_count < 2 {
        return Err(ApiError::bad_request("order_count must be >= 2"));
    }
    if !(60..=31_536_000).contains(&req.interval_seconds) {
        return Err(ApiError::bad_request(
            "interval_seconds must be 60..=31536000",
        ));
    }

    let kp = load_wallet_keypair_for_id(&state, req.wallet_id.trim())?;
    if kp.pubkey().to_string() != session.wallet_pubkey {
        return Err(ApiError::bad_request(
            "host keypair pubkey mismatch vs Trigger session wallet_pubkey",
        ));
    }

    let client = JupiterTriggerClient::new(Some(api_key));
    let craft = client
        .craft_deposit(
            &session.tokens.access_token,
            &TriggerDepositCraftRequest {
                input_mint: req.input_mint.trim().to_string(),
                output_mint: req.output_mint.trim().to_string(),
                user_address: session.wallet_pubkey.clone(),
                amount: req.amount.trim().to_string(),
                order_type: "dca".into(),
                order_sub_type: None,
            },
        )
        .await
        .map_err(|e| ApiError::bad_request(format!("deposit/craft (dca) failed: {e}")))?;

    let signed_tx = sign_trigger_versioned_tx_base64(&craft.transaction, &kp)?;
    let created = client
        .create_dca_order(
            &session.tokens.access_token,
            &TriggerDcaOrderCreateRequest {
                deposit_request_id: craft.request_id.clone(),
                deposit_signed_tx: signed_tx,
                user_pubkey: session.wallet_pubkey.clone(),
                input_mint: req.input_mint.trim().to_string(),
                output_mint: req.output_mint.trim().to_string(),
                input_amount: req.amount.trim().to_string(),
                order_count: req.order_count,
                interval_seconds: req.interval_seconds,
                order_type: "time_based".into(),
                min_price_usd: None,
                max_price_usd: None,
                trigger_mint: None,
            },
        )
        .await
        .map_err(|e| ApiError::bad_request(format!("orders/dca create failed: {e}")))?;

    Ok(Json(DipTriggerDcaOrderCreateResponse {
        wallet_id: session.wallet_id,
        wallet_pubkey: session.wallet_pubkey,
        order_id: created.id,
        tx_signature: created.tx_signature,
        deposit_request_id: craft.request_id,
        order_count: req.order_count,
        interval_seconds: req.interval_seconds,
        notes: vec![
            "E5 live DCA create: time_based only (price_conditional later)".into(),
            "requires jupiter.dca_allowed + mode:live + confirm_execute".into(),
        ],
    }))
}

/// `POST /dip/trigger/orders/dca/cancel` — E5 T19: cancel + sign withdraw + confirm.
#[utoipa::path(
    post,
    path = "/dip/trigger/orders/dca/cancel",
    tag = "Dip",
    request_body = DipTriggerDcaOrderCancelRequest,
    responses(
        (status = 200, description = "Cancelled DCA + withdraw", body = DipTriggerDcaOrderCancelResponse)
    )
)]
pub async fn post_dip_trigger_orders_dca_cancel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<DipTriggerDcaOrderCancelRequest>,
) -> ApiResult<Json<DipTriggerDcaOrderCancelResponse>> {
    require_dca_allowed()?;
    require_live_execute_gate(req.confirm_execute, "dca cancel")?;
    let session = require_attached_session(&req.wallet_id)?;
    let api_key = jupiter_api_key(&headers).ok_or_else(|| {
        ApiError::bad_request("JUPITER_API_KEY missing — set env or x-api-key header")
    })?;
    let order_id = req.order_id.trim();
    if order_id.is_empty() {
        return Err(ApiError::bad_request("order_id required"));
    }

    let kp = load_wallet_keypair_for_id(&state, req.wallet_id.trim())?;
    if kp.pubkey().to_string() != session.wallet_pubkey {
        return Err(ApiError::bad_request(
            "host keypair pubkey mismatch vs Trigger session wallet_pubkey",
        ));
    }

    let client = JupiterTriggerClient::new(Some(api_key));
    let cancel = client
        .cancel_dca_order(&session.tokens.access_token, order_id)
        .await
        .map_err(|e| ApiError::bad_request(format!("orders/dca/cancel failed: {e}")))?;
    let signed_tx = sign_trigger_versioned_tx_base64(&cancel.transaction, &kp)?;
    let confirmed = client
        .confirm_cancel_dca_order(
            &session.tokens.access_token,
            order_id,
            &TriggerPriceOrderConfirmCancelRequest {
                signed_transaction: signed_tx,
                cancel_request_id: cancel.request_id.clone(),
            },
        )
        .await
        .map_err(|e| ApiError::bad_request(format!("orders/dca/confirm-cancel failed: {e}")))?;

    Ok(Json(DipTriggerDcaOrderCancelResponse {
        wallet_id: session.wallet_id,
        wallet_pubkey: session.wallet_pubkey,
        order_id: confirmed.id,
        cancel_request_id: cancel.request_id,
        tx_signature: confirmed.tx_signature,
        refund_amount: cancel.refund_amount,
        notes: vec!["E5 live DCA cancel: withdraw signed with host wallet".into()],
    }))
}

/// `POST /dip/propose` — append `kind=dip_propose` row to `agent_decisions.jsonl` (no Trigger write).
#[utoipa::path(
    post,
    path = "/dip/propose",
    tag = "Dip",
    request_body = DipProposeRequest,
    responses(
        (status = 200, description = "Dip propose appended", body = DipProposeResponse)
    )
)]
pub async fn post_dip_propose(
    Json(req): Json<DipProposeRequest>,
) -> ApiResult<Json<DipProposeResponse>> {
    let action =
        DipProposeAction::parse(&req.action).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let mode =
        DipProposeMode::parse(&req.mode).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let mut decision = DipProposeDecision::new(action, mode, req.rationale, req.refs)
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    decision.zone = req
        .zone
        .map(|z| z.trim().to_string())
        .filter(|s| !s.is_empty());
    decision.sol_usd = req.sol_usd;
    decision.proposed_orders = req.proposed_orders;
    decision
        .validate()
        .map_err(|e| ApiError::bad_request(e.to_string()))?;

    let path = req
        .path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(default_agent_decisions_path);

    let row = DipAuditDecisionRow {
        ts_utc: String::new(),
        source: clmm_lp_data::dip::DIP_PROPOSE_SOURCE.to_string(),
        strategy_id: req
            .strategy_id
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        position_id: req
            .position_id
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        chain_id: None,
        session_id: req
            .session_id
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        decision,
    };
    let written = append_dip_propose(&path, row).map_err(|e| ApiError::internal(e.to_string()))?;

    let api_row = AgentDecisionRow {
        ts_utc: written.ts_utc.clone(),
        source: written.source.clone(),
        strategy_id: written.strategy_id.clone(),
        position_id: written.position_id.clone(),
        chain_id: written.chain_id.clone(),
        session_id: written.session_id.clone(),
        decision: serde_json::to_value(&written.decision)
            .map_err(|e| ApiError::internal(format!("serialize dip decision: {e}")))?,
    };

    Ok(Json(DipProposeResponse {
        path: path.to_string_lossy().to_string(),
        written: true,
        row: api_row,
    }))
}
