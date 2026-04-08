/// Centralised error types.
///
/// Mirrors Go's `internal/apperror` package (`code.go` + `errors.go`).
/// `AppError` is the internal domain error; `ErrorResponse` is the HTTP response shape.
use axum::{
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ── Error leaf codes (mirrors apperror/code.go ErrorCode constants) ───────────

pub const SERVICE_PREFIX: &str = "ucs-fe";

/// Identifies the specific reason for an error — the final segment of
/// `ucs-fe.<module>.<code>`.
///
/// Mirrors Go's `apperror.ErrorCode` string constants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ErrorCode(pub &'static str);

impl ErrorCode {
    // ── General ──────────────────────────────────────────────────────────────
    pub const SYS_ERR: Self = Self("unknown_err");
    pub const PARAM_ERR: Self = Self("param_err");
    pub const REQ_ERR: Self = Self("req_param_err");
    pub const FORMAT_ERROR: Self = Self("format_error");
    pub const UPLOAD_ERROR: Self = Self("upload_error");
    pub const JSON_ERR: Self = Self("json_err");
    pub const AUTH_ERR: Self = Self("auth_err");
    pub const SIGN_ERR: Self = Self("sign_err");
    pub const FREQUENCY_ERR: Self = Self("frequency_err");
    pub const NETWORK_TIMEOUT: Self = Self("network_timeout");
    pub const STATUS_ERR: Self = Self("status_err");
    pub const AMOUNT_INVALID: Self = Self("amount_invalid");

    // ── Database ─────────────────────────────────────────────────────────────
    pub const DATA_NOT_FOUND: Self = Self("data_not_found");
    pub const DATA_HAS_EXISTED: Self = Self("data_has_existed");
    pub const INSERT_FAILED: Self = Self("insert_failed");
    pub const UPDATE_FAILED: Self = Self("update_failed");
    pub const DELETE_FAILED: Self = Self("delete_failed");
    pub const NO_DATA_UPDATE: Self = Self("no_data_update");
    pub const DB_BIND_PARAM_ERROR: Self = Self("db_bind_param_error");
    pub const SQL_EXECUTION_FAIL: Self = Self("sql_execution_fail");

    // ── Merchant ─────────────────────────────────────────────────────────────
    pub const MERCHANT_NOT_FOUND: Self = Self("merchant_not_found");
    pub const MERCHANT_EXISTED: Self = Self("merchant_already_exist");
    pub const MERCHANT_IS_NULL: Self = Self("merchant_is_null");
    pub const MERCHANT_NOT_EXIST: Self = Self("merchant_not_exist");

    // ── Business ─────────────────────────────────────────────────────────────
    pub const FIELD_CONFIG_NOT_EXIST: Self = Self("field_config_not_exist");
    pub const VALIDATION_RECORD_NOT_EXIST: Self = Self("validation_record_not_exist");
    pub const EXCEED_LIMIT: Self = Self("exceed_limit");
    pub const TASK_SUBMIT_FAIL: Self = Self("task_submit_fail");
    pub const INVALID_PARAM: Self = Self("invalid_param");
    pub const INVALID_DATE_PARAM: Self = Self("invalid_date_param");
    pub const TIME_RANGE_ERROR: Self = Self("time_range_error");
    pub const NO_LOG_RECORD: Self = Self("no_log_record");
    pub const INVALID_OPERAND: Self = Self("invalid_operand");

    // ── Downstream clients ────────────────────────────────────────────────────
    pub const UCS_CLIENT_ERR: Self = Self("ucs_client_err");
    pub const USS_CLIENT_ERR: Self = Self("uss_client_err");
    pub const TAC_CLIENT_ERR: Self = Self("tac_client_err");
    pub const PSS_CLIENT_ERR: Self = Self("pss_client_err");
    pub const WPS_CLIENT_ERR: Self = Self("wps_client_err");
    pub const MCS_CLIENT_ERR: Self = Self("mcs_client_err");

    // ── Customer ─────────────────────────────────────────────────────────────
    pub const CUSTOMER_ILLEGAL_MERCHANT: Self = Self("customer_merchant_error");

    // ── Rust-only additions ───────────────────────────────────────────────────
    pub const INTERNAL_ERROR: Self = Self("internal_error");
    pub const PARAM_MISSING: Self = Self("param_missing");
    pub const RULE_CONFIG_INVALID: Self = Self("rule_config_invalid");
    pub const CUSTOMER_FETCH_FAILED: Self = Self("fetch_failed");
    pub const RETRY_LIMIT_EXHAUSTED: Self = Self("retry_limit_exhausted");
    pub const MCS_FAILED: Self = Self("mcs_failed");
    pub const PARSE_FAILED: Self = Self("parse_failed");
    pub const PASSWORD_RESET_FAILED: Self = Self("reset_failed");
    pub const RATE_LIMIT_EXCEEDED: Self = Self("rate_limit_exceeded");
    pub const UNAVAILABLE: Self = Self("unavailable");
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

// ── Module identifiers (mirrors apperror/code.go Module constants) ────────────

/// Identifies the subsystem — the middle segment of `ucs-fe.<module>.<code>`.
///
/// Mirrors Go's `apperror.Module` type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Module {
    Oracle,
    Redis,
    Mongo,
    SecurityQuestion,
    Customer,
    Profile,
    NonBusiness,
    Merchant,
    Remember,
    Password,
    CustomerPermission,
    Sort,
    Mail,
    DynamicField,
    Relay,
    Register,
    // Rust-side modules
    Verification,
    Database,
    Wps,
    Uss,
    Mcs,
}

impl Module {
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Oracle => "oracle",
            Self::Redis => "redis",
            Self::Mongo => "mongo",
            Self::SecurityQuestion => "security_question",
            Self::Customer => "customer",
            Self::Profile => "profile",
            Self::NonBusiness => "non",
            Self::Merchant => "merchant",
            Self::Remember => "remember",
            Self::Password => "password",
            Self::CustomerPermission => "customer_permission",
            Self::Sort => "sort",
            Self::Mail => "mail",
            Self::DynamicField => "dynamic_field",
            Self::Relay => "relay",
            Self::Register => "register",
            Self::Verification => "verification",
            Self::Database => "db",
            Self::Wps => "wps",
            Self::Uss => "uss",
            Self::Mcs => "mcs",
        }
    }
}

