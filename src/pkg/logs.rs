/// Structured, context-aware logging API.
///
/// Complete Rust port of Go's `pkg/logs/` package (all 5 files):
/// `config.go`, `encoder.go`, `logger.go`, `logs.go`, `writer.go`.
///
/// # Design
///
/// Go's `pkg/logs` wraps `zap.Logger` with:
/// - A singleton global logger + optional behavior logger
/// - Context-aware helpers (`Debug`/`Info`/`Warn`/`Err`) that extract
///   `user_id` and `trace_id` from `context.Context`
/// - A custom encoder (`[ts] [service] [LEVEL] [file:line] - msg | k=v`)
/// - File rotation via lumberjack
/// - Buffered writes flushed on `Close`
///
/// Rust uses `tracing` + `tracing-subscriber` which provides equivalent
/// functionality:
/// - The custom encoder is implemented in `telemetry::TextFormat` / `JsonFormat`
/// - Context propagation happens automatically through tracing spans
/// - Context-aware variants (`info_ctx`, `warn_ctx`, …) accept optional
///   `user_id` / `trace_id` for call sites that have the values explicitly
/// - Behavior logs route through `target: "behavior"` so a future appender
///   can filter them to a separate file (mirrors Go's `PathBehavior`)
/// - Buffered I/O: writes go through a 30 MB `BufWriter` flushed every 10 ms
///
/// # Usage
///
/// ```rust
/// use crate::pkg::logs;
///
/// // Simple
/// logs::info("HTTP server started");
/// logs::warn("retry limit approaching");
/// logs::error("database query failed");
///
/// // With explicit context fields (mirrors Go's ctx parameter)
/// logs::info_ctx("user authenticated", Some("alice"), Some("trace-abc"));
///
/// // Behavior / API-request log (mirrors Go's logs.BehaviorInfo)
/// logs::behavior_info("[traceID/spanID] [API-REQUEST] …");
/// ```
use crate::config::LogConfig;

// ── Init / lifecycle ──────────────────────────────────────────────────────────

/// Initialise the global tracing subscriber from config.
///
/// Must be called **once** at startup before any log calls.
/// Subsequent calls are silently ignored.
///
/// Mirrors Go's `logs.NewLogger(cfg)` / `zlog.Init(cfg)`.
pub fn init(cfg: &LogConfig) {
    crate::telemetry::init_tracing(cfg);
}

/// Flush the buffered log writer immediately.
///
/// Mirrors Go's `logs.Flush()` and `logger.Flush()`.
#[inline]
pub fn flush() {
    crate::telemetry::flush_log_buf();
}

/// Flush and close the logger.
///
/// Mirrors Go's `logs.Close()` / `logger.Close()`.
#[inline]
pub fn close() {
    crate::telemetry::flush_log_buf();
}

// ── Level parser ──────────────────────────────────────────────────────────────

/// Parse a level string to `tracing::Level`, defaulting to `INFO`.
///
/// Mirrors Go's `parseLevel(level string) zapcore.Level` in `logger.go`.
pub fn parse_level(level: &str) -> tracing::Level {
    match level.to_ascii_lowercase().as_str() {
        "debug" => tracing::Level::DEBUG,
        "warn" => tracing::Level::WARN,
        "error" => tracing::Level::ERROR,
        _ => tracing::Level::INFO,
    }
}

// ── Free-function log API ─────────────────────────────────────────────────────
//
// These are the primary call-site API — direct Rust equivalents of Go's
// package-level functions in `logs.go`.

/// DEBUG log.
///
/// Mirrors Go's `logs.Debug(ctx, msg, format...)`.
#[inline]
pub fn debug(msg: &str) {
    tracing::debug!("{}", msg);
}

/// INFO log.
///
/// Mirrors Go's `logs.Info(ctx, msg, format...)`.
#[inline]
pub fn info(msg: &str) {
    tracing::info!("{}", msg);
}

/// WARN log.
///
/// Mirrors Go's `logs.Warn(ctx, msg, format...)`.
#[inline]
pub fn warn(msg: &str) {
    tracing::warn!("{}", msg);
}

/// ERROR log.
///
/// Mirrors Go's `logs.Err(ctx, msg, format...)`.
#[inline]
pub fn error(msg: &str) {
    tracing::error!("{}", msg);
}

/// ERROR log then `process::exit(1)`.
///
/// Mirrors Go's `logs.Fatal(ctx, msg, format...)`.
pub fn fatal(msg: &str) -> ! {
    tracing::error!(fatal = true, "{}", msg);
    crate::telemetry::flush_log_buf();
    std::process::exit(1);
}

/// Formatted ERROR log then `process::exit(1)`.
///
/// Mirrors Go's `logs.Fatalf(ctx, format, args...)`.
pub fn fatalf(msg: &str) -> ! {
    tracing::error!(fatal = true, "{}", msg);
    crate::telemetry::flush_log_buf();
    std::process::exit(1);
}

/// Formatted ERROR log then panic.
///
/// Mirrors Go's `logs.Panicf(ctx, format, args...)`.
/// The panic hook installed by `init_tracing` will also flush, but we
/// flush here first for defense-in-depth.
pub fn panic_log(msg: &str) -> ! {
    tracing::error!(panic = true, "{}", msg);
    crate::telemetry::flush_log_buf();
    panic!("{}", msg);
}

