//! Dip Agent endpoints: snapshot (Faza B) + propose-only decision envelope (Faza C).

use crate::error::{ApiError, ApiResult};
use crate::models::{
    AgentDecisionRow, DipBuyDipOrder, DipProposeRequest, DipProposeResponse, DipSnapshotResponse,
    DipZoneBandDto, DipZonesDto,
};
use crate::services::price_fetch::fetch_mint_prices_usd;
use axum::Json;
use axum::extract::Query;
use axum::http::HeaderMap;
use clmm_lp_data::dip::{
    DipAuditDecisionRow, DipProposeAction, DipProposeDecision, DipProposeMode, DipZoneLabel,
    append_dip_propose, classify_zone, default_agent_decisions_path, default_zones_file,
    load_dip_zones_file,
};
use clmm_lp_data::providers::{JupiterTriggerClient, OrderHistoryQuery, filter_open_buy_dips};
use serde::Deserialize;
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

fn trigger_jwt(headers: &HeaderMap) -> Option<String> {
    bearer_from_headers(headers).or_else(|| {
        std::env::var("CLMM_JUPITER_TRIGGER_JWT")
            .ok()
            .filter(|s| !s.trim().is_empty())
    })
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

/// `GET /dip/snapshot` — SOL zone + optional open Buy-the-Dip Trigger orders (read-only).
#[utoipa::path(
    get,
    path = "/dip/snapshot",
    tag = "Dip",
    params(
        ("include_orders" = Option<bool>, Query, description = "Fetch Trigger active orders when JWT present"),
        ("zones_path" = Option<String>, Query, description = "Override path to zones.json")
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

    let mut buy_dip_orders: Vec<DipBuyDipOrder> = Vec::new();
    let mut orders_source = "skipped".to_string();

    if q.include_orders {
        match (jupiter_api_key(&headers), trigger_jwt(&headers)) {
            (Some(api_key), Some(jwt)) => {
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
                        let dips = filter_open_buy_dips(&hist.orders);
                        buy_dip_orders = dips
                            .into_iter()
                            .map(|o| DipBuyDipOrder {
                                id: o.id,
                                order_state: o.order_state,
                                trigger_price_usd: o.trigger_price_usd,
                                input_mint: o.input_mint,
                                output_mint: o.output_mint,
                                remaining_input_amount: o.remaining_input_amount,
                                expires_at: o.expires_at,
                            })
                            .collect();
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
                    "Trigger JWT missing — pass Authorization: Bearer <jwt> or CLMM_JUPITER_TRIGGER_JWT"
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
        buy_dip_orders,
        orders_source,
        notes,
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
    let written =
        append_dip_propose(&path, row).map_err(|e| ApiError::internal(e.to_string()))?;

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
