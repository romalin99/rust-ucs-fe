/// Miscellaneous HTTP handlers.
///
/// Full port of Go's `internal/handler/handler.go`.
///
/// Routes:
///   GET  /tcg-ucs-fe/ping              → ping          ({"Pong":"success"})
///   GET  /tcg-ucs-fe/pong              → pong          ("pong" text)
///   GET  /tcg-ucs-fe/hello             → hello         ({"message":"Hello {name}"})
///   GET  /tcg-ucs-fe/health            → health        (200 no CORS headers)
///   GET  /tcg-ucs-fe/healthz           → health_check  ({"status":"ok","time":"..."})
///   GET  /tcg-ucs-fe/monitor           → monitor       ({"status":"ok","version":"..."})
///   GET  /tcg-ucs-fe/test/quick        → quick         (6 s, cancellation-aware)
///   GET  /tcg-ucs-fe/test/normal       → normal        (5 s)
///   GET  /tcg-ucs-fe/test/long         → long          (15 s)
///   GET  /tcg-ucs-fe/test/timeout      → timeout_handler (50 s)
///   POST /tcg-ucs-fe/upload            → upload        (multipart, ≤50 MB)
///   POST /tcg-ucs-fe/upload/v2         → upload_v2     (allowlist extensions)
use std::path::Path;
use std::time::Duration;

use axum::{
    body::Body,
    extract::{Query, Request},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use bytes::Bytes;
use serde::Deserialize;
use tokio::{fs, io::AsyncWriteExt, time};
use tracing::{error, info};

// ── Query params ──────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct HelloQuery {
    #[serde(default = "default_name")]
    pub name: String,
}

fn default_name() -> String {
    "World".to_string()
}

// ── ping ──────────────────────────────────────────────────────────────────────

/// GET /tcg-ucs-fe/ping
///
/// Mirrors Go's `PingHandler`:
/// ```go
/// return c.JSON(fiber.Map{"Pong": "success"})
/// ```
pub async fn ping() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "Pong": "success" }))
}

// ── pong ──────────────────────────────────────────────────────────────────────

/// GET /tcg-ucs-fe/pong
///
/// Mirrors Go's `Ping` (returns raw "pong" string).
pub async fn pong() -> &'static str {
    "pong"
}

// ── hello ─────────────────────────────────────────────────────────────────────

/// GET /tcg-ucs-fe/hello?name=...
///
/// Mirrors Go's `HelloHandler`:
/// ```go
/// name := c.Query("name", "World")
/// return c.JSON(fiber.Map{"message": "Hello " + name})
/// ```
pub async fn hello(Query(q): Query<HelloQuery>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "message": format!("Hello {}", q.name) }))
}

// ── health ────────────────────────────────────────────────────────────────────

/// GET /tcg-ucs-fe/health
///
/// Mirrors Go's `HealthHandler`: returns 200 with no CORS headers and no body.
/// Used by load-balancer probes that reject CORS headers.
pub async fn health() -> Response {
    (
        StatusCode::OK,
        [
            (axum::http::header::CONTENT_TYPE,  "application/json"),
            (axum::http::header::CACHE_CONTROL, "no-cache"),
        ],
        Body::empty(),
    )
        .into_response()
}

// ── health_check ──────────────────────────────────────────────────────────────

/// GET /tcg-ucs-fe/healthz
///
/// Mirrors Go's `HealthCheck`: `{"status":"ok","time":"<RFC3339>"}`.
pub async fn health_check() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "time":   chrono::Utc::now().to_rfc3339(),
    }))
}

// ── monitor ───────────────────────────────────────────────────────────────────

/// GET /tcg-ucs-fe/monitor
///
/// Returns service version + status for internal dashboards.
pub async fn monitor() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status":  "ok",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

// ── timeout_handler ───────────────────────────────────────────────────────────

