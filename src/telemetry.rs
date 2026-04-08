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
//!
//! ## Buffered I/O
//!
//! All log output flows through a shared `BufWriter<Stdout>` (default 30 MB)
//! to avoid per-line syscalls.  A dedicated background thread flushes the
//! buffer every N ms (default 10 ms, configurable via `bufferFlushInterval`).

use std::fmt;
use std::fmt::Write as FmtWrite;
use std::io::{self, BufWriter, Write};
use std::sync::{Arc, Mutex, OnceLock};

use chrono::SecondsFormat;
use serde_json::{Map, Value};
use tracing::{
    Event, Subscriber,
    field::{Field, Visit},
};
use tracing_subscriber::{
    EnvFilter, Layer,
    filter::filter_fn,
    fmt::{FmtContext, FormatEvent, FormatFields, MakeWriter, format},
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

// ── Buffered stdout writer ────────────────────────────────────────────────────

/// Global handle for flushing from `logs::flush()` / `logs::close()`.
static LOG_BUF: OnceLock<Arc<Mutex<BufWriter<io::Stdout>>>> = OnceLock::new();

/// Shared buffered stdout — passed to `fmt::Layer::with_writer`.
///
/// A single `BufWriter<Stdout>` is protected by a `Mutex`.
/// Each `make_writer` call returns a `BufGuard` (a `MutexGuard` wrapper)
/// so only one thread writes at a time, just like `io::Stdout`'s internal
/// lock, but batching many small writes into one large `write(2)` syscall.
#[derive(Clone)]
struct BufferedStdout(Arc<Mutex<BufWriter<io::Stdout>>>);

/// RAII guard returned by `BufferedStdout::make_writer`.
struct BufGuard<'a>(std::sync::MutexGuard<'a, BufWriter<io::Stdout>>);

