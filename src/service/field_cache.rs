/// In-memory field-config cache.
///
/// Mirrors Go's `internal/service/field_cache.go`.
///
/// Key difference vs Go:
///   Go uses `sync.WaitGroup.Wait()` inside `GetFieldConfig` to block callers
///   until the initial DB load finishes.
///   Rust uses `AtomicBool` + `tokio::sync::Notify` to replicate the same
///   "wait-until-loaded" contract in an async context.
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use anyhow::Result;
use dashmap::DashMap;
use once_cell::sync::OnceCell;
use tokio::sync::Notify;

use crate::model::template::DropdownItem;
use crate::repository::MerchantRuleRepository;

// ── Global cache ─────────────────────────────────────────────────────────────

/// Dropdown map for a single merchant (wrapped in Arc to avoid deep cloning on read).
pub type MerchantDropdownMap = Arc<HashMap<String, Vec<DropdownItem>>>;

/// Global field-config cache: merchantCode → Arc<fieldId → Vec<DropdownItem>>.
/// Using `Arc<HashMap>` as values means `get_field_config` returns a cheap
/// `Arc::clone` (~1 ns) instead of deep-cloning the entire map (~µs).
pub static GLOBAL_FIELD_CONFIGS: OnceCell<DashMap<String, MerchantDropdownMap>> = OnceCell::new();

/// Get (or lazily create) the global map.
fn global_configs() -> &'static DashMap<String, MerchantDropdownMap> {
    GLOBAL_FIELD_CONFIGS.get_or_init(DashMap::new)
}

// ── Init barrier (mirrors Go's sync.WaitGroup) ────────────────────────────────
//
// Pattern:
//   1. INIT_COMPLETE starts as `false`.
//   2. After the initial DB load, set it to `true` then call `notify_waiters()`.
//   3. Any caller of `wait_for_init()` that arrives after the flag is set
//      returns immediately.  Any caller that arrives before the flag is set
//      arms the `Notified` future *before* the second atomic check to avoid
//      the TOCTOU race, then awaits if still not done.

static INIT_COMPLETE: AtomicBool = AtomicBool::new(false);
static INIT_NOTIFY: OnceCell<Arc<Notify>> = OnceCell::new();

fn init_notify() -> Arc<Notify> {
    INIT_NOTIFY.get_or_init(|| Arc::new(Notify::new())).clone()
}

/// Block asynchronously until the initial field-config load has completed.
///
/// Mirrors Go's `initWg.Wait()` inside `GetFieldConfig`.
/// Returns immediately if loading is already done.
pub async fn wait_for_init() {
    // Fast path — already done.
    if INIT_COMPLETE.load(Ordering::Acquire) {
        return;
    }

    // Arm the Notified future *before* the second check so we cannot miss a
    // `notify_waiters()` that fires between the first load and the await.
    let notify = init_notify();
    let notified = notify.notified();
    tokio::pin!(notified);
    notified.as_mut().enable(); // registers interest before we re-check

    // Re-check: if it completed while we were arming, skip the wait.
    if INIT_COMPLETE.load(Ordering::Acquire) {
        return;
    }

    notified.await;
}

// ── Core load function (shared by startup + cron) ────────────────────────────

/// Fetch all merchant field configs from Oracle and refresh the cache.
///
/// Mirrors Go's `doLoadFieldConfigs`.
pub async fn do_load_field_configs(repo: &MerchantRuleRepository) -> Result<()> {
    let start = Instant::now();
    tracing::info!("doLoadFieldConfigs started");

    let merchant_map = repo.find_all_as_map().await?;
    let total = merchant_map.len();

    let cache = global_configs();

    // Collect fresh keys so we can purge stale entries afterwards (mirrors Go).
    let mut fresh_keys = std::collections::HashSet::with_capacity(total);

    for (i, (merchant_code, dd_map)) in merchant_map.into_iter().enumerate() {
        if i < 10 {
            tracing::info!(
                i,
                merchant_code = %merchant_code,
                items = dd_map.len(),
                "Loaded field config entry"
            );
        }
        fresh_keys.insert(merchant_code.clone());
        cache.insert(merchant_code, Arc::new(dd_map));
    }

    // Purge merchants no longer present in DB (mirrors Go's Range + Delete).
    let stale_keys: Vec<String> = cache
        .iter()
        .filter(|entry| !fresh_keys.contains(entry.key()))
        .map(|entry| entry.key().clone())
        .collect();

    for key in &stale_keys {
        cache.remove(key);
        tracing::info!(merchant_code = %key, "Purged stale field config");
    }

    tracing::info!(
        total,
        purged = stale_keys.len(),
        elapsed_ms = start.elapsed().as_millis(),
        "✅ template fields configs loaded"
    );

    // Signal that initial load is complete — unblocks all `wait_for_init()` callers.
    INIT_COMPLETE.store(true, Ordering::Release);
    init_notify().notify_waiters();

    Ok(())
}

// ── InitLoadingData (startup loader only — periodic refresh via CommonCronJobs) ─

pub struct InitLoadingData {
    _guard: (),
}

impl InitLoadingData {
    /// Starts async initial load. Periodic refresh is handled by `CommonCronJobs`,
    /// matching Go where `startPeriodicLoad` is unused and cron drives refreshes.
    #[allow(clippy::needless_pass_by_value)]
    pub fn start(repo: Arc<MerchantRuleRepository>) -> Self {
        let repo_clone = repo;
        tokio::spawn(async move {
            tracing::info!("Starting async initialisation of field configs...");

            if let Err(e) = do_load_field_configs(&repo_clone).await {
                tracing::error!(error = %e, "Initial field config load failed");
                INIT_COMPLETE.store(true, Ordering::Release);
                init_notify().notify_waiters();
            }
        });

        Self { _guard: () }
    }

    #[allow(clippy::unused_self)]
    pub fn stop(&self) {
        tracing::info!("Field config loader stopped (refresh handled by cron)");
    }
}

// ── Public accessors ─────────────────────────────────────────────────────────

/// Get a cloned copy of the dropdown-item map for a merchant.
///
/// **Waits** for the initial DB load to complete before reading the cache —
/// mirrors Go's `GetFieldConfig` which calls `initWg.Wait()` internally.
pub async fn get_field_config(merchant_code: &str) -> Option<MerchantDropdownMap> {
    wait_for_init().await;
    global_configs().get(merchant_code).map(|v| Arc::clone(v.value()))
}

pub fn set_field_config(key: String, value: HashMap<String, Vec<DropdownItem>>) {
    global_configs().insert(key, Arc::new(value));
}

/// Return a snapshot of all merchant → dropdown configs.
///
/// Mirrors Go's `GetAllFieldConfigs()` which calls `initWg.Wait()` then
/// iterates `GlobalFieldConfigs` (sync.Map) to build the full map.
pub async fn get_all_field_configs() -> HashMap<String, MerchantDropdownMap> {
    wait_for_init().await;
    let cache = global_configs();
    cache
        .iter()
        .map(|entry| (entry.key().clone(), Arc::clone(entry.value())))
        .collect()
}
