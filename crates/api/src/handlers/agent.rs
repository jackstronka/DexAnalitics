//! Per-position agent chat handlers (MVP).

use crate::error::{ApiError, ApiResult};
use crate::models::{
    AgentChatMessage, AgentChatResponse, AgentChatUiPayload, AgentLlmReplyRequest,
    AgentLlmReplyResponse, AgentMessageRequest, AgentPositionSupervisorResponse, AgentScanRequest,
    AgentScanResponse, AgentSessionRequest, AgentSessionResponse, AgentSupervisorScenario,
    AgentWorkerSettings, AgentWorkerSettingsUpdateRequest, AgentWorkerStatus,
};
use crate::services::position_stream_lineage::compute_position_stream_lineage;
use crate::services::position_stream_pnl::compute_position_stream_pnl;
use crate::services::position_valuation::{
    compute_position_usd_valuation, fetch_prices_for_positions, monitored_position_from_chain,
};
use crate::services::position_agent_llm;
use crate::services::position_agent_service;
use crate::state::AppState;
use axum::{
    Json,
    extract::{Path, State},
};
use solana_sdk::pubkey::Pubkey;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use std::str::FromStr;

fn validate_position(address: &str) -> ApiResult<()> {
    Pubkey::from_str(address.trim()).map_err(|_| ApiError::bad_request("Invalid position address"))?;
    Ok(())
}

#[utoipa::path(
    get,
    path = "/positions/{address}/agent-chat",
    tag = "Agent",
    params(
        ("address" = String, Path, description = "Position PDA (base58)")
    ),
    responses(
        (status = 200, description = "Agent session + chat history", body = AgentChatResponse),
        (status = 400, description = "Invalid address")
    )
)]
pub async fn get_position_agent_chat(
    State(_state): State<AppState>,
    Path(address): Path<String>,
) -> ApiResult<Json<AgentChatResponse>> {
    let pos = address.trim();
    validate_position(pos)?;
    let (session, messages) = position_agent_service::list_chat(pos)?;
    Ok(Json(AgentChatResponse {
        position_address: pos.to_string(),
        session,
        messages,
    }))
}

#[utoipa::path(
    get,
    path = "/positions/{address}/agent-chat/ui",
    tag = "Agent",
    params(
        ("address" = String, Path, description = "Position PDA (base58)")
    ),
    responses(
        (status = 200, description = "UI payload for position agent tab", body = AgentChatUiPayload),
        (status = 400, description = "Invalid address")
    )
)]
pub async fn get_position_agent_chat_ui(
    State(_state): State<AppState>,
    Path(address): Path<String>,
) -> ApiResult<Json<AgentChatUiPayload>> {
    let pos = address.trim();
    validate_position(pos)?;
    let (session, messages) = position_agent_service::list_chat(pos)?;
    Ok(Json(AgentChatUiPayload {
        position_address: pos.to_string(),
        session,
        messages,
        quick_actions: vec![
            "scan_now".to_string(),
            "compare_7d_ranges".to_string(),
            "compare_30d_ranges".to_string(),
            "cross_pair_scan".to_string(),
        ],
        suggested_prompts: vec![
            "Porownaj moj obecny range z top 3 zakresami z ostatnich 7 dni.".to_string(),
            "Pokaz wariant conservative/balanced/aggressive dla mojego kapitalu.".to_string(),
            "Czy warto teraz zwezic range czy zostac przy obecnym?".to_string(),
        ],
    }))
}

#[utoipa::path(
    post,
    path = "/positions/{address}/agent/start",
    tag = "Agent",
    params(
        ("address" = String, Path, description = "Position PDA (base58)")
    ),
    request_body = AgentSessionRequest,
    responses(
        (status = 200, description = "Agent supervision started/ensured", body = AgentSessionResponse),
        (status = 400, description = "Invalid address")
    )
)]
pub async fn start_position_agent(
    State(_state): State<AppState>,
    Path(address): Path<String>,
    Json(request): Json<AgentSessionRequest>,
) -> ApiResult<Json<AgentSessionResponse>> {
    let pos = address.trim();
    validate_position(pos)?;
    let session = position_agent_service::get_or_create_session(pos, request.scan_interval_hours)?;
    let _ = position_agent_service::append_message(
        pos,
        "agent",
        "info",
        "Nadzor agenta uruchomiony dla tej pozycji. Bede monitorowac range i podpowiadac alternatywy.".to_string(),
    )?;
    Ok(Json(AgentSessionResponse { session }))
}

#[utoipa::path(
    post,
    path = "/positions/{address}/agent/message",
    tag = "Agent",
    params(
        ("address" = String, Path, description = "Position PDA (base58)")
    ),
    request_body = AgentMessageRequest,
    responses(
        (status = 200, description = "Stored user+agent messages", body = AgentChatMessage),
        (status = 400, description = "Invalid request")
    )
)]
pub async fn post_position_agent_message(
    State(_state): State<AppState>,
    Path(address): Path<String>,
    Json(request): Json<AgentMessageRequest>,
) -> ApiResult<Json<AgentChatMessage>> {
    let pos = address.trim();
    validate_position(pos)?;
    let prompt = request.content.trim();
    if prompt.is_empty() {
        return Err(ApiError::bad_request("Message content cannot be empty"));
    }
    position_agent_service::get_or_create_session(pos, None)?;
    let _ = position_agent_service::append_message(pos, "user", "question", prompt.to_string())?;
    let (reply, _meta) = position_agent_llm::generate_agent_reply(pos, prompt, None).await?;
    let msg = position_agent_service::append_message(pos, "agent", "insight", reply)?;
    Ok(Json(msg))
}

