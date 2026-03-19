/// Shared constants.
///
/// Full port of Go's `pkg/constant/constant.go`.
///
/// Used across services, middleware, and handlers for consistent key names,
/// limits, and sort directions.

// ── Context / span key names ──────────────────────────────────────────────────

/// Request-scoped user ID key (tracing span / request extensions).
pub const CTX_USER_ID:   &str = "user_id";
/// Request-scoped trace ID key.
pub const CTX_TRACE_ID:  &str = "trace_id";
/// Request-scoped service name key.
pub const CTX_SERVICE:   &str = "service";
/// Request-scoped client IP key.
pub const CTX_CLIENT_IP: &str = "client_ip";

// ── Batch and version limits ──────────────────────────────────────────────────

/// Maximum number of items processed in a single batch.
pub const MAX_BATCH_SIZE: usize = 500;
/// Maximum version counter value.
pub const MAX_VERSION:    i64   = 10_000_000;
/// Expected number of items in specific validation structures.
pub const EXPECTED_SIZE:  usize = 16;

// ── Sort direction strings ────────────────────────────────────────────────────

/// Ascending sort direction.
pub const ASC:  &str = "ASC";
/// Descending sort direction.
pub const DESC: &str = "DESC";

// ── Database connection pool defaults ────────────────────────────────────────

/// Default maximum number of open Oracle connections.
pub const MAX_OPEN_CONN: u32 = 100;
/// Default maximum number of idle Oracle connections.
pub const MAX_IDLE_CONN: u32 = 100;
