/// Shared application state injected into every Axum handler via `State<Arc<AppState>>`.
///
/// Mirrors Go's dependency-injection pattern (infra.ComManager + service layer).
use std::sync::Arc;

use crate::config::AppConfig;
use crate::service::PlayerVerificationService;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub verification_svc: Arc<PlayerVerificationService>,
}

impl AppState {
    pub fn new(
        config: Arc<AppConfig>,
        verification_svc: Arc<PlayerVerificationService>,
    ) -> Arc<Self> {
        Arc::new(Self {
            config,
            verification_svc,
        })
    }
}
