use std::time::Duration;

use aws_sdk_secretsmanager::Client as SecretsClient;
use serde::Deserialize;

// ── Oracle credentials loaded from AWS Secrets Manager ────────────────────────

#[derive(Debug, Deserialize)]
pub struct OracleConnectInfo {
    #[serde(rename = "oracledb.user")]
    pub user: String,
    #[serde(rename = "oracledb.password")]
    pub password: String,
    #[serde(rename = "oracledb.uconnectStringer")]
    pub connect_string: String,
}

// ── Per-request timeout categories (seconds) ─────────────────────────────────

#[derive(Debug, Deserialize, Clone)]
pub struct AppTimeouts {
    pub quick:  u64,
    pub normal: u64,
    pub long:   u64,
    pub upload: u64,
}

impl Default for AppTimeouts {
    fn default() -> Self {
        Self { quick: 5, normal: 15, long: 60, upload: 120 }
    }
}

impl AppTimeouts {
    pub fn quick_duration(&self)  -> Duration { Duration::from_secs(self.quick)  }
    pub fn normal_duration(&self) -> Duration { Duration::from_secs(self.normal) }
    pub fn long_duration(&self)   -> Duration { Duration::from_secs(self.long)   }
    pub fn upload_duration(&self) -> Duration { Duration::from_secs(self.upload) }
}

// ── Cron job configuration ────────────────────────────────────────────────────

/// Per-job scheduler configuration.
///
/// Uses a custom `Deserialize` impl so that `toml`/`config` deserializers honour
/// missing optional fields (both crates have quirks with `#[serde(default)]` inside
/// `HashMap` values).
#[derive(Debug, Clone)]
pub struct JobConfig {
    pub cron:        String,
    pub interval:    u64,
    pub timeout:     u64,
    pub concurrency: u32,
    pub enabled:     bool,
}

impl Default for JobConfig {
    fn default() -> Self {
        Self {
            cron:        String::new(),
            interval:    0,
            timeout:     60,
            concurrency: 0,
            enabled:     false,
        }
    }
}

impl<'de> serde::Deserialize<'de> for JobConfig {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        struct Raw {
            cron:        Option<String>,
            interval:    Option<u64>,
            timeout:     Option<u64>,
            concurrency: Option<u32>,
            enabled:     Option<bool>,
        }
        let raw = Raw::deserialize(deserializer)?;
        Ok(JobConfig {
            cron:        raw.cron.unwrap_or_default(),
            interval:    raw.interval.unwrap_or(0),
            timeout:     raw.timeout.unwrap_or(60),
            concurrency: raw.concurrency.unwrap_or(0),
            enabled:     raw.enabled.unwrap_or(false),
        })
    }
}

// ── Downstream HTTP service addresses ────────────────────────────────────────

#[derive(Debug, Deserialize, Clone, Default)]
pub struct ServiceConfig {
    #[serde(default)]
    pub host:      String,
    #[serde(default)]
    pub base_path: String,
}

// ── Oracle DB configuration ───────────────────────────────────────────────────

/// Oracle connection pool configuration.
///
/// Mirrors Go's `pkg/oracle.Config` (TOML key `[oracle]`).
/// Field names match Go's `mapstructure` keys so the same TOML file drives both services.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct OracleConfig {
    #[serde(default)]
    pub user:           String,
    /// TOML key: `passwd`  (mirrors Go's `OraclePWD string \`mapstructure:"passwd"\``).
    #[serde(default, alias = "passwd")]
    pub password:       String,
    /// TOML key: `addr_connect_stringer`.
    #[serde(default, alias = "addr_connect_stringer")]
    pub connect_string: String,
    /// Minimum idle connections in the pool.
    #[serde(default)]
    pub pool_min:       u32,
    /// Maximum open connections (TOML key: `max_open_conn`).
    #[serde(default = "default_100", alias = "maxOpenConn")]
    pub max_open_conn:  u32,
    /// Maximum idle connections (TOML key: `max_idle_conn`).
    #[serde(default = "default_100", alias = "maxIdleConn")]
    pub max_idle_conn:  u32,
    /// Connection max lifetime in seconds (TOML key: `max_life_time`).
    #[serde(default = "default_30_u64", alias = "maxLifeTime")]
    pub max_life_time:  u64,
    /// Connection max idle time in minutes (TOML key: `max_idle_time`).
    #[serde(default = "default_30_u64", alias = "maxIdleTime")]
    pub max_idle_time:  u64,
    /// Enable pool-stats monitoring (TOML key: `enable_stats_monitor`).
    #[serde(default, alias = "enableStatsMonitor")]
    pub enable_stats_monitor: bool,
    /// Pool-stats sampling interval in seconds (TOML key: `stats_interval`).
    #[serde(default = "default_60_u64", alias = "statsInterval")]
    pub stats_interval: u64,
    /// Per-query read timeout in seconds (TOML key: `read_timeout`).
    #[serde(default = "default_15_u64", alias = "readTimeout")]
    pub read_timeout:   u64,
    /// Per-query write timeout in seconds.
    /// Per-query write timeout in seconds (TOML key: `write_timeout`).
    #[serde(default = "default_15_u64", alias = "writeTimeout")]
    pub write_timeout:  u64,
}

