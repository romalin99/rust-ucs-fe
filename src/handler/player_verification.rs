/// HTTP handlers for player verification.
///
/// Full port of Go's `internal/handler/player_verification.go`.
///
/// Routes:
///   GET  /tcg-ucs-fe/verification/questions   → get_question_list
///   POST /tcg-ucs-fe/verification/materials    → submit_verify_materials
///
/// Error convention (mirrors Go exactly):
///   - Parameter validation errors → 400
///   - ALL business / service errors → **500** (Go uses `fiber.StatusInternalServerError`)
///   - PhoneAlreadyBound / EmailAlreadyBound → 200 (business-normal, not errors)
///   - errorCode prefix: `ucsfe.questions.*` / `ucsfe.materials.*`
use axum::{
    extract::{Query, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json, Response},
};
use std::borrow::Cow;
use std::sync::Arc;

use crate::app_state::AppState;
use crate::client::wps::{ResetPasswordStatusResponse, ResetPasswordStatusValue};
use crate::error::{ApiSuccess, AppError};
use crate::types::req::{GetQuestionListParams, SubmitVerifyRequest};
use crate::types::resp::ErrResponse;

// ── GET /tcg-ucs-fe/verification/questions ────────────────────────────────────

/// Mirrors Go's `PlayerVerification.GetQuestionList`.
///
/// All service-level errors return **500 Internal Server Error** (matching Go).
pub async fn get_question_list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<GetQuestionListParams>,
) -> Response {
    let merchant_code = extract_header(&headers, "Merchant");
    let customer_ip = extract_header(&headers, "CustomerIP");
    let language = extract_header(&headers, "Language");

    // ── Parameter validation (400) ────────────────────────────────────────────
    if merchant_code.is_empty() {
        tracing::warn!("missing Merchant header");
        return err_response(
            StatusCode::BAD_REQUEST,
            "ucsfe.questions.merchant_param_missing",
            "merchant header is required",
        );
    }
    if customer_ip.is_empty() {
        tracing::warn!("missing CustomerIP header");
        return err_response(
            StatusCode::BAD_REQUEST,
            "ucsfe.questions.customerip_param_missing",
            "CustomerIP header is required",
        );
    }
    if language.is_empty() {
        tracing::warn!("missing Language header");
        return err_response(
            StatusCode::BAD_REQUEST,
            "ucsfe.questions.language_param_missing",
            "Language header is required",
        );
    }
    if params.customer_name.is_empty() {
        tracing::warn!("missing customerName query param");
        return err_response(
            StatusCode::BAD_REQUEST,
            "ucsfe.questions.customer_name_param_missing",
            "customerName is required",
        );
    }

    tracing::info!(
        "request: merchant={} ip={} customerName={}",
        merchant_code,
        customer_ip,
        params.customer_name
    );

    // ── Dispatch to service ───────────────────────────────────────────────────
    match state
        .verification_svc
        .get_question_list(
            &merchant_code,
            &customer_ip,
            &params.customer_name,
            &language,
        )
        .await
    {
        Ok(result) => {
            tracing::info!(
                "success: merchant={}, customerName={}, questions count={}",
                merchant_code,
                params.customer_name,
                result.questions.len()
            );
            Json(ApiSuccess::new(result)).into_response()
        }

        // ── MerchantNotFound → 500 (Go: StatusInternalServerError) ────────────
        Err(AppError::MerchantNotFound(_)) => err_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "ucsfe.questions.merchant_rule_not_found",
            "Merchant rule not found",
        ),

        // ── QuestionLimitExceeded → 500 (Go: StatusInternalServerError) ───────
        Err(AppError::QuestionLimitExceeded) => {
            tracing::warn!(
                "question retry limit exhausted: merchant={} ip={} customerName={}",
                merchant_code,
                customer_ip,
                params.customer_name
            );
            err_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "ucsfe.questions.retry_limit_exhausted",
                "Retry limit reached. Please try again tomorrow.",
            )
        }

        // ── RedisNotFound → 500 ───────────────────────────────────────────────
        Err(AppError::RedisNotFound) => {
            tracing::error!(
                "redis instance unavailable: merchant={} err=redis instance not found",
                merchant_code
            );
            err_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "ucsfe.questions.redis_unavailable",
                "Service temporarily unavailable, please try again later",
            )
        }

        // ── ParseJsonFailed → 500 ────────────────────────────────────────────
        Err(AppError::ParseJsonFailed(ref e)) => {
            tracing::error!(
                "failed to parse merchant rule questions: merchant={} err={}",
                merchant_code,
                e
            );
            err_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "ucsfe.questions.rule_config_invalid",
                "Invalid merchant rule configuration",
            )
        }

        // ── WpsApiFailed → 500 ───────────────────────────────────────────────
        Err(AppError::WpsApiFailed(ref e)) => {
            tracing::error!(
                "wps service unavailable: merchant={} err={}",
                merchant_code,
                e
            );
            err_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "ucsfe.questions.wps_service_unavailable",
                "Service temporarily unavailable, please try again later",
            )
        }

        // ── CustomerFetchFailed → 500 ────────────────────────────────────────
        Err(AppError::CustomerFetchFailed(ref e)) => {
            tracing::error!(
                "failed to fetch customer info: merchant={} customerName={} err={}",
                merchant_code,
                params.customer_name,
                e
            );
            err_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "ucsfe.questions.customer_not_exists",
                "Service temporarily unavailable, please try again later",
            )
        }

        // ── PhoneAlreadyBound → 200 (business-normal) ───────────────────────
        Err(AppError::PhoneAlreadyBound) => Json(ResetPasswordStatusResponse {
            success: true,
            value: ResetPasswordStatusValue {
                is_email_reset_enabled: false,
                is_sms_reset_enabled: true,
                is_personal_info_reset_enabled: false,
            },
        })
        .into_response(),

        // ── EmailAlreadyBound → 200 (business-normal) ───────────────────────
        Err(AppError::EmailAlreadyBound) => Json(ResetPasswordStatusResponse {
            success: true,
            value: ResetPasswordStatusValue {
                is_email_reset_enabled: true,
                is_sms_reset_enabled: false,
                is_personal_info_reset_enabled: false,
            },
        })
        .into_response(),

        // ── Unexpected → 500 ─────────────────────────────────────────────────
        Err(e) => {
            tracing::error!(
                "GetQuestionList unexpected error: merchant={} err={}",
                merchant_code,
                e
            );
            e.into_response()
        }
    }
}

