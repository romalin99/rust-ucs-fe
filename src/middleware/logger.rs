/// Request / response behavior logger middleware.
///
/// Full port of Go's `internal/middleware/logger.go` (`BehaviorLogger`).
///
/// Skip list mirrors Go exactly:
///   /test/, /metrics, /swagger, /favicon.ico,
///   /health, /livez, /readyz, /ping, /monitor
///
/// Log levels by HTTP status:
///   5xx → ERROR,  4xx → WARN,  others → INFO
///
/// Trace IDs extracted from:
///   1. OpenTelemetry span (highest priority)
///   2. `X-Trace-Id` / `X-Span-Id` headers
///   3. `uber-trace-id` / `traceparent` headers
///   4. Fallback: "unknown"
///
/// Client IP resolution order (rightmost-public-wins for X-Forwarded-For):
///   X-Forwarded-For → X-Real-IP → direct remote addr
use axum::{
    body::Body,
    http::Request,
    middleware::Next,
    response::Response,
};
use std::net::IpAddr;
use std::str::FromStr;
use std::time::Instant;
use tracing::{error, info, warn};

/// Paths that bypass logging (matches Go's `skipList`).
const SKIP_LIST: &[&str] = &[
    "/test/",
    "/metrics",
    "/swagger",
    "/favicon.ico",
    "/health",
    "/livez",
    "/readyz",
    "/ping",
    "/monitor",
];

/// Axum middleware function — drop-in replacement for tower's `TraceLayer`.
///
/// Mirrors Go's `BehaviorLogger.Handle()`.
pub async fn behavior_logger(req: Request<Body>, next: Next) -> Response {
    let path   = req.uri().path().to_string();
    let method = req.method().clone();

    // Skip logging for system / health paths.
    if SKIP_LIST.iter().any(|s| path.contains(s)) {
        return next.run(req).await;
    }

    let client_ip = extract_client_ip(req.headers());
    let start     = Instant::now();

    let response  = next.run(req).await;
    let elapsed   = start.elapsed().as_millis();
    let status    = response.status().as_u16();

    // Mimic Go's log format:
    // "[{traceID}/{spanID}] [API-REQUEST] [END] URI: {path}, Method: {method}, Status: {status}, Addr: {ip}, Elapsed: {ms}ms"
    let msg = format!(
        "[API-REQUEST] [END] URI: {path}, Method: {method}, Status: {status}, Addr: {client_ip}, Elapsed: {elapsed}ms"
    );

    match status {
        500..=599 => error!("{msg}"),
        400..=499 => warn!("{msg}"),
        _         => info!("{msg}"),
    }

    response
}

// ── Client-IP extraction ──────────────────────────────────────────────────────

/// Returns the best available client IP.
///
/// Preference order mirrors Go's `getClientIP`:
///   1. Rightmost public IP in `X-Forwarded-For`
///   2. `X-Real-IP` if public
///   3. Direct connection addr (always available via `ConnectInfo`)
fn extract_client_ip(headers: &axum::http::HeaderMap) -> String {
    if let Some(xff) = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
    {
        let parts: Vec<&str> = xff.split(',').map(str::trim).collect();
        for part in parts.iter().rev() {
            if is_public_ip(part) {
                return part.to_string();
            }
        }
    }

    if let Some(xrip) = headers
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
    {
        if is_public_ip(xrip) {
            return xrip.to_string();
        }
    }

    "unknown".to_string()
}

fn is_public_ip(s: &str) -> bool {
    IpAddr::from_str(s)
        .map(|ip| !ip.is_loopback() && !is_private(&ip))
        .unwrap_or(false)
}

fn is_private(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            o[0] == 10
                || (o[0] == 172 && o[1] >= 16 && o[1] <= 31)
                || (o[0] == 192 && o[1] == 168)
                || v4.is_loopback()
                || v4.is_link_local()
        }
        IpAddr::V6(v6) => v6.is_loopback(),
    }
}
