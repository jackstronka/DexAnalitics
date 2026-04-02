//! Analytics handlers.

use crate::error::ApiResult;
use crate::models::{PortfolioAnalyticsResponse, SimulationRequest, SimulationResponse};
use crate::services::simulation_analytics::run_dashboard_simulation;
use crate::state::AppState;
use axum::{Json, extract::State};
use rust_decimal::Decimal;

/// Get portfolio analytics.
#[utoipa::path(
    get,
    path = "/analytics/portfolio",
    tag = "Analytics",
    responses(
        (status = 200, description = "Portfolio analytics", body = PortfolioAnalyticsResponse)
    )
)]
pub async fn get_portfolio_analytics(
    State(state): State<AppState>,
) -> ApiResult<Json<PortfolioAnalyticsResponse>> {
    let positions = state.monitor.get_positions().await;

    let mut total_value = Decimal::ZERO;
    let mut total_pnl = Decimal::ZERO;
    let mut total_fees = Decimal::ZERO;
    let mut total_il = Decimal::ZERO;
    let mut in_range_count = 0u32;
    let mut best_pnl = Decimal::MIN;
    let mut worst_pnl = Decimal::MAX;
    let mut best_position = None;
    let mut worst_position = None;

    for position in &positions {
        total_value += position.pnl.current_value_usd;
        total_pnl += position.pnl.net_pnl_usd;
        total_fees += position.pnl.fees_usd;
        total_il += position.pnl.il_pct;

        if position.in_range {
            in_range_count += 1;
        }

        if position.pnl.net_pnl_pct > best_pnl {
            best_pnl = position.pnl.net_pnl_pct;
            best_position = Some(position.address.to_string());
        }

        if position.pnl.net_pnl_pct < worst_pnl {
            worst_pnl = position.pnl.net_pnl_pct;
            worst_position = Some(position.address.to_string());
        }
    }

    let position_count = positions.len() as u32;
    let avg_il = if position_count > 0 {
        total_il / Decimal::from(position_count)
    } else {
        Decimal::ZERO
    };

    let total_pnl_pct = if total_value > Decimal::ZERO {
        (total_pnl / total_value) * Decimal::from(100)
    } else {
        Decimal::ZERO
    };

    let response = PortfolioAnalyticsResponse {
        total_value_usd: total_value,
        total_pnl_usd: total_pnl,
        total_pnl_pct,
        total_fees_usd: total_fees,
        total_il_pct: avg_il,
        active_positions: position_count,
        positions_in_range: in_range_count,
        best_position,
        worst_position,
    };

    Ok(Json(response))
}

/// Run a simulation.
#[utoipa::path(
    post,
    path = "/analytics/simulate",
    tag = "Analytics",
    request_body = SimulationRequest,
    responses(
        (status = 200, description = "Simulation results", body = SimulationResponse),
        (status = 400, description = "Invalid request")
    )
)]
pub async fn run_simulation(
    State(state): State<AppState>,
    Json(request): Json<SimulationRequest>,
) -> ApiResult<Json<SimulationResponse>> {
    let response = run_dashboard_simulation(&state, request).await?;
    Ok(Json(response))
}
