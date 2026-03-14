//! Request-behavior logging middleware.
//!
//! Mirrors Go's `BehaviorLogger` in `internal/middleware/logger.go`.
//!
//! Logs every non-skipped request after completion:
//! ```text
//! [traceId/spanId] [API-REQUEST] [END] URI: /path Method: POST Status: 200 Addr: 1.2.3.4 Elapsed: 12ms
//! ```
//!
//! Level selection:
//! - `>= 500` → ERROR
//! - `>= 400` → WARN
//! - else     → INFO

use axum::{body::Body, extract::Request, middleware::Next, response::Response};
use std::{net::IpAddr, str::FromStr, time::Instant};
use tracing::{error, info, warn};

/// Paths that should be skipped entirely.
const SKIP_PREFIXES: &[&str] = &[
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

pub async fn behavior_logger(req: Request<Body>, next: Next) -> Response {
    let path = req.uri().path().to_string();
    let method = req.method().to_string();

    // Skip non-business paths.
    if SKIP_PREFIXES.iter().any(|p| path.contains(p)) {
        return next.run(req).await;
    }

    // Extract client IP from X-Forwarded-For / X-Real-IP.
    let client_ip = extract_client_ip(req.headers());

    // Extract trace/span IDs from standard headers.
    let trace_id = req
        .headers()
        .get("X-Trace-Id")
        .or_else(|| req.headers().get("uber-trace-id"))
        .or_else(|| req.headers().get("traceparent"))
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string();

    let span_id = req
        .headers()
        .get("X-Span-Id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string();

    let start = Instant::now();
    let resp = next.run(req).await;
    let elapsed_ms = start.elapsed().as_millis();

    let status: u16 = resp.status().as_u16();
    let msg = format!(
        "[{}/{}] [API-REQUEST] [END] URI: {}, Method: {}, Status: {}, Addr: {}, Elapsed: {}ms",
        trace_id, span_id, path, method, status, client_ip, elapsed_ms
    );

    match status {
        500..=599 => error!("{}", msg),
        400..=499 => warn!("{}", msg),
        _ => info!("{}", msg),
    }

    resp
}

/// Returns the rightmost public IP from X-Forwarded-For, else X-Real-IP,
/// else falls back to an empty string.
///
/// Using the rightmost public IP (not leftmost) resists client-side spoofing
/// because only the last trusted proxy can append to XFF.
fn extract_client_ip(headers: &axum::http::HeaderMap) -> String {
    if let Some(xff) = headers.get("X-Forwarded-For").and_then(|v| v.to_str().ok()) {
        for part in xff.split(',').rev() {
            let ip = part.trim();
            if is_public_ip(ip) {
                return ip.to_string();
            }
        }
    }

    if let Some(xri) = headers.get("X-Real-IP").and_then(|v| v.to_str().ok()) {
        let ip = xri.trim();
        if is_public_ip(ip) {
            return ip.to_string();
        }
    }

    String::new()
}

fn is_public_ip(s: &str) -> bool {
    IpAddr::from_str(s).map_or(false, |ip| {
        !ip.is_loopback()
            && match ip {
                IpAddr::V4(v4) => !v4.is_private() && !v4.is_link_local(),
                IpAddr::V6(v6) => !v6.is_loopback(),
            }
    })
}
