/// Oracle repository for TCG_UCS.MERCHANT_RULE.
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

use anyhow::{anyhow, Context, Result};

use crate::model::merchant_rule::{MerchantRule, MerchantRuleConfig, Question};
use crate::model::template::{DropdownItem, TemplateField};

// ── Type aliases ──────────────────────────────────────────────────────────────

pub type OraclePool = r2d2::Pool<OracleConnectionManager>;

// ── Connection manager ────────────────────────────────────────────────────────

/// r2d2 connection manager for rust-oracle.
///
/// Mirrors Go's `pkg/oracle/Config` + `godror` driver registration.
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
        oracle::Connection::connect(&self.user, &self.password, &self.connect_string)
    }

    fn is_valid(&self, conn: &mut oracle::Connection) -> std::result::Result<(), oracle::Error> {
        conn.ping()
    }

    fn has_broken(&self, _conn: &mut oracle::Connection) -> bool {
        false
    }
}

/// Pool configuration mirroring Go's `pkg/oracle/Config`.
pub struct PoolConfig {
    /// Maximum number of open connections (`max_open_conn`).
    pub max_size: u32,
    /// Minimum idle connections kept alive after initialisation.
    ///
    /// **Set to 0 for lazy (on-demand) connection creation**, which matches
    /// Go's `database/sql` behaviour: `sql.Open` creates 0 connections; they
    /// are established only when the first query runs.
    ///
    /// r2d2's default is `None` which is silently treated as `max_size`,
    /// causing the full pool (e.g. 100 connections) to be created at startup
    /// and blocking the process for several seconds.
    pub min_idle: u32,
    /// Max time a connection may be reused (`max_life_time`, seconds).
    /// Mirrors Go's `db.SetConnMaxLifetime`.
    pub max_lifetime_secs: u64,
    /// Max time a connection may sit idle before being closed (`max_idle_time`, minutes).
    /// Mirrors Go's `db.SetConnMaxIdleTime`.
    pub max_idle_time_mins: u64,
    /// Timeout waiting for a connection from a fully-occupied pool (seconds).
    pub connection_timeout_secs: u64,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_size:               100,
            min_idle:               0,    // lazy — matches Go sql.Open
            max_lifetime_secs:      30,
            max_idle_time_mins:     30,
            connection_timeout_secs: 30,
        }
    }
}

/// Build an r2d2 connection pool for rust-oracle.
///
/// # Why `build_unchecked`
///
/// `r2d2::Pool::build()` internally calls `wait_for_initialization()`, which
/// blocks until `min_idle` connections have been established — even when
/// `min_idle = Some(0)` this still acquires a mutex and may yield to the
/// background replenishment thread.
///
/// `build_unchecked()` skips all of that: it constructs the pool struct,
/// starts the background management thread, and returns **immediately** with
/// zero blocking I/O.  This mirrors Go's `sql.Open()` which is also
/// non-blocking; actual connections are opened lazily on first use.
///
/// Connectivity is validated separately via [`ping_pool`] in a background
/// task (mirrors Go's `db.Ping()` after `sql.Open`).
pub fn build_pool(
    user: &str,
    password: &str,
    connect_string: &str,
    cfg: PoolConfig,
) -> OraclePool {
    let manager = OracleConnectionManager::new(user, password, connect_string);
    r2d2::Pool::builder()
        .max_size(cfg.max_size)
        .min_idle(Some(cfg.min_idle))
        .max_lifetime(Some(Duration::from_secs(cfg.max_lifetime_secs)))
        .idle_timeout(Some(Duration::from_secs(cfg.max_idle_time_mins * 60)))
        .connection_timeout(Duration::from_secs(cfg.connection_timeout_secs))
        // build_unchecked: no blocking wait — pool is ready instantly.
        // Connections are opened on the first query (lazy).
        .build_unchecked(manager)
}

/// Validate Oracle connectivity in a background task.
///
/// Mirrors Go's `db.Ping()` call after `sql.Open`.
/// Runs in `spawn_blocking` so it never blocks the async runtime.
/// Logs success or the error without crashing the application.
pub fn ping_pool(pool: Arc<OraclePool>) {
    tokio::task::spawn_blocking(move || {
        let start = std::time::Instant::now();
        match pool.get() {
            Ok(conn) => match conn.ping() {
                Ok(()) => tracing::info!(
                    elapsed_ms = start.elapsed().as_millis(),
                    "✅ Oracle connection pool: ping OK"
                ),
                Err(e) => tracing::warn!(error = %e, "Oracle ping failed"),
            },
            Err(e) => tracing::warn!(error = %e, "Oracle pool: could not get connection for ping"),
        }
    });
}

