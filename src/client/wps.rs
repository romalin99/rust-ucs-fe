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
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::time::sleep;

use crate::config::ServiceConfig;

// ── Retry constants (mirrors Go's doWithRetry) ────────────────────────────────

const MAX_ATTEMPTS:    u32      = 3;
const RETRY_DELAY:     Duration = Duration::from_millis(700);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_RESPONSE_SIZE: usize  = 1024 * 100; // 100 KB

// ── Error type (mirrors Go's wps/error.go) ────────────────────────────────────

/// Non-200 HTTP response returned by the WPS service.
/// Mirrors Go's `wps.HTTPError`.
#[derive(Debug, Clone)]
pub struct WpsHttpError {
    pub body:   String,
    pub status: u16,
}

impl std::fmt::Display for WpsHttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unexpected status code: {}", self.status)
    }
}

impl std::error::Error for WpsHttpError {}

/// Mirrors Go's `wps.IsHTTPError`.
pub fn is_http_error(err: &anyhow::Error) -> bool {
    err.downcast_ref::<WpsHttpError>().is_some()
}

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
    /// Mirrors Go's `Client.GetResetPasswordStatus`.
    /// Retries up to 3 times with 700 ms delay and 5 s per-attempt timeout.
    pub async fn get_reset_password_status(
        &self,
        merchant_code: &str,
    ) -> Result<ResetPasswordStatusResponse> {
        let start = Instant::now();
        let url = format!("{}/members/reset-password-status", self.base_url);

        tracing::info!(url = %url, merchant = merchant_code, "[WPSClient] GetResetPasswordStatus");

        let body = self
            .do_get_with_retry(&url, merchant_code)
            .await
            .map_err(|e| {
                tracing::warn!(
                    merchant = merchant_code,
                    elapsed_ms = start.elapsed().as_millis(),
                    error = %e,
                    "[WPSClient] GetResetPasswordStatus failed"
                );
                e
            })?;

        let result: ResetPasswordStatusResponse = serde_json::from_slice(&body)
            .with_context(|| {
                format!(
                    "deserialization failed, raw response: {}",
                    String::from_utf8_lossy(&body)
                )
            })?;

        tracing::info!(
            merchant = merchant_code,
            is_email_reset_enabled = result.value.is_email_reset_enabled,
            is_sms_reset_enabled   = result.value.is_sms_reset_enabled,
            elapsed_ms = start.elapsed().as_millis(),
            "[WPSClient] GetResetPasswordStatus success"
        );

        Ok(result)
    }

    // ── Private helpers (mirrors Go's doGet / doGetWithRetry / doWithRetry) ──

    async fn do_get_with_retry(&self, url: &str, merchant_code: &str) -> Result<bytes::Bytes> {
        self.do_with_retry(|| self.do_get(url, merchant_code)).await
    }

    async fn do_with_retry<F, Fut>(&self, f: F) -> Result<bytes::Bytes>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Result<bytes::Bytes>>,
    {
        let mut last_err = anyhow::anyhow!("no attempts made");

        for attempt in 1..=MAX_ATTEMPTS {
            match tokio::time::timeout(REQUEST_TIMEOUT, f()).await {
                Ok(Ok(body)) => return Ok(body),
                Ok(Err(e)) => {
                    tracing::warn!(
                        attempt,
                        max = MAX_ATTEMPTS,
                        error = %e,
                        "[WPSClient] attempt {}/{} failed, retrying...",
                        attempt, MAX_ATTEMPTS
                    );
                    last_err = e;
                }
                Err(_timeout) => {
                    tracing::warn!(
                        attempt,
                        "[WPSClient] attempt {}/{} timed out after {:?}",
                        attempt, MAX_ATTEMPTS, REQUEST_TIMEOUT
                    );
                    last_err = anyhow::anyhow!("WPS request timed out after {:?}", REQUEST_TIMEOUT);
                }
            }

            if attempt < MAX_ATTEMPTS {
                tokio::select! {
                    _ = sleep(RETRY_DELAY) => {}
                    _ = tokio::signal::ctrl_c() => {
                        return Err(anyhow::anyhow!("context cancelled, aborting WPS retries"));
                    }
                }
            }
        }

        Err(last_err.context(format!("all {MAX_ATTEMPTS} WPS attempts failed")))
    }

    async fn do_get(&self, url: &str, merchant_code: &str) -> Result<bytes::Bytes> {
        let resp = self
            .inner
            .get(url)
            .header("Accept", "application/json")
            .header("Merchant", merchant_code)
            .send()
            .await
            .with_context(|| format!("WPS GET request failed: {url}"))?;

        read_body(resp).await
    }
}

/// Accept only status 200; read up to `MAX_RESPONSE_SIZE` bytes.
/// Non-200 → `WpsHttpError`. Mirrors Go's `readBody`.
async fn read_body(resp: reqwest::Response) -> Result<bytes::Bytes> {
    let status = resp.status();

    if status.as_u16() != 200 {
        return Err(WpsHttpError {
            body: format!("unexpected status code: {}", status.as_u16()),
            status: status.as_u16(),
        }
        .into());
    }

    if let Some(cl) = resp.content_length() {
        if cl > MAX_RESPONSE_SIZE as u64 {
            return Err(anyhow::anyhow!(
                "WPS response too large: content-length={cl}, max={MAX_RESPONSE_SIZE}"
            ));
        }
    }

    let full = resp
        .bytes()
        .await
        .context("failed to read WPS response body")?;

    if full.len() > MAX_RESPONSE_SIZE {
        Ok(full.slice(..MAX_RESPONSE_SIZE))
    } else {
        Ok(full)
    }
}
