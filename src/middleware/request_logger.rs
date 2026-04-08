/// Request / response behavior-logger middleware.
///
/// Full port of Go's `internal/middleware/logger.go` (`BehaviorLogger`).
///
/// Log lines (mirrors Go wire format):
/// ```text
/// [API-REQUEST] [START] method=POST uri=/path body={"customerName":"john","data":[...]} addr=1.2.3.4
/// [traceID/spanID] [API-REQUEST] URI: /path, Method: POST, Status: 200, Addr: 1.2.3.4, Elapsed: 5ms
/// ```
///
/// Level routing matches Go's `logRequest`:
///   ≥ 500 → ERROR  ·  ≥ 400 → WARN  ·  else → INFO
///
/// Skip list is identical to Go's:
///   `/test/`, `/metrics`, `/swagger`, `/favicon.ico`, `/health`,
///   `/livez`, `/readyz`, `/ping`, `/monitor`
///
/// Header priority for trace IDs (mirrors Go's `extractTraceIDs`):
///   1. `X-App-Trace-ID` (WPS Relay canonical trace header) / `X-Span-Id`
///   2. `X-Trace-Id` / `X-Span-Id`
///   3. W3C `traceparent`
///   4. Jaeger `uber-trace-id`
///   5. Fallback: `"unknown"`
///
/// Header priority for client IP (mirrors Go's `getClientIP`):
///   1. `CustomerIP` (WPS Relay Pass Header — already resolved by relay)
///   2. `X-Forwarded-For` rightmost public IP
///   3. `X-Real-IP` public IP
///   4. TCP peer address (`ConnectInfo`)
use axum::{
    body::Body,
    extract::ConnectInfo,
    extract::Request,
    middleware::Next,
    response::Response,
};
use http_body_util::BodyExt;
use std::net::{IpAddr, SocketAddr};
use std::time::Instant;

use crate::masking::mask_request_body;

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
    // Check skip-path — use HasPrefix semantics like Go (not Contains).
    let path = req.uri().path().to_owned();
    if SKIP_PATHS.iter().any(|p| path.starts_with(p)) {
        return next.run(req).await;
    }

    let method = req.method().clone();
    let client_ip = extract_client_ip(&req);
    let (trace_id, span_id) = extract_trace_ids(&req);

    // ── [START] log incoming request body (mirrors Go's logIncomingRequest) ──
    // We read the body bytes here, log them (masked), then put them back so the
    // handler can still read them.  axum::body::to_bytes buffers the full body.
    let (parts, body) = req.into_parts();

    let body_bytes = axum::body::to_bytes(body, usize::MAX)
        .await
        .unwrap_or_default();

    if !body_bytes.is_empty() {
        let masked = mask_request_body(&body_bytes);
        let masked_str = String::from_utf8_lossy(&masked);
        tracing::info!(
            "[API-REQUEST] [START] method={method} uri={path} body={masked_str} addr={client_ip}"
        );
    } else {
        tracing::info!("[API-REQUEST] [START] method={method} uri={path} addr={client_ip}");
    }

    // Reconstruct the request with the original body bytes.
    let req = Request::from_parts(parts, Body::from(body_bytes));

    let start = Instant::now();
    let response = next.run(req).await;
    let elapsed = start.elapsed().as_millis();
    let status = response.status().as_u16();

    // ── Buffer response body for logging (mirrors Go's `c.Response().Body()`) ──
    let (parts, body) = response.into_parts();
    let resp_bytes = body
        .collect()
        .await
        .map(|collected| collected.to_bytes())
        .unwrap_or_default();

    let resp_body_str = String::from_utf8_lossy(&resp_bytes);

    // ── [API-RESPONSE] [END] — mirrors Go's logRequest second half ──────────
    // Format: [API-RESPONSE] [END] method=POST uri=/path status=200 elapsed=123ms addr=1.2.3.4 body={...}
    let end_msg = format!(
        "[API-RESPONSE] [END] method={method} uri={path} status={status} elapsed={elapsed}ms addr={client_ip} body={resp_body_str}",
    );

    match status {
        s if s >= 500 => tracing::error!("{end_msg}"),
        400..=499 => tracing::warn!("{end_msg}"),
        _ => tracing::info!("{end_msg}"),
    }

    // ── Behavior log (to file) ──────────────────────────────────────────────
    let behavior_msg = format!(
        "[{trace_id}/{span_id}] [API-REQUEST] URI: {path}, Method: {method}, Status: {status}, Addr: {client_ip}, Elapsed: {elapsed}ms",
    );

    match status {
        s if s >= 500 => tracing::error!(target: "behavior", "{behavior_msg}"),
        400..=499 => tracing::warn!(target: "behavior", "{behavior_msg}"),
        _ => tracing::info!(target: "behavior", "{behavior_msg}"),
    }

    // Reconstruct response with the buffered body.
    Response::from_parts(parts, Body::from(resp_bytes))
}

