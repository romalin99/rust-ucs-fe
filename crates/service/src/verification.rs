//! Core player-verification business logic.
//!
//! Mirrors Go's `internal/service/verification.go`.
//!
//! Two public entry points:
//!   - `get_question_list`       — fetch enabled questions + enforce rate limit
//!   - `submit_verify_materials` — score answers and write a validation record

use infra::clients::{
    mcs::{
        McsClient, PlayerHeaders, VerifyFinanceHistoryReq, VerifyFinanceHistoryResp,
        VerifyPlayerFinanceInfo, VerifyPlayerHistoryInfo,
    },
    uss::UssClient,
};
use common::error::{AppError, ServiceError};
use domain::{MerchantRule, QuestionInfo, ValidationRecord, QA};
use repository::{MerchantRuleRepo, ValidationRecordRepo};
use crate::field_cache::FieldCache;
use chrono::Local;
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use tracing::{info, warn};

// ── Rate-limit key formats ────────────────────────────────────────────────────

/// Atomic INCR + EXPIREAT Lua script.
/// On the first call for a key, also sets its expiry to end-of-day.
const INCR_WITH_TTL_SCRIPT: &str = r#"
local v = redis.call('INCR', KEYS[1])
if v == 1 then
    redis.call('EXPIREAT', KEYS[1], ARGV[1])
end
return v
"#;

/// Field IDs whose verification is handled by MCS (financial / txn history)
/// rather than the USS profile.
fn financial_field_ids() -> HashSet<&'static str> {
    [
        "BANK_ACCOUNT",
        "CARD_HOLDER_NAME",
        "VIRTUAL_WALLET_ADDRESS",
        "VIRTUAL_WALLET_NAME",
        "E_WALLET_ACCOUNT",
        "E_WALLET_NAME",
        "LAST_DEPOSIT_AMOUNT",
        "LAST_DEPOSIT_TIME",
        "LAST_DEPOSIT_METHOD",
        "LAST_WITHDRAWAL_AMOUNT",
        "LAST_WITHDRAWAL_TIME",
        "LAST_WITHDRAWAL_METHOD",
    ]
    .into()
}

// ── Request / response DTOs ───────────────────────────────────────────────────

#[derive(Debug, serde::Deserialize)]
pub struct VerifyItem {
    #[serde(rename = "fieldId")]
    pub field_id: String,
    #[serde(rename = "fieldValue")]
    pub field_value: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct VerifyDataItem {
    pub item: VerifyItem,
    pub bind: bool,
}

#[derive(Debug, serde::Deserialize)]
pub struct SubmitVerifyRequest {
    #[serde(rename = "customerName")]
    pub customer_name: String,
    pub data: Vec<VerifyDataItem>,
}

#[derive(Debug, Serialize)]
pub struct MerchantRuleResponse {
    #[serde(rename = "merchantCode")]
    pub merchant_code: String,
    pub questions: Vec<QuestionInfo>,
}

#[derive(Debug, Serialize)]
pub struct SubmitVerifyResponse {
    #[serde(rename = "bindType")]
    pub bind_type: String,
    #[serde(rename = "oneTimePassword")]
    pub one_time_password: String,
}

// ── Service ───────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct VerificationService {
    merchant_rule_repo: MerchantRuleRepo,
    validation_record_repo: ValidationRecordRepo,
    uss: UssClient,
    mcs: McsClient,
    field_cache: FieldCache,
    rate_limit_redis: Arc<redis::Client>,
}

impl VerificationService {
    pub fn new(
        merchant_rule_repo: MerchantRuleRepo,
        validation_record_repo: ValidationRecordRepo,
        uss: UssClient,
        mcs: McsClient,
        field_cache: FieldCache,
        rate_limit_redis: Arc<redis::Client>,
    ) -> Self {
        Self {
            merchant_rule_repo,
            validation_record_repo,
            uss,
            mcs,
            field_cache,
            rate_limit_redis,
        }
    }

    // ── Public API ────────────────────────────────────────────────────────────

