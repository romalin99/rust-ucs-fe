/// Oracle repository for TCG_UCS.VALIDATION_RECORD.
///
/// Uses the `oracle` crate for direct Oracle Database access via an `r2d2`
/// connection pool.  All Oracle calls are wrapped in `tokio::task::spawn_blocking`
/// because `oracle::Connection` is sync-only.
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Timelike, Utc};
use serde::{Deserialize, Serialize};

use crate::model::validation_record::ValidationRecord;
use crate::repository::merchant_rule::OraclePool;

// ── Analytics result structs ──────────────────────────────────────────────────

/// Aggregated validation statistics per customer and merchant.
///
/// Mirrors Go's `ValidationSummary` in `internal/repository/validation_record.go`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationSummary {
    pub last_created:  Option<DateTime<Utc>>,
    pub merchant_code: String,
    pub customer_id:   i64,
    pub total_count:   i64,
    pub fail_count:    i64,
    pub success_count: i64,
}

/// Anti-fraud statistics per IP address.
///
/// Mirrors Go's `ValidationIpStats`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationIpStats {
    pub first_seen:  Option<DateTime<Utc>>,
    pub last_seen:   Option<DateTime<Utc>>,
    pub ip:          String,
    pub total_count: i64,
    pub fail_count:  i64,
}

/// Per-minute validation count for time-series dashboards.
///
/// Mirrors Go's `ValidationMinuteStat`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationMinuteStat {
    pub tx_minute: DateTime<Utc>,
    pub count:     i64,
}

// ── Repository ────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct ValidationRecordRepository {
    pool:          Arc<OraclePool>,
    read_timeout:  Duration,
    write_timeout: Duration,
}

impl ValidationRecordRepository {
    pub fn new(pool: Arc<OraclePool>) -> Self {
        Self {
            pool,
            read_timeout:  Duration::from_secs(15),
            write_timeout: Duration::from_secs(10),
        }
    }

    // ── Write operations ──────────────────────────────────────────────────────

    /// Upsert a validation record using Oracle MERGE.
    ///
    /// Finds the most recent record for `(customer_id, merchant_code)`:
    /// • if found → UPDATE that row in-place
    /// • if not   → INSERT a new row
    ///
    /// Mirrors Go's `UpsertWithContext`.
    pub async fn upsert(&self, record: ValidationRecord) -> Result<()> {
        let pool    = self.pool.clone();
        let timeout = self.write_timeout;

        let blocking = tokio::task::spawn_blocking(move || {
            let conn = pool.get().context("Oracle pool: get connection")?;

            let sql = "MERGE INTO TCG_UCS.VALIDATION_RECORD tgt \
                       USING ( \
                           SELECT \
                               NVL( \
                                   (SELECT ID \
                                    FROM   TCG_UCS.VALIDATION_RECORD \
                                    WHERE  CUSTOMER_ID   = :1 \
                                      AND  MERCHANT_CODE = :2 \
                                    ORDER BY CREATED_AT DESC \
                                    FETCH FIRST 1 ROWS ONLY), \
                                   -1 \
                               )   AS MATCH_ID, \
                               :3  AS P_CUSTOMER_ID, \
                               :4  AS P_CUSTOMER_NAME, \
                               :5  AS P_SUCCESS, \
                               :6  AS P_MERCHANT_CODE, \
                               :7  AS P_IP, \
                               :8  AS P_PASSING_SCORE, \
                               :9  AS P_SCORE, \
                               :10 AS P_QAS \
                           FROM DUAL \
                       ) src ON (tgt.ID = src.MATCH_ID) \
                       WHEN MATCHED THEN \
                           UPDATE SET \
                               tgt.CUSTOMER_NAME = src.P_CUSTOMER_NAME, \
                               tgt.SUCCESS       = src.P_SUCCESS, \
                               tgt.IP            = src.P_IP, \
                               tgt.PASSING_SCORE = src.P_PASSING_SCORE, \
                               tgt.SCORE         = src.P_SCORE, \
                               tgt.QAS           = src.P_QAS, \
                               tgt.CREATED_AT    = SYSTIMESTAMP \
                       WHEN NOT MATCHED THEN \
                           INSERT (CUSTOMER_ID,         CUSTOMER_NAME,         SUCCESS, \
                                   MERCHANT_CODE,       IP,                    PASSING_SCORE, \
                                   SCORE,               QAS,                   CREATED_AT) \
                           VALUES (src.P_CUSTOMER_ID,   src.P_CUSTOMER_NAME,   src.P_SUCCESS, \
                                   src.P_MERCHANT_CODE, src.P_IP,              src.P_PASSING_SCORE, \
                                   src.P_SCORE,         src.P_QAS,             SYSTIMESTAMP)";

            let success = record.success as i32;
            conn.execute(sql, &[
                &record.customer_id,   // :1  WHERE CUSTOMER_ID
                &record.merchant_code, // :2  WHERE MERCHANT_CODE
                &record.customer_id,   // :3  P_CUSTOMER_ID
                &record.customer_name, // :4  P_CUSTOMER_NAME
                &success,              // :5  P_SUCCESS
                &record.merchant_code, // :6  P_MERCHANT_CODE
                &record.ip,            // :7  P_IP
                &record.passing_score, // :8  P_PASSING_SCORE
                &record.score,         // :9  P_SCORE
                &record.qas,           // :10 P_QAS
            ]).context("ValidationRecord MERGE execute")?;

            conn.commit().context("ValidationRecord MERGE commit")?;
            Ok(())
        });

        tokio::time::timeout(timeout, blocking)
            .await
            .map_err(|_| anyhow!("upsert timed out after {:?}", timeout))?
            .context("spawn_blocking panicked in upsert")?
    }

