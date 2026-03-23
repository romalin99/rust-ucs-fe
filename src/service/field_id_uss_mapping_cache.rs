/// In-memory FIELD_ID <-> USS_ID mapping cache.
///
/// Mirrors Go's `internal/service/field_id_uss_mapping_cache.go`.
///
/// Cache key format: `"{FIELD_ID}:{MCS_ID}"` -> USS_ID string.
/// Example: `"GENDER:1" -> "1"`, `"STATE:501" -> "1"`, `"ID_TYPE:4239" -> "31"`.
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use dashmap::DashMap;
use once_cell::sync::OnceCell;
use tokio::sync::Notify;

use crate::repository::FieldIdUssMappingRepository;

// ── Global cache ─────────────────────────────────────────────────────────────

/// Global USS mapping cache: `"{FIELD_ID}:{MCS_ID}"` -> USS_ID string.
pub static GLOBAL_USS_MAPPING_CONFIGS: OnceCell<Arc<DashMap<String, String>>> = OnceCell::new();

fn global_uss_mappings() -> Arc<DashMap<String, String>> {
    GLOBAL_USS_MAPPING_CONFIGS
        .get_or_init(|| Arc::new(DashMap::new()))
        .clone()
}

// ── Init barrier (mirrors Go's sync.WaitGroup) ─────────────────────────────

static USS_MAPPING_INIT_COMPLETE: AtomicBool = AtomicBool::new(false);
static USS_MAPPING_INIT_NOTIFY: OnceCell<Arc<Notify>> = OnceCell::new();

fn uss_mapping_init_notify() -> Arc<Notify> {
    USS_MAPPING_INIT_NOTIFY
        .get_or_init(|| Arc::new(Notify::new()))
        .clone()
}

pub async fn wait_for_uss_mapping_init() {
    if USS_MAPPING_INIT_COMPLETE.load(Ordering::Acquire) {
        return;
    }

    let notify = uss_mapping_init_notify();
    let notified = notify.notified();
    tokio::pin!(notified);
    notified.as_mut().enable();

    if USS_MAPPING_INIT_COMPLETE.load(Ordering::Acquire) {
        return;
    }

    notified.await;
}

// ── Core load function (shared by startup + cron) ────────────────────────────

/// Fetch all USS mapping records from Oracle and refresh the global cache.
///
/// Mirrors Go's `doLoadFieldIdUssMappingConfigs`.
pub async fn do_load_uss_mapping_configs(repo: &FieldIdUssMappingRepository) -> Result<()> {
    let start = Instant::now();
    tracing::info!("doLoadUssMappingConfigs started");

    let list = repo.find_all_mappings().await?;
    let total = list.len();

    let cache = global_uss_mappings();

    // Collect fresh keys to purge stale entries afterwards.
    let mut fresh_keys = HashSet::with_capacity(total);

    for (i, item) in list.iter().enumerate() {
        let key = build_uss_mapping_key(&item.field_id, item.mcs_id);
        let uss_id_str = item.uss_id.to_string();

        fresh_keys.insert(key.clone());
        cache.insert(key.clone(), uss_id_str.clone());

        if i < 10 {
            tracing::info!(i, key = %key, uss_id = %uss_id_str, "Loaded USS mapping entry");
        }
    }

    // Purge stale entries no longer present in DB.
    let stale_keys: Vec<String> = cache
        .iter()
        .filter(|entry| !fresh_keys.contains(entry.key()))
        .map(|entry| entry.key().clone())
        .collect();

    for key in stale_keys {
        cache.remove(&key);
        tracing::info!(key = %key, "Purged stale USS mapping config");
    }

    tracing::info!(
        total,
        elapsed_ms = start.elapsed().as_millis(),
        "✅ field id uss id mapping configs loaded"
    );

    USS_MAPPING_INIT_COMPLETE.store(true, Ordering::Release);
    uss_mapping_init_notify().notify_waiters();

    Ok(())
}

// ── FieldIdUssMappingLoader (startup loader + periodic refresh) ──────────────

pub struct FieldIdUssMappingLoader {
    stop_tx: tokio::sync::watch::Sender<bool>,
}

impl FieldIdUssMappingLoader {
    /// Starts async initial load and background 30-minute refresh.
    ///
    /// Mirrors Go's `NewFieldIdUssMappingLoader(cm)`.
    pub fn start(repo: Arc<FieldIdUssMappingRepository>) -> Self {
        global_uss_mappings();

        let (stop_tx, mut stop_rx) = tokio::sync::watch::channel(false);
        let repo_clone = repo.clone();

        tokio::spawn(async move {
            tracing::info!("Starting async initialisation of USS mapping configs...");

            if let Err(e) = do_load_uss_mapping_configs(&repo_clone).await {
                tracing::error!(error = %e, "Initial USS mapping config load failed");
                USS_MAPPING_INIT_COMPLETE.store(true, Ordering::Release);
                uss_mapping_init_notify().notify_waiters();
            }

            let mut interval = tokio::time::interval(Duration::from_secs(30 * 60));
            interval.tick().await; // consume the immediate first tick

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        tracing::info!("Starting scheduled refresh of USS mapping configs...");
                        if let Err(e) = do_load_uss_mapping_configs(&repo_clone).await {
                            tracing::warn!(error = %e, "Scheduled USS mapping config refresh failed");
                        }
                    }
                    _ = stop_rx.changed() => {
                        tracing::info!("USS mapping config refresh task exited");
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

fn build_uss_mapping_key(field_id: &str, mcs_id: i64) -> String {
    format!("{}:{}", field_id, mcs_id)
}

/// Build cache key from string field_id and string mcs_id (used in scoring).
/// Mirrors Go's `buildFieldIdUssIdMappingKey(fieldID, mcsID string)`.
pub fn build_field_id_uss_id_mapping_key(field_id: &str, mcs_id: &str) -> String {
    format!("{}:{}", field_id, mcs_id)
}

/// Look up a USS_ID string by field_id + mcs_id.
/// Waits for the initial load to complete before reading.
/// Mirrors Go's `GlobalUssMappingConfigs.Load(key)`.
pub async fn get_uss_mapping_config(field_id: &str, mcs_id: &str) -> Option<String> {
    wait_for_uss_mapping_init().await;
    let key = build_field_id_uss_id_mapping_key(field_id, mcs_id);
    global_uss_mappings().get(&key).map(|v| v.clone())
}

/// Synchronous lookup (no init wait) for use in hot paths where init is guaranteed.
pub fn get_uss_mapping_config_sync(field_id: &str, mcs_id: &str) -> Option<String> {
    let key = build_field_id_uss_id_mapping_key(field_id, mcs_id);
    global_uss_mappings().get(&key).map(|v| v.clone())
}
