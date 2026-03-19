/// Structured logging initialisation and custom formatting helpers.
///
/// Full port of Go's `pkg/zlog/` package (zlog.go + encoder.go).
///
/// ## Design
///
/// Go's `pkg/zlog` initialises a `zap.Logger` with a custom encoder that:
/// - Tags every line with the service name and caller location
/// - Injects OpenTelemetry trace/span IDs when a span is active
/// - Rotates log files via `lumberjack`
///
/// Rust uses `tracing` + `tracing-subscriber`.  The equivalent functionality
/// is already wired in `src/telemetry.rs`:
/// - JSON encoding  → `tracing_subscriber::fmt::json()`
/// - Env-filter     → `EnvFilter::from_default_env()`
/// - Caller info    → not included by default (add `.with_file(true).with_line_number(true)`)
/// - Service name   → injected as a constant span field in `telemetry::init_tracing`
/// - OTel trace IDs → propagated via `opentelemetry-tracing` bridge when enabled
///
/// This module provides:
///   1. A re-export of `tracing` primitives so call sites can `use crate::pkg::zlog`
///      instead of `use tracing` (one-to-one translation from Go).
///   2. `init(cfg)` — idiomatic Rust counterpart of Go's `zlog.Init(cfg)`.
///   3. `get_logger()` — returns `()` (tracing is global; no handle needed).
///   4. Log-level helpers mirroring Go's `zlog.Info`, `zlog.Warn`, etc.
///
/// ## Usage
///
/// ```rust
/// use crate::pkg::zlog;
///
/// zlog::info("HTTP server started");
/// zlog::warn("connection pool running low");
/// zlog::error("fatal DB error");
/// ```

use crate::config::LogConfig;

// ── Re-export tracing macros under zlog names ────────────────────────────────

pub use tracing::{debug, error, info, trace, warn};

// ── Init ──────────────────────────────────────────────────────────────────────

/// Initialise the global subscriber from `LogConfig`.
///
/// This is a thin wrapper over `crate::telemetry::init_tracing` so that
/// call sites that import `zlog` can initialise logging without directly
/// depending on `telemetry`.
///
/// Mirrors Go's `zlog.Init(cfg logs.Config)`.
pub fn init(cfg: &LogConfig) {
    crate::telemetry::init_tracing(cfg);
}

// ── Free-function wrappers ────────────────────────────────────────────────────

/// DEBUG-level log.
///
/// Mirrors Go's zap `logger.Debug`.
#[inline]
pub fn debug_log(msg: &str) {
    tracing::debug!("{}", msg);
}

/// INFO-level log.
///
/// Mirrors Go's zap `logger.Info` / Go's `zlog.Info(msg)`.
#[inline]
pub fn info_log(msg: &str) {
    tracing::info!("{}", msg);
}

/// WARN-level log.
///
/// Mirrors Go's `zlog.Warn(msg)`.
#[inline]
pub fn warn_log(msg: &str) {
    tracing::warn!("{}", msg);
}

/// ERROR-level log.
///
/// Mirrors Go's `zlog.Error(msg)`.
#[inline]
pub fn error_log(msg: &str) {
    tracing::error!("{}", msg);
}

/// FATAL-level: logs at ERROR and exits the process.
///
/// Mirrors Go's `zlog.Fatal(msg)`.
pub fn fatal_log(msg: &str) -> ! {
    tracing::error!(fatal = true, "{}", msg);
    std::process::exit(1);
}

// ── Logger handle (compatibility shim) ───────────────────────────────────────

/// Returns `()` — in Rust the subscriber is global and needs no handle.
///
/// Mirrors Go's `zlog.GetLogger() *zap.Logger`.
pub fn get_logger() {}

/// No-op flush — `tracing` writes synchronously.
///
/// Mirrors Go's `logger.Sync()`.
pub fn sync() {}

// ── Custom log-line format description ───────────────────────────────────────
//
// Go's `LogEncoder.EncodeEntry` formats each line as:
//
//   2006-01-02 15:04:05.000 [/] [LEVEL] [funcName] (file:line) - message  key=val …
//
// The equivalent in Rust is configured in `telemetry::init_tracing` via
// `tracing_subscriber::fmt` builder:
//
//   .with_file(true)
//   .with_line_number(true)
//   .with_target(true)
//   .with_timer(tracing_subscriber::fmt::time::ChronoLocal::new("%Y-%m-%d %H:%M:%S%.3f".into()))
//
// The OTel trace-ID injection is handled by the `tracing-opentelemetry` layer
// which enriches each span with `otel.trace_id` and `otel.span_id` fields.