    /// Simple INSERT — use when you always want a new row (no dedup).
    ///
    /// Mirrors Go's `InsertWithContext`.
    pub async fn insert(&self, record: ValidationRecord) -> Result<()> {
        let pool    = self.pool.clone();
        let timeout = self.write_timeout;

        let blocking = tokio::task::spawn_blocking(move || {
            let conn    = pool.get().context("Oracle pool: get connection")?;
            let success = record.success as i32;

            let sql = "INSERT INTO TCG_UCS.VALIDATION_RECORD \
                        (CUSTOMER_ID, CUSTOMER_NAME, SUCCESS, MERCHANT_CODE, IP, \
                         PASSING_SCORE, SCORE, QAS, CREATED_AT) \
                       VALUES (:1, :2, :3, :4, :5, :6, :7, :8, SYSTIMESTAMP)";

            conn.execute(sql, &[
                &record.customer_id,
                &record.customer_name,
                &success,
                &record.merchant_code,
                &record.ip,
                &record.passing_score,
                &record.score,
                &record.qas,
            ]).context("ValidationRecord INSERT execute")?;

            conn.commit().context("ValidationRecord INSERT commit")?;
            Ok(())
        });

        tokio::time::timeout(timeout, blocking)
            .await
            .map_err(|_| anyhow!("insert timed out after {:?}", timeout))?
            .context("spawn_blocking panicked in insert")?
    }

    // ── Read — single record ──────────────────────────────────────────────────

