//! USS (User Service) HTTP client.
//!
//! Mirrors Go's `internal/client/uss` package.
//! Two APIs:
//!   - `GET /customer?customerName=&force=` → `CustomerInfo`
//!   - `PUT /password/reset-generate`        → `PasswordResetTokenResponse`
//!
//! Retry strategy: up to 3 attempts with 700 ms delay between each;
//! each attempt gets its own 5-second timeout.


use crate::config::HttpServiceConfig;
use common::error::{AppError, InfraError};
use chrono::NaiveDateTime;
use reqwest::{Client, Response};
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use std::time::Duration;
use tracing::{info, warn};

// ── Nullable scalar wrappers ─────────────────────────────────────────────────

/// A string field that may arrive as `null` or `""`.
/// `val` is always a valid (possibly empty) string; `valid` indicates
/// whether a non-null/non-empty value was present.
#[derive(Debug, Clone, Default)]
pub struct NullString {
    pub val: String,
    pub valid: bool,
}

impl NullString {
    pub fn as_str(&self) -> &str {
        &self.val
    }
}

impl std::fmt::Display for NullString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.val)
    }
}

impl<'de> Deserialize<'de> for NullString {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let opt: Option<String> = Option::deserialize(d)?;
        Ok(match opt {
            None => NullString {
                val: String::new(),
                valid: false,
            },
            Some(s) if s.is_empty() => NullString {
                val: String::new(),
                valid: false,
            },
            Some(s) => NullString {
                val: s,
                valid: true,
            },
        })
    }
}

impl Serialize for NullString {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        if self.valid {
            s.serialize_str(&self.val)
        } else {
            s.serialize_none()
        }
    }
}

/// A nullable `i32` field.
#[derive(Debug, Clone, Default)]
pub struct NullInt32 {
    pub val: i32,
    pub valid: bool,
}

impl std::fmt::Display for NullInt32 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.val)
    }
}

impl<'de> Deserialize<'de> for NullInt32 {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let opt: Option<i32> = Option::deserialize(d)?;
        Ok(match opt {
            None => NullInt32::default(),
            Some(v) => NullInt32 {
                val: v,
                valid: true,
            },
        })
    }
}

impl Serialize for NullInt32 {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        if self.valid {
            s.serialize_i32(self.val)
        } else {
            s.serialize_none()
        }
    }
}

/// A nullable `i64` field.
#[derive(Debug, Clone, Default)]
pub struct NullInt {
    pub val: i64,
    pub valid: bool,
}

impl std::fmt::Display for NullInt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.val)
    }
}

impl<'de> Deserialize<'de> for NullInt {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let opt: Option<i64> = Option::deserialize(d)?;
        Ok(match opt {
            None => NullInt::default(),
            Some(v) => NullInt {
                val: v,
                valid: true,
            },
        })
    }
}

impl Serialize for NullInt {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        if self.valid {
            s.serialize_i64(self.val)
        } else {
            s.serialize_none()
        }
    }
}

// ── FlexTime ─────────────────────────────────────────────────────────────────

/// Timestamp that arrives as `"2006-01-02 15:04:05"` or `null` from USS.
#[derive(Debug, Clone, Default)]
pub struct FlexTime(pub Option<NaiveDateTime>);

const FLEX_TIME_FMT: &str = "%Y-%m-%d %H:%M:%S";

impl FlexTime {
    /// Return `"YYYY-MM-DD"` or `""` when null.
    pub fn format_date(&self) -> String {
        self.0
            .map(|t| t.format("%Y-%m-%d").to_string())
            .unwrap_or_default()
    }
}

impl<'de> Deserialize<'de> for FlexTime {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let opt: Option<String> = Option::deserialize(d)?;
        match opt {
            None => Ok(FlexTime(None)),
            Some(ref s) if s.is_empty() || s.to_lowercase() == "null" => Ok(FlexTime(None)),
            Some(s) => {
                let dt =
                    NaiveDateTime::parse_from_str(&s, FLEX_TIME_FMT).map_err(de::Error::custom)?;
                Ok(FlexTime(Some(dt)))
            }
        }
    }
}

