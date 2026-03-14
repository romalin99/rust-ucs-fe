//! Repository for TCG_UCS.MERCHANT_RULE.
//!
//! Key methods:
//! - `find_by_merchant_code`   — lookup by unique merchant code (verification flow)
//! - `get_rule_config`         — lean 5-column projection (submit flow)
//! - `get_all_merchant_codes`  — used by field-cache preloader
//! - `find_all_as_map`         — reads TEMPLATE_FIELDS and returns
//!                               `merchantCode → fieldId → Vec<DropdownItem>`

use infra::db::{oracle_ts_to_naive, run, OraclePool};
use common::error::AppError;
use domain::{DropdownItem, MerchantRule, MerchantRuleConfig, TemplateField};
use oracle::sql_type::Timestamp;
use std::collections::HashMap;

#[derive(Clone)]
pub struct MerchantRuleRepo {
    pool: OraclePool,
    #[allow(dead_code)]
    read_timeout: std::time::Duration,
}

impl MerchantRuleRepo {
    pub fn new(pool: OraclePool, read_timeout_secs: u64) -> Self {
        Self {
            pool,
            read_timeout: std::time::Duration::from_secs(read_timeout_secs),
        }
    }

    // ── Read-path ─────────────────────────────────────────────────────────────

    /// `SELECT … WHERE MERCHANT_CODE = :1`
    /// Corresponds to Go's `FindByMerchantCode`.
    pub async fn find_by_merchant_code(
        &self,
        merchant_code: String,
    ) -> Result<Option<MerchantRule>, AppError> {
        run(&self.pool, move |conn| {
            const SQL: &str = "\
                SELECT ID, IS_DEFAULT, MERCHANT_CODE, OPERATOR, \
                       IP_RETRY_LIMIT, ACCOUNT_RETRY_LIMIT, EMPTY_SCORE, \
                       LOCK_HOUR, BINDING_TYPE, PASSING_SCORE, QUESTIONS, \
                       CREATED_AT, UPDATED_AT \
                FROM TCG_UCS.MERCHANT_RULE \
                WHERE MERCHANT_CODE = :1";

            let rows = conn.query(SQL, &[&merchant_code])?;
            for row_result in rows {
                let row = row_result?;
                return Ok(Some(row_to_merchant_rule(&row)?));
            }
            Ok(None)
        })
        .await
    }

    /// Lightweight 5-column query — avoids reading large QUESTIONS CLOB
    /// unless needed.  Corresponds to Go's `GetRuleConfigByMerchantCode`.
    pub async fn get_rule_config(
        &self,
        merchant_code: String,
    ) -> Result<Option<MerchantRuleConfig>, AppError> {
        run(&self.pool, move |conn| {
            const SQL: &str = "\
                SELECT MERCHANT_CODE, BINDING_TYPE, EMPTY_SCORE, PASSING_SCORE, QUESTIONS \
                FROM TCG_UCS.MERCHANT_RULE \
                WHERE MERCHANT_CODE = :1";

            let rows = conn.query(SQL, &[&merchant_code])?;
            for row_result in rows {
                let row = row_result?;
                let cfg = MerchantRuleConfig {
                    merchant_code: row.get::<_, String>("MERCHANT_CODE")?,
                    binding_type: row.get::<_, String>("BINDING_TYPE")?,
                    empty_score: row.get::<_, i32>("EMPTY_SCORE")?,
                    passing_score: row.get::<_, i32>("PASSING_SCORE")?,
                    questions: row.get::<_, String>("QUESTIONS")?,
                };
                return Ok(Some(cfg));
            }
            Ok(None)
        })
        .await
    }

    /// Return all distinct merchant codes — used by the field-config preloader.
    pub async fn get_all_merchant_codes(&self) -> Result<Vec<String>, AppError> {
        run(&self.pool, move |conn| {
            let rows = conn.query("SELECT MERCHANT_CODE FROM TCG_UCS.MERCHANT_RULE", &[])?;
            let mut codes = Vec::new();
            for row_result in rows {
                codes.push(row_result?.get::<_, String>(0)?);
            }
            Ok(codes)
        })
        .await
    }