    /// Find the most recent validation record for a customer+merchant pair.
    ///
    /// Mirrors Go's `FindLatestByCustomerAndMerchant`.
    pub async fn find_latest_by_customer_and_merchant(
        &self,
        customer_id:   i64,
        merchant_code: &str,
    ) -> Result<Option<ValidationRecord>> {
        let pool    = self.pool.clone();
        let mc      = merchant_code.to_string();
        let timeout = self.read_timeout;

        let blocking = tokio::task::spawn_blocking(move || {
            let conn = pool.get().context("Oracle pool: get connection")?;

            let sql = "SELECT ID, CUSTOMER_ID, CUSTOMER_NAME, SUCCESS, MERCHANT_CODE, \
                              IP, PASSING_SCORE, SCORE, QAS \
                       FROM TCG_UCS.VALIDATION_RECORD \
                       WHERE CUSTOMER_ID = :1 AND MERCHANT_CODE = :2 \
                       ORDER BY CREATED_AT DESC \
                       FETCH FIRST 1 ROWS ONLY";

            let rows = conn.query(sql, &[&customer_id, &mc]).context("FindLatest query")?;
            for row_result in rows {
                let row = row_result.context("FindLatest row read")?;
                let rec = ValidationRecord {
                    id:            row.get::<_, Option<i64>>(0).unwrap_or_default(),
                    customer_id:   row.get::<_, i64>(1).context("CUSTOMER_ID")?,
                    customer_name: row.get::<_, String>(2).context("CUSTOMER_NAME")?,
                    success:       row.get::<_, i32>(3).unwrap_or(0) as i8,
                    merchant_code: row.get::<_, String>(4).context("MERCHANT_CODE")?,
                    ip:            row.get::<_, String>(5).context("IP")?,
                    passing_score: row.get::<_, i32>(6).context("PASSING_SCORE")?,
                    score:         row.get::<_, i32>(7).context("SCORE")?,
                    qas:           row.get::<_, String>(8).context("QAS")?,
                    created_at:    Utc::now(),
                };
                return Ok(Some(rec));
            }
            Ok(None)
        });

        tokio::time::timeout(timeout, blocking)
            .await
            .map_err(|_| anyhow!("find_latest timed out after {:?}", timeout))?
            .context("spawn_blocking panicked in find_latest")?
    }

    // ── Read — list queries ───────────────────────────────────────────────────

    fn map_row(row: oracle::Row) -> Result<ValidationRecord> {
        Ok(ValidationRecord {
            id:            row.get::<_, Option<i64>>(0).unwrap_or_default(),
            customer_id:   row.get::<_, i64>(1).context("CUSTOMER_ID")?,
            customer_name: row.get::<_, String>(2).context("CUSTOMER_NAME")?,
            success:       row.get::<_, i32>(3).unwrap_or(0) as i8,
            merchant_code: row.get::<_, String>(4).context("MERCHANT_CODE")?,
            ip:            row.get::<_, String>(5).context("IP")?,
            passing_score: row.get::<_, i32>(6).context("PASSING_SCORE")?,
            score:         row.get::<_, i32>(7).context("SCORE")?,
            qas:           row.get::<_, String>(8).context("QAS")?,
            created_at:    Utc::now(),
        })
    }

    /// All validation records for a given `customer_id`.
    ///
    /// Mirrors Go's `FindListByCustomerID`.
    pub async fn find_list_by_customer_id(&self, customer_id: i64) -> Result<Vec<ValidationRecord>> {
        let pool    = self.pool.clone();
        let timeout = self.read_timeout;

        let blocking = tokio::task::spawn_blocking(move || {
            let conn = pool.get().context("Oracle pool: get connection")?;
            let sql  = "SELECT ID, CUSTOMER_ID, CUSTOMER_NAME, SUCCESS, MERCHANT_CODE, \
                               IP, PASSING_SCORE, SCORE, QAS \
                        FROM TCG_UCS.VALIDATION_RECORD \
                        WHERE CUSTOMER_ID = :1 \
                        ORDER BY CREATED_AT DESC";

            let rows = conn.query(sql, &[&customer_id]).context("FindListByCustomerId query")?;
            let mut out = Vec::new();
            for r in rows { out.push(Self::map_row(r.context("row read")?)?); }
            Ok(out)
        });

        tokio::time::timeout(timeout, blocking)
            .await
            .map_err(|_| anyhow!("find_list_by_customer_id timed out"))?
            .context("spawn_blocking panicked")?
    }

