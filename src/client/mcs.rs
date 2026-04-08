/// MCS (Merchant Credit Service) HTTP client — full port of Go's `internal/client/mcs/`.
///
/// Files ported:
///   model.go  → [`PlayerHeaders`], [`VerifyPlayerFinanceInfo`], [`VerifyPlayerHistoryInfo`],
///               [`VerifyFinanceHistoryReq`], [`VerifyFinanceHistoryResp`],
///               [`VerifyFinanceHistoryResult`], [`VerifyPlayerHistoryResult`],
///               [`VerifyPlayerFinanceInfoResult`]
///   client.go → [`McsClient`]  (retry · per-attempt timeout · 100 KB / 512 B snippet body)
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::time::sleep;

use crate::config::ServiceConfig;

// ═══════════════════════════════════════════════════════════════════════════════
// model.go — Request structs
// ═══════════════════════════════════════════════════════════════════════════════

/// Mirrors Go's `VerifyPlayerFinanceInfo` (request side).
///
/// Holds financial binding fields sent to MCS for verification.
#[derive(Debug, Serialize, Default, Clone)]
pub struct VerifyPlayerFinanceInfo {
    #[serde(rename = "bcNumber")]
    pub bc_number: String,
    #[serde(rename = "bcHolderName")]
    pub bc_holder_name: String,
    #[serde(rename = "ewAccount")]
    pub ew_account: String,
    #[serde(rename = "ewHolderName")]
    pub ew_holder_name: String,
    #[serde(rename = "vwAddress")]
    pub vw_address: String,
    #[serde(rename = "vwHolderName")]
    pub vw_holder_name: String,
    /// Mirrors Go's `IsCaseSensitive bool`.
    #[serde(rename = "isCaseSensitive")]
    pub is_case_sensitive: bool,
}

/// Mirrors Go's `VerifyPlayerHistoryInfo` (request side).
///
/// Holds transaction history fields sent to MCS for verification.
#[derive(Debug, Serialize, Default, Clone)]
pub struct VerifyPlayerHistoryInfo {
    #[serde(rename = "lastDepositAmount")]
    pub last_deposit_amount: String,
    #[serde(rename = "lastDepositAmountRange")]
    pub last_deposit_amount_range: String,
    #[serde(rename = "lastDepositMethod")]
    pub last_deposit_method: String,
    #[serde(rename = "lastDepositTime")]
    pub last_deposit_time: String,
    #[serde(rename = "lastWithdrawAmount")]
    pub last_withdraw_amount: String,
    #[serde(rename = "lastWithdrawAmountRange")]
    pub last_withdraw_amount_range: String,
    #[serde(rename = "lastWithdrawMethod")]
    pub last_withdraw_method: String,
    #[serde(rename = "lastWithdrawTime")]
    pub last_withdraw_time: String,
    #[serde(rename = "lastDepositTimeRangeInDay")]
    pub last_deposit_time_range_in_day: i32,
    #[serde(rename = "lastWithdrawTimeRangeInDay")]
    pub last_withdraw_time_range_in_day: i32,
}

/// Mirrors Go's `VerifyFinanceHistoryReq`.
///
/// Top-level request body for `POST player/verifyPlayerInfo`.
///
/// ⚠️  The history field uses JSON key `"verifyPlayerTransactionHistory"`
///    (not `"verifyPlayerHistoryInfo"`) — matches Go's `json:"verifyPlayerTransactionHistory"`.
#[derive(Debug, Serialize, Default, Clone)]
pub struct VerifyFinanceHistoryReq {
    #[serde(rename = "verifyPlayerFinanceInfo")]
    pub verify_player_finance_info: VerifyPlayerFinanceInfo,
    /// JSON key is `"verifyPlayerTransactionHistory"` — mirrors Go's struct tag.
    #[serde(rename = "verifyPlayerTransactionHistory")]
    pub verify_player_history_info: VerifyPlayerHistoryInfo,
}