    /// Return the list of enabled verification questions for a merchant.
    ///
    /// 1. Load the full merchant rule.
    /// 2. Check+increment the per-IP and per-account daily retry counters.
    /// 3. Build `Vec<QuestionInfo>`, enriching DD fields from the field cache.
    pub async fn get_question_list(
        &self,
        merchant_code: &str,
        customer_ip: &str,
        customer_name: &str,
    ) -> Result<MerchantRuleResponse, AppError> {
        // 1. Load merchant rule.
        let rule = self
            .merchant_rule_repo
            .find_by_merchant_code(merchant_code.to_string())
            .await?
            .ok_or_else(|| {
                AppError::Service(ServiceError::MerchantNotFound(merchant_code.to_string()))
            })?;

        // 2. Rate limiting.
        self.check_and_incr_retry_limit(merchant_code, customer_ip, customer_name, &rule)
            .await?;

        // 3. Build question list with dropdown enrichment.
        let questions = self.get_valid_question_infos(merchant_code, &rule).await?;

        info!(
            "GetQuestionList success: merchant={} customer={} count={}",
            merchant_code,
            customer_name,
            questions.len()
        );

        Ok(MerchantRuleResponse {
            merchant_code: merchant_code.to_string(),
            questions,
        })
    }

    /// Validate submitted answers, write a VALIDATION_RECORD, and return a
    /// one-time password reset token on success.
    pub async fn submit_verify_materials(
        &self,
        merchant_code: &str,
        customer_ip: &str,
        req: SubmitVerifyRequest,
    ) -> Result<SubmitVerifyResponse, AppError> {
        if req.data.is_empty() {
            return Err(AppError::Service(ServiceError::InvalidRequestParam(
                "data array is required".to_string(),
            )));
        }

        let full_customer_name = format!("{}@{}", merchant_code, req.customer_name);

        // 1. Fetch player profile from USS.
        let customer_info = self
            .uss
            .get_customer(&full_customer_name, false)
            .await
            .map_err(|_| {
                AppError::Service(ServiceError::CustomerFetchFailed(
                    full_customer_name.clone(),
                ))
            })?;

        info!(
            "CustomerID={} CustomerName={}",
            customer_info.value.customer_id.val, full_customer_name
        );

        // 2. Generate one-time password reset token.
        let token_resp = self
            .uss
            .generate_password_reset_token(&req.customer_name, merchant_code)
            .await
            .map_err(|e| AppError::Service(ServiceError::PasswordResetFailed(e.to_string())))?;

        if !token_resp.success {
            return Err(AppError::Service(ServiceError::PasswordResetFailed(
                "USS returned failure".to_string(),
            )));
        }

        info!(
            "one-time password generated for customerName={}",
            full_customer_name
        );

        // 3. Load merchant rule config (lean projection).
        let rule_cfg = self
            .merchant_rule_repo
            .get_rule_config(merchant_code.to_string())
            .await?
            .ok_or_else(|| {
                AppError::Service(ServiceError::MerchantNotFound(merchant_code.to_string()))
            })?;

        // 4. Parse question config.
        let questions_map = rule_cfg
            .parse_questions()
            .map_err(|e| AppError::Service(ServiceError::ParseJsonFailed(e.to_string())))?;

        // 5. Score USS profile fields.
        let mut qas: HashMap<String, QA> = HashMap::new();
        accurate_judgment_score(
            &req.data,
            &customer_info,
            rule_cfg.empty_score,
            &mut qas,
            &questions_map,
        );

        // 6. Score MCS financial-history fields.
        let mcs_headers = PlayerHeaders {
            customer_id: customer_info.value.customer_id.val.to_string(),
            customer_name: req.customer_name.clone(),
            merchant: merchant_code.to_string(),
            customer_ip: customer_ip.to_string(),
        };
        let mcs_req = build_financial_history_req(&req.data, &questions_map);

        let mcs_resp = self
            .mcs
            .verify_player_info(&mcs_headers, &mcs_req)
            .await
            .map_err(|e| {
                warn!("MCS verify_player_info failed: {}", e);
                AppError::Service(ServiceError::McsVerifyFailed)
            })?;

        calculate_score_financial(&mcs_resp, rule_cfg.empty_score, &mut qas, &questions_map);

        // 7. Serialise QA map and compute total score.
        let qas_json = serde_json::to_string(&qas)
            .map_err(|e| AppError::Infra(common::error::InfraError::Json(e)))?;

        let actual_score: i32 = qas.values().map(|q| q.score).sum();
        let score_pass: i8 = if actual_score > rule_cfg.passing_score {
            1
        } else {
            0
        };

        info!(
            "customer_id={} merchant={} actual_score={} passing_score={} pass={}",
            customer_info.value.customer_id.val,
            merchant_code,
            actual_score,
            rule_cfg.passing_score,
            score_pass
        );

        // 8. Persist validation record (Insert → Upsert fallback).
        let record = ValidationRecord {
            id: 0,
            customer_id: customer_info.value.customer_id.val,
            customer_name: full_customer_name,
            success: score_pass,
            merchant_code: merchant_code.to_string(),
            ip: customer_ip.to_string(),
            passing_score: rule_cfg.passing_score,
            score: actual_score,
            qas: qas_json,
            created_at: chrono::Local::now().naive_local(),
        };

        // Go calls Insert only; Upsert fallback is commented out in the original.
        if let Err(e) = self.validation_record_repo.insert(record).await {
            warn!("insert validation record failed: {e}");
            return Err(e);
        }
        info!(
            "insert validation record success, customer_id={}, merchant_code={}",
            customer_info.value.customer_id.val, merchant_code
        );

        Ok(SubmitVerifyResponse {
            bind_type: rule_cfg.binding_type,
            one_time_password: token_resp.value,
        })
    }