    /// All validation records for a given `merchant_code`.
    ///
    /// Mirrors Go's `FindListByMerchantCode`.
    pub async fn find_list_by_merchant_code(&self, merchant_code: &str) -> Result<Vec<ValidationRecord>> {
        let pool    = self.pool.clone();
        let mc      = merchant_code.to_string();
        let timeout = self.read_timeout;

        let blocking = tokio::task::spawn_blocking(move || {
            let conn = pool.get().context("Oracle pool: get connection")?;
            let sql  = "SELECT ID, CUSTOMER_ID, CUSTOMER_NAME, SUCCESS, MERCHANT_CODE, \
                               IP, PASSING_SCORE, SCORE, QAS \
                        FROM TCG_UCS.VALIDATION_RECORD \
                        WHERE MERCHANT_CODE = :1 \
                        ORDER BY CREATED_AT DESC";

            let rows = conn.query(sql, &[&mc]).context("FindListByMerchantCode query")?;
            let mut out = Vec::new();
            for r in rows { out.push(Self::map_row(r.context("row read")?)?); }
            Ok(out)
        });

        tokio::time::timeout(timeout, blocking)
            .await
            .map_err(|_| anyhow!("find_list_by_merchant_code timed out"))?
            .context("spawn_blocking panicked")?
    }

    /// All validation records for a given `(customer_id, merchant_code)` pair.
    ///
    /// Mirrors Go's `FindListByCustomerAndMerchant`.
    pub async fn find_list_by_customer_and_merchant(
        &self,
        customer_id:   i64,
        merchant_code: &str,
    ) -> Result<Vec<ValidationRecord>> {
        let pool    = self.pool.clone();
        let mc      = merchant_code.to_string();
        let timeout = self.read_timeout;

        let blocking = tokio::task::spawn_blocking(move || {
            let conn = pool.get().context("Oracle pool: get connection")?;
            let sql  = "SELECT ID, CUSTOMER_ID, CUSTOMER_NAME, SUCCESS, MERCHANT_CODE, \
                               IP, PASSING_SCORE, SCORE, QAS \
                        FROM TCG_UCS.VALIDATION_RECORD \
                        WHERE CUSTOMER_ID = :1 AND MERCHANT_CODE = :2 \
                        ORDER BY CREATED_AT DESC";

            let rows = conn.query(sql, &[&customer_id, &mc])
                .context("FindListByCustomerAndMerchant query")?;
            let mut out = Vec::new();
            for r in rows { out.push(Self::map_row(r.context("row read")?)?); }
            Ok(out)
        });

        tokio::time::timeout(timeout, blocking)
            .await
            .map_err(|_| anyhow!("find_list_by_customer_and_merchant timed out"))?
            .context("spawn_blocking panicked")?
    }

    /// All validation records for a given `ip` address.
    ///
    /// Mirrors Go's `FindListByIp`.
    pub async fn find_list_by_ip(&self, ip: &str) -> Result<Vec<ValidationRecord>> {
        let pool    = self.pool.clone();
        let ip_str  = ip.to_string();
        let timeout = self.read_timeout;

        let blocking = tokio::task::spawn_blocking(move || {
            let conn = pool.get().context("Oracle pool: get connection")?;
            let sql  = "SELECT ID, CUSTOMER_ID, CUSTOMER_NAME, SUCCESS, MERCHANT_CODE, \
                               IP, PASSING_SCORE, SCORE, QAS \
                        FROM TCG_UCS.VALIDATION_RECORD \
                        WHERE IP = :1 \
                        ORDER BY CREATED_AT DESC";

            let rows = conn.query(sql, &[&ip_str]).context("FindListByIp query")?;
            let mut out = Vec::new();
            for r in rows { out.push(Self::map_row(r.context("row read")?)?); }
            Ok(out)
        });

        tokio::time::timeout(timeout, blocking)
            .await
            .map_err(|_| anyhow!("find_list_by_ip timed out"))?
            .context("spawn_blocking panicked")?
    }

    // ── Aggregation / analytics ───────────────────────────────────────────────