// ═══════════════════════════════════════════════════════════════════════════════
// model.go — Response structs
// ═══════════════════════════════════════════════════════════════════════════════

/// Mirrors Go's `VerifyPlayerHistoryResult`.
///
/// Per-field match scores returned for transaction history fields.
/// Score semantics (MCS): 3 = matched, 2 = empty-match, 1/0 = not matched.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct VerifyPlayerHistoryResult {
    #[serde(rename = "lastDepositAmount", default)]
    pub last_deposit_amount: i32,
    #[serde(rename = "lastDepositMethod", default)]
    pub last_deposit_method: i32,
    #[serde(rename = "lastDepositTime", default)]
    pub last_deposit_time: i32,
    #[serde(rename = "lastWithdrawAmount", default)]
    pub last_withdraw_amount: i32,
    #[serde(rename = "lastWithdrawMethod", default)]
    pub last_withdraw_method: i32,
    #[serde(rename = "lastWithdrawTime", default)]
    pub last_withdraw_time: i32,
}

/// Mirrors Go's `VerifyPlayerFinanceInfoResult`.
///
/// Per-field match scores returned for financial info fields.
/// Includes extended bank-code fields added in a later Go iteration.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct VerifyPlayerFinanceInfoResult {
    // 银行卡 (bank card)
    #[serde(rename = "bcNumber", default)]
    pub bc_number: i32,
    #[serde(rename = "bcHolderName", default)]
    pub bc_holder_name: i32,
    #[serde(rename = "bcBankCode", default)]
    pub bc_bank_code: i32,
    #[serde(rename = "bcSubBranch", default)]
    pub bc_sub_branch: i32,
    #[serde(rename = "bcCity", default)]
    pub bc_city: i32,
    #[serde(rename = "bcProvince", default)]
    pub bc_province: i32,
    // 虚拟钱包 (virtual wallet)
    #[serde(rename = "vwAddress", default)]
    pub vw_address: i32,
    #[serde(rename = "vwHolderName", default)]
    pub vw_holder_name: i32,
    #[serde(rename = "vwBankCode", default)]
    pub vw_bank_code: i32,
    // 电子钱包 (e-wallet)
    #[serde(rename = "ewAccount", default)]
    pub ew_account: i32,
    #[serde(rename = "ewHolderName", default)]
    pub ew_holder_name: i32,
    #[serde(rename = "ewBankCode", default)]
    pub ew_bank_code: i32,
}

/// Mirrors Go's `VerifyFinanceHistoryResult`.
///
/// Nested `value` payload inside [`VerifyFinanceHistoryResp`].
///
/// ⚠️  The history field uses JSON key `"verifyPlayerTransactionHistory"`.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct VerifyFinanceHistoryResult {
    #[serde(rename = "verifyPlayerFinanceInfo", default)]
    pub verify_player_finance_info: VerifyPlayerFinanceInfoResult,
    /// JSON key is `"verifyPlayerTransactionHistory"` — mirrors Go's struct tag.
    #[serde(rename = "verifyPlayerTransactionHistory", default)]
    pub verify_player_history_info: VerifyPlayerHistoryResult,
}

/// Mirrors Go's `VerifyFinanceHistoryResp`.
///
/// Top-level response envelope: `{ "success": true, "value": { … } }`.
///
/// JSON keys are **lowercase** (`"success"`, `"value"`) — distinct from USS which
/// uses `"Success"` / `"Value"`.
#[derive(Debug, Deserialize, Clone)]
pub struct VerifyFinanceHistoryResp {
    #[serde(default)]
    pub success: bool,
    #[serde(default)]
    pub value: VerifyFinanceHistoryResult,
}

impl std::fmt::Display for VerifyFinanceHistoryResp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match serde_json::to_string(self) {
            Ok(s) => write!(f, "{s}"),
            Err(e) => write!(f, "<VerifyFinanceHistoryResp marshal error: {e}>"),
        }
    }
}

