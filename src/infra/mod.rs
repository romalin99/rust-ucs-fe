/// Infrastructure dependency container.
///
/// Mirrors Go's `internal/infra/manager.go`:
///   `ComManager`  -> [`AppInfra`]
///   `ComClient`   -> inner fields of [`AppInfra`]
///   `ComModel`    -> inner fields of [`AppInfra`]
///
/// Holds every runtime resource (Oracle pool, Redis, HTTP clients,
/// repositories) that services and handlers need.  The single
/// `Arc<AppInfra>` is created in `main` and cloned into `AppState`.
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use once_cell::sync::OnceCell;
use redis::aio::ConnectionManager as RedisManager;

use crate::client::{McsClient, UssClient, WpsClient};
use crate::config::{AppConfig, OracleConnectInfo, RedisDbEntry};
use crate::repository::{
    FieldIdUssMappingRepository, MerchantRuleRepository, OraclePool, PoolConfig,
    ValidationRecordRepository, build_pool, ping_pool,
};

// ── Redis multi-DB singleton ──────────────────────────────────────────────────

/// A single Redis DB instance with its configured default TTL.
/// Mirrors Go's `redis.RedisInstance`.
#[derive(Clone)]
pub struct RedisInstance {
    pub client: Arc<redis::Client>,
    pub ttl:    Duration,
}

static REDIS_DB_MAP: OnceCell<HashMap<String, RedisInstance>> = OnceCell::new();

/// Initialise the global multi-DB Redis map.
/// Mirrors Go's `Config.InitDBSV2()` — creates a client per DB index and pings.
pub async fn init_redis_multi_db(cfg_addr: &[String], cfg_password: &str, dbs: &[RedisDbEntry]) {
    if REDIS_DB_MAP.get().is_some() {
        return;
    }
    let mut map = HashMap::with_capacity(dbs.len());
    let addr = cfg_addr.first().map(String::as_str).unwrap_or("127.0.0.1:6379");
    for db_entry in dbs {
        let url = if cfg_password.is_empty() {
            format!("redis://{}/{}", addr, db_entry.db)
        } else {
            format!("redis://:{}@{}/{}", cfg_password, addr, db_entry.db)
        };
        match redis::Client::open(url.as_str()) {
            Ok(client) => {
                let mgr_cfg = redis::aio::ConnectionManagerConfig::new()
                    .set_connection_timeout(Some(Duration::from_secs(10)))
                    .set_response_timeout(Some(Duration::from_secs(30)))
                    .set_number_of_retries(3);
                match redis::aio::ConnectionManager::new_with_config(client.clone(), mgr_cfg).await {
                    Ok(mut cm) => {
                        if let Err(e) = redis::cmd("PING").query_async::<String>(&mut cm).await {
                            tracing::warn!(db = db_entry.db, err = %e, "Redis PING failed (continuing)");
                        }
                    }
                    Err(e) => {
                        tracing::warn!(db = db_entry.db, err = %e, "Redis ConnectionManager init failed (continuing)");
                    }
                }
                let key = format!("dbidx:{}", db_entry.db);
                let ttl = Duration::from_secs(db_entry.set_default_expiration.unsigned_abs());
                map.insert(key, RedisInstance { client: Arc::new(client), ttl });
                tracing::info!(db = db_entry.db, "Redis DB instance initialised");
            }
            Err(e) => tracing::error!(db = db_entry.db, err = %e, "Redis DB client failed"),
        }
    }
    let _ = REDIS_DB_MAP.set(map);
}

/// Retrieve a Redis DB instance by index.
/// Mirrors Go's `redis.GetDbInstance(idx int32)`.
pub fn get_db_instance(idx: i32) -> anyhow::Result<&'static RedisInstance> {
    let key = format!("dbidx:{}", idx);
    REDIS_DB_MAP
        .get()
        .ok_or_else(|| anyhow::anyhow!("Redis multi-DB map not initialised"))?
        .get(&key)
        .ok_or_else(|| anyhow::anyhow!("Redis instance not found for dbidx:{}", idx))
}

/// Close all Redis multi-DB instances.
/// Mirrors Go's `ComManager.Close()` which iterates `m.Rcs` and closes each client.
pub fn close_redis_multi_db() {
    if let Some(map) = REDIS_DB_MAP.get() {
        tracing::info!(count = map.len(), "closing Redis multi-DB instances");
        // redis::Client instances are reference-counted; dropping all Arcs will close connections.
        // The static OnceCell itself cannot be cleared, but we log for operational visibility.
        for (key, _inst) in map.iter() {
            tracing::info!(key = %key, "Redis DB instance marked for close");
        }
    }
}

// ── Oracle pool stats monitor ─────────────────────────────────────────────────