impl<'a> io::Write for BufGuard<'a> {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }
    #[inline]
    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl<'a> MakeWriter<'a> for BufferedStdout {
    type Writer = BufGuard<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        BufGuard(self.0.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

/// Create the shared buffer and spawn the background flush thread.
fn init_buffered_writer(cfg: &LogConfig) -> BufferedStdout {
    // Guard against multiple initialization — subsequent calls reuse the
    // existing buffer without spawning additional flush threads.
    if let Some(existing) = LOG_BUF.get() {
        return BufferedStdout(existing.clone());
    }

    let cap_bytes = (cfg.buffer_size.max(1) as usize) * 1024 * 1024;
    let inner = Arc::new(Mutex::new(BufWriter::with_capacity(cap_bytes, io::stdout())));

    LOG_BUF.set(inner.clone()).ok();

    let flush_ms = cfg.buffer_flush_interval.max(1) as u64;
    let flush_interval = std::time::Duration::from_millis(flush_ms);
    let writer = inner.clone();
    std::thread::Builder::new()
        .name("log-flusher".into())
        .spawn(move || {
            loop {
                std::thread::sleep(flush_interval);
                // try_lock: if a writer currently holds the mutex, skip this flush
                // cycle rather than blocking all subsequent writers behind us.
                let mut guard = match writer.try_lock() {
                    Ok(g) => g,
                    Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
                    Err(std::sync::TryLockError::WouldBlock) => continue,
                };
                let _ = guard.flush();
                // guard is dropped here, releasing the lock immediately
            }
        })
        .expect("failed to spawn log flush thread");

    BufferedStdout(inner)
}

/// Flush the global log buffer immediately.
///
/// Safe to call from any thread; no-op before `init_tracing`.
pub fn flush_log_buf() {
    if let Some(buf) = LOG_BUF.get()
        && let Ok(mut w) = buf.lock()
    {
        let _ = w.flush();
    }
    if let Some(buf) = BEHAVIOR_BUF.get()
        && let Ok(mut w) = buf.lock()
    {
        let _ = w.flush();
    }
}

/// Best-effort flush that never blocks (uses `try_lock`).
///
/// Suitable for signal handlers and `atexit` where the mutex may already
/// be held by the thread that triggered the exit.
fn flush_log_buf_nonblocking() {
    if let Some(buf) = LOG_BUF.get()
        && let Ok(mut w) = buf.try_lock()
    {
        let _ = w.flush();
    }
    if let Some(buf) = BEHAVIOR_BUF.get()
        && let Ok(mut w) = buf.try_lock()
    {
        let _ = w.flush();
    }
}

// ── Behavior file writer ──────────────────────────────────────────────────────

/// Global handle for flushing the behavior log buffer.
static BEHAVIOR_BUF: OnceLock<Arc<Mutex<BufWriter<std::fs::File>>>> = OnceLock::new();

/// Buffered file writer for behavior logs (API request logs).
/// Mirrors Go's `getBehaviorFileWriter` + `lumberjack`.
#[derive(Clone)]
struct BehaviorFileWriter(Arc<Mutex<BufWriter<std::fs::File>>>);

struct BehaviorBufGuard<'a>(std::sync::MutexGuard<'a, BufWriter<std::fs::File>>);

impl<'a> io::Write for BehaviorBufGuard<'a> {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }
    #[inline]
    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl<'a> MakeWriter<'a> for BehaviorFileWriter {
    type Writer = BehaviorBufGuard<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        BehaviorBufGuard(self.0.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

/// Create the behavior file writer and background flush thread.
/// Returns `None` if behavior logging is not configured.
fn init_behavior_writer(cfg: &LogConfig) -> Option<BehaviorFileWriter> {
    // Guard against multiple initialization — subsequent calls reuse the
    // existing buffer without spawning additional flush threads.
    if let Some(existing) = BEHAVIOR_BUF.get() {
        return Some(BehaviorFileWriter(existing.clone()));
    }

    let behavior_dir = if !cfg.path_behavior.is_empty() {
        cfg.path_behavior.clone()
    } else if !cfg.path.is_empty() {
        format!("{}/behavior", cfg.path)
    } else {
        return None;
    };

    if let Err(e) = std::fs::create_dir_all(&behavior_dir) {
        eprintln!("Failed to create behavior log directory {}: {}", behavior_dir, e);
        return None;
    }

    let behavior_file_path = format!("{}/{}-behavior.log", behavior_dir, cfg.name);
    eprintln!("Log behavior file path: {}", behavior_file_path);

    let file = match std::fs::OpenOptions::new().create(true).append(true).open(&behavior_file_path)
    {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Failed to open behavior log file {}: {}", behavior_file_path, e);
            return None;
        }
    };

    let inner = Arc::new(Mutex::new(BufWriter::with_capacity(256 * 1024, file)));
    BEHAVIOR_BUF.set(inner.clone()).ok();

    let writer = inner.clone();
    std::thread::Builder::new()
        .name("behavior-log-flusher".into())
        .spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(1));
                // try_lock: if a writer currently holds the mutex, skip this flush
                // cycle rather than blocking all subsequent writers behind us.
                let mut guard = match writer.try_lock() {
                    Ok(g) => g,
                    Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
                    Err(std::sync::TryLockError::WouldBlock) => continue,
                };
                let _ = guard.flush();
                // guard is dropped here, releasing the lock immediately
            }
        })
        .expect("failed to spawn behavior log flush thread");

    Some(BehaviorFileWriter(inner))
}

// ── Exit hooks ────────────────────────────────────────────────────────────────

