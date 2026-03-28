/// Prometheus metrics for Oracle/Redis/Kafka pool monitoring + standalone metrics server.
///
/// Mirrors Go's `pkg/metrics/` package:
///   - `oracle_metrics.go` → [`OraclePoolMetrics`]
///   - `redis_metrics.go`  → [`RedisPoolMetrics`]
///   - `kafka_metrics.go`  → [`KafkaMetrics`]
///   - `metrics.go`        → [`init`]
///   - `server.go`         → [`MetricsServer`]
///
/// All gauge/counter vectors are registered once via `OnceLock`.
use std::net::SocketAddr;
use std::sync::OnceLock;

use prometheus::{
    CounterVec, GaugeVec, Opts, Registry,
    register_counter_vec_with_registry, register_gauge_vec_with_registry,
};
use tracing::{error, info};

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

/// Prometheus gauges/counters for Redis connection pool statistics.
///
/// Mirrors Go's `pkg/metrics/redis_metrics.go`.
/// Gauge for current values (pool_size, total_conns, idle_conns, wait_duration_ns).
/// Counter for cumulative monotonic values (hits, misses, timeouts, wait_count, stale_conns).
pub struct RedisPoolMetrics {
    pub max_pool_size:    GaugeVec,
    pub hits:             CounterVec,
    pub misses:           CounterVec,
    pub timeouts:         CounterVec,
    pub total_conns:      GaugeVec,
    pub idle_conns:       GaugeVec,
    pub stale_conns:      CounterVec,
    pub wait_count:       CounterVec,
    pub wait_duration_ns: GaugeVec,
}

impl RedisPoolMetrics {
    fn new(registry: &Registry) -> Self {
        let g = |name: &str, help: &str| {
            register_gauge_vec_with_registry!(
                Opts::new(name, help),
                &["pool"],
                registry
            ).unwrap_or_else(|_| GaugeVec::new(Opts::new(name, help), &["pool"]).unwrap())
        };
        let c = |name: &str, help: &str| {
            register_counter_vec_with_registry!(
                Opts::new(name, help),
                &["pool"],
                registry
            ).unwrap_or_else(|_| CounterVec::new(Opts::new(name, help), &["pool"]).unwrap())
        };

        Self {
            max_pool_size:    g("redis_max_pool_size",          "Configured Redis pool size"),
            hits:             c("redis_hits_total",             "Total number of Redis connection pool hits"),
            misses:           c("redis_misses_total",           "Total number of Redis connection pool misses"),
            timeouts:         c("redis_timeouts_total",         "Total number of Redis connection pool timeouts"),
            total_conns:      g("redis_total_connections",      "Current total connections"),
            idle_conns:       g("redis_idle_connections",       "Current idle connections"),
            stale_conns:      c("redis_stale_conns_total",      "Total number of stale Redis connections"),
            wait_count:       c("redis_wait_count_total",       "Total number of times waiting for a Redis connection"),
            wait_duration_ns: g("redis_wait_duration_ns_total", "Total wait duration for Redis connections (ns)"),
        }
    }
}

// ── Kafka commit metrics ──────────────────────────────────────────────────────

/// Prometheus counters/gauges for Kafka offset-commit monitoring.
///
/// Mirrors Go's `pkg/metrics/kafka_metrics.go`.
pub struct KafkaMetrics {
    /// Total successful Kafka offset commits.
    pub commit_success_total:       CounterVec,
    /// Total Kafka offset commits that failed after retries.
    pub commit_failures_total:      CounterVec,
    /// Total Kafka offset commit retry attempts.
    pub commit_retries_total:       CounterVec,
    /// Current consecutive Kafka offset commit failures.
    pub commit_consecutive_failures: GaugeVec,
}

