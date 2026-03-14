//! In-memory cache of per-merchant dropdown field configurations.
//!
//! Maps `merchantCode → fieldId → Vec<DropdownItem>`.
//!
//! Mirrors Go's `GlobalFieldConfigs` + `InitLoadingData`:
//!   - Loaded once from Oracle (`find_all_as_map`) at startup.
//!   - Refreshed every 30 minutes in the background.
//!   - `get_dropdown` is lock-free thanks to `DashMap`.

use domain::DropdownItem;
use repository::MerchantRuleRepo;
use dashmap::DashMap;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::sync::watch;
use tracing::{info, warn};

/// Thread-safe, clone-cheap handle to the field config cache.
#[derive(Clone)]
pub struct FieldCache {
    /// `merchantCode → fieldId → Vec<DropdownItem>`
    inner: Arc<DashMap<String, HashMap<String, Vec<DropdownItem>>>>,
    /// Becomes `true` once the initial load has completed.
    ready_rx: watch::Receiver<bool>,
}

impl FieldCache {
    /// Create an empty cache.  Call `start_loader` to populate it.
    pub fn new() -> (Self, watch::Sender<bool>) {
        let (tx, rx) = watch::channel(false);
        let cache = FieldCache {
            inner: Arc::new(DashMap::new()),
            ready_rx: rx,
        };
        (cache, tx)
    }

    // ── Read ──────────────────────────────────────────────────────────────────

    /// Wait for the initial load to complete, then return the dropdown list
    /// for `(merchant_code, field_id)`.
    ///
    /// Returns `None` when the field has no dropdown or the merchant is unknown.
    pub async fn get_dropdown(
        &self,
        merchant_code: &str,
        field_id: &str,
    ) -> Option<Vec<DropdownItem>> {
        self.wait_ready().await;
        self.inner
            .get(merchant_code)
            .and_then(|m| m.get(field_id).cloned())
    }

    /// Non-async variant — does not block for initial load.
    /// Safe to call from synchronous contexts after the server has started.
    pub fn get_dropdown_sync(
        &self,
        merchant_code: &str,
        field_id: &str,
    ) -> Option<Vec<DropdownItem>> {
        self.inner
            .get(merchant_code)
            .and_then(|m| m.get(field_id).cloned())
    }

    /// Snapshot of the complete cache — for diagnostics / cron jobs.
    pub fn snapshot(&self) -> HashMap<String, HashMap<String, Vec<DropdownItem>>> {
        self.inner
            .iter()
            .map(|e| (e.key().clone(), e.value().clone()))
            .collect()
    }

    // ── Write ─────────────────────────────────────────────────────────────────

    /// Directly insert or overwrite a merchant's field map.
    pub fn set_merchant(&self, merchant_code: String, fields: HashMap<String, Vec<DropdownItem>>) {
        self.inner.insert(merchant_code, fields);
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    async fn wait_ready(&self) {
        let mut rx = self.ready_rx.clone();
        if *rx.borrow() {
            return;
        }
        let _ = rx.wait_for(|ready| *ready).await;
    }

    /// Same as `wait_ready` but public — called from `main` after starting the loader.
    pub async fn wait_ready_blocking(&self) {
        self.wait_ready().await;
    }

    fn reload_from_map(&self, map: HashMap<String, HashMap<String, Vec<DropdownItem>>>) {
        let total = map.len();
        for (merchant, fields) in map {
            self.inner.insert(merchant, fields);
        }
        info!("FieldCache reloaded: {} merchants", total);
    }
}

// ── Background loader ─────────────────────────────────────────────────────────

/// Spawn the background goroutine that:
/// 1. Performs the initial DB load and signals `ready_tx`.
/// 2. Refreshes the cache every 30 minutes.
///
/// The returned `JoinHandle` can be aborted during graceful shutdown.
pub fn start_loader(
    cache: FieldCache,
    repo: MerchantRuleRepo,
    ready_tx: watch::Sender<bool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        info!("FieldCache: starting initial load…");

        // Initial load.
        match repo.find_all_as_map().await {
            Ok(map) => {
                cache.reload_from_map(map);
                let _ = ready_tx.send(true);
                info!("FieldCache: initial load complete, cache is ready");
            }
            Err(e) => {
                warn!(
                    "FieldCache: initial load failed: {} — cache will be empty",
                    e
                );
                // Signal ready anyway so the server doesn't block indefinitely.
                let _ = ready_tx.send(true);
            }
        }

        // Periodic refresh every 30 minutes.
        let mut interval = tokio::time::interval(Duration::from_secs(30 * 60));
        interval.tick().await; // consume the immediate first tick

        loop {
            interval.tick().await;
            info!("FieldCache: starting scheduled refresh…");
            match repo.find_all_as_map().await {
                Ok(map) => cache.reload_from_map(map),
                Err(e) => warn!("FieldCache: refresh failed: {}", e),
            }
        }
    })
}
