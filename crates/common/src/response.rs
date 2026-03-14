//! Canonical HTTP response types, mirroring Go's `internal/types/resp` package.
//!
//! # Success wire format
//! ```json
//! {"success": true, "value": {"code": 0, "message": "success", "data": <T>}}
//! ```
//!
//! # Error wire format
//! ```json
//! {"success": false, "errorCode": "merchant.rule.not_found", "message": "..."}
//! ```

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;

// ── Success ───────────────────────────────────────────────────────────────────

/// Inner `value` object embedded in every successful response.
#[derive(Debug, Serialize)]
pub struct CommonData<T: Serialize> {
    pub data: T,
    pub message: &'static str,
    pub code: i32,
}

/// Top-level success envelope.
#[derive(Debug, Serialize)]
pub struct ApiSuccess<T: Serialize> {
    pub value: CommonData<T>,
    pub success: bool,
}

impl<T: Serialize> ApiSuccess<T> {
    pub fn ok(data: T) -> Self {
        Self {
            success: true,
            value: CommonData {
                code: 0,
                message: "success",
                data,
            },
        }
    }
}

impl<T: Serialize + Send> IntoResponse for ApiSuccess<T> {
    fn into_response(self) -> Response {
        Json(self).into_response()
    }
}

// ── Error ─────────────────────────────────────────────────────────────────────

/// Error response body — matches Go's `ErrResponse`.
#[derive(Debug, Serialize)]
pub struct ApiError {
    #[serde(rename = "errorCode")]
    pub error_code: &'static str,
    pub message: String,
    pub success: bool,
}

impl ApiError {
    pub fn new(error_code: &'static str, message: impl Into<String>) -> Self {
        Self {
            error_code,
            message: message.into(),
            success: false,
        }
    }
}

/// Combines an HTTP status code with an `ApiError` body.
///
/// Build helpers mirror Go's handler patterns:
/// - `bad_request` → 400
/// - `not_found`   → 404
/// - `too_many`    → 429
/// - `unavailable` → 503
/// - `internal`    → 500
pub struct ErrorResponse(pub StatusCode, pub ApiError);

impl ErrorResponse {
    pub fn bad_request(code: &'static str, msg: impl Into<String>) -> Self {
        Self(StatusCode::BAD_REQUEST, ApiError::new(code, msg))
    }
    pub fn not_found(code: &'static str, msg: impl Into<String>) -> Self {
        Self(StatusCode::NOT_FOUND, ApiError::new(code, msg))
    }
    pub fn too_many(code: &'static str, msg: impl Into<String>) -> Self {
        Self(StatusCode::TOO_MANY_REQUESTS, ApiError::new(code, msg))
    }
    pub fn unavailable(code: &'static str, msg: impl Into<String>) -> Self {
        Self(StatusCode::SERVICE_UNAVAILABLE, ApiError::new(code, msg))
    }
    pub fn internal(code: &'static str, msg: impl Into<String>) -> Self {
        Self(StatusCode::INTERNAL_SERVER_ERROR, ApiError::new(code, msg))
    }
}

impl IntoResponse for ErrorResponse {
    fn into_response(self) -> Response {
        (self.0, Json(self.1)).into_response()
    }
}