    /// Count failed attempts for a customer+merchant since a given timestamp.
    ///
    /// Mirrors Go's `CountFailByCustomerAndMerchantSince`.
    pub async fn count_fail_since(
        &self,
        customer_id:   i64,
        merchant_code: &str,
        since:         DateTime<Utc>,
    ) -> Result<i64> {
        let pool      = self.pool.clone();
        let mc        = merchant_code.to_string();
        let timeout   = self.read_timeout;
        let since_str = since.format("%Y-%m-%d %H:%M:%S").to_string();

        let blocking = tokio::task::spawn_blocking(move || {
            let conn = pool.get().context("Oracle pool: get connection")?;
            let sql  = "SELECT COUNT(1) \
                        FROM TCG_UCS.VALIDATION_RECORD \
                        WHERE CUSTOMER_ID   = :1 \
                          AND MERCHANT_CODE = :2 \
                          AND SUCCESS       = 0 \
                          AND CREATED_AT   >= TO_TIMESTAMP(:3, 'YYYY-MM-DD HH24:MI:SS')";

            let rows = conn.query(sql, &[&customer_id, &mc, &since_str])
                .context("CountFailSince query")?;
            for row_result in rows {
                let row = row_result.context("CountFailSince row read")?;
                let cnt: i64 = row.get(0).context("COUNT")?;
                return Ok(cnt);
            }
            Ok(0)
        });

        tokio::time::timeout(timeout, blocking)
            .await
            .map_err(|_| anyhow!("count_fail_since timed out after {:?}", timeout))?
            .context("spawn_blocking panicked in count_fail_since")?
    }

    /// Aggregated statistics (total / fail / success / last_created) for a
    /// customer+merchant pair.
    ///
    /// Mirrors Go's `GetSummaryByCustomerAndMerchant`.
    pub async fn get_summary_by_customer_and_merchant(
        &self,
        customer_id:   i64,
        merchant_code: &str,
    ) -> Result<Option<ValidationSummary>> {
        let pool    = self.pool.clone();
        let mc      = merchant_code.to_string();
        let timeout = self.read_timeout;

        let blocking = tokio::task::spawn_blocking(move || {
            let conn = pool.get().context("Oracle pool: get connection")?;

            let sql = "SELECT CUSTOMER_ID, MERCHANT_CODE, \
                              COUNT(1)                                       AS TOTAL_COUNT, \
                              SUM(CASE WHEN SUCCESS = 0 THEN 1 ELSE 0 END)  AS FAIL_COUNT, \
                              SUM(CASE WHEN SUCCESS = 1 THEN 1 ELSE 0 END)  AS SUCCESS_COUNT, \
                              MAX(CREATED_AT)                                AS LAST_CREATED \
                       FROM TCG_UCS.VALIDATION_RECORD \
                       WHERE CUSTOMER_ID = :1 AND MERCHANT_CODE = :2 \
                       GROUP BY CUSTOMER_ID, MERCHANT_CODE";

            let rows = conn.query(sql, &[&customer_id, &mc])
                .context("GetSummary query")?;
            for row_result in rows {
                let row = row_result.context("GetSummary row read")?;
                let summary = ValidationSummary {
                    customer_id:   row.get::<_, i64>(0).context("CUSTOMER_ID")?,
                    merchant_code: row.get::<_, String>(1).context("MERCHANT_CODE")?,
                    total_count:   row.get::<_, i64>(2).context("TOTAL_COUNT")?,
                    fail_count:    row.get::<_, i64>(3).context("FAIL_COUNT")?,
                    success_count: row.get::<_, i64>(4).context("SUCCESS_COUNT")?,
                    last_created:  row.get::<_, Option<chrono::NaiveDateTime>>(5)
                        .unwrap_or_default()
                        .map(|ndt| DateTime::from_naive_utc_and_offset(ndt, Utc)),
                };
                return Ok(Some(summary));
            }
            Ok(None)
        });

        tokio::time::timeout(timeout, blocking)
            .await
            .map_err(|_| anyhow!("get_summary timed out after {:?}", timeout))?
            .context("spawn_blocking panicked in get_summary")?
    }

