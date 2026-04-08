/// USS (User Service) HTTP client — full port of Go's `internal/client/uss/`.
///
/// Files ported:
///   error.go  → [`UssHttpError`], [`is_http_error`], [`is_profile_not_found`]
///   model.go  → [`NullString`], [`NullInt32`], [`NullInt`], [`FlexTime`],
///               [`Profile`], [`CustomerAdditionalInfo`], [`Value`], [`CustomerInfo`],
///               [`PasswordResetTokenRequest`], [`PasswordResetTokenResponse`]
///   client.go → [`UssClient`]  (retry · per-attempt timeout · 100 KB body cap)
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Deserializer, Serialize, de};

// ── Serde helpers ─────────────────────────────────────────────────────────────

/// Deserialize a JSON bool-or-null into `bool`.
/// `null` → `false` (mirrors Go's `bool` zero value when unmarshalling null).
fn bool_from_null_or_bool<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<bool, D::Error> {
    Ok(Option::<bool>::deserialize(d)?.unwrap_or(false))
}

/// Deserialize a JSON integer-or-null into `i32`.
/// `null` → `0`.
fn i32_from_null_or_int<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<i32, D::Error> {
    Ok(Option::<i32>::deserialize(d)?.unwrap_or(0))
}

/// Go's `UnmarshalJSON` pre-sets `UsState = NullInt32{Val: -1}` before unmarshalling,
/// so when the `"usState"` key is absent, serde needs to use this instead of `NullInt32::default()`.
const fn default_us_state() -> NullInt32 {
    NullInt32 {
        val: -1,
        valid: false,
    }
}

/// Deserialize `Option<T>` where an explicit JSON `null` maps to `T::default()`.
///
/// `#[serde(default)]` only fires when a key is *absent* from the object.
/// When the key is present with value `null`, serde tries to call `T::deserialize(null)`,
/// which fails for structs that don't implement that.  This helper handles both cases.
fn null_as_default<'de, D, T>(d: D) -> std::result::Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}
use tokio::time::sleep;

use crate::config::ServiceConfig;

// ═══════════════════════════════════════════════════════════════════════════════
// error.go
// ═══════════════════════════════════════════════════════════════════════════════

/// Non-200 HTTP response returned by the USS service.
/// Mirrors Go's `uss.HTTPError`.
#[derive(Debug, Clone)]
pub struct UssHttpError {
    pub body: String,
    pub status: u16,
}

impl std::fmt::Display for UssHttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.body)
    }
}

impl std::error::Error for UssHttpError {}

/// Mirrors Go's `uss.IsHTTPError`.
pub fn is_http_error(err: &anyhow::Error) -> bool {
    err.downcast_ref::<UssHttpError>().is_some()
}

/// Mirrors Go's `uss.IsProfileNotFound`.
///
/// Returns `true` when the USS error body contains
/// `"uss-ae.profile.data_not_found"` or `"profile.data_not_found"`.
pub fn is_profile_not_found(err: &anyhow::Error) -> bool {
    err.downcast_ref::<UssHttpError>().is_some_and(|e| {
        e.body.contains("uss-ae.profile.data_not_found")
            || e.body.contains("profile.data_not_found")
    })
}

// ═══════════════════════════════════════════════════════════════════════════════
// model.go — Null-safe scalar types
// ═══════════════════════════════════════════════════════════════════════════════

/// Mirrors Go's `FlexTime`.
///
/// Accepts `"2006-01-02 15:04:05"` and `null` / `""` from JSON.
/// `format_date()` returns `"YYYY-MM-DD"` or `""`.
#[derive(Debug, Clone, Default)]
pub struct FlexTime(pub Option<chrono::NaiveDateTime>);

const FLEX_TIME_FMT: &str = "%Y-%m-%d %H:%M:%S";

impl FlexTime {
    /// Returns `"YYYY-MM-DD"` or `""` if the value is null/zero.
    /// Mirrors Go's `FlexTime.FormatDate()`.
    pub fn format_date(&self) -> String {
        self.0.map(|dt| dt.format("%Y-%m-%d").to_string()).unwrap_or_default()
    }
}

impl Serialize for FlexTime {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        match self.0 {
            Some(dt) => s.serialize_str(&dt.format(FLEX_TIME_FMT).to_string()),
            None => s.serialize_none(),
        }
    }
}

impl<'de> Deserialize<'de> for FlexTime {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        // Accept null or string
        let opt = Option::<String>::deserialize(d)?;
        match opt {
            None => Ok(Self(None)),
            Some(s) if s.is_empty() || s.eq_ignore_ascii_case("null") => Ok(Self(None)),
            Some(s) => {
                let dt = chrono::NaiveDateTime::parse_from_str(&s, FLEX_TIME_FMT)
                    .map_err(de::Error::custom)?;
                Ok(Self(Some(dt)))
            }
        }
    }
}

