/// Template field model — mirrors Go's `internal/model/template.go`.
use serde::{Deserialize, Serialize};

/// A single dropdown option stored in the TEMPLATE_FIELDS CLOB.
///
/// Mirrors Go's:
///   type DropdownItem struct {
///       DropdownValue string `json:"dropdownValue"`
///       DropdownID    int    `json:"dropdownId"`
///   }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DropdownItem {
    #[serde(rename = "dropdownValue")]
    pub dropdown_value: String,

    #[serde(rename = "dropdownId", default)]
    pub dropdown_id: i64,
}

/// A template field row in the TEMPLATE_FIELDS CLOB.
///
/// Mirrors Go's `TemplateField`.  The JSON stored in Oracle was written by
/// the Go service, so serde rename attributes must match Go's JSON tags.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateField {
    #[serde(rename = "fieldId", default)]
    pub field_id: String,

    #[serde(rename = "fieldName", default)]
    pub field_name: String,

    /// `"DD"` = dropdown, `"ST"` = string, etc.
    #[serde(rename = "fieldAttribute", default)]
    pub field_attribute: String,

    #[serde(rename = "fieldType", default)]
    pub field_type: String,

    /// Key in the Go JSON is `"fieldDropdownList"`.
    #[serde(rename = "fieldDropdownList", default)]
    pub dropdown_list: Vec<DropdownItem>,

    #[serde(rename = "isFeDisplay", default)]
    pub display: bool,

    #[serde(rename = "isPlayerEditable", default)]
    pub editable: bool,

    #[serde(rename = "isRequired", default)]
    pub required: bool,

    #[serde(rename = "isUnique", default)]
    pub unique: bool,
}

/// Map type: merchantCode → (fieldId → Vec<DropdownItem>).
pub type FieldConfigMap =
    std::collections::HashMap<String, std::collections::HashMap<String, Vec<DropdownItem>>>;
