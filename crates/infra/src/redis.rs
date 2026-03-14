//! Redis Sentinel client builder.

use crate::config::RedisConfig;
use std::sync::Arc;

/// Build a `redis::Client` from the sentinel configuration.
pub fn build_client(cfg: &RedisConfig) -> anyhow::Result<Arc<redis::Client>> {
    let url = build_sentinel_url(cfg);
    let client = redis::Client::open(url)?;
    Ok(Arc::new(client))
}

/// Build a `redis-rs` connection URL for Redis Sentinel.
///
/// Format: `redis+sentinel://[:password@]host1:port,host2:port/masterName[/db]`
pub fn build_sentinel_url(cfg: &RedisConfig) -> String {
    if cfg.sentinel_addrs.is_empty() {
        return format!("redis://127.0.0.1:6379/{}", cfg.rate_limit_db);
    }

    let addrs = cfg.sentinel_addrs.join(",");
    if cfg.password.is_empty() {
        format!(
            "redis+sentinel://{}/{}/{}",
            addrs, cfg.master_name, cfg.rate_limit_db
        )
    } else {
        format!(
            "redis+sentinel://:{}@{}/{}/{}",
            cfg.password, addrs, cfg.master_name, cfg.rate_limit_db
        )
    }
}