/// Mirrors Go's `NullString`.
///
/// JSON value is a plain string or `null`; never `{"Val":…,"Valid":…}`.
/// Empty string is treated as null (Valid=false).
#[derive(Debug, Clone, Default)]
pub struct NullString {
    pub val: String,
    pub valid: bool,
}

impl serde::Serialize for NullString {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if self.valid { serializer.serialize_str(&self.val) } else { serializer.serialize_none() }
    }
}

impl NullString {
    pub fn as_str(&self) -> &str {
        &self.val
    }
}

impl std::fmt::Display for NullString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.val)
    }
}

impl<'de> Deserialize<'de> for NullString {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        // The USS API sometimes returns non-string JSON primitives (e.g. `"icon": 0`,
        // `"merchantTimeZone": 8`) for fields that are logically strings.
        // We accept any JSON primitive and coerce to String; null/absent → default.
        struct V;
        impl<'de> de::Visitor<'de> for V {
            type Value = NullString;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "a string, number, bool, or null")
            }
            fn visit_none<E: de::Error>(self) -> std::result::Result<NullString, E> {
                Ok(NullString::default())
            }
            fn visit_unit<E: de::Error>(self) -> std::result::Result<NullString, E> {
                Ok(NullString::default())
            }
            fn visit_some<D: Deserializer<'de>>(
                self,
                d: D,
            ) -> std::result::Result<NullString, D::Error> {
                NullString::deserialize(d)
            }
            fn visit_str<E: de::Error>(self, v: &str) -> std::result::Result<NullString, E> {
                if v.is_empty() || v.eq_ignore_ascii_case("null") {
                    Ok(NullString::default())
                } else {
                    Ok(NullString {
                        val: v.to_string(),
                        valid: true,
                    })
                }
            }
            fn visit_string<E: de::Error>(self, v: String) -> std::result::Result<NullString, E> {
                self.visit_str(&v)
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> std::result::Result<NullString, E> {
                Ok(NullString {
                    val: v.to_string(),
                    valid: true,
                })
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> std::result::Result<NullString, E> {
                Ok(NullString {
                    val: v.to_string(),
                    valid: true,
                })
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> std::result::Result<NullString, E> {
                Ok(NullString {
                    val: v.to_string(),
                    valid: true,
                })
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> std::result::Result<NullString, E> {
                Ok(NullString {
                    val: v.to_string(),
                    valid: true,
                })
            }
        }
        d.deserialize_any(V)
    }
}

/// Mirrors Go's `NullInt32`.
///
/// Go's `NullInt32.UnmarshalJSON(null)` sets `Val = -1, Valid = false`.
/// `#[serde(default)]` (absent key) uses `Default` → `{val: 0, valid: false}` (Go zero-value).
/// Explicit JSON `null` goes through `Deserialize` → `{val: -1, valid: false}`.
#[derive(Debug, Clone, Default)]
pub struct NullInt32 {
    pub val: i32,
    pub valid: bool,
}

impl serde::Serialize for NullInt32 {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if self.valid { serializer.serialize_i32(self.val) } else { serializer.serialize_none() }
    }
}

impl<'de> Deserialize<'de> for NullInt32 {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        Option::<i32>::deserialize(d)?.map_or_else(
            || {
                Ok(Self {
                    val: -1,
                    valid: false,
                })
            },
            |v| {
                Ok(Self {
                    val: v,
                    valid: true,
                })
            },
        )
    }
}

/// Mirrors Go's `NullBool`.
///
/// JSON value may be `true`, `false`, or `null`; null → `{val: false, valid: false}`.
#[derive(Debug, Clone, Default)]
pub struct NullBool {
    pub val: bool,
    pub valid: bool,
}

impl serde::Serialize for NullBool {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if self.valid { serializer.serialize_bool(self.val) } else { serializer.serialize_none() }
    }
}

impl<'de> Deserialize<'de> for NullBool {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        Option::<bool>::deserialize(d)?.map_or_else(
            || {
                Ok(Self {
                    val: false,
                    valid: false,
                })
            },
            |v| {
                Ok(Self {
                    val: v,
                    valid: true,
                })
            },
        )
    }
}

/// Mirrors Go's `NullInt` (int64).
#[derive(Debug, Clone, Default, Serialize)]
pub struct NullInt {
    pub val: i64,
    pub valid: bool,
}