// VerifyFinanceHistoryResp only needs to be serialised for Display; derive Serialize:
impl Serialize for VerifyFinanceHistoryResp {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = s.serialize_struct("VerifyFinanceHistoryResp", 2)?;
        st.serialize_field("success", &self.success)?;
        st.serialize_field("value", &self.value)?;
        st.end()
    }
}
impl Serialize for VerifyFinanceHistoryResult {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = s.serialize_struct("VerifyFinanceHistoryResult", 2)?;
        st.serialize_field("verifyPlayerFinanceInfo", &self.verify_player_finance_info)?;
        st.serialize_field("verifyPlayerTransactionHistory", &self.verify_player_history_info)?;
        st.end()
    }
}
impl Serialize for VerifyPlayerFinanceInfoResult {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = s.serialize_struct("VerifyPlayerFinanceInfoResult", 12)?;
        st.serialize_field("bcNumber", &self.bc_number)?;
        st.serialize_field("bcHolderName", &self.bc_holder_name)?;
        st.serialize_field("bcBankCode", &self.bc_bank_code)?;
        st.serialize_field("bcSubBranch", &self.bc_sub_branch)?;
        st.serialize_field("bcCity", &self.bc_city)?;
        st.serialize_field("bcProvince", &self.bc_province)?;
        st.serialize_field("vwAddress", &self.vw_address)?;
        st.serialize_field("vwHolderName", &self.vw_holder_name)?;
        st.serialize_field("vwBankCode", &self.vw_bank_code)?;
        st.serialize_field("ewAccount", &self.ew_account)?;
        st.serialize_field("ewHolderName", &self.ew_holder_name)?;
        st.serialize_field("ewBankCode", &self.ew_bank_code)?;
        st.end()
    }
}
impl Serialize for VerifyPlayerHistoryResult {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = s.serialize_struct("VerifyPlayerHistoryResult", 6)?;
        st.serialize_field("lastDepositAmount", &self.last_deposit_amount)?;
        st.serialize_field("lastDepositMethod", &self.last_deposit_method)?;
        st.serialize_field("lastDepositTime", &self.last_deposit_time)?;
        st.serialize_field("lastWithdrawAmount", &self.last_withdraw_amount)?;
        st.serialize_field("lastWithdrawMethod", &self.last_withdraw_method)?;
        st.serialize_field("lastWithdrawTime", &self.last_withdraw_time)?;
        st.end()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// model.go — PlayerHeaders
// ═══════════════════════════════════════════════════════════════════════════════

/// Per-request identity headers for all MCS player APIs.
///
/// Mirrors Go's `PlayerHeaders` struct and its `apply(*http.Request)` method.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PlayerHeaders {
    #[serde(rename = "customerId", skip_serializing_if = "String::is_empty")]
    pub customer_id: String,
    #[serde(rename = "customerName", skip_serializing_if = "String::is_empty")]
    pub customer_name: String,
    #[serde(rename = "merchant", skip_serializing_if = "String::is_empty")]
    pub merchant: String,
    #[serde(rename = "customerIp", skip_serializing_if = "String::is_empty")]
    pub customer_ip: String,
}

