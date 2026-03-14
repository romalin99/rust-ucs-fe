use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

/// One answer/score pair stored inside the QAS column.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QA {
    #[serde(rename = "fieldId")]
    pub field_id: String,
    #[serde(rename = "fieldType")]
    pub field_type: String,
    #[serde(rename = "score")]
    pub score: i32,
    #[serde(rename = "totalScore")]
    pub total_score: i32,
    #[serde(rename = "correct")]
    pub correct: bool,
}

/// Row from TCG_UCS.VALIDATION_RECORD.
#[derive(Debug, Clone, Default)]
pub struct ValidationRecord {
    pub id: i64,
    pub customer_id: i64,
    pub customer_name: String,
    pub success: i8,
    pub merchant_code: String,
    pub ip: String,
    pub passing_score: i32,
    pub score: i32,
    /// JSON-serialised `HashMap<String, QA>`.
    pub qas: String,
    pub created_at: NaiveDateTime,
}