impl Serialize for FlexTime {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            None => s.serialize_none(),
            Some(t) => s.serialize_str(&t.format(FLEX_TIME_FMT).to_string()),
        }
    }
}

// ── USS data models ───────────────────────────────────────────────────────────

/// Player profile — subset of USS customer payload.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Profile {
    #[serde(rename = "regDate", default)]
    pub reg_date: FlexTime,
    #[serde(rename = "birthday", default)]
    pub birthday: FlexTime,
    #[serde(rename = "lastLoginTime", default)]
    pub last_login_time: FlexTime,
    #[serde(rename = "idNumber", default)]
    pub id_number: NullString,
    #[serde(rename = "zipcode", default)]
    pub zip_code: NullString,
    #[serde(rename = "qqNo", default)]
    pub qq_no: NullString,
    #[serde(rename = "lineId", default)]
    pub line_id: NullString,
    #[serde(rename = "whatsAppId", default)]
    pub whats_app_id: NullString,
    #[serde(rename = "facebookId", default)]
    pub facebook_id: NullString,
    #[serde(rename = "twitter", default)]
    pub twitter: NullString,
    #[serde(rename = "viber", default)]
    pub viber: NullString,
    #[serde(rename = "zalo", default)]
    pub zalo: NullString,
    #[serde(rename = "appleId", default)]
    pub apple_id: NullString,
    #[serde(rename = "payeeName", default)]
    pub payee_name: NullString,
    #[serde(rename = "mobileNo", default)]
    pub mobile_no: NullString,
    #[serde(rename = "address", default)]
    pub address: NullString,
    #[serde(rename = "nickname", default)]
    pub nickname: NullString,
    #[serde(rename = "customerName", default)]
    pub customer_name: NullString,
    #[serde(rename = "wechat", default)]
    pub wechat: NullString,
    #[serde(rename = "telegram", default)]
    pub telegram: NullString,
    #[serde(rename = "customerId", default)]
    pub customer_id: NullInt,
    #[serde(rename = "gender", default)]
    pub gender: NullInt32,
    #[serde(rename = "maritalStatus", default)]
    pub marital_status: NullInt32,
    #[serde(rename = "idType", default)]
    pub id_type: NullInt32,
    #[serde(rename = "sourceOfIncome", default)]
    pub source_of_income: NullInt32,
    #[serde(rename = "occupation", default)]
    pub occupation: NullInt32,
}

/// Customer additional / KYC info.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct CustomerAdditionalInfo {
    #[serde(rename = "permanentAddress", default)]
    pub permanent_address: NullString,
    #[serde(rename = "placeOfBirth", default)]
    pub place_of_birth: NullString,
    #[serde(rename = "nationality", default)]
    pub nationality: NullString,
    #[serde(rename = "region", default)]
    pub region: NullString,
    #[serde(rename = "kakao", default)]
    pub kakao: NullString,
    #[serde(rename = "google", default)]
    pub google: NullString,
    #[serde(rename = "customerId", default)]
    pub customer_id: NullInt,
    #[serde(rename = "usState", default)]
    pub us_state: NullInt32,
}

/// Top-level `value` object inside a USS `CustomerInfo` response.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct CustomerValue {
    #[serde(rename = "customerAdditionalInfo", default)]
    pub additional_info: CustomerAdditionalInfo,
    #[serde(rename = "customerName", default)]
    pub customer_name: NullString,
    #[serde(rename = "email", default)]
    pub email: NullString,
    #[serde(rename = "merchantCode", default)]
    pub merchant_code: NullString,
    #[serde(rename = "profile", default)]
    pub profile: Profile,
    #[serde(rename = "customerId", default)]
    pub customer_id: NullInt,
    #[serde(rename = "merchantId", default)]
    pub merchant_id: NullInt,
}

/// Full USS customer response.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct CustomerInfo {
    pub value: CustomerValue,
    pub success: bool,
}

