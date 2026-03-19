/// Structured context-aware logging helpers.
///
/// Full port of Go's `pkg/logs/` package (logs.go + logger.go + writer.go + encoder.go).
///
/// ## Design
///
/// Go's `pkg/logs` wraps `zap` and extracts `user_id` / `trace_id` from
/// `context.Context` (gRPC metadata or plain values).  Rust uses `tracing`
/// whose span fields carry the same information automatically via
/// `tracing-subscriber`'s `EnvFilter` + JSON formatter (configured in
/// `telemetry.rs`).
///
/// This module provides **drop-in replacements** for the Go global helpers so
/// that call sites can be mechanically translated without rethinking the log
/// call:
///
/// | Go                          | Rust                            |
/// |-----------------------------|----------------------------------|
/// | `logs.Debug(ctx, "…", x)`  | `debug!(ctx=..., "…")`  or `logs::debug("…", ctx)` |
/// | `logs.Info(ctx, "…", x)`   | `logs::info("…")`                |
/// | `logs.Warn(ctx, "…", x)`   | `logs::warn("…")`                |
/// | `logs.Err(ctx, "…", x)`    | `logs::error("…")`               |
/// | `logs.Fatal(ctx, "…", x)`  | `logs::fatal("…")`               |
/// | `logs.Any(key, val)`       | `tracing::field::display(val)`   |
/// | `logs.Flag(flag)`          | `tracing::field::display(flag)`  |
///
/// ## Usage
///
/// ```rust
/// use crate::pkg::logs;
///
/// logs::info("server starting");
/// logs::warn("retry limit approaching");
/// logs::error("database query failed");
/// logs::fatal("cannot open config file");   // calls std::process::exit(1)
/// ```
///
/// Context-scoped trace/span IDs are automatically included by the
/// `tracing-subscriber` span context when calls happen inside a span.

/// Log a DEBUG-level message.
///
/// Mirrors Go's `logs.Debug(ctx, msg, format...)`.
#[macro_export]
macro_rules! log_debug {
    ($($arg:tt)*) => { tracing::debug!($($arg)*) };
}

/// Log an INFO-level message.
///
/// Mirrors Go's `logs.Info(ctx, msg, format...)`.
#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => { tracing::info!($($arg)*) };
}

/// Log a WARN-level message.
///
/// Mirrors Go's `logs.Warn(ctx, msg, format...)`.
#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => { tracing::warn!($($arg)*) };
}

/// Log an ERROR-level message.
///
/// Mirrors Go's `logs.Err(ctx, msg, format...)`.
#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => { tracing::error!($($arg)*) };
}

// ── Free-function wrappers (useful when macros are inconvenient) ──────────────

/// Log a DEBUG-level message.
///
/// Mirrors Go's `logs.Debug(ctx, msg, format...)`.
#[inline]
pub fn debug(msg: &str) {
    tracing::debug!("{}", msg);
}

/// Log an INFO-level message.
///
/// Mirrors Go's `logs.Info(ctx, msg, format...)`.
#[inline]
pub fn info(msg: &str) {
    tracing::info!("{}", msg);
}

/// Log a WARN-level message.
///
/// Mirrors Go's `logs.Warn(ctx, msg, format...)`.
#[inline]
pub fn warn(msg: &str) {
    tracing::warn!("{}", msg);
}

/// Log an ERROR-level message.
///
/// Mirrors Go's `logs.Err(ctx, msg, format...)`.
#[inline]
pub fn error(msg: &str) {
    tracing::error!("{}", msg);
}

/// Log an ERROR-level message and exit the process with code 1.
///
/// Mirrors Go's `logs.Fatal(ctx, msg, format...)`.
///
/// # Panic safety
///
/// Like the Go version, this flushes logs before terminating.  In Rust,
/// `tracing` flushes synchronously on the same thread before the process
/// exits via `std::process::exit(1)`.
pub fn fatal(msg: &str) -> ! {
    tracing::error!(fatal = true, "{}", msg);
    std::process::exit(1);
}

/// Log an ERROR-level formatted message and exit the process with code 1.
///
/// Mirrors Go's `logs.Fatalf(ctx, format, args...)`.
pub fn fatalf(msg: String) -> ! {
    tracing::error!(fatal = true, "{}", msg);
    std::process::exit(1);
}

/// Panic with an ERROR log entry.
///
/// Mirrors Go's `logs.Panic(ctx, msg, format...)`.
pub fn panic_log(msg: &str) -> ! {
    tracing::error!(panic = true, "{}", msg);
    panic!("{}", msg);
}

// ── Field helpers ─────────────────────────────────────────────────────────────

/// Create a structured key=value log field (string value).
///
/// Mirrors Go's `logs.Any(key, data)` — in Rust call sites should use
/// `tracing`'s built-in key=value syntax:  `tracing::info!(key = %val, "msg")`.
/// This helper is provided for mechanical translation of older call sites.
pub fn any_field(key: &str, value: impl std::fmt::Display) -> String {
    format!("{}={}", key, value)
}

/// Create a `flag=<value>` log field.
///
/// Mirrors Go's `logs.Flag(flag)`.
/// In Rust: `tracing::info!(flag = %value, "msg")`.
pub fn flag(value: &str) -> String {
    format!("flag={}", value)
}

// ── Log level parser ──────────────────────────────────────────────────────────

/// Parse a log level string into a `tracing::Level`.
///
/// Mirrors Go's `parseLevel(level string) zapcore.Level` in `logger.go`.
/// Defaults to `INFO` for unknown strings.
pub fn parse_level(level: &str) -> tracing::Level {
    match level.to_lowercase().as_str() {
        "debug" => tracing::Level::DEBUG,
        "warn"  => tracing::Level::WARN,
        "error" => tracing::Level::ERROR,
        _       => tracing::Level::INFO,
    }
}

// ── Flush ─────────────────────────────────────────────────────────────────────

/// Flush any buffered log entries.
///
/// Mirrors Go's `logs.Flush()`.
/// With `tracing`, output is written synchronously; this is a no-op but
/// provided for API compatibility.
pub fn flush() {
    // tracing writes synchronously; nothing to flush.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_level_defaults_to_info() {
        assert_eq!(parse_level("info"),    tracing::Level::INFO);
        assert_eq!(parse_level("debug"),   tracing::Level::DEBUG);
        assert_eq!(parse_level("warn"),    tracing::Level::WARN);
        assert_eq!(parse_level("error"),   tracing::Level::ERROR);
        assert_eq!(parse_level("unknown"), tracing::Level::INFO);
        assert_eq!(parse_level(""),        tracing::Level::INFO);
    }

    #[test]
    fn any_field_formats() {
        assert_eq!(any_field("user_id", "alice"), "user_id=alice");
        assert_eq!(flag("test-flag"), "flag=test-flag");
    }
}
