//! Structured error types mirroring Go's `internal/apperror` package.
//!
//! The three-level hierarchy mirrors Go:
//! - `Module`       — which subsystem produced the error
//! - `ErrorCode`    — the specific failure kind
//! - `ServiceError` — sentinel errors used as cross-layer signals
//! - `InfraError`   — infrastructure-level failures (DB, cache, HTTP)
//! - `AppError`     — top-level application error that axum can convert to HTTP

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use thiserror::Error;

// ── Module ────────────────────────────────────────────────────────────────────

/// Subsystem identifier — middle segment of a fully-qualified error code,
/// e.g. `"ucs-fe.merchant.not_found"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Module {
    Oracle,
    Redis,
    Mongo,
    SecurityQuestion,
    Customer,
    Profile,
    None,
    Merchant,
    Remember,
    Password,
    CustomerPermission,
    Sort,
    Mail,
    DynamicField,
    Relay,
    Register,
}

impl Module {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Oracle => "oracle",
            Self::Redis => "redis",
            Self::Mongo => "mongo",
            Self::SecurityQuestion => "security_question",
            Self::Customer => "customer",
            Self::Profile => "profile",
            Self::None => "non",
            Self::Merchant => "merchant",
            Self::Remember => "remember",
            Self::Password => "password",
            Self::CustomerPermission => "customer_permission",
            Self::Sort => "sort",
            Self::Mail => "mail",
            Self::DynamicField => "dynamic_field",
            Self::Relay => "relay",
            Self::Register => "register",
        }
    }
}

// ── ErrorCode ─────────────────────────────────────────────────────────────────

/// Specific failure reason — final segment of a fully-qualified error code.
pub mod code {
    // General
    pub const SYS_ERR: &str = "unknown_err";
    pub const PARAM_ERR: &str = "param_err";
    pub const REQ_ERR: &str = "req_param_err";
    pub const FORMAT_ERROR: &str = "format_error";
    pub const UPLOAD_ERROR: &str = "upload_error";
    pub const JSON_ERR: &str = "json_err";
    pub const AUTH_ERR: &str = "auth_err";
    pub const SIGN_ERR: &str = "sign_err";
    pub const FREQUENCY_ERR: &str = "frequency_err";
    pub const NET_TIMEOUT: &str = "network_timeout";
    pub const STATUS_ERR: &str = "status_err";
    pub const AMOUNT_INVALID: &str = "amount_invalid";

    // Database
    pub const DATA_NOT_FOUND: &str = "data_not_found";
    pub const DATA_HAS_EXISTED: &str = "data_has_existed";
    pub const INSERT_FAILED: &str = "insert_failed";
    pub const UPDATE_FAILED: &str = "update_failed";
    pub const DELETE_FAILED: &str = "delete_failed";
    pub const NO_DATA_UPDATE: &str = "no_data_update";
    pub const DB_BIND_PARAM_ERR: &str = "db_bind_param_error";
    pub const SQL_EXECUTION_FAIL: &str = "sql_execution_fail";

    // Merchant
    pub const MERCHANT_NOT_FOUND: &str = "merchant_not_found";
    pub const MERCHANT_EXISTED: &str = "merchant_already_exist";
    pub const MERCHANT_IS_NULL: &str = "merchant_is_null";
    pub const MERCHANT_NOT_EXIST: &str = "merchant_not_exist";

    // Business
    pub const FIELD_CONFIG_NOT_EXIST: &str = "field_config_not_exist";
    pub const VALIDATION_RECORD_NOT_EXIST: &str = "validation_record_not_exist";
    pub const EXCEED_LIMIT: &str = "exceed_limit";
    pub const TASK_SUBMIT_FAIL: &str = "task_submit_fail";
    pub const INVALID_PARAM: &str = "invalid_param";
    pub const INVALID_DATE_PARAM: &str = "invalid_date_param";
    pub const TIME_RANGE_ERROR: &str = "time_range_error";
    pub const NO_LOG_RECORD: &str = "no_log_record";
    pub const INVALID_OPERAND: &str = "invalid_operand";

