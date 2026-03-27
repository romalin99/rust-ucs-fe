/// Concurrency utilities.
///
/// Mirrors Go's `pkg/gos/` package:
///   - `routines.go` → [`spawn_safe`]
///   - `lock.go`     → [`LockCounter`], [`lock_timeout`]
///   - `go_pool.go`  → [`TaskPool`]
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;

// ── Safe goroutine spawn (mirrors gos/routines.go) ────────────────────────────

/// Spawn a future that recovers from panics, logging the error but NOT
/// propagating it to the spawner.
///
/// Mirrors Go's `gos.GoSafe(fn func())`:
/// ```go
/// func GoSafe(fn func()) { go runSafe(fn) }
/// func runSafe(fn func()) {
///     defer func() {
///         if p := recover(); p != nil {
///             fmt.Fprintf(os.Stderr, "goroutine panic: %+v\n%s", p, debug.Stack())
///         }
///     }()
///     fn()
/// }
/// ```
///
/// In Rust a panic in a `tokio::spawn` task is caught by tokio and
/// the `JoinHandle` returns `Err(JoinError)`.  We log it here.
pub fn spawn_safe<F>(fut: F) -> tokio::task::JoinHandle<()>
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    tokio::spawn(async move {
        let result = tokio::spawn(fut).await;
        if let Err(e) = result {
            if e.is_panic() {
                let bt = std::backtrace::Backtrace::force_capture();
                tracing::error!("[spawn_safe] goroutine panic: {e}\nstack backtrace:\n{bt}");
                eprintln!("[spawn_safe] goroutine panic: {e}\nstack backtrace:\n{bt}");
                crate::pkg::logs::flush();
            }
        }
    })
}

// ── Dynamic lock timeout (mirrors gos/lock.go) ────────────────────────────────

/// Tracks how many Oracle `FOR UPDATE` operations are currently in flight.
///
/// Mirrors Go's `gos.LockedCount` and `gos.LockedActive`.
pub struct LockCounter {
    count:  AtomicI64,
    active: AtomicBool,
}

impl LockCounter {
    pub const fn new() -> Self {
        Self {
            count:  AtomicI64::new(0),
            active: AtomicBool::new(false),
        }
    }

    pub fn increment(&self) { self.count.fetch_add(1, Ordering::Release); }
    pub fn decrement(&self) { self.count.fetch_sub(1, Ordering::Release); }
    pub fn set_active(&self, v: bool) { self.active.store(v, Ordering::Release); }
    pub fn load_count(&self)  -> i64  { self.count.load(Ordering::Acquire) }
    pub fn is_active(&self)   -> bool { self.active.load(Ordering::Acquire) }
}

impl Default for LockCounter {
    fn default() -> Self { Self::new() }
}

/// Global lock counter — mirrors Go's `gos.LockedCount` and `gos.LockedActive`.
static LOCK_COUNTER: LockCounter = LockCounter::new();

/// Return the Oracle `FOR UPDATE WAIT` timeout in seconds.
///
/// Mirrors Go's `gos.LockTimeout()`:
/// - If the lock counter is inactive → 10 s.
/// - Otherwise → `clamp(16 - concurrent_count, 1, 15)`.
pub fn lock_timeout() -> i64 {
    if LOCK_COUNTER.is_active() {
        let count = LOCK_COUNTER.load_count();
        let t = 16 - count;
        t.clamp(1, 15)
    } else {
        10
    }
}

pub fn lock_counter() -> &'static LockCounter {
    &LOCK_COUNTER
}

// ── Worker pool (mirrors gos/go_pool.go) ──────────────────────────────────────

/// Bounded async task pool.
///
/// Mirrors Go's `gos.InitGoroutinePool` / `gos.GetPool` backed by `pond`.
/// Here we use a `tokio::sync::Semaphore` as the concurrency limiter and a
/// `tokio::sync::mpsc` channel as the work queue.
///
/// Usage:
/// ```rust
/// let pool = TaskPool::new(1000, 20000);
/// pool.submit(async { /* work */ }).await;
/// ```
#[derive(Clone)]
pub struct TaskPool {
    semaphore: Arc<tokio::sync::Semaphore>,
}

impl TaskPool {
    /// Create a new pool.
    ///
    /// - `concurrency` — max simultaneous tasks (mirrors pond pool size).
    /// - `_queue_size` — kept for API parity with Go; Tokio futures are lazy.
    pub fn new(concurrency: usize, _queue_size: usize) -> Self {
        Self {
            semaphore: Arc::new(tokio::sync::Semaphore::new(concurrency)),
        }
    }

    /// Submit a task to the pool.  Blocks until a worker slot is available.
    ///
    /// Mirrors Go's `pool.Submit(task)`.
    pub async fn submit<F>(&self, fut: F)
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        let permit = self.semaphore.clone().acquire_owned().await
            .expect("TaskPool semaphore closed");

        tokio::spawn(async move {
            let _guard = permit; // released when this task completes
            fut.await;
        });
    }

    /// Resize the pool (update concurrency limit).
    ///
    /// Note: Tokio `Semaphore` doesn't support dynamic resize; this is a
    /// no-op kept for API parity with Go's `gos.Resize(count)`.
    pub fn resize(&self, _new_concurrency: usize) {}
}

static DEFAULT_POOL_SIZE: usize = 1000;
static DEFAULT_QUEUE_SIZE: usize = 20000;

/// Global default pool — mirrors Go's `gos.goPool` singleton.
static GLOBAL_POOL: std::sync::OnceLock<TaskPool> = std::sync::OnceLock::new();

/// Get (or initialise) the global task pool.
///
/// Mirrors Go's `gos.GetPool()`.
pub fn get_pool() -> &'static TaskPool {
    GLOBAL_POOL.get_or_init(|| TaskPool::new(DEFAULT_POOL_SIZE, DEFAULT_QUEUE_SIZE))
}
