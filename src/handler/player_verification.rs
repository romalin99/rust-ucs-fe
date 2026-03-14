/// HTTP handlers for player verification.
///
/// Mirrors Go's `internal/handler/player_verification.go`.
/// Uses Axum extractors and returns typed JSON responses.
use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json, Response},
};
use std::sync::Arc;

use crate::app_state::AppState;
use crate::client::wps::{ResetPasswordStatusResponse, ResetPasswordStatusValue};
use crate::error::{ApiSuccess, AppError, ErrorResponse};
use crate::types::req::{GetQuestionListParams, SubmitVerifyRequest};

// ── GET /tcg-ucs-fe/verification/questions ────────────────────────────────────

pub async fn get_question_list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<GetQuestionListParams>,
) -> Response {
    let merchant_code = extract_header(&headers, "Merchant");
    let customer_ip = extract_header(&headers, "CustomerIP");

    if merchant_code.is_empty() {
        tracing::warn!("missing Merchant header");
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::raw(
                "merchant.param.missing",
                "merchant header is required",
            )),
        )
            .into_response();
    }
    if customer_ip.is_empty() {
        tracing::warn!("missing CustomerIP header");
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::raw(
                "customerip.param.missing",
                "CustomerIP header is required",
            )),
        )
            .into_response();
    }
    if params.customer_name.is_empty() {
        tracing::warn!("missing customerName query param");
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::raw(
                "customerName.param.missing",
                "customerName is required",
            )),
        )
            .into_response();
    }

    tracing::info!(
        merchant_code = %merchant_code,
        ip = %customer_ip,
        customer_name = %params.customer_name,
        "GetQuestionList request"
    );

    match state
        .verification_svc
        .get_question_list(&merchant_code, &customer_ip, &params.customer_name)
        .await
    {
        Ok(result) => {
            tracing::info!(
                merchant_code = %merchant_code,
                questions = result.questions.len(),
                "GetQuestionList success"
            );
            // Mirrors Go's resp.Success(result):
            // {"success":true,"value":{"code":0,"message":"success","data":{...}}}
            Json(ApiSuccess::new(result)).into_response()
        }

        Err(AppError::MerchantNotFound(_)) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::raw(
                "merchant.rule.not_found",
                "Merchant rule not found",
            )),
        )
            .into_response(),

        Err(AppError::QuestionLimitExceeded) => {
            tracing::warn!(
                merchant_code = %merchant_code,
                ip = %customer_ip,
                "question retry limit exhausted"
            );
            (
                StatusCode::TOO_MANY_REQUESTS,
                Json(ErrorResponse::raw(
                    "question.retry.limit_exhausted",
                    "Retry limit reached. Please try again tomorrow.",
                )),
            )
                .into_response()
        }

        Err(AppError::RedisNotFound) => {
            tracing::error!(merchant_code = %merchant_code, "redis instance unavailable");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse::raw(
                    "redis.unavailable",
                    "Service temporarily unavailable, please try again later",
                )),
            )
                .into_response()
        }

        Err(AppError::ParseJsonFailed(e)) => {
            tracing::error!(merchant_code = %merchant_code, error = %e, "failed to parse merchant rule questions");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::raw(
                    "rule.config.invalid",
                    "Invalid merchant rule configuration",
                )),
            )
                .into_response()
        }

        Err(AppError::WpsApiFailed(e)) => {
            tracing::error!(merchant_code = %merchant_code, error = %e, "wps service unavailable");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse::raw(
                    "wps.service.unavailable",
                    "Service temporarily unavailable, please try again later",
                )),
            )
                .into_response()
        }

        Err(AppError::CustomerFetchFailed(e)) => {
            tracing::error!(merchant_code = %merchant_code, error = %e, "failed to fetch customer info");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse::raw(
                    "customer.fetch.failed",
                    "Service temporarily unavailable, please try again later",
                )),
            )
                .into_response()
        }

        // 手机号已绑定 — 返回 200 + WPS 结构体，与 Go 完全一致:
        // {"success":true,"value":{"isEmailResetEnabled":false,"isSmsResetEnabled":true,...}}
        Err(AppError::PhoneAlreadyBound) => Json(ResetPasswordStatusResponse {
            success: true,
            value: ResetPasswordStatusValue {
                is_email_reset_enabled: false,
                is_sms_reset_enabled: true,
                is_personal_info_reset_enabled: false,
            },
        })
        .into_response(),

        // 邮箱已验证 — 返回 200 + WPS 结构体
        Err(AppError::EmailAlreadyBound) => Json(ResetPasswordStatusResponse {
            success: true,
            value: ResetPasswordStatusValue {
                is_email_reset_enabled: true,
                is_sms_reset_enabled: false,
                is_personal_info_reset_enabled: false,
            },
        })
        .into_response(),

        Err(e) => {
            tracing::error!(merchant_code = %merchant_code, error = %e, "GetQuestionList unexpected error");
            e.into_response()
        }
    }
}

// ── POST /tcg-ucs-fe/verification/materials ───────────────────────────────────

pub async fn submit_verify_materials(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<SubmitVerifyRequest>,
) -> Response {
    let merchant_code = extract_header(&headers, "Merchant");
    let customer_ip = extract_header(&headers, "CustomerIP");

    tracing::info!(
        merchant_code = %merchant_code,
        customer_ip   = %customer_ip,
        customer_name = %body.customer_name,
        "SubmitVerifyMaterials request"
    );

    if body.data.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::raw(
                "verify.param.missing",
                "data array is required",
            )),
        )
            .into_response();
    }

    match state
        .verification_svc
        .submit_verify_materials(&merchant_code, &customer_ip, body)
        .await
    {
        Ok(result) => Json(ApiSuccess::new(result)).into_response(),

        Err(AppError::MerchantNotFound(_)) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::raw(
                "merchant.rule.not_found",
                "Merchant rule not found",
            )),
        )
            .into_response(),

        Err(AppError::ParseJsonFailed(e)) => {
            tracing::error!(merchant_code = %merchant_code, error = %e, "failed to parse merchant rule");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::raw(
                    "rule.config.invalid",
                    "Invalid merchant rule configuration",
                )),
            )
                .into_response()
        }

        Err(AppError::CustomerFetchFailed(e)) => {
            tracing::error!(merchant_code = %merchant_code, error = %e, "customer fetch failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::raw(
                    "verify.customer.internal_error",
                    "Internal server error",
                )),
            )
                .into_response()
        }

        Err(AppError::PasswordResetFailed(e)) => {
            tracing::warn!(merchant_code = %merchant_code, error = %e, "password reset failed");
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse::raw(
                    "password.reset.failed",
                    "failed to generate reset token",
                )),
            )
                .into_response()
        }

        Err(AppError::VerifyPlayerInfoFailed(e)) => {
            tracing::error!(merchant_code = %merchant_code, error = %e, "MCS verification failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::raw(
                    "verify.mcs.failed",
                    "MCS verification failed",
                )),
            )
                .into_response()
        }

        Err(e) => {
            tracing::warn!(merchant_code = %merchant_code, error = %e, "SubmitVerifyMaterials failed");
            e.into_response()
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn extract_header(headers: &HeaderMap, name: &str) -> String {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string()
}

// ── Ping ──────────────────────────────────────────────────────────────────────

pub async fn ping() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "Pong": "success" }))
}
