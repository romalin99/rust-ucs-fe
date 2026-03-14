//! Shared application state injected into every axum handler via `State<AppState>`.

use service::VerificationService;

/// Holds all stateful components that handlers need.
///
/// `Clone` is cheap — each field is behind an `Arc` internally.
#[derive(Clone)]
pub struct AppState {
    pub verification_svc: VerificationService,
}