    /// Read `TEMPLATE_FIELDS` JSON for every merchant and build a nested
    /// lookup map:
    /// ```
    /// merchantCode → fieldId → Vec<DropdownItem>
    /// ```
    ///
    /// This is the data source for `FieldCache` — it gives every question's
    /// dropdown options enriched from the USS template definition.
    ///
    /// Corresponds to Go's `FindAllAsMap`.
    pub async fn find_all_as_map(
        &self,
    ) -> Result<HashMap<String, HashMap<String, Vec<DropdownItem>>>, AppError> {
        run(&self.pool, move |conn| {
            // TEMPLATE_FIELDS stores a JSON array of TemplateField objects.
            // Rows where the column is NULL are silently skipped.
            const SQL: &str = "SELECT MERCHANT_CODE, TEMPLATE_FIELDS FROM TCG_UCS.MERCHANT_RULE \
                 WHERE TEMPLATE_FIELDS IS NOT NULL";

            let rows = conn.query(SQL, &[])?;
            let mut result: HashMap<String, HashMap<String, Vec<DropdownItem>>> = HashMap::new();

            for row_result in rows {
                let row = row_result?;
                let merchant_code: String = row.get(0)?;
                let template_json: String = match row.get::<_, Option<String>>(1)? {
                    Some(s) if !s.is_empty() => s,
                    _ => continue,
                };

                let fields: Vec<TemplateField> =
                    serde_json::from_str(&template_json).unwrap_or_default();

                let mut field_map: HashMap<String, Vec<DropdownItem>> =
                    HashMap::with_capacity(fields.len());

                for tf in fields {
                    if !tf.dropdown_list.is_empty() {
                        field_map.insert(tf.field_id, tf.dropdown_list);
                    }
                }

                result.insert(merchant_code, field_map);
            }

            Ok(result)
        })
        .await
    }

    // ── Write-path ────────────────────────────────────────────────────────────

    /// Insert a new merchant rule row.
    /// Uses `SEQ_MERCHANT_RULE.NEXTVAL` for the primary key; returns the new ID.
    pub async fn insert(&self, rule: MerchantRule) -> Result<i64, AppError> {
        run(&self.pool, move |conn| {
            conn.execute(
                "INSERT INTO TCG_UCS.MERCHANT_RULE ( \
                    ID, IS_DEFAULT, MERCHANT_CODE, OPERATOR, \
                    IP_RETRY_LIMIT, ACCOUNT_RETRY_LIMIT, EMPTY_SCORE, \
                    LOCK_HOUR, BINDING_TYPE, PASSING_SCORE, QUESTIONS, \
                    CREATED_AT, UPDATED_AT \
                ) VALUES ( \
                    SEQ_MERCHANT_RULE.NEXTVAL, :1, :2, :3, :4, :5, :6, :7, :8, :9, :10, \
                    SYSTIMESTAMP, SYSTIMESTAMP \
                )",
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
                    &rule.questions,
                ],
            )?;

            let mut id = 0i64;
            let rows = conn.query("SELECT SEQ_MERCHANT_RULE.CURRVAL FROM DUAL", &[])?;
            for row_result in rows {
                id = row_result?.get::<_, i64>(0)?;
            }
            conn.commit()?;
            Ok(id)
        })
        .await
    }

    /// Update the TEMPLATE_FIELDS column for a given merchant.
    pub async fn update_template_fields(
        &self,
        merchant_code: String,
        template_fields: String,
    ) -> Result<(), AppError> {
        run(&self.pool, move |conn| {
            conn.execute(
                "UPDATE TCG_UCS.MERCHANT_RULE SET TEMPLATE_FIELDS = :1, UPDATED_AT = SYSTIMESTAMP \
                 WHERE MERCHANT_CODE = :2",
                &[&template_fields, &merchant_code],
            )?;
            conn.commit()?;
            Ok(())
        })
        .await
    }
}

// ── Row → struct ──────────────────────────────────────────────────────────────

fn row_to_merchant_rule(row: &oracle::Row) -> anyhow::Result<MerchantRule> {
    let created_ts: Timestamp = row.get("CREATED_AT")?;
    let updated_ts: Timestamp = row.get("UPDATED_AT")?;

    Ok(MerchantRule {
        id: row.get::<_, i64>("ID")?,
        is_default: row.get::<_, i8>("IS_DEFAULT")?,
        merchant_code: row.get::<_, String>("MERCHANT_CODE")?,
        operator: row
            .get::<_, Option<String>>("OPERATOR")?
            .unwrap_or_default(),
        ip_retry_limit: row.get::<_, i32>("IP_RETRY_LIMIT")?,
        account_retry_limit: row.get::<_, i32>("ACCOUNT_RETRY_LIMIT")?,
        empty_score: row.get::<_, i32>("EMPTY_SCORE")?,
        lock_hour: row.get::<_, i32>("LOCK_HOUR")?,
        binding_type: row.get::<_, String>("BINDING_TYPE")?,
        passing_score: row.get::<_, i32>("PASSING_SCORE")?,
        questions: row.get::<_, String>("QUESTIONS")?,
        created_at: oracle_ts_to_naive(created_ts),
        updated_at: oracle_ts_to_naive(updated_ts),
    })
}
