//! Application configuration.
//!
//! Loaded from a TOML file (dev/sit/prod) via the `config` crate.
//! Mirrors Go's `internal/config/config.go` and `internal/config/init.go`.
//!
//! ## Environment selection
//!
//! The file to load is chosen in the following priority order:
//!
//! 1. Command-line flag `-f <path>` — passed as the `path` argument to `AppConfig::load`.
//! 2. `ENV` environment variable (`dev` | `sit` | `prod`) — auto-maps to
//!    `./config/{env}.toml`.
//! 3. Hard-coded fallback: `./config/dev.toml`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Sub-structs ───────────────────────────────────────────────────────────────

/// Oracle connection pool settings.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OracleConfig {
    #[serde(default)]
    pub user: String,
    #[serde(default, rename = "passwd")]
    pub password: String,
    #[serde(default, rename = "addr_connect_stringer")]
    pub connect_string: String,
    #[serde(default = "default_max_open")]
    pub max_open_conn: u32,
    #[serde(default = "default_max_idle")]
    pub max_idle_conn: Option<u32>,
    #[serde(default = "default_read_timeout", rename = "read_timeout")]
    pub read_timeout_secs: u64,
    #[serde(default = "default_write_timeout", rename = "write_timeout")]
    pub write_timeout_secs: u64,
}

impl Default for OracleConfig {
    fn default() -> Self {
        Self {
            user: String::new(),
            password: String::new(),
            connect_string: String::new(),
            max_open_conn: default_max_open(),
            max_idle_conn: default_max_idle(),
            read_timeout_secs: default_read_timeout(),
            write_timeout_secs: default_write_timeout(),
        }
    }
}

fn default_max_open() -> u32 {
    100
}
fn default_max_idle() -> Option<u32> {
    Some(10)
}
fn default_read_timeout() -> u64 {
    15
}
fn default_write_timeout() -> u64 {
    15
}

/// Redis Sentinel connection settings.
///
/// `addr` is a list of sentinel addresses (or a single node address).
/// `rate_limit_db` is the Redis DB index used for rate-limiting Lua scripts
/// — mirrors Go's `InitSentinelDBS` / `GetDbInstance(2)`.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct RedisConfig {
    #[serde(default, rename = "addr")]
    pub sentinel_addrs: Vec<String>,
    #[serde(default)]
    pub master_name: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub db: i64,
    /// DB index used for rate-limiting keys.
    #[serde(default)]
    pub rate_limit_db: i64,
    #[serde(default = "default_pool")]
    pub pool_size: u32,
}

fn default_pool() -> u32 {
    50
}

/// Downstream HTTP service (USS / MCS).
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct HttpServiceConfig {
    #[serde(default)]
    pub host: String,
    #[serde(default, rename = "basePath")]
    pub base_path: String,
}

/// Logging configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LogConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
    /// Output format: `"json"` | `"text"`.
    #[serde(default = "default_log_format")]
    pub format: String,
    #[serde(default, rename = "serviceName")]
    pub service_name: String,
    /// Log mode: `"console"` | `"file"` | `"kafka"` (mirrors Go).
    #[serde(default = "default_log_mode")]
    pub mode: String,
}

fn default_log_level() -> String {
    "info".to_string()
}
fn default_log_format() -> String {
    "text".to_string()
}
fn default_log_mode() -> String {
    "console".to_string()
}

impl Default for LogConfig {
    fn default() -> Self {
        LogConfig {
            level: default_log_level(),
            format: default_log_format(),
            service_name: "TCG-UCS-FE".to_string(),
            mode: default_log_mode(),
        }
    }
}

/// OpenTelemetry / tracing export configuration.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct TelemetryConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub endpoint: String,
    #[serde(default, rename = "server_name")]
    pub service_name: String,
    /// Paths that OTel tracing should skip.
    #[serde(default, rename = "skipPaths")]
    pub skip_paths: Vec<String>,
}

/// Per-operation HTTP timeout values (seconds).
///
/// Mirrors Go's `AppTimeouts` in `internal/config/config.go`.
///
/// | Name     | Go default | Purpose                        |
/// |----------|------------|--------------------------------|
/// | `quick`  | 5 s        | `/verification/questions`      |
/// | `normal` | 30 s       | `/verification/materials`      |
/// | `long`   | 60 s       | Long-running analytics routes  |
/// | `upload` | 120 s      | File-upload routes             |
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AppTimeouts {
    #[serde(default = "default_quick")]
    pub quick: u64,
    #[serde(default = "default_normal")]
    pub normal: u64,
    #[serde(default = "default_long")]
    pub long: u64,
    #[serde(default = "default_upload")]
    pub upload: u64,
}

fn default_quick() -> u64 {
    5
}
fn default_normal() -> u64 {
    30
}
fn default_long() -> u64 {
    60
}
fn default_upload() -> u64 {
    120
}

impl Default for AppTimeouts {
    fn default() -> Self {
        AppTimeouts {
            quick: default_quick(),
            normal: default_normal(),
            long: default_long(),
            upload: default_upload(),
        }
    }
}