impl std::fmt::Display for PlayerHeaders {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match serde_json::to_string(self) {
            Ok(s) => write!(f, "{s}"),
            Err(e) => write!(f, "<PlayerHeaders marshal error: {e}>"),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// client.go — constants
// ═══════════════════════════════════════════════════════════════════════════════

/// Mirrors Go's `maxResponseSize = 1024 * 100`.
const MAX_RESPONSE_SIZE: usize = 1024 * 100; // 100 KB

/// Snippet size logged on non-200 responses (mirrors Go's `io.LimitReader(body, 512)`).
const ERR_SNIPPET_SIZE: usize = 512;

const MAX_RETRIES: u32 = 3;
const RETRY_DELAY: Duration = Duration::from_millis(700);
const SINGLE_REQ_TIMEOUT: Duration = Duration::from_secs(5);

// ═══════════════════════════════════════════════════════════════════════════════
// client.go — McsClient
// ═══════════════════════════════════════════════════════════════════════════════

/// MCS HTTP client.
///
/// Mirrors Go's `mcs.Client`.
/// Retry logic: up to 3 attempts, 700 ms delay between attempts,
/// 5 s per-attempt timeout.
#[derive(Debug, Clone)]
pub struct McsClient {
    inner: Client,
    base_url: String,
    /// Trailing slash guaranteed (e.g. `"tcg-mcs-ae/"`).
    base_path: String,
    /// Pre-computed URL for `POST player/verifyPlayerInfo`.
    verify_player_info_url: String,
}

impl McsClient {
    /// Mirrors Go's `NewClient(host, basePath)`.
    pub fn new(cfg: &ServiceConfig) -> Self {
        let inner = Client::builder()
            .pool_idle_timeout(Duration::from_secs(30))
            .pool_max_idle_per_host(30)
            .connect_timeout(Duration::from_secs(10))
            .build()
            .expect("Failed to build MCS HTTP client");

        let base_url = cfg.host.trim_end_matches('/').to_string();
        let base_path = {
            let p = cfg.base_path.trim_end_matches('/').trim_start_matches('/');
            if p.is_empty() { String::new() } else { format!("{p}/") }
        };

        let verify_player_info_url = format!("{}/{}player/verifyPlayerInfo", base_url, base_path);

        Self {
            inner,
            base_url,
            base_path,
            verify_player_info_url,
        }
    }

    // ── Private helpers ───────────────────────────────────────────────────────

    /// Execute `f` up to `MAX_RETRIES` times with per-attempt timeout.
    /// Mirrors Go's `doWithRetry`.
    async fn do_with_retry<F, Fut>(&self, url: &str, f: F) -> Result<bytes::Bytes>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Result<bytes::Bytes>>,
    {
        let mut last_err = anyhow::anyhow!("no attempts made");

        for attempt in 1..=MAX_RETRIES {
            match tokio::time::timeout(SINGLE_REQ_TIMEOUT, f()).await {
                Ok(Ok(body)) => return Ok(body),
                Ok(Err(e)) => {
                    tracing::warn!(
                        attempt,
                        max  = MAX_RETRIES,
                        url,
                        error = %e,
                        "[MCSClient] attempt {}/{} failed, retrying...",
                        attempt, MAX_RETRIES
                    );
                    last_err = e;
                }
                Err(_timeout) => {
                    tracing::warn!(
                        attempt,
                        url,
                        "[MCSClient] attempt {}/{} timed out after {:?}",
                        attempt,
                        MAX_RETRIES,
                        SINGLE_REQ_TIMEOUT
                    );
                    last_err =
                        anyhow::anyhow!("MCS request timed out after {:?}", SINGLE_REQ_TIMEOUT);
                }
            }

            if attempt < MAX_RETRIES {
                tokio::select! {
                    _ = sleep(RETRY_DELAY) => {}
                    _ = tokio::signal::ctrl_c() => {
                        return Err(anyhow::anyhow!("context cancelled, aborting MCS retries"));
                    }
                }
            }
        }

        Err(last_err.context(format!("all {MAX_RETRIES} MCS attempts failed (url={url})")))
    }

    /// POST with player identity headers; body bytes are re-sent on every retry.
    ///
    /// Mirrors Go's `playerPost`.
    async fn player_post(
        &self,
        url: &str,
        headers: &PlayerHeaders,
        body_bytes: bytes::Bytes,
    ) -> Result<bytes::Bytes> {
        self.do_with_retry(url, || {
            let body = body_bytes.clone(); // O(1) Arc increment per retry
            async move {
                let resp = self
                    .inner
                    .post(url)
                    .header("Content-Type", "application/json")
                    .header("Accept", "application/json")
                    .header("CustomerId", &headers.customer_id)
                    .header("CustomerName", &headers.customer_name)
                    .header("Merchant", &headers.merchant)
                    .header("CustomerIP", &headers.customer_ip)
                    .body(body)
                    .send()
                    .await
                    .with_context(|| format!("MCS POST failed: {url}"))?;

                read_body(resp).await
            }
        })
        .await
    }

