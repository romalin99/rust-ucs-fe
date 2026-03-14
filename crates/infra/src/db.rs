//! Oracle connection pool — r2d2 + tokio::task::spawn_blocking.
//!
//! The `oracle` crate is synchronous. All actual DB work is pushed onto a
//! dedicated blocking thread-pool via `run()`, so the async executor is never
//! stalled by I/O.

use crate::config::OracleConfig;
use common::error::{AppError, InfraError};
use oracle::sql_type::Timestamp;
use oracle::Connection;
use r2d2::{ManageConnection, Pool, PooledConnection};
use std::sync::Arc;

// ── r2d2 connection manager ───────────────────────────────────────────────────

pub struct OracleManager {
    user: String,
    password: String,
    connect_string: String,
}

impl ManageConnection for OracleManager {
    type Connection = Connection;
    type Error = oracle::Error;

    fn connect(&self) -> Result<Connection, oracle::Error> {
        Connection::connect(&self.user, &self.password, &self.connect_string)
    }

    fn is_valid(&self, conn: &mut Connection) -> Result<(), oracle::Error> {
        conn.ping()
    }

    fn has_broken(&self, _: &mut Connection) -> bool {
        false
    }
}

// ── Public types ──────────────────────────────────────────────────────────────

pub type OraclePool = Arc<Pool<OracleManager>>;

/// Build an Oracle r2d2 pool from configuration.
pub fn build_pool(cfg: &OracleConfig) -> anyhow::Result<OraclePool> {
    let manager = OracleManager {
        user: cfg.user.clone(),
        password: cfg.password.clone(),
        connect_string: cfg.connect_string.clone(),
    };

    let mut builder = Pool::builder().max_size(cfg.max_open_conn);
    if let Some(idle) = cfg.max_idle_conn {
        builder = builder.min_idle(Some(idle));
    }

    Ok(Arc::new(builder.build(manager)?))
}

// ── Async execution wrapper ───────────────────────────────────────────────────

/// Run a synchronous Oracle closure on the blocking thread-pool.
///
/// The closure receives a pooled `Connection` and must return
/// `anyhow::Result<R>`.  Both `oracle::Error` and `r2d2::PoolError` are
/// automatically converted via `?` because they implement `std::error::Error`.
pub async fn run<F, R>(pool: &OraclePool, f: F) -> Result<R, AppError>
where
    F: FnOnce(PooledConnection<OracleManager>) -> anyhow::Result<R> + Send + 'static,
    R: Send + 'static,
{
    let pool = Arc::clone(pool);

    tokio::task::spawn_blocking(move || {
        let conn = pool
            .get()
            .map_err(|e| anyhow::anyhow!("oracle pool acquire failed: {e}"))?;
        f(conn)
    })
    .await
    .map_err(|e| AppError::Infra(InfraError::Pool(e.to_string())))?
    .map_err(|e| AppError::Infra(InfraError::Pool(e.to_string())))
}

// ── Timestamp conversion ──────────────────────────────────────────────────────

/// Convert an Oracle `Timestamp` to `chrono::NaiveDateTime`.
pub fn oracle_ts_to_naive(ts: Timestamp) -> chrono::NaiveDateTime {
    let date = chrono::NaiveDate::from_ymd_opt(ts.year(), ts.month() as u32, ts.day() as u32)
        .unwrap_or_default();

    let time =
        chrono::NaiveTime::from_hms_opt(ts.hour() as u32, ts.minute() as u32, ts.second() as u32)
            .unwrap_or_default();

    chrono::NaiveDateTime::new(date, time)
}
