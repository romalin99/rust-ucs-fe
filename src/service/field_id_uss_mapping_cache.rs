/// In-memory `FIELD_ID` <-> `USS_ID` mapping cache.
///
/// Mirrors Go's `internal/service/field_id_uss_mapping_cache.go`.
///
/// Cache key format: `"{FIELD_ID}:{FIELD_NAME}"` -> `USS_ID` string.
/// Example: `"GENDER:Male"` -> `"1"`, `"ID_TYPE:Passport"` -> `"1"`.
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use anyhow::Result;
use dashmap::DashMap;
use once_cell::sync::OnceCell;
use tokio::sync::Notify;

use crate::repository::FieldIdUssMappingRepository;

// ── Global cache ─────────────────────────────────────────────────────────────

/// Global USS mapping cache: `"{FIELD_ID}:{FIELD_NAME}"` -> `USS_ID` string.
pub static GLOBAL_USS_MAPPING_CONFIGS: OnceCell<DashMap<String, String>> = OnceCell::new();

fn global_uss_mappings() -> &'static DashMap<String, String> {
    GLOBAL_USS_MAPPING_CONFIGS.get_or_init(DashMap::new)
}

// ── Init barrier (mirrors Go's sync.WaitGroup) ─────────────────────────────

static USS_MAPPING_INIT_COMPLETE: AtomicBool = AtomicBool::new(false);
static USS_MAPPING_INIT_NOTIFY: OnceCell<Arc<Notify>> = OnceCell::new();

fn uss_mapping_init_notify() -> Arc<Notify> {
    USS_MAPPING_INIT_NOTIFY.get_or_init(|| Arc::new(Notify::new())).clone()
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
        let key = build_uss_mapping_key(&item.field_id, &item.field_name);
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

// ── FieldIdUssMappingLoader (startup loader only — periodic refresh via CommonCronJobs) ─

pub struct FieldIdUssMappingLoader {
    _guard: (),
}

impl FieldIdUssMappingLoader {
    /// Starts async initial load. Periodic refresh is handled by `CommonCronJobs`,
    /// matching Go where the internal periodic loop is unused and cron drives refreshes.
    #[allow(clippy::needless_pass_by_value)]
    pub fn start(repo: Arc<FieldIdUssMappingRepository>) -> Self {
        let repo_clone = repo;
        tokio::spawn(async move {
            tracing::info!("Starting async initialisation of USS mapping configs...");

            if let Err(e) = do_load_uss_mapping_configs(&repo_clone).await {
                tracing::error!(error = %e, "Initial USS mapping config load failed");
                USS_MAPPING_INIT_COMPLETE.store(true, Ordering::Release);
                uss_mapping_init_notify().notify_waiters();
            }
        });

        Self { _guard: () }
    }

    #[allow(clippy::unused_self)]
    pub fn stop(&self) {
        tracing::info!("USS mapping loader stopped (refresh handled by cron)");
    }
}

// ── Public accessors ─────────────────────────────────────────────────────────

/// Build cache key for internal loading: `"{FIELD_ID}:{FIELD_NAME}"`.
/// Mirrors Go's `buildUssMappingKey`(`fieldID`, `fieldName` string).
fn build_uss_mapping_key(field_id: &str, field_name: &str) -> String {
    format!("{field_id}:{field_name}")
}

/// Build cache key from string `field_id` and string `field_name` (used in scoring).
/// Mirrors Go's `buildFieldIdUssIdMappingKey`(`fieldID`, `fieldName` string).
pub fn build_field_id_uss_id_mapping_key(field_id: &str, field_name: &str) -> String {
    format!("{field_id}:{field_name}")
}

/// Set a single USS mapping entry. Mirrors Go's `SetUssMappingConfig`.
pub fn set_uss_mapping_config(field_id: &str, field_name: &str, uss_id: i32) {
    let key = build_uss_mapping_key(field_id, field_name);
    global_uss_mappings().insert(key, uss_id.to_string());
}

/// Look up a `USS_ID` string by `field_id` + `field_name`.
/// Waits for the initial load to complete before reading.
/// Mirrors Go's `GetUssMappingConfig`.
pub async fn get_uss_mapping_config(field_id: &str, field_name: &str) -> Option<String> {
    wait_for_uss_mapping_init().await;
    let key = build_field_id_uss_id_mapping_key(field_id, field_name);
    global_uss_mappings().get(&key).map(|v| v.value().clone())
}

/// Synchronous lookup (no init wait) for use in hot paths where init is guaranteed.
pub fn get_uss_mapping_config_sync(field_id: &str, field_name: &str) -> Option<String> {
    let key = build_field_id_uss_id_mapping_key(field_id, field_name);
    global_uss_mappings().get(&key).map(|v| v.value().clone())
}

/// Return a snapshot of all USS mapping configs.
/// Mirrors Go's `GetAllUssMappingConfigs`.
pub async fn get_all_uss_mapping_configs() -> HashMap<String, String> {
    wait_for_uss_mapping_init().await;
    let cache = global_uss_mappings();
    cache.iter().map(|entry| (entry.key().clone(), entry.value().clone())).collect()
}