/// Request body for `PUT /password/reset-generate`.
#[derive(Debug, Serialize)]
pub struct PasswordResetTokenRequest {
    #[serde(rename = "customerName")]
    pub customer_name: String,
    #[serde(rename = "merchantCode")]
    pub merchant_code: String,
}

/// Response body from `PUT /password/reset-generate`.
#[derive(Debug, Deserialize, Serialize)]
pub struct PasswordResetTokenResponse {
    pub value: String,
    pub success: bool,
}

// ── Client ───────────────────────────────────────────────────────────────────

const MAX_RETRIES: u32 = 3;
const RETRY_DELAY_MS: u64 = 700;
const REQ_TIMEOUT_SECS: u64 = 5;

#[derive(Clone)]
pub struct UssClient {
    http: Client,
    base_url: String,
    base_path: String,
}

impl UssClient {
    pub fn new(cfg: &HttpServiceConfig) -> anyhow::Result<Self> {
        let http = Client::builder()
            .pool_max_idle_per_host(30)
            .connection_verbose(false)
            .timeout(Duration::from_secs(
                REQ_TIMEOUT_SECS * (MAX_RETRIES as u64 + 1),
            ))
            .build()?;
        Ok(Self {
            http,
            base_url: cfg.host.clone(),
            base_path: cfg.base_path.clone(),
        })
    }

    // ── Retry helper ──────────────────────────────────────────────────────────

    async fn do_with_retry<F, Fut>(&self, mut build: F) -> Result<Response, AppError>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<Response, reqwest::Error>>,
    {
        let mut last_err: Option<reqwest::Error> = None;
        for attempt in 1..=MAX_RETRIES {
            match build().await {
                Ok(resp) => return Ok(resp),
                Err(e) => {
                    warn!(
                        "[USSClient] attempt {}/{} failed: {}",
                        attempt, MAX_RETRIES, e
                    );
                    last_err = Some(e);
                    if attempt < MAX_RETRIES {
                        tokio::time::sleep(Duration::from_millis(RETRY_DELAY_MS)).await;
                    }
                }
            }
        }
        Err(AppError::Infra(InfraError::Http(last_err.unwrap())))
    }

    // ── GET /customer ─────────────────────────────────────────────────────────

    /// `GET {host}/{basePath}customer?customerName=<name>&force=<force>`
    pub async fn get_customer(
        &self,
        customer_name: &str,
        force: bool,
    ) -> Result<CustomerInfo, AppError> {
        let url = format!(
            "{}/{}customer?customerName={}&force={}",
            self.base_url, self.base_path, customer_name, force
        );
        info!("[USSClient] GetCustomer url={}", url);

        let resp = self
            .do_with_retry(|| {
                self.http
                    .get(&url)
                    .header("Accept", "application/json")
                    .send()
            })
            .await?;

        let body = resp
            .json::<CustomerInfo>()
            .await
            .map_err(|e| AppError::Infra(InfraError::Http(e)))?;
        Ok(body)
    }

    // ── PUT /password/reset-generate ─────────────────────────────────────────

    /// `PUT {host}/{basePath}password/reset-generate`
    pub async fn generate_password_reset_token(
        &self,
        customer_name: &str,
        merchant_code: &str,
    ) -> Result<PasswordResetTokenResponse, AppError> {
        let url = format!(
            "{}/{}password/reset-generate",
            self.base_url, self.base_path
        );
        info!(
            "[USSClient] GeneratePasswordResetToken url={} customerName={} merchantCode={}",
            url, customer_name, merchant_code
        );

        let payload = PasswordResetTokenRequest {
            customer_name: customer_name.to_string(),
            merchant_code: merchant_code.to_string(),
        };

        let resp = self
            .do_with_retry(|| {
                self.http
                    .put(&url)
                    .header("Content-Type", "application/json")
                    .header("Accept", "application/json")
                    .json(&payload)
                    .send()
            })
            .await?;

        let body = resp
            .json::<PasswordResetTokenResponse>()
            .await
            .map_err(|e| AppError::Infra(InfraError::Http(e)))?;
        Ok(body)
    }
}