    // ── Rate limiting ─────────────────────────────────────────────────────────

    /// Read per-IP and per-account counters; if either ≥ limit, refuse the request.
    /// Then atomically increment both counters (INCR+EXPIREAT via Lua script).
    async fn check_and_incr_retry_limit(
        &self,
        merchant_code: &str,
        customer_ip: &str,
        customer_name: &str,
        rule: &MerchantRule,
    ) -> Result<(), AppError> {
        let today = Local::now().format("%Y%m%d").to_string();

        let ip_key = format!(
            "ucsfe:ql:ip:{}:{}:{}:{}",
            merchant_code, customer_name, customer_ip, today
        );
        let acct_key = format!(
            "ucsfe:ql:acct:{}:{}:{}",
            merchant_code, customer_name, today
        );

        let mut conn = self
            .rate_limit_redis
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| AppError::Service(ServiceError::RedisUnavailable(e.to_string())))?;

        // Concurrent read of both counters via pipeline.
        let (ip_cnt, acct_cnt): (Option<i64>, Option<i64>) = redis::pipe()
            .cmd("GET")
            .arg(&ip_key)
            .cmd("GET")
            .arg(&acct_key)
            .query_async(&mut conn)
            .await
            .map_err(|e| AppError::Service(ServiceError::RedisUnavailable(e.to_string())))?;

        let ip_cnt = ip_cnt.unwrap_or(0);
        let acct_cnt = acct_cnt.unwrap_or(0);
        let ip_limit = rule.ip_retry_limit as i64;
        let acct_limit = rule.account_retry_limit as i64;

        info!(
            "retryLimit check: merchant={} customer={} ip={} date={} ip={}/{} acct={}/{}",
            merchant_code,
            customer_name,
            customer_ip,
            today,
            ip_cnt,
            ip_limit,
            acct_cnt,
            acct_limit
        );

        if ip_cnt >= ip_limit || acct_cnt >= acct_limit {
            warn!(
                "question retry limit exhausted: merchant={} customer={} ip={}",
                merchant_code, customer_name, customer_ip
            );
            return Err(AppError::Service(ServiceError::QuestionLimitExceeded));
        }

