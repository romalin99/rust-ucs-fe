/// Route registration.
///
/// Mirrors Go's `internal/router/routes.go` + `prometheus.go`.
///
/// System routes (no auth / no rate-limit):
///   GET  /livez       → "ok"
///   GET  /readyz      → "ok"
///   GET  /monitor     → JSON status
///   GET  /metrics     → Prometheus OpenMetrics
///
/// API routes under `/tcg-ucs-fe`:
///   GET  /ping
///   GET  /pong
///   GET  /hello
///   GET  /health
///   GET  /healthz
///   GET  /monitor
///   GET  /test/quick          (quick timeout + 500 rps)
///   GET  /test/normal
///   GET  /test/long
///   GET  /test/timeout
///   POST /upload
///   POST /upload/v2
///   GET  /verification/questions   (quick timeout + 500 rps)
///   POST /verification/materials
///
/// Rate limits (mirrors Go's `limiter.New` calls):
///   • Global group: 800 rps  (key = IP)
///   • questions/quick: 500 rps  (key = IP)
use std::sync::Arc;
use std::time::Duration;

use axum::{
    Router, middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use axum_prometheus::PrometheusMetricLayer;
use tower::ServiceBuilder;
use tower_governor::{GovernorLayer, governor::GovernorConfigBuilder};
use tower_http::{cors::CorsLayer, timeout::TimeoutLayer};

pub use crate::app_state::AppState;
use crate::handler;
use crate::middleware::{RecoverLayer, behavior_logger};

// ── Router builder ────────────────────────────────────────────────────────────

/// Build the full Axum router.
///
/// Mirrors Go's `NewRouter(...)`.
pub fn build_router(state: Arc<AppState>, quick_timeout_secs: u64) -> Router {
    let (prometheus_layer, metrics_handle) = PrometheusMetricLayer::pair();

    // ── Rate-limit configs ────────────────────────────────────────────────────
    // Global: 800 rps (mirrors Go's `groupLimiter` Max=800)
    let global_cfg = Arc::new(
        GovernorConfigBuilder::default()
            .per_second(800)
            .burst_size(800)
            .finish()
            .expect("invalid global governor config"),
    );
    // Per-path: 500 rps (mirrors Go's `getListLimiter` Max=500)
    let fast_cfg = Arc::new(
        GovernorConfigBuilder::default()
            .per_second(500)
            .burst_size(500)
            .finish()
            .expect("invalid fast governor config"),
    );

    // ── System routes (outside /tcg-ucs-fe, no rate-limit) ───────────────────
    let system_router = Router::new()
        .route("/livez",  get(|| async { "ok" }))
        .route("/readyz", get(|| async { "ok" }))
        .route(
            "/monitor",
            get(|| async {
                axum::Json(serde_json::json!({
                    "status":  "ok",
                    "version": env!("CARGO_PKG_VERSION"),
                }))
                .into_response()
            }),
        )
        .route(
            "/metrics",
            get(move || {
                let h = metrics_handle.clone();
                async move { h.render() }
            }),
        );

    // ── Simple routes (no timeout, global rate-limit) ─────────────────────────
    let simple_router = Router::new()
        .route("/ping",        get(handler::ping))
        .route("/pong",        get(handler::pong))
        .route("/hello",       get(handler::hello))
        .route("/health",      get(handler::health))
        .route("/healthz",     get(handler::health_check))
        .route("/monitor",     get(handler::monitor))
        .route("/test/normal", get(handler::normal))
        .route("/test/long",   get(handler::long))
        .route("/upload",      post(handler::upload))
        .route("/upload/v2",   post(handler::upload_v2));

    // ── Routes with quick timeout + 500 rps ──────────────────────────────────
    let quick_router = Router::new()
        .route("/test/quick",               get(handler::quick))
        .route("/test/timeout",             get(handler::timeout_handler))
        .route("/verification/questions",   get(handler::get_question_list))
        .layer(
            ServiceBuilder::new()
                .layer(TimeoutLayer::with_status_code(axum::http::StatusCode::REQUEST_TIMEOUT, Duration::from_secs(quick_timeout_secs)))
                .layer(GovernorLayer::new(fast_cfg)),
        );

    // ── materials (no timeout override, global rate-limit) ───────────────────
    let materials_router = Router::new().route(
        "/verification/materials",
        post(handler::submit_verify_materials),
    );

    // ── /tcg-ucs-fe  API group (global 800 rps) ───────────────────────────────
    let api_router = Router::new()
        .merge(simple_router)
        .merge(quick_router)
        .merge(materials_router)
        .layer(GovernorLayer::new(global_cfg));

    // ── Assemble full router ───────────────────────────────────────────────────
    //
    // Layer order (outermost first):
    //   1. Prometheus  — records ALL requests
    //   2. RecoverLayer — catches panics
    //   3. CORS
    //   4. BehaviorLogger
    Router::new()
        .merge(system_router)
        .nest("/tcg-ucs-fe", api_router)
        .layer(
            ServiceBuilder::new()
                .layer(prometheus_layer)
                .layer(RecoverLayer)
                .layer(CorsLayer::permissive())
                .layer(middleware::from_fn(behavior_logger)),
        )
        .with_state(state)
}
