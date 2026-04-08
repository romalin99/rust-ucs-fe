// Ported Go functionality: many items are defined but not yet all wired up.
// Suppress warnings during this porting phase.
#![allow(dead_code, unused_imports)]

/// Application entry point.
///
/// Start-up sequence mirrors Go's `newApplication` + `main` in `cmd/api/main.go`:
///   1. Load config (`config/{ENV}.toml`, ENV env-var)
///   2. Init structured logging (`telemetry::init_tracing`)
///   3. Load Oracle credentials (AWS Secrets Manager → config fallback)
///   4. Build all infrastructure (Oracle pool, Redis, HTTP clients, repos)
///   5. Start field-config loader (initial load + 30-min periodic refresh)
///   6. Start cron scheduler
///   7. Build domain service → `AppState` → router
///   8. Bind listener; serve with graceful shutdown on `SIGINT` / `SIGTERM`
use std::{net::SocketAddr, sync::Arc};

use tokio::{net::TcpListener, signal};
use tracing::info;

mod app_state;
mod client;
mod config;
mod error;
mod handler;
mod infra;
mod masking;
mod middleware;
mod model;
mod observability;
mod pkg;
mod repository;
mod router;
mod service;
mod telemetry;
mod types;

use crate::{
    app_state::AppState,
    config::{AppConfig, OracleConnectInfo, load_oracle_connect_info},
    infra::AppInfra,
    observability::FlightRecorder,
    pkg::memstats,
    router::build_router,
    service::{
        CommonCronJobs, FieldIdUssMappingLoader, InitLoadingData, PlayerVerificationService,
    },
};

// ── Application container ─────────────────────────────────────────────────────

/// Holds all runtime components in dependency order.
/// Mirrors Go's `application` struct in `cmd/api/main.go`.
struct Application {
    infra: Arc<AppInfra>,
    cron_jobs: CommonCronJobs,
    field_loader: InitLoadingData,
    uss_mapping_loader: FieldIdUssMappingLoader,
    flight_recorder: Option<FlightRecorder>,
    mem_stats_handle: memstats::StopHandle,
    oracle_monitor_stop: tokio::sync::watch::Sender<bool>,
    router: axum::Router,
    cfg: AppConfig,
}

impl Application {
    /// Builds all layers in dependency order:
    ///   infra → repos → field cache → cron → service → state → router
    ///
    /// Mirrors Go's `newApplication`.
    async fn new(cfg: AppConfig) -> anyhow::Result<Self> {
        // ── Flight recorder (SIGUSR1/SIGUSR2 → diagnostic snapshot) ──────────
        let flight_recorder = FlightRecorder::new()
            .map_err(|e| tracing::warn!(error = %e, "flight recorder failed to start"))
            .ok();

        // ── Prometheus metrics ──────────────────────────────────────────────────
        // Mirrors Go's `metrics.Init(cfg.Log.ServiceName)`.
        crate::pkg::metrics::init(&cfg.log.service_name);

        // ── Oracle credentials ────────────────────────────────────────────────
        // Mirrors Go's `buildInfra` → `c.LoadOracleConnectInfoFromAws(envStr)`.
        // Falls back to values already present in the TOML config if AWS fails
        // (useful in local / dev environments).
        let oracle_info = load_oracle_connect_info(&cfg.env).await.unwrap_or_else(|e| {
            tracing::warn!(
                error = %e,
                "AWS oracle creds load failed — falling back to config values"
            );
            OracleConnectInfo {
                user: cfg.oracle.user.clone(),
                password: cfg.oracle.password.clone(),
                connect_string: cfg.oracle.connect_string.clone(),
            }
        });

        // ── Infrastructure (Oracle pool, Redis, HTTP clients, repos) ─────────
        let infra = Arc::new(AppInfra::new(&cfg, &oracle_info).await?);

        // ── Redis multi-DB (DB 2 for rate limiting) ─────────────────────────
        // Mirrors Go's `rdsCfg.InitDBSV2()`.
        crate::infra::init_redis_multi_db(&cfg.redis.addr, &cfg.redis.password, &cfg.redis.dbs)
            .await;

        // ── Field-config loader (initial load + 30-min periodic refresh) ─────
        // Mirrors Go's `service.NewInitLoadingData(com)`.
        let field_loader = InitLoadingData::start(infra.merchant_repo.clone());

        // ── USS mapping loader (initial load + 30-min periodic refresh) ──────
        // Mirrors Go's `service.NewFieldIdUssMappingLoader(com)`.
        let uss_mapping_loader = FieldIdUssMappingLoader::start(infra.uss_mapping_repo.clone());

        // ── Cron scheduler ────────────────────────────────────────────────────
        // Mirrors Go's `service.NewCommonCronJobs(cfg, com)`.
        let cron_jobs = CommonCronJobs::start(
            &cfg.jobs,
            infra.merchant_repo.clone(),
            infra.uss_mapping_repo.clone(),
        );

        // ── Oracle pool monitor ────────────────────────────────────────────────
        // Mirrors Go's `oracle.Config.monitorOraclePool` goroutine.
        let oracle_monitor_stop = crate::infra::start_oracle_pool_monitor(
            infra.oracle_pool.clone(),
            cfg.oracle.stats_interval,
            "oracle-main-pool",
        );

        // ── Memstats background task ─────────────────────────────────────────
        // Mirrors Go's `gos.GoSafe(func() { memstatus.MemStats(memStatsCtx) })`.
        let mem_stats_handle = memstats::start_mem_stats();

        // ── Domain service ────────────────────────────────────────────────────
        // Mirrors Go's `service.NewPlayerVerificationService(cfg, com)`.
        let player_svc = PlayerVerificationService::new(
            infra.merchant_repo.clone(),
            infra.validation_repo.clone(),
            infra.uss.clone(),
            infra.mcs.clone(),
            infra.wps.clone(),
            infra.redis.clone(),
        );

        // ── Handlers → state → router ─────────────────────────────────────────
        let state = AppState::new(Arc::new(cfg.clone()), Arc::new(player_svc));
        let router = build_router(state, cfg.timeouts.quick);

        Ok(Self {
            infra,
            cron_jobs,
            field_loader,
            uss_mapping_loader,
            flight_recorder,
            mem_stats_handle,
            oracle_monitor_stop,
            router,
            cfg,
        })
    }

