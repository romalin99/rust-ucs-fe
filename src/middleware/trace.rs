/// Request tracing middleware.
///
/// Full port of Go's `internal/middleware/trace.go` (`EnableOtelTrace`).
///
/// Since `opentelemetry` is not a direct dependency in this build, this
/// middleware provides equivalent behaviour using only `tracing`:
///   - Creates a structured `tracing::info_span!` per request.
///   - Reads W3C `traceparent` / `X-Trace-Id` headers and attaches them as
///     span fields so they appear in JSON log output.
///   - Skips paths in `SKIP_PATHS` (mirrors Go's `OtelConfig.SkipPaths`).
///
/// When `opentelemetry` is added to `Cargo.toml` in the future the span
/// created here can be promoted to a full `OTel` span by calling
/// `span.set_parent(...)` via `tracing_opentelemetry`.
use axum::{body::Body, http::Request, middleware::Next, response::Response};
use tracing::Instrument;

// ── Skip paths (mirrors Go's `OtelConfig.SkipPaths`) ─────────────────────────

const SKIP_PATHS: &[&str] = &[
    "/metrics",
    "/livez",
    "/readyz",
    "/favicon.ico",
    "/ping",
    "/monitor",
];

// ── Middleware function ───────────────────────────────────────────────────────

/// Axum middleware that creates a `tracing` span for each request and
/// attaches trace-ID information extracted from incoming headers.
///
/// Mirrors Go's `EnableOtelTrace(cfg OtelConfig) fiber.Handler`.
pub async fn otel_trace(req: Request<Body>, next: Next) -> Response {
    let path = req.uri().path().to_string();
    let method = req.method().to_string();

    // Skip probe / system paths (mirrors Go's `WithNext` filter).
    if SKIP_PATHS.contains(&path.as_str()) {
        return next.run(req).await;
    }

    // Extract trace propagation headers.
    // Priority mirrors Go's extractTraceIDs:
    //   1. X-App-Trace-ID (WPS Relay canonical header — highest priority)
    //   2. X-Trace-Id
    //   3. traceparent (W3C)
    //   4. uber-trace-id (Jaeger)
    //   5. "unknown" fallback
    let headers = req.headers();
    let trace_id = headers
        .get("X-App-Trace-ID")
        .or_else(|| headers.get("X-Trace-Id"))
        .or_else(|| headers.get("traceparent"))
        .or_else(|| headers.get("uber-trace-id"))
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty())
        .unwrap_or("unknown")
        .to_string();

    let span_id = headers
        .get("X-Span-Id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string();

    // Create a tracing span that mirrors Go's span name: "{METHOD} {path}".
    let span = tracing::info_span!(
        "http_request",
        otel.name    = format!("{method} {path}"),
        http.method  = %method,
        http.target  = %path,
        trace_id     = %trace_id,
        span_id      = %span_id,
    );

    next.run(req).instrument(span).await
}
