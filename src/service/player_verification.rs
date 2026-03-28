/// PlayerVerification service.
///
/// Mirrors Go's `internal/service/player_verification.go`.
/// Contains rate limiting (Redis), WPS/USS/MCS orchestration, and scoring.
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use redis::AsyncCommands;

use crate::client::mcs::{McsClient, PlayerHeaders, VerifyFinanceHistoryReq};
use crate::client::uss::{CustomerPersonalInfoValue, UssClient};
use crate::client::wps::WpsClient;
use crate::error::AppError;
use crate::model::merchant_rule::{MerchantRule, MerchantRuleConfig, Question, QuestionInfo};
use crate::model::validation_record::{QA, QaMap, ValidationRecord};
use crate::repository::{MerchantRuleRepository, ValidationRecordRepository};
use crate::service::field_cache::get_field_config;
use crate::service::field_id_uss_mapping_cache::{
    build_field_id_uss_id_mapping_key, get_uss_mapping_config_sync,
};
use crate::service::finance_history::{FINANCE_SETTERS, HISTORY_SETTERS, apply_field_setters};
use crate::types::req::{SubmitVerifyRequest, VerifyDataItem};
use crate::types::resp::{MerchantRuleResponse, SubmitVerifyData};

// ── Financial-history field IDs (must not be scored client-side) ─────────────

static FINANCIAL_HISTORY_IDS: &[&str] = &[
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
];

fn is_financial_history(field_id: &str) -> bool {
    FINANCIAL_HISTORY_IDS.contains(&field_id)
}

/// Fields hidden from the question list but still scored via MCS.
/// Mirrors Go's `fieldIdBlockList`.
static FIELD_ID_BLOCK_LIST: &[&str] = &[
    "LAST_WITHDRAWAL_AMOUNT",
    "LAST_WITHDRAWAL_METHOD",
    "LAST_WITHDRAWAL_TIME",
    "LAST_DEPOSIT_AMOUNT",
    "LAST_DEPOSIT_METHOD",
    "LAST_DEPOSIT_TIME",
];

fn is_blocked_field(field_id: &str) -> bool {
    FIELD_ID_BLOCK_LIST.contains(&field_id)
}

// ── Redis rate-limit keys ─────────────────────────────────────────────────────

// Key format mirrors Go exactly:
//   ip_key  = "ucsfe:ql:ip:{customerIP}:{date}"
//   acct_key = "ucsfe:ql:acct:{merchantCode}@{customerName}:{date}"
const QUESTION_LIST_REDIS_DB: i32 = 2;

fn ip_key(ip: &str, date: &str) -> String {
    format!("ucsfe:ql:ip:{}:{}", ip, date)
}

fn acct_key(merchant: &str, customer: &str, date: &str) -> String {
    format!("ucsfe:ql:acct:{}@{}:{}", merchant, customer, date)
}

/// Compute (date_key, end_of_day_unix) from a single clock read.
fn today_and_eod() -> (String, i64) {
    let now = chrono::Local::now();
    let date_key = now.format("%Y%m%d").to_string();
    let eod = now
        .date_naive()
        .and_hms_opt(23, 59, 59)
        .expect("valid time")
        .and_local_timezone(chrono::Local)
        .unwrap();
    (date_key, eod.timestamp())
}

/// Atomic INCR + EXPIREAT via Lua (same script as Go version).
const INCR_WITH_TTL_SCRIPT: &str = r#"
local v = redis.call('INCR', KEYS[1])
if v == 1 then
    redis.call('EXPIREAT', KEYS[1], ARGV[1])
end
return v
"#;

static INCR_SCRIPT: once_cell::sync::Lazy<redis::Script> =
    once_cell::sync::Lazy::new(|| redis::Script::new(INCR_WITH_TTL_SCRIPT));

/// Get a cached Redis ConnectionManager for rate limiting (DB 2).
/// Returns a clone of the pre-built manager — O(1), no TCP overhead.
async fn get_rate_limit_redis() -> Result<redis::aio::ConnectionManager, AppError> {
    crate::infra::get_db_manager(QUESTION_LIST_REDIS_DB).map_err(|_| AppError::RedisNotFound)
}

// ── Service ───────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct PlayerVerificationService {
    merchant_repo: Arc<MerchantRuleRepository>,
    validation_repo: Arc<ValidationRecordRepository>,
    uss: Arc<UssClient>,
    mcs: Arc<McsClient>,
    wps: Arc<WpsClient>,
    redis: redis::aio::ConnectionManager,
}

