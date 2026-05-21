//! Handlers for bulk position close (`POST/GET /positions/close-all`).

use crate::error::{ApiError, ApiResult};
use crate::models::{
    CloseAllBatchStatusResponse, CloseAllPositionsPreviewResponse, CloseAllPositionsRequest,
    CloseAllPositionsStartResponse,
};
use crate::services::position_close_all::{
    get_close_all_batch, preview_close_all, start_close_all_batch,
};
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;

/// Start closing all monitored positions (async batch job).
#[utoipa::path(
    post,
    path = "/positions/close-all",
    tag = "Positions",
    request_body = CloseAllPositionsRequest,
    responses(
        (status = 202, description = "Batch queued", body = CloseAllPositionsStartResponse),
        (status = 400, description = "Invalid request")
    )
)]
pub async fn post_close_all_positions(
    State(state): State<AppState>,
    Json(req): Json<CloseAllPositionsRequest>,
) -> Result<(StatusCode, Json<CloseAllPositionsStartResponse>), ApiError> {
    let resp = start_close_all_batch(state, req).await?;
    Ok((StatusCode::ACCEPTED, Json(resp)))
}

/// Preview wallet groups and skipped positions before bulk close (no on-chain txs).
#[utoipa::path(
    post,
    path = "/positions/close-all/preview",
    tag = "Positions",
    request_body = CloseAllPositionsRequest,
    responses(
        (status = 200, description = "Close-all plan preview", body = CloseAllPositionsPreviewResponse),
        (status = 400, description = "Invalid request")
    )
)]
pub async fn post_close_all_positions_preview(
    State(state): State<AppState>,
    Json(req): Json<CloseAllPositionsRequest>,
) -> ApiResult<Json<CloseAllPositionsPreviewResponse>> {
    Ok(Json(preview_close_all(&state, req).await?))
}

/// Poll close-all batch status.
#[utoipa::path(
    get,
    path = "/positions/close-all/{batch_id}",
    tag = "Positions",
    params(
        ("batch_id" = String, Path, description = "Batch UUID from POST /positions/close-all")
    ),
    responses(
        (status = 200, description = "Batch status", body = CloseAllBatchStatusResponse),
        (status = 404, description = "Batch not found")
    )
)]
pub async fn get_close_all_positions_batch(
    State(_state): State<AppState>,
    Path(batch_id): Path<String>,
) -> ApiResult<Json<CloseAllBatchStatusResponse>> {
    Ok(Json(get_close_all_batch(&batch_id).await?))
}
