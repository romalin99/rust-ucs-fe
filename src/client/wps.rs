/// WPS (Wallet/Payment Service) HTTP client.
///
/// # Target endpoint
/// ```text
/// GET http://10.80.0.58:9007/wps-core/members/reset-password-status
/// Header: Merchant: dfstar
///
/// Response:
/// {
///   "success": true,
///   "value": {
///     "isEmailResetEnabled": true,
///     "isSmsResetEnabled": true,
///     "isPersonalInfoResetEnabled": false
///   }
/// }
/// ```
use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time::sleep;

use crate::config::ServiceConfig;

// ── Retry constants (mirrors Go's doWithRetry) ────────────────────────────────

/// Number of attempts (1 initial + 2 retries = 3 total), matching Go.
const MAX_ATTEMPTS:    u32      = 3;
/// Delay between attempts (mirrors Go's 700 ms sleep).
const RETRY_DELAY:     Duration = Duration::from_millis(700);
/// Per-request timeout (mirrors Go's 5 s context deadline per attempt).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

// ── Response models ───────────────────────────────────────────────────────────

/// Nested `value` object in the WPS response.
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct ResetPasswordStatusValue {
    /// Whether email-based password reset is enabled for this merchant.
    #[serde(rename = "isEmailResetEnabled", default)]
    pub is_email_reset_enabled: bool,

    /// Whether SMS-based password reset is enabled for this merchant.
    #[serde(rename = "isSmsResetEnabled", default)]
    pub is_sms_reset_enabled: bool,

    /// Whether personal-info-based password reset is enabled for this merchant.
    ///
    /// Present in WPS response but omitted from outgoing JSON (not in Go struct)
    /// unless explicitly set to `true`.
    #[serde(rename = "isPersonalInfoResetEnabled", default, skip_serializing_if = "std::ops::Not::not")]
    pub is_personal_info_reset_enabled: bool,
}

/// Top-level WPS response envelope.
///
/// JSON keys are lowercase (`"success"`, `"value"`) — distinct from USS/MCS
/// which use `"Success"` / `"Value"`.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ResetPasswordStatusResponse {
    /// `true` when the API call succeeded.
    #[serde(default)]
    pub success: bool,

    /// Payload containing the three reset-mode flags.
    #[serde(default)]
    pub value: ResetPasswordStatusValue,
}

// ── Client ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct WpsClient {
    inner: Client,
    /// Fully assembled base URL, e.g. `http://10.80.0.58:9007/wps-core`
    base_url: String,
}

impl WpsClient {
    pub fn new(cfg: &ServiceConfig) -> Self {
        // No global timeout on the client — per-attempt timeout is applied
        // inside do_with_retry via tokio::time::timeout.
        let inner = Client::builder()
            .pool_idle_timeout(Duration::from_secs(30))
            .pool_max_idle_per_host(50)
            .build()
            .expect("Failed to build WPS HTTP client");

        // e.g. host="http://10.80.0.58:9007"  base_path="wps-core/"
        //   → base_url = "http://10.80.0.58:9007/wps-core"
        let base_url = format!(
            "{}/{}",
            cfg.host.trim_end_matches('/'),
            cfg.base_path.trim_end_matches('/')
        );

        Self { inner, base_url }
    }

    /// `GET {base_url}/members/reset-password-status`
    ///
    /// Sends the `Merchant` header and deserialises the response into
    /// [`ResetPasswordStatusResponse`].
    ///
    /// # Equivalent curl
    /// ```bash
    /// curl -X GET \
    ///   'http://10.80.0.58:9007/wps-core/members/reset-password-status' \
    ///   -H 'accept: application/json' \
    ///   -H 'Merchant: dfstar'
    /// ```
    /// `GET {base_url}/members/reset-password-status`
    ///
    /// Retries up to 3 times with 700 ms delay between attempts and a 5 s
    /// per-attempt timeout — identical to Go's `doWithRetry`.
    pub async fn get_reset_password_status(
        &self,
        merchant_code: &str,
    ) -> Result<ResetPasswordStatusResponse> {
        let url = format!("{}/members/reset-password-status", self.base_url);

        tracing::debug!(url = %url, merchant = merchant_code, "WPS → get_reset_password_status");

        self.do_with_retry(merchant_code, &url).await
    }

    // ── Retry helper ──────────────────────────────────────────────────────────

    /// Execute the WPS GET with up to MAX_ATTEMPTS attempts.
    ///
    /// Mirrors Go's `doWithRetry`:
    ///   - 3 total attempts
    ///   - 700 ms sleep between each attempt
    ///   - 5 s per-attempt timeout
    async fn do_with_retry(
        &self,
        merchant_code: &str,
        url: &str,
    ) -> Result<ResetPasswordStatusResponse> {
        let mut last_err = anyhow::anyhow!("no attempts made");

        for attempt in 1..=MAX_ATTEMPTS {
            match tokio::time::timeout(REQUEST_TIMEOUT, self.do_get(merchant_code, url)).await {
                Ok(Ok(resp)) => return Ok(resp),
                Ok(Err(e)) => {
                    tracing::warn!(
                        attempt,
                        error = %e,
                        merchant = merchant_code,
                        "WPS get_reset_password_status failed, will retry"
                    );
                    last_err = e;
                }
                Err(_elapsed) => {
                    tracing::warn!(
                        attempt,
                        merchant = merchant_code,
                        "WPS get_reset_password_status timed out after {:?}",
                        REQUEST_TIMEOUT
                    );
                    last_err = anyhow::anyhow!("WPS request timed out after {:?}", REQUEST_TIMEOUT);
                }
            }

            if attempt < MAX_ATTEMPTS {
                sleep(RETRY_DELAY).await;
            }
        }

        Err(last_err.context(format!(
            "WPS get_reset_password_status failed after {} attempts",
            MAX_ATTEMPTS
        )))
    }

    async fn do_get(
        &self,
        merchant_code: &str,
        url: &str,
    ) -> Result<ResetPasswordStatusResponse> {
        let response = self
            .inner
            .get(url)
            .header("accept", "application/json")
            .header("Merchant", merchant_code)
            .send()
            .await
            .with_context(|| format!("WPS GET request failed: {url}"))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("WPS returned HTTP {}: {}", status, body);
        }

        let parsed = response
            .json::<ResetPasswordStatusResponse>()
            .await
            .with_context(|| {
                format!("Failed to deserialise WPS ResetPasswordStatusResponse from {url}")
            })?;

        tracing::debug!(
            merchant = merchant_code,
            email_enabled    = parsed.value.is_email_reset_enabled,
            sms_enabled      = parsed.value.is_sms_reset_enabled,
            personal_enabled = parsed.value.is_personal_info_reset_enabled,
            "WPS reset-password-status"
        );

        Ok(parsed)
    }
}
