/// Prometheus metrics middleware.
///
/// Full port of Go's `internal/middleware/metrics.go` (`FiberPrometheus`).
///
/// Metrics collected:
///   • `http_requests_total`           — Counter   (method, path, status)
///   • `http_request_duration_seconds` — Histogram (method, path, status)
///   • `http_request_size_bytes`       — Histogram (method, path, status)
///   • `http_response_size_bytes`      — Histogram (method, path, status)
///   • `http_requests_in_progress`     — Gauge     (method, path)  [defined but NOT used — matches Go]
///
/// Note: Go uses `SummaryVec` for size metrics; the `prometheus` Rust crate has no
/// Summary type, so `HistogramVec` with wide buckets is the closest equivalent.
/// PromQL queries differ: Summary uses `{quantile="0.99"}`, Histogram uses
/// `histogram_quantile(0.99, rate(..._bucket[5m]))`.
///
/// All metrics carry a const label `service = "<service_name>"` matching Go.
use axum::{body::Body, http::Request, middleware::Next, response::Response};
use once_cell::sync::Lazy;
use prometheus::{
    CounterVec, HistogramOpts, HistogramVec, Opts, register_counter_vec, register_histogram_vec,
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
        HistogramOpts::new("http_request_duration_seconds", "HTTP request duration in seconds",)
            .const_label("service", SERVICE_NAME)
            .buckets(vec![
                0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0
            ]),
        &["method", "path", "status"]
    )
    .expect("http_request_duration_seconds registration failed")
});

static REQUEST_SIZE: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        HistogramOpts::new("http_request_size_bytes", "HTTP request size in bytes")
            .const_label("service", SERVICE_NAME)
            .buckets(vec![
                64.0, 256.0, 1024.0, 4096.0, 16384.0, 65536.0, 262144.0, 1048576.0
            ]),
        &["method", "path", "status"]
    )
    .expect("http_request_size_bytes registration failed")
});

static RESPONSE_SIZE: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        HistogramOpts::new("http_response_size_bytes", "HTTP response size in bytes")
            .const_label("service", SERVICE_NAME)
            .buckets(vec![
                64.0, 256.0, 1024.0, 4096.0, 16384.0, 65536.0, 262144.0, 1048576.0
            ]),
        &["method", "path", "status"]
    )
    .expect("http_response_size_bytes registration failed")
});

// ── Skip paths (mirrors Go's prometheus.go configuration) ─────────────────────

const SKIP_PATHS: &[&str] = &["/ping", "/swagger", "/favicon.ico", "/metrics", "/monitor"];

// ── Middleware function ───────────────────────────────────────────────────────

/// Axum middleware that records Prometheus metrics for every request.
///
/// Mirrors Go's `FiberPrometheus.Middleware`.
/// Note: Go never increments `activeRequests` gauge; we match that behavior.
pub async fn prometheus_metrics(req: Request<Body>, next: Next) -> Response {
    // Check skip-path with a borrowed &str — no allocation
    if SKIP_PATHS.contains(&req.uri().path()) {
        return next.run(req).await;
    }

    // Only allocate for non-skipped requests that actually need metrics
    let raw_path = req.uri().path().to_string();
    let method = req.method().to_string();

    let matched = req
        .extensions()
        .get::<axum::extract::MatchedPath>()
        .map(|m| m.as_str().to_string())
        .unwrap_or_else(|| raw_path.clone());

    let start = Instant::now();
    let request_size = req
        .headers()
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);

    let response = next.run(req).await;

    let status_code = response.status().as_u16();
    let duration = start.elapsed().as_secs_f64();
    // Stack-allocate the 3-digit status string — no heap allocation
    let mut status_buf = [b'0'; 3];
    let n = status_code as u32;
    status_buf[0] = b'0' + (n / 100) as u8;
    status_buf[1] = b'0' + ((n / 10) % 10) as u8;
    status_buf[2] = b'0' + (n % 10) as u8;
    let status = std::str::from_utf8(&status_buf).unwrap_or("000");
    let labels = [method.as_str(), matched.as_str(), status];

    let response_size = response
        .headers()
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
