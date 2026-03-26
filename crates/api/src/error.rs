//! API error types.

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::ToSchema;

/// API error type.
#[derive(Debug, Error)]
pub enum ApiError {
    /// Bad request error.
    #[error("Bad request: {0}")]
    BadRequest(String),

    /// Unauthorized error.
    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    /// Forbidden error.
    #[error("Forbidden: {0}")]
    Forbidden(String),

    /// Not found error.
    #[error("Not found: {0}")]
    NotFound(String),

    /// Conflict error.
    #[error("Conflict: {0}")]
    Conflict(String),

    /// Validation error.
    #[error("Validation error: {0}")]
    Validation(String),

    /// Internal server error.
    #[error("Internal error: {0}")]
    Internal(String),

    /// Service unavailable.
    #[error("Service unavailable: {0}")]
    ServiceUnavailable(String),
}

impl From<clmm_lp_execution::agent_decision::AgentDecisionValidationError> for ApiError {
    fn from(e: clmm_lp_execution::agent_decision::AgentDecisionValidationError) -> Self {
        Self::Validation(e.to_string())
    }
}

impl ApiError {
    /// Creates a bad request error.
    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self::BadRequest(msg.into())
    }

    /// Creates an unauthorized error.
    pub fn unauthorized(msg: impl Into<String>) -> Self {
        Self::Unauthorized(msg.into())
    }

    /// Creates a not found error.
    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::NotFound(msg.into())
    }

    /// Creates an internal error.
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::Internal(msg.into())
    }

    /// Gets the error code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::BadRequest(_) => "BAD_REQUEST",
            Self::Unauthorized(_) => "UNAUTHORIZED",
            Self::Forbidden(_) => "FORBIDDEN",
            Self::NotFound(_) => "NOT_FOUND",
            Self::Conflict(_) => "CONFLICT",
            Self::Validation(_) => "VALIDATION_ERROR",
            Self::Internal(_) => "INTERNAL_ERROR",
            Self::ServiceUnavailable(_) => "SERVICE_UNAVAILABLE",
        }
    }

    /// Gets the HTTP status code.
    pub fn status_code(&self) -> StatusCode {
        match self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::Validation(_) => StatusCode::UNPROCESSABLE_ENTITY,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::ServiceUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status_code();
        let body = ErrorResponse {
            code: self.code().to_string(),
            message: self.to_string(),
            details: None,
        };

        (status, Json(body)).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(err: anyhow::Error) -> Self {
        Self::Internal(err.to_string())
    }
}

/// Error response body.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ErrorResponse {
    /// Error code.
    pub code: String,
    /// Error message.
    pub message: String,
    /// Additional details.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

/// Result type for API handlers.
pub type ApiResult<T> = Result<T, ApiError>;
