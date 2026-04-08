/// Oracle repository for `TCG_UCS.MERCHANT_RULE`.
///
/// Uses [rust-oracle](https://github.com/kubo/rust-oracle) (`oracle` crate) for
/// direct Oracle Database access, wrapped in an `r2d2` connection pool.
///
/// All Oracle calls are wrapped in `tokio::task::spawn_blocking` because
/// `oracle::Connection` is sync-only.  The pool is `r2d2::Pool` which is
/// `Clone + Send + Sync`.
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use once_cell::sync::Lazy;

use crate::model::merchant_rule::{MerchantRule, MerchantRuleConfig};
use crate::model::template::{DropdownItem, TemplateField};

// ── Type aliases ──────────────────────────────────────────────────────────────

pub type OraclePool = r2d2::Pool<OracleConnectionManager>;

// ── Oracle connection tuning constants ────────────────────────────────────────
//
// Mirrors Go's godror `appendedOptions`:
//   stmtCacheSize=64 enableStmtCache=true fetchArraySize=110
//   prefetch_count=100 enableClientResultCache=true
//
// These are applied per-connection (stmt cache, LOB prefetch) or per-statement
// (prefetch_rows, fetch_array_size).

/// `stmtCacheSize=64` — number of parsed statement handles cached per session.
pub const STMT_CACHE_SIZE: u32 = 64;

/// `prefetch_count=100` — rows prefetched by OCI per round-trip.
pub const DEFAULT_PREFETCH_ROWS: u32 = 100;

/// `fetchArraySize=110` — internal OCI fetch buffer row count.
pub const DEFAULT_FETCH_ARRAY_SIZE: u32 = 110;

/// LOB data (≤ 128 KB) is prefetched alongside LOB locators, eliminating extra
/// round-trips for CLOB columns (QUESTIONS, `TEMPLATE_FIELDS`).
/// Only effective when `lob_locator()` is used on the statement builder.
const LOB_PREFETCH_BYTES: u32 = 128 * 1024;

// ── Connection manager ────────────────────────────────────────────────────────

/// r2d2 connection manager for rust-oracle.
///
/// Every new connection is configured with:
/// - Statement cache (`STMT_CACHE_SIZE = 64`)
/// - LOB prefetch (`LOB_PREFETCH_BYTES = 128 KB`, i.e. `128 * 1024`)
///
/// Mirrors Go's `pkg/oracle/Config` + `godror` driver with `appendedOptions`.
pub struct OracleConnectionManager {
    user: String,
    password: String,
    connect_string: String,
}

impl OracleConnectionManager {
    pub fn new(user: &str, password: &str, connect_string: &str) -> Self {
        Self {
            user: user.to_string(),
            password: password.to_string(),
            connect_string: connect_string.to_string(),
        }
    }
}

impl r2d2::ManageConnection for OracleConnectionManager {
    type Connection = oracle::Connection;
    type Error = oracle::Error;

    fn connect(&self) -> std::result::Result<oracle::Connection, oracle::Error> {
        let mut conn = oracle::Connector::new(&self.user, &self.password, &self.connect_string)
            .stmt_cache_size(STMT_CACHE_SIZE)
            .connect()?;

        // OCI_ATTR_DEFAULT_LOBPREFETCH_SIZE — prefetch LOB data with locators.
        // Eliminates extra round-trips when `lob_locator()` is used on statements.
        conn.set_oci_attr::<oracle::oci_attr::DefaultLobPrefetchSize>(&LOB_PREFETCH_BYTES)
            .unwrap_or_else(|e| tracing::warn!(error = %e, "failed to set DefaultLobPrefetchSize"));

        Ok(conn)
    }

    fn is_valid(&self, conn: &mut oracle::Connection) -> std::result::Result<(), oracle::Error> {
        conn.ping()
    }

    fn has_broken(&self, _conn: &mut oracle::Connection) -> bool {
        false
    }
}

// ── Pool configuration ───────────────────────────────────────────────────────

/// Pool configuration mirroring Go's `pkg/oracle/Config` + `appendedOptions`.
///
/// Go uses two layers:
///   1. `database/sql` pool: `SetMaxOpenConns`, `SetMaxIdleConns`, etc.
///   2. godror OCI session pool: `poolMinSessions`, `poolMaxSessions`.
///
/// In Rust there is only the r2d2 pool, so we map both layers into a single
/// `PoolConfig`.
pub struct PoolConfig {
    /// Maximum connections in the pool.
    /// Maps to Go's `poolMaxSessions` / `max_open_conn`.
    pub max_size: u32,
    /// Minimum idle connections maintained by r2d2's background thread.
    /// Maps to Go's `poolMinSessions`.
    /// r2d2 creates them lazily in the background after `build_unchecked`.
    pub min_idle: u32,
    /// Max time a connection may be reused (seconds).
    /// Maps to Go's `db.SetConnMaxLifetime`.
    pub max_lifetime_secs: u64,
    /// Max time a connection may sit idle before being closed (minutes).
    /// Maps to Go's `db.SetConnMaxIdleTime`.
    pub max_idle_time_mins: u64,
    /// Timeout waiting for a free connection (seconds).
    pub connection_timeout_secs: u64,
}

