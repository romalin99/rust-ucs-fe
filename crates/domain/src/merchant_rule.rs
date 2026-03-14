//! Domain models for TCG_UCS.MERCHANT_RULE and related structures.

use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Dropdown item ─────────────────────────────────────────────────────────────

/// One entry in a field's dropdown list — matches the USS template payload shape.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DropdownItem {
    #[serde(rename = "dropdownValue")]
    pub dropdown_value: String,
    #[serde(rename = "dropdownId")]
    pub dropdown_id: i32,
}

// ── Question config ───────────────────────────────────────────────────────────

/// One verification question as stored in the QUESTIONS CLOB column.
///
/// `accuracy` is forwarded to MCS for amount/time-range fields.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Question {
    #[serde(rename = "fieldId")]
    pub field_id: String,
    #[serde(rename = "fieldName")]
    pub field_name: String,
    #[serde(rename = "fieldAttribute")]
    pub field_attribute: String,
    #[serde(rename = "fieldType")]
    pub field_type: String,
    #[serde(rename = "accuracy")]
    pub accuracy: String,
    #[serde(rename = "score")]
    pub score: i32,
    #[serde(rename = "valid")]
    pub valid: bool,
}

/// Public-facing question shape returned to API callers.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QuestionInfo {
    #[serde(rename = "fieldId")]
    pub field_id: String,
    #[serde(rename = "fieldName")]
    pub field_name: String,
    #[serde(rename = "fieldAttribute")]
    pub field_attribute: String,
    #[serde(rename = "fieldType")]
    pub field_type: String,
    #[serde(rename = "fieldDropdownList")]
    pub field_dropdown_list: Vec<DropdownItem>,
}

// ── MerchantRule ──────────────────────────────────────────────────────────────

/// Full row from TCG_UCS.MERCHANT_RULE.
#[derive(Debug, Clone, Default)]
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
    /// Raw JSON CLOB content — call `parse_questions()` to decode.
    pub questions: String,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

impl MerchantRule {
    /// Deserialize the QUESTIONS CLOB into a map keyed by `fieldId`.
    pub fn parse_questions(&self) -> anyhow::Result<HashMap<String, Question>> {
        parse_questions_json(&self.questions)
    }

    /// Return only questions where `valid == true`.
    pub fn parse_valid_questions(&self) -> anyhow::Result<HashMap<String, Question>> {
        let all = self.parse_questions()?;
        Ok(all.into_iter().filter(|(_, q)| q.valid).collect())
    }
}

// ── MerchantRuleConfig ────────────────────────────────────────────────────────

/// Lightweight projection for the verification flow — avoids loading the full row.
#[derive(Debug, Clone, Default)]
pub struct MerchantRuleConfig {
    pub merchant_code: String,
    pub binding_type: String,
    pub empty_score: i32,
    pub passing_score: i32,
    /// Raw JSON CLOB content — call `parse_questions()` to decode.
    pub questions: String,
}

impl MerchantRuleConfig {
    pub fn parse_questions(&self) -> anyhow::Result<HashMap<String, Question>> {
        parse_questions_json(&self.questions)
    }
}

// ── Shared helper ─────────────────────────────────────────────────────────────

fn parse_questions_json(raw: &str) -> anyhow::Result<HashMap<String, Question>> {
    if raw.is_empty() {
        anyhow::bail!("questions field is empty");
    }
    let map: HashMap<String, Question> = serde_json::from_str(raw)?;
    Ok(map)
}
