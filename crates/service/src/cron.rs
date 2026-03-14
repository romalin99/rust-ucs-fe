//! Background cron scheduler.
//!
//! Mirrors Go's `internal/service/cron.go` (`CommonCronJobs`).
//! Jobs are driven by `tokio::time::interval` rather than a cron library;
//! the interval duration comes from `config.Jobs[name].interval` (seconds).

use infra::config::JobConfig;
use std::{collections::HashMap, time::Duration};
use tokio::{sync::watch, task::JoinHandle};
use tracing::{info, warn};

/// A running background scheduler.
///
/// Drop it or call `shutdown()` to stop all background tasks.
pub struct CronScheduler {
    handles: Vec<JoinHandle<()>>,
}

impl CronScheduler {
    /// Register and immediately start all enabled jobs from `jobs`.
    pub fn start(jobs: &HashMap<String, JobConfig>) -> Self {
        let mut handles = Vec::new();

        for (name, cfg) in jobs {
            if !cfg.enabled {
                info!("[CRON] job '{}' is disabled — skipping", name);
                continue;
            }
            if cfg.interval == 0 {
                warn!("[CRON] job '{}' has interval=0 — skipping", name);
                continue;
            }

            let name = name.clone();
            let interval = Duration::from_secs(cfg.interval);
            let timeout = Duration::from_secs(cfg.timeout.max(1));

            let handle = tokio::spawn(async move {
                info!(
                    "[CRON] '{}' registered (interval={}s timeout={}s)",
                    name,
                    interval.as_secs(),
                    timeout.as_secs()
                );

                let mut ticker = tokio::time::interval(interval);
                ticker.tick().await; // consume the immediate first tick

                loop {
                    ticker.tick().await;

                    let job_name = name.clone();
                    let start = std::time::Instant::now();
                    info!("[CRON] '{}' started", job_name);

                    // Each execution gets its own timeout.
                    let result = tokio::time::timeout(timeout, run_job(&job_name)).await;

                    match result {
                        Ok(Ok(())) => {
                            info!("[CRON] '{}' completed in {:?}", job_name, start.elapsed());
                        }
                        Ok(Err(e)) => {
                            warn!("[CRON] '{}' failed: {}", job_name, e);
                        }
                        Err(_) => {
                            warn!("[CRON] '{}' timed out after {:?}", job_name, timeout);
                        }
                    }
                }
            });

            handles.push(handle);
        }

        CronScheduler { handles }
    }

    /// Abort all background job tasks.
    pub fn shutdown(self) {
        for h in self.handles {
            h.abort();
        }
        info!("[CRON] scheduler shut down");
    }

    /// Create a `watch::Sender` that will trigger a graceful stop when sent.
    /// Currently unused; shutdown is done via task abortion.
    pub fn stop_signal() -> (watch::Sender<()>, watch::Receiver<()>) {
        watch::channel(())
    }
}

// ── Individual job dispatch ───────────────────────────────────────────────────

/// Route a job name to its implementation.
/// Currently these are stub implementations matching Go's behaviour; extend
/// with real logic (e.g. calling `MerchantRuleRepo::find_all_as_map`) once
/// the scheduler is wired to the DI graph.
async fn run_job(name: &str) -> Result<(), String> {
    match name {
        "template_field_sync" => sync_template_fields().await,
        "field_configs_loading" => load_field_configs().await,
        other => Err(format!("unknown job: {}", other)),
    }
}

async fn sync_template_fields() -> Result<(), String> {
    info!("[CRON] template_field_sync: starting");
    tokio::time::sleep(Duration::from_secs(2)).await;
    info!("[CRON] template_field_sync: completed");
    Ok(())
}

async fn load_field_configs() -> Result<(), String> {
    info!("[CRON] field_configs_loading: starting");
    tokio::time::sleep(Duration::from_secs(1)).await;
    info!("[CRON] field_configs_loading: completed");
    Ok(())
}
