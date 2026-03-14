//! Panic-recovery middleware.
//!
//! Catches any task-panics that escape a handler and converts them into a
//! structured 500 response, mirroring Go's `middleware.Recover()` implementation.

use axum::{
    body::Body,
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use tracing::error;

#[derive(Serialize)]
struct ErrBody {
    #[serde(rename = "errorCode")]
    error_code: &'static str,
    message: &'static str,
    success: bool,
}

/// Axum middleware that catches panics and returns a 500 JSON response.
///
/// In async Rust, panics in a `tokio::task` (including spawned handlers) do
/// not propagate across task boundaries. However, panics within the
/// request-handling future *do* propagate up through the axum service stack.
/// This middleware catches those with `std::panic::catch_unwind`-style
/// semantics provided by `tower`.
pub async fn recover(req: Request<Body>, next: Next) -> Response {
    // Use tokio's catch_unwind-compatible future to intercept panics.
    match tokio::task::spawn(next.run(req)).await {
        Ok(resp) => resp,
        Err(e) => {
            // The spawned task panicked.
            let msg = if e.is_panic() {
                match e.into_panic().downcast_ref::<&str>() {
                    Some(s) => s.to_string(),
                    None => "unknown panic payload".to_string(),
                }
            } else {
                "task was cancelled".to_string()
            };

            error!("[PANIC] handler panicked: {msg}");

            let body = ErrBody {
                error_code: "ucs-fe.non.internal_error",
                message: "Internal server error",
                success: false,
            };
            (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(body)).into_response()
        }
    }
}
