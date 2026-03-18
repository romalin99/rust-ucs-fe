pub mod aws;

use std::collections::HashMap;

use anyhow::{Context, Result};
use serde::Deserialize;

pub use aws::{OracleConnectInfo, load_oracle_connect_info};

// ── Sub-config structs ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Clone)]
pub struct ServiceConfig {
    pub host: String,
    pub base_path: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct OracleConfig {
    pub user: String,
    pub passwd: String,
    pub addr_connect_stringer: String,
    #[serde(default = "default_100")]
    pub max_open_conn: u32,
    #[serde(default = "default_100")]
    pub max_idle_conn: u32,
    #[serde(default = "default_30")]
    pub max_life_time: u64,
    #[serde(default = "default_30")]
    pub max_idle_time: u64,
    #[serde(default)]
    pub enable_stats_monitor: bool,
    #[serde(default = "default_60")]
    pub stats_interval: u64,
    #[serde(default = "default_15")]
    pub read_timeout: u64,
    #[serde(default = "default_15")]
    pub write_timeout: u64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RedisDbConfig {
    pub db: i64,
    #[serde(default = "default_50")]
    pub pool_size: u32,
    #[serde(default)]
    pub set_default_expiration: u64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RedisConfig {
    pub addr: Vec<String>,
    #[serde(default)]
    pub master_name: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub db: i64,
    #[serde(default)]
    pub dbs: Vec<RedisDbConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct JobConfig {
    #[serde(default)]
    pub cron: String,
    #[serde(default)]
    pub interval: u64,
    #[serde(default = "default_60")]
    pub timeout: u64,
    #[serde(default)]
    pub concurrency: u32,
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AppTimeouts {
    #[serde(default = "default_5")]
    pub quick: u64,
    #[serde(default = "default_30")]
    pub normal: u64,
    #[serde(default = "default_60")]
    pub long: u64,
    #[serde(default = "default_120")]
    pub upload: u64,
}

impl Default for AppTimeouts {
    fn default() -> Self {
        Self {
            quick: 5,
            normal: 30,
            long: 60,
            upload: 120,
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct LogConfig {
    #[serde(default = "default_info")]
    pub level: String,
    /// Log output encoding: `"json"` (default) or `"text"`.
    /// Maps to TOML key `encoding`.
    #[serde(default = "default_json")]
    pub encoding: String,
    #[serde(default = "default_service_name", rename = "serviceName")]
    pub service_name: String,
    #[serde(default)]
    pub path: String,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: "info".into(),
            encoding: "json".into(),
            service_name: "UCS-FE-RUST".into(),
            path: "/var/log/ucsfeService".into(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct PprofConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_6068")]
    pub port: u16,
    #[serde(default = "default_host")]
    pub host: String,
}

impl Default for PprofConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            port: 6068,
            host: "0.0.0.0".into(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct TelemetryConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_service_name")]
    pub server_name: String,
    #[serde(default = "default_otlp_endpoint")]
    pub endpoint: String,
    #[serde(default = "default_sampler")]
    pub sampler: f64,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            server_name: "UCS-FE-RUST".into(),
            endpoint: "localhost:4318".into(),
            sampler: 1.0,
        }
    }
}

// ── Root config ───────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Clone)]
pub struct AppConfig {
    pub name: String,
    pub env: String,
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_30_i64")]
    pub timeout: i64,
    #[serde(default = "default_body_limit")]
    pub body_limit: usize,
    #[serde(default = "default_30")]
    pub shutdown_timeout: u64,

    #[serde(default)]
    pub timeouts: AppTimeouts,

    #[serde(default)]
    pub jobs: HashMap<String, JobConfig>,

    pub oracle: OracleConfig,
    pub redis: RedisConfig,

    pub uss_service: ServiceConfig,
    pub mcs_service: ServiceConfig,
    pub wps_service: ServiceConfig,

    #[serde(default)]
    pub log: LogConfig,
    #[serde(default)]
    pub pprof: PprofConfig,
    #[serde(default)]
    pub telemetry: TelemetryConfig,
}

impl AppConfig {
    /// Load config from `config/{env}.toml`.
    /// `env` defaults to the `ENV` environment variable, falling back to `"dev"`.
    pub fn load() -> Result<Self> {
        let env = std::env::var("ENV").unwrap_or_else(|_| "dev".to_string());
        Self::load_for_env(&env)
    }

    pub fn load_for_env(env: &str) -> Result<Self> {
        let cfg = config::Config::builder()
            .add_source(
                config::File::with_name(&format!("config/{}", env))
                    .format(config::FileFormat::Toml)
                    .required(true),
            )
            .add_source(config::Environment::with_prefix("APP").separator("__"))
            .build()
            .with_context(|| format!("Failed to load config from config/{env}.toml"))?;

        cfg.try_deserialize()
            .with_context(|| format!("Failed to deserialise config from config/{env}.toml"))
    }

    /// Update oracle credentials from AWS Secrets Manager and return a new config.
    pub fn with_oracle_credentials(mut self, info: &OracleConnectInfo) -> Self {
        self.oracle.user = info.user.clone();
        self.oracle.passwd = info.password.clone();
        self.oracle.addr_connect_stringer = info.connect_string.clone();
        self
    }
}

// ── Default helpers ───────────────────────────────────────────────────────────
fn default_json() -> String {
    "json".to_string()
}
fn default_100() -> u32 {
    100
}
fn default_50() -> u32 {
    50
}
fn default_30() -> u64 {
    30
}
fn default_30_i64() -> i64 {
    30
}
fn default_15() -> u64 {
    15
}
fn default_60() -> u64 {
    60
}
fn default_5() -> u64 {
    5
}
fn default_120() -> u64 {
    120
}
fn default_6068() -> u16 {
    6068
}
fn default_port() -> u16 {
    7009
}
fn default_host() -> String {
    "0.0.0.0".to_string()
}
fn default_info() -> String {
    "info".to_string()
}
fn default_service_name() -> String {
    "UCS-FE-RUST".to_string()
}
fn default_otlp_endpoint() -> String {
    "localhost:4318".to_string()
}
fn default_sampler() -> f64 {
    1.0
}
fn default_body_limit() -> usize {
    52_428_800
}
