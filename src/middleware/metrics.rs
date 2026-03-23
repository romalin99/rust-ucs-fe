/// Prometheus metrics middleware.
///
/// Full port of Go's `internal/middleware/metrics.go` (`FiberPrometheus`).
///
/// Metrics collected:
///   • `http_requests_total`           — Counter   (method, path, status)
///   • `http_request_duration_seconds` — Histogram (method, path, status)
///   • `http_request_size_bytes`       — Histogram (method, path, status)
///   • `http_response_size_bytes`      — Histogram (method, path, status)
///   • `http_requests_in_progress`     — Gauge     (method, path)
///
/// All metrics carry a const label `service = "<service_name>"` matching Go.
use axum::{
    body::Body,
    http::Request,
    middleware::Next,
    response::Response,
};
use once_cell::sync::Lazy;
use prometheus::{
    CounterVec, GaugeVec, HistogramOpts, HistogramVec, Opts,
    register_counter_vec, register_gauge_vec, register_histogram_vec,
};
use std::time::Instant;

const SERVICE_NAME: &str = "UCS-FE-RUST";

// ── Prometheus metrics (global, registered once) ──────────────────────────────

static REQUESTS_TOTAL: Lazy<CounterVec> = Lazy::new(|| {
    register_counter_vec!(
        Opts::new("http_requests_total", "Total number of HTTP requests")
            .const_label("service", SERVICE_NAME),
        &["method", "path", "status"]
    )
    .expect("http_requests_total registration failed")
});

static REQUEST_DURATION: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        HistogramOpts::new(
            "http_request_duration_seconds",
            "HTTP request duration in seconds",
        )
        .const_label("service", SERVICE_NAME)
        .buckets(vec![0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0]),
        &["method", "path", "status"]
    )
    .expect("http_request_duration_seconds registration failed")
});

static REQUEST_SIZE: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        HistogramOpts::new("http_request_size_bytes", "HTTP request size in bytes")
            .const_label("service", SERVICE_NAME)
            .buckets(vec![64.0, 256.0, 1024.0, 4096.0, 16384.0, 65536.0, 262144.0]),
        &["method", "path", "status"]
    )
    .expect("http_request_size_bytes registration failed")
});

static RESPONSE_SIZE: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        HistogramOpts::new("http_response_size_bytes", "HTTP response size in bytes")
            .const_label("service", SERVICE_NAME)
            .buckets(vec![64.0, 256.0, 1024.0, 4096.0, 16384.0, 65536.0, 262144.0]),
        &["method", "path", "status"]
    )
    .expect("http_response_size_bytes registration failed")
});

static ACTIVE_REQUESTS: Lazy<GaugeVec> = Lazy::new(|| {
    register_gauge_vec!(
        Opts::new("http_requests_in_progress", "Current number of HTTP requests being processed")
            .const_label("service", SERVICE_NAME),
        &["method", "path"]
    )
    .expect("http_requests_in_progress registration failed")
});

// ── Skip paths (mirrors Go's prometheus.go configuration) ─────────────────────

const SKIP_PATHS: &[&str] = &["/ping", "/swagger", "/favicon.ico", "/metrics", "/monitor",
                                "/livez", "/readyz"];

const IGNORE_STATUS_CODES: &[u16] = &[401, 403, 404];

// ── Middleware function ───────────────────────────────────────────────────────

/// Axum middleware that records Prometheus metrics for every request.
///
/// Mirrors Go's `FiberPrometheus.Middleware`.
pub async fn prometheus_metrics(req: Request<Body>, next: Next) -> Response {
    let path   = req.uri().path().to_string();
    let method = req.method().to_string();

    if SKIP_PATHS.iter().any(|p| path.contains(p)) {
        return next.run(req).await;
    }

    ACTIVE_REQUESTS
        .with_label_values(&[&method, &path])
        .inc();

    let start = Instant::now();
    let request_size = req.headers()
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);

    let response = next.run(req).await;

    ACTIVE_REQUESTS
        .with_label_values(&[&method, &path])
        .dec();

    let status_code = response.status().as_u16();

    if IGNORE_STATUS_CODES.contains(&status_code) {
        return response;
    }

    let duration = start.elapsed().as_secs_f64();
    let status   = status_code.to_string();
    let labels   = [method.as_str(), path.as_str(), status.as_str()];

    let response_size = response.headers()
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);

    REQUESTS_TOTAL.with_label_values(&labels).inc();
    REQUEST_DURATION.with_label_values(&labels).observe(duration);
    REQUEST_SIZE.with_label_values(&labels).observe(request_size);
    RESPONSE_SIZE.with_label_values(&labels).observe(response_size);

    response
}
