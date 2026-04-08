/// Sensitive-field log masking.
///
/// Full port of Go's `internal/masking/masking.go`.
///
/// # Purpose
/// Prevent personal or financial data (phone numbers, wallet addresses, social
/// IDs, etc.) from appearing in plain-text log files.  All sensitive-field
/// definitions live here so every call site stays in sync automatically.
use serde::{Deserialize, Serialize};

/// Fields whose logged values must be partially redacted.
///
/// Mirrors Go's `SensitiveFields` map.
const SENSITIVE_FIELDS: &[&str] = &[
    "ID",
    "BANK_ACCOUNT",
    "CARD_HOLDER_NAME",
    "VIRTUAL_WALLET_ADDRESS",
    "VIRTUAL_WALLET_NAME",
    "E_WALLET_ACCOUNT",
    "E_WALLET_NAME",
    "QQ",
    "WECHAT_ID",
    "LINE_ID",
    "FB_ID",
    "WHATSAPP",
    "ZALO",
    "TELEGRAM",
    "VIBER",
    "TWITTER",
    "EMAIL",
    "MOBILE_NUMBER",
    "WITHDRAWER_NAME",
    "APPLE_ID",
    "KAKAO",
    "GOOGLE",
];

fn is_sensitive(field_id: &str) -> bool {
    SENSITIVE_FIELDS.contains(&field_id)
}

/// Returns `value` unchanged when `field_id` is not sensitive.
///
/// When `field_id` is sensitive and the character count exceeds 5, the first
/// 5 characters are kept and every subsequent character is replaced with `'*'`,
/// so the masked length matches the original.
///
/// Mirrors Go's `masking.Value`.
pub fn mask_value(field_id: &str, value: &str) -> String {
    if !is_sensitive(field_id) {
        return value.to_string();
    }
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= 5 {
        return value.to_string();
    }
    let mut out = String::with_capacity(value.len());
    for (i, c) in chars.iter().enumerate() {
        if i < 5 {
            out.push(*c);
        } else {
            out.push('*');
        }
    }
    out
}

// ── Request body masking ──────────────────────────────────────────────────────

/// Internal types used only for JSON parsing (not exposed to caller).
/// Mirrors Go's unexported `verifyItem`, `verifyDataItem`, `submitVerifyRequest`.

#[derive(Debug, Deserialize, Serialize)]
struct VerifyItem {
    #[serde(rename = "fieldId", default)]
    field_id: String,
    #[serde(rename = "fieldValue", default)]
    field_value: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct VerifyDataItem {
    #[serde(rename = "item")]
    item: VerifyItem,
    #[serde(rename = "bind", default)]
    bind: bool,
}

#[derive(Debug, Deserialize, Serialize)]
struct SubmitVerifyRequest {
    #[serde(rename = "customerName", default)]
    customer_name: String,
    #[serde(rename = "data", default)]
    data: Vec<VerifyDataItem>,
}

/// Parse a `SubmitVerifyRequest` JSON body, mask sensitive `fieldValue` entries
/// in-place, and return the re-serialised JSON.
///
/// The original bytes are returned unchanged if parsing fails so the caller
/// always has loggable output.
///
/// Mirrors Go's `masking.RequestBody`.
pub fn mask_request_body(body: &[u8]) -> Vec<u8> {
    if body.is_empty() {
        return body.to_vec();
    }
    let Ok(mut req) = serde_json::from_slice::<SubmitVerifyRequest>(body) else {
        return body.to_vec();
    };
    for item in &mut req.data {
        item.item.field_value = mask_value(&item.item.field_id, &item.item.field_value);
    }
    serde_json::to_vec(&req).unwrap_or_else(|_| body.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_value_non_sensitive_passthrough() {
        assert_eq!(mask_value("NICKNAME", "hello"), "hello");
        assert_eq!(mask_value("UNKNOWN_FIELD", "abc"), "abc");
    }

    #[test]
    fn mask_value_sensitive_short_passthrough() {
        // ≤ 5 chars → unchanged even for sensitive field
        assert_eq!(mask_value("EMAIL", "a@b.c"), "a@b.c");
    }

    #[test]
    fn mask_value_sensitive_long_masked() {
        let result = mask_value("EMAIL", "user@example.com");
        assert_eq!(&result[..5], "user@");
        assert!(result[5..].chars().all(|c| c == '*'));
        assert_eq!(result.len(), "user@example.com".len());
    }

    #[test]
    fn mask_request_body_masks_sensitive_fields() {
        let body = br#"{"customerName":"john","data":[{"item":{"fieldId":"EMAIL","fieldValue":"user@example.com"},"bind":true}]}"#;
        let masked = mask_request_body(body);
        let s = String::from_utf8(masked).unwrap();
        assert!(s.contains("user@"));
        assert!(!s.contains("user@example.com"));
    }

    #[test]
    fn mask_request_body_invalid_json_passthrough() {
        let body = b"not-json";
        assert_eq!(mask_request_body(body), body);
    }
}