impl<'de> Deserialize<'de> for NullInt {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        Option::<i64>::deserialize(d)?.map_or_else(
            || Ok(Self::default()),
            |v| {
                Ok(Self {
                    val: v,
                    valid: true,
                })
            },
        )
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// model.go — Domain structs
// ═══════════════════════════════════════════════════════════════════════════════

/// Mirrors Go's `Profile` — aligned to actual USS JSON response.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Profile {
    // ── Timestamps ────────────────────────────────────────────────────────────
    #[serde(rename = "createTime", default)]
    pub create_time: FlexTime,
    #[serde(rename = "updateTime", default)]
    pub update_time: FlexTime,
    #[serde(rename = "regDate", default)]
    pub reg_date: FlexTime,
    #[serde(rename = "birthday", default)]
    pub birthday: FlexTime,
    #[serde(rename = "typeUpdateTime", default)]
    pub type_update_time: FlexTime,
    #[serde(rename = "passwdLastModifyDate", default)]
    pub passwd_last_modify_date: FlexTime,
    #[serde(rename = "lastLoginTime", default)]
    pub last_login_time: FlexTime,
    #[serde(rename = "lastLogoutTime", default)]
    pub last_logout_time: FlexTime,
    #[serde(rename = "firstDepositTime", default)]
    pub first_deposit_time: FlexTime,
    #[serde(rename = "updatePayeeNameTime", default)]
    pub update_payee_name_time: FlexTime,
    #[serde(rename = "lastWithdrawTime", default)]
    pub last_withdraw_time: FlexTime,
    #[serde(rename = "lastDepositTime", default)]
    pub last_deposit_time: FlexTime,
    #[serde(rename = "previousLoginTime", default)]
    pub previous_login_time: FlexTime,
    #[serde(rename = "activeFlagUpdateTime", default)]
    pub active_flag_update_time: FlexTime,
    // ── String fields ─────────────────────────────────────────────────────────
    #[serde(rename = "customerName", default)]
    pub customer_name: NullString,
    #[serde(rename = "merchantCode", default)]
    pub merchant_code: NullString,
    #[serde(rename = "nickname", default)]
    pub nickname: NullString,
    #[serde(rename = "nickname2", default)]
    pub nickname2: NullString,
    #[serde(rename = "appleId", default)]
    pub apple_id: NullString,
    #[serde(rename = "mobileNo", default)]
    pub mobile_no: NullString,
    #[serde(rename = "qqNo", default)]
    pub qq_no: NullString,
    #[serde(rename = "lineId", default)]
    pub line_id: NullString,
    #[serde(rename = "lineUuid", default)]
    pub line_uuid: NullString,
    #[serde(rename = "whatsAppId", default)]
    pub whats_app_id: NullString,
    #[serde(rename = "facebookId", default)]
    pub face_book_id: NullString,
    #[serde(rename = "twitter", default)]
    pub twitter: NullString,
    #[serde(rename = "twitterId", default)]
    pub twitter_id: NullString,
    #[serde(rename = "viber", default)]
    pub viber: NullString,
    #[serde(rename = "zalo", default)]
    pub zalo: NullString,
    #[serde(rename = "wechat", default)]
    pub wechat: NullString,
    #[serde(rename = "telegram", default)]
    pub telegram: NullString,
    #[serde(rename = "idNumber", default)]
    pub id_number: NullString,
    #[serde(rename = "idVerificationStatus", default)]
    pub id_verification_status: NullString,
    #[serde(rename = "payeeName", default)]
    pub payee_name: NullString,
    #[serde(rename = "city", default)]
    pub city: NullString,
    #[serde(rename = "zipcode", default)]
    pub zip_code: NullString,
    #[serde(rename = "address", default)]
    pub address: NullString,
    #[serde(rename = "countryCode", default)]
    pub country_code: NullString,
    #[serde(rename = "verificationMode", default)]
    pub verification_mode: NullString,
    #[serde(rename = "login", default)]
    pub login: NullString,
    #[serde(rename = "lastLoginIp", default)]
    pub last_login_ip: NullString,
    #[serde(rename = "previousLoginIp", default)]
    pub previous_login_ip: NullString,
    #[serde(rename = "refer", default)]
    pub refer: NullString,
    #[serde(rename = "icon", default)]
    pub icon: NullString,
    #[serde(rename = "email", default)]
    pub email: NullString,
    // ── Numeric fields ────────────────────────────────────────────────────────
    #[serde(rename = "customerId", default)]
    pub customer_id: NullInt,
    #[serde(rename = "recommenderId", default)]
    pub recommender_id: NullInt,
    #[serde(rename = "levelId", default)]
    pub level_id: NullInt,
    #[serde(rename = "version", deserialize_with = "i32_from_null_or_int", default)]
    pub version: i32,
    #[serde(rename = "type", default)]
    #[allow(clippy::struct_field_names)]
    pub profile_type: NullInt32,
    #[serde(rename = "createSuboFlag", default)]
    pub create_subo_flag: NullInt32,
    #[serde(rename = "activeFlag", default)]
    pub active_flag: NullInt32,
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
    // ── Bool fields ───────────────────────────────────────────────────────────
    /// JSON may be `false` or absent — plain bool is fine here.
    #[serde(rename = "idVerification", default)]
    pub id_verification: bool,
}