    /// Anti-fraud statistics for an IP address since the specified time.
    ///
    /// Mirrors Go's `GetIpStats`.
    pub async fn get_ip_stats(
        &self,
        ip:    &str,
        since: DateTime<Utc>,
    ) -> Result<Option<ValidationIpStats>> {
        let pool      = self.pool.clone();
        let ip_str    = ip.to_string();
        let timeout   = self.read_timeout;
        let since_str = since.format("%Y-%m-%d %H:%M:%S").to_string();

        let blocking = tokio::task::spawn_blocking(move || {
            let conn = pool.get().context("Oracle pool: get connection")?;

            let sql = "SELECT IP, \
                              COUNT(1)                                       AS TOTAL_COUNT, \
                              SUM(CASE WHEN SUCCESS = 0 THEN 1 ELSE 0 END)  AS FAIL_COUNT, \
                              MIN(CREATED_AT)                                AS FIRST_SEEN, \
                              MAX(CREATED_AT)                                AS LAST_SEEN \
                       FROM TCG_UCS.VALIDATION_RECORD \
                       WHERE IP = :1 \
                         AND CREATED_AT >= TO_TIMESTAMP(:2, 'YYYY-MM-DD HH24:MI:SS') \
                       GROUP BY IP";

            let rows = conn.query(sql, &[&ip_str, &since_str])
                .context("GetIpStats query")?;
            for row_result in rows {
                let row   = row_result.context("GetIpStats row read")?;
                let stats = ValidationIpStats {
                    ip:          row.get::<_, String>(0).context("IP")?,
                    total_count: row.get::<_, i64>(1).context("TOTAL_COUNT")?,
                    fail_count:  row.get::<_, i64>(2).context("FAIL_COUNT")?,
                    first_seen:  row.get::<_, Option<chrono::NaiveDateTime>>(3)
                        .unwrap_or_default()
                        .map(|ndt| DateTime::from_naive_utc_and_offset(ndt, Utc)),
                    last_seen:   row.get::<_, Option<chrono::NaiveDateTime>>(4)
                        .unwrap_or_default()
                        .map(|ndt| DateTime::from_naive_utc_and_offset(ndt, Utc)),
                };
                return Ok(Some(stats));
            }
            Ok(None)
        });

        tokio::time::timeout(timeout, blocking)
            .await
            .map_err(|_| anyhow!("get_ip_stats timed out after {:?}", timeout))?
            .context("spawn_blocking panicked in get_ip_stats")?
    }

