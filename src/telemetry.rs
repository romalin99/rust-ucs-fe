//! Tracing / structured-logging initialisation.
//!
//! Full port of Go's `pkg/logs/encoder.go` custom log encoder, adapted for
//! the `tracing` ecosystem.
//!
//! Two output formats are supported, selected by `LogConfig.encoding`:
//!
//! | encoding | format |
//! |----------|--------|
//! | `"json"` | `{"time":"…","level":"INFO","service":"ucs-fe","file_line":"src/foo.rs:42","message":"…",…}` |
//! | anything else | `[2026-03-20 10:00:00.000] [ucs-fe] [INFO ] [foo.rs:42] - message \| key=val, …` |
//!
//! The text format mirrors Go's `LogEncoder.EncodeEntry`:
//! ```text
//! [timestamp] [ServiceName] [LEVEL ] [file.go:42] - message | key=val, key2=val2
//! ```

use std::fmt;
use std::sync::OnceLock;

use chrono::SecondsFormat;
use serde_json::{Map, Value};
use tracing::{
    Event, Subscriber,
    field::{Field, Visit},
};
use tracing_subscriber::{
    EnvFilter,
    fmt::{FmtContext, FormatEvent, FormatFields, format},
    layer::SubscriberExt,
    registry::LookupSpan,
    util::SubscriberInitExt,
};

use crate::config::LogConfig;

// ── Global service name ───────────────────────────────────────────────────────

/// Set once during `init_tracing`; used by both formatters.
static SERVICE_NAME: OnceLock<String> = OnceLock::new();

/// Returns the service name injected by `init_tracing`, or `"ucs-fe"`.
pub fn service_name() -> &'static str {
    SERVICE_NAME.get().map(String::as_str).unwrap_or("ucs-fe")
}

// ── Shared field visitor ──────────────────────────────────────────────────────

/// Collects all tracing `Event` fields into an ordered `serde_json::Map`.
/// Used by both formatters.
struct FieldCollector(Map<String, Value>);

impl Visit for FieldCollector {
    fn record_f64(&mut self, field: &Field, value: f64) {
        self.0.insert(field.name().to_owned(), value.into());
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.0.insert(field.name().to_owned(), value.into());
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.0.insert(field.name().to_owned(), value.into());
    }
    fn record_i128(&mut self, field: &Field, value: i128) {
        self.0.insert(field.name().to_owned(), value.to_string().into());
    }
    fn record_u128(&mut self, field: &Field, value: u128) {
        self.0.insert(field.name().to_owned(), value.to_string().into());
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.0.insert(field.name().to_owned(), value.into());
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.into());
    }
    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.0.insert(field.name().to_owned(), value.to_string().into());
    }
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}").into());
    }
}

// ── JSON formatter ────────────────────────────────────────────────────────────

/// Structured JSON log line.
///
/// Output shape (one line per event):
/// ```json
/// {"time":"2026-03-20T10:00:00.000Z","level":"INFO","service":"ucs-fe","file_line":"src/foo.rs:42","message":"…","key":"val"}
/// ```
///
/// The `"service"` field is added here so that every line is self-describing,
/// mirroring the way Go's `LogEncoder` stamps `ServiceName` on every entry.
pub struct JsonFormat;

impl<S, N> FormatEvent<S, N> for JsonFormat
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        _ctx: &FmtContext<'_, S, N>,
        mut writer: format::Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let meta = event.metadata();

        let mut collector = FieldCollector(Map::new());
        event.record(&mut collector);

        // Promote "message" to top-level.
        let message = collector
            .0
            .remove("message")
            .unwrap_or(Value::String(String::new()));

        // "file_line" = "src/foo.rs:42" (same shape as Go's ShortCallerEncoder)
        let file_line = format!(
            "{}:{}",
            meta.file().unwrap_or("<unknown>"),
            meta.line().unwrap_or(0),
        );

        let now = chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);

        let mut entry: Map<String, Value> = Map::new();
        entry.insert("time".into(),      now.into());
        entry.insert("level".into(),     meta.level().to_string().into());
        entry.insert("service".into(),   service_name().into());
        entry.insert("file_line".into(), file_line.into());
        entry.insert("message".into(),   message);
        entry.extend(collector.0); // remaining structured fields

        writeln!(writer, "{}", Value::Object(entry))
    }
}

