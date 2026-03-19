/// Global error-handler middleware.
///
/// Mirrors Go's `internal/middleware/error_handler.go`.
///
/// In Go this is a Fiber error-handler callback that maps typed errors to HTTP
/// responses.  In Axum, typed domain errors implement `IntoResponse` directly
/// (see `crate::error::AppError::into_response`), so most mapping is done at
/// the handler level.
///
/// This middleware catches **unhandled** errors surfaced by Tower layers
/// (e.g. timeout, body-limit exceeded) and converts them into a consistent
/// JSON envelope so clients never receive a raw 5xx with no body.
use axum::{
    body::Body,
    http::{Request, Response, StatusCode},
    middleware::Next,
    response::IntoResponse,
    Json,
};
use serde_json::json;
use tracing::warn;

// ── AppErrorHandlerLayer ──────────────────────────────────────────────────────

/// Axum middleware function that intercepts non-2xx responses from downstream
/// layers that carry no JSON body, and wraps them in the standard error
/// envelope.
///
/// Usage:
/// ```ignore
/// .layer(axum::middleware::from_fn(error_handler))
/// ```
pub async fn error_handler(req: Request<Body>, next: Next) -> Response<Body> {
    let response = next.run(req).await;

    let status = response.status();

    // Only intercept error responses.
    if status.is_success() || status.is_informational() || status.is_redirection() {
        return response;
    }

    // Check whether the response already carries a JSON Content-Type.
    // If so, pass it through unchanged — the handler already produced a
    // well-formed error envelope.
    let ct = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if ct.contains("application/json") {
        return response;
    }

    // Replace the raw error with a standard JSON envelope.
    let (error_code, message) = map_status(status);

    warn!(status = %status, error_code = %error_code, "error_handler: wrapping non-JSON error response");

    (
        status,
        Json(json!({
            "success":   false,
            "errorCode": error_code,
            "message":   message,
        })),
    )
        .into_response()
}

/// Map common HTTP status codes to `(errorCode, message)` pairs.
///
/// Mirrors Go's `ErrorHandler` switch cases.
fn map_status(status: StatusCode) -> (&'static str, &'static str) {
    match status {
        StatusCode::BAD_REQUEST          => ("ucs-fe.non.bad_request",           "bad request"),
        StatusCode::UNAUTHORIZED         => ("ucs-fe.non.unauthorized",           "unauthorized"),
        StatusCode::FORBIDDEN            => ("ucs-fe.non.forbidden",              "forbidden"),
        StatusCode::NOT_FOUND            => ("ucs-fe.non.not_found",              "resource not found"),
        StatusCode::METHOD_NOT_ALLOWED   => ("ucs-fe.non.method_not_allowed",     "method not allowed"),
        StatusCode::REQUEST_TIMEOUT      => ("ucs-fe.non.request_timeout",        "request timed out"),
        StatusCode::TOO_MANY_REQUESTS    => ("ucs-fe.non.too_many_requests",      "rate limit exceeded"),
        StatusCode::INTERNAL_SERVER_ERROR=> ("ucs-fe.non.unknown_err",            "internal server error"),
        StatusCode::SERVICE_UNAVAILABLE  => ("ucs-fe.non.service_unavailable",    "service unavailable"),
        StatusCode::GATEWAY_TIMEOUT      => ("ucs-fe.non.gateway_timeout",        "gateway timeout"),
        _                                => ("ucs-fe.non.unknown_err",            "unknown error"),
    }
}