#[utoipa::path(
    post,
    path = "/positions/{address}/agent/llm-reply",
    tag = "Agent",
    params(
        ("address" = String, Path, description = "Position PDA (base58)")
    ),
    request_body = AgentLlmReplyRequest,
    responses(
        (status = 200, description = "Generated answer persisted in chat", body = AgentLlmReplyResponse),
        (status = 400, description = "Invalid request")
    )
)]
pub async fn post_position_agent_llm_reply(
    State(_state): State<AppState>,
    Path(address): Path<String>,
    Json(request): Json<AgentLlmReplyRequest>,
) -> ApiResult<Json<AgentLlmReplyResponse>> {
    let pos = address.trim();
    validate_position(pos)?;
    let prompt = request.prompt.trim();
    if prompt.is_empty() {
        return Err(ApiError::bad_request("prompt cannot be empty"));
    }
    position_agent_service::get_or_create_session(pos, None)?;
    let _ = position_agent_service::append_message(pos, "user", "question", prompt.to_string())?;
    let (reply, meta) =
        position_agent_llm::generate_agent_reply(pos, prompt, request.context.as_ref()).await?;
    let message = position_agent_service::append_message(pos, "agent", "insight", reply)?;
    Ok(Json(AgentLlmReplyResponse {
        position_address: pos.to_string(),
        message,
        meta,
    }))
}

#[utoipa::path(
    post,
    path = "/positions/{address}/agent/scan-now",
    tag = "Agent",
    params(
        ("address" = String, Path, description = "Position PDA (base58)")
    ),
    request_body = AgentScanRequest,
    responses(
        (status = 200, description = "Scan recommendations", body = AgentScanResponse),
        (status = 400, description = "Invalid request")
    )
)]
pub async fn trigger_position_agent_scan(
    State(_state): State<AppState>,
    Path(address): Path<String>,
    Json(request): Json<AgentScanRequest>,
) -> ApiResult<Json<AgentScanResponse>> {
    let pos = address.trim();
    validate_position(pos)?;
    position_agent_service::get_or_create_session(pos, None)?;
    let scan = position_agent_service::scan_recommendations(pos, request.include_cross_pair_scan)?;
    for rec in &scan.recommendations {
        let _ = position_agent_service::append_message(pos, "agent", "insight", rec.clone())?;
    }
    Ok(Json(scan))
}

