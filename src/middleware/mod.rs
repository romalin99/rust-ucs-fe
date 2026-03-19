/// HTTP middleware layer collection.
///
/// Mirrors Go's `internal/middleware/` package:
///
/// | Go file            | Rust module          | Notes                          |
/// |--------------------|----------------------|--------------------------------|
/// | `logger.go`        | `request_logger`     | BehaviorLogger, skip-list, IP  |
/// | `metrics.go`       | `metrics`            | Prometheus counter/histogram   |
/// | `error_handler.go` | `error_handler`      | Wrap non-JSON error responses  |
/// | `recover.go`       | `recover`            | Panic → 500 JSON               |
/// | `trace.go`         | `trace`              | OTel / tracing span per request|

pub mod error_handler;
pub mod metrics;
pub mod recover;
pub mod request_logger;
pub mod trace;

// ── Public re-exports ─────────────────────────────────────────────────────────

/// Tower `Layer` that catches panics and returns 500 JSON.
/// Mirrors Go's `recover.New(...)`.
pub use recover::RecoverLayer;

/// Axum middleware fn: request/response behavior logger.
/// Mirrors Go's `middleware.NewBehaviorLogger(serviceName).Handle()`.
pub use request_logger::behavior_logger;

/// Axum middleware fn: wrap non-JSON error responses in a standard envelope.
/// Mirrors Go's `middleware.ErrorHandler` (fiber error handler).
pub use error_handler::error_handler;

/// Axum middleware fn: per-request Prometheus counter/histogram.
/// Mirrors Go's `FiberPrometheus.Middleware`.
pub use metrics::prometheus_metrics;

/// Axum middleware fn: per-request tracing span with W3C trace-context.
/// Mirrors Go's `middleware.EnableOtelTrace(cfg)`.
pub use trace::otel_trace;
