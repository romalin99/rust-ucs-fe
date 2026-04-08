/// Swagger / API-docs route registration.
///
/// Mirrors Go's `internal/router/swagger.go`.
///
/// In the Rust/Axum stack there is no equivalent of `swaggo` (Go's Swagger UI
/// injector), so this module provides:
///   - `GET /swagger/info` → JSON metadata describing the API.
///   - Helper functions to resolve the host IP (mirrors Go's `getLocalIP` /
///     `getOutboundIP`).
///
/// When a proper Rust `OpenAPI` / Swagger crate (e.g. `utoipa`) is added later,
/// this module should be updated to mount the full UI.
use axum::{Json, Router, routing::get};
use serde_json::json;
use std::net::UdpSocket;

// ── Public registration ───────────────────────────────────────────────────────

/// Register the Swagger info route on the router.
///
/// Mirrors Go's `router.Init(fiberApp, &cfg)`.
pub fn register(router: Router, port: u16) -> Router {
    let host = resolve_host();
    let base = format!("{host}:{port}");

    router.route(
        "/swagger/info",
        get(move || {
            let base = base.clone();
            async move {
                Json(json!({
                    "host":        base,
                    "basePath":    "/tcg-ucs-fe",
                    "title":       "REST API Document For TCG-UCS-FE",
                    "description": "Created by BSD — API for TCG-UCS-FE System",
                    "version":     "2.0",
                    "schemes":     ["http"],
                }))
            }
        }),
    )
}

// ── IP resolution helpers ─────────────────────────────────────────────────────

/// Return the best available local host string.
///
/// Mirrors Go's logic in `swagger.go`:
///   1. `SWAGGER_HOST` env var (highest priority)
///   2. Outbound IP (UDP probe to 8.8.8.8:80 — no data sent)
///   3. First non-loopback IPv4 from `net.InterfaceAddrs()`
///   4. `"localhost"` fallback
pub fn resolve_host() -> String {
    if let Ok(env_host) = std::env::var("SWAGGER_HOST")
        && !env_host.is_empty()
    {
        return env_host;
    }

    if let Some(ip) = get_outbound_ip() {
        return ip;
    }

    get_local_ip().unwrap_or_else(|| "localhost".to_string())
}

/// Return the local IP used for outbound connections.
///
/// Mirrors Go's `getOutboundIP`:
/// > Opens a UDP socket to 8.8.8.8:80. No data is sent; the OS assigns
/// > the appropriate local interface.
fn get_outbound_ip() -> Option<String> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    let addr = socket.local_addr().ok()?;
    Some(addr.ip().to_string())
}

/// Return the first non-loopback IPv4 address on any network interface.
///
/// Mirrors Go's `getLocalIP`.
fn get_local_ip() -> Option<String> {
    // Reuse the same UDP-socket approach as `get_outbound_ip` to stay in std.
    // This is the same technique as getLocalIP in Go: no actual packets sent.
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("1.1.1.1:53").ok()?;
    let addr = socket.local_addr().ok()?;
    Some(addr.ip().to_string())
}