// ── Redis configuration ───────────────────────────────────────────────────────

/// Redis connection configuration.
///
/// Mirrors Go's `pkg/redis.Config` (TOML key `[redis]`).
#[derive(Debug, Deserialize, Clone, Default)]
pub struct RedisConfig {
    pub addr:        Vec<String>,
    #[serde(default)]
    pub password:    String,
    #[serde(default)]
    pub master_name: String,
    /// Default database index (single-DB mode).
    #[serde(default)]
    pub db:  i64,
    /// Multi-DB configuration (one entry per DB index).
    #[serde(default)]
    pub dbs: Vec<RedisDbEntry>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct RedisDbEntry {
    pub db:                     i64,
    pub pool_size:              u32,
    pub set_default_expiration: i64,
}

// ── Telemetry / OpenTelemetry ─────────────────────────────────────────────────

/// OpenTelemetry tracing configuration.
///
/// Mirrors Go's `pkg/telemetry.Telemetry` struct (TOML key `[telemetry]`).
#[derive(Debug, Deserialize, Clone, Default)]
pub struct TelemetryConfig {
    #[serde(default)]
    pub enabled:     bool,
    /// Collector endpoint, e.g. `"localhost:4318"` (OTLP HTTP).
    #[serde(default)]
    pub endpoint:    String,
    /// Service name reported to the trace backend (TOML key: `server_name`).
    #[serde(default)]
    pub server_name: String,
    /// Sampling ratio 0.0–1.0 (default 1.0 = always sample).
    #[serde(default = "default_sampler")]
    pub sampler:     f64,
    /// Batcher type: `"otlp"` | `"none"` (default `"none"`).
    #[serde(default = "default_batcher")]
    pub batcher:     String,
    /// Paths the OTel middleware should skip (no spans created).
    #[serde(default)]
    pub skip_paths:  Vec<String>,
}

// ── Logging ───────────────────────────────────────────────────────────────────

/// Logger configuration.
///
/// Mirrors Go's `pkg/logs.Config` (TOML key `[log]`) with all defaults
/// from `internal/config/init.go`.
#[derive(Debug, Deserialize, Clone)]
pub struct LogConfig {
    /// Logger name (used as log file base name, e.g. `"tcg-ucs-fe"`).
    #[serde(default = "default_log_name")]
    pub name:         String,

    /// Service name tag added to every log line (e.g. `"TCG-UCS-FE"`).
    #[serde(default = "default_service_name", alias = "serviceName")]
    pub service_name: String,

    /// Minimum log level: `"debug"` | `"info"` | `"warn"` | `"error"`.
    #[serde(default = "default_info_str")]
    pub level:        String,

    /// Log output format: `"json"` (default, structured) or `"text"` (human-readable).
    #[serde(default = "default_json_str")]
    pub encoding:     String,

    /// Output mode: `"console"` | `"file"` | `"kafka"`.
    /// Mirrors Go's `log.mode` default `"file"`.
    #[serde(default = "default_log_mode")]
    pub mode:         String,

    /// Timestamp format string (Go `time.Format` layout).
    /// Default: `"2006-01-02 15:04:05.000"`.
    #[serde(default = "default_time_format", rename = "timeFormat")]
    pub time_format:  String,

    /// Log file directory (mirrors Go's `FileInfo.Path` + `log.path`).
    #[serde(default)]
    pub path:         String,

    /// Log rotation strategy: `"size"` (default) or `"time"`.
    #[serde(default = "default_rotation")]
    pub rotation:     String,

    /// Max single log file size in MB (default 500).
    #[serde(default = "default_500", rename = "maxSize")]
    pub max_size:     u32,

    /// Max number of old log files to keep (default 10).
    #[serde(default = "default_10", rename = "maxBackups")]
    pub max_backups:  u32,

    /// Max number of days to retain old log files (default 5).
    #[serde(default = "default_5", rename = "keepDays")]
    pub keep_days:    u32,

    /// Whether to gzip old log files (default true).
    #[serde(default = "default_true")]
    pub compress:     bool,

    /// Whether to emit pool-stat log lines (mirrors Go's `log.stat`).
    #[serde(default = "default_true")]
    pub stat:         bool,

    /// Write buffer size in MB (default 30).
    #[serde(default = "default_30_u32", rename = "bufferSize")]
    pub buffer_size:  u32,