// ── Repository ────────────────────────────────────────────────────────────────

/// SELECT columns (includes TEMPLATE_FIELDS) used by all query methods.
const RULE_COLS_FULL: &str = "ID, IS_DEFAULT, MERCHANT_CODE, OPERATOR, \
                               IP_RETRY_LIMIT, ACCOUNT_RETRY_LIMIT, EMPTY_SCORE, \
                               LOCK_HOUR, BINDING_TYPE, PASSING_SCORE, \
                               QUESTIONS, TEMPLATE_FIELDS, CREATED_AT, UPDATED_AT";

#[derive(Clone)]
pub struct MerchantRuleRepository {
    pool:         Arc<OraclePool>,
    read_timeout: Duration,
}

impl MerchantRuleRepository {
    pub fn new(pool: Arc<OraclePool>) -> Self {
        Self::with_timeout(pool, 15)
    }

    pub fn with_timeout(pool: Arc<OraclePool>, read_timeout_secs: u64) -> Self {
        Self {
            pool,
            read_timeout: Duration::from_secs(read_timeout_secs),
        }
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    /// Map a result row (14 columns: RULE_COLS_FULL) into a [`MerchantRule`].
    fn map_full_row(row: oracle::Row) -> Result<MerchantRule> {
        Ok(MerchantRule {
            id:                   row.get::<_, i64>(0).context("ID")?,
            is_default:           row.get::<_, i32>(1).unwrap_or(0) as i8,
            merchant_code:        row.get::<_, String>(2).context("MERCHANT_CODE")?,
            operator:             row.get::<_, Option<String>>(3).unwrap_or_default().unwrap_or_default(),
            ip_retry_limit:       row.get::<_, i32>(4).context("IP_RETRY_LIMIT")?,
            account_retry_limit:  row.get::<_, i32>(5).context("ACCOUNT_RETRY_LIMIT")?,
            empty_score:          row.get::<_, i32>(6).context("EMPTY_SCORE")?,
            lock_hour:            row.get::<_, i32>(7).unwrap_or(0),
            binding_type:         row.get::<_, String>(8).context("BINDING_TYPE")?,
            passing_score:        row.get::<_, i32>(9).context("PASSING_SCORE")?,
            questions_json:       row.get::<_, Option<String>>(10).unwrap_or_default(),
            template_fields_json: row.get::<_, Option<String>>(11).unwrap_or_default(),
            created_at:           None,
            updated_at:           None,
        })
    }

    // ── Public query methods ──────────────────────────────────────────────────

    /// Find a merchant rule by exact merchant code.
    ///
    /// Returns `None` when no matching row exists.
    /// Mirrors Go's `FindByMerchantCode`.
    pub async fn find_by_merchant_code(
        &self,
        merchant_code: &str,
    ) -> Result<Option<MerchantRule>> {
        let pool    = self.pool.clone();
        let mc      = merchant_code.to_string();
        let timeout = self.read_timeout;

        let blocking = tokio::task::spawn_blocking(move || {
            let conn = pool.get().context("Oracle pool: get connection")?;
            let sql = format!(
                "SELECT {} FROM TCG_UCS.MERCHANT_RULE \
                 WHERE MERCHANT_CODE = :1 \
                 FETCH FIRST 1 ROWS ONLY",
                RULE_COLS_FULL
            );

            let rows = conn.query(&sql, &[&mc]).context("MerchantRule query")?;

            for row_result in rows {
                let row = row_result.context("MerchantRule row read")?;
                return Ok(Some(Self::map_full_row(row)?));
            }
            Ok(None)
        });

        tokio::time::timeout(timeout, blocking)
            .await
            .map_err(|_| anyhow!("find_by_merchant_code timed out after {:?}", timeout))?
            .context("spawn_blocking panicked")?
    }

    /// Find a merchant rule by merchant code and `IS_DEFAULT` flag.
    ///
    /// Mirrors Go's `FindByMerchantCodeAndDefault`.
    pub async fn find_by_merchant_code_and_default(
        &self,
        merchant_code: &str,
        is_default: i32,
    ) -> Result<Option<MerchantRule>> {
        let pool    = self.pool.clone();
        let mc      = merchant_code.to_string();
        let timeout = self.read_timeout;

        let blocking = tokio::task::spawn_blocking(move || {
            let conn = pool.get().context("Oracle pool: get connection")?;
            let sql = format!(
                "SELECT {} FROM TCG_UCS.MERCHANT_RULE \
                 WHERE MERCHANT_CODE = :1 AND IS_DEFAULT = :2 \
                 FETCH FIRST 1 ROWS ONLY",
                RULE_COLS_FULL
            );

            let rows = conn
                .query(&sql, &[&mc, &is_default])
                .context("MerchantRule+default query")?;

            for row_result in rows {
                let row = row_result.context("MerchantRule row read")?;
                return Ok(Some(Self::map_full_row(row)?));
            }
            Ok(None)
        });

        tokio::time::timeout(timeout, blocking)
            .await
            .map_err(|_| anyhow!("find_by_merchant_code_and_default timed out after {:?}", timeout))?
            .context("spawn_blocking panicked")?
    }

    /// Slim version: returns only the fields required by the verification flow.
    ///
    /// Mirrors Go's `GetRuleConfigByMerchantCode`.
    pub async fn get_rule_config(
        &self,
        merchant_code: &str,
    ) -> Result<Option<MerchantRuleConfig>> {
        let rule = self.find_by_merchant_code(merchant_code).await?;
        Ok(rule.map(|r| MerchantRuleConfig {
            id:                  r.id,
            merchant_code:       r.merchant_code,
            binding_type:        r.binding_type,
            passing_score:       r.passing_score,
            empty_score:         r.empty_score,
            lock_hour:           r.lock_hour,
            ip_retry_limit:      r.ip_retry_limit,
            account_retry_limit: r.account_retry_limit,
            // QUESTIONS CLOB is a JSON *object* keyed by fieldId — NOT an array.
            questions: r
                .questions_json
                .as_deref()
                .and_then(|j| {
                    serde_json::from_str::<std::collections::HashMap<String, Question>>(j)
                        .map_err(|e| tracing::warn!(error = %e, "get_rule_config: parse questions_json failed"))
                        .ok()
                        .map(|m| m.into_values().collect::<Vec<_>>())
                })
                .unwrap_or_default(),
        }))
    }

    /// Load all merchant rules and build the field-config dropdown map.
    ///
    /// Only fields with `fieldAttribute == "DD"` (dropdown) are included —
    /// mirrors Go's `FindAllTemplateFieldsAsMap` filter logic.
    ///
    /// Returns: `HashMap<merchantCode, HashMap<fieldId, Vec<DropdownItem>>>`
    pub async fn find_all_as_map(
        &self,
    ) -> Result<HashMap<String, HashMap<String, Vec<DropdownItem>>>> {
        let pool    = self.pool.clone();
        let timeout = self.read_timeout;

        let blocking = tokio::task::spawn_blocking(move || {
            let conn = pool.get().context("Oracle pool: get connection")?;

            // Read only the two columns we need — avoid fetching heavy CLOBs.
            let sql = "SELECT MERCHANT_CODE, TEMPLATE_FIELDS \
                       FROM TCG_UCS.MERCHANT_RULE \
                       WHERE TEMPLATE_FIELDS IS NOT NULL";

            let rows = conn
                .query(sql, &[])
                .context("find_all_as_map query")?;

            let mut result: HashMap<String, HashMap<String, Vec<DropdownItem>>> = HashMap::new();

            for row_result in rows {
                let row = row_result.context("find_all_as_map row read")?;
                let merchant_code: String        = row.get(0).context("MERCHANT_CODE")?;
                let tf_json:       Option<String> = row.get(1).unwrap_or_default();

                let json = match tf_json {
                    Some(j) if !j.is_empty() => j,
                    _ => continue,
                };

                let fields: Vec<TemplateField> = match serde_json::from_str(&json) {
                    Ok(f)  => f,
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
                    // Only include dropdown-type fields (Go: field.FieldAttribute == "DD").
                    if f.field_attribute == "DD"
                        && !f.field_id.is_empty()
                        && !f.dropdown_list.is_empty()
                    {
                        field_map.insert(f.field_id, f.dropdown_list);
                    }
                }

                if !field_map.is_empty() {
                    result.insert(merchant_code, field_map);
                }
            }

            tracing::info!(
                total_merchants = result.len(),
                "find_all_as_map: merchant field-config map loaded"
            );
            Ok(result)
        });

        tokio::time::timeout(timeout, blocking)
            .await
            .map_err(|_| anyhow!("find_all_as_map timed out after {:?}", timeout))?
            .context("spawn_blocking panicked")?
    }

    /// Update `TEMPLATE_FIELDS` for a merchant.
    ///
    /// Mirrors Go's `UpdateTemplateFieldsByMerchantCode`.
    pub async fn update_template_fields(
        &self,
        merchant_code: &str,
        template_fields_json: &str,
    ) -> Result<u64> {
        let pool         = self.pool.clone();
        let mc           = merchant_code.to_string();
        let fields_json  = template_fields_json.to_string();
        let timeout      = self.read_timeout;

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
                return Err(anyhow!("UpdateTemplateFields: no row for merchant_code={}", mc));
            }
            conn.commit().context("commit UpdateTemplateFields")?;
            Ok(rows_affected)
        });

        tokio::time::timeout(timeout, blocking)
            .await
            .map_err(|_| anyhow!("update_template_fields timed out after {:?}", timeout))?
            .context("spawn_blocking panicked")?
    }
    // ── Additional CRUD mirrors ───────────────────────────────────────────────

    /// Map an Oracle result row (using RULE_COLS_FULL column order) to `MerchantRule`.
    ///
    /// Column indices (0-based):
    ///   0:ID 1:IS_DEFAULT 2:MERCHANT_CODE 3:OPERATOR
    ///   4:IP_RETRY_LIMIT 5:ACCOUNT_RETRY_LIMIT 6:EMPTY_SCORE
    ///   7:LOCK_HOUR 8:BINDING_TYPE 9:PASSING_SCORE
    ///   10:QUESTIONS 11:TEMPLATE_FIELDS 12:CREATED_AT 13:UPDATED_AT
    fn map_rule_row(row: oracle::Row) -> anyhow::Result<MerchantRule> {
        Ok(MerchantRule {
            id:                   row.get::<_, i64>(0).context("ID")?,
            is_default:           row.get::<_, i32>(1).unwrap_or(0) as i8,
            merchant_code:        row.get::<_, String>(2).context("MERCHANT_CODE")?,
            operator:             row.get::<_, String>(3).unwrap_or_default(),
            ip_retry_limit:       row.get::<_, i32>(4).unwrap_or(0),
            account_retry_limit:  row.get::<_, i32>(5).unwrap_or(0),
            empty_score:          row.get::<_, i32>(6).unwrap_or(0),
            lock_hour:            row.get::<_, i32>(7).unwrap_or(0),
            binding_type:         row.get::<_, String>(8).unwrap_or_default(),
            passing_score:        row.get::<_, i32>(9).unwrap_or(0),
            questions_json:       row.get::<_, Option<String>>(10).unwrap_or_default(),
            template_fields_json: row.get::<_, Option<String>>(11).unwrap_or_default(),
            created_at:           row.get::<_, Option<chrono::NaiveDateTime>>(12)
                .unwrap_or_default()
                .map(|ndt| chrono::DateTime::from_naive_utc_and_offset(ndt, chrono::Utc)),
            updated_at:           row.get::<_, Option<chrono::NaiveDateTime>>(13)
                .unwrap_or_default()
                .map(|ndt| chrono::DateTime::from_naive_utc_and_offset(ndt, chrono::Utc)),
        })
    }

    /// Find a single merchant rule by primary key.  Mirrors Go's `FindOne`.
    pub async fn find_one(&self, id: i64) -> Result<Option<MerchantRule>> {
        let pool    = self.pool.clone();
        let timeout = self.read_timeout;

        let blocking = tokio::task::spawn_blocking(move || {
            let conn = pool.get().context("Oracle pool: get connection")?;
            let sql  = format!(
                "SELECT {} FROM TCG_UCS.MERCHANT_RULE WHERE ID = :1",
                RULE_COLS_FULL
            );
            let rows = conn.query(&sql, &[&id]).context("FindOne query")?;
            for row_result in rows {
                return Ok(Some(Self::map_rule_row(row_result.context("FindOne row")?)?));
            }
            Ok(None)
        });

        tokio::time::timeout(timeout, blocking)
            .await
            .map_err(|_| anyhow!("find_one timed out"))?
            .context("spawn_blocking panicked")?
    }

    /// Return all MERCHANT_CODE values in the table.  Mirrors Go's `GetAllMerchantCodes`.
    pub async fn get_all_merchant_codes(&self) -> Result<Vec<String>> {
        let pool    = self.pool.clone();
        let timeout = self.read_timeout;

        let blocking = tokio::task::spawn_blocking(move || {
            let conn = pool.get().context("Oracle pool: get connection")?;
            let rows = conn.query("SELECT MERCHANT_CODE FROM TCG_UCS.MERCHANT_RULE", &[])
                .context("GetAllMerchantCodes query")?;
            let mut out = Vec::new();
            for row_result in rows {
                let row:  oracle::Row = row_result.context("row")?;
                let code: String      = row.get(0).context("MERCHANT_CODE")?;
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
    /// Fetches the next sequence value first, then inserts — mirrors Go's `Insert`.
    pub async fn insert(&self, rule: MerchantRule) -> Result<i64> {
        let pool    = self.pool.clone();
        let timeout = self.read_timeout;

        let blocking = tokio::task::spawn_blocking(move || {
            let conn   = pool.get().context("Oracle pool: get connection")?;
            let new_id = conn
                .query_row_as::<i64>("SELECT SEQ_MERCHANT_RULE.NEXTVAL FROM DUAL", &[])
                .context("SEQ_MERCHANT_RULE.NEXTVAL")?;

            let sql = "INSERT INTO TCG_UCS.MERCHANT_RULE \
                        (ID, IS_DEFAULT, MERCHANT_CODE, OPERATOR, \
                         IP_RETRY_LIMIT, ACCOUNT_RETRY_LIMIT, EMPTY_SCORE, \
                         LOCK_HOUR, BINDING_TYPE, PASSING_SCORE, QUESTIONS, \
                         CREATED_AT, UPDATED_AT) \
                       VALUES \
                        (:1, :2, :3, :4, :5, :6, :7, :8, :9, :10, :11, \
                         SYSTIMESTAMP, SYSTIMESTAMP)";

            conn.execute(sql, &[
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
            ]).context("MerchantRule INSERT execute")?;

            conn.commit().context("MerchantRule INSERT commit")?;
            Ok(new_id)
        });

        tokio::time::timeout(timeout, blocking)
            .await
            .map_err(|_| anyhow!("merchant_rule insert timed out"))?
            .context("spawn_blocking panicked")?
    }

    /// Full UPDATE of a merchant rule row by primary key.  Mirrors Go's `Update`.
    pub async fn update(&self, rule: MerchantRule) -> Result<u64> {
        let id      = rule.id;
        let pool    = self.pool.clone();
        let timeout = self.read_timeout;

        let blocking = tokio::task::spawn_blocking(move || {
            let conn = pool.get().context("Oracle pool: get connection")?;
            let sql  = "UPDATE TCG_UCS.MERCHANT_RULE SET \
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
                            UPDATED_AT          = SYSTIMESTAMP \
                        WHERE ID = :11";

            let stmt = conn.execute(sql, &[
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
                &id,
            ]).context("MerchantRule UPDATE execute")?;

            let rows = stmt.row_count().context("row_count")?;
            if rows == 0 {
                return Err(anyhow!("MerchantRule UPDATE: no row for id={}", id));
            }
            conn.commit().context("MerchantRule UPDATE commit")?;
            Ok(rows)
        });

        tokio::time::timeout(timeout, blocking)
            .await
            .map_err(|_| anyhow!("merchant_rule update timed out"))?
            .context("spawn_blocking panicked")?
    }

    /// Delete a merchant rule row by primary key.  Mirrors Go's `Delete`.
    pub async fn delete(&self, id: i64) -> Result<u64> {
        let pool    = self.pool.clone();
        let timeout = self.read_timeout;

        let blocking = tokio::task::spawn_blocking(move || {
            let conn = pool.get().context("Oracle pool: get connection")?;
            let stmt = conn
                .execute("DELETE FROM TCG_UCS.MERCHANT_RULE WHERE ID = :1", &[&id])
                .context("MerchantRule DELETE execute")?;
            let rows = stmt.row_count().context("row_count")?;
            if rows == 0 {
                return Err(anyhow!("MerchantRule DELETE: no row for id={}", id));
            }
            conn.commit().context("MerchantRule DELETE commit")?;
            Ok(rows)
        });

        tokio::time::timeout(timeout, blocking)
            .await
            .map_err(|_| anyhow!("merchant_rule delete timed out"))?
            .context("spawn_blocking panicked")?
    }

}