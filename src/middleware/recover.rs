/// Panic recovery middleware.
///
/// Mirrors Go's `internal/middleware/recover.go`:
///   catches any `panic!()` inside a handler, logs it at ERROR level,
///   and returns `500 Internal Server Error` with a structured JSON body.
///
/// Axum does not expose panics as `Response`s by default — the connection
/// is simply dropped.  This layer wraps the inner service with
/// `tower::ServiceExt::catch_panic` (via `tower_http`) to intercept them.
use axum::{
    body::Body,
    http::{Request, Response, StatusCode},
    response::IntoResponse,
};
use futures::FutureExt;
use serde_json::json;
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};
use tower::{Layer, Service};

// ── Layer ─────────────────────────────────────────────────────────────────────

/// Tower `Layer` that wraps any service with panic recovery.
#[derive(Debug, Clone, Copy)]
pub struct RecoverLayer;

impl<S> Layer<S> for RecoverLayer {
    type Service = RecoverService<S>;
    fn layer(&self, inner: S) -> Self::Service {
        RecoverService { inner }
    }
}

// ── Service ───────────────────────────────────────────────────────────────────

/// Tower `Service` that catches panics and converts them to 500 JSON responses.
#[derive(Debug, Clone)]
pub struct RecoverService<S> {
    inner: S,
}

impl<S> Service<Request<Body>> for RecoverService<S>
where
    S: Service<Request<Body>, Response = Response<Body>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    type Response = Response<Body>;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        // Extract method and path before moving `req` into the future.
        let method = req.method().clone();
        let path = req.uri().path().to_string();

        let mut inner = self.inner.clone();

        Box::pin(async move {
            // `.catch_unwind()` is from `futures::FutureExt`; it wraps the
            // future in `AssertUnwindSafe` and catches any panic.
            let result: Result<Result<Response<Body>, _>, _> =
                std::panic::AssertUnwindSafe(inner.call(req)).catch_unwind().await;

            match result {
                Ok(response) => response,
                Err(payload) => {
                    let msg: String = if let Some(s) = payload.downcast_ref::<String>() {
                        s.clone()
                    } else if let Some(s) = payload.downcast_ref::<&str>() {
                        s.to_string()
                    } else {
                        "unknown panic".to_string()
                    };

                    // Log with method + path, mirroring Go:
                    //   logs.Err(c, "[PANIC] %s %s => %v", c.Method(), c.Path(), r)
                    tracing::error!("[PANIC] {} {} => {}", method, path, msg);

                    let body = json!({
                        "success":   false,
                        "errorCode": "ucs-fe.non.internal_error",
                        "message":   "Internal server error"
                    });

                    Ok((StatusCode::INTERNAL_SERVER_ERROR, axum::Json(body)).into_response())
                }
            }
        })
    }
}
