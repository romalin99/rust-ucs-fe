/// Mirrors Go's `internal/model/merchant_rule.go`.
use std::collections::HashMap;

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
    /// Raw JSON CLOB for per-language field translations.
    pub field_translations: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

/// Slim config version used in the verification flow.
/// Mirrors Go's `model.MerchantRuleConfig` — stores the raw QUESTIONS CLOB;
/// parsing happens on demand via `parse_questions()` so errors are propagated.
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
    /// Raw JSON CLOB from DB — parsed on demand, matching Go.
    pub questions_json: Option<String>,
    /// Raw JSON CLOB for per-language field translations.
    pub field_translations: Option<String>,
}

impl MerchantRuleConfig {
    /// Parse QUESTIONS CLOB into a map keyed by fieldId.
    /// Mirrors Go's `(c *MerchantRuleConfig) ParseQuestions()`.
    pub fn parse_questions(&self) -> anyhow::Result<std::collections::HashMap<String, Question>> {
        let raw = self.questions_json.as_deref().unwrap_or("");
        if raw.is_empty() {
            anyhow::bail!("questions field is empty for merchant: {}", self.merchant_code);
        }
        let map: std::collections::HashMap<String, Question> =
            serde_json::from_str(raw).map_err(|e| {
                anyhow::anyhow!(
                    "unmarshal questions failed for merchant {}: {}",
                    self.merchant_code,
                    e
                )
            })?;
        Ok(map)
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
    #[serde(rename = "fieldDropdownList", skip_serializing_if = "is_dropdown_empty")]
    pub field_dropdown_list: Option<Vec<crate::model::template::DropdownItem>>,
}

/// Mirrors Go's `omitempty` for slices: skip when None OR empty.
fn is_dropdown_empty(v: &Option<Vec<crate::model::template::DropdownItem>>) -> bool {
    v.as_ref().is_none_or(|list| list.is_empty())
}

/// Single translation entry for a field ID.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldIdTranslation {
    #[serde(rename = "fieldId")]
    pub field_id: String,
    #[serde(rename = "fieldTranslation")]
    pub field_translation: String,
}

/// Map from language code → list of field translations.
/// JSON structure: { "EN": [{fieldId, fieldTranslation}, ...], "ZH": [...] }
pub type FieldTranslationsMap = HashMap<String, Vec<FieldIdTranslation>>;

impl MerchantRule {
    /// Parse QUESTIONS CLOB into a map keyed by fieldId.
    /// Mirrors Go's `(m *MerchantRule) ParseQuestions()`.
    pub fn parse_questions(&self) -> anyhow::Result<HashMap<String, Question>> {
        let raw = self.questions_json.as_deref().unwrap_or("");
        if raw.is_empty() {
            anyhow::bail!("questions field is empty");
        }
        let map: HashMap<String, Question> = serde_json::from_str(raw)
            .map_err(|e| anyhow::anyhow!("unmarshal questions failed: {e}"))?;
        Ok(map)
    }

    /// Returns only questions with `valid == true`.
    /// Mirrors Go's `(m *MerchantRule) ParseValidQuestions()`.
    pub fn parse_valid_questions(&self) -> anyhow::Result<HashMap<String, Question>> {
        let all = self.parse_questions()?;
        Ok(all.into_iter().filter(|(_, q)| q.valid).collect())
    }

    /// Returns sorted `QuestionInfo` list for all valid, non-empty-fieldId questions.
    /// Mirrors Go's `(m *MerchantRule) GetValidQuestionInfos()`.
    pub fn get_valid_question_infos(&self) -> anyhow::Result<Vec<QuestionInfo>> {
        let all = self.parse_questions()?;
        let mut result: Vec<QuestionInfo> = all
            .into_values()
            .filter(|q| q.valid && !q.field_id.is_empty())
            .map(|q| QuestionInfo {
                field_id: q.field_id,
                field_name: q.field_name,
                field_attribute: q.field_attribute,
                field_type: q.field_type,
                field_dropdown_list: None,
            })
            .collect();
        result.sort_by(|a, b| a.field_id.cmp(&b.field_id));
        Ok(result)
    }

    /// Serializes a question map back into the QUESTIONS CLOB field.
    /// Mirrors Go's `(m *MerchantRule) MarshalQuestions(questions)`.
    pub fn marshal_questions(
        &mut self,
        questions: &HashMap<String, Question>,
    ) -> anyhow::Result<()> {
        let json = serde_json::to_string(questions)
            .map_err(|e| anyhow::anyhow!("marshal questions failed: {e}"))?;
        self.questions_json = Some(json);
        Ok(())
    }

    pub fn parse_field_translations_map(&self) -> FieldTranslationsMap {
        let raw = self.field_translations.as_deref().unwrap_or("");
        if raw.is_empty() || raw == "{}" {
            return HashMap::new();
        }
        serde_json::from_str(raw).unwrap_or_default()
    }

    /// Returns fieldId → fieldTranslation map for the requested language.
    /// Falls back to "EN" if language not found. Returns empty map on error.
    pub fn get_translations_by_language(&self, language: &str) -> HashMap<String, String> {
        let start = std::time::Instant::now();
        let raw = self.field_translations.as_deref().unwrap_or("");
        if raw.is_empty() || raw == "{}" {
            tracing::info!(
                language,
                fields = 0,
                elapsed_ms = 0,
                "GetTranslationsByLanguage (empty CLOB)"
            );
            return HashMap::new();
        }

        // Single parse directly into the target type — no intermediate Value allocation.
        let all: HashMap<String, Vec<FieldIdTranslation>> = match serde_json::from_str(raw) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    elapsed_ms = start.elapsed().as_millis() as u64,
                    "GetTranslationsByLanguage unmarshal failed"
                );
                return HashMap::new();
            }
        };

        // Pick the requested language, fall back to "EN".
        let (list, fallback) = match all.get(language) {
            Some(v) => (v, ""),
            None => match all.get("EN") {
                Some(v) => (v, " (fallback to EN)"),
                None => {
                    tracing::info!(
                        language,
                        fallback = " (no EN fallback)",
                        fields = 0,
                        elapsed_ms = start.elapsed().as_millis() as u64,
                        "GetTranslationsByLanguage"
                    );
                    return HashMap::new();
                }
            },
        };

        let result: HashMap<String, String> =
            list.iter().map(|t| (t.field_id.clone(), t.field_translation.clone())).collect();

        tracing::info!(
            language,
            fallback,
            fields = result.len(),
            elapsed_ms = start.elapsed().as_millis() as u64,
            "GetTranslationsByLanguage"
        );
        result
    }
}