        // Atomic INCR+EXPIREAT for both keys (sequential to avoid double-mutable borrow).
        let end_of_day = Local::now()
            .date_naive()
            .and_hms_opt(23, 59, 59)
            .unwrap()
            .and_local_timezone(Local)
            .unwrap()
            .timestamp();

        let script = redis::Script::new(INCR_WITH_TTL_SCRIPT);

        let r1: Result<i64, _> = script
            .key(&ip_key)
            .arg(end_of_day)
            .invoke_async(&mut conn)
            .await;
        if let Err(e) = r1 {
            warn!("redis incr ip key failed: {}", e);
        }

        let r2: Result<i64, _> = script
            .key(&acct_key)
            .arg(end_of_day)
            .invoke_async(&mut conn)
            .await;
        if let Err(e) = r2 {
            warn!("redis incr acct key failed: {}", e);
        }

        Ok(())
    }

    // ── Question-list builder ─────────────────────────────────────────────────

    /// Parse the QUESTIONS CLOB and enrich DD (dropdown) fields from
    /// `FieldCache` — mirrors Go's `getValidQuestionInfos`.
    async fn get_valid_question_infos(
        &self,
        merchant_code: &str,
        rule: &MerchantRule,
    ) -> Result<Vec<QuestionInfo>, AppError> {
        let questions_map = rule
            .parse_valid_questions()
            .map_err(|e| AppError::Service(ServiceError::ParseJsonFailed(e.to_string())))?;

        let mut result = Vec::with_capacity(questions_map.len());

        for (_, q) in &questions_map {
            if q.field_id.is_empty() {
                continue;
            }

            let dropdown = if q.field_attribute == "DD" {
                // Try to get dropdown list from the field cache.
                self.field_cache
                    .get_dropdown(merchant_code, &q.field_id)
                    .await
                    .unwrap_or_default()
            } else {
                Vec::new()
            };

            result.push(QuestionInfo {
                field_id: q.field_id.clone(),
                field_name: q.field_name.clone(),
                field_attribute: q.field_attribute.clone(),
                field_type: q.field_type.clone(),
                field_dropdown_list: dropdown,
            });
        }

        Ok(result)
    }
}

// ── Scoring helpers ───────────────────────────────────────────────────────────