    /// Buffer flush interval in ms (default 50).
    #[serde(default = "default_50_u32", rename = "bufferFlushInterval")]
    pub buffer_flush_interval: u32,

    /// Runtime environment tag (e.g. `"pro"` | `"dev"`).
    #[serde(default)]
    pub env:          String,

    /// Deprecated alias for `path`.
    #[serde(default)]
    pub output_path:  String,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            name:                   "tcg-ucs-fe".into(),
            service_name:           "TCG-UCS-FE".into(),
            level:                  "info".into(),
            encoding:               "json".into(),
            mode:                   "file".into(),
            time_format:            "2006-01-02 15:04:05.000".into(),
            path:                   "logs".into(),
            rotation:               "size".into(),
            max_size:               500,
            max_backups:            10,
            keep_days:              5,
            compress:               true,
            stat:                   true,
            buffer_size:            30,
            buffer_flush_interval:  50,
            env:                    "pro".into(),
            output_path:            String::new(),
        }
    }
}

// ── pprof (profiling endpoint) ────────────────────────────────────────────────

#[derive(Debug, Deserialize, Clone, Default)]
pub struct PprofConfig {
    pub enabled: bool,
    pub host:    String,
    pub port:    u16,
}

// ── BigCache (in-process local cache) ────────────────────────────────────────

/// Mirrors Go's `pkg/bigcache.Config`.
///
/// BigCache is a Go-specific in-process LRU cache library.  In a Rust service
/// the equivalent is an in-memory `DashMap` with optional TTL tracking.
/// This struct preserves the TOML key shape so the same config file can drive
/// both services.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct BigCacheConfig {
    /// Entry TTL in seconds (mirrors Go's `EntryExpireSeconds`).
    #[serde(default = "default_300")]
    pub entry_expire_seconds: u64,
}

// ── Consul service discovery ──────────────────────────────────────────────────

/// Mirrors Go's `pkg/consul.Config`.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct ConsulConfig {
    /// Consul key path to watch for dynamic configuration.
    #[serde(default)]
    pub key: String,
    /// Consul agent host addresses.
    #[serde(default)]
    pub hosts: Vec<String>,
}

// ── Top-level application configuration ──────────────────────────────────────

#[derive(Debug, Deserialize, Clone)]
pub struct AppConfig {
    pub name:             String,
    pub env:              String,
    pub host:             String,
    pub port:             u16,
    pub timeout:          u64,
    pub body_limit:       usize,
    pub shutdown_timeout: u64,

    pub timeouts:  AppTimeouts,
    pub oracle:    OracleConfig,
    pub redis:     RedisConfig,
    pub log:       LogConfig,
    pub telemetry: TelemetryConfig,
    pub pprof:     PprofConfig,

    #[serde(alias = "mcsService")]
    pub mcs_service: ServiceConfig,
    #[serde(alias = "ussService")]
    pub uss_service: ServiceConfig,
    #[serde(alias = "wpsService")]
    pub wps_service: ServiceConfig,

    #[serde(default)]
    pub jobs: std::collections::HashMap<String, JobConfig>,

    /// In-process local cache configuration.
    /// Mirrors Go's `Config.BigCache bigcache.Config`.
    #[serde(default, rename = "bigcache")]
    pub bigcache: BigCacheConfig,

    /// Consul service-discovery configuration.
    /// Mirrors Go's `Config.Consul consul.Config`.
    #[serde(default)]
    pub consul: ConsulConfig,

    /// Paths that the OTel trace middleware should skip.
    /// Mirrors Go's `Config.TraceIgnorePaths []string`.
    #[serde(default, rename = "traceIgnorePaths")]
    pub trace_ignore_paths: Vec<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            name:                "ucs-fe".into(),
            env:                 "dev".into(),
            host:                "0.0.0.0".into(),
            port:                7009,
            timeout:             30,
            body_limit:          4 * 1024 * 1024,
            shutdown_timeout:    30,
            timeouts:            AppTimeouts::default(),
            oracle:              OracleConfig::default(),
            redis:               RedisConfig::default(),
            log:                 LogConfig::default(),
            telemetry:           TelemetryConfig::default(),
            pprof:               PprofConfig::default(),
            mcs_service:         ServiceConfig::default(),
            uss_service:         ServiceConfig::default(),
            wps_service:         ServiceConfig::default(),
            bigcache:            BigCacheConfig::default(),
            consul:              ConsulConfig::default(),
            trace_ignore_paths:  Vec::new(),
            jobs:             Default::default(),
        }
    }
}

// ── Config loader ─────────────────────────────────────────────────────────────