impl KafkaMetrics {
    fn new(registry: &Registry) -> Self {
        let c = |name: &str, help: &str| {
            register_counter_vec_with_registry!(
                Opts::new(name, help),
                &["group", "topic"],
                registry
            ).unwrap_or_else(|_| CounterVec::new(Opts::new(name, help), &["group", "topic"]).unwrap())
        };
        let g = |name: &str, help: &str| {
            register_gauge_vec_with_registry!(
                Opts::new(name, help),
                &["group", "topic"],
                registry
            ).unwrap_or_else(|_| GaugeVec::new(Opts::new(name, help), &["group", "topic"]).unwrap())
        };

        Self {
            commit_success_total:        c("kafka_commit_success_total",
                                           "Total number of successful Kafka offset commits"),
            commit_failures_total:       c("kafka_commit_failures_total",
                                           "Total number of Kafka offset commits that failed after retries"),
            commit_retries_total:        c("kafka_commit_retries_total",
                                           "Total number of Kafka offset commit retry attempts"),
            commit_consecutive_failures: g("kafka_commit_consecutive_failures",
                                           "Current number of consecutive Kafka offset commit failures"),
        }
    }

    /// Report whether Kafka metrics have been initialised.
    ///
    /// Mirrors Go's `KafkaMetricsEnabled()`.
    pub fn enabled() -> bool {
        METRICS.get().is_some()
    }
}

// ── Global singleton ─────────────────────────────────────────────────────────

pub struct AppMetrics {
    pub service_name: String,
    pub oracle: OraclePoolMetrics,
    pub redis:  RedisPoolMetrics,
    pub kafka:  KafkaMetrics,
}

pub static METRICS: OnceLock<AppMetrics> = OnceLock::new();

/// Initialise all metric families once, scoped to `service_name`.
///
/// Mirrors Go's `metrics.Init(serviceName)` which calls
/// `InitDBMetrics` + `InitRedisMetrics` + `InitKafkaMetrics`.
/// The `service_name` is stored for const-label injection in custom dashboards.
pub fn init(service_name: &str) {
    let svc = service_name.to_string();
    METRICS.get_or_init(|| {
        let registry = prometheus::default_registry();
        tracing::info!(service = %svc, "initialising Prometheus metrics");
        AppMetrics {
            service_name: svc,
            oracle: OraclePoolMetrics::new(registry),
            redis:  RedisPoolMetrics::new(registry),
            kafka:  KafkaMetrics::new(registry),
        }
    });
}

/// Get a reference to the global metrics (panics if `init` not called first).
pub fn get() -> &'static AppMetrics {
    METRICS.get().expect("metrics not initialised — call pkg::metrics::init() first")
}

// ── Standalone Prometheus HTTP server ────────────────────────────────────────

/// Standalone HTTP server that exposes `/metrics` via the default Prometheus registry.
///
/// Mirrors Go's `pkg/metrics/server.go` `Server` struct.
///
/// In the Axum-based application the `/metrics` route is already registered
/// in the main router via `axum-prometheus`.  This struct is provided for
/// environments that need a *separate* metrics endpoint on a different port
/// (e.g. side-car scraping configurations).
pub struct MetricsServer {
    addr: SocketAddr,
}

impl MetricsServer {
    /// Create a metrics server bound to the given address.
    ///
    /// Mirrors Go's `NewMetricsServer(addr string, logger *zap.Logger)`.
    pub fn new(addr: SocketAddr) -> Self {
        Self { addr }
    }

    /// Launch the metrics server in a background `tokio::spawn` task.
    ///
    /// Mirrors Go's `Server.Start()`.
    pub fn start(self) {
        let addr = self.addr;
        tokio::spawn(async move {
            use axum::{Router, routing::get};

            let app = Router::new().route("/metrics", get(|| async {
                use prometheus::{Encoder, TextEncoder};
                let encoder = TextEncoder::new();
                let metric_families = prometheus::gather();
                let mut buf = Vec::new();
                if let Err(e) = encoder.encode(&metric_families, &mut buf) {
                    error!("metrics encode failed: {e}");
                    return (
                        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                        "encode error".to_string(),
                    );
                }
                (
                    axum::http::StatusCode::OK,
                    String::from_utf8_lossy(&buf).into_owned(),
                )
            }));

            let listener = match tokio::net::TcpListener::bind(addr).await {
                Ok(l)  => l,
                Err(e) => {
                    error!("metrics server bind failed on {addr}: {e}");
                    return;
                }
            };

            info!(%addr, "metrics server started");
            if let Err(e) = axum::serve(listener, app).await {
                error!("metrics server error: {e}");
            }
        });
    }
}