/// Merchant info embedded inside `CustomerValue`.
#[allow(clippy::struct_field_names)]
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Merchant {
    #[serde(rename = "merchantDesc", default)]
    pub merchant_desc: NullString,
    #[serde(rename = "currencyCode", default)]
    pub currency_code: NullString,
    #[serde(rename = "merchantTimeZone", default)]
    pub merchant_time_zone: NullString,
    #[serde(rename = "creator", default)]
    pub creator: NullString,
    #[serde(rename = "deleteFlag", default)]
    pub delete_flag: NullString,
    #[serde(rename = "customerId", default)]
    pub customer_id: NullInt,
    #[serde(rename = "parentId", default)]
    pub parent_id: NullInt,
    #[serde(rename = "groupId", default)]
    pub group_id: NullInt,
    #[serde(rename = "deptId", default)]
    pub dept_id: NullInt,
    #[serde(rename = "status", default)]
    pub status: NullInt32,
    #[serde(rename = "type", default)]
    pub merchant_type: NullInt32,
}

/// Mirrors Go's `CustomerAdditionalInfo` — aligned to actual USS JSON response.
///
/// Go has a custom `UnmarshalJSON` that pre-sets `UsState = NullInt32{Val: -1, Valid: false}`
/// before unmarshalling, so even when the entire object is `null` or when `usState` is `null`,
/// `UsState.Val` is `-1` (not `0`). The custom `Default` here mirrors that behaviour.
#[allow(clippy::struct_field_names)]
#[derive(Debug, Clone, Deserialize)]
pub struct CustomerAdditionalInfo {
    // ── Timestamps ────────────────────────────────────────────────────────────
    #[serde(rename = "createTime", default)]
    pub create_time: FlexTime,
    #[serde(rename = "updateTime", default)]
    pub update_time: FlexTime,
    // ── String fields ─────────────────────────────────────────────────────────
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
    #[serde(rename = "googleId", default)]
    pub google_id: NullString,
    #[serde(rename = "telegramId", default)]
    pub telegram_id: NullString,
    #[serde(rename = "glifeId", default)]
    pub glife_id: NullString,
    #[serde(rename = "mayaId", default)]
    pub maya_id: NullString,
    #[serde(rename = "appleUid", default)]
    pub apple_uid: NullString,
    #[serde(rename = "facebookUid", default)]
    pub facebook_uid: NullString,
    #[serde(rename = "officialAppLoginStatus", default)]
    pub official_app_login_status: NullString,
    // ── Numeric fields ────────────────────────────────────────────────────────
    #[serde(rename = "customerId", default)]
    pub customer_id: NullInt,
    #[serde(rename = "usState", default = "default_us_state")]
    pub us_state: NullInt32,
    #[serde(rename = "version", deserialize_with = "i32_from_null_or_int", default)]
    pub version: i32,
    // ── Bool fields ───────────────────────────────────────────────────────────
    /// JSON may be `true`, `false`, or **`null`** — null → false.
    /// Uses a custom deserializer because `#[serde(default)]` only applies when
    /// the key is *absent*, not when the key is present with an explicit `null`.
    #[serde(rename = "emailVerification", deserialize_with = "bool_from_null_or_bool", default)]
    pub email_verification: bool,
}

impl Default for CustomerAdditionalInfo {
    fn default() -> Self {
        Self {
            create_time: FlexTime::default(),
            update_time: FlexTime::default(),
            permanent_address: NullString::default(),
            place_of_birth: NullString::default(),
            nationality: NullString::default(),
            region: NullString::default(),
            kakao: NullString::default(),
            google: NullString::default(),
            google_id: NullString::default(),
            telegram_id: NullString::default(),
            glife_id: NullString::default(),
            maya_id: NullString::default(),
            apple_uid: NullString::default(),
            facebook_uid: NullString::default(),
            official_app_login_status: NullString::default(),
            customer_id: NullInt::default(),
            us_state: NullInt32 {
                val: -1,
                valid: false,
            },
            version: 0,
            email_verification: false,
        }
    }
}

