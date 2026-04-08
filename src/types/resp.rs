/// HTTP response types.
///
/// Mirrors Go's `internal/types/resp/response.go` and `submit.go`.
use serde::Serialize;

use crate::model::QuestionInfo;

// ── Primitive responses ───────────────────────────────────────────────────────

/// Empty response body (`{}`).
/// Mirrors Go's `resp.EmptyResp`.
#[derive(Debug, Serialize, Default)]
pub struct EmptyResp {}

/// Single-field success flag.
/// Mirrors Go's `resp.SuccessResp`.
#[derive(Debug, Serialize)]
pub struct SuccessResp {
    pub success: bool,
}

impl SuccessResp {
    pub fn ok() -> Self {
        Self { success: true }
    }
}

// ── Base / generic responses ──────────────────────────────────────────────────

/// Flexible response with an optional value, message, and error code.
/// Mirrors Go's `resp.ApiBaseMessageResp`.
#[derive(Debug, Serialize)]
pub struct ApiBaseMessageResp {
    pub success: bool,
    #[serde(rename = "value", skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
    #[serde(rename = "errorCode", skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(rename = "message", skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Login/auth response with raw bytes payload.
/// Mirrors Go's `resp.LoginWithInfoResp`.
#[derive(Debug, Serialize)]
pub struct LoginWithInfoResp {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<Vec<u8>>,
}

/// Generic base response.
/// Mirrors Go's `resp.BaseResponse`.
#[derive(Debug, Serialize)]
pub struct BaseResponse {
    pub success: bool,
    #[serde(rename = "value", skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
    #[serde(rename = "message", skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(rename = "errorCode", skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

/// Generic typed response.
/// Mirrors Go's `resp.BaseResponseT[T]`.
#[derive(Debug, Serialize)]
pub struct BaseResponseT<T: Serialize> {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<T>,
}

impl<T: Serialize> BaseResponseT<T> {
    pub fn ok(value: T) -> Self {
        Self {
            success: true,
            value: Some(value),
        }
    }
    pub fn fail() -> Self {
        Self {
            success: false,
            value: None,
        }
    }
}

// ── Domain response types ─────────────────────────────────────────────────────

/// Question-list response.
/// Mirrors Go's `resp.MerchantRuleResponse`.
#[derive(Debug, Serialize)]
pub struct MerchantRuleResponse {
    #[serde(rename = "merchantCode")]
    pub merchant_code: String,
    pub questions: Vec<QuestionInfo>,
}

// ── Common response (inner value) ─────────────────────────────────────────────

/// Inner data object for the standard API envelope.
/// Mirrors Go's `resp.CommonResponse`.
#[derive(Debug, Serialize)]
pub struct CommonResponse {
    pub data: serde_json::Value,
    pub message: String,
    pub code: i32,
}

/// Builds a success envelope wrapping a `CommonResponse`.
/// Mirrors Go's `resp.Success(data)`.
pub fn success(data: impl Serialize) -> BaseResponseT<CommonResponse> {
    BaseResponseT::ok(CommonResponse {
        code: 0,
        message: "success".to_string(),
        data: serde_json::to_value(data).unwrap_or(serde_json::Value::Null),
    })
}

/// Builds a failure envelope wrapping a `CommonResponse`.
/// Mirrors Go's `resp.Fail(code, msg)`.
pub fn fail(code: i32, msg: impl Into<String>) -> BaseResponseT<CommonResponse> {
    BaseResponseT {
        success: false,
        value: Some(CommonResponse {
            code,
            message: msg.into(),
            data: serde_json::Value::Null,
        }),
    }
}

// ── Error responses ───────────────────────────────────────────────────────────

/// Unified error response.
/// Mirrors Go's `resp.ErrResponse`.
#[derive(Debug, Serialize)]
pub struct ErrResponse {
    pub success: bool,
    #[serde(rename = "errorCode")]
    pub error_code: String,
    pub message: String,
}

impl ErrResponse {
    /// Mirrors Go's `resp.NewErrResponse(errorCode, message)`.
    pub fn new(error_code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            success: false,
            error_code: error_code.into(),
            message: message.into(),
        }
    }
}

/// 400 Missing parameter error.
/// Mirrors Go's `resp.ErrMissingParam`.
#[derive(Debug, Serialize)]
pub struct ErrMissingParam {
    pub code: String,
    pub message: String,
}

impl ErrMissingParam {
    /// Mirrors Go's `resp.NewErrMissingParam(field)`.
    pub fn new(field: &str) -> Self {
        Self {
            code: "MISSING_PARAM".to_string(),
            message: format!("{field} is required"),
        }
    }
}

/// 404 Not found error.
/// Mirrors Go's `resp.ErrNotFound`.
#[derive(Debug, Serialize)]
pub struct ErrNotFound {
    pub code: String,
    pub message: String,
}

impl ErrNotFound {
    /// Mirrors Go's `resp.NewErrNotFound(resource)`.
    pub fn new(resource: &str) -> Self {
        Self {
            code: "NOT_FOUND".to_string(),
            message: format!("{resource} not found"),
        }
    }
}

/// 500 Internal server error.
/// Mirrors Go's `resp.ErrInternalServer`.
#[derive(Debug, Serialize)]
pub struct ErrInternalServer {
    pub code: String,
    pub message: String,
}

impl ErrInternalServer {
    /// Mirrors Go's `resp.NewErrInternalServer(msg)`.
    pub fn new(msg: impl Into<String>) -> Self {
        Self {
            code: "INTERNAL_ERROR".to_string(),
            message: msg.into(),
        }
    }
}

// ── Verification submit response variants ────────────────────────────────────

/// Response data when score was NOT sufficient to unlock reset.
/// Mirrors Go's `resp.SubmitVerifyDataScoreNotChecked`.
#[derive(Debug, Serialize)]
pub struct SubmitVerifyDataScoreNotChecked {
    #[serde(rename = "scoreChecked")]
    pub score_checked: bool,
}

/// Response data when score WAS sufficient — OTP is issued.
/// Mirrors Go's `resp.SubmitVerifyDataScoreChecked`.
#[derive(Debug, Serialize)]
pub struct SubmitVerifyDataScoreChecked {
    #[serde(rename = "bindType")]
    pub bind_type: String,
    #[serde(rename = "oneTimePassword")]
    pub one_time_password: String,
}

/// Top-level envelope for score-not-checked result.
/// Mirrors Go's `resp.SubmitVerifyResponseScoreNotChecked`.
#[derive(Debug, Serialize)]
pub struct SubmitVerifyResponseScoreNotChecked {
    pub success: bool,
    pub value: SubmitVerifyInnerNotChecked,
}

#[derive(Debug, Serialize)]
pub struct SubmitVerifyInnerNotChecked {
    pub message: String,
    pub code: i32,
    pub data: SubmitVerifyDataScoreNotChecked,
}

/// Top-level envelope for score-checked result.
/// Mirrors Go's `resp.SubmitVerifyResponseScoreChecked`.
#[derive(Debug, Serialize)]
pub struct SubmitVerifyResponseScoreChecked {
    pub success: bool,
    pub value: SubmitVerifyInnerChecked,
}

#[derive(Debug, Serialize)]
pub struct SubmitVerifyInnerChecked {
    pub data: SubmitVerifyDataScoreChecked,
    pub message: String,
    pub code: i32,
}

// ── Flat submit data (used directly by handlers) ──────────────────────────────

/// Merged submit verify data; the handler picks the right envelope above.
/// Mirrors Go's `resp.SubmitVerifyData`.
/// Go uses `omitempty` on all fields including the bool, so `false` is omitted.
#[derive(Debug, Serialize)]
pub struct SubmitVerifyData {
    #[serde(rename = "scoreChecked", skip_serializing_if = "std::ops::Not::not")]
    pub score_checked: bool,
    #[serde(rename = "bindType", skip_serializing_if = "Option::is_none")]
    pub bind_type: Option<String>,
    #[serde(rename = "oneTimePassword", skip_serializing_if = "Option::is_none")]
    pub one_time_password: Option<String>,
}
