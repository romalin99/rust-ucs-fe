/// AWS Secrets Manager integration.
///
/// Mirrors Go's `Config.LoadOracleConnectInfoFromAws`.
/// Secret names per environment:
///   dev  → tcg-uad/db/go-ucs-fe/dev
///   sit  → tcg-uad/db/go-ucs-fe/sit
///   prod → tcg-uad/db/go-ucs-fe
///
/// Required env vars (set before running):
///   AWS_ACCESS_KEY_ID
///   AWS_SECRET_ACCESS_KEY
///   AWS_REGION  (e.g. ap-southeast-1)
use anyhow::{Context, Result, bail};
use aws_config::BehaviorVersion;
use aws_sdk_secretsmanager::Client as SecretsClient;
use serde::Deserialize;

/// Oracle connection info stored in AWS Secrets Manager as a JSON string.
/// JSON keys match exactly what the Go service uses.
#[derive(Debug, Deserialize, Clone)]
pub struct OracleConnectInfo {
    #[serde(rename = "oracledb.user")]
    pub user: String,

    #[serde(rename = "oracledb.password")]
    pub password: String,

    /// Oracle Easy Connect / JDBC-style connection string.
    #[serde(rename = "oracledb.uconnectStringer")]
    pub connect_string: String,

    #[serde(rename = "mongodb.connectStringer", default)]
    pub mongodb_connect_string: Option<String>,
}

/// Fetch Oracle credentials from AWS Secrets Manager.
///
/// The `env` parameter is typically read from the `ENV` environment variable
/// (default: `"dev"`).  Credentials are taken from the standard AWS
/// environment variables (`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`,
/// `AWS_REGION`).
pub async fn load_oracle_connect_info(env: &str) -> Result<OracleConnectInfo> {
    let secret_name = match env.to_lowercase().as_str() {
        "dev" => "tcg-uad/db/go-ucs-fe/dev",
        "sit" => "tcg-uad/db/go-ucs-fe/sit",
        "prod" => "tcg-uad/db/go-ucs-fe",
        other => bail!(
            "Unsupported environment '{}' — expected dev | sit | prod",
            other
        ),
    };

    tracing::info!(
        env,
        secret_name,
        "Loading Oracle credentials from AWS Secrets Manager"
    );

    // Load AWS SDK config from environment variables.
    // Reads: AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY, AWS_REGION automatically.
    let aws_cfg = aws_config::defaults(BehaviorVersion::latest()).load().await;

    let client = SecretsClient::new(&aws_cfg);

    let response = client
        .get_secret_value()
        .secret_id(secret_name)
        .version_stage("AWSCURRENT")
        .send()
        .await
        .with_context(|| format!("AWS GetSecretValue failed for secret '{secret_name}'"))?;

    let secret_string = response
        .secret_string()
        .with_context(|| format!("Secret '{secret_name}' is not stored as a string value"))?;

    let info: OracleConnectInfo = serde_json::from_str(secret_string).with_context(|| {
        format!("Failed to deserialise Oracle connection info from secret '{secret_name}'")
    })?;

    tracing::info!(secret_name, user = %info.user, "Oracle credentials loaded successfully");
    Ok(info)
}
