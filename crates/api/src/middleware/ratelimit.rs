//! Token-bucket rate-limiting middleware.
//!
//! Mirrors Go's `limiter.New` in `internal/router/routes.go`:
//!
//! | Limiter       | Go config                     | Key            | Limit  |
//! |---------------|-------------------------------|----------------|--------|
//! | `global`      | `Max:800, KeyGen:"global"`    | single bucket  | 800/s  |
//! | `per_path`    | `Max:500, KeyGen:c.Path()`    | request path   | 500/s  |
//!
//! Both use a token-bucket algorithm (via the `governor` crate) which
//! closely matches Go Fiber's sliding-window rate limiter behaviour.

use axum::{
    body::Body,
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
    Extension,
};
use governor::{
    clock::DefaultClock,
    middleware::NoOpMiddleware,
    state::{keyed::DefaultKeyedStateStore, InMemoryState, NotKeyed},
    Quota, RateLimiter,
};
use serde::Serialize;
use std::{num::NonZeroU32, sync::Arc};
use tracing::warn;

// ── Type aliases ──────────────────────────────────────────────────────────────

/// Non-keyed (global) rate limiter — single token bucket shared by all requests.
pub type GlobalLimiter =
    Arc<RateLimiter<NotKeyed, InMemoryState, DefaultClock, NoOpMiddleware>>;

/// Path-keyed rate limiter — one token bucket per request path.
pub type PathLimiter = Arc<
    RateLimiter<String, DefaultKeyedStateStore<String>, DefaultClock, NoOpMiddleware>,
>;

// ── Constructors ──────────────────────────────────────────────────────────────

/// Build a global rate limiter allowing `max_rps` requests per second.
pub fn new_global_limiter(max_rps: u32) -> GlobalLimiter {
    let rps = NonZeroU32::new(max_rps).unwrap_or(NonZeroU32::new(800).unwrap());
    Arc::new(RateLimiter::direct(Quota::per_second(rps)))
}

/// Build a per-path rate limiter allowing `max_rps` requests per second per path.
pub fn new_path_limiter(max_rps: u32) -> PathLimiter {
    let rps = NonZeroU32::new(max_rps).unwrap_or(NonZeroU32::new(500).unwrap());
    Arc::new(RateLimiter::keyed(Quota::per_second(rps)))
}

// ── Wire format for 429 responses ────────────────────────────────────────────

#[derive(Serialize)]
struct TooManyBody {
    #[serde(rename = "errorCode")]
    error_code: &'static str,
    message:    &'static str,
    success:    bool,
}

fn too_many_response() -> Response {
    let body = TooManyBody {
        error_code: "rate.limit.exceeded",
        message:    "Too many requests, please slow down",
        success:    false,
    };
    (StatusCode::TOO_MANY_REQUESTS, axum::Json(body)).into_response()
}

// ── Axum middleware functions ─────────────────────────────────────────────────

/// Global rate-limit middleware.
///
/// Expects `Extension<GlobalLimiter>` to be present on the router.
/// All requests share a single token bucket capped at `max_rps` per second.
/// Mirrors Go's `KeyGenerator: func(c fiber.Ctx) string { return "global" }`.
pub async fn global_rate_limit(
    Extension(limiter): Extension<GlobalLimiter>,
    req:  Request<Body>,
    next: Next,
) -> Response {
    if limiter.check().is_err() {
        warn!(
            "global rate limit exceeded for {} {}",
            req.method(),
            req.uri().path()
        );
        return too_many_response();
    }
    next.run(req).await
}

/// Per-path rate-limit middleware.
///
/// Expects `Extension<PathLimiter>` to be present on the router.
/// Each distinct request path gets its own token bucket capped at `per_path_rps`
/// per second. Mirrors Go's `KeyGenerator: func(c fiber.Ctx) string { return c.Path() }`.
pub async fn per_path_rate_limit(
    Extension(limiter): Extension<PathLimiter>,
    req:  Request<Body>,
    next: Next,
) -> Response {
    let path = req.uri().path().to_string();
    if limiter.check_key(&path).is_err() {
        warn!("per-path rate limit exceeded for {}", path);
        return too_many_response();
    }
    next.run(req).await
}