// ── Domain error ──────────────────────────────────────────────────────────────

/// Internal domain error returned by service / repository layers.
///
/// Mirrors Go's `apperror.AppError`.
#[derive(Debug, Error)]
pub enum AppError {
    // ── Business errors (handler maps these to specific HTTP codes) ───────────
    #[error("merchant not found: {0}")]
    MerchantNotFound(String),

    #[error("customer fetch failed: {0}")]
    CustomerFetchFailed(String),

    #[error("customer personal info fetch failed: {0}")]
    CustomerPersonalInfoFetchFailed(String),

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

    // ── Infrastructure errors ─────────────────────────────────────────────────
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

// ── HTTP error response shape (mirrors apperror.HTTPError) ───────────────────

/// JSON error body returned to clients.
///
/// Mirrors Go's `apperror.HTTPError`:
/// ```json
/// {"success":false,"errorCode":"ucs-fe.merchant.not_found","message":"..."}
/// ```
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub success: bool,
    #[serde(rename = "errorCode")]
    pub error_code: String,
    pub message: String,
}

impl ErrorResponse {
    /// Build `"ucs-fe.<module>.<code>"` error code automatically.
    pub fn new(module: &Module, code: &ErrorCode, message: impl Into<String>) -> Self {
        Self {
            success: false,
            error_code: format!("{}.{}.{}", SERVICE_PREFIX, module.as_str(), code.0),
            message: message.into(),
        }
    }

    /// Build from a pre-assembled raw error code string (for handler inline usage).
    pub fn raw(error_code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            success: false,
            error_code: error_code.into(),
            message: message.into(),
        }
    }
}

// ── IntoResponse — AppError → HTTP 500 ───────────────────────────────────────