/// Build an r2d2 connection pool for rust-oracle.
///
/// Uses `build_unchecked()` so the call returns instantly with zero I/O.
/// r2d2's background thread will create `min_idle` connections asynchronously.
/// Connectivity is validated via [`ping_pool`] after construction.
#[allow(clippy::needless_pass_by_value)]
pub fn build_pool(user: &str, password: &str, connect_string: &str, cfg: PoolConfig) -> OraclePool {
    let manager = OracleConnectionManager::new(user, password, connect_string);
    r2d2::Pool::builder()
        .max_size(cfg.max_size)
        .min_idle(Some(cfg.min_idle))
        .max_lifetime(Some(Duration::from_secs(cfg.max_lifetime_secs)))
        .idle_timeout(Some(Duration::from_secs(cfg.max_idle_time_mins * 60)))
        .connection_timeout(Duration::from_secs(cfg.connection_timeout_secs))
        .build_unchecked(manager)
}

/// Validate Oracle connectivity and warm the connection pool.
///
/// Creates `warm_count` connections in parallel so concurrent startup tasks
/// (field-config loader, USS-mapping loader, etc.) each get a pre-warmed
/// connection.
///
/// Mirrors Go's `db.Ping()` + `poolMinSessions` pre-creation.
pub async fn ping_pool(pool: Arc<OraclePool>, warm_count: usize) {
    let t0 = std::time::Instant::now();
    let handles: Vec<_> = (0..warm_count)
        .map(|i| {
            let pool = pool.clone();
            tokio::task::spawn_blocking(move || {
                let start = std::time::Instant::now();
                match pool.get() {
                    Ok(conn) => match conn.ping() {
                        Ok(()) => tracing::info!(
                            i,
                            elapsed_ms = start.elapsed().as_millis(),
                            "Oracle pool: connection warmed"
                        ),
                        Err(e) => tracing::warn!(i, error = %e, "Oracle ping failed"),
                    },
                    Err(e) => tracing::warn!(
                        i,
                        error = %e,
                        "Oracle pool: could not get connection for ping"
                    ),
                }
            })
        })
        .collect();

    for h in handles {
        if let Err(e) = h.await {
            tracing::warn!(error = %e, "ping_pool: spawn_blocking panicked");
        }
    }
    tracing::info!(
        warm_count,
        elapsed_ms = t0.elapsed().as_millis(),
        "✅ Oracle connection pool warmed"
    );
}

// ── Repository ────────────────────────────────────────────────────────────────

/// SELECT columns (14 columns — mirrors Go's goqu-derived column list).
/// Note: `TEMPLATE_FIELDS` is NOT included here; it's only read in `find_all_template_fields_as_map`.
const RULE_COLS_FULL: &str = "ID, IS_DEFAULT, MERCHANT_CODE, OPERATOR, \
                               IP_RETRY_LIMIT, ACCOUNT_RETRY_LIMIT, EMPTY_SCORE, \
                               LOCK_HOUR, BINDING_TYPE, PASSING_SCORE, \
                               QUESTIONS, FIELD_TRANSLATIONS, \
                               CREATED_AT, UPDATED_AT";

// ── Cached (once) SQL strings for hot-path queries ────────────────────────────
//
// These queries contain no runtime-variable parts — only RULE_COLS_FULL and
// static WHERE clauses — so we pay the format! cost exactly once at first use.

static SQL_FIND_BY_MC: Lazy<String> = Lazy::new(|| {
    format!(
        "SELECT {RULE_COLS_FULL} FROM TCG_UCS.MERCHANT_RULE \
         WHERE MERCHANT_CODE = :1 \
         FETCH FIRST 1 ROWS ONLY"
    )
});

static SQL_FIND_BY_MC_DEFAULT: Lazy<String> = Lazy::new(|| {
    format!(
        "SELECT {RULE_COLS_FULL} FROM TCG_UCS.MERCHANT_RULE \
         WHERE MERCHANT_CODE = :1 AND IS_DEFAULT = :2 \
         FETCH FIRST 1 ROWS ONLY"
    )
});

static SQL_FIND_ONE: Lazy<String> =
    Lazy::new(|| format!("SELECT {RULE_COLS_FULL} FROM TCG_UCS.MERCHANT_RULE WHERE ID = :1"));

#[derive(Clone)]
pub struct MerchantRuleRepository {
    pool: Arc<OraclePool>,
    read_timeout: Duration,
}

