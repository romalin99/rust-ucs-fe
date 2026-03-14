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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use dashmap::DashMap;
use once_cell::sync::OnceCell;
use tokio::sync::Notify;

use crate::model::template::DropdownItem;
use crate::repository::MerchantRuleRepository;

// ── Global cache ─────────────────────────────────────────────────────────────

/// Global field-config cache: merchantCode → fieldId → Vec<DropdownItem>.
pub static GLOBAL_FIELD_CONFIGS: OnceCell<Arc<DashMap<String, HashMap<String, Vec<DropdownItem>>>>> =
    OnceCell::new();

/// Get (or lazily create) the global map.
pub fn global_configs() -> Arc<DashMap<String, HashMap<String, Vec<DropdownItem>>>> {
    GLOBAL_FIELD_CONFIGS
        .get_or_init(|| Arc::new(DashMap::new()))
        .clone()
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
static INIT_NOTIFY:   OnceCell<Arc<Notify>> = OnceCell::new();

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
    for (i, (merchant_code, dd_map)) in merchant_map.into_iter().enumerate() {
        if i < 10 {
            tracing::info!(
                i,
                merchant_code = %merchant_code,
                items = dd_map.len(),
                "Loaded field config entry"
            );
        }
        cache.insert(merchant_code, dd_map);
    }

    tracing::info!(
        total,
        elapsed_ms = start.elapsed().as_millis(),
        "✅ template fields configs loaded"
    );

    // Signal that initial load is complete — unblocks all `wait_for_init()` callers.
    INIT_COMPLETE.store(true, Ordering::Release);
    init_notify().notify_waiters();

    Ok(())
}

// ── InitLoadingData (startup loader + periodic refresh) ──────────────────────

pub struct InitLoadingData {
    stop_tx: tokio::sync::watch::Sender<bool>,
}

impl InitLoadingData {
    /// Starts async initial load and background 30-minute refresh.
    ///
    /// Mirrors Go's `NewInitLoadingData(cm)`.
    pub fn start(repo: Arc<MerchantRuleRepository>) -> Self {
        // Ensure the global cache exists.
        global_configs();

        let (stop_tx, mut stop_rx) = tokio::sync::watch::channel(false);

        let repo_clone = repo.clone();

        tokio::spawn(async move {
            tracing::info!("Starting async initialisation of field configs...");

            if let Err(e) = do_load_field_configs(&repo_clone).await {
                tracing::error!(error = %e, "Initial field config load failed");
                // Still mark as complete so callers are not blocked forever.
                INIT_COMPLETE.store(true, Ordering::Release);
                init_notify().notify_waiters();
            }

            // Periodic refresh every 30 minutes (mirrors Go's `startPeriodicLoad`).
            let mut interval = tokio::time::interval(Duration::from_secs(30 * 60));
            interval.tick().await; // consume the immediate first tick

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
///
/// **Waits** for the initial DB load to complete before reading the cache —
/// mirrors Go's `GetFieldConfig` which calls `initWg.Wait()` internally.
pub async fn get_field_config(
    merchant_code: &str,
) -> Option<HashMap<String, Vec<DropdownItem>>> {
    wait_for_init().await;
    global_configs().get(merchant_code).map(|v| v.clone())
}

pub fn set_field_config(key: String, value: HashMap<String, Vec<DropdownItem>>) {
    global_configs().insert(key, value);
}