/// Mirrors Go's `Value` (inner payload of `CustomerInfo`) — aligned to actual USS JSON response.
#[allow(clippy::struct_field_names)]
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CustomerValue {
    // ── String / identity fields ──────────────────────────────────────────────
    #[serde(rename = "customerName", default)]
    pub customer_name: NullString,
    #[serde(rename = "customerNameExcludeMerchant", default)]
    pub customer_name_excl_merchant: NullString,
    #[serde(rename = "email", default)]
    pub email: NullString,
    #[serde(rename = "merchantCode", default)]
    pub merchant_code: NullString,
    #[serde(rename = "password", default)]
    pub password: NullString,
    #[serde(rename = "paymentPassword", default)]
    pub payment_password: NullString,
    #[serde(rename = "loginLanguage", default)]
    pub login_language: NullString,
    #[serde(rename = "idVerificationStatus", default)]
    pub id_verification_status: NullString,
    // ── Numeric fields ────────────────────────────────────────────────────────
    #[serde(rename = "customerId", default)]
    pub customer_id: NullInt,
    #[serde(rename = "merchantId", default)]
    pub merchant_id: NullInt,
    #[serde(rename = "systemId", default)]
    pub system_id: NullInt,
    #[serde(rename = "activeFlag", default)]
    pub active_flag: NullInt32,
    #[serde(rename = "hashAlgorithm", default)]
    pub hash_algorithm: NullInt32,
    // ── Time fields ───────────────────────────────────────────────────────────
    #[serde(rename = "errorTime", default)]
    pub error_time: FlexTime,
    // ── Nested objects ────────────────────────────────────────────────────────
    #[serde(rename = "profile", default)]
    pub profile: Profile,
    #[serde(rename = "merchant", default)]
    pub merchant: Merchant,
    // `null_as_default` handles both absent key AND explicit `null` in JSON.
    // (`#[serde(default)]` alone only handles the absent-key case.)
    #[serde(rename = "customerAdditionalInfo", default, deserialize_with = "null_as_default")]
    pub customer_additional_info: CustomerAdditionalInfo,
}

/// Mirrors Go's `CustomerInfo`.
///
/// JSON shape: `{ "success": true, "value": { … } }`
#[derive(Debug, Clone, Deserialize)]
pub struct CustomerInfo {
    #[serde(default)]
    pub success: bool,
    #[serde(default)]
    pub value: CustomerValue,
}

/// Mirrors Go's `PasswordResetTokenRequest`.
#[derive(Debug, Serialize)]
pub struct PasswordResetTokenRequest {
    #[serde(rename = "customerName")]
    pub customer_name: String,
    #[serde(rename = "merchantCode")]
    pub merchant_code: String,
}

/// Mirrors Go's `PasswordResetTokenResponse`.
///
/// JSON shape: `{ "success": true, "value": "TOKEN_STRING" }`
#[derive(Debug, Deserialize)]
pub struct PasswordResetTokenResponse {
    #[serde(default)]
    pub success: bool,
    #[serde(default)]
    pub value: String,
}

/// Mirrors Go's `CustomerPersonalInfo` — envelope for personal info API.
///
/// JSON shape: `{ "success": true, "value": { … } }`
#[derive(Debug, Clone, Deserialize)]
pub struct CustomerPersonalInfo {
    #[serde(default)]
    pub success: bool,
    #[serde(default)]
    pub value: CustomerPersonalInfoValue,
}