impl PlayerVerificationService {
    pub fn new(
        merchant_repo: Arc<MerchantRuleRepository>,
        validation_repo: Arc<ValidationRecordRepository>,
        uss: Arc<UssClient>,
        mcs: Arc<McsClient>,
        wps: Arc<WpsClient>,
        redis: redis::aio::ConnectionManager,
    ) -> Self {
        Self {
            merchant_repo,
            validation_repo,
            uss,
            mcs,
            wps,
            redis,
        }
    }

    // ── GetQuestionList ───────────────────────────────────────────────────────

    pub async fn get_question_list(
        &self,
        merchant_code: &str,
        customer_ip: &str,
        customer_name: &str,
        language: &str,
    ) -> Result<MerchantRuleResponse, AppError> {
        // ── Phase 1: DB + Redis GET in parallel ──────────────────────────
        // Redis keys only depend on request params, not on the DB result,
        // so we can fire both at the same time.
        let (today, expire_at) = today_and_eod();
        let ip_k = ip_key(customer_ip, &today);
        let ac_k = acct_key(merchant_code, customer_name, &today);

        // Use Redis DB 2 for rate limiting (mirrors Go's questionListRedisDB = 2).
        let rate_limit_redis = get_rate_limit_redis().await?;
        let mut c1 = rate_limit_redis.clone();
        let mut c2 = rate_limit_redis.clone();
        let ip_k_r = ip_k.clone();
        let ac_k_r = ac_k.clone();

        let (db_result, ip_cnt, ac_cnt) = tokio::try_join!(
            async {
                self.merchant_repo
                    .find_by_merchant_code(merchant_code)
                    .await
                    .map_err(|e| AppError::Internal(e))
            },
            async {
                let v: Option<i64> = redis::AsyncCommands::get(&mut c1, &ip_k_r)
                    .await
                    .map_err(AppError::RedisError)?;
                Ok::<i64, AppError>(v.unwrap_or(0))
            },
            async {
                let v: Option<i64> = redis::AsyncCommands::get(&mut c2, &ac_k_r)
                    .await
                    .map_err(AppError::RedisError)?;
                Ok::<i64, AppError>(v.unwrap_or(0))
            },
        )?;

        let merchant_rule =
            db_result.ok_or_else(|| AppError::MerchantNotFound(merchant_code.to_string()))?;

        // ── Rate-limit check ─────────────────────────────────────────────
        let ip_limit = merchant_rule.ip_retry_limit as i64;
        let acct_limit = merchant_rule.account_retry_limit as i64;

        tracing::info!(
            merchant_code,
            customer_name,
            customer_ip,
            today,
            ip_cnt,
            ip_limit,
            ac_cnt,
            acct_limit,
            "retryLimit check"
        );

        if ip_cnt >= ip_limit || ac_cnt >= acct_limit {
            tracing::warn!(
                merchant_code,
                customer_name,
                customer_ip,
                ip_cnt,
                ip_limit,
                ac_cnt,
                acct_limit,
                "question retry limit exhausted"
            );
            return Err(AppError::QuestionLimitExceeded);
        }

        // ── Phase 2: Redis INCR + WPS + USS — all three in parallel ─────
        // INCR (82ms) hides completely behind the slower HTTP calls (~176ms).
        let full_customer_name = format!("{}@{}", merchant_code, customer_name);
        let script = &*INCR_SCRIPT;
        let mut incr_c1 = rate_limit_redis.clone();
        let mut incr_c2 = rate_limit_redis.clone();
        let t = Instant::now();

        let (_, wps_result, uss_result) = tokio::join!(
            // Redis INCR (fire-and-log, never blocks the response)
            async {
                let mut k1 = script.key(&ip_k);
                let inv1 = k1.arg(expire_at);
                let mut k2 = script.key(&ac_k);
                let inv2 = k2.arg(expire_at);
                let (r1, r2) = tokio::join!(
                    inv1.invoke_async::<i64>(&mut incr_c1),
                    inv2.invoke_async::<i64>(&mut incr_c2),
                );
                if let Err(e) = r1 {
                    tracing::warn!(error = %e, key = %ip_k, "redis incr failed");
                }
                if let Err(e) = r2 {
                    tracing::warn!(error = %e, key = %ac_k, "redis incr failed");
                }
            },
            // WPS HTTP
            self.wps.get_reset_password_status(merchant_code),
            // USS HTTP
            self.uss.get_customer(&full_customer_name, false),
        );

        let wps_resp = wps_result.map_err(|e| AppError::WpsApiFailed(e.to_string()))?;
        let customer = uss_result.map_err(|e| AppError::CustomerFetchFailed(e.to_string()))?;

        tracing::info!(
            merchant_code,
            elapsed_ms = %t.elapsed().as_millis(),
            wps_email = wps_resp.value.is_email_reset_enabled,
            wps_sms   = wps_resp.value.is_sms_reset_enabled,
            "[INCR+WPS+USS] parallel done"
        );

        if !wps_resp.success {
            return Err(AppError::WpsApiFailed("success=false".to_string()));
        }

        let wps_email = wps_resp.value.is_email_reset_enabled;
        let wps_sms = wps_resp.value.is_sms_reset_enabled;
        let verification_mode = &customer.value.profile.verification_mode.val;
        let email_verification = customer.value.customer_additional_info.email_verification;

        tracing::info!(
            wps_email, wps_sms, email_verification, verification_mode = %verification_mode,
            "WPS-USS flags"
        );

        if wps_email && email_verification {
            return Err(AppError::EmailAlreadyBound);
        }

        if wps_sms && verification_mode == "0" {
            return Err(AppError::PhoneAlreadyBound);
        }

        // ── Build question list ──────────────────────────────────────────
        let translations = merchant_rule.get_translations_by_language(language);
        let questions = self
            .get_valid_question_infos(&merchant_rule, merchant_code, &translations)
            .await?;

        Ok(MerchantRuleResponse {
            merchant_code: merchant_code.to_string(),
            questions,
        })
    }

