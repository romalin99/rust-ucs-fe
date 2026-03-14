/// Mirrors Go's `internal/model/merchant_rule.go`.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Full DB row for TCG_UCS.MERCHANT_RULE.
#[derive(Debug, Clone)]
pub struct MerchantRule {
    pub id: i64,
    pub is_default: i8,
    pub merchant_code: String,
    pub operator: String,
    pub ip_retry_limit: i32,
    pub account_retry_limit: i32,
    pub empty_score: i32,
    pub lock_hour: i32,
    pub binding_type: String,
    pub passing_score: i32,
    /// Raw JSON CLOB from DB.
    pub questions_json: Option<String>,
    /// Raw JSON CLOB for template fields.
    pub template_fields_json: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

/// Slim config version used in the verification flow.
#[derive(Debug, Clone)]
pub struct MerchantRuleConfig {
    pub id: i64,
    pub merchant_code: String,
    pub binding_type: String,
    pub passing_score: i32,
    pub empty_score: i32,
    pub lock_hour: i32,
    pub ip_retry_limit: i32,
    pub account_retry_limit: i32,
    /// Parsed questions map (fieldId → Question).
    pub questions: Vec<Question>,
}

impl MerchantRuleConfig {
    /// Parse the `questions_json` field into a map keyed by fieldId.
    pub fn parse_questions_map(
        json: &str,
    ) -> anyhow::Result<std::collections::HashMap<String, Question>> {
        let questions: Vec<Question> = serde_json::from_str(json)?;
        Ok(questions
            .into_iter()
            .filter(|q| q.valid && !q.field_id.is_empty())
            .map(|q| (q.field_id.clone(), q))
            .collect())
    }
}

/// A single verification question stored in the QUESTIONS CLOB.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Question {
    #[serde(rename = "fieldId", default)]
    pub field_id: String,

    #[serde(rename = "fieldName", default)]
    pub field_name: String,

    #[serde(rename = "fieldAttribute", default)]
    pub field_attribute: String,

    #[serde(rename = "fieldType", default)]
    pub field_type: String,

    #[serde(default)]
    pub valid: bool,

    #[serde(default)]
    pub score: i32,

    #[serde(default)]
    pub accuracy: String,
}

/// Public shape returned to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionInfo {
    #[serde(rename = "fieldId")]
    pub field_id: String,
    #[serde(rename = "fieldName")]
    pub field_name: String,
    #[serde(rename = "fieldAttribute")]
    pub field_attribute: String,
    #[serde(rename = "fieldType")]
    pub field_type: String,
    #[serde(rename = "fieldDropdownList", skip_serializing_if = "Option::is_none")]
    pub field_dropdown_list: Option<Vec<crate::model::template::DropdownItem>>,
}