    // Downstream clients
    pub const UCS_CLIENT_ERR: &str = "ucs_client_err";
    pub const USS_CLIENT_ERR: &str = "uss_client_err";
    pub const TAC_CLIENT_ERR: &str = "tac_client_err";
    pub const PSS_CLIENT_ERR: &str = "pss_client_err";
    pub const WPS_CLIENT_ERR: &str = "wps_client_err";
    pub const MCS_CLIENT_ERR: &str = "mcs_client_err";

    // Customer
    pub const CUSTOMER_MERCHANT_ERROR: &str = "customer_merchant_error";
}

// ── Sentinel service errors ───────────────────────────────────────────────────

/// Business-logic sentinel errors — returned by services, mapped to HTTP codes
/// by handlers.  Mirrors Go's `var Err* = errors.New(...)` declarations.
#[derive(Debug, Error, PartialEq)]
pub enum ServiceError {
    #[error("merchant not found: {0}")]
    MerchantNotFound(String),

    #[error("customer fetch failed: {0}")]
    CustomerFetchFailed(String),

    #[error("password reset failed: {0}")]
    PasswordResetFailed(String),

    #[error("verify player info failed")]
    VerifyPlayerInfoFailed,

    #[error("parse json failed: {0}")]
    ParseJsonFailed(String),

    #[error("question retry limit exceeded")]
    QuestionLimitExceeded,

    #[error("redis instance not found")]
    RedisNotFound,

    #[error("redis unavailable: {0}")]
    RedisUnavailable(String),

    #[error("invalid request parameter: {0}")]
    InvalidRequestParam(String),

    #[error("internal service error: {0}")]
    Internal(String),

    #[error("score calculation failed")]
    ScoreCalculationFailed,

    #[error("mcs verification failed")]
    McsVerifyFailed,

    #[error("uss api failed: {0}")]
    UssApiFailed(String),

    #[error("rule config invalid: {0}")]
    RuleConfigInvalid(String),
}

// ── Infrastructure errors ─────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum InfraError {
    #[error("oracle: {0}")]
    Oracle(#[from] oracle::Error),

    #[error("r2d2 pool: {0}")]
    Pool(String),

    #[error("redis: {0}")]
    Redis(#[from] redis::RedisError),

    #[error("http client: {0}")]
    Http(#[from] reqwest::Error),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

// ── Top-level AppError ────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum AppError {
    #[error("service: {0}")]
    Service(#[from] ServiceError),

    #[error("infra: {0}")]
    Infra(#[from] InfraError),
}

// ── Error wire body (for AppError → HTTP) ────────────────────────────────────

#[derive(Serialize)]
struct ErrBody {
    #[serde(rename = "errorCode")]
    error_code: String,
    message: String,
    success: bool,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, error_code, message) = match &self {
            AppError::Service(e) => match e {
                ServiceError::MerchantNotFound(_) => (
                    StatusCode::NOT_FOUND,
                    "merchant.rule.not_found",
                    e.to_string(),
                ),
                ServiceError::QuestionLimitExceeded => (
                    StatusCode::TOO_MANY_REQUESTS,
                    "question.retry.limit_exhausted",
                    "Retry limit exhausted, please try again after 24 hours".to_string(),
                ),
                ServiceError::RedisNotFound => (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "redis.unavailable",
                    "Service temporarily unavailable, please try again later".to_string(),
                ),
                ServiceError::RedisUnavailable(_) => (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "redis.unavailable",
                    "Service temporarily unavailable, please try again later".to_string(),
                ),
                ServiceError::ParseJsonFailed(_) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "rule.config.invalid",
                    "Invalid merchant rule configuration".to_string(),
                ),
                ServiceError::RuleConfigInvalid(_) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "rule.config.invalid",
                    "Invalid merchant rule configuration".to_string(),
                ),
                ServiceError::CustomerFetchFailed(_) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "verify.customer.internal_error",
                    "Internal server error".to_string(),
                ),
                ServiceError::PasswordResetFailed(_) => (
                    StatusCode::BAD_REQUEST,
                    "password.reset.failed",
                    "Failed to generate password reset token".to_string(),
                ),
                _ => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal.server.error",
                    "Internal server error".to_string(),
                ),
            },
            AppError::Infra(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal.server.error",
                "Internal server error".to_string(),
            ),
        };

        let body = ErrBody {
            error_code: error_code.to_string(),
            message,
            success: false,
        };
        (status, Json(body)).into_response()
    }
}