    // ── SubmitVerifyMaterials ─────────────────────────────────────────────────

    pub async fn submit_verify_materials(
        &self,
        merchant_code: &str,
        customer_ip: &str,
        req_body: SubmitVerifyRequest,
    ) -> Result<SubmitVerifyData, AppError> {
        let customer_name = format!("{}@{}", merchant_code, req_body.customer_name);

        // ── Phase 1: Fire customer + token + rule_config in parallel ──────
        //
        // - get_customer needs customer_name (from input)
        // - generate_token needs customer_name + merchant_code (from input)
        // - get_rule_config needs merchant_code (from input)
        // None depend on each other — all three can start immediately.
        let t_phase1 = Instant::now();

        let (cust_result, token_result, rule_result) = tokio::join!(
            async {
                let t = Instant::now();
                let r = self.uss.get_customer(&customer_name, false).await;
                tracing::info!(merchant_code, customer_name = %req_body.customer_name, elapsed_ms = %t.elapsed().as_millis(), "[USSClient] GetCustomer");
                r
            },
            async {
                let t = Instant::now();
                let r = self
                    .uss
                    .generate_password_reset_token(&req_body.customer_name, merchant_code)
                    .await;
                tracing::info!(merchant_code, elapsed_ms = %t.elapsed().as_millis(), "[USSClient] GeneratePasswordResetToken");
                r
            },
            async {
                let t = Instant::now();
                let r = self.merchant_repo.get_rule_config(merchant_code).await;
                tracing::info!(merchant_code, elapsed_ms = %t.elapsed().as_millis(), "[Oracle] GetRuleConfig");
                r
            },
        );

        let customer = cust_result.map_err(|e| AppError::CustomerFetchFailed(e.to_string()))?;
        tracing::info!(
            customer_id = customer.value.customer_id.val,
            customer_name = %customer_name,
            "CustomerInfo fetched"
        );

        let token_resp = token_result.map_err(|e| AppError::PasswordResetFailed(e.to_string()))?;
        if !token_resp.success {
            tracing::warn!(customer_name = %customer_name, "USS GeneratePasswordResetToken returned failure");
            return Err(AppError::PasswordResetFailed(
                "USS returned failure".to_string(),
            ));
        }
        let one_time_passwd = token_resp.value.clone();

        let rule_cfg = rule_result
            .map_err(|e| AppError::Internal(e))?
            .ok_or_else(|| AppError::MerchantNotFound(merchant_code.to_string()))?;

        tracing::info!(merchant_code, elapsed_ms = %t_phase1.elapsed().as_millis(), "Phase 1 (customer + token + rule-config) complete");

        // ── Phase 2: Fetch personal-info (needs customer_id from Phase 1) ─
        let t = Instant::now();
        let customer_personal_info = self
            .uss
            .get_customer_personal_info(customer.value.customer_id.val)
            .await
            .map_err(|e| AppError::CustomerPersonalInfoFetchFailed(e.to_string()))?;
        tracing::info!(merchant_code, elapsed_ms = %t.elapsed().as_millis(), "[USSClient] GetCustomerPersonalInfo");
        tracing::info!(
            customer_id = customer.value.customer_id.val,
            customer_name = %customer_name,
            raw_kakao = %customer_personal_info.kakao.val,
            "CustomerPersonalInfo fetched"
        );

        // 5. Parse merchant question configuration
        let questions_map: HashMap<String, Question> = rule_cfg.parse_questions().map_err(|e| {
            tracing::warn!(merchant_code, error = %e, "QUESTIONS CLOB unmarshal failed");
            AppError::ParseJsonFailed(format!("merchantCode={}", merchant_code))
        })?;

        // 6. Score profile fields (tracking submitted IDs)
        let mut submitted_field_ids: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        let mut qa_map: QaMap = HashMap::new();
        accurate_judgment_score(
            &mut submitted_field_ids,
            &req_body.data,
            &customer,
            &customer_personal_info,
            &rule_cfg,
            &mut qa_map,
            &questions_map,
        );

        // 7. Build MCS request and call VerifyPlayerInfo
        let mut field_id_map: HashMap<String, String> = HashMap::new();
        let mut bind_map: HashMap<String, bool> = HashMap::new();
        let mut mcs_req = VerifyFinanceHistoryReq::default();

        for item in &req_body.data {
            if is_financial_history(&item.item.field_id) {
                field_id_map.insert(item.item.field_id.clone(), item.item.field_value.clone());
                bind_map.insert(item.item.field_id.clone(), item.bind);
            }
        }
        apply_field_setters(
            FINANCE_SETTERS,
            &mut mcs_req,
            &field_id_map,
            &bind_map,
            &questions_map,
        );
        apply_field_setters(
            HISTORY_SETTERS,
            &mut mcs_req,
            &field_id_map,
            &bind_map,
            &questions_map,
        );

        let mcs_headers = PlayerHeaders {
            customer_id: customer.value.customer_id.val.to_string(),
            customer_name: req_body.customer_name.clone(),
            merchant: merchant_code.to_string(),
            customer_ip: customer_ip.to_string(),
        };

        let t = Instant::now();
        let mcs_resp = self
            .mcs
            .verify_player_info(&mcs_headers, &mcs_req)
            .await
            .map_err(|e| AppError::VerifyPlayerInfoFailed(e.to_string()))?;
        tracing::info!(merchant_code, elapsed_ms = %t.elapsed().as_millis(), "[MCSClient] VerifyPlayerInfo");

        // 8. Score financial history from MCS response
        tracing::info!("[MCSClient] response={}", mcs_resp);
        calculate_score_for_financial_history(
            &submitted_field_ids,
            &mcs_resp,
            &rule_cfg,
            &mut qa_map,
            &questions_map,
        );

        // 9. Serialise QA map and compute total score
        let qas_bytes = serde_json::to_string(&qa_map)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("marshal qas: {}", e)))?;

        let actual_score: i32 = qa_map.values().map(|qa| qa.score).sum();
        let score_checked = actual_score >= rule_cfg.passing_score;

        tracing::info!(
            customer_id = customer.value.customer_id.val,
            merchant_code,
            actual_score,
            passing_score = rule_cfg.passing_score,
            score_checked,
            "Score calculated"
        );

        // 10. Insert validation record
        let record = ValidationRecord {
            id: None,
            customer_id: customer.value.customer_id.val,
            customer_name: customer_name.clone(),
            success: if score_checked { 1 } else { 0 },
            merchant_code: merchant_code.to_string(),
            ip: customer_ip.to_string(),
            passing_score: rule_cfg.passing_score,
            score: actual_score,
            qas: qas_bytes,
            created_at: chrono::Local::now().naive_local(),
        };

        let vr = self.validation_repo.clone();
        let cid = customer.value.customer_id.val;
        let mc = merchant_code.to_string();
        tokio::spawn(async move {
            if let Err(e) = vr.insert(record).await {
                tracing::warn!(error = %e, "insert validation record failed");
            } else {
                tracing::info!(customer_id = cid, merchant_code = %mc, "insert success");
            }
        });

        let mut data = SubmitVerifyData {
            score_checked,
            bind_type: None,
            one_time_password: None,
        };
        if score_checked {
            data.bind_type = Some(rule_cfg.binding_type.clone());
            data.one_time_password = Some(one_time_passwd);
        }

        Ok(data)
    }

    // ── Question list builder ─────────────────────────────────────────────────

    /// Mirrors Go's `getValidQuestionInfos(m *model.MerchantRule, merchantCode string)`:
    ///   1. Parse raw QUESTIONS CLOB as `HashMap<String, Question>`
    ///   2. Filter by `valid == true && field_id != ""`
    ///   3. Enrich DD fields with dropdown from cache
    ///   4. Return `Result` — errors are propagated, never silently swallowed
    async fn get_valid_question_infos(
        &self,
        rule: &MerchantRule,
        merchant_code: &str,
        translations: &HashMap<String, String>,
    ) -> Result<Vec<QuestionInfo>, AppError> {
        let raw = rule.questions_json.as_deref().unwrap_or("");
        if raw.is_empty() {
            tracing::warn!(merchant_code, "questions field is empty");
            return Err(AppError::ParseJsonFailed(format!(
                "merchantCode={}",
                merchant_code
            )));
        }

        let all: std::collections::HashMap<String, Question> =
            serde_json::from_str(raw).map_err(|e| {
                tracing::warn!(merchant_code, error = %e, "unmarshal questions failed");
                AppError::ParseJsonFailed(format!("merchantCode={}", merchant_code))
            })?;

        let dd_map = get_field_config(merchant_code).await;

        let mut result = Vec::with_capacity(all.len());
        for (_, q) in all {
            if !q.valid || q.field_id.is_empty() || is_blocked_field(&q.field_id) {
                continue;
            }
            let dropdown = if q.field_attribute == "DD" {
                dd_map
                    .as_ref()
                    .and_then(|m| m.get(q.field_id.as_str()))
                    .cloned()
            } else {
                None
            };
            let field_name = translations
                .get(&q.field_id)
                .cloned()
                .unwrap_or(q.field_name);
            result.push(QuestionInfo {
                field_id: q.field_id,
                field_name,
                field_attribute: q.field_attribute,
                field_type: q.field_type,
                field_dropdown_list: dropdown,
            });
        }

        result.sort_by(|a, b| a.field_id.cmp(&b.field_id));

        tracing::info!(
            merchant_code,
            valid_questions = result.len(),
            "getValidQuestionInfos done"
        );

        Ok(result)
    }
}