/// GET /tcg-ucs-fe/test/timeout
///
/// Mirrors Go's `TimeoutHandler`: blocks for 50 s.
/// The router's `TimeoutLayer` fires before this completes in production.
pub async fn timeout_handler() -> Response {
    // Using tokio::select! so the outer TimeoutLayer can cancel us.
    tokio::select! {
        _ = time::sleep(Duration::from_secs(50)) => {
            Json(serde_json::json!({ "Pong": "success" })).into_response()
        }
    }
}

// ── quick ─────────────────────────────────────────────────────────────────────

/// GET /tcg-ucs-fe/test/quick
///
/// Mirrors Go's `QuickHandler`: polls every 500 ms for 6 s total.
pub async fn quick() -> Json<serde_json::Value> {
    let start    = std::time::Instant::now();
    let target   = Duration::from_secs(6);
    let mut tick = time::interval(Duration::from_millis(500));
    tick.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

    loop {
        tick.tick().await;
        if start.elapsed() >= target {
            info!("Quick handler processed");
            return Json(serde_json::json!({
                "code":    200,
                "message": "success",
                "data":    { "type": "quick", "duration": "6s" },
            }));
        }
    }
}

// ── normal ────────────────────────────────────────────────────────────────────

/// GET /tcg-ucs-fe/test/normal
///
/// Mirrors Go's `NormalHandler`: waits 5 s.
pub async fn normal() -> Json<serde_json::Value> {
    time::sleep(Duration::from_secs(5)).await;
    info!("Normal handler processed");
    Json(serde_json::json!({
        "code":    200,
        "message": "success",
        "data": {
            "type":     "normal",
            "duration": "5s",
            "result":   "some database query result",
        },
    }))
}

// ── long ──────────────────────────────────────────────────────────────────────

/// GET /tcg-ucs-fe/test/long
///
/// Mirrors Go's `LongHandler`: waits 15 s.
pub async fn long() -> Json<serde_json::Value> {
    time::sleep(Duration::from_secs(15)).await;
    info!("Long handler processed");
    Json(serde_json::json!({
        "code":    200,
        "message": "success",
        "data": {
            "type":     "long",
            "duration": "15s",
            "result":   "complex calculation result",
        },
    }))
}

// ── upload ────────────────────────────────────────────────────────────────────

/// POST /tcg-ucs-fe/upload
///
/// Mirrors Go's `UploadHandler` (multipart/form-data, field `"file"`).
/// Parses multipart from raw body bytes — avoids the axum `multipart` feature.
pub async fn upload(req: Request) -> Response {
    handle_upload(req, None).await
}

// ── upload_v2 ─────────────────────────────────────────────────────────────────

/// POST /tcg-ucs-fe/upload/v2
///
/// Mirrors Go's `UploadHandlerV2`:
/// - Extension allowlist: `.jpg`, `.png`, `.pdf`, `.xlsx`
/// - Secure filename: `{nanoseconds}{ext}`
pub async fn upload_v2(req: Request) -> Response {
    const ALLOWED: &[&str] = &[".jpg", ".png", ".pdf", ".xlsx"];
    handle_upload(req, Some(ALLOWED)).await
}

// ── shared upload implementation ──────────────────────────────────────────────

