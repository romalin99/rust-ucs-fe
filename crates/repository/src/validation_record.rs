//! Repository for TCG_UCS.VALIDATION_RECORD.
//!
//! Write strategy mirrors Go: try `Insert` first; if that fails (e.g. a row
//! already exists due to a unique constraint), fall back to `Upsert` (MERGE).

use infra::db::{oracle_ts_to_naive, run, OraclePool};
use common::error::AppError;
use domain::ValidationRecord;
use oracle::sql_type::Timestamp;
use tracing::warn;

#[derive(Clone)]
pub struct ValidationRecordRepo {
    pool: OraclePool,
}

impl ValidationRecordRepo {
    pub fn new(pool: OraclePool) -> Self {
        Self { pool }
    }

    // ── Write ─────────────────────────────────────────────────────────────────

    /// Try INSERT; on failure fall back to Oracle MERGE (upsert).
    ///
    /// Mirrors Go's:
    /// ```go
    /// if err = s.com.Insert(ctx, &record); err != nil {
    ///     s.com.Upsert(ctx, &record)
    /// }
    /// ```
    pub async fn save(&self, record: ValidationRecord) -> Result<(), AppError> {
        // Clone so we can retry on failure.
        let record2 = record.clone();

        match self.insert(record).await {
            Ok(_) => Ok(()),
            Err(e) => {
                warn!(
                    "ValidationRecord INSERT failed, falling back to UPSERT: {}",
                    e
                );
                self.upsert(record2).await
            }
        }
    }

    /// Plain INSERT — uses the sequence `SEQ_VALIDATION_RECORD.NEXTVAL` for the PK.
    pub async fn insert(&self, record: ValidationRecord) -> Result<i64, AppError> {
        run(&self.pool, move |conn| {
            const SQL: &str = "\
                INSERT INTO TCG_UCS.VALIDATION_RECORD ( \
                    ID, CUSTOMER_ID, CUSTOMER_NAME, SUCCESS, MERCHANT_CODE, IP, \
                    PASSING_SCORE, SCORE, QAS, CREATED_AT \
                ) VALUES ( \
                    SEQ_VALIDATION_RECORD.NEXTVAL, :1, :2, :3, :4, :5, :6, :7, :8, SYSTIMESTAMP \
                )";

            conn.execute(
                SQL,
                &[
                    &record.customer_id,
                    &record.customer_name,
                    &record.success,
                    &record.merchant_code,
                    &record.ip,
                    &record.passing_score,
                    &record.score,
                    &record.qas,
                ],
            )?;
            conn.commit()?;

            // Return the newly assigned ID.
            let rows = conn.query("SELECT SEQ_VALIDATION_RECORD.CURRVAL FROM DUAL", &[])?;
            let mut id = 0i64;
            for row_result in rows {
                id = row_result?.get::<_, i64>(0)?;
            }
            Ok(id)
        })
        .await
    }

    /// Oracle MERGE — update the latest record for (customer_id, merchant_code)
    /// if it exists, otherwise insert a new row.
    pub async fn upsert(&self, record: ValidationRecord) -> Result<(), AppError> {
        run(&self.pool, move |conn| {
            const SQL: &str = "\
                MERGE INTO TCG_UCS.VALIDATION_RECORD tgt \
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
                        ) AS MATCH_ID, \
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
                    INSERT (CUSTOMER_ID, CUSTOMER_NAME, SUCCESS, MERCHANT_CODE, IP, \
                            PASSING_SCORE, SCORE, QAS, CREATED_AT) \
                    VALUES (src.P_CUSTOMER_ID, src.P_CUSTOMER_NAME, src.P_SUCCESS, \
                            src.P_MERCHANT_CODE, src.P_IP, src.P_PASSING_SCORE, \
                            src.P_SCORE, src.P_QAS, SYSTIMESTAMP)";

            conn.execute(
                SQL,
                &[
                    &record.customer_id,
                    &record.merchant_code,
                    &record.customer_id,
                    &record.customer_name,
                    &record.success,
                    &record.merchant_code,
                    &record.ip,
                    &record.passing_score,
                    &record.score,
                    &record.qas,
                ],
            )?;
            conn.commit()?;
            Ok(())
        })
        .await
    }

    // ── Read ──────────────────────────────────────────────────────────────────

    /// Fetch the most recent validation record for a customer + merchant pair.
    pub async fn find_latest(
        &self,
        customer_id: i64,
        merchant_code: String,
    ) -> Result<Option<ValidationRecord>, AppError> {
        run(&self.pool, move |conn| {
            const SQL: &str = "\
                SELECT ID, CUSTOMER_ID, CUSTOMER_NAME, SUCCESS, MERCHANT_CODE, IP, \
                       PASSING_SCORE, SCORE, QAS, CREATED_AT \
                FROM TCG_UCS.VALIDATION_RECORD \
                WHERE CUSTOMER_ID = :1 AND MERCHANT_CODE = :2 \
                ORDER BY CREATED_AT DESC \
                FETCH FIRST 1 ROWS ONLY";

            let rows = conn.query(SQL, &[&customer_id, &merchant_code])?;
            for row_result in rows {
                let row = row_result?;
                return Ok(Some(row_to_record(&row)?));
            }
            Ok(None)
        })
        .await
    }
}

// ── Row → struct ──────────────────────────────────────────────────────────────

fn row_to_record(row: &oracle::Row) -> anyhow::Result<ValidationRecord> {
    let created_ts: Timestamp = row.get("CREATED_AT")?;

    Ok(ValidationRecord {
        id: row.get::<_, i64>("ID")?,
        customer_id: row.get::<_, i64>("CUSTOMER_ID")?,
        customer_name: row.get::<_, String>("CUSTOMER_NAME")?,
        success: row.get::<_, i8>("SUCCESS")?,
        merchant_code: row.get::<_, String>("MERCHANT_CODE")?,
        ip: row.get::<_, String>("IP")?,
        passing_score: row.get::<_, i32>("PASSING_SCORE")?,
        score: row.get::<_, i32>("SCORE")?,
        qas: row.get::<_, String>("QAS")?,
        created_at: oracle_ts_to_naive(created_ts),
    })
}