// ── Scoring helpers ───────────────────────────────────────────────────────────

/// Score 32 profile fields submitted by the player against USS customer data.
///
/// Mirrors Go's `accurateJudgmentScore` line-by-line:
///   bind=true:
///     1. submitted == ""                       → score 0, correct false
///     2. submitted == expected (case-insensitive) → score full, correct true
///     3. submitted != expected                 → score 0, correct false
///   bind=false (ignore submitted, check actual):
///     1. expected == "" || expected == "-1"     → score emptyScore, correct true
///     2. otherwise                              → score 0, correct false
fn accurate_judgment_score(
    submitted_field_ids: &mut std::collections::HashSet<String>,
    data_items: &[VerifyDataItem],
    customer: &crate::client::uss::CustomerInfo,
    customer_personal_info: &CustomerPersonalInfoValue,
    rule_cfg: &MerchantRuleConfig,
    qa_map: &mut QaMap,
    field_id_map_cfg: &HashMap<String, Question>,
) {
    let p = &customer.value.profile;
    let a = &customer.value.customer_additional_info;
    let empty_score = rule_cfg.empty_score;

    // Lazy field value lookup — avoids building a full HashMap and cloning all strings.
    // Returns "" for unknown fields (mirrors the original HashMap miss path).
    let lookup_field = |field_id: &str| -> std::borrow::Cow<str> {
        match field_id {
            "PLACE_OF_BIRTH" => std::borrow::Cow::Borrowed(a.place_of_birth.val.as_str()),
            "MARITAL_STATUS" => std::borrow::Cow::Owned(p.marital_status.val.to_string()),
            "NICKNAME" => std::borrow::Cow::Borrowed(p.nickname.val.as_str()),
            "TAG_REGION" => std::borrow::Cow::Borrowed(a.region.val.as_str()),
            "QQ" => std::borrow::Cow::Borrowed(p.qq_no.val.as_str()),
            "WECHAT_ID" => std::borrow::Cow::Borrowed(p.wechat.val.as_str()),
            "LINE_ID" => std::borrow::Cow::Borrowed(p.line_id.val.as_str()),
            "FB_ID" => std::borrow::Cow::Borrowed(p.face_book_id.val.as_str()),
            "WHATSAPP" => std::borrow::Cow::Borrowed(p.whats_app_id.val.as_str()),
            "ZALO" => std::borrow::Cow::Borrowed(p.zalo.val.as_str()),
            "TELEGRAM" => std::borrow::Cow::Borrowed(p.telegram.val.as_str()),
            "VIBER" => std::borrow::Cow::Borrowed(p.viber.val.as_str()),
            "TWITTER" => std::borrow::Cow::Borrowed(p.twitter.val.as_str()),
            "EMAIL" => std::borrow::Cow::Borrowed(customer.value.email.val.as_str()),
            "MOBILE_NUMBER" => std::borrow::Cow::Borrowed(p.mobile_no.val.as_str()),
            "FIXED_ADDRESS" => std::borrow::Cow::Borrowed(a.permanent_address.val.as_str()),
            "ADDRESS" => std::borrow::Cow::Borrowed(p.address.val.as_str()),
            "WITHDRAWER_NAME" => std::borrow::Cow::Borrowed(p.payee_name.val.as_str()),
            "DATE_OF_BIRTH" => std::borrow::Cow::Owned(p.birthday.format_date()),
            "NATIONALITY" => std::borrow::Cow::Borrowed(a.nationality.val.as_str()),
            "STATE" => std::borrow::Cow::Owned(a.us_state.val.to_string()),
            "ID" => std::borrow::Cow::Borrowed(p.id_number.val.as_str()),
            "GENDER" => std::borrow::Cow::Owned(p.gender.val.to_string()),
            "JOB" => std::borrow::Cow::Owned(p.occupation.val.to_string()),
            "SOURCE_OF_INCOME" => std::borrow::Cow::Owned(p.source_of_income.val.to_string()),
            "ID_TYPE" => std::borrow::Cow::Owned(p.id_type.val.to_string()),
            "ZIP_CODE" => std::borrow::Cow::Borrowed(p.zip_code.val.as_str()),
            "APPLE_ID" => std::borrow::Cow::Borrowed(p.apple_id.val.as_str()),
            "KAKAO" => std::borrow::Cow::Borrowed(customer_personal_info.kakao.val.as_str()),
            "GOOGLE" => std::borrow::Cow::Borrowed(a.google.val.as_str()),
            "LAST_LOGIN_TIME" => std::borrow::Cow::Owned(p.last_login_time.format_date()),
            "REGISTRATION_TIME" => std::borrow::Cow::Owned(p.reg_date.format_date()),
            _ => std::borrow::Cow::Borrowed(""),
        }
    };

    for data_item in data_items {
        let field_id = &data_item.item.field_id;
        submitted_field_ids.insert(field_id.clone());

        if is_financial_history(field_id) {
            continue;
        }

        // Go: `expectedValue, ok1 := fieldLookup[fieldId]`
        // Go: `questionValue, ok2 := fieldIdMapCfg[fieldId]`
        // Go: `if !ok1 && !ok2 { continue }`
        let ok2 = field_id_map_cfg.contains_key(field_id.as_str());
        let expected_value_cow = lookup_field(field_id.as_str());
        let ok1 = !expected_value_cow.is_empty()
            || matches!(
                field_id.as_str(),
                "PLACE_OF_BIRTH"
                    | "MARITAL_STATUS"
                    | "NICKNAME"
                    | "TAG_REGION"
                    | "QQ"
                    | "WECHAT_ID"
                    | "LINE_ID"
                    | "FB_ID"
                    | "WHATSAPP"
                    | "ZALO"
                    | "TELEGRAM"
                    | "VIBER"
                    | "TWITTER"
                    | "EMAIL"
                    | "MOBILE_NUMBER"
                    | "FIXED_ADDRESS"
                    | "ADDRESS"
                    | "WITHDRAWER_NAME"
                    | "DATE_OF_BIRTH"
                    | "NATIONALITY"
                    | "STATE"
                    | "ID"
                    | "GENDER"
                    | "JOB"
                    | "SOURCE_OF_INCOME"
                    | "ID_TYPE"
                    | "ZIP_CODE"
                    | "APPLE_ID"
                    | "KAKAO"
                    | "GOOGLE"
                    | "LAST_LOGIN_TIME"
                    | "REGISTRATION_TIME"
            );

        if !ok1 && !ok2 {
            tracing::warn!(field_id = %field_id, "unknown fieldId");
            continue;
        }

        let expected_value: &str = &expected_value_cow;
        let question_cfg = match field_id_map_cfg.get(field_id) {
            Some(q) => q,
            None => continue,
        };

        // Field Id USS Id mapping: translate MCS FieldValue → USS_ID string.
        // Mirrors Go's `GlobalUssMappingConfigs.Load(fieldUssIdCacheKey)`.
        let raw_submitted = &data_item.item.field_value;
        let submitted: String;
        if let Some(uss_id) = get_uss_mapping_config_sync(field_id, raw_submitted) {
            tracing::info!(
                field_id = %field_id,
                raw_value = %raw_submitted,
                uss_id = %uss_id,
                "field id uss id mapping cache hit"
            );
            submitted = uss_id;
        } else {
            submitted = raw_submitted.clone();
        }

        let (score, is_correct);
        if data_item.bind {
            if submitted.is_empty() {
                score = 0;
                is_correct = false;
                tracing::warn!(field_id = %field_id, expected = %expected_value, "bind=true empty value");
            } else if submitted.to_lowercase() == expected_value.to_lowercase() {
                score = question_cfg.score;
                is_correct = true;
                tracing::info!(
                    field_id = %field_id, submitted = %submitted, expected = %expected_value, score,
                    "bind=true matched"
                );
            } else if !question_cfg.accuracy.is_empty() && question_cfg.accuracy != "exact" {
                match match_with_accuracy(&submitted, expected_value, &question_cfg.accuracy) {
                    Ok((true, detail)) => {
                        score = question_cfg.score;
                        is_correct = true;
                        tracing::info!(
                            field_id = %field_id, submitted = %submitted, expected = %expected_value,
                            detail = %detail, score,
                            "bind=true accuracy matched"
                        );
                    }
                    Ok((false, detail)) => {
                        score = 0;
                        is_correct = false;
                        tracing::warn!(
                            field_id = %field_id, submitted = %submitted, expected = %expected_value,
                            detail = %detail,
                            "bind=true accuracy mismatched"
                        );
                    }
                    Err(e) => {
                        score = 0;
                        is_correct = false;
                        tracing::warn!(
                            field_id = %field_id, submitted = %submitted, expected = %expected_value,
                            error = %e,
                            "bind=true accuracy error"
                        );
                    }
                }
            } else {
                score = 0;
                is_correct = false;
                tracing::warn!(
                    field_id = %field_id, submitted = %submitted, expected = %expected_value,
                    "bind=true mismatched"
                );
            }
        } else {
            if expected_value.is_empty() || expected_value == "-1" {
                score = empty_score;
                is_correct = true;
                tracing::info!(
                    field_id = %field_id, expected = %expected_value, empty_score,
                    "bind=false matched"
                );
            } else {
                score = 0;
                is_correct = false;
                tracing::warn!(
                    field_id = %field_id, expected = %expected_value,
                    "bind=false mismatched"
                );
            }
        }

        qa_map.insert(
            field_id.clone(),
            QA {
                field_id: field_id.clone(),
                field_type: question_cfg.field_type.clone(),
                correct: is_correct,
                score,
                total_score: question_cfg.score,
            },
        );
    }
}