/// Compare submitted profile-field answers against USS customer data.
///
/// For integer fields (gender, marital_status, etc.) the expected value is the
/// decimal string representation — matching Go's `strconv.FormatInt(int64(val), 10)`.
///
/// Ports Go's `accurateJudgmentScore`.
fn accurate_judgment_score(
    data: &[VerifyDataItem],
    customer_info: &infra::clients::uss::CustomerInfo,
    empty_score: i32,
    qas: &mut HashMap<String, QA>,
    question_cfg: &HashMap<String, domain::Question>,
) {
    let p = &customer_info.value.profile;
    let a = &customer_info.value.additional_info;
    let v = &customer_info.value;
    let financial = financial_field_ids();

    let field_lookup: HashMap<&str, String> = [
        ("PLACE_OF_BIRTH", a.place_of_birth.val.clone()),
        ("MARITAL_STATUS", p.marital_status.val.to_string()),
        ("NICKNAME", p.nickname.val.clone()),
        ("TAG_REGION", a.region.val.clone()),
        ("QQ", p.qq_no.val.clone()),
        ("WECHAT_ID", p.wechat.val.clone()),
        ("LINE_ID", p.line_id.val.clone()),
        ("FB_ID", p.facebook_id.val.clone()),
        ("WHATSAPP", p.whats_app_id.val.clone()),
        ("ZALO", p.zalo.val.clone()),
        ("TELEGRAM", p.telegram.val.clone()),
        ("VIBER", p.viber.val.clone()),
        ("TWITTER", p.twitter.val.clone()),
        ("EMAIL", v.email.val.clone()),
        ("MOBILE_NUMBER", p.mobile_no.val.clone()),
        ("FIXED_ADDRESS", a.permanent_address.val.clone()),
        ("ADDRESS", p.address.val.clone()),
        ("WITHDRAWER_NAME", p.payee_name.val.clone()),
        ("DATE_OF_BIRTH", p.birthday.format_date()),
        ("NATIONALITY", a.nationality.val.clone()),
        ("STATE", a.us_state.val.to_string()),
        ("ID", p.id_number.val.clone()),
        ("GENDER", p.gender.val.to_string()),
        ("JOB", p.occupation.val.to_string()),
        ("SOURCE_OF_INCOME", p.source_of_income.val.to_string()),
        ("ID_TYPE", p.id_type.val.to_string()),
        ("ZIP_CODE", p.zip_code.val.clone()),
        ("APPLE_ID", p.apple_id.val.clone()),
        ("KAKAO", a.kakao.val.clone()),
        ("GOOGLE", a.google.val.clone()),
        ("LAST_LOGIN_TIME", p.last_login_time.format_date()),
        ("REGISTRATION_TIME", p.reg_date.format_date()),
    ]
    .into_iter()
    .collect();

    for item in data {
        let field_id = item.item.field_id.as_str();

        if financial.contains(field_id) {
            continue;
        }

        let expected = match field_lookup.get(field_id) {
            Some(v) => v,
            None => {
                warn!("unknown fieldId: {}", field_id);
                continue;
            }
        };

        let q_cfg = match question_cfg.get(field_id) {
            Some(q) => q,
            None => continue,
        };

        let submitted = &item.item.field_value;
        let is_correct = submitted.eq_ignore_ascii_case(expected);

        if !is_correct {
            warn!(
                "mismatch {}: submitted={} expected={}",
                field_id, submitted, expected
            );
        }

        let score = if item.bind {
            if is_correct {
                q_cfg.score
            } else {
                0
            }
        } else {
            empty_score
        };

        qas.insert(
            field_id.to_string(),
            QA {
                field_id: field_id.to_string(),
                field_type: q_cfg.field_type.clone(),
                correct: is_correct,
                score,
                total_score: q_cfg.score,
            },
        );
    }
}

/// Build the MCS request from submitted financial-history fields.
///
/// Key logic mirrors Go's `applyFieldSetters`:
/// - `bind = false`     → field is zeroed (empty string / 0)
/// - `bind = true` + empty value → `"NULL"` (MCS treats "NULL" as "user said nothing")
/// - `bind = true` + value present → use the value
///
/// For amount/time range fields, `accuracy` from the question config is also forwarded.
fn build_financial_history_req(
    data: &[VerifyDataItem],
    question_cfg: &HashMap<String, domain::Question>,
) -> VerifyFinanceHistoryReq {
    let mut values: HashMap<&str, &str> = HashMap::new();
    let mut bind_map: HashMap<&str, bool> = HashMap::new();

    for item in data {
        let id = item.item.field_id.as_str();
        if financial_field_ids().contains(id) {
            values.insert(id, &item.item.field_value);
            bind_map.insert(id, item.bind);
        }
    }

    // Returns ("NULL", acc_or_null) or ("", "") depending on bind logic.
    let get_val_acc = |key: &str| -> (String, String) {
        if !bind_map.get(key).copied().unwrap_or(false) {
            return (String::new(), String::new());
        }
        let v = values.get(key).copied().unwrap_or("");
        let value = if v.is_empty() {
            "NULL".to_string()
        } else {
            v.to_string()
        };
        let accuracy = if value == "NULL" {
            "NULL".to_string()
        } else {
            question_cfg
                .get(key)
                .map(|q| q.accuracy.clone())
                .unwrap_or_default()
        };
        (value, accuracy)
    };

    let (bc_number, _) = get_val_acc("BANK_ACCOUNT");
    let (bc_holder, _) = get_val_acc("CARD_HOLDER_NAME");
    let (ew_account, _) = get_val_acc("E_WALLET_ACCOUNT");
    let (ew_holder, _) = get_val_acc("E_WALLET_NAME");
    let (vw_address, _) = get_val_acc("VIRTUAL_WALLET_ADDRESS");
    let (vw_holder, _) = get_val_acc("VIRTUAL_WALLET_NAME");

    let (dep_amount, dep_amt_range) = get_val_acc("LAST_DEPOSIT_AMOUNT");
    let (dep_time, dep_time_range) = get_val_acc("LAST_DEPOSIT_TIME");
    let (dep_method, _) = get_val_acc("LAST_DEPOSIT_METHOD");
    let (wd_amount, wd_amt_range) = get_val_acc("LAST_WITHDRAWAL_AMOUNT");
    let (wd_time, wd_time_range) = get_val_acc("LAST_WITHDRAWAL_TIME");
    let (wd_method, _) = get_val_acc("LAST_WITHDRAWAL_METHOD");

    VerifyFinanceHistoryReq {
        finance_info: VerifyPlayerFinanceInfo {
            bc_number,
            bc_holder_name: bc_holder,
            ew_account,
            ew_holder_name: ew_holder,
            vw_address,
            vw_holder_name: vw_holder,
            ..Default::default()
        },
        history_info: VerifyPlayerHistoryInfo {
            last_deposit_amount: dep_amount,
            last_deposit_amount_range: dep_amt_range,
            last_deposit_method: dep_method,
            last_deposit_time: dep_time,
            last_deposit_time_range_in_day: dep_time_range.parse().unwrap_or(0),
            last_withdraw_amount: wd_amount,
            last_withdraw_amount_range: wd_amt_range,
            last_withdraw_method: wd_method,
            last_withdraw_time: wd_time,
            last_withdraw_time_range_in_day: wd_time_range.parse().unwrap_or(0),
        },
    }
}

