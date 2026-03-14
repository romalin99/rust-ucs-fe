use serde::Serialize;

use crate::model::QuestionInfo;

#[derive(Debug, Serialize)]
pub struct MerchantRuleResponse {
    #[serde(rename = "merchantCode")]
    pub merchant_code: String,
    pub questions: Vec<QuestionInfo>,
}

#[derive(Debug, Serialize)]
pub struct SubmitVerifyData {
    #[serde(rename = "scoreChecked")]
    pub score_checked: bool,
    #[serde(rename = "bindType", skip_serializing_if = "Option::is_none")]
    pub bind_type: Option<String>,
    #[serde(rename = "oneTimePassword", skip_serializing_if = "Option::is_none")]
    pub one_time_password: Option<String>,
}