/// Mirrors Go's `CustomerPersonalInfoValue` — full personal info from
/// `GET /customer/personal-info?customerId=xxx` (unencrypted field values).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CustomerPersonalInfoValue {
    #[serde(rename = "birthday", default)]
    pub birthday: FlexTime,
    #[serde(rename = "passwordLastModifyDate", default)]
    pub password_last_modify_date: FlexTime,
    #[serde(rename = "placeOfBirth", default)]
    pub place_of_birth: NullString,
    #[serde(rename = "mayaId", default)]
    pub maya_id: NullString,
    #[serde(rename = "merchantCode", default)]
    pub merchant_code: NullString,
    #[serde(rename = "customerName", default)]
    pub customer_name: NullString,
    #[serde(rename = "stateValue", default)]
    pub state_value: NullString,
    #[serde(rename = "appleUid", default)]
    pub apple_uid: NullString,
    #[serde(rename = "nickname", default)]
    pub nickname: NullString,
    #[serde(rename = "payeeName", default)]
    pub payee_name: NullString,
    #[serde(rename = "mobileNo", default)]
    pub mobile_no: NullString,
    #[serde(rename = "countryCode", default)]
    pub country_code: NullString,
    #[serde(rename = "qqNo", default)]
    pub qq_no: NullString,
    #[serde(rename = "wechat", default)]
    pub wechat: NullString,
    #[serde(rename = "lineId", default)]
    pub line_id: NullString,
    #[serde(rename = "facebookId", default)]
    pub facebook_id: NullString,
    #[serde(rename = "whatsAppId", default)]
    pub whats_app_id: NullString,
    #[serde(rename = "idNumber", default)]
    pub id_number: NullString,
    #[serde(rename = "zalo", default)]
    pub zalo: NullString,
    #[serde(rename = "password", default)]
    pub password: NullString,
    #[serde(rename = "telegram", default)]
    pub telegram: NullString,
    #[serde(rename = "google", default)]
    pub google: NullString,
    #[serde(rename = "facebookUid", default)]
    pub facebook_uid: NullString,
    #[serde(rename = "googleId", default)]
    pub google_id: NullString,
    #[serde(rename = "idVerificationStatus", default)]
    pub id_verification_status: NullString,
    #[serde(rename = "glifeId", default)]
    pub glife_id: NullString,
    #[serde(rename = "paymentPassword", default)]
    pub payment_password: NullString,
    #[serde(rename = "city", default)]
    pub city: NullString,
    #[serde(rename = "kakao", default)]
    pub kakao: NullString,
    #[serde(rename = "zipcode", default)]
    pub zipcode: NullString,
    #[serde(rename = "address", default)]
    pub address: NullString,
    #[serde(rename = "twitter", default)]
    pub twitter: NullString,
    #[serde(rename = "viber", default)]
    pub viber: NullString,
    #[serde(rename = "appleId", default)]
    pub apple_id: NullString,
    #[serde(rename = "email", default)]
    pub email: NullString,
    #[serde(rename = "permanentAddress", default)]
    pub permanent_address: NullString,
    #[serde(rename = "region", default)]
    pub region: NullString,
    #[serde(rename = "nationality", default)]
    pub nationality: NullString,
    #[serde(rename = "customerId", default)]
    pub customer_id: NullInt,
    #[serde(rename = "recommenderId", default)]
    pub recommender_id: NullInt,
    #[serde(rename = "gender", default)]
    pub gender: NullInt32,
    #[serde(rename = "idType", default)]
    pub id_type: NullInt32,
    #[serde(rename = "type", default)]
    pub customer_type: NullInt32,
    #[serde(rename = "maritalStatus", default)]
    pub marital_status: NullInt32,
    #[serde(rename = "occupation", default)]
    pub occupation: NullInt32,
    #[serde(rename = "sourceOfIncome", default)]
    pub source_of_income: NullInt32,
    #[serde(rename = "usState", default)]
    pub us_state: NullInt32,
    #[serde(rename = "isEmailVerification", deserialize_with = "bool_from_null_or_bool", default)]
    pub is_email_verification: bool,
    #[serde(rename = "idVerification", deserialize_with = "bool_from_null_or_bool", default)]
    pub id_verification: bool,
    #[serde(rename = "isOfficialAppLogin", default)]
    pub is_official_app_login: NullBool,
    #[serde(rename = "isMobileVerified", default)]
    pub is_mobile_verified: NullBool,
}

// ═══════════════════════════════════════════════════════════════════════════════
// client.go
// ═══════════════════════════════════════════════════════════════════════════════

/// Maximum response body size accepted from USS (mirrors Go's `maxResponseSize`).
const MAX_RESPONSE_SIZE: usize = 1024 * 100; // 100 KB

/// Retry / timeout constants (mirrors Go's `NewClient` defaults).
const MAX_RETRIES: u32 = 3;
const RETRY_DELAY: Duration = Duration::from_millis(700);
const SINGLE_REQ_TIMEOUT: Duration = Duration::from_secs(5);

/// USS HTTP client.
///
/// Mirrors Go's `uss.Client`.
/// Retry logic: up to 3 attempts, 700 ms delay between attempts,
/// 5 s per-attempt timeout; cancels immediately on context cancellation.
#[derive(Debug, Clone)]
pub struct UssClient {
    inner: Client,
    base_url: String,
    /// Trailing slash is guaranteed (e.g. `"tcg-uss-ae/"`).
    base_path: String,
    /// Pre-computed URL for `PUT password/reset-generate`.
    reset_generate_url: String,
}

impl UssClient {
    /// Mirrors Go's `NewClient(host, basePath)`.
    pub fn new(cfg: &ServiceConfig) -> Self {
        let inner = Client::builder()
            .pool_idle_timeout(Duration::from_secs(30))
            .pool_max_idle_per_host(30)
            .connect_timeout(Duration::from_secs(10))
            .build()
            .expect("Failed to build USS HTTP client");

        let base_url = cfg.host.trim_end_matches('/').to_string();
        let base_path = {
            let p = cfg.base_path.trim_end_matches('/').trim_start_matches('/');
            if p.is_empty() { String::new() } else { format!("{p}/") }
        };

        let reset_generate_url = format!("{base_url}/{base_path}password/reset-generate");

        Self {
            inner,
            base_url,
            base_path,
            reset_generate_url,
        }
    }

