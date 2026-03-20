/// Shared utility modules.
///
/// Mirrors Go's `pkg/` package hierarchy.
///
/// | Go package               | Rust module            | Notes                                       |
/// |--------------------------|------------------------|---------------------------------------------|
/// | `pkg/constant`           | `pkg::constant`        | Context keys, limits, sort dirs             |
/// | `pkg/math`               | `pkg::math`            | `round2`                                    |
/// | `pkg/conv`               | `pkg::conv`            | String → numeric conversions, null-safe DB  |
/// | `pkg/helper`             | `pkg::helper`          | Decimal helpers                             |
/// | `pkg/gos`                | `pkg::concurrency`     | Goroutine pool / safe spawn                 |
/// | `pkg/memstatus`          | `pkg::memstats`        | Periodic memory-stats logging               |
/// | `pkg/metrics`            | `pkg::metrics`         | Oracle / Redis pool metrics                 |
/// | `pkg/logs` (all 5 files) | `pkg::logs`            | Global logging API, behavior logger, init   |
/// | `pkg/kafka`              | `pkg::kafka`           | Config stubs, topic consts                  |
/// | `pkg/oracle`             | _(via repository)_     | Pool lives in `repository/`                 |
/// | `pkg/redis`              | _(via infra)_          | Client lives in `infra/`                    |
/// | `pkg/bigcache`           | `config::BigCacheConfig` | Struct in config module                   |
/// | `pkg/pprof`              | `config::PprofConfig`  | Struct in config module                     |

pub mod concurrency;
pub mod constant;
pub mod conv;
pub mod helper;
pub mod kafka;
pub mod logs;
pub mod math;
pub mod memstats;
pub mod metrics;
