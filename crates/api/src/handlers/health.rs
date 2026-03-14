//! Health and liveness probe handlers.

use axum::{http::StatusCode, response::IntoResponse, Json};
use serde_json::json;

/// `GET /ping` — basic liveness probe for load balancers.
pub async fn ping() -> impl IntoResponse {
    Json(json!({"message": "pong"}))
}

/// `GET /livez` — Kubernetes liveness probe.
pub async fn liveness() -> impl IntoResponse {
    StatusCode::OK
}

/// `GET /readyz` — Kubernetes readiness probe.
pub async fn readiness() -> impl IntoResponse {
    StatusCode::OK
}