impl AppError {
    /// Map each variant to its (module, error code) pair — mirrors Go's
    /// `ErrorHandler` middleware which inspects `*apperror.AppError` and
    /// returns the module/code to the caller.
    fn module_and_code(&self) -> (&Module, &ErrorCode) {
        match self {
            Self::MerchantNotFound(_) => (&Module::Merchant, &ErrorCode::MERCHANT_NOT_FOUND),
            Self::CustomerFetchFailed(_) | Self::CustomerPersonalInfoFetchFailed(_) => {
                (&Module::Uss, &ErrorCode::USS_CLIENT_ERR)
            }
            Self::QuestionLimitExceeded => (&Module::Verification, &ErrorCode::EXCEED_LIMIT),
            Self::RedisNotFound => (&Module::Redis, &ErrorCode::DATA_NOT_FOUND),
            Self::WpsApiFailed(_) => (&Module::Wps, &ErrorCode::WPS_CLIENT_ERR),
            Self::EmailAlreadyBound | Self::PhoneAlreadyBound => {
                (&Module::Verification, &ErrorCode::STATUS_ERR)
            }
            Self::ParseJsonFailed(_) | Self::JsonError(_) => {
                (&Module::NonBusiness, &ErrorCode::JSON_ERR)
            }
            Self::VerifyPlayerInfoFailed(_) => (&Module::Mcs, &ErrorCode::MCS_CLIENT_ERR),
            Self::PasswordResetFailed(_) => (&Module::Uss, &ErrorCode::USS_CLIENT_ERR),
            Self::OracleError(_) => (&Module::Oracle, &ErrorCode::SQL_EXECUTION_FAIL),
            Self::RedisError(_) => (&Module::Redis, &ErrorCode::SYS_ERR),
            Self::HttpClientError(_) => (&Module::NonBusiness, &ErrorCode::NETWORK_TIMEOUT),
            Self::Internal(_) => (&Module::NonBusiness, &ErrorCode::SYS_ERR),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (module, code) = self.module_and_code();
        tracing::error!(error = %self, module = module.as_str(), code = %code, "AppError → 500");
        let body = ErrorResponse::new(module, code, self.to_string());
        (StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response()
    }
}

// ── Success response helpers ──────────────────────────────────────────────────

/// Inner `value` object for `BaseResponseT[*CommonResponse]`.
///
/// Mirrors Go's `resp.CommonResponse`.
#[derive(Debug, Serialize)]
pub struct CommonValue<T: Serialize> {
    pub code: i32,
    pub message: String,
    pub data: T,
}

/// Top-level success response wrapping `CommonValue`.
///
/// Mirrors Go's `resp.Success(data)` which returns
/// `BaseResponseT[*CommonResponse]`:
/// ```json
/// {"success":true,"value":{"code":0,"message":"success","data":{...}}}
/// ```
#[derive(Debug, Serialize)]
pub struct ApiSuccess<T: Serialize> {
    pub success: bool,
    pub value: CommonValue<T>,
}

impl<T: Serialize> ApiSuccess<T> {
    #[must_use]
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

/// Thin wrapper for endpoints that return a flat `{success, value}` shape
/// (e.g. `/ping`, `PhoneAlreadyBound` handler).
///
/// Mirrors Go's `BaseResponseT[T]`.
#[derive(Debug, Serialize)]
pub struct SuccessResponse<T: Serialize> {
    pub success: bool,
    pub value: T,
}

impl<T: Serialize> SuccessResponse<T> {
    #[must_use]
    pub fn new(value: T) -> Self {
        Self {
            success: true,
            value,
        }
    }
}

/// Simple `{success, message}` response with no value payload.
///
/// Mirrors Go's `SuccessResp` + optional message.
#[derive(Debug, Serialize)]
pub struct BaseResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl BaseResponse {
    #[must_use]
    pub fn ok() -> Self {
        Self {
            success: true,
            message: None,
        }
    }

    #[must_use]
    pub fn ok_with_message(msg: impl Into<String>) -> Self {
        Self {
            success: true,
            message: Some(msg.into()),
        }
    }
}