impl AppConfig {
    /// Load config for the given environment name.
    ///
    /// Reads `config/{env}.toml` using the `toml` crate directly.
    ///
    /// Using `toml::from_str` (rather than the `config` crate's `try_deserialize`) ensures
    /// that `#[serde(default)]` attributes are properly honoured for nested structs and map
    /// values — the `config` crate 0.15 does not reliably propagate serde defaults for missing
    /// fields inside `HashMap` values or deeply-nested structs.
    ///
    /// Mirrors Go's `config.Init(envStr)`.
    pub fn load_for_env(env: &str) -> anyhow::Result<Self> {
        let path = format!("config/{env}.toml");
        let content = std::fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("Failed to read {path}: {e}"))?;

        let mut cfg: AppConfig = toml::from_str(&content)
            .map_err(|e| anyhow::anyhow!("Failed to deserialise {path}: {e}"))?;

        // Overlay APP__* environment variables (APP__ORACLE__USER, APP__REDIS__ADDR, …).
        // Format: APP__<SECTION>__<KEY>=<value>  (case-insensitive section/key).
        for (k, v) in std::env::vars() {
            let Some(rest) = k.strip_prefix("APP__") else { continue };
            let parts: Vec<&str> = rest.splitn(2, "__").collect();
            if parts.len() != 2 { continue; }
            let (section, key) = (parts[0].to_lowercase(), parts[1].to_lowercase());
            match section.as_str() {
                "oracle" => match key.as_str() {
                    "user"                 => cfg.oracle.user = v,
                    "passwd" | "password"  => cfg.oracle.password = v,
                    "addr_connect_stringer" | "connect_string" => cfg.oracle.connect_string = v,
                    _ => {}
                },
                "redis" => match key.as_str() {
                    "password" => cfg.redis.password = v,
                    _ => {}
                },
                "log" => match key.as_str() {
                    "level"    => cfg.log.level = v,
                    "encoding" => cfg.log.encoding = v,
                    _ => {}
                },
                _ => {}
            }
        }

        Ok(cfg)
    }

    /// Load config for the `ENV` environment variable, defaulting to `"dev"`.
    pub fn load() -> anyhow::Result<Self> {
        let env = std::env::var("ENV").unwrap_or_else(|_| "dev".to_string());
        Self::load_for_env(&env)
    }
}

/// Loads configuration from a TOML file at an explicit path.
pub fn load(path: &str) -> anyhow::Result<AppConfig> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("Failed to read {path}: {e}"))?;
    toml::from_str(&content)
        .map_err(|e| anyhow::anyhow!("Failed to deserialise {path}: {e}"))
}

/// Top-level alias for [`load_oracle_from_aws`] so `config::load_oracle_connect_info`
/// matches the call-site in `main.rs`.
///
/// Mirrors Go's `config.LoadOracleConnectInfoFromAws(envStr)`.
pub async fn load_oracle_connect_info(env: &str) -> anyhow::Result<OracleConnectInfo> {
    load_oracle_from_aws(env).await
}

fn default_sampler()    -> f64    { 1.0 }
fn default_batcher()    -> String { "none".into() }
fn default_300()        -> u64    { 300 }
fn default_30_u64()     -> u64    { 30 }
fn default_60_u64()     -> u64    { 60 }
fn default_15_u64()     -> u64    { 15 }
fn default_100()        -> u32    { 100 }
fn default_info_str()   -> String { "info".into() }
fn default_json_str()   -> String { "json".into() }
fn default_log_name()   -> String { "tcg-ucs-fe".into() }
fn default_service_name() -> String { "TCG-UCS-FE".into() }
fn default_log_mode()   -> String { "file".into() }
fn default_time_format()-> String { "2006-01-02 15:04:05.000".into() }
fn default_rotation()   -> String { "size".into() }
fn default_500()        -> u32    { 500 }
fn default_10()         -> u32    { 10 }
fn default_5()          -> u32    { 5 }
fn default_true()       -> bool   { true }
fn default_30_u32()     -> u32    { 30 }
fn default_50_u32()     -> u32    { 50 }

/// Loads Oracle credentials from AWS Secrets Manager based on `env`.
pub async fn load_oracle_from_aws(env: &str) -> anyhow::Result<OracleConnectInfo> {
    let aws_cfg = aws_config::load_from_env().await;
    let client  = SecretsClient::new(&aws_cfg);

    let secret_name = match env.to_lowercase().as_str() {
        "dev"  => "tcg-uad/db/go-ucs-fe/dev",
        "sit"  => "tcg-uad/db/go-ucs-fe/sit",
        "prod" => "tcg-uad/db/go-ucs-fe",
        other  => anyhow::bail!("unsupported environment: {}", other),
    };

    let resp = client
        .get_secret_value()
        .secret_id(secret_name)
        .version_stage("AWSCURRENT")
        .send()
        .await?;

    let secret_str = resp
        .secret_string()
        .ok_or_else(|| anyhow::anyhow!("secret {} has no string value", secret_name))?;

    let info: OracleConnectInfo = serde_json::from_str(secret_str)?;
    Ok(info)
}