/// Score the MCS finance-history response.
///
/// `rawScore` semantics: `3` = matched, `2` = empty match, other = not matched.
/// Ports Go's `calculateScoreForFinancialHistory`.
fn calculate_score_financial(
    resp: &VerifyFinanceHistoryResp,
    empty_score: i32,
    qas: &mut HashMap<String, QA>,
    question_cfg: &HashMap<String, domain::Question>,
) {
    let h = &resp.value.history_info;
    let f = &resp.value.finance_info;

    let score_fields: &[(&str, i32)] = &[
        ("BANK_ACCOUNT", f.bc_number),
        ("CARD_HOLDER_NAME", f.bc_holder_name),
        ("E_WALLET_ACCOUNT", f.ew_account),
        ("E_WALLET_NAME", f.ew_holder_name),
        ("VIRTUAL_WALLET_ADDRESS", f.vw_address),
        ("VIRTUAL_WALLET_NAME", f.vw_holder_name),
        ("LAST_DEPOSIT_AMOUNT", h.last_deposit_amount),
        ("LAST_DEPOSIT_TIME", h.last_deposit_time),
        ("LAST_DEPOSIT_METHOD", h.last_deposit_method),
        ("LAST_WITHDRAWAL_AMOUNT", h.last_withdraw_amount),
        ("LAST_WITHDRAWAL_TIME", h.last_withdraw_time),
        ("LAST_WITHDRAWAL_METHOD", h.last_withdraw_method),
    ];

    for (field_id, raw_score) in score_fields {
        let q_cfg = match question_cfg.get(*field_id) {
            Some(q) => q,
            None => continue,
        };

        let (score, is_correct) = match *raw_score {
            3 => (q_cfg.score, true),
            2 => (empty_score, false),
            _ => (0, false),
        };

        if !is_correct {
            warn!(
                "[MCSClient] rawScore={} for {} — {}",
                raw_score,
                field_id,
                if *raw_score == 2 {
                    "Empty Match"
                } else {
                    "Not Matched"
                }
            );
        }

        qas.insert(
            field_id.to_string(),
            QA {
                field_id: field_id.to_string(),
                field_type: q_cfg.field_type.clone(),
                correct: is_correct,
                score,
                total_score: q_cfg.score,
            },
        );
    }
}
