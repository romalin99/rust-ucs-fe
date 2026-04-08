/// Request / response behavior-logger middleware.
///
/// Full port of Go's `internal/middleware/logger.go` (`BehaviorLogger`).
///
/// Final log line (same wire format as Go):
/// ```text
/// [2026-03-17 10:00:00.000] [ucs-fe] [INFO ] [request_logger.rs:XX] - [traceID/spanID] [API-REQUEST] [END] URI: /path, Method: POST, Status: 200, Addr: 1.2.3.4, Elapsed: 5ms
/// ```
///
/// Level routing matches Go's `logRequest`:
///   ≥ 500 → ERROR  ·  ≥ 400 → WARN  ·  else → INFO
///
/// Skip list is identical to Go's:
///   `/test/`, `/metrics`, `/swagger`, `/favicon.ico`, `/health`,
///   `/livez`, `/readyz`, `/ping`, `/monitor`
use axum::{
    body::Body, extract::ConnectInfo, extract::Request, middleware::Next, response::Response,
};
use std::net::{IpAddr, SocketAddr};
use std::time::Instant;

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
///
/// `tracing` macros are called **directly** in this function (not through
/// wrapper functions) so that the file/line metadata points here, matching
/// Go's `zap.AddCallerSkip(1)` which shows the middleware as caller.
pub async fn behavior_logger(req: Request<Body>, next: Next) -> Response {
    // Check skip-path with borrowed &str — no allocation needed for skipped paths
    if SKIP_PATHS.iter().any(|p| req.uri().path().contains(p)) {
        return next.run(req).await;
    }

    // Only allocate for non-skipped (business API) requests
    let path = req.uri().path().to_string();
    let method = req.method().clone();

    let client_ip = extract_client_ip(&req);
    let (trace_id, span_id) = extract_trace_ids(&req);

    let start = Instant::now();
    let response = next.run(req).await;
    let elapsed = start.elapsed().as_millis();
    let status = response.status().as_u16();

    let msg = format!(
        "[{}/{}] [API-REQUEST] [END] URI: {}, Method: {}, Status: {}, Addr: {}, Elapsed: {}ms",
        trace_id, span_id, path, method, status, client_ip, elapsed,
    );

    // Call tracing macros DIRECTLY so file:line points to this middleware,
    // not to a wrapper function in logs.rs.
    match status {
        s if s >= 500 => tracing::error!(target: "behavior", "{}", msg),
        400..=499 => tracing::warn!(target: "behavior", "{}", msg),
        _ => tracing::info!(target: "behavior", "{}", msg),
    }

    response
}

// ── Trace ID extraction ───────────────────────────────────────────────────────

/// Extract (traceID, spanID) from the request.
///
/// Strategy (mirrors Go's `extractTraceIDs` priority):
///   1. OpenTelemetry span context (Go: `trace.SpanFromContext(ctx)`)
///      — Not available without `tracing-opentelemetry`; skip for now.
///   2. `X-Trace-Id` / `X-Span-Id` explicit headers
///   3. W3C `traceparent`: `"00-<traceID>-<spanID>-<flags>"`
///   4. Jaeger `uber-trace-id`: `"<traceID>:<spanID>:<parentID>:<flags>"`
///   5. Generate a UUID v4 as request-scoped traceID (ensures every log
///      line is traceable even without upstream propagation).
///
/// Go falls back to `"unknown"` in step 5, but generating a UUID per
/// request is strictly better for production debugging.
fn extract_trace_ids(req: &Request<Body>) -> (String, String) {
    let headers = req.headers();

    // 1. Explicit X-Trace-Id / X-Span-Id headers
    let x_trace = headers.get("X-Trace-Id").and_then(|v| v.to_str().ok());
    let x_span = headers.get("X-Span-Id").and_then(|v| v.to_str().ok());
    if x_trace.is_some() || x_span.is_some() {
        return (x_trace.unwrap_or("unknown").to_string(), x_span.unwrap_or("unknown").to_string());
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

    // 4. Generate IDs conforming to W3C / OTel wire format so logs are
    //    consistent whether upstream propagation is present or not.
    //    trace_id = 32 hex chars, span_id = 16 hex chars.
    let raw1 = uuid::Uuid::new_v4().as_u128();
    let raw2 = uuid::Uuid::new_v4().as_u128();
    let trace_id = format!("{:032x}", raw1);
    let span_id = format!("{:016x}", raw2 as u64);
    (trace_id, span_id)
}

// ── Client IP extraction ──────────────────────────────────────────────────────

/// Return the most reliable client IP for this request.
///
/// Strategy (mirrors Go's `getClientIP`):
///   1. `X-Forwarded-For` right-to-left → first public IP
///   2. `X-Real-IP` → if public
///   3. `ConnectInfo<SocketAddr>` → TCP peer address (Go's `c.IP()`)
///   4. `"-"` fallback
fn extract_client_ip(req: &Request<Body>) -> String {
    let headers = req.headers();

    // 1. X-Forwarded-For — rightmost public IP
    if let Some(xff) = headers.get("X-Forwarded-For").and_then(|v| v.to_str().ok()) {
        for part in xff.split(',').rev() {
            let candidate = part.trim();
            // Parse once and reuse — avoids the second parse inside is_public_ip
            if let Ok(addr) = candidate.parse::<IpAddr>() {
                if !addr.is_loopback() && !is_private(&addr) {
                    return candidate.to_string();
                }
            }
        }
    }

    // 2. X-Real-IP
    if let Some(xrip) = headers.get("X-Real-IP").and_then(|v| v.to_str().ok()).map(str::trim) {
        if is_public_ip(xrip) {
            return xrip.to_string();
        }
    }

    // 3. Direct TCP peer address (mirrors Go's `c.IP()`)
    //    Available because main.rs uses `into_make_service_with_connect_info`.
    if let Some(ConnectInfo(addr)) = req.extensions().get::<ConnectInfo<SocketAddr>>() {
        return addr.ip().to_string();
    }

    "-".to_string()
}

/// Returns `true` when `ip` is a valid, non-loopback, non-private address.
fn is_public_ip(ip: &str) -> bool {
    match ip.parse::<IpAddr>() {
        Ok(addr) => !addr.is_loopback() && !is_private(&addr),
        Err(_) => false,
    }
}

/// Mirrors Go 1.17+ `net.IP.IsPrivate()`.
/// Go's IsPrivate covers RFC 1918 (10.x, 172.16-31.x, 192.168.x) and fc00::/7,
/// but NOT link-local (169.254.x.x / fe80::/10).
fn is_private(addr: &IpAddr) -> bool {
    match addr {
        IpAddr::V4(v4) => v4.is_private(),
        IpAddr::V6(v6) => {
            let seg = v6.segments();
            (seg[0] & 0xfe00) == 0xfc00
        }
    }
}
