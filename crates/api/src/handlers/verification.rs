//! HTTP handlers for the player-verification endpoints.
//!
//! Mirrors Go's `internal/handler/player_verification.go`.
//!
//! Response wire formats are identical to the Go service:
//! - Success: `{"success":true,"value":{"code":0,"message":"success","data":{…}}}`
//! - Error:   `{"success":false,"errorCode":"…","message":"…"}`

use common::error::{AppError, ServiceError};
use common::response::{ApiSuccess, ErrorResponse};
use service::verification::{MerchantRuleResponse, SubmitVerifyRequest, SubmitVerifyResponse};
use crate::state::AppState;
use crate::handlers::JsonBody;
use axum::{
    extract::{Query, State},
    http::HeaderMap,
    response::IntoResponse,
};
use serde::Deserialize;
use tracing::{info, warn};

// ── Query params ──────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct QuestionListParams {
    #[serde(rename = "customerName")]
    pub customer_name: Option<String>,
}

// ── GET /verification/questions ───────────────────────────────────────────────

/// Return the question list for the requesting customer.
///
/// Required headers: `Merchant`, `CustomerIP`
/// Required query:   `customerName`
pub async fn get_question_list(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<QuestionListParams>,
) -> impl IntoResponse {
    // Header extraction.
    let merchant_code = match headers
        .get("Merchant")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
    {
        Some(v) => v.to_string(),
        None => {
            warn!("missing Merchant header");
            return ErrorResponse::bad_request(
                "merchant.param.missing",
                "merchant header is required",
            )
            .into_response();
        }
    };

    let customer_ip = match headers
        .get("CustomerIP")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
    {
        Some(v) => v.to_string(),
        None => {
            warn!("missing CustomerIP header");
            return ErrorResponse::bad_request(
                "customerip.param.missing",
                "CustomerIP header is required",
            )
            .into_response();
        }
    };

    let customer_name = match params.customer_name.filter(|s| !s.is_empty()) {
        Some(v) => v,
        None => {
            warn!("missing customerName query param");
            return ErrorResponse::bad_request(
                "customerName.param.missing",
                "customerName is required",
            )
            .into_response();
        }
    };

    info!(
        "GetQuestionList: merchant={} ip={} customerName={}",
        merchant_code, customer_ip, customer_name
    );

    match state
        .verification_svc
        .get_question_list(&merchant_code, &customer_ip, &customer_name)
        .await
    {
        Ok(result) => ApiSuccess::<MerchantRuleResponse>::ok(result).into_response(),
        Err(e) => map_question_list_error(e, &merchant_code, &customer_ip, &customer_name),
    }
}

fn map_question_list_error(
    e: AppError,
    merchant_code: &str,
    customer_ip: &str,
    customer_name: &str,
) -> axum::response::Response {
    match e {
        AppError::Service(ServiceError::MerchantNotFound(_)) => {
            ErrorResponse::not_found("merchant.rule.not_found", "Merchant rule not found")
                .into_response()
        }
        AppError::Service(ServiceError::QuestionLimitExceeded) => {
            warn!(
                "question retry limit exhausted: merchant={} ip={} customerName={}",
                merchant_code, customer_ip, customer_name
            );
            ErrorResponse::too_many(
                "question.retry.limit_exhausted",
                "Retry limit exhausted, please try again after 24 hours",
            )
            .into_response()
        }
        AppError::Service(ServiceError::RedisNotFound)
        | AppError::Service(ServiceError::RedisUnavailable(_)) => ErrorResponse::unavailable(
            "redis.unavailable",
            "Service temporarily unavailable, please try again later",
        )
        .into_response(),
        AppError::Service(ServiceError::ParseJsonFailed(_))
        | AppError::Service(ServiceError::RuleConfigInvalid(_)) => {
            ErrorResponse::internal("rule.config.invalid", "Invalid merchant rule configuration")
                .into_response()
        }
        other => {
            warn!(
                "GetQuestionList unexpected error: merchant={} err={}",
                merchant_code, other
            );
            ErrorResponse::internal("internal.server.error", "Internal server error")
                .into_response()
        }
    }
}

// ── POST /verification/materials ──────────────────────────────────────────────

/// Accept submitted verification materials and return a one-time password token.
///
/// Required headers: `Merchant`, `CustomerIP`
/// Request body:     `SubmitVerifyRequest` JSON
pub async fn submit_verify_materials(
    State(state): State<AppState>,
    headers: HeaderMap,
    JsonBody(body): JsonBody<SubmitVerifyRequest>,
) -> impl IntoResponse {
    let merchant_code = headers
        .get("Merchant")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let customer_ip = headers
        .get("CustomerIP")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    if body.data.is_empty() {
        return ErrorResponse::bad_request("verify.param.missing", "data array is required")
            .into_response();
    }

    info!(
        "SubmitVerifyMaterials: merchant={} ip={} customerName={}",
        merchant_code, customer_ip, body.customer_name
    );

    match state
        .verification_svc
        .submit_verify_materials(&merchant_code, &customer_ip, body)
        .await
    {
        Ok(result) => ApiSuccess::<SubmitVerifyResponse>::ok(result).into_response(),
        Err(e) => map_submit_error(e),
    }
}

fn map_submit_error(e: AppError) -> axum::response::Response {
    match e {
        AppError::Service(ServiceError::MerchantNotFound(_)) => {
            ErrorResponse::not_found("merchant.rule.not_found", "Merchant rule not found")
                .into_response()
        }
        AppError::Service(ServiceError::CustomerFetchFailed(_)) => {
            ErrorResponse::internal("verify.customer.internal_error", "Internal server error")
                .into_response()
        }
        AppError::Service(ServiceError::PasswordResetFailed(_)) => ErrorResponse::bad_request(
            "password.reset.failed",
            "Failed to generate password reset token",
        )
        .into_response(),
        AppError::Service(ServiceError::InvalidRequestParam(msg)) => {
            ErrorResponse::bad_request("verify.param.invalid", msg).into_response()
        }
        other => {
            warn!("SubmitVerifyMaterials unexpected error: {}", other);
            ErrorResponse::internal("verify.internal_error", "Internal server error")
                .into_response()
        }
    }
}
