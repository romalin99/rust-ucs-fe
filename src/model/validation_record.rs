/// Mirrors Go's `internal/model/validation_record.go`.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// TCG_UCS.VALIDATION_RECORD row.
#[derive(Debug, Clone)]
pub struct ValidationRecord {
    pub id: Option<i64>,
    pub customer_id: i64,
    pub customer_name: String,
    pub success: i8,
    pub merchant_code: String,
    pub ip: String,
    pub passing_score: i32,
    pub score: i32,
    /// JSON-encoded map of QA answers.
    pub qas: String,
    pub created_at: DateTime<Utc>,
}

/// Per-question answer + score stored in the QAS CLOB.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QA {
    #[serde(rename = "fieldId")]
    pub field_id: String,
    #[serde(rename = "fieldType")]
    pub field_type: String,
    pub correct: bool,
    pub score: i32,
    #[serde(rename = "totalScore")]
    pub total_score: i32,
}

pub type QaMap = HashMap<String, QA>;
