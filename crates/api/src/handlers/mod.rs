pub mod health;
pub mod verification;

pub use health::{liveness, ping, readiness};
pub use verification::{get_question_list, submit_verify_materials};

use axum::{
    extract::{FromRequest, Request},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::de::DeserializeOwned;
use serde_json::json;
use tracing::warn;

// ── Custom JSON extractor ─────────────────────────────────────────────────────

/// Wraps axum's `Json` extractor but converts parse failures into
/// `400 {"errorCode":"verify.param.invalid","message":"…","success":false}`
/// matching Go's `ctx.Bind().JSON(&reqBody)` behaviour.
pub struct JsonBody<T>(pub T);

impl<S, T> FromRequest<S> for JsonBody<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = axum::response::Response;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(value)) => Ok(Self(value)),
            Err(rejection) => {
                warn!("JSON body rejected: {}", rejection);
                let body = json!({
                    "errorCode": "verify.param.invalid",
                    "message":   "invalid request body",
                    "success":   false,
                });
                Err((StatusCode::BAD_REQUEST, Json(body)).into_response())
            }
        }
    }
}