// ── Context-aware variants ────────────────────────────────────────────────────
//
// Go's `appendContextFields` extracts `user_id` and `trace_id` from
// `context.Context`.  In Rust these propagate automatically through tracing
// spans, but these helpers are useful when the values are available explicitly
// at the call site (e.g. from HTTP headers or JWT claims).

/// INFO log with optional context fields.
///
/// Mirrors Go's `logs.Info(ctx, msg, ...)` where `ctx` carries `user_id` / `trace_id`.
pub fn info_ctx(msg: &str, user_id: Option<&str>, trace_id: Option<&str>) {
    match (user_id, trace_id) {
        (Some(uid), Some(tid)) => tracing::info!(user_id = uid, trace_id = tid, "{}", msg),
        (Some(uid), None) => tracing::info!(user_id = uid, "{}", msg),
        (None, Some(tid)) => tracing::info!(trace_id = tid, "{}", msg),
        (None, None) => tracing::info!("{}", msg),
    }
}

/// WARN log with optional context fields.
///
/// Mirrors Go's `logs.Warn(ctx, msg, ...)`.
pub fn warn_ctx(msg: &str, user_id: Option<&str>, trace_id: Option<&str>) {
    match (user_id, trace_id) {
        (Some(uid), Some(tid)) => tracing::warn!(user_id = uid, trace_id = tid, "{}", msg),
        (Some(uid), None) => tracing::warn!(user_id = uid, "{}", msg),
        (None, Some(tid)) => tracing::warn!(trace_id = tid, "{}", msg),
        (None, None) => tracing::warn!("{}", msg),
    }
}

/// ERROR log with optional context fields.
///
/// Mirrors Go's `logs.Err(ctx, msg, ...)`.
pub fn error_ctx(msg: &str, user_id: Option<&str>, trace_id: Option<&str>) {
    match (user_id, trace_id) {
        (Some(uid), Some(tid)) => tracing::error!(user_id = uid, trace_id = tid, "{}", msg),
        (Some(uid), None) => tracing::error!(user_id = uid, "{}", msg),
        (None, Some(tid)) => tracing::error!(trace_id = tid, "{}", msg),
        (None, None) => tracing::error!("{}", msg),
    }
}

// ── Field helpers ─────────────────────────────────────────────────────────────

/// Create a `key=value` log field string.
///
/// Mirrors Go's `logs.Any(key, data any) zap.Field`.
/// In idiomatic Rust, prefer `tracing::info!(key = %value, "msg")` directly.
pub fn any_field(key: &str, value: impl std::fmt::Display) -> String {
    format!("{key}={value}")
}

/// Create a `flag=<value>` log field string.
///
/// Mirrors Go's `logs.Flag(flag string) zap.Field`.
pub fn flag(value: &str) -> String {
    format!("flag={value}")
}

// ── Behavior logger ───────────────────────────────────────────────────────────
//
// Go's behavior logger writes API-request log lines to a dedicated file
// (`Name-behavior.log`) via a separate `zap.Logger` instance.
//
// In Rust we route behavior events through tracing with `target: "behavior"`.
// A future `tracing-appender` layer or `EnvFilter` directive can redirect
// events with this target to a separate file, matching Go's `PathBehavior`
// configuration.
//
// Mirrors Go's `behaviorLog`, `BehaviorInfo`, `BehaviorWarn`, `BehaviorError`
// in `logs.go`.

/// Write an INFO-level API-request log line.
///
/// Mirrors Go's `logs.BehaviorInfo(msg)`.
#[inline]
pub fn behavior_info(msg: &str) {
    tracing::info!(target: "behavior", "{}", msg);
}

/// Write a WARN-level API-request log line.
///
/// Mirrors Go's `logs.BehaviorWarn(msg)`.
#[inline]
pub fn behavior_warn(msg: &str) {
    tracing::warn!(target: "behavior", "{}", msg);
}

/// Write an ERROR-level API-request log line.
///
/// Mirrors Go's `logs.BehaviorError(msg)`.
#[inline]
pub fn behavior_error(msg: &str) {
    tracing::error!(target: "behavior", "{}", msg);
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_level_known_values() {
        assert_eq!(parse_level("debug"), tracing::Level::DEBUG);
        assert_eq!(parse_level("DEBUG"), tracing::Level::DEBUG);
        assert_eq!(parse_level("info"), tracing::Level::INFO);
        assert_eq!(parse_level("warn"), tracing::Level::WARN);
        assert_eq!(parse_level("error"), tracing::Level::ERROR);
    }

    #[test]
    fn parse_level_defaults_to_info() {
        assert_eq!(parse_level("unknown"), tracing::Level::INFO);
        assert_eq!(parse_level(""), tracing::Level::INFO);
        assert_eq!(parse_level("trace"), tracing::Level::INFO); // Go maps to info too
    }

    #[test]
    fn field_helpers() {
        assert_eq!(any_field("user_id", "alice"), "user_id=alice");
        assert_eq!(any_field("count", 42), "count=42");
        assert_eq!(flag("db-timeout"), "flag=db-timeout");
    }
}
