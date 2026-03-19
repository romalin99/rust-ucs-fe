/// Periodic memory/runtime statistics logging.
///
/// Mirrors Go's `pkg/memstatus/memstatus.go`.
///
/// Go uses `runtime.ReadMemStats` to log heap/GC stats; Rust exposes
/// equivalent information via the `jemalloc` or `sys_info` crates, but
/// since those are not available (Cargo.toml unchanged) we log the most
/// useful process-level metrics via `std::mem` and platform /proc files.
use std::time::Duration;

use tokio::sync::watch;
use tokio::time::interval;

/// Start the memstats background goroutine.
///
/// Logs memory and task stats every 60 seconds until the returned
/// [`StopHandle`] is dropped or [`StopHandle::stop`] is called.
///
/// Mirrors Go's `go memstatus.MemStats(ctx)`.
pub fn start_mem_stats() -> StopHandle {
    let (tx, rx) = watch::channel(false);

    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(60));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        ticker.tick().await; // skip immediate first tick

        let mut rx = rx;

        loop {
            tokio::select! {
                _ = ticker.tick() => log_stats(),
                _ = rx.changed() => {
                    tracing::info!("[memstats] stopping");
                    break;
                }
            }
        }
    });

    StopHandle(tx)
}

/// Opaque handle returned by [`start_mem_stats`].
/// Drop it (or call [`StopHandle::stop`]) to stop the background task.
pub struct StopHandle(watch::Sender<bool>);

impl StopHandle {
    pub fn stop(self) {
        let _ = self.0.send(true);
    }
}

fn log_stats() {
    // ── tokio runtime metrics ─────────────────────────────────────────────────
    // `metrics()` is behind the `tokio_unstable` feature and not available
    // in stable builds; always report 0 for portability.
    let num_tasks: usize = 0;

    // ── process memory via /proc/self/status (Linux) or estimate ─────────────
    let (rss_mb, virt_mb) = read_proc_mem().unwrap_or((0.0, 0.0));

    tracing::warn!(
        rss_mb    = rss_mb,
        virt_mb   = virt_mb,
        num_tasks = num_tasks,
        "[memstats] process memory snapshot"
    );
}

/// Read VmRSS and VmSize from `/proc/self/status` on Linux.
/// Returns `None` on other platforms or if parsing fails.
fn read_proc_mem() -> Option<(f64, f64)> {
    #[cfg(target_os = "linux")]
    {
        let content = std::fs::read_to_string("/proc/self/status").ok()?;
        let mut rss  = 0u64;
        let mut virt = 0u64;
        for line in content.lines() {
            if let Some(kb) = line.strip_prefix("VmRSS:") {
                rss = kb.trim().split_whitespace().next()?.parse().ok()?;
            } else if let Some(kb) = line.strip_prefix("VmSize:") {
                virt = kb.trim().split_whitespace().next()?.parse().ok()?;
            }
        }
        Some((rss as f64 / 1024.0, virt as f64 / 1024.0))
    }

    #[cfg(not(target_os = "linux"))]
    None
}
