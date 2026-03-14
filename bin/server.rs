//! Application entry point.
//!
//! Start-up sequence (mirrors Go's `newApplication` + `main`):
//!
//!  1. Resolve config file — `-f <path>` CLI flag, then `ENV` env var, then fallback.
//!  2. Load configuration.
//!  3. Initialise structured logging.
//!  4. Build Oracle connection pool.
//!  5. Build Redis client (rate-limit DB).
//!  6. Build USS / MCS HTTP clients.
//!  7. Build repositories.
//!  8. Initialise `FieldCache` (blocks until first DB load completes).
//!  9. Start field-cache background refresh task (30 min).
//! 10. Start cron-job scheduler.
//! 11. Build `VerificationService`.
//! 12. Build axum router with rate limiters and per-route timeouts.
//! 13. Bind listener and serve with graceful shutdown on SIGINT / SIGTERM.

use api::{build, AppState};
use infra::{
    clients::{McsClient, UssClient},
    config::AppConfig,
    db::build_pool,
    redis::build_client as build_redis,
};
use repository::{MerchantRuleRepo, ValidationRecordRepo};
use service::{start_loader, CronScheduler, FieldCache, VerificationService};
use std::{net::SocketAddr, sync::Arc};
use tokio::net::TcpListener;
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ── Config ─────────────────────────────────────────────────────────────────
    // Priority: -f <path>  >  ENV env-var  >  dev.toml fallback
    let explicit_path = std::env::args()
        .skip_while(|a| a != "-f")
        .nth(1)
        .unwrap_or_default();

    let cfg = AppConfig::load(&explicit_path)
        .unwrap_or_else(|e| panic!("failed to load config: {e}"));

    // ── Logging ────────────────────────────────────────────────────────────────
    infra::tracing::init(&cfg.log.level, &cfg.log.format);
    info!("══════════════════════════════════════════════════");
    info!("Service     : {}", cfg.name);
    info!("Env         : {}", cfg.env);
    info!("Listen      : {}", cfg.server_addr());
    info!("Timeout     : {}s (global)", cfg.timeout);
    info!("Quick TOut  : {}s (/questions)", cfg.app_timeouts.quick);
    info!("Normal TOut : {}s (/materials)", cfg.app_timeouts.normal);
    info!(
        "Rate limit  : {} req/s global | {} req/s per-path",
        cfg.rate_limit.max_rps, cfg.rate_limit.per_path_rps
    );
    info!("Body limit  : {} bytes", cfg.body_limit);
    info!("══════════════════════════════════════════════════");

    // ── Oracle ─────────────────────────────────────────────────────────────────
    let oracle_pool = build_pool(&cfg.oracle)
        .unwrap_or_else(|e| panic!("oracle pool init failed: {e}"));

    // ── Redis (rate-limit DB) ──────────────────────────────────────────────────
    let redis_client = build_redis(&cfg.redis)
        .unwrap_or_else(|e| panic!("redis client init failed: {e}"));

    // ── HTTP clients ───────────────────────────────────────────────────────────
    let uss_client = UssClient::new(&cfg.uss_service)
        .unwrap_or_else(|e| panic!("USS client init failed: {e}"));
    let mcs_client = McsClient::new(&cfg.mcs_service)
        .unwrap_or_else(|e| panic!("MCS client init failed: {e}"));

    // ── Repositories ──────────────────────────────────────────────────────────
    let merchant_rule_repo =
        MerchantRuleRepo::new(Arc::clone(&oracle_pool), cfg.oracle.read_timeout_secs);
    let validation_record_repo = ValidationRecordRepo::new(Arc::clone(&oracle_pool));

    // ── Field cache ────────────────────────────────────────────────────────────
    let (field_cache, ready_tx) = FieldCache::new();
    let _loader_handle = start_loader(field_cache.clone(), merchant_rule_repo.clone(), ready_tx);
    info!("FieldCache loader started — waiting for initial load…");
    field_cache.wait_ready_blocking().await;
    info!("FieldCache ready");

    // ── Cron jobs ──────────────────────────────────────────────────────────────
    let _cron = CronScheduler::start(&cfg.jobs);

    // ── Service ────────────────────────────────────────────────────────────────
    let verification_svc = VerificationService::new(
        merchant_rule_repo,
        validation_record_repo,
        uss_client,
        mcs_client,
        field_cache,
        redis_client,
    );

    // ── Router ────────────────────────────────────────────────────────────────
    let app_state = AppState { verification_svc };
    let app_router = build(
        app_state,
        cfg.timeout,
        cfg.app_timeouts.quick,
        cfg.app_timeouts.normal,
        cfg.rate_limit.max_rps,
        cfg.rate_limit.per_path_rps,
    );

    // ── Bind ──────────────────────────────────────────────────────────────────
    let addr: SocketAddr = cfg
        .server_addr()
        .parse()
        .unwrap_or_else(|e| panic!("invalid server addr: {e}"));

    let listener = TcpListener::bind(addr).await?;
    info!("Listening on http://{}", addr);

    // ── Serve with graceful shutdown ──────────────────────────────────────────
    axum::serve(listener, app_router.router)
        .with_graceful_shutdown(shutdown_signal(std::time::Duration::from_secs(
            cfg.shutdown_timeout,
        )))
        .await?;

    info!("Server stopped.");
    Ok(())
}

async fn shutdown_signal(timeout: std::time::Duration) {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install CTRL+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c    => info!("received CTRL+C — starting graceful shutdown…"),
        _ = terminate => info!("received SIGTERM — starting graceful shutdown…"),
    }

    info!("Waiting up to {}s for in-flight requests…", timeout.as_secs());
    tokio::time::sleep(timeout).await;
}
