//! Prometheus metrics middleware.
//!
//! Mirrors Go's `FiberPrometheus` in `internal/middleware/metrics.go`.
//!
//! Instruments:
//! - `http_requests_total`           — counter   {method, path, status, service}
//! - `http_request_duration_seconds` — histogram {method, path, status, service}
//! - `http_requests_in_progress`     — gauge     {method, path, service}
//!
//! The `service` const-label matches Go's `constLabels := prometheus.Labels{"service": serviceName}`.
//!
//! Skipped paths: `/ping`, `/metrics`, `/livez`, `/readyz`, `/monitor`, `/swagger`, `/favicon.ico`.

use axum::{body::Body, extract::Request, middleware::Next, response::Response};
use metrics::{counter, gauge, histogram};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use std::time::Instant;

const SERVICE_NAME: &str = "tcg-ucs-fe";

const SKIP_PATHS: &[&str] = &[
    "/ping",
    "/metrics",
    "/livez",
    "/readyz",
    "/monitor",
    "/swagger",
    "/favicon.ico",
];

/// Build and install the Prometheus recorder with the service const-label;
/// return a handle for the `/metrics` endpoint.
pub fn setup_metrics() -> PrometheusHandle {
    PrometheusBuilder::new()
        .add_global_label("service", SERVICE_NAME)
        .install_recorder()
        .expect("failed to install Prometheus recorder")
}

/// Handler for `GET /metrics` — renders current Prometheus exposition text.
pub async fn metrics_handler(
    axum::extract::State(handle): axum::extract::State<PrometheusHandle>,
) -> impl axum::response::IntoResponse {
    handle.render()
}

/// Per-request middleware that records all three metric families.
pub async fn track_metrics(req: Request<Body>, next: Next) -> Response {
    let path = req.uri().path().to_string();
    let method = req.method().to_string();

    if SKIP_PATHS.iter().any(|p| path.starts_with(p)) {
        return next.run(req).await;
    }

    // Increment in-progress gauge.
    gauge!("http_requests_in_progress",
        "method" => method.clone(),
        "path"   => path.clone(),
    )
    .increment(1.0);

    let start = Instant::now();
    let resp = next.run(req).await;
    let elapsed = start.elapsed().as_secs_f64();

    gauge!("http_requests_in_progress",
        "method" => method.clone(),
        "path"   => path.clone(),
    )
    .decrement(1.0);

    let status = resp.status().as_u16().to_string();

    counter!("http_requests_total",
        "method" => method.clone(),
        "path"   => path.clone(),
        "status" => status.clone(),
    )
    .increment(1);

    histogram!("http_request_duration_seconds",
        "method" => method.clone(),
        "path"   => path.clone(),
        "status" => status,
    )
    .record(elapsed);

    resp
}
