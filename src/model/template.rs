/// Template field model.
///
/// Mirrors Go's `internal/model/template.go`.
use serde::{Deserialize, Serialize};

// ── Dropdown item ─────────────────────────────────────────────────────────────

/// A single dropdown option stored in the `TEMPLATE_FIELDS` CLOB.
///
/// Mirrors Go's:
/// ```go
/// type DropdownItem struct {
///     DropdownValue string `json:"dropdownValue"`
///     DropdownID    int    `json:"dropdownId"`
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DropdownItem {
    #[serde(rename = "dropdownValue", default)]
    pub dropdown_value: String,

    #[serde(rename = "dropdownId", default)]
    pub dropdown_id: i64,
}

// ── TemplateField ─────────────────────────────────────────────────────────────

/// A template field row in the `TEMPLATE_FIELDS` CLOB.
///
/// Mirrors Go's full `TemplateField` struct — all JSON tags match exactly so
/// that round-tripping through the Oracle CLOB (written by Go) is lossless.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TemplateField {
    // ── Identity ──────────────────────────────────────────────────────────────
    #[serde(rename = "fieldId", default)]
    pub field_id: String,

    #[serde(rename = "fieldName", default)]
    pub field_name: String,

    /// `"DD"` = dropdown, `"I"` = input, `"D"` = date.
    #[serde(rename = "fieldAttribute", default)]
    pub field_attribute: String,

    /// `"Social"`, `"ID"`, `"Financial"`, `"History"`.
    #[serde(rename = "fieldType", default)]
    pub field_type: String,

    // ── Dropdown values ───────────────────────────────────────────────────────
    #[serde(rename = "fieldDropdownList", default)]
    pub dropdown_list: Vec<DropdownItem>,

    // ── Format constraints ────────────────────────────────────────────────────
    #[serde(rename = "formatMax", default)]
    pub format_max: i64,

    #[serde(rename = "formatMin", default)]
    pub format_min: i64,

    #[serde(rename = "format", default)]
    pub format: Option<serde_json::Value>,

    // ── Display / edit flags ──────────────────────────────────────────────────
    #[serde(rename = "isFeDisplay", default)]
    pub is_fe_display: bool,

    #[serde(rename = "isFeDisplayEnabled", default)]
    pub is_fe_display_enabled: bool,

    #[serde(rename = "isPlayerEditable", default)]
    pub is_player_editable: bool,

    #[serde(
        rename = "isPlayerEditableEnabled",
        default,
        skip_serializing_if = "std::ops::Not::not"
    )]
    pub is_player_editable_enabled: bool,

    // ── Required flags ────────────────────────────────────────────────────────
    #[serde(rename = "isRequired", default)]
    pub is_required: bool,

    #[serde(rename = "isRequiredEnabled", default)]
    pub is_required_enabled: bool,

    // ── Unique flags ──────────────────────────────────────────────────────────
    #[serde(rename = "isUnique", default)]
    pub is_unique: bool,

    #[serde(rename = "isUniqueEnabled", default)]
    pub is_unique_enabled: bool,

    // ── KYC ───────────────────────────────────────────────────────────────────
    #[serde(rename = "kycVerification", default)]
    pub kyc_verification: bool,

    // ── Audit fields ──────────────────────────────────────────────────────────
    #[serde(rename = "status", default)]
    pub status: String,

    #[serde(rename = "createdBy", default)]
    pub created_by: String,

    /// Unix ms timestamp (Go stores `int64`).
    #[serde(rename = "createdAt", default)]
    pub created_at: i64,

    #[serde(rename = "updatedBy", default)]
    pub updated_by: Option<serde_json::Value>,

    #[serde(rename = "updatedAt", default)]
    pub updated_at: Option<serde_json::Value>,

    #[serde(rename = "customDisplayName", default)]
    pub custom_display_name: Option<serde_json::Value>,
}

// ── Value (inner object of TemplateFieldsInfo) ────────────────────────────────

/// Inner `value` payload of `TemplateFieldsInfo`.
///
/// Mirrors Go's `Value` struct inside `template.go`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TemplateValue {
    #[serde(rename = "templateId", default)]
    pub template_id: i64,

    #[serde(rename = "templateName", default)]
    pub template_name: String,

    #[serde(rename = "templateFields", default)]
    pub template_fields: Vec<TemplateField>,

    #[serde(rename = "isMobileCountryCodeDisplayEnabled", default)]
    pub is_mobile_country_code_display_enabled: bool,

    #[serde(rename = "isFixedMobileCountryCodeEnabled", default)]
    pub is_fixed_mobile_country_code_enabled: bool,

    /// `null` when not configured.
    #[serde(rename = "mobileCountryCode", default)]
    pub mobile_country_code: Option<serde_json::Value>,

    #[serde(rename = "remark", default)]
    pub remark: Option<serde_json::Value>,
}

// ── TemplateFieldsInfo (API response from MCS/USS template endpoint) ──────────

/// Top-level response envelope for the template-fields API call.
///
/// Mirrors Go's `TemplateFieldsInfo`:
/// ```json
/// {
///   "success":   true,
///   "value":     { "templateId": 1, "templateFields": [...] },
///   "message":   "OK",
///   "errorCode": null
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TemplateFieldsInfo {
    pub success: bool,

    #[serde(default)]
    pub value: TemplateValue,

    #[serde(default)]
    pub message: String,

    #[serde(rename = "errorCode", default)]
    pub error_code: Option<serde_json::Value>,
}

// ── Type aliases ──────────────────────────────────────────────────────────────

/// `merchantCode → (fieldId → Vec<DropdownItem>)`
pub type FieldConfigMap =
    std::collections::HashMap<String, std::collections::HashMap<String, Vec<DropdownItem>>>;
