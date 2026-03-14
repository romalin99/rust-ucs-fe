mod app_state;
mod client;
mod config;
mod error;
mod handler;
mod infra;
mod middleware;
mod model;
mod repository;
mod router;
mod service;
mod telemetry;
mod types;

use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::signal;

use app_state::AppState;
use config::AppConfig;
use infra::AppInfra;
use service::{CommonCronJobs, InitLoadingData, PlayerVerificationService};

// ═══════════════════════════════════════════════════════════════════════════════
// Entry point
// ═══════════════════════════════════════════════════════════════════════════════

/// @title          REST API — UCS-FE (Rust/Axum port)
/// @version        2.0
/// @description    Player Self-Service Password Reset
/// @host           localhost:7009
/// @BasePath       /tcg-ucs-fe
#[tokio::main]
async fn main() -> Result<()> {
    // ── 1. Config ─────────────────────────────────────────────────────────────
    // ENV env-var selects the TOML file (dev / sit / prod).
    // Falls back to "dev" so local development works without extra setup.
    let env = std::env::var("ENV").unwrap_or_else(|_| "dev".to_string());

    let cfg = AppConfig::load_for_env(&env).unwrap_or_else(|e| {
        eprintln!("[FATAL] failed to load config for env={env}: {e}");
        std::process::exit(1);
    });

    // ── 2. Logging ────────────────────────────────────────────────────────────
    telemetry::init_tracing(&cfg.log);
    tracing::info!(env = %env, version = env!("CARGO_PKG_VERSION"), "Configuration loaded");

    // ── 3. AWS Secrets Manager → Oracle credentials ───────────────────────────
    //
    // Mirrors Go's `buildInfra → c.LoadOracleConnectInfoFromAws(envStr)`:
    //   env  → secret path
    //   dev  → tcg-uad/db/go-ucs-fe/dev
    //   sit  → tcg-uad/db/go-ucs-fe/sit
    //   prod → tcg-uad/db/go-ucs-fe
    //
    // Requires env vars (set before starting):
    //   AWS_ACCESS_KEY_ID
    //   AWS_SECRET_ACCESS_KEY
    //   AWS_REGION  (e.g. ap-southeast-1)
    tracing::info!(
        "Loading Oracle credentials from AWS Secrets Manager (env={})",
        env
    );
    let oracle_info = config::load_oracle_connect_info(&env)
        .await
        .context("LoadOracleConnectInfoFromAws failed")?;
    tracing::info!(user = %oracle_info.user, "Oracle credentials loaded");

    // ── 4. Infrastructure (Oracle + Redis + HTTP clients + repositories) ──────
    //
    // Mirrors Go's `NewComManager(odbCfg, ussCfg, mcsCfg, wpsCfg, rdsCfg)`.
    let infra = Arc::new(
        AppInfra::new(&cfg, &oracle_info)
            .await
            .context("AppInfra::new failed")?,
    );
    tracing::info!("Infrastructure initialised");

    let cfg = Arc::new(cfg);

    // ── 5. Services ───────────────────────────────────────────────────────────
    let verification_svc = Arc::new(PlayerVerificationService::new(
        infra.merchant_repo.clone(),
        infra.validation_repo.clone(),
        infra.uss.clone(),
        infra.mcs.clone(),
        infra.wps.clone(),
        infra.redis.clone(),
    ));

    // ── 6. Field-config cache (async initial load + 30-min refresh) ──────────
    // Mirrors Go's `service.NewInitLoadingData(com)`.
    let field_loader = InitLoadingData::start(infra.merchant_repo.clone());

    // ── 7. Cron jobs ──────────────────────────────────────────────────────────
    // Mirrors Go's `service.NewCommonCronJobs(cfg, com)`.
    let cron_jobs = CommonCronJobs::start(&cfg.jobs, infra.merchant_repo.clone());

    // ── 8. Axum router ────────────────────────────────────────────────────────
    let state = AppState::new(cfg.clone(), verification_svc);
    let app = router::build_router(state, cfg.timeouts.quick);

    let addr = format!("{}:{}", cfg.host, cfg.port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("Failed to bind to {addr}"))?;

    display_startup_banner(&cfg, &addr, &env);

    // ── 9. Serve with graceful shutdown ───────────────────────────────────────
    //
    // `into_make_service_with_connect_info` makes `ConnectInfo<SocketAddr>`
    // available in every request's extensions.  tower_governor's
    // PeerIpKeyExtractor (used by GovernorLayer) depends on this; without it
    // every request returns "Unable To Extract Key!" and is rejected.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .context("Axum server error")?;

    // ── 10. Ordered shutdown (mirrors Go's gracefulShutdown) ──────────────────
    //
    // Go order: Fiber → pprof → FlightRecorder → cronJobs → fieldLoader → com → cfg
    // Rust order (no pprof / FlightRecorder needed):
    //   cron jobs → field loader → infra drop
    ordered_shutdown(cron_jobs, field_loader).await;

    tracing::info!("✓ rust-ucs-fe shut down gracefully");
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════════
// Shutdown helpers
// ═══════════════════════════════════════════════════════════════════════════════

/// Wait for SIGINT (Ctrl-C) or SIGTERM.
///
/// Mirrors Go's `signal.Notify(sigCh, syscall.SIGINT, syscall.SIGTERM, syscall.SIGHUP)`.
async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c    => tracing::info!("Received SIGINT, initiating graceful shutdown..."),
        _ = terminate => tracing::info!("Received SIGTERM, initiating graceful shutdown..."),
    }
}

/// Stop all background tasks in the correct order.
///
/// Mirrors Go's `gracefulShutdown` inner goroutine:
///   1. Stop cron jobs.
///   2. Stop field-config loader.
///   (Infrastructure connections are dropped when `AppInfra` goes out of scope.)
async fn ordered_shutdown(cron_jobs: CommonCronJobs, field_loader: InitLoadingData) {
    tracing::info!("Stopping cron jobs...");
    cron_jobs.stop_all();
    tracing::info!("Cron jobs stopped");

    tracing::info!("Stopping field-config loader...");
    field_loader.stop();
    tracing::info!("Field-config loader stopped");

    tracing::info!("Infrastructure connections will be released when dropped");
}

// ═══════════════════════════════════════════════════════════════════════════════
// Startup banner (mirrors Go's displayServerInfos)
// ═══════════════════════════════════════════════════════════════════════════════

fn display_startup_banner(cfg: &AppConfig, addr: &str, env: &str) {
    tracing::info!("═══════════════════════════════════════════════════");
    tracing::info!(
        name    = %cfg.name,
        env     = %env,
        version = env!("CARGO_PKG_VERSION"),
        "rust-ucs-fe"
    );
    tracing::info!("Server URL  : http://{}", addr);
    tracing::info!("Bound on    : {} port {}", cfg.host, cfg.port);
    tracing::info!("Body limit  : {} bytes", cfg.body_limit);
    tracing::info!("Quick timeout: {}s", cfg.timeouts.quick);
    tracing::info!("═══════════════════════════════════════════════════");
}
