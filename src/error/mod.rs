/// Centralised error types.
///
/// Mirrors Go's `internal/apperror` package.
/// `AppError` is an internal domain error; `ApiError` is the HTTP response shape.
use axum::{
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use serde::Serialize;
use thiserror::Error;

// ── Error codes (mirrors apperror/code.go) ────────────────────────────────────

pub const SERVICE_PREFIX: &str = "ucs-fe";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Module {
    Merchant,
    Customer,
    Verification,
    Redis,
    Database,
    Wps,
    Uss,
    Mcs,
    Password,
    NonBusiness,
}

impl Module {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Merchant => "merchant",
            Self::Customer => "customer",
            Self::Verification => "verification",
            Self::Redis => "redis",
            Self::Database => "db",
            Self::Wps => "wps",
            Self::Uss => "uss",
            Self::Mcs => "mcs",
            Self::Password => "password",
            Self::NonBusiness => "non",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorCode {
    // General
    UnknownErr,
    InternalError,
    ParamMissing,
    ParamInvalid,
    // Merchant
    MerchantNotFound,
    MerchantRuleConfigInvalid,
    // Customer
    CustomerFetchFailed,
    // Verification
    QuestionRetryLimitExhausted,
    VerifyMcsFailed,
    ParseJsonFailed,
    // Password
    PasswordResetFailed,
    // Infrastructure
    RedisUnavailable,
    WpsUnavailable,
    // Rate limit
    RateLimitExceeded,
}

impl ErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::UnknownErr => "unknown_err",
            Self::InternalError => "internal_error",
            Self::ParamMissing => "param_missing",
            Self::ParamInvalid => "param_invalid",
            Self::MerchantNotFound => "not_found",
            Self::MerchantRuleConfigInvalid => "rule_config_invalid",
            Self::CustomerFetchFailed => "fetch_failed",
            Self::QuestionRetryLimitExhausted => "retry_limit_exhausted",
            Self::VerifyMcsFailed => "mcs_failed",
            Self::ParseJsonFailed => "parse_failed",
            Self::PasswordResetFailed => "reset_failed",
            Self::RedisUnavailable => "unavailable",
            Self::WpsUnavailable => "unavailable",
            Self::RateLimitExceeded => "rate_limit_exceeded",
        }
    }
}

// ── Domain error ──────────────────────────────────────────────────────────────

/// Internal domain error returned by service/repository layers.
#[derive(Debug, Error)]
pub enum AppError {
    // Business errors (handler maps these to specific HTTP codes)
    #[error("merchant not found: {0}")]
    MerchantNotFound(String),

    #[error("customer fetch failed: {0}")]
    CustomerFetchFailed(String),

    #[error("question retry limit exceeded")]
    QuestionLimitExceeded,

    #[error("redis instance not found")]
    RedisNotFound,

    #[error("WPS API failed: {0}")]
    WpsApiFailed(String),

    #[error("email already verified (bound)")]
    EmailAlreadyBound,

    #[error("phone already bound")]
    PhoneAlreadyBound,

    #[error("parse JSON failed: {0}")]
    ParseJsonFailed(String),

    #[error("verify player info (MCS) failed: {0}")]
    VerifyPlayerInfoFailed(String),

    #[error("password reset failed: {0}")]
    PasswordResetFailed(String),

    // Infrastructure errors
    #[error("oracle error: {0}")]
    OracleError(#[from] oracle::Error),

    #[error("redis error: {0}")]
    RedisError(#[from] redis::RedisError),

    #[error("http client error: {0}")]
    HttpClientError(#[from] reqwest::Error),

    #[error("json error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("internal error: {0}")]
    Internal(#[from] anyhow::Error),
}

// ── JSON error response shape ─────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub success: bool,
    #[serde(rename = "errorCode")]
    pub error_code: String,
    pub message: String,
}

impl ErrorResponse {
    pub fn new(module: &Module, code: &ErrorCode, message: impl Into<String>) -> Self {
        Self {
            success: false,
            error_code: format!("{}.{}.{}", SERVICE_PREFIX, module.as_str(), code.as_str()),
            message: message.into(),
        }
    }

    /// Build from a raw pre-assembled error code string (for handler inline usage).
    pub fn raw(error_code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            success: false,
            error_code: error_code.into(),
            message: message.into(),
        }
    }
}

// ── IntoResponse — AppError → HTTP ───────────────────────────────────────────

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        tracing::error!(error = %self, "unhandled AppError → 500");

        let body = ErrorResponse::new(
            &Module::NonBusiness,
            &ErrorCode::InternalError,
            self.to_string(),
        );
        (StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response()
    }
}

// ── Success response helpers ──────────────────────────────────────────────────

/// Mirrors Go's `resp.CommonResponse` — the inner `value` object.
#[derive(Debug, Serialize)]
pub struct CommonValue<T: Serialize> {
    pub code: i32,
    pub message: String,
    pub data: T,
}

/// Mirrors Go's `resp.Success(data)` → `BaseResponseT[*CommonResponse]`.
///
/// Serialises as:
/// ```json
/// {"success":true,"value":{"code":0,"message":"success","data":{...}}}
/// ```
#[derive(Debug, Serialize)]
pub struct ApiSuccess<T: Serialize> {
    pub success: bool,
    pub value: CommonValue<T>,
}

impl<T: Serialize> ApiSuccess<T> {
    pub fn new(data: T) -> Self {
        Self {
            success: true,
            value: CommonValue {
                code: 0,
                message: "success".into(),
                data,
            },
        }
    }
}

/// Thin wrapper kept for ping / other endpoints that don't wrap in CommonValue.
#[derive(Debug, Serialize)]
pub struct SuccessResponse<T: Serialize> {
    pub success: bool,
    pub value: T,
}

impl<T: Serialize> SuccessResponse<T> {
    pub fn new(value: T) -> Self {
        Self {
            success: true,
            value,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct BaseResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl BaseResponse {
    pub fn ok_with_message(msg: impl Into<String>) -> Self {
        Self {
            success: true,
            message: Some(msg.into()),
        }
    }
}