/// Global rate-limit settings (applied at the router level).
///
/// Mirrors Go's `limiter.New` in `internal/router/routes.go`:
/// - `max_rps`       → group-level global limiter (key = `"global"`) — 800/s
/// - `per_path_rps`  → per-path limiter (key = request path)         — 500/s
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RateLimitConfig {
    /// Max requests-per-second for the whole service (global token bucket).
    #[serde(default = "default_global_rps")]
    pub max_rps: u32,
    /// Max requests-per-second keyed by request path.
    #[serde(default = "default_path_rps")]
    pub per_path_rps: u32,
}

fn default_global_rps() -> u32 {
    800
}
fn default_path_rps() -> u32 {
    500
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        RateLimitConfig {
            max_rps: default_global_rps(),
            per_path_rps: default_path_rps(),
        }
    }
}

/// Per-job cron configuration — matches Go's `JobConfig`.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct JobConfig {
    #[serde(default)]
    pub cron: String,
    #[serde(default = "default_interval")]
    pub interval: u64,
    #[serde(default = "default_job_timeout")]
    pub timeout: u64,
    #[serde(default)]
    pub concurrency: u32,
    #[serde(default)]
    pub enabled: bool,
}

fn default_interval() -> u64 {
    60
}
fn default_job_timeout() -> u64 {
    30
}

// ── Top-level AppConfig ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AppConfig {
    #[serde(default = "default_name")]
    pub name: String,
    #[serde(default = "default_env")]
    pub env: String,
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    /// Global request timeout in seconds (fallback when no route-level timeout applies).
    #[serde(default = "default_timeout")]
    pub timeout: u64,
    #[serde(default = "default_shutdown")]
    pub shutdown_timeout: u64,
    /// Maximum request body size in bytes (default 50 MiB — mirrors Go's `BodyLimit`).
    #[serde(default = "default_body_limit", rename = "bodyLimit")]
    pub body_limit: usize,

    #[serde(default)]
    pub oracle: OracleConfig,
    #[serde(default)]
    pub redis: RedisConfig,
    #[serde(default, rename = "ussService")]
    pub uss_service: HttpServiceConfig,
    #[serde(default, rename = "mcsService")]
    pub mcs_service: HttpServiceConfig,

    #[serde(default)]
    pub log: LogConfig,
    #[serde(default)]
    pub telemetry: TelemetryConfig,
    /// Per-operation timeouts — mirrors Go's `AppTimeouts`.
    #[serde(default, rename = "timeouts")]
    pub app_timeouts: AppTimeouts,
    #[serde(default)]
    pub rate_limit: RateLimitConfig,
    /// Named background jobs.
    #[serde(default)]
    pub jobs: HashMap<String, JobConfig>,
}

fn default_name() -> String {
    "tcg-ucs-fe".to_string()
}
fn default_env() -> String {
    "prod".to_string()
}
fn default_host() -> String {
    "0.0.0.0".to_string()
}
fn default_port() -> u16 {
    7009
}
fn default_timeout() -> u64 {
    30
}
fn default_shutdown() -> u64 {
    30
}
fn default_body_limit() -> usize {
    10 * 1024 * 1024 * 5
} // 50 MiB

impl AppConfig {
    /// Load configuration.
    ///
    /// `path` is the explicit config file path (from the `-f` CLI flag).
    /// When `path` is empty, the `ENV` environment variable is used to pick
    /// `./config/{dev|sit|prod}.toml`, defaulting to `dev` if unset.
    pub fn load(path: &str) -> anyhow::Result<Self> {
        let resolved = resolve_config_path(path);
        tracing::info!("Loading config from: {}", resolved);

        let cfg = config::Config::builder()
            .add_source(
                config::File::with_name(&resolved)
                    .format(config::FileFormat::Toml)
                    .required(true),
            )
            .add_source(config::Environment::with_prefix("APP").separator("__"))
            .build()?;

        Ok(cfg.try_deserialize()?)
    }

    /// `"host:port"` bind string.
    pub fn server_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// Resolve the actual config file path from either an explicit override
/// or the `ENV` environment variable, mirroring Go's `config.Init` logic.
fn resolve_config_path(explicit: &str) -> String {
    if !explicit.is_empty() {
        return explicit.to_string();
    }

    let env = std::env::var("ENV").unwrap_or_default().to_lowercase();

    let file_name = match env.as_str() {
        "dev" => "dev.toml",
        "sit" => "sit.toml",
        "prod" => "prod.toml",
        _ => {
            eprintln!(
                "[config] ENV='{}' unrecognised or unset; falling back to dev.toml",
                env
            );
            "dev.toml"
        }
    };

    // Search the same paths as Go's viper config.
    for base in &[".", "./config", "../config", "../../config"] {
        let path = format!("{}/{}", base, file_name);
        if std::path::Path::new(&path).exists() {
            return path;
        }
    }

    // Last resort — hand the path to `config` crate and let it fail with a
    // meaningful error message.
    format!("./config/{}", file_name)
}