/// Map MCS rawScore (3=matched, 2=empty-match, else=not-matched) → score/isCorrect.
///
/// Mirrors Go's `calculateScoreForFinancialHistory`: only scores fields that were
/// actually submitted (present in `submitted_field_ids`).
fn calculate_score_for_financial_history(
    submitted_field_ids: &std::collections::HashSet<String>,
    resp: &crate::client::mcs::VerifyFinanceHistoryResp,
    rule_cfg: &MerchantRuleConfig,
    qa_map: &mut QaMap,
    field_id_map_cfg: &HashMap<String, Question>,
) {
    let empty_score = rule_cfg.empty_score;

    let score_fields: &[(&str, i32)] = &[
        (
            "BANK_ACCOUNT",
            resp.value.verify_player_finance_info.bc_number,
        ),
        (
            "CARD_HOLDER_NAME",
            resp.value.verify_player_finance_info.bc_holder_name,
        ),
        (
            "E_WALLET_ACCOUNT",
            resp.value.verify_player_finance_info.ew_account,
        ),
        (
            "E_WALLET_NAME",
            resp.value.verify_player_finance_info.ew_holder_name,
        ),
        (
            "VIRTUAL_WALLET_ADDRESS",
            resp.value.verify_player_finance_info.vw_address,
        ),
        (
            "VIRTUAL_WALLET_NAME",
            resp.value.verify_player_finance_info.vw_holder_name,
        ),
        (
            "LAST_DEPOSIT_AMOUNT",
            resp.value.verify_player_history_info.last_deposit_amount,
        ),
        (
            "LAST_DEPOSIT_TIME",
            resp.value.verify_player_history_info.last_deposit_time,
        ),
        (
            "LAST_DEPOSIT_METHOD",
            resp.value.verify_player_history_info.last_deposit_method,
        ),
        (
            "LAST_WITHDRAWAL_AMOUNT",
            resp.value.verify_player_history_info.last_withdraw_amount,
        ),
        (
            "LAST_WITHDRAWAL_TIME",
            resp.value.verify_player_history_info.last_withdraw_time,
        ),
        (
            "LAST_WITHDRAWAL_METHOD",
            resp.value.verify_player_history_info.last_withdraw_method,
        ),
    ];

    for (field_id, raw_score) in score_fields {
        let q = match field_id_map_cfg.get(*field_id) {
            Some(q) => q,
            None => continue,
        };
        if !submitted_field_ids.contains(*field_id) {
            continue;
        }

        // Go: 3=matched(true), 2=empty-match(true), else=not-matched(false)
        let (score, is_correct, msg) = match *raw_score {
            3 => (q.score, true, "Matched"),
            2 => (empty_score, true, "Empty Match"),
            _ => (0, false, "Not Matched"),
        };

        if !is_correct {
            tracing::warn!(
                raw_score, msg, field_id = %field_id,
                "[MCSClient] score result"
            );
        }

        qa_map.insert(
            field_id.to_string(),
            QA {
                field_id: field_id.to_string(),
                field_type: q.field_type.clone(),
                correct: is_correct,
                score,
                total_score: q.score,
            },
        );
    }
}