/// Install hooks that flush the log buffer on every possible exit path:
///
/// 1. **`atexit`** — called by `libc::exit()` which backs both
///    `std::process::exit()` and normal `main` returns.
/// 2. **Panic hook** — wraps the default hook so the buffer is flushed
///    *before* the panic message hits stderr.
///
/// Must be called once, right after `init_tracing`.
pub fn install_exit_hooks() {
    // ── C atexit ──────────────────────────────────────────────────────────
    unsafe extern "C" {
        safe fn atexit(cb: extern "C" fn()) -> std::os::raw::c_int;
    }
    extern "C" fn on_exit() {
        flush_log_buf_nonblocking();
    }
    atexit(on_exit);

    // ── Panic hook ────────────────────────────────────────────────────────
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Flush first so all tracing output preceding the panic is visible.
        flush_log_buf();
        // Then run the default hook (prints the panic message to stderr).
        default_hook(info);
    }));
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

        let mut collector = FieldCollector(Map::with_capacity(8));
        event.record(&mut collector);

        // Promote "message" to top-level.
        let message = collector.0.remove("message").unwrap_or(Value::String(String::new()));

        // "file_line" = "src/foo.rs:42" (same shape as Go's ShortCallerEncoder)
        let mut file_line = String::with_capacity(48);
        let _ = write!(
            &mut file_line,
            "{}:{}",
            meta.file().unwrap_or("<unknown>"),
            meta.line().unwrap_or(0),
        );

        let now = chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);

        let mut entry: Map<String, Value> = Map::with_capacity(5 + collector.0.len());
        entry.insert("time".into(), now.into());
        let level_str: &'static str = match *meta.level() {
            tracing::Level::ERROR => "ERROR",
            tracing::Level::WARN => "WARN",
            tracing::Level::INFO => "INFO",
            tracing::Level::DEBUG => "DEBUG",
            tracing::Level::TRACE => "TRACE",
        };
        entry.insert("level".into(), level_str.into());
        entry.insert("service".into(), service_name().into());
        entry.insert("file_line".into(), file_line.into());
        entry.insert("message".into(), message);
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
        Self {
            service_name: service_name.into(),
        }
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

        let mut collector = FieldCollector(Map::with_capacity(8));
        event.record(&mut collector);

        // Promote "message".
        let message = collector
            .0
            .remove("message")
            .and_then(|v| v.as_str().map(|s| s.to_owned()))
            .unwrap_or_default();

        // Only the base filename (mirrors Go's `path.Base(ent.Caller.File)`).
        let file = meta.file().map(|f| f.rsplit('/').next().unwrap_or(f)).unwrap_or("<unknown>");
        let line = meta.line().unwrap_or(0);

        // Timestamp in Go's default layout `2006-01-02 15:04:05.000`.
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");

        // Level is left-padded to 5 characters (matches `%-5s` in Go's fmt.Fprintf).
        let level: &'static str = match *meta.level() {
            tracing::Level::ERROR => "ERROR",
            tracing::Level::WARN => "WARN ",
            tracing::Level::INFO => "INFO ",
            tracing::Level::DEBUG => "DEBUG",
            tracing::Level::TRACE => "TRACE",
        };

        // Core line: [ts] [service] [LEVEL] [file:line] - message
        write!(
            writer,
            "[{}] [{}] [{}] [{}:{}] - {}",
            now, self.service_name, level, file, line, message
        )?;

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
    SERVICE_NAME.set(cfg.service_name.clone()).ok();

    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&cfg.level));

    let buf_writer = init_buffered_writer(cfg);
    let behavior_writer = init_behavior_writer(cfg);

    // Main stdout layer excludes behavior events (they go to file only).
    // Behavior file layer only captures target="behavior" events.
    if cfg.encoding == "json" {
        let main_layer = tracing_subscriber::fmt::layer()
            .event_format(JsonFormat)
            .with_writer(buf_writer)
            .with_filter(filter_fn(|meta| meta.target() != "behavior"));

        let behavior_layer = behavior_writer.map(|bw| {
            tracing_subscriber::fmt::layer()
                .event_format(TextFormat::new(&cfg.service_name))
                .with_writer(bw)
                .with_filter(filter_fn(|meta| meta.target() == "behavior"))
        });

        let _ = tracing_subscriber::registry()
            .with(env_filter)
            .with(main_layer)
            .with(behavior_layer)
            .try_init();
    } else {
        let main_layer = tracing_subscriber::fmt::layer()
            .event_format(TextFormat::new(&cfg.service_name))
            .with_writer(buf_writer)
            .with_filter(filter_fn(|meta| meta.target() != "behavior"));

        let behavior_layer = behavior_writer.map(|bw| {
            tracing_subscriber::fmt::layer()
                .event_format(TextFormat::new(&cfg.service_name))
                .with_writer(bw)
                .with_filter(filter_fn(|meta| meta.target() == "behavior"))
        });

        let _ = tracing_subscriber::registry()
            .with(env_filter)
            .with(main_layer)
            .with(behavior_layer)
            .try_init();
    }

    install_exit_hooks();
}

/// Shut down telemetry resources.
///
/// Mirrors Go's `Telemetry.Close()` → `tp.Shutdown(ctx)`.
/// Currently a no-op because the `opentelemetry` crate is not wired in;
/// when it is, call `opentelemetry::global::shutdown_tracer_provider()` here.
pub fn shutdown() {
    tracing::info!("telemetry shutdown");
}
