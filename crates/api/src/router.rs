//! Route registration.
//!
//! Mirrors Go's `internal/router/routes.go` and `prometheus.go`.
//!
//! ## Route structure
//!
//! ```text
//! /tcg-ucs-fe
//!   GET  /ping
//!   GET  /verification/questions   → GetQuestionList  (5 s timeout, 500 req/s per-path)
//!   POST /verification/materials   → SubmitVerifyMaterials (global timeout, 500 req/s per-path)
//!
//! /metrics   → Prometheus OpenMetrics text
//! /livez     → Kubernetes liveness probe  (200 OK)
//! /readyz    → Kubernetes readiness probe (200 OK)
//! ```
//!
//! ## Rate-limiting (mirrors Go's fiber/limiter)
//!
//! | Layer          | Max   | Key          | Applied to              |
//! |----------------|-------|--------------|-------------------------|
//! | global         | 800/s | single bucket| all `/tcg-ucs-fe` routes|
//! | per_path       | 500/s | request path | `/questions`, `/materials`|
//!
//! ## Timeouts (mirrors Go's AppTimeouts + timeout.New)
//!
//! | Route                      | Timeout            |
//! |----------------------------|--------------------|
//! | `/verification/questions`  | `quick`   (default 5 s)  |
//! | `/verification/materials`  | `normal`  (default 30 s) |
//! | Everything else            | global timeout (30 s)    |

use crate::{
    handlers,
    middleware::{
        logger::behavior_logger,
        metrics::{metrics_handler, setup_metrics, track_metrics},
        ratelimit::{
            global_rate_limit, new_global_limiter, new_path_limiter, per_path_rate_limit,
            GlobalLimiter, PathLimiter,
        },
        recover::recover,
    },
    state::AppState,
};
use axum::{
    middleware,
    routing::{get, post},
    Extension, Router,
};
use metrics_exporter_prometheus::PrometheusHandle;
use std::time::Duration;
use tower::ServiceBuilder;
use tower_http::{
    cors::{Any, CorsLayer},
    timeout::TimeoutLayer,
};

pub struct AppRouter {
    pub router:      Router,
    pub prom_handle: PrometheusHandle,
}

/// Build the full axum application router.
///
/// # Arguments
/// * `state`           — shared application state injected into handlers.
/// * `timeout_secs`    — global fallback timeout (seconds).
/// * `quick_timeout`   — per-route timeout for `/verification/questions` (seconds).
/// * `normal_timeout`  — per-route timeout for `/verification/materials` (seconds).
/// * `global_rps`      — global bucket rate limit (requests per second).
/// * `per_path_rps`    — per-path bucket rate limit (requests per second).
pub fn build(
    state:          AppState,
    timeout_secs:   u64,
    quick_timeout:  u64,
    normal_timeout: u64,
    global_rps:     u32,
    per_path_rps:   u32,
) -> AppRouter {
    let prom_handle = setup_metrics();

    // ── Rate limiters (shared via Extension) ──────────────────────────────
    let global_limiter: GlobalLimiter = new_global_limiter(global_rps);
    let path_limiter:   PathLimiter   = new_path_limiter(per_path_rps);

    // ── Business routes ───────────────────────────────────────────────────
    //
    // Route-level timeouts follow Go's `timeout.New(handler, cfg)` pattern:
    //   /questions → quickTimeout (5 s by default)
    //   /materials → normalTimeout (30 s by default)
    let api = Router::new()
        .route("/ping", get(handlers::ping))
        // /verification/questions: quick timeout (5 s) + per-path rate limit
        .route(
            "/verification/questions",
            get(handlers::get_question_list)
                .layer(
                    ServiceBuilder::new()
                        .layer(TimeoutLayer::with_status_code(
                            axum::http::StatusCode::REQUEST_TIMEOUT,
                            Duration::from_secs(quick_timeout),
                        ))
                        .layer(middleware::from_fn(per_path_rate_limit)),
                ),
        )
        // /verification/materials: normal timeout (30 s) + per-path rate limit
        .route(
            "/verification/materials",
            post(handlers::submit_verify_materials)
                .layer(
                    ServiceBuilder::new()
                        .layer(TimeoutLayer::with_status_code(
                            axum::http::StatusCode::REQUEST_TIMEOUT,
                            Duration::from_secs(normal_timeout),
                        ))
                        .layer(middleware::from_fn(per_path_rate_limit)),
                ),
        )
        // Inject limiters as extensions and state
        .layer(Extension(path_limiter))
        .layer(Extension(global_limiter.clone()))
        .layer(middleware::from_fn(global_rate_limit))
        .with_state(state);

    // ── Observability routes ──────────────────────────────────────────────
    let obs = Router::new()
        .route("/metrics", get(metrics_handler))
        .with_state(prom_handle.clone())
        .route("/livez",  get(handlers::liveness))
        .route("/readyz", get(handlers::readiness));

    // ── Assemble with global middleware stack ─────────────────────────────
    let router = Router::new()
        .nest("/tcg-ucs-fe", api)
        .merge(obs)
        .layer(
            ServiceBuilder::new()
                .layer(CorsLayer::new().allow_origin(Any))
                .layer(TimeoutLayer::with_status_code(
                    axum::http::StatusCode::REQUEST_TIMEOUT,
                    Duration::from_secs(timeout_secs),
                ))
                .layer(middleware::from_fn(behavior_logger))
                .layer(middleware::from_fn(track_metrics))
                .layer(middleware::from_fn(recover)),
        );

    AppRouter { router, prom_handle }
}