// ── Trace ID extraction ───────────────────────────────────────────────────────

/// Extract (traceID, spanID) from the request.
///
/// Priority (mirrors Go's `extractTraceIDs`):
///   1. `X-App-Trace-ID` (WPS Relay canonical header) + `X-Span-Id`
///   2. `X-Trace-Id` + `X-Span-Id`
///   3. W3C `traceparent`: `"00-<traceID>-<spanID>-<flags>"`
///   4. Jaeger `uber-trace-id`: `"<traceID>:<spanID>:<parentID>:<flags>"`
///   5. `"unknown"` fallback (same as Go)
fn extract_trace_ids(req: &Request<Body>) -> (String, String) {
    let headers = req.headers();

    // 1. X-App-Trace-ID (WPS Relay canonical trace header — highest priority)
    if let Some(app_trace) = headers.get("X-App-Trace-ID").and_then(|v| v.to_str().ok()) {
        if !app_trace.is_empty() {
            let span = headers
                .get("X-Span-Id")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("unknown")
                .to_string();
            return (app_trace.to_string(), span);
        }
    }

    // 2. X-Trace-Id / X-Span-Id
    let x_trace = headers.get("X-Trace-Id").and_then(|v| v.to_str().ok());
    let x_span = headers.get("X-Span-Id").and_then(|v| v.to_str().ok());
    if x_trace.is_some() || x_span.is_some() {
        return (
            x_trace.map_or_else(|| "unknown".to_string(), ToString::to_string),
            x_span.map_or_else(|| "unknown".to_string(), ToString::to_string),
        );
    }

    // 3. W3C traceparent: "00-<traceID>-<spanID>-<flags>"
    if let Some(tp) = headers.get("traceparent").and_then(|v| v.to_str().ok()) {
        let parts: Vec<&str> = tp.splitn(4, '-').collect();
        if parts.len() == 4 {
            return (parts[1].to_string(), parts[2].to_string());
        }
    }

    // 4. Jaeger uber-trace-id: "<traceID>:<spanID>:<parentID>:<flags>"
    if let Some(j) = headers.get("uber-trace-id").and_then(|v| v.to_str().ok()) {
        let parts: Vec<&str> = j.splitn(4, ':').collect();
        if parts.len() >= 2 {
            return (parts[0].to_string(), parts[1].to_string());
        }
    }

    // 5. Fallback (mirrors Go's "unknown")
    ("unknown".to_string(), "unknown".to_string())
}

// ── Client IP extraction ──────────────────────────────────────────────────────

/// Return the most reliable client IP for this request.
///
/// Priority (mirrors Go's `getClientIP`):
///   1. `CustomerIP` header — WPS Relay Pass Header (relay pre-resolves the real IP)
///   2. `X-Forwarded-For` rightmost public IP
///   3. `X-Real-IP` public IP
///   4. TCP peer address (`ConnectInfo<SocketAddr>`)
///   5. `"-"` fallback
fn extract_client_ip(req: &Request<Body>) -> String {
    let headers = req.headers();

    // 1. CustomerIP (WPS Relay Pass Header — authoritative when present)
    if let Some(cip) = headers
        .get("CustomerIP")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
    {
        if !cip.is_empty() {
            return cip.to_string();
        }
    }

    // 2. X-Forwarded-For — rightmost public IP
    if let Some(xff) = headers.get("X-Forwarded-For").and_then(|v| v.to_str().ok()) {
        for part in xff.split(',').rev() {
            let candidate = part.trim();
            if let Ok(addr) = candidate.parse::<IpAddr>()
                && !addr.is_loopback()
                && !is_private(&addr)
            {
                return candidate.to_string();
            }
        }
    }

    // 3. X-Real-IP
    if let Some(xrip) = headers.get("X-Real-IP").and_then(|v| v.to_str().ok()).map(str::trim)
        && is_public_ip(xrip)
    {
        return xrip.to_string();
    }

    // 4. Direct TCP peer address
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
const fn is_private(addr: &IpAddr) -> bool {
    match addr {
        IpAddr::V4(v4) => v4.is_private(),
        IpAddr::V6(v6) => {
            let seg = v6.segments();
            (seg[0] & 0xfe00) == 0xfc00
        }
    }
}
