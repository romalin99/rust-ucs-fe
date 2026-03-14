/// Oracle repository for TCG_UCS.VALIDATION_RECORD.
///
/// Uses [rust-oracle](https://github.com/kubo/rust-oracle) (`oracle` crate)
/// for direct Oracle Database access via an `r2d2` connection pool.
///
/// All Oracle calls are wrapped in `tokio::task::spawn_blocking` because
/// `oracle::Connection` is sync-only.
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use chrono::Utc;

use crate::model::validation_record::ValidationRecord;
use crate::repository::merchant_rule::OraclePool;

#[derive(Clone)]
pub struct ValidationRecordRepository {
    pool: Arc<OraclePool>,
    read_timeout: Duration,
    write_timeout: Duration,
}

impl ValidationRecordRepository {
    pub fn new(pool: Arc<OraclePool>) -> Self {
        Self {
            pool,
            read_timeout: Duration::from_secs(15),
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
        let pool = self.pool.clone();
        let timeout = self.write_timeout;

        let blocking = tokio::task::spawn_blocking(move || {
            let conn = pool.get().context("Oracle pool: get connection")?;

            // Identical to Go's UpsertWithContext MERGE statement.
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

            conn.execute(
                sql,
                &[
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
                ],
            )
            .context("ValidationRecord MERGE execute")?;

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
        let pool = self.pool.clone();
        let timeout = self.write_timeout;

        let blocking = tokio::task::spawn_blocking(move || {
            let conn = pool.get().context("Oracle pool: get connection")?;
            let success = record.success as i32;

            let sql = "INSERT INTO TCG_UCS.VALIDATION_RECORD \
                        (CUSTOMER_ID, CUSTOMER_NAME, SUCCESS, MERCHANT_CODE, IP, \
                         PASSING_SCORE, SCORE, QAS, CREATED_AT) \
                       VALUES \
                        (:1, :2, :3, :4, :5, :6, :7, :8, SYSTIMESTAMP)";

            conn.execute(
                sql,
                &[
                    &record.customer_id,
                    &record.customer_name,
                    &success,
                    &record.merchant_code,
                    &record.ip,
                    &record.passing_score,
                    &record.score,
                    &record.qas,
                ],
            )
            .context("ValidationRecord INSERT execute")?;

            conn.commit().context("ValidationRecord INSERT commit")?;
            Ok(())
        });

        tokio::time::timeout(timeout, blocking)
            .await
            .map_err(|_| anyhow!("insert timed out after {:?}", timeout))?
            .context("spawn_blocking panicked in insert")?
    }

    // ── Read operations ───────────────────────────────────────────────────────

    /// Find the most recent validation record for a customer+merchant pair.
    ///
    /// Returns `None` when no matching row exists.
    /// Mirrors Go's `FindLatestByCustomerAndMerchant`.
    pub async fn find_latest_by_customer_and_merchant(
        &self,
        customer_id: i64,
        merchant_code: &str,
    ) -> Result<Option<ValidationRecord>> {
        let pool = self.pool.clone();
        let mc = merchant_code.to_string();
        let timeout = self.read_timeout;

        let blocking = tokio::task::spawn_blocking(move || {
            let conn = pool.get().context("Oracle pool: get connection")?;

            let sql = "SELECT ID, CUSTOMER_ID, CUSTOMER_NAME, SUCCESS, MERCHANT_CODE, \
                              IP, PASSING_SCORE, SCORE, QAS \
                       FROM TCG_UCS.VALIDATION_RECORD \
                       WHERE CUSTOMER_ID = :1 AND MERCHANT_CODE = :2 \
                       ORDER BY CREATED_AT DESC \
                       FETCH FIRST 1 ROWS ONLY";

            let rows = conn
                .query(sql, &[&customer_id, &mc])
                .context("FindLatest query")?;

            for row_result in rows {
                let row = row_result.context("FindLatest row read")?;

                let rec = ValidationRecord {
                    id: row.get::<_, Option<i64>>(0).unwrap_or_default(),
                    customer_id: row.get::<_, i64>(1).context("CUSTOMER_ID")?,
                    customer_name: row.get::<_, String>(2).context("CUSTOMER_NAME")?,
                    success: row.get::<_, i32>(3).unwrap_or(0) as i8,
                    merchant_code: row.get::<_, String>(4).context("MERCHANT_CODE")?,
                    ip: row.get::<_, String>(5).context("IP")?,
                    passing_score: row.get::<_, i32>(6).context("PASSING_SCORE")?,
                    score: row.get::<_, i32>(7).context("SCORE")?,
                    qas: row.get::<_, String>(8).context("QAS")?,
                    created_at: Utc::now(), // CREATED_AT not needed by caller
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

    /// Count failed attempts for a customer+merchant since a given timestamp.
    ///
    /// Mirrors Go's `CountFailByCustomerAndMerchantSince`.
    pub async fn count_fail_since(
        &self,
        customer_id: i64,
        merchant_code: &str,
        since: chrono::DateTime<Utc>,
    ) -> Result<i64> {
        let pool = self.pool.clone();
        let mc = merchant_code.to_string();
        let timeout = self.read_timeout;
        // Pass as a naive-local string; Oracle CAST handles the rest.
        let since_str = since.format("%Y-%m-%d %H:%M:%S").to_string();

        let blocking = tokio::task::spawn_blocking(move || {
            let conn = pool.get().context("Oracle pool: get connection")?;

            let sql = "SELECT COUNT(1) \
                       FROM TCG_UCS.VALIDATION_RECORD \
                       WHERE CUSTOMER_ID   = :1 \
                         AND MERCHANT_CODE = :2 \
                         AND SUCCESS       = 0 \
                         AND CREATED_AT   >= TO_TIMESTAMP(:3, 'YYYY-MM-DD HH24:MI:SS')";

            let rows = conn
                .query(sql, &[&customer_id, &mc, &since_str])
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
}