impl MerchantRuleRepository {
    #[allow(clippy::missing_const_for_fn)]
    pub fn new(pool: Arc<OraclePool>, read_timeout_secs: u64) -> Self {
        Self {
            pool,
            read_timeout: Duration::from_secs(if read_timeout_secs > 0 {
                read_timeout_secs
            } else {
                15
            }),
        }
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    /// Map a result row (14 columns: `RULE_COLS_FULL`) into a [`MerchantRule`].
    #[allow(clippy::needless_pass_by_value)]
    #[allow(clippy::cast_possible_truncation)]
    fn map_full_row(row: oracle::Row) -> Result<MerchantRule> {
        Ok(MerchantRule {
            id: row.get::<_, i64>(0).context("ID")?,
            is_default: row.get::<_, i32>(1).unwrap_or(0) as i8,
            merchant_code: row.get::<_, String>(2).context("MERCHANT_CODE")?,
            operator: row.get::<_, Option<String>>(3).unwrap_or_default().unwrap_or_default(),
            ip_retry_limit: row.get::<_, i32>(4).context("IP_RETRY_LIMIT")?,
            account_retry_limit: row.get::<_, i32>(5).context("ACCOUNT_RETRY_LIMIT")?,
            empty_score: row.get::<_, i32>(6).context("EMPTY_SCORE")?,
            lock_hour: row.get::<_, i32>(7).unwrap_or(0),
            binding_type: row.get::<_, String>(8).context("BINDING_TYPE")?,
            passing_score: row.get::<_, i32>(9).context("PASSING_SCORE")?,
            questions_json: row.get::<_, Option<String>>(10).context("QUESTIONS")?,
            template_fields_json: None,
            field_translations: row.get::<_, Option<String>>(11).unwrap_or_default(),
            created_at: row
                .get::<_, Option<chrono::NaiveDateTime>>(12)
                .unwrap_or_default()
                .map(|ndt| chrono::DateTime::from_naive_utc_and_offset(ndt, chrono::Utc)),
            updated_at: row
                .get::<_, Option<chrono::NaiveDateTime>>(13)
                .unwrap_or_default()
                .map(|ndt| chrono::DateTime::from_naive_utc_and_offset(ndt, chrono::Utc)),
        })
    }

    // ── Public query methods ──────────────────────────────────────────────────

    /// Find a merchant rule by exact merchant code.
    ///
    /// Returns `None` when no matching row exists.
    /// Mirrors Go's `FindByMerchantCode` method.
    pub async fn find_by_merchant_code(&self, merchant_code: &str) -> Result<Option<MerchantRule>> {
        let pool = self.pool.clone();
        let mc = merchant_code.to_string();
        let timeout = self.read_timeout;

        let blocking = tokio::task::spawn_blocking(move || {
            let conn = pool.get().context("Oracle pool: get connection")?;
            let sql = &*SQL_FIND_BY_MC;

            let mut rows = conn.query(sql, &[&mc]).context("MerchantRule query")?;

            if let Some(row_result) = rows.next() {
                let row = row_result.context("MerchantRule row read")?;
                return Ok(Some(Self::map_full_row(row)?));
            }
            Ok(None)
        });

        tokio::time::timeout(timeout, blocking)
            .await
            .map_err(|_| anyhow!("find_by_merchant_code timed out after {timeout:?}"))?
            .context("spawn_blocking panicked")?
    }

    /// Find a merchant rule by merchant code and `IS_DEFAULT` flag.
    ///
    /// Mirrors Go's `FindByMerchantCodeAndDefault` method.
    pub async fn find_by_merchant_code_and_default(
        &self,
        merchant_code: &str,
        is_default: i32,
    ) -> Result<Option<MerchantRule>> {
        let pool = self.pool.clone();
        let mc = merchant_code.to_string();
        let timeout = self.read_timeout;

        let blocking = tokio::task::spawn_blocking(move || {
            let conn = pool.get().context("Oracle pool: get connection")?;
            let sql = &*SQL_FIND_BY_MC_DEFAULT;

            let mut rows =
                conn.query(sql, &[&mc, &is_default]).context("MerchantRule+default query")?;

            if let Some(row_result) = rows.next() {
                let row = row_result.context("MerchantRule row read")?;
                return Ok(Some(Self::map_full_row(row)?));
            }
            Ok(None)
        });

        tokio::time::timeout(timeout, blocking)
            .await
            .map_err(|_| anyhow!("find_by_merchant_code_and_default timed out after {timeout:?}"))?
            .context("spawn_blocking panicked")?
    }

    /// Slim version: selects only the 6 columns required by the verification flow.
    ///
    /// Mirrors Go's `GetRuleConfigByMerchantCode` method — uses a dedicated SELECT
    /// (`MERCHANT_CODE`, `BINDING_TYPE`, `EMPTY_SCORE`, `PASSING_SCORE`, `QUESTIONS`,
    /// `FIELD_TRANSLATIONS`) instead of fetching all 15 columns.
    pub async fn get_rule_config(&self, merchant_code: &str) -> Result<Option<MerchantRuleConfig>> {
        let pool = self.pool.clone();
        let mc = merchant_code.to_string();
        let timeout = self.read_timeout;

        let blocking = tokio::task::spawn_blocking(move || {
            let conn = pool.get().context("Oracle pool: get connection")?;
            let sql = "SELECT MERCHANT_CODE, BINDING_TYPE, EMPTY_SCORE, PASSING_SCORE, \
                              QUESTIONS, FIELD_TRANSLATIONS \
                       FROM TCG_UCS.MERCHANT_RULE WHERE MERCHANT_CODE = :1";

            let mut rows = conn.query(sql, &[&mc]).context("get_rule_config query")?;

            if let Some(row_result) = rows.next() {
                let row = row_result.context("get_rule_config row read")?;
                #[allow(clippy::cast_possible_truncation)]
                let questions_clob: Option<String> =
                    row.get::<_, Option<String>>(4).unwrap_or(None);
                #[allow(clippy::cast_possible_truncation)]
                let translations_clob: Option<String> =
                    row.get::<_, Option<String>>(5).unwrap_or(None);
                return Ok(Some(MerchantRuleConfig {
                    id: 0,
                    merchant_code: row.get::<_, String>(0)?,
                    binding_type: row.get::<_, String>(1)?,
                    empty_score: row.get::<_, i32>(2)?,
                    passing_score: row.get::<_, i32>(3)?,
                    lock_hour: 0,
                    ip_retry_limit: 0,
                    account_retry_limit: 0,
                    questions_json: questions_clob,
                    field_translations: translations_clob,
                }));
            }
            Ok(None)
        });

        tokio::time::timeout(timeout, blocking)
            .await
            .map_err(|_| anyhow!("get_rule_config timed out after {timeout:?}"))?
            .context("spawn_blocking panicked")?
    }

    /// Load all merchant rules and build the field-config dropdown map.
    ///
    /// Only fields with `fieldAttribute == "DD"` (dropdown) are included —
    /// mirrors Go's `FindAllTemplateFieldsAsMap` method filter logic.
    ///
    /// Strategy: try Oracle 12.2 `JSON_ARRAYAGG` first (collapses N CLOB
    /// round-trips into 1). If the DB doesn't support it or the JSON is
    /// malformed, transparently fall back to row-by-row fetching.
    ///
    /// Returns: `HashMap<merchantCode, HashMap<fieldId, Vec<DropdownItem>>>`
    pub async fn find_all_as_map(
        &self,
    ) -> Result<HashMap<String, HashMap<String, Vec<DropdownItem>>>> {
        let pool = self.pool.clone();
        let timeout = self.read_timeout;

        let blocking = tokio::task::spawn_blocking(move || {
            let t0 = std::time::Instant::now();
            let conn = pool.get().context("Oracle pool: get connection")?;
            let pool_get_ms = t0.elapsed().as_millis();

            match Self::find_all_as_map_aggregated(&conn, pool_get_ms, t0) {
                Ok(result) => Ok(result),
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "JSON_ARRAYAGG path failed — falling back to row-by-row"
                    );
                    Self::find_all_as_map_rowbyrow(&conn, pool_get_ms, t0)
                }
            }
        });

