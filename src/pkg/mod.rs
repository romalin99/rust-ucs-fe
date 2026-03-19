/// Shared utility modules.
///
/// Mirrors Go's `pkg/` package hierarchy:
///
/// | Go package               | Rust module            | Notes                         |
/// |--------------------------|------------------------|-------------------------------|
/// | `pkg/constant`           | `pkg::constant`        | Context keys, limits, dirs    |
/// | `pkg/math`               | `pkg::math`            | `round2`                      |
/// | `pkg/conv`               | `pkg::conv`            | String → numeric conversions  |
/// | `pkg/helper`             | `pkg::helper`          | Decimal helpers               |
/// | `pkg/gos`                | `pkg::concurrency`     | Goroutine pool, safe spawn    |
/// | `pkg/memstatus`          | `pkg::memstats`        | Periodic memory stats logging |
/// | `pkg/metrics`            | `pkg::metrics`         | Oracle/Redis pool metrics     |
/// | `pkg/logs`               | `pkg::logs`            | Global logging API wrappers   |
/// | `pkg/kafka`              | `pkg::kafka`           | Config, stubs, topic consts   |
/// | `pkg/oracle`             | _via repository_       | Pool in `repository/`         |
/// | `pkg/redis`              | _via infra_            | Client in `infra/`            |
/// | `pkg/bigcache`           | `config::BigCacheConfig` | Struct in config module     |
/// | `pkg/pprof`              | `config::PprofConfig`  | Struct in config module       |
/// | `pkg/conv/db_field.go`   | `pkg::conv`            | `Option<T>` null-safe helpers |
/// | `pkg/zlog`               | `pkg::zlog`            | Custom encoder + init wrapper |

pub mod concurrency;
pub mod constant;
pub mod conv;
pub mod helper;
pub mod kafka;
pub mod logs;
pub mod math;
pub mod memstats;
pub mod metrics;
pub mod zlog;
