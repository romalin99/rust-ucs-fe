/// Route registration.
///
/// Mirrors Go's `internal/router/routes.go` + `prometheus.go`:
///
/// ```text
/// GET  /metrics                               Prometheus metrics (OpenMetrics)
/// GET  /livez                                 Liveness probe
/// GET  /readyz                                Readiness probe
/// GET  /monitor                               Simple status JSON
/// GET  /tcg-ucs-fe/ping
/// GET  /tcg-ucs-fe/verification/questions     quick timeout (AppTimeouts.Quick)
/// POST /tcg-ucs-fe/verification/materials
/// ```
///
/// Rate limits (mirrors Go's `limiter.New` calls):
///   • Global group limiter : 800 rps  (key = IP)
///   • questions limiter    : 500 rps  (key = IP)
use std::sync::Arc;
use std::time::Duration;

use axum::{
    Router, middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use axum_prometheus::PrometheusMetricLayer;
use tower::ServiceBuilder;
use tower_governor::{
    GovernorLayer, governor::GovernorConfigBuilder, key_extractor::SmartIpKeyExtractor,
};
use tower_http::{cors::CorsLayer, timeout::TimeoutLayer};

use crate::app_state::AppState;
use crate::handler;
use crate::middleware::{RecoverLayer, behavior_logger};

// ── Router builder ────────────────────────────────────────────────────────────

/// Build the full Axum router.
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
    let questions_cfg = Arc::new(
        GovernorConfigBuilder::default()
            .per_second(500)
            .burst_size(500)
            .finish()
            .expect("invalid questions governor config"),
    );

    // ── System routes (no rate-limit) ─────────────────────────────────────────
    let system_router = Router::new()
        .route("/livez", get(|| async { "ok" }))
        .route("/readyz", get(|| async { "ok" }))
        .route("/monitor", get(monitor_handler))
        .route(
            "/metrics",
            get(move || {
                let handle = metrics_handle.clone();
                async move { handle.render() }
            }),
        );

    // ── /tcg-ucs-fe/ping ─────────────────────────────────────────────────────
    let ping_router = Router::new().route("/ping", get(handler::ping));

    // ── /tcg-ucs-fe/verification/questions  (quick timeout + 500 rps) ────────
    let questions_router = Router::new()
        .route("/verification/questions", get(handler::get_question_list))
        .layer(
            ServiceBuilder::new()
                .layer(TimeoutLayer::new(Duration::from_secs(quick_timeout_secs)))
                .layer(GovernorLayer::new(questions_cfg)),
        );

    // ── /tcg-ucs-fe/verification/materials ───────────────────────────────────
    let materials_router = Router::new().route(
        "/verification/materials",
        post(handler::submit_verify_materials),
    );

    // ── /tcg-ucs-fe  API group (global 800 rps) ───────────────────────────────
    let api_router = Router::new()
        .merge(ping_router)
        .merge(questions_router)
        .merge(materials_router)
        .layer(GovernorLayer::new(global_cfg));

    // ── Assemble full router ───────────────────────────────────────────────────
    //
    // Layer order (outermost first = applied first):
    //   1. Prometheus  — must be outermost so it records ALL requests
    //   2. RecoverLayer — catches panics before they reach the user
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

// ── /monitor handler ──────────────────────────────────────────────────────────

async fn monitor_handler() -> Response {
    axum::Json(serde_json::json!({
        "status":  "ok",
        "version": env!("CARGO_PKG_VERSION"),
    }))
    .into_response()
}