        tokio::time::timeout(timeout, blocking)
            .await
            .map_err(|_| anyhow!("find_all_as_map timed out after {timeout:?}"))?
            .context("spawn_blocking panicked")?
    }

    /// Fast path: aggregate all `TEMPLATE_FIELDS` into one JSON CLOB server-side.
    fn find_all_as_map_aggregated(
        conn: &oracle::Connection,
        pool_get_ms: u128,
        t0: std::time::Instant,
    ) -> Result<HashMap<String, HashMap<String, Vec<DropdownItem>>>> {
        let sql = "SELECT JSON_ARRAYAGG(\
                       JSON_OBJECT(\
                           KEY 'mc' VALUE MERCHANT_CODE, \
                           KEY 'tf' VALUE TEMPLATE_FIELDS FORMAT JSON \
                           ABSENT ON NULL\
                       ) \
                       RETURNING CLOB\
                   ) \
                   FROM TCG_UCS.MERCHANT_RULE \
                   WHERE TEMPLATE_FIELDS IS NOT NULL";

        let row = conn.query_row(sql, &[]).context("JSON_ARRAYAGG query")?;

        let agg_json: Option<String> = row.get(0).unwrap_or_default();
        let query_ms = t0.elapsed().as_millis() - pool_get_ms;

        let mut result: HashMap<String, HashMap<String, Vec<DropdownItem>>> = HashMap::new();

        if let Some(ref json_str) = agg_json {
            #[derive(serde::Deserialize)]
            struct AggEntry {
                mc: String,
                tf: Vec<TemplateField>,
            }

            let entries: Vec<AggEntry> =
                serde_json::from_str(json_str).context("parse aggregated JSON")?;

            for entry in entries {
                let mut field_map: HashMap<String, Vec<DropdownItem>> = HashMap::new();
                for f in entry.tf {
                    if f.field_attribute == "DD" && !f.field_id.is_empty() {
                        field_map.insert(f.field_id, f.dropdown_list);
                    }
                }
                if !field_map.is_empty() {
                    result.insert(entry.mc, field_map);
                }
            }
        }

        let parse_ms = t0.elapsed().as_millis() - pool_get_ms - query_ms;
        tracing::info!(
            total_merchants = result.len(),
            pool_get_ms,
            query_ms,
            parse_ms,
            total_ms = t0.elapsed().as_millis(),
            "find_all_as_map: loaded via JSON_ARRAYAGG"
        );
        Ok(result)
    }

    /// Slow path: fetch rows one-by-one (works on all Oracle versions).
    fn find_all_as_map_rowbyrow(
        conn: &oracle::Connection,
        pool_get_ms: u128,
        t0: std::time::Instant,
    ) -> Result<HashMap<String, HashMap<String, Vec<DropdownItem>>>> {
        let sql = "SELECT MERCHANT_CODE, TEMPLATE_FIELDS \
                   FROM TCG_UCS.MERCHANT_RULE \
                   WHERE TEMPLATE_FIELDS IS NOT NULL";

        let mut stmt = conn
            .statement(sql)
            .prefetch_rows(DEFAULT_PREFETCH_ROWS)
            .fetch_array_size(DEFAULT_FETCH_ARRAY_SIZE)
            .build()
            .context("find_all_as_map rowbyrow prepare")?;
        let rows = stmt.query(&[]).context("find_all_as_map rowbyrow query")?;

        let query_ms = t0.elapsed().as_millis() - pool_get_ms;
        let mut result: HashMap<String, HashMap<String, Vec<DropdownItem>>> = HashMap::new();

        for row_result in rows {
            let row = row_result.context("row read")?;
            let merchant_code: String = row.get(0).context("MERCHANT_CODE")?;
            let tf_json: Option<String> = row.get(1).unwrap_or_default();

            let json = match tf_json {
                Some(j) if !j.is_empty() => j,
                _ => continue,
            };

            let fields: Vec<TemplateField> = match serde_json::from_str(&json) {
                Ok(f) => f,
                Err(e) => {
                    tracing::warn!(
                        merchant_code = %merchant_code,
                        error         = %e,
                        "Failed to parse TEMPLATE_FIELDS JSON — skipping"
                    );
                    continue;
                }
            };

            let mut field_map: HashMap<String, Vec<DropdownItem>> = HashMap::new();
            for f in fields {
                if f.field_attribute == "DD" && !f.field_id.is_empty() {
                    field_map.insert(f.field_id, f.dropdown_list);
                }
            }
            if !field_map.is_empty() {
                result.insert(merchant_code, field_map);
            }
        }

        let iterate_ms = t0.elapsed().as_millis() - pool_get_ms - query_ms;
        tracing::info!(
            total_merchants = result.len(),
            pool_get_ms,
            query_ms,
            iterate_ms,
            total_ms = t0.elapsed().as_millis(),
            "find_all_as_map: loaded via row-by-row fallback"
        );
        Ok(result)
    }

    /// Update `TEMPLATE_FIELDS` for a merchant.
    ///
    /// Mirrors Go's `UpdateTemplateFieldsByMerchantCode` method.
    pub async fn update_template_fields(
        &self,
        merchant_code: &str,
        template_fields_json: &str,
    ) -> Result<u64> {
        let pool = self.pool.clone();
        let mc = merchant_code.to_string();
        let fields_json = template_fields_json.to_string();
        let timeout = self.read_timeout;

        let blocking = tokio::task::spawn_blocking(move || {
            let conn = pool.get().context("Oracle pool: get connection")?;
            let sql = "UPDATE TCG_UCS.MERCHANT_RULE \
                       SET TEMPLATE_FIELDS = :1, UPDATED_AT = SYSTIMESTAMP \
                       WHERE MERCHANT_CODE = :2";

            let stmt = conn
                .execute(sql, &[&fields_json, &mc])
                .context("UpdateTemplateFields execute")?;

            let rows_affected = stmt.row_count().context("row_count")?;
            if rows_affected == 0 {
                return Err(anyhow!("UpdateTemplateFields: no row for merchant_code={mc}"));
            }
            conn.commit().context("commit UpdateTemplateFields")?;
            Ok(rows_affected)
        });

        tokio::time::timeout(timeout, blocking)
            .await
            .map_err(|_| anyhow!("update_template_fields timed out after {timeout:?}"))?
            .context("spawn_blocking panicked")?
    }
    // ── Additional CRUD mirrors ───────────────────────────────────────────────

    /// Map an Oracle result row (using `RULE_COLS_FULL` column order) to `MerchantRule`.
    ///
    /// Column indices (0-based, 14 columns):
    ///   0:`ID` 1:`IS_DEFAULT` 2:`MERCHANT_CODE` 3:`OPERATOR`
    ///   4:`IP_RETRY_LIMIT` 5:`ACCOUNT_RETRY_LIMIT` 6:`EMPTY_SCORE`
    ///   7:`LOCK_HOUR` 8:`BINDING_TYPE` 9:`PASSING_SCORE`
    ///   10:`QUESTIONS` 11:`FIELD_TRANSLATIONS`
    ///   12:`CREATED_AT` 13:`UPDATED_AT`
    #[allow(clippy::needless_pass_by_value)]
    #[allow(clippy::cast_possible_truncation)]
    fn map_rule_row(row: oracle::Row) -> anyhow::Result<MerchantRule> {
        Ok(MerchantRule {
            id: row.get::<_, i64>(0).context("ID")?,
            is_default: row.get::<_, i32>(1).unwrap_or(0) as i8,
            merchant_code: row.get::<_, String>(2).context("MERCHANT_CODE")?,
            operator: row.get::<_, String>(3).unwrap_or_default(),
            ip_retry_limit: row.get::<_, i32>(4).unwrap_or(0),
            account_retry_limit: row.get::<_, i32>(5).unwrap_or(0),
            empty_score: row.get::<_, i32>(6).unwrap_or(0),
            lock_hour: row.get::<_, i32>(7).unwrap_or(0),
            binding_type: row.get::<_, String>(8).unwrap_or_default(),
            passing_score: row.get::<_, i32>(9).unwrap_or(0),
            questions_json: row.get::<_, Option<String>>(10).context("QUESTIONS")?,
            template_fields_json: None,
            field_translations: row.get::<_, Option<String>>(11).unwrap_or_default(),
            created_at: row
                .get::<_, Option<chrono::NaiveDateTime>>(12)
                .unwrap_or_default()
                .map(|ndt| chrono::DateTime::from_naive_utc_and_offset(ndt, chrono::Utc)),
            updated_at: row
                .get::<_, Option<chrono::NaiveDateTime>>(13)
                .unwrap_or_default()
                .map(|ndt| chrono::DateTime::from_naive_utc_and_offset(ndt, chrono::Utc)),
        })
    }

    /// Find a single merchant rule by primary key.  Mirrors Go's `FindOne` method.
    pub async fn find_one(&self, id: i64) -> Result<Option<MerchantRule>> {
        let pool = self.pool.clone();
        let timeout = self.read_timeout;

        let blocking = tokio::task::spawn_blocking(move || {
            let conn = pool.get().context("Oracle pool: get connection")?;
            let sql = &*SQL_FIND_ONE;
            let mut rows = conn.query(sql, &[&id]).context("FindOne query")?;
            if let Some(row_result) = rows.next() {
                return Ok(Some(Self::map_rule_row(row_result.context("FindOne row")?)?));
            }
            Ok(None)
        });

        tokio::time::timeout(timeout, blocking)
            .await
            .map_err(|_| anyhow!("find_one timed out"))?
            .context("spawn_blocking panicked")?
    }

    /// Return all `MERCHANT_CODE` values in the table.  Mirrors Go's `GetAllMerchantCodes` method.
    pub async fn get_all_merchant_codes(&self) -> Result<Vec<String>> {
        let pool = self.pool.clone();
        let timeout = self.read_timeout;

        let blocking = tokio::task::spawn_blocking(move || {
            let conn = pool.get().context("Oracle pool: get connection")?;
            let mut stmt = conn
                .statement("SELECT MERCHANT_CODE FROM TCG_UCS.MERCHANT_RULE")
                .prefetch_rows(DEFAULT_PREFETCH_ROWS)
                .fetch_array_size(DEFAULT_FETCH_ARRAY_SIZE)
                .build()
                .context("GetAllMerchantCodes prepare")?;
            let rows = stmt.query(&[]).context("GetAllMerchantCodes query")?;
            let mut out = Vec::new();
            for row_result in rows {
                let row: oracle::Row = row_result.context("row")?;
                let code: String = row.get(0).context("MERCHANT_CODE")?;
                out.push(code);
            }
            Ok(out)
        });

        tokio::time::timeout(timeout, blocking)
            .await
            .map_err(|_| anyhow!("get_all_merchant_codes timed out"))?
            .context("spawn_blocking panicked")?
    }

    /// Insert a new merchant rule and return the generated `ID`.
    ///
    /// Fetches the next sequence value first, then inserts — mirrors Go's `Insert` method.
    pub async fn insert(&self, rule: MerchantRule) -> Result<i64> {
        let pool = self.pool.clone();
        let timeout = self.read_timeout;

        let blocking = tokio::task::spawn_blocking(move || {
            let conn = pool.get().context("Oracle pool: get connection")?;
            let new_id = conn
                .query_row_as::<i64>("SELECT SEQ_MERCHANT_RULE.NEXTVAL FROM DUAL", &[])
                .context("SEQ_MERCHANT_RULE.NEXTVAL")?;

            let sql = "INSERT INTO TCG_UCS.MERCHANT_RULE \
                        (ID, IS_DEFAULT, MERCHANT_CODE, OPERATOR, \
                         IP_RETRY_LIMIT, ACCOUNT_RETRY_LIMIT, EMPTY_SCORE, \
                         LOCK_HOUR, BINDING_TYPE, PASSING_SCORE, QUESTIONS, \
                         FIELD_TRANSLATIONS, \
                         CREATED_AT, UPDATED_AT) \
                       VALUES \
                        (:1, :2, :3, :4, :5, :6, :7, :8, :9, :10, :11, \
                         :12, SYSTIMESTAMP, SYSTIMESTAMP)";

            let ft = rule.field_translations.as_deref().unwrap_or("{}");

            conn.execute(
                sql,
                &[
                    &new_id,
                    &rule.is_default,
                    &rule.merchant_code,
                    &rule.operator,
                    &rule.ip_retry_limit,
                    &rule.account_retry_limit,
                    &rule.empty_score,
                    &rule.lock_hour,
                    &rule.binding_type,
                    &rule.passing_score,
                    &rule.questions_json,
                    &ft,
                ],
            )
            .context("MerchantRule INSERT execute")?;

            conn.commit().context("MerchantRule INSERT commit")?;
            Ok(new_id)
        });

        tokio::time::timeout(timeout, blocking)
            .await
            .map_err(|_| anyhow!("merchant_rule insert timed out"))?
            .context("spawn_blocking panicked")?
    }

    /// Full UPDATE of a merchant rule row by primary key.  Mirrors Go's `Update` method.
    pub async fn update(&self, rule: MerchantRule) -> Result<u64> {
        let id = rule.id;
        let pool = self.pool.clone();
        let timeout = self.read_timeout;

        let blocking = tokio::task::spawn_blocking(move || {
            let conn = pool.get().context("Oracle pool: get connection")?;
            let sql = "UPDATE TCG_UCS.MERCHANT_RULE SET \
                            IS_DEFAULT          = :1, \
                            MERCHANT_CODE       = :2, \
                            OPERATOR            = :3, \
                            IP_RETRY_LIMIT      = :4, \
                            ACCOUNT_RETRY_LIMIT = :5, \
                            EMPTY_SCORE         = :6, \
                            LOCK_HOUR           = :7, \
                            BINDING_TYPE        = :8, \
                            PASSING_SCORE       = :9, \
                            QUESTIONS           = :10, \
                            FIELD_TRANSLATIONS  = :11, \
                            UPDATED_AT          = SYSTIMESTAMP \
                        WHERE ID = :12";

            let ft = rule.field_translations.as_deref().unwrap_or("{}");

            let stmt = conn
                .execute(
                    sql,
                    &[
                        &rule.is_default,
                        &rule.merchant_code,
                        &rule.operator,
                        &rule.ip_retry_limit,
                        &rule.account_retry_limit,
                        &rule.empty_score,
                        &rule.lock_hour,
                        &rule.binding_type,
                        &rule.passing_score,
                        &rule.questions_json,
                        &ft,
                        &id,
                    ],
                )
                .context("MerchantRule UPDATE execute")?;

            let rows = stmt.row_count().context("row_count")?;
            if rows == 0 {
                return Err(anyhow!("MerchantRule UPDATE: no row for id={id}"));
            }
            conn.commit().context("MerchantRule UPDATE commit")?;
            Ok(rows)
        });

        tokio::time::timeout(timeout, blocking)
            .await
            .map_err(|_| anyhow!("merchant_rule update timed out"))?
            .context("spawn_blocking panicked")?
    }

    /// Delete a merchant rule row by primary key.  Mirrors Go's `Delete` method.
    pub async fn delete(&self, id: i64) -> Result<u64> {
        let pool = self.pool.clone();
        let timeout = self.read_timeout;

        let blocking = tokio::task::spawn_blocking(move || {
            let conn = pool.get().context("Oracle pool: get connection")?;
            let stmt = conn
                .execute("DELETE FROM TCG_UCS.MERCHANT_RULE WHERE ID = :1", &[&id])
                .context("MerchantRule DELETE execute")?;
            let rows = stmt.row_count().context("row_count")?;
            if rows == 0 {
                return Err(anyhow!("MerchantRule DELETE: no row for id={id}"));
            }
            conn.commit().context("MerchantRule DELETE commit")?;
            Ok(rows)
        });

        tokio::time::timeout(timeout, blocking)
            .await
            .map_err(|_| anyhow!("merchant_rule delete timed out"))?
            .context("spawn_blocking panicked")?
    }

    // ── Additional methods mirroring Go's MerchantRuleRepository ─────────────

    /// Fetch and lock a merchant rule by ID inside a transaction-like flow.
    ///
    /// Mirrors Go's `FindOneForUpdate(tx, id)` method which uses
    /// `SELECT ... FOR UPDATE WAIT <lockTimeout>` syntax.
    ///
    /// Because `rust-oracle` connections are not transactional by default
    /// (auto-commit is off until explicit `conn.commit()`), we use a dedicated
    /// connection from the pool, issue `SELECT ... FOR UPDATE WAIT N`,
    /// and return both the rule and the connection so the caller can
    /// `conn.commit()` or `conn.rollback()` later.
    pub async fn find_one_for_update(
        &self,
        id: i64,
        lock_timeout_secs: u32,
    ) -> Result<(MerchantRule, r2d2::PooledConnection<OracleConnectionManager>)> {
        let pool = self.pool.clone();
        let timeout = self.read_timeout;

        let blocking = tokio::task::spawn_blocking(move || {
            let conn = pool.get().context("Oracle pool: get connection")?;
            let sql = format!(
                "SELECT {RULE_COLS_FULL} FROM TCG_UCS.MERCHANT_RULE WHERE ID = :1 FOR UPDATE WAIT {lock_timeout_secs}"
            );
            let mut rows = conn.query(&sql, &[&id]).context("FindOneForUpdate query")?;

            if let Some(row_result) = rows.next() {
                let row = row_result.context("FindOneForUpdate row read")?;
                let rule = Self::map_full_row(row)?;
                return Ok((rule, conn));
            }
            Err(anyhow!("FindOneForUpdate: no row for id={id}"))
        });

        tokio::time::timeout(timeout, blocking)
            .await
            .map_err(|_| anyhow!("find_one_for_update timed out"))?
            .context("spawn_blocking panicked")?
    }

    /// Dynamic single-row SELECT with optional FOR UPDATE.
    ///
    /// Mirrors Go's `FindOnlyByEx(ctx, ex, tx, forUpdateWaitOpt)` method.
    ///
    /// `where_clause` is a raw SQL fragment (e.g. `"MERCHANT_CODE = :1 AND IS_DEFAULT = :2"`).
    /// `params` are the bind values in positional order.
    /// `for_update` controls locking: `None` = no lock, `Some(secs)` = `FOR UPDATE WAIT <secs>` syntax.
    pub async fn find_only_by_expression(
        &self,
        where_clause: &str,
        params: Vec<Box<dyn oracle::sql_type::ToSql + Send>>,
        for_update: Option<u32>,
    ) -> Result<Option<MerchantRule>> {
        let pool = self.pool.clone();
        let timeout = self.read_timeout;
        let wc = where_clause.to_string();

        let blocking = tokio::task::spawn_blocking(move || {
            use std::fmt::Write;
            let conn = pool.get().context("Oracle pool: get connection")?;

            let mut sql = format!("SELECT {RULE_COLS_FULL} FROM TCG_UCS.MERCHANT_RULE WHERE {wc}");
            if let Some(wait_secs) = for_update {
                let _ = write!(sql, " FOR UPDATE WAIT {wait_secs}");
            }

            let param_refs: Vec<&dyn oracle::sql_type::ToSql> =
                params.iter().map(|p| p.as_ref() as &dyn oracle::sql_type::ToSql).collect();
            let mut rows =
                conn.query(&sql, param_refs.as_slice()).context("FindOnlyByExpression query")?;

            if let Some(row_result) = rows.next() {
                let row = row_result.context("FindOnlyByExpression row read")?;
                return Ok(Some(Self::map_full_row(row)?));
            }
            Ok(None)
        });

        tokio::time::timeout(timeout, blocking)
            .await
            .map_err(|_| anyhow!("find_only_by_expression timed out"))?
            .context("spawn_blocking panicked")?
    }

    /// Dynamic multi-row SELECT with optional ordering and pagination.
    ///
    /// Mirrors Go's `FindList(ctx, ex, optionalParams...)` method.
    ///
    /// `where_clause` — raw SQL WHERE fragment (e.g. `"IS_DEFAULT = :1"`).
    /// `params`       — positional bind values.
    /// `order_by`     — optional ORDER BY clause (e.g. `"CREATED_AT DESC"`).
    /// `pagination`   — optional `(page, page_size)` for Oracle OFFSET/FETCH.
    pub async fn find_list(
        &self,
        where_clause: &str,
        params: Vec<Box<dyn oracle::sql_type::ToSql + Send>>,
        order_by: Option<&str>,
        pagination: Option<(u32, u32)>,
    ) -> Result<Vec<MerchantRule>> {
        let pool = self.pool.clone();
        let timeout = self.read_timeout;
        let wc = where_clause.to_string();
        let ob = order_by.map(ToString::to_string);

        let blocking = tokio::task::spawn_blocking(move || {
            use std::fmt::Write;
            let conn = pool.get().context("Oracle pool: get connection")?;

            let mut sql = format!("SELECT {RULE_COLS_FULL} FROM TCG_UCS.MERCHANT_RULE WHERE {wc}");
            if let Some(ref order) = ob {
                let _ = write!(sql, " ORDER BY {order}");
            }
            if let Some((page, page_size)) = pagination {
                #[allow(clippy::cast_possible_truncation)]
                let offset = (page.saturating_sub(1)) * page_size;
                let _ = write!(sql, " OFFSET {offset} ROWS FETCH NEXT {page_size} ROWS ONLY");
            }

            let param_refs: Vec<&dyn oracle::sql_type::ToSql> =
                params.iter().map(|p| p.as_ref() as _).collect();
            let mut stmt = conn
                .statement(&sql)
                .prefetch_rows(DEFAULT_PREFETCH_ROWS)
                .fetch_array_size(DEFAULT_FETCH_ARRAY_SIZE)
                .build()
                .context("FindList prepare")?;
            let rows = stmt.query(param_refs.as_slice()).context("FindList query")?;

            let mut result = Vec::new();
            for row_result in rows {
                let row = row_result.context("FindList row read")?;
                result.push(Self::map_full_row(row)?);
            }
            Ok(result)
        });

        tokio::time::timeout(timeout, blocking)
            .await
            .map_err(|_| anyhow!("find_list timed out"))?
            .context("spawn_blocking panicked")?
    }

    /// Dynamic UPDATE with arbitrary SET and WHERE clauses.
    ///
    /// Mirrors Go's `UpdateByEx(ctx, record, ex, tx)` method.
    ///
    /// `set_clause`    — raw SQL SET fragment (e.g. `"QUESTIONS = :1, UPDATED_AT = SYSTIMESTAMP"`).
    /// `where_clause`  — raw SQL WHERE fragment (e.g. `"MERCHANT_CODE = :2"`).
    /// `params`        — all bind values for both SET and WHERE in positional order.
    pub async fn update_by_expression(
        &self,
        set_clause: &str,
        where_clause: &str,
        params: Vec<Box<dyn oracle::sql_type::ToSql + Send>>,
    ) -> Result<u64> {
        let pool = self.pool.clone();
        let timeout = self.read_timeout;
        let sc = set_clause.to_string();
        let wc = where_clause.to_string();

        let blocking = tokio::task::spawn_blocking(move || {
            let conn = pool.get().context("Oracle pool: get connection")?;

            let sql = format!(
                "UPDATE TCG_UCS.MERCHANT_RULE SET {sc}, UPDATED_AT = SYSTIMESTAMP WHERE {wc}"
            );

            let param_refs: Vec<&dyn oracle::sql_type::ToSql> =
                params.iter().map(|p| p.as_ref() as &dyn oracle::sql_type::ToSql).collect();
            let stmt = conn
                .execute(&sql, param_refs.as_slice())
                .context("UpdateByExpression execute")?;
            let rows = stmt.row_count().context("row_count")?;
            conn.commit().context("UpdateByExpression commit")?;
            Ok(rows)
        });

        tokio::time::timeout(timeout, blocking)
            .await
            .map_err(|_| anyhow!("update_by_expression timed out"))?
            .context("spawn_blocking panicked")?
    }

    /// Update `TEMPLATE_FIELDS` using an existing connection (transaction).
    ///
    /// Mirrors Go's `UpdateTemplateFieldsByMerchantCodeTx(tx, merchantCode, templateFields)` method.
    ///
    /// The caller must pass in a pooled connection that already holds a
    /// transaction (e.g. from `find_one_for_update`).  This method does NOT
    /// commit — the caller is responsible for `conn.commit()`.
    pub fn update_template_fields_tx(
        conn: &oracle::Connection,
        merchant_code: &str,
        template_fields: &str,
    ) -> Result<u64> {
        let sql = "UPDATE TCG_UCS.MERCHANT_RULE                    SET TEMPLATE_FIELDS = :1, UPDATED_AT = SYSTIMESTAMP                    WHERE MERCHANT_CODE = :2";
        let stmt = conn
            .execute(sql, &[&template_fields, &merchant_code])
            .context("UpdateTemplateFieldsTx execute")?;
        let rows = stmt.row_count().context("row_count")?;
        if rows == 0 {
            return Err(anyhow!("no merchant rule found for merchantCode: {merchant_code}"));
        }
        Ok(rows)
    }
}
