/// Background cron jobs.
///
/// Mirrors Go's `internal/service/cron.go`.
/// Uses `tokio::time::interval` for simple interval-based jobs (no external cron crate needed).
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::config::JobConfig;
use crate::repository::{FieldIdUssMappingRepository, MerchantRuleRepository};
use crate::service::field_cache::do_load_field_configs;
use crate::service::field_id_uss_mapping_cache::do_load_uss_mapping_configs;

pub struct CommonCronJobs {
    stop_handles: Vec<tokio::sync::watch::Sender<bool>>,
}

/// Shared repo references passed into each cron job.
struct CronRepos {
    merchant_repo: Arc<MerchantRuleRepository>,
    uss_mapping_repo: Arc<FieldIdUssMappingRepository>,
}

impl CommonCronJobs {
    /// Register and start all enabled jobs from config.
    pub fn start(
        jobs: &HashMap<String, JobConfig>,
        merchant_repo: Arc<MerchantRuleRepository>,
        uss_mapping_repo: Arc<FieldIdUssMappingRepository>,
    ) -> Self {
        let mut stop_handles = Vec::new();
        let repos = Arc::new(CronRepos {
            merchant_repo,
            uss_mapping_repo,
        });

        for (name, cfg) in jobs {
            if !cfg.enabled {
                tracing::info!(job = %name, "cron job disabled, skipping");
                continue;
            }
            if cfg.interval == 0 {
                tracing::warn!(job = %name, "cron job has interval=0, skipping");
                continue;
            }

            let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
            stop_handles.push(stop_tx);

            let name = name.clone();
            let interval_secs = cfg.interval;
            let timeout_secs = cfg.timeout;
            let repos = repos.clone();

            let running = Arc::new(AtomicBool::new(false));

            tokio::spawn(async move {
                let mut ticker = tokio::time::interval(Duration::from_secs(interval_secs));
                ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                ticker.tick().await; // skip initial immediate tick

                let mut rx = stop_rx;

                tracing::info!(job = %name, interval = interval_secs, timeout = timeout_secs, "cron job registered");

                loop {
                    tokio::select! {
                        _ = ticker.tick() => {
                            if running.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err() {
                                tracing::warn!(job = %name, "CRON job still running, skipping this tick (singleton mode)");
                                continue;
                            }
                            let guard = running.clone();

                            let start = std::time::Instant::now();
                            tracing::info!(job = %name, interval = interval_secs, "CRON job started");

                            let job_future = execute_job(&name, &repos);
                            let result = tokio::time::timeout(
                                Duration::from_secs(timeout_secs),
                                job_future,
                            )
                            .await;

                            guard.store(false, Ordering::Release);

                            match result {
                                Ok(Ok(())) => tracing::info!(
                                    job = %name,
                                    elapsed_ms = start.elapsed().as_millis(),
                                    "CRON job completed"
                                ),
                                Ok(Err(e)) => tracing::error!(job = %name, error = %e, "CRON job failed"),
                                Err(_)     => tracing::error!(job = %name, "CRON job timed out"),
                            }
                        }
                        _ = rx.changed() => {
                            tracing::info!(job = %name, "cron job stopped");
                            break;
                        }
                    }
                }
            });
        }

        let job_count = stop_handles.len();
        tracing::info!("──── Registered Cron Jobs ({}) ────", job_count);
        for (name, cfg) in jobs {
            if cfg.enabled && cfg.interval > 0 {
                tracing::info!(
                    job = %name,
                    interval_secs = cfg.interval,
                    timeout_secs = cfg.timeout,
                    "  ✓ registered"
                );
            }
        }
        tracing::info!("──── End Registered Cron Jobs ────");

        Self { stop_handles }
    }

    pub fn stop_all(&self) {
        for tx in &self.stop_handles {
            let _ = tx.send(true);
        }
    }
}

async fn execute_job(name: &str, repos: &CronRepos) -> anyhow::Result<()> {
    match name {
        "template_field_load" => {
            tracing::info!(job = name, "starting to sync template fields");
            do_load_field_configs(&repos.merchant_repo).await?;
        }
        "fieldid_ussid_mapping_load" => {
            tracing::info!(job = name, "starting to sync USS mapping configs");
            do_load_uss_mapping_configs(&repos.uss_mapping_repo).await?;
        }
        other => anyhow::bail!("Unknown job: {other}"),
    }
    Ok(())
}