// ── POST /tcg-ucs-fe/verification/materials ───────────────────────────────────

/// Mirrors Go's `PlayerVerification.SubmitVerifyMaterials`.
///
/// All service-level errors return **500 Internal Server Error** (matching Go).
pub async fn submit_verify_materials(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body_result: Result<Json<SubmitVerifyRequest>, JsonRejection>,
) -> Response {
    let merchant_code = extract_header(&headers, "Merchant");
    let customer_ip = extract_header(&headers, "CustomerIP");

    let body = match body_result {
        Ok(Json(b)) => b,
        Err(e) => {
            tracing::warn!("bind json failed: {}", e);
            return err_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "ucsfe.materials.param_invalid",
                "invalid request body",
            );
        }
    };

    tracing::info!(
        "Received merchant={}, customerIP={}, body={:?}",
        merchant_code,
        customer_ip,
        body
    );

    if body.data.is_empty() {
        return err_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "ucsfe.materials.param_missing",
            "data array is required",
        );
    }

    match state
        .verification_svc
        .submit_verify_materials(&merchant_code, &customer_ip, body)
        .await
    {
        Ok(result) => Json(ApiSuccess::new(result)).into_response(),

        Err(AppError::MerchantNotFound(_)) => err_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "ucsfe.materials.merchant_rule_not_found",
            "Merchant rule not found",
        ),

        Err(AppError::ParseJsonFailed(ref e)) => {
            tracing::error!(
                "failed to parse merchant rule: merchant={} err={}",
                merchant_code,
                e
            );
            err_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "ucsfe.materials.rule_config_invalid",
                "Invalid merchant rule configuration",
            )
        }

        Err(AppError::CustomerFetchFailed(_))
        | Err(AppError::CustomerPersonalInfoFetchFailed(_)) => err_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "ucsfe.materials.customer_not_exists",
            "Internal server error",
        ),

        Err(AppError::PasswordResetFailed(_)) => err_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "ucsfe.materials.password_reset_failed",
            "failed to generate reset token",
        ),

        Err(AppError::VerifyPlayerInfoFailed(ref e)) => {
            tracing::error!(
                "MCS verification failed: merchant={} err={}",
                merchant_code,
                e
            );
            err_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "ucsfe.materials.mcs_verify_failed",
                "MCS verification failed",
            )
        }

        Err(e) => {
            tracing::warn!("SubmitVerifyMaterials failed: {}", e);
            e.into_response()
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn extract_header<'a>(headers: &'a HeaderMap, name: &str) -> Cow<'a, str> {
    match headers.get(name).and_then(|v| v.to_str().ok()) {
        Some(val) => Cow::Borrowed(val),
        None => Cow::Borrowed(""),
    }
}

/// Build a typed error response.
///
/// Mirrors Go's `ctx.Status(code).JSON(resp.NewErrResponse(errorCode, message))`.
fn err_response(status: StatusCode, error_code: &str, message: &str) -> Response {
    (status, Json(ErrResponse::new(error_code, message))).into_response()
}
