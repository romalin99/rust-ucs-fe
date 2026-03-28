/// Route registration.
///
/// Mirrors Go's `internal/router/routes.go` + `prometheus.go`.
///
/// System routes (no auth / no rate-limit):
///   GET  /livez       → "ok"
///   GET  /readyz      → "ok"
///   GET  /healthv2    → "ok"
///   GET  /monitor     → JSON status
///   GET  /metrics     → Prometheus OpenMetrics
///
/// API routes under `/tcg-ucs-fe` (global 800 rps):
///   GET  /ping
///   GET  /verification/questions   (path 500 rps + quick timeout)
///   POST /verification/materials   (path 500 rps)
///
/// Rate limits (mirrors Go's `limiter.New` calls in `RegisterHandlers`):
///   • groupLimiter:   800 rps  (key = "global" — all reqs share one bucket)
///   • getListLimiter: 500 rps  (key = request path — per-path bucket)
use std::sync::Arc;
use std::time::Duration;

use axum::{
    Router, middleware,
    routing::{get, post},
};
use prometheus::{TextEncoder, Encoder};
use tower::ServiceBuilder;
use tower_governor::{
    GovernorLayer,
    errors::GovernorError,
    governor::GovernorConfigBuilder,
    key_extractor::{GlobalKeyExtractor, KeyExtractor},
};
use axum::extract::DefaultBodyLimit;
use tower_http::{cors::CorsLayer, timeout::TimeoutLayer};

pub use crate::app_state::AppState;
use crate::handler;
use crate::middleware::{RecoverLayer, behavior_logger, error_handler, otel_trace, prometheus_metrics};
use crate::router::swagger;

// ── Custom key extractor: request path ────────────────────────────────────────

/// Rate-limit key = request URI path.
///
/// Mirrors Go's `getListLimiter` whose `KeyGenerator` returns `c.Path()`.
/// Each unique path gets its own rate-limit bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PathKeyExtractor;

impl KeyExtractor for PathKeyExtractor {
    type Key = String;

    fn extract<T>(&self, req: &axum::http::Request<T>) -> Result<Self::Key, GovernorError> {
        Ok(req.uri().path().to_string())
    }
}

// ── Router builder ────────────────────────────────────────────────────────────

/// Build the full Axum router.
///
/// Mirrors Go's `RegisterHandlers(fiberApp, playerVerification, cfg)`.
pub fn build_router(state: Arc<AppState>, quick_timeout_secs: u64) -> Router {
    let port = state.config.port;

    // ── Rate-limit configs ────────────────────────────────────────────────────

    // Global: 800 rps, key = "global" (all requests share one bucket).
    // Mirrors Go's `groupLimiter` with `KeyGenerator: func(c) string { return "global" }`.
    let mut global_builder = GovernorConfigBuilder::default()
        .key_extractor(GlobalKeyExtractor);
    global_builder.per_second(800);
    global_builder.burst_size(800);
    let global_cfg = Arc::new(
        global_builder.finish().expect("invalid global governor config"),
    );

    // Per-path: 500 rps, key = request path.
    // Mirrors Go's `getListLimiter` with `KeyGenerator: func(c) string { return c.Path() }`.
    let mut path_builder = GovernorConfigBuilder::default()
        .key_extractor(PathKeyExtractor);
    path_builder.per_second(500);
    path_builder.burst_size(500);
    let path_cfg = Arc::new(
        path_builder.finish().expect("invalid path governor config"),
    );

    // ── System routes (outside /tcg-ucs-fe, no rate-limit) ───────────────────
    let system_router = Router::new()
        .route("/livez",    get(|| async { "ok" }))
        .route("/readyz",   get(|| async { "ok" }))
        .route("/healthv2", get(|| async { "ok" }))
        .route(
            "/monitor",
            get(|| async {
                axum::Json(serde_json::json!({
                    "status":  "ok",
                    "version": env!("CARGO_PKG_VERSION"),
                }))
            }),
        )
        .route(
            "/metrics",
            get(|| async {
                let encoder = TextEncoder::new();
                let metric_families = prometheus::gather();
                let mut buffer = Vec::new();
                encoder.encode(&metric_families, &mut buffer).unwrap_or_default();
                (
                    [(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")],
                    buffer,
                )
            }),
        );

    // ── Production API routes under /tcg-ucs-fe ──────────────────────────────
    //
    // Mirrors Go's `RegisterHandlers` exactly:
    //   /ping                         (global limiter only)
    //   /verification/questions       (path limiter + quick timeout)
    //   /verification/materials       (path limiter)

    let ping_router = Router::new()
        .route("/ping", get(handler::ping));

    let questions_router = Router::new()
        .route("/verification/questions", get(handler::get_question_list))
        .layer(
            ServiceBuilder::new()
                .layer(TimeoutLayer::with_status_code(
                    axum::http::StatusCode::REQUEST_TIMEOUT,
                    Duration::from_secs(quick_timeout_secs),
                ))
                .layer(GovernorLayer::new(path_cfg.clone())),
        );

    let materials_router = Router::new()
        .route("/verification/materials", post(handler::submit_verify_materials))
        .layer(GovernorLayer::new(path_cfg));

    // Example / test routes — mirrors Go's `RegisterExamplesHandlers`.
    let normal_timeout_secs = state.config.timeouts.normal;
    let long_timeout_secs = state.config.timeouts.long;
    let upload_timeout_secs = state.config.timeouts.upload;
    let example_router = Router::new()
        .route("/pong", get(handler::pong))
        .route("/hello", get(handler::hello))
        .route("/health", get(handler::health))
        .route("/healthz", get(handler::health_check))
        .route("/monitor", get(handler::monitor))
        .route("/test/quick", get(handler::quick).layer(TimeoutLayer::with_status_code(
            axum::http::StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(quick_timeout_secs),
        )))
        .route("/test/normal", get(handler::normal).layer(TimeoutLayer::with_status_code(
            axum::http::StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(normal_timeout_secs),
        )))
        .route("/test/long", get(handler::long).layer(TimeoutLayer::with_status_code(
            axum::http::StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(long_timeout_secs),
        )))
        .route("/test/timeout", get(handler::timeout_handler))
        .route("/upload", post(handler::upload).layer(TimeoutLayer::with_status_code(
            axum::http::StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(upload_timeout_secs),
        )))
        .route("/upload/v2", post(handler::upload_v2).layer(TimeoutLayer::with_status_code(
            axum::http::StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(upload_timeout_secs),
        )));

    let api_router = Router::new()
        .merge(ping_router)
        .merge(questions_router)
        .merge(materials_router)
        .merge(example_router)
        .layer(GovernorLayer::new(global_cfg));

    let body_limit = state.config.body_limit;

    // ── Assemble full router ───────────────────────────────────────────────────
    //
    // Layer order (outermost first):
    let telemetry_enabled = state.config.telemetry.enabled;

    let mut base = Router::new()
        .merge(system_router)
        .nest("/tcg-ucs-fe", api_router)
        .layer(
            ServiceBuilder::new()
                .layer(middleware::from_fn(prometheus_metrics))
                .layer(RecoverLayer)
                .layer(middleware::from_fn(error_handler))
                .layer(CorsLayer::permissive())
                .layer(DefaultBodyLimit::max(body_limit))
                .layer(middleware::from_fn(behavior_logger)),
        );

    if telemetry_enabled {
        base = base.layer(middleware::from_fn(otel_trace));
        tracing::info!("otel_trace middleware enabled");
    }

    let base = base.with_state(state);

    swagger::register(base, port)
}
