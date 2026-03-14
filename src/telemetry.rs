//! Tracing / structured-logging initialisation.
//!
//! Provides a custom JSON formatter that emits a single
//! `"file_line": "src/foo.rs:42"` field instead of the default
//! separate `"filename"` / `"line_number"` fields produced by
//! `tracing_subscriber`'s built-in JSON layer.

use std::fmt;

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

// ── Field visitor ─────────────────────────────────────────────────────────────

/// Visits every field on a tracing `Event` and collects them into a
/// `serde_json::Map` so we can serialise the whole event ourselves.
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
        self.0
            .insert(field.name().to_owned(), value.to_string().into());
    }
    fn record_u128(&mut self, field: &Field, value: u128) {
        self.0
            .insert(field.name().to_owned(), value.to_string().into());
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.0.insert(field.name().to_owned(), value.into());
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.into());
    }
    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.0
            .insert(field.name().to_owned(), value.to_string().into());
    }
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.0
            .insert(field.name().to_owned(), format!("{value:?}").into());
    }
}

// ── Custom JSON event formatter ───────────────────────────────────────────────

/// JSON log formatter.
///
/// Output shape per line:
/// ```json
/// {"time":"2026-03-10T12:00:00.000Z","level":"INFO","file_line":"src/main.rs:42","target":"rust_ucs_fe","message":"...","<extra_field>":"..."}
/// ```
pub struct JsonFileLine;

impl<S, N> FormatEvent<S, N> for JsonFileLine
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

        // Collect all fields (including "message")
        let mut collector = FieldCollector(Map::new());
        event.record(&mut collector);

        // "message" is promoted to a top-level field
        let message = collector
            .0
            .remove("message")
            .unwrap_or(Value::String(String::new()));

        // "file_line" = "src/foo.rs:42"
        let file_line = format!(
            "{}:{}",
            meta.file().unwrap_or("<unknown>"),
            meta.line().unwrap_or(0),
        );

        let now = chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);

        // Build the JSON object in a defined field order
        let mut entry: Map<String, Value> = Map::new();
        entry.insert("time".into(), now.into());
        entry.insert("level".into(), meta.level().to_string().into());
        entry.insert("file_line".into(), file_line.into());
        entry.insert("target".into(), meta.target().into());
        entry.insert("message".into(), message);
        // Remaining structured fields (span fields, etc.)
        entry.extend(collector.0);

        writeln!(writer, "{}", Value::Object(entry))
    }
}

// ── Public init ───────────────────────────────────────────────────────────────

/// Initialise the global tracing subscriber from `LogConfig`.
///
/// - `encoding = "json"` → compact JSON, one line per event,
///   `"file_line": "src/foo.rs:42"`.
/// - anything else → human-readable text with `file:line` prefix.
///
/// `RUST_LOG` environment variable overrides `cfg.level` when set.
pub fn init_tracing(cfg: &LogConfig) {
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&cfg.level));

    if cfg.encoding == "json" {
        tracing_subscriber::registry()
            .with(env_filter)
            .with(tracing_subscriber::fmt::layer().event_format(JsonFileLine))
            .init();
    } else {
        tracing_subscriber::registry()
            .with(env_filter)
            .with(
                tracing_subscriber::fmt::layer()
                    .with_file(true)
                    .with_line_number(true),
            )
            .init();
    }
}
