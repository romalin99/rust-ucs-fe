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
/// | `pkg/logs`               | _replaced_             | Use `tracing` crate           |
/// | `pkg/kafka`              | _not ported_           | Not used in this service      |
/// | `pkg/oracle`             | _via repository_       | Pool in `repository/`         |
/// | `pkg/redis`              | _via infra_            | Client in `infra/`            |
/// | `pkg/bigcache`           | _not ported_           | Go in-process cache specific  |
/// | `pkg/pprof`              | _not ported_           | Go profiling specific         |
/// | `pkg/conv/db_field.go`   | _not ported_           | `sql.Null*` → `Option<T>`     |

pub mod concurrency;
pub mod constant;
pub mod conv;
pub mod helper;
pub mod math;
pub mod memstats;
pub mod metrics;