#[utoipa::path(
    get,
    path = "/positions/{address}/agent/supervisor",
    tag = "Agent",
    params(
        ("address" = String, Path, description = "Position PDA (base58)")
    ),
    responses(
        (status = 200, description = "Cost/profit supervision snapshot + scenarios", body = AgentPositionSupervisorResponse),
        (status = 400, description = "Invalid address")
    )
)]
pub async fn get_position_agent_supervisor(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> ApiResult<Json<AgentPositionSupervisorResponse>> {
    let pos = address.trim();
    validate_position(pos)?;
    let stream_pnl = compute_position_stream_pnl(&state, pos).await?;
    let lineage = compute_position_stream_lineage(&state, pos).await?;

    let live_value = {
        let pubkey = Pubkey::from_str(pos).map_err(|_| ApiError::bad_request("Invalid position address"))?;
        let from_monitor = state
            .monitor
            .get_positions()
            .await
            .into_iter()
            .find(|p| p.address == pubkey);
        let p = if let Some(existing) = from_monitor {
            existing
        } else {
            monitored_position_from_chain(state.provider.clone(), &pubkey).await?
        };
        let prices = fetch_prices_for_positions(state.provider.clone(), std::slice::from_ref(&p)).await;
        compute_position_usd_valuation(state.provider.clone(), &p, &prices)
            .await
            .ok()
            .map(|v| v.value_usd)
            .unwrap_or(stream_pnl.current_value_usd)
    };

    let entry = stream_pnl.baseline_value_usd.max(Decimal::ZERO);
    let costs = stream_pnl.tx_fees_usd.max(Decimal::ZERO);
    let earnings = stream_pnl
        .realized_cashflow_usd
        .max(Decimal::ZERO)
        + lineage
            .chain_cost_summary
            .as_ref()
            .map(|x| x.fees_collected_usd_total.max(Decimal::ZERO))
            .unwrap_or(Decimal::ZERO);
    let net = live_value + earnings - entry - costs;
    let net_pct = if entry > Decimal::ZERO {
        (net / entry) * Decimal::from(100u8)
    } else {
        Decimal::ZERO
    };

    let elapsed_hours = stream_pnl.baseline_ts_utc.as_deref().and_then(|ts| {
        chrono::DateTime::parse_from_rfc3339(ts)
            .ok()
            .map(|x| (chrono::Utc::now() - x.with_timezone(&chrono::Utc)).num_hours())
    });
    let rebalance_count = lineage
        .chain
        .len()
        .saturating_sub(1)
        .try_into()
        .unwrap_or(0u64);

    let (entry_token_a_ui, entry_token_b_ui) = (None, None);
    let (entry_token_a_label, entry_token_b_label) =
        lineage.nodes.first().map_or((None, None), |node| {
            (node.token_a_label.clone(), node.token_b_label.clone())
        });

    let net_pct_f = net_pct.to_f64().unwrap_or(0.0);
    let scenarios = vec![
        AgentSupervisorScenario {
            scenario: "bullish".to_string(),
            expectation: "Cena wybija wyzej; pozycja moze szybciej dojsc do gornej granicy range.".to_string(),
            suggested_action: if net_pct_f < 0.0 {
                "Ogranicz dalsze straty: rozwaz recenter range wyzej i pilnuj kosztu kolejnego rebalance vs potencjal fee.".to_string()
            } else {
                "Bron zysk: rozwaz przesuniecie range wyzej dopiero gdy prog fee > koszt transakcji i spread slippage.".to_string()
            },
        },
        AgentSupervisorScenario {
            scenario: "bearish".to_string(),
            expectation: "Cena spada; ryzyko wyjscia dolem i spadku aktywnej ekspozycji fee.".to_string(),
            suggested_action: if net_pct_f < 0.0 {
                "Priorytet defensywny: jesli strata rosnie szybciej niz fee, rozwaz zawieszenie aktywnej rotacji lub waski reset nizej.".to_string()
            } else {
                "Chroń kapital: utrzymuj szerszy bufor dolny i recenter wykonuj tylko przy jasnym przewadze ekonomicznej.".to_string()
            },
        },
        AgentSupervisorScenario {
            scenario: "sideways".to_string(),
            expectation: "Ruch boczny; potencjal stabilnego fee przy umiarkowanej liczbie rebalance.".to_string(),
            suggested_action: "W trybie bocznym preferuj mniejsza liczbe interwencji: porownaj narrow vs mid range i wybierz wariant z lepszym fee/net po kosztach.".to_string(),
        },
    ];

    Ok(Json(AgentPositionSupervisorResponse {
        position_address: pos.to_string(),
        entry_capital_usd: entry,
        current_value_usd: live_value,
        earnings_total_usd: earnings,
        costs_total_usd: costs,
        net_since_entry_usd: net,
        net_since_entry_pct: net_pct.round_dp(4),
        rebalance_count,
        elapsed_hours,
        entry_token_a_ui,
        entry_token_b_ui,
        entry_token_a_label,
        entry_token_b_label,
        scenarios,
        note: Some(
            "MVP: entry token leg amounts are best-effort and chain-level economics use stream scopes."
                .to_string(),
        ),
    }))
}

#[utoipa::path(
    get,
    path = "/agent/worker/settings",
    tag = "Agent",
    responses(
        (status = 200, description = "Global agent worker settings", body = AgentWorkerSettings)
    )
)]
pub async fn get_agent_worker_settings(
    State(_state): State<AppState>,
) -> ApiResult<Json<AgentWorkerSettings>> {
    Ok(Json(position_agent_service::load_worker_settings()?))
}

#[utoipa::path(
    put,
    path = "/agent/worker/settings",
    tag = "Agent",
    request_body = AgentWorkerSettingsUpdateRequest,
    responses(
        (status = 200, description = "Updated global worker settings", body = AgentWorkerSettings),
        (status = 400, description = "Invalid settings")
    )
)]
pub async fn put_agent_worker_settings(
    State(_state): State<AppState>,
    Json(req): Json<AgentWorkerSettingsUpdateRequest>,
) -> ApiResult<Json<AgentWorkerSettings>> {
    let mut current = position_agent_service::load_worker_settings()?;
    if let Some(v) = req.enabled {
        current.enabled = v;
    }
    if let Some(v) = req.default_position_scan_interval_hours {
        if v == 0 {
            return Err(ApiError::bad_request(
                "default_position_scan_interval_hours must be >= 1",
            ));
        }
        current.default_position_scan_interval_hours = v;
    }
    if let Some(v) = req.cross_pair_scan_interval_hours {
        if v == 0 {
            return Err(ApiError::bad_request(
                "cross_pair_scan_interval_hours must be >= 1",
            ));
        }
        current.cross_pair_scan_interval_hours = v;
    }
    if let Some(v) = req.include_cross_pair_scan {
        current.include_cross_pair_scan = v;
    }
    position_agent_service::save_worker_settings(&current)?;
    Ok(Json(current))
}

#[utoipa::path(
    get,
    path = "/agent/worker/status",
    tag = "Agent",
    responses(
        (status = 200, description = "Background worker runtime status", body = AgentWorkerStatus)
    )
)]
pub async fn get_agent_worker_status(
    State(_state): State<AppState>,
) -> ApiResult<Json<AgentWorkerStatus>> {
    Ok(Json(position_agent_service::load_worker_status()?))
}