// ── Accuracy-based matching helpers ───────────────────────────────────────────

/// Dispatch to date-range or amount-range matching based on accuracy format.
/// A trailing '%' means amount matching; otherwise date/duration matching.
fn match_with_accuracy(
    submitted_value: &str,
    expected_value: &str,
    accuracy: &str,
) -> Result<(bool, String)> {
    if accuracy.ends_with('%') {
        match_amount_with_accuracy(submitted_value, expected_value, accuracy)
    } else {
        match_date_with_accuracy(submitted_value, expected_value, accuracy)
    }
}

/// Check whether `expected_value` falls within `[submitted - dur, submitted + dur]`.
fn match_date_with_accuracy(
    submitted_value: &str,
    expected_value: &str,
    accuracy: &str,
) -> Result<(bool, String)> {
    let dur = parse_accuracy_duration(accuracy)?;
    let submitted = chrono::NaiveDate::parse_from_str(submitted_value, "%Y-%m-%d")
        .map_err(|e| anyhow::anyhow!("parse submitted date '{}': {}", submitted_value, e))?;
    let expected = chrono::NaiveDate::parse_from_str(expected_value, "%Y-%m-%d")
        .map_err(|e| anyhow::anyhow!("parse expected date '{}': {}", expected_value, e))?;

    let lo = submitted - dur;
    let hi = submitted + dur;
    let matched = expected >= lo && expected <= hi;

    let detail = format!(
        "dateRange=[{} ~ {}], expected={}, inRange={}",
        lo.format("%Y-%m-%d"),
        hi.format("%Y-%m-%d"),
        expected.format("%Y-%m-%d"),
        matched,
    );
    Ok((matched, detail))
}

