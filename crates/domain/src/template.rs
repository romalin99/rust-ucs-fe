//! Template field models returned by the USS template API and stored in
//! the TEMPLATE_FIELDS column of TCG_UCS.MERCHANT_RULE.
//!
//! These are the source of truth for per-merchant dropdown lists that
//! enrich the `fieldDropdownList` of each `QuestionInfo`.

use crate::merchant_rule::DropdownItem;
use serde::{Deserialize, Serialize};

/// One field descriptor from the USS template endpoint.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct TemplateField {
    #[serde(rename = "fieldId", default)]
    pub field_id: String,
    #[serde(rename = "fieldName", default)]
    pub field_name: String,
    #[serde(rename = "fieldAttribute", default)]
    pub field_attribute: String,
    #[serde(rename = "fieldType", default)]
    pub field_type: String,
    #[serde(rename = "status", default)]
    pub status: String,
    #[serde(rename = "fieldDropdownList", default)]
    pub dropdown_list: Vec<DropdownItem>,
    #[serde(rename = "formatMax", default)]
    pub format_max: i32,
    #[serde(rename = "formatMin", default)]
    pub format_min: i32,
    #[serde(rename = "isFeDisplay", default)]
    pub is_fe_display: bool,
    #[serde(rename = "isPlayerEditable", default)]
    pub is_player_editable: bool,
    #[serde(rename = "isRequired", default)]
    pub is_required: bool,
    #[serde(rename = "isUnique", default)]
    pub is_unique: bool,
    #[serde(rename = "kycVerification", default)]
    pub kyc_verification: bool,
    #[serde(rename = "createdAt", default)]
    pub created_at: i64,
}
