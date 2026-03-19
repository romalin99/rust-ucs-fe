/// Flight recorder / observability utilities.
///
/// Mirrors Go's `internal/observability/recorder.go`.
///
/// Go uses `runtime/trace.FlightRecorder` with SIGUSR1/SIGUSR2 signal
/// handling to dump in-memory traces to `/tmp/traces/flight_trace.out`.
///
/// Rust's `tokio-console` or `tracing` subscriber handles diagnostics,
/// but we implement the same signal-based dump concept using `tokio::signal`
/// and the `tracing` framework:
///   - SIGUSR1 → dump current runtime diagnostics to a file.
///   - SIGUSR2 → same as SIGUSR1.
///
/// On macOS/Linux SIGUSR1 is available; on Windows this is a no-op.
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;

const DEFAULT_OUTPUT_DIR: &str = "/tmp/traces";
const TRACE_FILE_NAME:    &str = "flight_trace.out";

// ── Config ────────────────────────────────────────────────────────────────────

/// Configuration for the [`FlightRecorder`].
///
/// Mirrors Go's `FlightRecorderConfig`.
pub struct FlightRecorderConfig {
    /// Directory where trace files are written.  Default: `/tmp/traces`.
    pub output_dir: String,
    /// Retention window (not used in Rust but kept for API parity).
    pub min_age:    Duration,
}

impl Default for FlightRecorderConfig {
    fn default() -> Self {
        Self {
            output_dir: DEFAULT_OUTPUT_DIR.to_string(),
            min_age:    Duration::from_secs(600),
        }
    }
}

// ── FlightRecorder ────────────────────────────────────────────────────────────

/// Listens for SIGUSR1 / SIGUSR2 and dumps diagnostic info to a file.
///
/// Mirrors Go's `observability.FlightRecorder`.
pub struct FlightRecorder {
    stop_tx: watch::Sender<bool>,
    cfg:     Arc<FlightRecorderConfig>,
}

impl FlightRecorder {
    /// Create and start a new `FlightRecorder` with default config.
    ///
    /// Mirrors Go's `observability.NewFlightRecorder()`.
    pub fn new() -> anyhow::Result<Self> {
        Self::with_config(FlightRecorderConfig::default())
    }

    /// Create and start a new `FlightRecorder` with custom config.
    ///
    /// Mirrors Go's `observability.NewFlightRecorderWithConfig(cfg)`.
    pub fn with_config(cfg: FlightRecorderConfig) -> anyhow::Result<Self> {
        std::fs::create_dir_all(&cfg.output_dir).map_err(|e| {
            anyhow::anyhow!("FlightRecorder: failed to create output dir {}: {e}", cfg.output_dir)
        })?;

        let cfg = Arc::new(cfg);
        let (stop_tx, stop_rx) = watch::channel(false);

        let cfg_clone = cfg.clone();
        tokio::spawn(async move {
            listen_signals(cfg_clone, stop_rx).await;
        });

        Ok(Self { stop_tx, cfg })
    }

    /// Manually trigger a trace dump without waiting for a signal.
    ///
    /// Mirrors Go's `FlightRecorder.DumpNow()`.
    pub fn dump_now(&self) -> anyhow::Result<()> {
        dump_trace(&self.cfg.output_dir)
    }

    /// Stop the signal listener.
    ///
    /// Mirrors Go's `FlightRecorder.Stop()`.
    pub fn stop(self) {
        let _ = self.stop_tx.send(true);
        tracing::info!("[FlightRecorder] stopped");
    }

    fn trace_path(&self) -> PathBuf {
        Path::new(&self.cfg.output_dir).join(TRACE_FILE_NAME)
    }
}

// ── Signal listener ───────────────────────────────────────────────────────────

async fn listen_signals(cfg: Arc<FlightRecorderConfig>, mut stop_rx: watch::Receiver<bool>) {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};

        let mut usr1 = match signal(SignalKind::user_defined1()) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("[FlightRecorder] failed to register SIGUSR1: {e}");
                return;
            }
        };
        let mut usr2 = match signal(SignalKind::user_defined2()) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("[FlightRecorder] failed to register SIGUSR2: {e}");
                return;
            }
        };

        loop {
            tokio::select! {
                Some(()) = usr1.recv() => {
                    tracing::info!("[FlightRecorder] SIGUSR1 received — dumping trace");
                    if let Err(e) = dump_trace(&cfg.output_dir) {
                        tracing::error!("[FlightRecorder] dump failed: {e}");
                    }
                }
                Some(()) = usr2.recv() => {
                    tracing::info!("[FlightRecorder] SIGUSR2 received — dumping trace");
                    if let Err(e) = dump_trace(&cfg.output_dir) {
                        tracing::error!("[FlightRecorder] dump failed: {e}");
                    }
                }
                _ = stop_rx.changed() => {
                    tracing::info!("[FlightRecorder] signal listener stopped");
                    break;
                }
            }
        }
    }

    #[cfg(not(unix))]
    {
        // On non-Unix platforms (e.g. Windows) just wait for stop signal.
        let _ = stop_rx.changed().await;
    }
}

// ── Dump helper ───────────────────────────────────────────────────────────────

/// Write runtime diagnostic information to the fixed trace file path.
///
/// Mirrors Go's `FlightRecorder.dumpTrace()` which writes the in-memory
/// `runtime/trace.FlightRecorder` buffer to a file.
///
/// Rust writes a JSON diagnostic snapshot instead (no flight recorder
/// equivalent exists without unstable tokio-console).
fn dump_trace(output_dir: &str) -> anyhow::Result<()> {
    use std::io::Write;

    let path = Path::new(output_dir).join(TRACE_FILE_NAME);
    tracing::info!("[FlightRecorder] dumping diagnostic snapshot to {}", path.display());

    let ts  = chrono::Utc::now().to_rfc3339();
    let pid = std::process::id();

    let content = serde_json::json!({
        "timestamp": ts,
        "pid":       pid,
        "note":      "Rust runtime/trace flight recorder snapshot — use SIGUSR1/SIGUSR2 to trigger"
    });

    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&path)
        .map_err(|e| anyhow::anyhow!("FlightRecorder: open {}: {e}", path.display()))?;

    writeln!(f, "{}", serde_json::to_string_pretty(&content)?)?;
    tracing::info!("[FlightRecorder] diagnostic snapshot written to {}", path.display());
    Ok(())
}