/// Check whether `expected_value` falls within `[submitted - delta, submitted + delta]`
/// where `delta = |submitted| * pct / 100`.
fn match_amount_with_accuracy(
    submitted_value: &str,
    expected_value: &str,
    accuracy: &str,
) -> Result<(bool, String)> {
    let pct_str = accuracy.trim_end_matches('%');
    let pct: f64 = pct_str
        .parse()
        .map_err(|e| anyhow::anyhow!("parse accuracy pct '{}': {}", pct_str, e))?;
    let submitted: f64 = submitted_value
        .parse()
        .map_err(|e| anyhow::anyhow!("parse submitted amount '{}': {}", submitted_value, e))?;
    let expected: f64 = expected_value
        .parse()
        .map_err(|e| anyhow::anyhow!("parse expected amount '{}': {}", expected_value, e))?;

    let delta = submitted.abs() * pct / 100.0;
    let lo = submitted - delta;
    let hi = submitted + delta;
    let matched = expected >= lo && expected <= hi;

    let detail = format!(
        "amountRange=[{:.2} ~ {:.2}], expected={:.2}, inRange={}",
        lo, hi, expected, matched,
    );
    Ok((matched, detail))
}

/// Parse a duration string like "3d", "12h", "30m" into a `chrono::Duration`.
fn parse_accuracy_duration(s: &str) -> Result<chrono::Duration> {
    let s = s.trim();
    anyhow::ensure!(s.len() >= 2, "accuracy duration too short: '{}'", s);

    let unit = s.as_bytes()[s.len() - 1];
    let n: i64 = s[..s.len() - 1]
        .parse()
        .map_err(|e| anyhow::anyhow!("parse duration number '{}': {}", &s[..s.len() - 1], e))?;

    match unit {
        b'd' | b'D' => Ok(chrono::Duration::days(n)),
        b'h' | b'H' => Ok(chrono::Duration::hours(n)),
        b'm' | b'M' => Ok(chrono::Duration::minutes(n)),
        _ => anyhow::bail!("unknown duration unit '{}' in '{}'", unit as char, s),
    }
}
