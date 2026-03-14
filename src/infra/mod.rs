/// Infrastructure dependency container.
///
/// Mirrors Go's `internal/infra/manager.go`:
///   `ComManager`  → [`AppInfra`]
///   `ComClient`   → inner fields of [`AppInfra`]
///   `ComModel`    → inner fields of [`AppInfra`]
///
/// Holds every runtime resource (Oracle pool, Redis, HTTP clients,
/// repositories) that services and handlers need.  The single
/// `Arc<AppInfra>` is created in `main` and cloned into `AppState`.
use std::sync::Arc;

use redis::aio::ConnectionManager as RedisManager;

use crate::client::{McsClient, UssClient, WpsClient};
use crate::config::{AppConfig, OracleConnectInfo};
use crate::repository::{
    MerchantRuleRepository, OraclePool, PoolConfig, ValidationRecordRepository, build_pool,
    ping_pool,
};

// ── Re-exports for structural parity with Go's infra package ─────────────────

pub use crate::client::{
    McsClient as ComMcsClient, UssClient as ComUssClient, WpsClient as ComWpsClient,
};
pub use crate::repository::{
    MerchantRuleRepository as ComMerchantRepo, OraclePool as ComOraclePool,
    ValidationRecordRepository as ComValidationRepo,
};

// ═══════════════════════════════════════════════════════════════════════════════
// AppInfra  (mirrors Go's ComManager + ComClient + ComModel)
// ═══════════════════════════════════════════════════════════════════════════════

/// Central infrastructure container.
///
/// Mirrors Go's `ComManager` (which embeds `ComClient` and `ComModel`).
///
/// # Construction
/// ```ignore
/// let infra = Arc::new(
///     AppInfra::new(&cfg, &oracle_info).await.expect("infra init failed")
/// );
/// ```
pub struct AppInfra {
    // ── Database ──────────────────────────────────────────────────────────────
    /// Oracle connection pool (r2d2-managed synchronous pool).
    pub oracle_pool: Arc<OraclePool>,

    // ── Cache ─────────────────────────────────────────────────────────────────
    /// Redis async connection manager (auto-reconnect).
    pub redis: RedisManager,

    // ── HTTP clients ──────────────────────────────────────────────────────────
    /// USS (User Service System) HTTP client.
    pub uss: Arc<UssClient>,
    /// MCS (Merchant Credit Service) HTTP client.
    pub mcs: Arc<McsClient>,
    /// WPS (Wallet/Payment Service) HTTP client.
    pub wps: Arc<WpsClient>,

    // ── Repositories ──────────────────────────────────────────────────────────
    /// Oracle-backed merchant rule repository.
    pub merchant_repo: Arc<MerchantRuleRepository>,
    /// Oracle-backed validation record repository.
    pub validation_repo: Arc<ValidationRecordRepository>,
}

impl AppInfra {
    /// Construct every infrastructure resource in dependency order:
    /// Oracle → Redis → HTTP clients → Repositories.
    ///
    /// Mirrors Go's `NewComManager(odbCfg, ussCfg, mcsCfg, wpsCfg, rdsCfg)`.
    pub async fn new(cfg: &AppConfig, oracle_info: &OracleConnectInfo) -> anyhow::Result<Self> {
        // ── Oracle ────────────────────────────────────────────────────────────
        // ── Oracle pool (instant — no connections opened here) ────────────────
        //
        // build_unchecked() returns immediately; connections are created lazily
        // on first query.  This mirrors Go's sql.Open() which is also instant.
        // A background ping validates connectivity without blocking startup.
        let t_oracle = std::time::Instant::now();
        tracing::info!(
            user             = %oracle_info.user,
            conn_string      = %oracle_info.connect_string,
            max_open_conn    = cfg.oracle.max_open_conn,
            max_idle_conn    = cfg.oracle.max_idle_conn,
            max_life_time_s  = cfg.oracle.max_life_time,
            max_idle_time_m  = cfg.oracle.max_idle_time,
            "Building Oracle connection pool (lazy/unchecked)"
        );
        let pool_cfg = PoolConfig {
            max_size: cfg.oracle.max_open_conn,
            min_idle: 0, // lazy — matches Go sql.Open
            max_lifetime_secs: cfg.oracle.max_life_time,
            max_idle_time_mins: cfg.oracle.max_idle_time,
            connection_timeout_secs: 30,
        };
        let oracle_pool = Arc::new(build_pool(
            &oracle_info.user,
            &oracle_info.password,
            &oracle_info.connect_string,
            pool_cfg,
        ));
        tracing::info!(
            elapsed_us = t_oracle.elapsed().as_micros(),
            "Oracle pool struct created (no TCP yet)"
        );
        // Background ping — mirrors Go's db.Ping() after sql.Open().
        // Does NOT block startup; logs result when the first TCP connection
        // to Oracle completes (or fails).
        ping_pool(oracle_pool.clone());

        // ── Redis ─────────────────────────────────────────────────────────────
        tracing::info!("Connecting to Redis");
        let redis_addr = cfg
            .redis
            .addr
            .first()
            .map(|s| s.as_str())
            .unwrap_or("127.0.0.1:6379");

        let redis_url = if cfg.redis.password.is_empty() {
            format!("redis://{}/{}", redis_addr, cfg.redis.db)
        } else {
            format!(
                "redis://:{}@{}/{}",
                cfg.redis.password, redis_addr, cfg.redis.db
            )
        };

        let redis_client = redis::Client::open(redis_url)
            .map_err(|e| anyhow::anyhow!("Invalid Redis URL: {e}"))?;
        let redis = redis::aio::ConnectionManager::new(redis_client)
            .await
            .map_err(|e| anyhow::anyhow!("Redis connection failed: {e}"))?;
        tracing::info!("Redis connected");

        // ── HTTP clients ──────────────────────────────────────────────────────
        let uss = Arc::new(UssClient::new(&cfg.uss_service));
        let mcs = Arc::new(McsClient::new(&cfg.mcs_service));
        let wps = Arc::new(WpsClient::new(&cfg.wps_service));

        // ── Repositories ──────────────────────────────────────────────────────
        let merchant_repo = Arc::new(MerchantRuleRepository::new(oracle_pool.clone()));
        let validation_repo = Arc::new(ValidationRecordRepository::new(oracle_pool.clone()));

        Ok(Self {
            oracle_pool,
            redis,
            uss,
            mcs,
            wps,
            merchant_repo,
            validation_repo,
        })
    }
}