    // ── Private helpers ───────────────────────────────────────────────────────

    /// Execute `f` up to `MAX_RETRIES` times with per-attempt timeout and
    /// inter-attempt delay. Mirrors Go's `doWithRetry`.
    async fn do_with_retry<F, Fut>(&self, f: F) -> Result<bytes::Bytes>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Result<bytes::Bytes>>,
    {
        let mut last_err = anyhow::anyhow!("no attempts made");

        for attempt in 1..=MAX_RETRIES {
            match tokio::time::timeout(SINGLE_REQ_TIMEOUT, f()).await {
                Ok(Ok(body)) => return Ok(body),
                Ok(Err(e)) => {
                    tracing::warn!(
                        attempt,
                        max = MAX_RETRIES,
                        error = %e,
                        retry_in_ms = RETRY_DELAY.as_millis(),
                        "[USSClient] attempt {attempt}/{MAX_RETRIES} failed, retrying..."
                    );
                    last_err = e;
                }
                Err(_timeout) => {
                    tracing::warn!(
                        attempt,
                        "[USSClient] attempt {attempt}/{MAX_RETRIES} timed out after {SINGLE_REQ_TIMEOUT:?}"
                    );
                    last_err =
                        anyhow::anyhow!("USS request timed out after {SINGLE_REQ_TIMEOUT:?}");
                }
            }

            if attempt < MAX_RETRIES {
                tokio::select! {
                    () = sleep(RETRY_DELAY) => {}
                    _ = tokio::signal::ctrl_c() => {
                        return Err(anyhow::anyhow!("context cancelled, aborting retries"));
                    }
                }
            }
        }

        Err(last_err.context(format!("all {MAX_RETRIES} USS attempts failed")))
    }

    /// Mirrors Go's `doGet`: send GET, check status == 200, read ≤100 KB body.
    #[allow(clippy::future_not_send)]
    async fn do_get(&self, url: &str) -> Result<bytes::Bytes> {
        let resp = self
            .inner
            .get(url)
            .header("Accept", "application/json")
            .send()
            .await
            .with_context(|| format!("USS GET request failed: {url}"))?;

        read_body(resp).await
    }

