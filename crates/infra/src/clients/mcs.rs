//! MCS (Merchant Credit Service) HTTP client.
//!
//! Mirrors Go's `internal/client/mcs` package.
//! One API:
//!   `POST {host}/{basePath}player/verifyPlayerInfo`
//!
//! Retry strategy: up to 3 attempts with 700 ms delay; 5-second per-attempt timeout.

use crate::config::HttpServiceConfig;
use common::error::{AppError, InfraError};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{info, warn};

// ── Request types ─────────────────────────────────────────────────────────────

/// Financial-binding information submitted for verification.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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
    #[serde(rename = "isCaseSensitive")]
    pub is_case_sensitive: bool,
}

/// Transaction history submitted for verification.
/// `*_amount_range` and `*_time_range_in_day` carry the `accuracy` values from
/// the merchant question config, mirroring Go's `applyFieldSetters` logic.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VerifyPlayerHistoryInfo {
    #[serde(rename = "lastDepositAmount")]
    pub last_deposit_amount: String,
    #[serde(rename = "lastDepositAmountRange")]
    pub last_deposit_amount_range: String,
    #[serde(rename = "lastDepositMethod")]
    pub last_deposit_method: String,
    #[serde(rename = "lastDepositTime")]
    pub last_deposit_time: String,
    #[serde(rename = "lastDepositTimeRangeInDay")]
    pub last_deposit_time_range_in_day: i32,
    #[serde(rename = "lastWithdrawAmount")]
    pub last_withdraw_amount: String,
    #[serde(rename = "lastWithdrawAmountRange")]
    pub last_withdraw_amount_range: String,
    #[serde(rename = "lastWithdrawMethod")]
    pub last_withdraw_method: String,
    #[serde(rename = "lastWithdrawTime")]
    pub last_withdraw_time: String,
    #[serde(rename = "lastWithdrawTimeRangeInDay")]
    pub last_withdraw_time_range_in_day: i32,
}

/// Top-level request body for `POST player/verifyPlayerInfo`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VerifyFinanceHistoryReq {
    #[serde(rename = "verifyPlayerFinanceInfo")]
    pub finance_info: VerifyPlayerFinanceInfo,
    #[serde(rename = "verifyPlayerTransactionHistory")]
    pub history_info: VerifyPlayerHistoryInfo,
}

// ── Response types ────────────────────────────────────────────────────────────

/// Per-field match scores for financial binding.
/// rawScore semantics: 3 = matched, 2 = empty match, 0/1 = not matched.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct VerifyPlayerFinanceInfoResult {
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
    #[serde(rename = "vwAddress", default)]
    pub vw_address: i32,
    #[serde(rename = "vwHolderName", default)]
    pub vw_holder_name: i32,
    #[serde(rename = "vwBankCode", default)]
    pub vw_bank_code: i32,
    #[serde(rename = "ewAccount", default)]
    pub ew_account: i32,
    #[serde(rename = "ewHolderName", default)]
    pub ew_holder_name: i32,
    #[serde(rename = "ewBankCode", default)]
    pub ew_bank_code: i32,
}

/// Per-field match scores for transaction history.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
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

/// Nested result object inside `VerifyFinanceHistoryResp`.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct VerifyFinanceHistoryResult {
    #[serde(rename = "verifyPlayerTransactionHistory")]
    pub history_info: VerifyPlayerHistoryResult,
    #[serde(rename = "verifyPlayerFinanceInfo")]
    pub finance_info: VerifyPlayerFinanceInfoResult,
}

/// Top-level MCS response envelope.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct VerifyFinanceHistoryResp {
    pub success: bool,
    pub value: VerifyFinanceHistoryResult,
}

// ── Player header ─────────────────────────────────────────────────────────────

/// Per-request identity headers for MCS player APIs.
pub struct PlayerHeaders {
    pub customer_id: String,
    pub customer_name: String,
    pub merchant: String,
    pub customer_ip: String,
}

// ── Client ───────────────────────────────────────────────────────────────────

const MAX_RETRIES: u32 = 3;
const RETRY_DELAY_MS: u64 = 700;

#[derive(Clone)]
pub struct McsClient {
    http: Client,
    base_url: String,
    base_path: String,
}

impl McsClient {
    pub fn new(cfg: &HttpServiceConfig) -> anyhow::Result<Self> {
        let http = Client::builder()
            .pool_max_idle_per_host(30)
            .timeout(Duration::from_secs(5 * (MAX_RETRIES as u64 + 1)))
            .build()?;
        Ok(Self {
            http,
            base_url: cfg.host.clone(),
            base_path: cfg.base_path.clone(),
        })
    }

    /// `POST {host}/{basePath}player/verifyPlayerInfo`
    pub async fn verify_player_info(
        &self,
        headers: &PlayerHeaders,
        req: &VerifyFinanceHistoryReq,
    ) -> Result<VerifyFinanceHistoryResp, AppError> {
        let url = format!(
            "{}/{}player/verifyPlayerInfo",
            self.base_url, self.base_path
        );
        info!(
            "[MCSClient] VerifyPlayerInfo url={} merchant={}",
            url, headers.merchant
        );

        let body_bytes =
            serde_json::to_vec(req).map_err(|e| AppError::Infra(InfraError::Json(e)))?;

        let customer_id = headers.customer_id.clone();
        let customer_name = headers.customer_name.clone();
        let merchant = headers.merchant.clone();
        let customer_ip = headers.customer_ip.clone();

        let mut last_err: Option<reqwest::Error> = None;

        for attempt in 1..=MAX_RETRIES {
            let result = self
                .http
                .post(&url)
                .header("Content-Type", "application/json")
                .header("Accept", "application/json")
                .header("CustomerId", &customer_id)
                .header("CustomerName", &customer_name)
                .header("Merchant", &merchant)
                .header("CustomerIP", &customer_ip)
                .body(body_bytes.clone())
                .send()
                .await;

            match result {
                Ok(resp) => {
                    let data = resp
                        .json::<VerifyFinanceHistoryResp>()
                        .await
                        .map_err(|e| AppError::Infra(InfraError::Http(e)))?;
                    return Ok(data);
                }
                Err(e) => {
                    warn!(
                        "[MCSClient] attempt {}/{} failed: {}",
                        attempt, MAX_RETRIES, e
                    );
                    last_err = Some(e);
                    if attempt < MAX_RETRIES {
                        tokio::time::sleep(Duration::from_millis(RETRY_DELAY_MS)).await;
                    }
                }
            }
        }

        Err(AppError::Infra(InfraError::Http(last_err.unwrap())))
    }
}