    // ── Public API ────────────────────────────────────────────────────────────

    /// `POST {host}/{basePath}player/verifyPlayerInfo`
    ///
    /// Mirrors Go's `Client.VerifyPlayerInfo`.
    pub async fn verify_player_info(
        &self,
        headers: &PlayerHeaders,
        req: &VerifyFinanceHistoryReq,
    ) -> Result<VerifyFinanceHistoryResp> {
        let start = Instant::now();
        const OP: &str = "verifyPlayerInfo";

        let url = &self.verify_player_info_url;

        let req_body = serde_json::to_vec(req)
            .with_context(|| format!("{OP}: marshal request body failed"))?;
        let body_bytes = bytes::Bytes::from(req_body);

        tracing::info!(
            url = %url,
            headers = %headers,
            req_body = %String::from_utf8_lossy(&body_bytes),
            "[MCSClient] VerifyPlayerInfo request"
        );

        let resp_body = self
            .player_post(&url, headers, body_bytes)
            .await
            .map_err(|e| {
                tracing::warn!(
                    merchant = %headers.merchant,
                    elapsed_ms = start.elapsed().as_millis(),
                    error = %e,
                    "[MCSClient] VerifyPlayerInfo failed"
                );
                e
            })
            .with_context(|| format!("{OP}: request failed"))?;

        tracing::info!(
            merchant = %headers.merchant,
            elapsed_ms = start.elapsed().as_millis(),
            raw_resp = %String::from_utf8_lossy(&resp_body),
            "[MCSClient] VerifyPlayerInfo success"
        );

        serde_json::from_slice::<VerifyFinanceHistoryResp>(&resp_body).with_context(|| {
            format!("{OP}: unmarshal failed, raw={}", String::from_utf8_lossy(&resp_body))
        })
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// readBody — mirrors Go's mcs.readBody
// ═══════════════════════════════════════════════════════════════════════════════

/// Accept **only status 200**; stream-read ≤ `MAX_RESPONSE_SIZE` bytes.
///
/// Non-200: stream-reads up to 512 bytes as an error snippet (mirrors Go's
/// `io.LimitReader(resp.Body, 512)`).
///
/// Uses `resp.chunk()` streaming to cap memory usage — matching Go's
/// `io.LimitReader` which never allocates more than the limit.
async fn read_body(mut resp: reqwest::Response) -> Result<bytes::Bytes> {
    let status = resp.status();

    if status.as_u16() != 200 {
        let snippet = stream_read_up_to(&mut resp, ERR_SNIPPET_SIZE).await;
        let snippet_str = String::from_utf8_lossy(&snippet).trim().to_string();
        anyhow::bail!("status {}: {}", status.as_u16(), snippet_str);
    }

    if let Some(cl) = resp.content_length() {
        if cl > MAX_RESPONSE_SIZE as u64 {
            return Err(anyhow::anyhow!(
                "MCS response too large: content-length={cl}, max={MAX_RESPONSE_SIZE}"
            ));
        }
    }

    let buf = stream_read_up_to(&mut resp, MAX_RESPONSE_SIZE).await;
    Ok(bytes::Bytes::from(buf))
}

/// Stream-read up to `limit` bytes from a response, then stop.
/// Mirrors Go's `io.ReadAll(io.LimitReader(resp.Body, limit))`.
async fn stream_read_up_to(resp: &mut reqwest::Response, limit: usize) -> Vec<u8> {
    let mut buf = Vec::with_capacity(limit.min(8192));
    while let Ok(Some(chunk)) = resp.chunk().await {
        let remaining = limit.saturating_sub(buf.len());
        if remaining == 0 {
            break;
        }
        let take = chunk.len().min(remaining);
        buf.extend_from_slice(&chunk[..take]);
        if buf.len() >= limit {
            break;
        }
    }
    buf
}