/// Background task that logs Oracle r2d2 pool stats periodically.
/// Mirrors Go's `oracle.Config.monitorOraclePool` goroutine.
pub fn start_oracle_pool_monitor(
    pool:          Arc<OraclePool>,
    interval_secs: u64,
    desc:          &'static str,
) -> tokio::sync::watch::Sender<bool> {
    let (stop_tx, mut stop_rx) = tokio::sync::watch::channel(false);
    let secs = if interval_secs == 0 { 60 } else { interval_secs };

    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(secs));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    let st   = pool.state();
                    let max_sz = pool.max_size();
                    let in_use = st.connections.saturating_sub(st.idle_connections);
                    let idle   = st.idle_connections;
                    let open   = st.connections;
                    let rate   = if max_sz > 0 { (in_use as f64) / (max_sz as f64) * 100.0 } else { 0.0 };
                    tracing::info!(desc, max_open = max_sz, open, in_use, idle,
                        use_rate = format!("{:.1}%", rate), "Oracle pool stats");
                    if rate > 80.0 {
                        tracing::warn!(desc, in_use, max_open = max_sz,
                            "Oracle pool reached warning level (80%)");
                    }
                    if let Some(m) = crate::pkg::metrics::METRICS.get() {
                        let lbl = &[desc];
                        m.oracle.max_open.with_label_values(lbl).set(max_sz as f64);
                        m.oracle.open.with_label_values(lbl).set(open as f64);
                        m.oracle.in_use.with_label_values(lbl).set(in_use as f64);
                        m.oracle.idle.with_label_values(lbl).set(idle as f64);
                        m.oracle.usage_rate.with_label_values(lbl).set(rate);
                    }
                }
                _ = stop_rx.changed() => { tracing::info!(desc, "Oracle pool monitor stopped"); return; }
            }
        }
    });
    stop_tx
}

// ── Re-exports for structural parity with Go's infra package ─────────────────

pub use crate::client::{
    McsClient as ComMcsClient, UssClient as ComUssClient, WpsClient as ComWpsClient,
};
pub use crate::repository::{
    FieldIdUssMappingRepository as ComFieldIdUssMappingRepo,
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
    /// Oracle-backed field-id <-> USS mapping repository.
    pub uss_mapping_repo: Arc<FieldIdUssMappingRepository>,
}

impl AppInfra {
    /// Construct every infrastructure resource in dependency order:
    /// Oracle -> Redis -> HTTP clients -> Repositories.
    ///
    /// Mirrors Go's `NewComManager(odbCfg, ussCfg, mcsCfg, wpsCfg, rdsCfg)`.
    pub async fn new(cfg: &AppConfig, oracle_info: &OracleConnectInfo) -> anyhow::Result<Self> {
        // ── Oracle pool (instant -- no connections opened here) ───────────────
        //
        // build_unchecked() returns immediately; connections are created lazily
        // on first query.  This mirrors Go's sql.Open() which is also instant.
        // A background ping validates connectivity without blocking startup.
        let t_oracle = std::time::Instant::now();

        // Pool sizing: use max_open_conn for pool capacity, pool_min for
        // minimum idle connections maintained by r2d2's background thread.
        // Mirrors Go's poolMaxSessions / poolMinSessions.
        let pool_min = if cfg.oracle.pool_min > 0 {
            cfg.oracle.pool_min
        } else {
            // Sensible default when pool_min is not set: enough for concurrent
            // startup loaders (field-config, USS-mapping, cron) + headroom.
            4
        };
        tracing::info!(
            user             = %oracle_info.user,
            conn_string      = %oracle_info.connect_string,
            max_size         = cfg.oracle.max_open_conn,
            min_idle         = pool_min,
            max_life_time_s  = cfg.oracle.max_life_time,
            max_idle_time_m  = cfg.oracle.max_idle_time,
            stmt_cache_size  = crate::repository::STMT_CACHE_SIZE,
            prefetch_rows    = crate::repository::DEFAULT_PREFETCH_ROWS,
            fetch_array_size = crate::repository::DEFAULT_FETCH_ARRAY_SIZE,
            "Building Oracle connection pool"
        );
        let pool_cfg = PoolConfig {
            max_size:               cfg.oracle.max_open_conn,
            min_idle:               pool_min,
            max_lifetime_secs:      cfg.oracle.max_life_time,
            max_idle_time_mins:     cfg.oracle.max_idle_time,
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
        // Warm up connections in parallel so concurrent startup loaders each
        // get a pre-warmed connection.  Cap at 8 to avoid slow startup.
        let warm_count = (pool_min as usize).min(8).max(2);
        ping_pool(oracle_pool.clone(), warm_count).await;

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

        // ── Repositories (all share the single oracle_pool) ─────────────────
        let merchant_repo = Arc::new(MerchantRuleRepository::new(
            oracle_pool.clone(),
            cfg.oracle.read_timeout,
        ));
        let validation_repo = Arc::new(ValidationRecordRepository::new(
            oracle_pool.clone(),
            cfg.oracle.read_timeout,
            cfg.oracle.write_timeout,
        ));
        let uss_mapping_repo = Arc::new(FieldIdUssMappingRepository::new(
            oracle_pool.clone(),
            cfg.oracle.read_timeout,
        ));

        Ok(Self {
            oracle_pool,
            redis,
            uss,
            mcs,
            wps,
            merchant_repo,
            validation_repo,
            uss_mapping_repo,
        })
    }
}