async fn handle_upload(req: Request, allowed_exts: Option<&[&str]>) -> Response {
    const MAX_SIZE: usize = 50 * 1024 * 1024;

    // 1. Extract boundary from Content-Type header.
    let boundary = req
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|ct| ct.to_str().ok())
        .and_then(parse_boundary);

    let Some(boundary) = boundary else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "code": 400, "message": "missing multipart boundary" })),
        )
            .into_response();
    };

    // 2. Read entire body (capped at MAX_SIZE + 1 to detect oversize).
    let body_bytes = match axum::body::to_bytes(req.into_body(), MAX_SIZE + 1).await {
        Ok(b)  => b,
        Err(e) => {
            error!("Failed to read body: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "code": 500, "message": "failed to read body" })),
            )
                .into_response();
        }
    };

    if body_bytes.len() > MAX_SIZE {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "code": 400, "message": "file size exceeds limit" })),
        )
            .into_response();
    }

    // 3. Parse the multipart body.
    let field = match parse_multipart_file(&body_bytes, &boundary) {
        Some(f) => f,
        None    => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "code": 400, "message": "file is required" })),
            )
                .into_response();
        }
    };

    // 4. Extension validation (v2 only).
    let ext = Path::new(&field.filename)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{}", e.to_lowercase()))
        .unwrap_or_default();

    if let Some(allowed) = allowed_exts {
        if !allowed.contains(&ext.as_str()) {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "code": 400, "message": "file type not allowed" })),
            )
                .into_response();
        }
    }

    // 5. Build save path and persist.
    let _ = fs::create_dir_all("./uploads").await;

    let (save_name, save_path) = if allowed_exts.is_some() {
        let ts = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let n  = format!("{ts}{ext}");
        let p  = format!("./uploads/{n}");
        (n, p)
    } else {
        let n = format!("{}_{}", chrono::Utc::now().timestamp(), &field.filename);
        let p = format!("./uploads/{n}");
        (n, p)
    };

    if let Err(e) = write_chunks(&save_path, &field.data).await {
        error!("Failed to write file: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "code": 500, "message": "failed to save file" })),
        )
            .into_response();
    }

    let size = field.data.len();
    info!("File uploaded: {save_name}, {size} bytes");
    Json(serde_json::json!({
        "code":    200,
        "message": "file uploaded successfully",
        "data": { "filename": save_name, "size": size, "path": save_path },
    }))
    .into_response()
}

// ── minimal multipart parser ──────────────────────────────────────────────────

struct MultipartField {
    filename: String,
    data:     Bytes,
}

/// Extract `parse_boundary` from a `Content-Type` header value such as
/// `multipart/form-data; boundary=----FormBoundaryXYZ`.
fn parse_boundary(content_type: &str) -> Option<String> {
    content_type
        .split(';')
        .map(str::trim)
        .find(|p| p.to_lowercase().starts_with("boundary="))
        .map(|p| p["boundary=".len()..].trim_matches('"').to_string())
}

/// Parse the first `name="file"` part from a multipart body.
/// Returns `None` when the field is absent.
fn parse_multipart_file(body: &Bytes, boundary: &str) -> Option<MultipartField> {
    let delim      = format!("--{boundary}");
    let body_str   = String::from_utf8_lossy(body);

    for part in body_str.split(&delim) {
        // Skip preamble and epilogue markers.
        if part.starts_with("--") || part.trim().is_empty() {
            continue;
        }

        // Split headers from body at the first blank line.
        let sep = "\r\n\r\n";
        let Some(sep_pos) = part.find(sep) else { continue };
        let headers_str = &part[..sep_pos];
        let raw_body    = &part[sep_pos + sep.len()..];

        // Only process the `name="file"` field.
        let headers_lower = headers_str.to_lowercase();
        if !headers_lower.contains("name=\"file\"") {
            continue;
        }

        // Extract filename from Content-Disposition.
        let filename = headers_str
            .split(';')
            .map(str::trim)
            .find(|p| p.to_lowercase().starts_with("filename="))
            .map(|p| p["filename=".len()..].trim_matches('"').to_string())
            .unwrap_or_else(|| "upload".to_string());

        // Trim the trailing boundary delimiter `\r\n`.
        let data_str = raw_body.trim_end_matches("\r\n");

        return Some(MultipartField {
            filename,
            data: Bytes::copy_from_slice(data_str.as_bytes()),
        });
    }
    None
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Chunked async write — mirrors Go's `ioCopyChunked` (32 KiB buffer).
async fn write_chunks(path: &str, data: &[u8]) -> std::io::Result<()> {
    let mut file = fs::File::create(path).await?;
    for chunk in data.chunks(32 * 1024) {
        file.write_all(chunk).await?;
    }
    file.flush().await
}
