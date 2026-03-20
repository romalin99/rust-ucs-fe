/// Request / response behavior-logger middleware.
///
/// Full port of Go's `internal/middleware/logger.go` (`BehaviorLogger`).
///
/// Logs every non-skipped business request via `pkg::logs::behavior_*` using
/// the same wire format as the Go middleware:
///
/// ```text
/// [traceID/spanID] [API-REQUEST] [END] URI: /path, Method: POST, Status: 200, Addr: 1.2.3.4, Elapsed: 5ms
/// ```
///
/// Level routing matches Go's `logRequest`:
///   ≥ 500 → `BehaviorError`  ·  ≥ 400 → `BehaviorWarn`  ·  else → `BehaviorInfo`
///
/// Skip list is identical to Go's:
///   `/test/`, `/metrics`, `/swagger`, `/favicon.ico`, `/health`,
///   `/livez`, `/readyz`, `/ping`, `/monitor`

use axum::{body::Body, extract::Request, middleware::Next, response::Response};
use std::{net::IpAddr, time::Instant};

use crate::pkg::logs;

static SKIP_PATHS: &[&str] = &[
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

/// Axum middleware that logs each API request to the behavior log target.
pub async fn behavior_logger(req: Request<Body>, next: Next) -> Response {
    let path   = req.uri().path().to_string();
    let method = req.method().clone();

    // Skip non-business paths (mirrors Go's skipList check).
    if SKIP_PATHS.iter().any(|p| path.contains(p)) {
        return next.run(req).await;
    }

    let client_ip = extract_client_ip(&req);
    let (trace_id, span_id) = extract_trace_ids(&req);

    let start    = Instant::now();
    let response = next.run(req).await;
    let elapsed  = start.elapsed().as_millis();
    let status   = response.status().as_u16();

    // Format mirrors Go's `logRequest` exactly:
    // "[traceID/spanID] [API-REQUEST] [END] URI: …, Method: …, Status: …, Addr: …, Elapsed: …ms"
    let msg = format!(
        "[{}/{}] [API-REQUEST] [END] URI: {}, Method: {}, Status: {}, Addr: {}, Elapsed: {}ms",
        trace_id, span_id, path, method, status, client_ip, elapsed,
    );

    match status {
        500..=599 => logs::behavior_error(&msg),
        400..=499 => logs::behavior_warn(&msg),
        _         => logs::behavior_info(&msg),
    }

    response
}

// ── Trace ID extraction ───────────────────────────────────────────────────────

/// Extract (traceID, spanID) from the request.
///
/// Strategy (mirrors Go's `extractTraceIDs`):
/// 1. `X-Trace-Id` / `X-Span-Id`
/// 2. `traceparent` (W3C Trace Context — traceID is field 2, spanID is field 3)
/// 3. `uber-trace-id` (Jaeger — traceID is field 1, spanID is field 2)
/// 4. Falls back to `"unknown"`.
fn extract_trace_ids(req: &Request<Body>) -> (String, String) {
    let headers = req.headers();

    // 1. Explicit X-Trace-Id / X-Span-Id headers
    let x_trace = header_str(req, "X-Trace-Id");
    let x_span  = header_str(req, "X-Span-Id");
    if x_trace.is_some() || x_span.is_some() {
        return (
            x_trace.unwrap_or("unknown").to_string(),
            x_span.unwrap_or("unknown").to_string(),
        );
    }

    // 2. W3C traceparent: "00-<traceID>-<spanID>-<flags>"
    if let Some(tp) = headers.get("traceparent").and_then(|v| v.to_str().ok()) {
        let parts: Vec<&str> = tp.splitn(4, '-').collect();
        if parts.len() == 4 {
            return (parts[1].to_string(), parts[2].to_string());
        }
    }

    // 3. Jaeger uber-trace-id: "<traceID>:<spanID>:<parentID>:<flags>"
    if let Some(j) = headers.get("uber-trace-id").and_then(|v| v.to_str().ok()) {
        let parts: Vec<&str> = j.splitn(4, ':').collect();
        if parts.len() >= 2 {
            return (parts[0].to_string(), parts[1].to_string());
        }
    }

    ("unknown".to_string(), "unknown".to_string())
}

/// Return a header value as `&str` or `None`.
fn header_str<'a>(req: &'a Request<Body>, name: &str) -> Option<&'a str> {
    req.headers().get(name).and_then(|v| v.to_str().ok())
}

// ── Client IP extraction ──────────────────────────────────────────────────────

/// Return the most reliable public client IP for this request.
///
/// Strategy (mirrors Go's `getClientIP`):
/// 1. Scan `X-Forwarded-For` right-to-left; return the first public IP
///    (resists prepend-spoofing).
/// 2. Fall back to `X-Real-IP` if it is a public IP.
/// 3. Return `"-"` if nothing usable is found.
fn extract_client_ip(req: &Request<Body>) -> String {
    let headers = req.headers();

    if let Some(xff) = headers.get("X-Forwarded-For").and_then(|v| v.to_str().ok()) {
        for part in xff.split(',').rev() {
            let candidate = part.trim();
            if is_public_ip(candidate) {
                return candidate.to_string();
            }
        }
    }

    if let Some(xrip) = headers
        .get("X-Real-IP")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
    {
        if is_public_ip(xrip) {
            return xrip.to_string();
        }
    }

    "-".to_string()
}

/// Returns `true` when `ip` is a valid, non-loopback, non-private address.
///
/// Mirrors Go's `isPublicIP(ip string) bool`.
fn is_public_ip(ip: &str) -> bool {
    match ip.parse::<IpAddr>() {
        Ok(addr) => !addr.is_loopback() && !is_private(&addr),
        Err(_)   => false,
    }
}

/// Mirrors Go 1.17+ `net.IP.IsPrivate()`.
fn is_private(addr: &IpAddr) -> bool {
    match addr {
        IpAddr::V4(v4) => v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            let seg = v6.segments();
            // fc00::/7 unique-local  or  fe80::/10 link-local
            (seg[0] & 0xfe00) == 0xfc00 || (seg[0] & 0xffc0) == 0xfe80
        }
    }
}