    /// Mirrors Go's `doPut`: serialise payload, send PUT, check status == 200.
    async fn do_put<T: Serialize>(&self, url: &str, payload: &T) -> Result<bytes::Bytes> {
        let resp = self
            .inner
            .put(url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(payload)
            .send()
            .await
            .with_context(|| format!("USS PUT request failed: {url}"))?;

        read_body(resp).await
    }

    async fn do_put_bytes(&self, url: &str, body_bytes: bytes::Bytes) -> Result<bytes::Bytes> {
        let resp = self
            .inner
            .put(url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .body(body_bytes)
            .send()
            .await
            .with_context(|| format!("USS PUT request failed: {url}"))?;

        read_body(resp).await
    }

    // ── Public API ────────────────────────────────────────────────────────────

    /// Retrieve customer information by customer name.
    ///
    /// Mirrors Go's `Client.GetCustomer`.
    pub async fn get_customer(&self, customer_name: &str, force: bool) -> Result<CustomerInfo> {
        let start = Instant::now();
        let url = format!(
            "{}/{}customer?customerName={}&force={}",
            self.base_url, self.base_path, customer_name, force
        );

        tracing::info!(url = %url, "[USSClient] GetCustomer request");

        let body = self
            .do_with_retry(|| self.do_get(&url))
            .await
            .map_err(|e| {
                tracing::warn!(
                    customer_name,
                    elapsed_ms = start.elapsed().as_millis(),
                    error = %e,
                    "[USSClient] GetCustomer failed"
                );
                e
            })
            .with_context(|| format!("GetCustomer request failed for '{customer_name}'"))?;

        let result = serde_json::from_slice::<CustomerInfo>(&body).with_context(|| {
            format!("deserialization failed, raw response: {}", String::from_utf8_lossy(&body))
        })?;

        tracing::info!(
            customer_name,
            elapsed_ms = start.elapsed().as_millis(),
            "[USSClient] GetCustomer success"
        );
        Ok(result)
    }

    /// Retrieve customer personal information by customer ID.
    ///
    /// Mirrors Go's `Client.GetCustomerPersonalInfo`.
    /// Corresponds to: GET /tcg-uss-ae/customer/personal-info?customerId=xxx
    pub async fn get_customer_personal_info(
        &self,
        customer_id: i64,
    ) -> Result<CustomerPersonalInfoValue> {
        let start = Instant::now();
        let url = format!(
            "{}/{}customer/personal-info?customerId={}",
            self.base_url, self.base_path, customer_id
        );

        tracing::info!(url = %url, "[USSClient] GetCustomerPersonalInfo request");

        let body = self
            .do_with_retry(|| self.do_get(&url))
            .await
            .map_err(|e| {
                tracing::warn!(
                    customer_id,
                    elapsed_ms = start.elapsed().as_millis(),
                    error = %e,
                    "[USSClient] GetCustomerPersonalInfo failed"
                );
                e
            })
            .with_context(|| {
                format!("GetCustomerPersonalInfo request failed for customerId={customer_id}")
            })?;

        let result = serde_json::from_slice::<CustomerPersonalInfo>(&body).with_context(|| {
            format!("deserialization failed, raw response: {}", String::from_utf8_lossy(&body))
        })?;

        tracing::info!(
            customer_id,
            elapsed_ms = start.elapsed().as_millis(),
            "[USSClient] GetCustomerPersonalInfo success"
        );
        Ok(result.value)
    }

    /// Request a one-time password reset token for the given customer.
    ///
    /// Mirrors Go's `Client.GeneratePasswordResetToken`.
    pub async fn generate_password_reset_token(
        &self,
        customer_name: &str,
        merchant_code: &str,
    ) -> Result<PasswordResetTokenResponse> {
        let start = Instant::now();
        let url = &self.reset_generate_url;

        tracing::info!(
            url = %url,
            customer_name,
            merchant_code,
            "[USSClient] GeneratePasswordResetToken request"
        );

        let payload = PasswordResetTokenRequest {
            customer_name: customer_name.to_string(),
            merchant_code: merchant_code.to_string(),
        };
        // Serialize once; each retry gets an O(1) Arc clone
        let body_bytes = bytes::Bytes::from(
            serde_json::to_vec(&payload)
                .with_context(|| "GeneratePasswordResetToken: serialize payload")?,
        );

        let body = self
            .do_with_retry(|| {
                let b = body_bytes.clone();
                self.do_put_bytes(url, b)
            })
            .await
            .map_err(|e| {
                tracing::warn!(
                    customer_name,
                    merchant_code,
                    elapsed_ms = start.elapsed().as_millis(),
                    error = %e,
                    "[USSClient] GeneratePasswordResetToken failed"
                );
                e
            })
            .with_context(|| {
                format!("GeneratePasswordResetToken request failed for '{customer_name}'")
            })?;

        let result =
            serde_json::from_slice::<PasswordResetTokenResponse>(&body).with_context(|| {
                format!("deserialization failed, raw response: {}", String::from_utf8_lossy(&body))
            })?;

        tracing::info!(
            customer_name,
            merchant_code,
            elapsed_ms = start.elapsed().as_millis(),
            "[USSClient] GeneratePasswordResetToken success"
        );
        Ok(result)
    }
}

// ── readBody ──────────────────────────────────────────────────────────────────

/// Accept **only status 200**; stream-read up to `MAX_RESPONSE_SIZE` bytes.
///
/// Mirrors Go's `readBody`:
/// - Non-200 → return `UssHttpError { body, status }`
/// - Body > 100 KB → truncated (same as Go's `io.LimitReader`)
///
/// Uses `resp.chunk()` streaming to cap memory at the limit.
async fn read_body(mut resp: reqwest::Response) -> Result<bytes::Bytes> {
    let status = resp.status();

    if status.as_u16() != 200 {
        let snippet = stream_read_up_to(&mut resp, 512).await;
        let body = String::from_utf8_lossy(&snippet).trim().to_string();
        return Err(UssHttpError {
            body,
            status: status.as_u16(),
        }
        .into());
    }

    if let Some(cl) = resp.content_length()
        && cl > MAX_RESPONSE_SIZE as u64
    {
        return Err(anyhow::anyhow!(
            "USS response too large: content-length={cl}, max={MAX_RESPONSE_SIZE}"
        ));
    }

    let buf = stream_read_up_to(&mut resp, MAX_RESPONSE_SIZE).await;
    Ok(bytes::Bytes::from(buf))
}

/// Stream-read up to `limit` bytes from a response, then stop.
async fn stream_read_up_to(resp: &mut reqwest::Response, limit: usize) -> Vec<u8> {
    let mut buf = Vec::with_capacity(limit.min(8192));
    while let Ok(Some(chunk)) = resp.chunk().await {
        let remaining = limit.saturating_sub(buf.len());
        if remaining == 0 {
            break;
        }
        let take = chunk.len().min(remaining);
        buf.extend_from_slice(&chunk[..take]);
        if buf.len() >= limit {
            break;
        }
    }
    buf
}