// ── Text formatter ────────────────────────────────────────────────────────────

/// Human-readable log line, mirroring Go's `LogEncoder.EncodeEntry`.
///
/// Output shape:
/// ```text
/// [2026-03-20 10:00:00.000] [ucs-fe] [INFO ] [foo.rs:42] - message | key=val, key2=val2
/// ```
///
/// Field encoding mirrors Go's `encodeFields`:
/// - String / bool / integer → plain value
/// - Everything else → `fmt::Debug` / `fmt::Display`
pub struct TextFormat {
    service_name: String,
}

impl TextFormat {
    pub fn new(service_name: impl Into<String>) -> Self {
        Self { service_name: service_name.into() }
    }
}

impl<S, N> FormatEvent<S, N> for TextFormat
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        _ctx: &FmtContext<'_, S, N>,
        mut writer: format::Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let meta = event.metadata();

        let mut collector = FieldCollector(Map::new());
        event.record(&mut collector);

        // Promote "message".
        let message = collector
            .0
            .remove("message")
            .and_then(|v| v.as_str().map(|s| s.to_owned()))
            .unwrap_or_default();

        // Only the base filename (mirrors Go's `path.Base(ent.Caller.File)`).
        let file = meta
            .file()
            .map(|f| f.rsplit('/').next().unwrap_or(f))
            .unwrap_or("<unknown>");
        let line = meta.line().unwrap_or(0);

        // Timestamp in Go's default layout `2006-01-02 15:04:05.000`.
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");

        // Level is left-padded to 5 characters (matches `%-5s` in Go's fmt.Fprintf).
        let level = format!("{:<5}", meta.level().to_string().to_uppercase());

        // Core line: [ts] [service] [LEVEL] [file:line] - message
        write!(writer, "[{}] [{}] [{}] [{}:{}] - {}",
            now, self.service_name, level, file, line, message)?;

        // Extra fields as `| key=val, key2=val2`
        // Mirrors Go's `encodeFields`.
        if !collector.0.is_empty() {
            write!(writer, " |")?;
            let mut first = true;
            for (k, v) in &collector.0 {
                write!(writer, "{}", if first { " " } else { ", " })?;
                first = false;
                write!(writer, "{}={}", k, v)?;
            }
        }

        writeln!(writer)
    }
}

// ── Public init ───────────────────────────────────────────────────────────────

/// Initialise the global tracing subscriber from `LogConfig`.
///
/// Mirrors Go's `logs.NewLogger(cfg)` + `cfg.InitLog()`.
///
/// Must be called **once** before any `tracing::*` calls.
/// Subsequent calls are silently ignored (tracing uses a global subscriber).
///
/// | `cfg.encoding` | format |
/// |---------------|--------|
/// | `"json"`      | [`JsonFormat`] — structured JSON, one line per event |
/// | anything else | [`TextFormat`] — `[ts] [service] [LEVEL] [file:line] - msg \| key=val` |
pub fn init_tracing(cfg: &LogConfig) {
    // Publish service name for formatters.
    SERVICE_NAME.set(cfg.service_name.clone()).ok();

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(&cfg.level));

    if cfg.encoding == "json" {
        let _ = tracing_subscriber::registry()
            .with(env_filter)
            .with(tracing_subscriber::fmt::layer().event_format(JsonFormat))
            .try_init();
    } else {
        let _ = tracing_subscriber::registry()
            .with(env_filter)
            .with(
                tracing_subscriber::fmt::layer()
                    .event_format(TextFormat::new(&cfg.service_name)),
            )
            .try_init();
    }
}