    /// Per-minute validation counts within a time range, with zero-fill for
    /// minutes that have no activity.
    ///
    /// Mirrors Go's `GetCountByMinute` (including the minute-gap fill).
    pub async fn get_count_by_minute(
        &self,
        start_time: DateTime<Utc>,
        end_time:   DateTime<Utc>,
    ) -> Result<Vec<ValidationMinuteStat>> {
        let pool        = self.pool.clone();
        let timeout     = Duration::from_secs(180);
        let start_str   = start_time.format("%Y-%m-%d %H:%M:%S").to_string();
        let end_str     = end_time.format("%Y-%m-%d %H:%M:%S").to_string();

        let blocking = tokio::task::spawn_blocking(move || {
            let conn = pool.get().context("Oracle pool: get connection")?;

            let sql = "SELECT TRUNC(CREATED_AT, 'MI') AS TX_MINUTE, COUNT(1) AS CNT \
                       FROM TCG_UCS.VALIDATION_RECORD \
                       WHERE CREATED_AT >= TO_TIMESTAMP(:1, 'YYYY-MM-DD HH24:MI:SS') \
                         AND CREATED_AT <  TO_TIMESTAMP(:2, 'YYYY-MM-DD HH24:MI:SS') \
                       GROUP BY TRUNC(CREATED_AT, 'MI') \
                       ORDER BY TX_MINUTE ASC";

            let rows = conn.query(sql, &[&start_str, &end_str])
                .context("GetCountByMinute query")?;

            let mut count_map: std::collections::HashMap<chrono::NaiveDateTime, i64> =
                std::collections::HashMap::new();

            for row_result in rows {
                let row    = row_result.context("GetCountByMinute row read")?;
                let minute = row.get::<_, chrono::NaiveDateTime>(0).context("TX_MINUTE")?;
                let count  = row.get::<_, i64>(1).context("CNT")?;
                count_map.insert(minute, count);
            }

            // Fill zero-count minutes between start and end — mirrors Go's gap fill.
            let mut stats = Vec::new();
            let start_naive = start_time.naive_utc();
            let end_naive   = end_time.naive_utc();
            let mut cur = chrono::NaiveDateTime::new(
                start_naive.date(),
                chrono::NaiveTime::from_hms_opt(
                    start_naive.hour(),
                    start_naive.minute(),
                    0,
                ).unwrap(),
            );
            let end_trunc = chrono::NaiveDateTime::new(
                end_naive.date(),
                chrono::NaiveTime::from_hms_opt(
                    end_naive.hour(),
                    end_naive.minute(),
                    0,
                ).unwrap(),
            );

            while cur <= end_trunc {
                let count = *count_map.get(&cur).unwrap_or(&0);
                stats.push(ValidationMinuteStat {
                    tx_minute: DateTime::from_naive_utc_and_offset(cur, Utc),
                    count,
                });
                cur += chrono::Duration::minutes(1);
            }
            Ok(stats)
        });

        tokio::time::timeout(timeout, blocking)
            .await
            .map_err(|_| anyhow!("get_count_by_minute timed out after {:?}", timeout))?
            .context("spawn_blocking panicked in get_count_by_minute")?
    }

    /// Total validation count within a time range.
    ///
    /// Mirrors Go's `GetCountByTimeRange`.
    pub async fn get_count_by_time_range(
        &self,
        start_time: DateTime<Utc>,
        end_time:   DateTime<Utc>,
    ) -> Result<i64> {
        let pool      = self.pool.clone();
        let timeout   = Duration::from_secs(180);
        let start_str = start_time.format("%Y-%m-%d %H:%M:%S").to_string();
        let end_str   = end_time.format("%Y-%m-%d %H:%M:%S").to_string();

        let blocking = tokio::task::spawn_blocking(move || {
            let conn = pool.get().context("Oracle pool: get connection")?;

            let sql = "SELECT COUNT(1) \
                       FROM TCG_UCS.VALIDATION_RECORD \
                       WHERE CREATED_AT >= TO_TIMESTAMP(:1, 'YYYY-MM-DD HH24:MI:SS') \
                         AND CREATED_AT <  TO_TIMESTAMP(:2, 'YYYY-MM-DD HH24:MI:SS')";

            let rows = conn.query(sql, &[&start_str, &end_str])
                .context("GetCountByTimeRange query")?;
            for row_result in rows {
                let row = row_result.context("GetCountByTimeRange row read")?;
                let cnt: i64 = row.get(0).context("COUNT")?;
                return Ok(cnt);
            }
            Ok(0)
        });

        tokio::time::timeout(timeout, blocking)
            .await
            .map_err(|_| anyhow!("get_count_by_time_range timed out after {:?}", timeout))?
            .context("spawn_blocking panicked in get_count_by_time_range")?
    }

