/// Request/response logger middleware.
///
/// Mirrors Go's `internal/middleware/logger.go` (`BehaviorLogger`).
///
/// Logs method, path, status, elapsed ms, and real client IP for every
/// non-skipped business request at the appropriate level:
///   ≥ 500 → ERROR · ≥ 400 → WARN · else → INFO
///
/// Skip list matches Go's `BehaviorLogger`:
///   /test/, /metrics, /swagger, /favicon.ico, /health,
///   /livez, /readyz, /ping, /monitor
use axum::{body::Body, extract::Request, middleware::Next, response::Response};
use std::{net::IpAddr, time::Instant};

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

pub async fn behavior_logger(req: Request<Body>, next: Next) -> Response {
    let path = req.uri().path().to_string();
    let method = req.method().clone();

    // Skip non-business paths (mirrors Go's skipList check)
    if SKIP_PATHS.iter().any(|p| path.contains(p)) {
        return next.run(req).await;
    }

    let client_ip = extract_client_ip(&req);

    let start = Instant::now();
    let response = next.run(req).await;
    let elapsed = start.elapsed().as_millis();
    let status = response.status().as_u16();

    match status {
        500..=599 => tracing::error!(
            method = %method, path = %path, status,
            elapsed_ms = elapsed, ip = %client_ip,
            "request completed with server error"
        ),
        400..=499 => tracing::warn!(
            method = %method, path = %path, status,
            elapsed_ms = elapsed, ip = %client_ip,
            "request completed with client error"
        ),
        _ => tracing::info!(
            method = %method, path = %path, status,
            elapsed_ms = elapsed, ip = %client_ip,
            "request completed"
        ),
    }

    response
}

// ── Client IP extraction ──────────────────────────────────────────────────────

/// Return the most reliable public client IP for this request.
///
/// Strategy (mirrors Go's `getClientIP`):
/// 1. Scan `X-Forwarded-For` right-to-left; return the first **public** IP
///    found (resists spoofing by prepending).
/// 2. Fall back to `X-Real-IP` if it is a public IP.
/// 3. Last resort: use the direct remote address from the `X-Real-IP`
///    or the raw `"Forwarded"` header.
/// 4. Return `"-"` if nothing usable is found.
fn extract_client_ip(req: &Request<Body>) -> String {
    let headers = req.headers();

    // 1. X-Forwarded-For — rightmost public IP
    if let Some(xff) = headers.get("X-Forwarded-For").and_then(|v| v.to_str().ok()) {
        let parts: Vec<&str> = xff.split(',').collect();
        for part in parts.iter().rev() {
            let candidate = part.trim();
            if is_public_ip(candidate) {
                return candidate.to_string();
            }
        }
    }

    // 2. X-Real-IP
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
/// Mirrors Go's `isPublicIP(ip string) bool`.
fn is_public_ip(ip: &str) -> bool {
    match ip.parse::<IpAddr>() {
        Ok(addr) => !addr.is_loopback() && !is_private(&addr),
        Err(_) => false,
    }
}

/// Mirrors Go's `net.IP.IsPrivate()` (introduced in Go 1.17).
fn is_private(addr: &IpAddr) -> bool {
    match addr {
        IpAddr::V4(v4) => {
            // RFC 1918 private ranges + link-local
            v4.is_private() || v4.is_link_local()
        }
        IpAddr::V6(v6) => {
            // fc00::/7  (unique-local)  +  fe80::/10 (link-local)
            let seg = v6.segments();
            (seg[0] & 0xfe00) == 0xfc00 || (seg[0] & 0xffc0) == 0xfe80
        }
    }
}
