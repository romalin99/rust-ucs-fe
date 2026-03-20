use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct VerifyItem {
    #[serde(rename = "fieldId")]
    pub field_id: String,
    #[serde(rename = "fieldValue", default)]
    pub field_value: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct VerifyDataItem {
    pub item: VerifyItem,
    #[serde(default)]
    pub bind: bool,
}

#[derive(Debug, Deserialize)]
pub struct SubmitVerifyRequest {
    #[serde(rename = "customerName")]
    pub customer_name: String,
    pub data: Vec<VerifyDataItem>,
}

/// Query parameters for GET /verification/questions.
///
/// `customerName` defaults to `""` so that the handler can return its own
/// typed error instead of Axum emitting a generic 400 rejection.
/// Mirrors Go's `ctx.Query("customerName")` which returns `""` when absent.
#[derive(Debug, Deserialize)]
pub struct GetQuestionListParams {
    #[serde(rename = "customerName", default)]
    pub customer_name: String,
}