    /// Stream validation records within a time range by invoking a callback
    /// for every row.  Returns on the first callback error.
    ///
    /// Mirrors Go's `StreamByTimeRange` (row-by-row callback, not batch load).
    pub async fn stream_by_time_range<F>(
        &self,
        start_time: DateTime<Utc>,
        end_time:   DateTime<Utc>,
        callback:   F,
    ) -> Result<()>
    where
        F: Fn(ValidationRecord) -> Result<()> + Send + 'static,
    {
        let pool      = self.pool.clone();
        let timeout   = Duration::from_secs(600);
        let start_str = start_time.format("%Y-%m-%d %H:%M:%S").to_string();
        let end_str   = end_time.format("%Y-%m-%d %H:%M:%S").to_string();

        let blocking = tokio::task::spawn_blocking(move || {
            let conn = pool.get().context("Oracle pool: get connection")?;

            let sql = "SELECT ID, CUSTOMER_ID, CUSTOMER_NAME, SUCCESS, MERCHANT_CODE, \
                              IP, PASSING_SCORE, SCORE, QAS \
                       FROM TCG_UCS.VALIDATION_RECORD \
                       WHERE CREATED_AT >= TO_TIMESTAMP(:1, 'YYYY-MM-DD HH24:MI:SS') \
                         AND CREATED_AT <  TO_TIMESTAMP(:2, 'YYYY-MM-DD HH24:MI:SS') \
                       ORDER BY CREATED_AT ASC";

            let rows = conn.query(sql, &[&start_str, &end_str])
                .context("StreamByTimeRange query")?;

            for row_result in rows {
                let record = Self::map_row(row_result.context("StreamByTimeRange row read")?)?;
                callback(record)?;
            }
            Ok(())
        });

        tokio::time::timeout(timeout, blocking)
            .await
            .map_err(|_| anyhow!("stream_by_time_range timed out after {:?}", timeout))?
            .context("spawn_blocking panicked in stream_by_time_range")?
    }
    /// Dynamic multi-row SELECT with optional ordering and pagination.
    ///
    /// Mirrors Go's `FindListByEx(ctx, ex, optionalParams...)`.
    ///
    /// `where_clause` — raw SQL WHERE fragment (e.g. `"CUSTOMER_ID = :1 AND MERCHANT_CODE = :2"`).
    /// `params`       — positional bind values.
    /// `order_by`     — optional ORDER BY clause (e.g. `"CREATED_AT DESC"`).
    /// `pagination`   — optional `(page, page_size)` for Oracle OFFSET/FETCH.
    pub async fn find_list_by_expression(
        &self,
        where_clause: &str,
        params: Vec<Box<dyn oracle::sql_type::ToSql + Send>>,
        order_by: Option<&str>,
        pagination: Option<(u32, u32)>,
    ) -> Result<Vec<ValidationRecord>> {
        let pool    = self.pool.clone();
        let timeout = self.read_timeout;
        let wc      = where_clause.to_string();
        let ob      = order_by.map(|s| s.to_string());

        let blocking = tokio::task::spawn_blocking(move || {
            let conn = pool.get().context("Oracle pool: get connection")?;

            let mut sql = format!(
                "SELECT ID, CUSTOMER_ID, CUSTOMER_NAME, SUCCESS, MERCHANT_CODE,                         IP, PASSING_SCORE, SCORE, QAS                  FROM TCG_UCS.VALIDATION_RECORD WHERE {}", wc
            );
            if let Some(ref order) = ob {
                sql.push_str(&format!(" ORDER BY {}", order));
            }
            if let Some((page, page_size)) = pagination {
                let offset = (page.saturating_sub(1)) * page_size;
                sql.push_str(&format!(
                    " OFFSET {} ROWS FETCH NEXT {} ROWS ONLY",
                    offset, page_size
                ));
            }

            let param_refs: Vec<&dyn oracle::sql_type::ToSql> =
                params.iter().map(|p| p.as_ref() as &dyn oracle::sql_type::ToSql).collect();
            let rows = conn.query(&sql, param_refs.as_slice())
                .context("FindListByExpression query")?;

            let mut result = Vec::new();
            for row_result in rows {
                let record = Self::map_row(row_result.context("FindListByExpression row read")?)?;
                result.push(record);
            }
            Ok(result)
        });

        tokio::time::timeout(timeout, blocking)
            .await
            .map_err(|_| anyhow!("find_list_by_expression timed out"))?
            .context("spawn_blocking panicked")?
    }

}