    /// Graceful shutdown in reverse dependency order.
    /// Mirrors Go's `gracefulShutdown`.
    async fn shutdown(self) {
        info!("graceful shutdown starting...");

        if let Some(fr) = self.flight_recorder {
            fr.stop().await;
            info!("flight recorder stopped");
        }

        self.cron_jobs.stop_all();
        info!("cron scheduler stopped");

        self.field_loader.stop();
        info!("field config loader stopped");

        self.uss_mapping_loader.stop();
        info!("USS mapping loader stopped");

        let _ = self.oracle_monitor_stop.send(true);
        info!("oracle pool monitor stopped");

        self.mem_stats_handle.stop();
        info!("memstats task stopped");

        crate::pkg::concurrency::release_pool();
        info!("task pool released");

        crate::infra::close_redis_multi_db();
        info!("Redis multi-DB instances closed");

        // AppInfra connections close when the last Arc drops.
        drop(self.infra);
        info!("infrastructure connections closed");

        crate::telemetry::shutdown();
        info!("telemetry shutdown complete");

        info!("graceful shutdown complete");

        // Final log buffer flush so the last lines are written to stdout.
        crate::pkg::logs::close();
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

/// @title    REST API Document for UCS-FE (Rust/Axum)
/// @version  2.0
/// @host     localhost:7009
/// @basePath /tcg-ucs-fe
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ── Config ────────────────────────────────────────────────────────────────
    // Priority: ENV env-var → config/{env}.toml → built-in defaults.
    // Mirrors Go's `cfg.Init("")`.
    let cfg = AppConfig::load().unwrap_or_else(|e| {
        eprintln!("config load failed ({e}) — using built-in defaults");
        AppConfig::default()
    });

    // ── Structured logging ────────────────────────────────────────────────────
    // Must happen BEFORE any `tracing::*` calls.
    // Mirrors Go's `cfg.InitLog()` + `zlog.Init(cfg)`.
    telemetry::init_tracing(&cfg.log);

    let addr: SocketAddr = format!("{}:{}", cfg.host, cfg.port).parse()?;

    // ── Build application ─────────────────────────────────────────────────────
    info!("building application...");
    let app = Application::new(cfg).await?;

    // ── Bind listener ─────────────────────────────────────────────────────────
    let listener = TcpListener::bind(addr).await?;

    info!("═══════════════════════════════════════════════════");
    info!("  UCS-FE  (Rust/Axum)");
    info!("  listening on  http://{}", addr);
    info!("  metrics       http://{}/metrics", addr);
    info!("  swagger       http://{}/swagger/", addr);
    info!("  liveness      http://{}/livez", addr);
    info!("  readiness     http://{}/readyz", addr);
    info!("═══════════════════════════════════════════════════");

    // ── Serve with graceful shutdown ─────────────────────────────────────────
    // `into_make_service_with_connect_info` injects `ConnectInfo<SocketAddr>`
    // into every request, which tower-governor's key extractors may need as
    // a fallback when proxy headers are absent.
    axum::serve(listener, app.router.clone().into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    let timeout = std::time::Duration::from_secs(app.cfg.shutdown_timeout);
    if tokio::time::timeout(timeout, app.shutdown()).await.is_err() {
        tracing::warn!("graceful shutdown timed out after {}s, forcing exit", timeout.as_secs());
        crate::pkg::logs::flush();
    }
    Ok(())
}

// ── Shutdown signal handler ───────────────────────────────────────────────────

/// Waits for SIGINT (Ctrl-C), SIGTERM (systemd / k8s), or SIGHUP.
/// Mirrors Go's signal channel in `gracefulShutdown`.
async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c().await.expect("failed to install Ctrl-C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(unix)]
    let hangup = async {
        signal::unix::signal(signal::unix::SignalKind::hangup())
            .expect("failed to install SIGHUP handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    #[cfg(not(unix))]
    let hangup = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c    => info!("received SIGINT"),
        () = terminate => info!("received SIGTERM"),
        () = hangup    => info!("received SIGHUP"),
    }

    info!("shutdown signal received, draining in-flight requests...");
}
