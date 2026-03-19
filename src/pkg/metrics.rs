/// Prometheus metrics for Oracle/Redis pool monitoring.
///
/// Mirrors Go's `pkg/metrics/` package:
///   - `oracle_metrics.go` → [`OraclePoolMetrics`]
///   - `redis_metrics.go`  → [`RedisPoolMetrics`]
///   - `metrics.go`        → [`init`]
///
/// All gauge vectors are registered once via `OnceLock`.
use std::sync::OnceLock;

use prometheus::{GaugeVec, Opts, Registry, register_gauge_vec_with_registry};

// ── Oracle pool metrics ───────────────────────────────────────────────────────

/// Prometheus gauges for Oracle connection pool statistics.
///
/// Mirrors Go's `pkg/metrics/oracle_metrics.go`.
pub struct OraclePoolMetrics {
    pub max_open:          GaugeVec,
    pub open:              GaugeVec,
    pub in_use:            GaugeVec,
    pub idle:              GaugeVec,
    pub wait_count:        GaugeVec,
    pub wait_duration:     GaugeVec,
    pub max_idle_closed:   GaugeVec,
    pub max_life_closed:   GaugeVec,
    pub usage_rate:        GaugeVec,
}

impl OraclePoolMetrics {
    fn new(registry: &Registry) -> Self {
        let g = |name: &str, help: &str| {
            register_gauge_vec_with_registry!(
                Opts::new(name, help),
                &["instance"],
                registry
            ).unwrap_or_else(|_| GaugeVec::new(Opts::new(name, help), &["instance"]).unwrap())
        };

        Self {
            max_open:        g("db_pool_max_open_connections",        "Max open connections in DB pool"),
            open:            g("db_pool_open_connections",            "Current open connections in DB pool"),
            in_use:          g("db_pool_in_use",                      "Connections currently in use"),
            idle:            g("db_pool_idle",                        "Idle connections in DB pool"),
            wait_count:      g("db_pool_wait_count_total",            "Cumulative waits for a connection"),
            wait_duration:   g("db_pool_wait_duration_seconds_total", "Cumulative wait duration (s)"),
            max_idle_closed: g("db_pool_max_idle_closed_total",       "Connections closed due to max-idle"),
            max_life_closed: g("db_pool_max_lifetime_closed_total",   "Connections closed due to max-lifetime"),
            usage_rate:      g("db_pool_usage_rate_percent",          "DB pool usage percentage"),
        }
    }
}

// ── Redis pool metrics ────────────────────────────────────────────────────────

/// Prometheus gauges for Redis connection pool statistics.
///
/// Mirrors Go's `pkg/metrics/redis_metrics.go`.
pub struct RedisPoolMetrics {
    pub max_pool_size:    GaugeVec,
    pub hits:             GaugeVec,
    pub misses:           GaugeVec,
    pub timeouts:         GaugeVec,
    pub total_conns:      GaugeVec,
    pub idle_conns:       GaugeVec,
    pub stale_conns:      GaugeVec,
    pub wait_count:       GaugeVec,
    pub wait_duration_ns: GaugeVec,
}

impl RedisPoolMetrics {
    fn new(registry: &Registry) -> Self {
        let g = |name: &str, help: &str| {
            register_gauge_vec_with_registry!(
                Opts::new(name, help),
                &["instance"],
                registry
            ).unwrap_or_else(|_| GaugeVec::new(Opts::new(name, help), &["instance"]).unwrap())
        };

        Self {
            max_pool_size:    g("redis_max_pool_size",          "Configured Redis pool size"),
            hits:             g("redis_hits_total",             "Cumulative cache hits"),
            misses:           g("redis_misses_total",           "Cumulative cache misses"),
            timeouts:         g("redis_timeouts_total",         "Cumulative pool timeouts"),
            total_conns:      g("redis_total_connections",      "Current total connections"),
            idle_conns:       g("redis_idle_connections",       "Current idle connections"),
            stale_conns:      g("redis_stale_connections_total","Cumulative stale connections closed"),
            wait_count:       g("redis_wait_count_total",       "Cumulative pool wait count"),
            wait_duration_ns: g("redis_wait_duration_ns_total", "Cumulative pool wait duration (ns)"),
        }
    }
}

// ── Global singleton ─────────────────────────────────────────────────────────

pub struct AppMetrics {
    pub oracle: OraclePoolMetrics,
    pub redis:  RedisPoolMetrics,
}

static METRICS: OnceLock<AppMetrics> = OnceLock::new();

/// Initialise all metric families once, scoped to `service_name`.
///
/// Mirrors Go's `metrics.Init(serviceName)` which calls
/// `InitDBMetrics` + `InitRedisMetrics` + `InitKafkaMetrics`.
pub fn init(_service_name: &str) {
    METRICS.get_or_init(|| {
        let registry = prometheus::default_registry();
        AppMetrics {
            oracle: OraclePoolMetrics::new(registry),
            redis:  RedisPoolMetrics::new(registry),
        }
    });
}

/// Get a reference to the global metrics (panics if `init` not called first).
pub fn get() -> &'static AppMetrics {
    METRICS.get().expect("metrics not initialised — call pkg::metrics::init() first")
}
