/// In-memory field-config cache.
///
/// Mirrors Go's `internal/service/field_cache.go`.
/// Uses `DashMap` (concurrent hashmap) as a direct equivalent of `sync.Map`.
/// `InitLoadingData` loads configs on startup and starts a periodic refresh.
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use dashmap::DashMap;
use once_cell::sync::OnceCell;
use tokio::sync::Notify;

use crate::model::template::DropdownItem;
use crate::repository::MerchantRuleRepository;

/// Global field-config cache: merchantCode → fieldId → Vec<DropdownItem>.
pub static GLOBAL_FIELD_CONFIGS: OnceCell<
    Arc<DashMap<String, HashMap<String, Vec<DropdownItem>>>>,
> = OnceCell::new();

/// Get a reference to the global map (initialises an empty one if not set).
pub fn global_configs() -> Arc<DashMap<String, HashMap<String, Vec<DropdownItem>>>> {
    GLOBAL_FIELD_CONFIGS
        .get_or_init(|| Arc::new(DashMap::new()))
        .clone()
}

/// Signal that the initial load has completed (used by `get_field_config` to wait).
static INIT_DONE: OnceCell<Arc<Notify>> = OnceCell::new();

fn init_notify() -> Arc<Notify> {
    INIT_DONE.get_or_init(|| Arc::new(Notify::new())).clone()
}

// ── Core load function (shared by startup + cron) ────────────────────────────

/// Fetch all merchant field configs from Oracle and refresh the cache.
pub async fn do_load_field_configs(repo: &MerchantRuleRepository) -> Result<()> {
    let start = Instant::now();
    tracing::info!("doLoadFieldConfigs started");

    let merchant_map = repo.find_all_as_map().await?;
    let total = merchant_map.len();

    let cache = global_configs();
    for (i, (merchant_code, dd_map)) in merchant_map.into_iter().enumerate() {
        if i < 10 {
            tracing::info!(i, merchant_code = %merchant_code, items = dd_map.len(), "Loaded field config entry");
        }
        cache.insert(merchant_code, dd_map);
    }

    tracing::info!(
        total,
        elapsed_ms = start.elapsed().as_millis(),
        "✅ template fields configs loaded"
    );
    Ok(())
}

// ── InitLoadingData (startup loader + periodic refresh) ──────────────────────

pub struct InitLoadingData {
    stop_tx: tokio::sync::watch::Sender<bool>,
}

impl InitLoadingData {
    /// Starts async initial load and background 30-minute refresh.
    pub fn start(repo: Arc<MerchantRuleRepository>) -> Self {
        // Ensure the global cache is initialised.
        global_configs();

        let (stop_tx, mut stop_rx) = tokio::sync::watch::channel(false);
        let notify = init_notify();

        let repo_clone = repo.clone();
        let notify_clone = notify.clone();

        tokio::spawn(async move {
            tracing::info!("Starting async initialisation of field configs...");
            if let Err(e) = do_load_field_configs(&repo_clone).await {
                tracing::error!(error = %e, "Initial field config load failed");
            }
            notify_clone.notify_waiters(); // unblock any waiting GetFieldConfig calls

            // Periodic refresh every 30 minutes.
            let mut interval = tokio::time::interval(Duration::from_secs(30 * 60));
            interval.tick().await; // consume immediate first tick

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        tracing::info!("Starting scheduled refresh of field configs...");
                        if let Err(e) = do_load_field_configs(&repo_clone).await {
                            tracing::warn!(error = %e, "Scheduled field config refresh failed");
                        }
                    }
                    _ = stop_rx.changed() => {
                        tracing::info!("Field config refresh goroutine exited");
                        break;
                    }
                }
            }
        });

        Self { stop_tx }
    }

    pub fn stop(&self) {
        let _ = self.stop_tx.send(true);
    }
}

// ── Public accessors ─────────────────────────────────────────────────────────

/// Get a cloned copy of the dropdown-item map for a merchant.
pub fn get_field_config(merchant_code: &str) -> Option<HashMap<String, Vec<DropdownItem>>> {
    global_configs().get(merchant_code).map(|v| v.clone())
}

pub fn set_field_config(key: String, value: HashMap<String, Vec<DropdownItem>>) {
    global_configs().insert(key, value);
}
