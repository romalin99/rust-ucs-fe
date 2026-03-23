/// Oracle repository for TCG_UCS.FIELD_ID_USS_MAPPING.
///
/// Mirrors Go's `internal/repository/field_id_uss_mapping.go`.
/// Only the `find_all_mappings` method is needed for the cache loader.
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::model::FieldIdUssMapping;
use crate::repository::merchant_rule::OraclePool;

const COLUMNS: &str = "ID, MCS_ID, FIELD_ID, FIELD_NAME, USS_ID";

#[derive(Clone)]
pub struct FieldIdUssMappingRepository {
    pool:         Arc<OraclePool>,
    read_timeout: Duration,
}

impl FieldIdUssMappingRepository {
    pub fn new(pool: Arc<OraclePool>) -> Self {
        Self {
            pool,
            read_timeout: Duration::from_secs(15),
        }
    }

    /// Fetch all mapping rows ordered by `FIELD_ID, USS_ID`.
    ///
    /// Used by the startup cache loader and the periodic cron refresh.
    /// Mirrors Go's `FieldIdUssMappingRepo.FindAllMappings`.
    pub async fn find_all_mappings(&self) -> Result<Vec<FieldIdUssMapping>> {
        let pool = self.pool.clone();
        let timeout = self.read_timeout;

        tokio::time::timeout(timeout, tokio::task::spawn_blocking(move || {
            let conn = pool.get().context("Oracle pool: get connection")?;

            let sql = format!(
                "SELECT {} FROM TCG_UCS.FIELD_ID_USS_MAPPING ORDER BY FIELD_ID, USS_ID",
                COLUMNS,
            );

            let rows = conn
                .query(&sql, &[])
                .context("query FIELD_ID_USS_MAPPING")?;

            let mut list = Vec::new();
            for row_result in rows {
                let row = row_result.context("read row")?;
                list.push(FieldIdUssMapping {
                    id:         row.get::<_, i64>("ID")?,
                    mcs_id:     row.get::<_, i64>("MCS_ID")?,
                    field_id:   row.get::<_, String>("FIELD_ID")?,
                    field_name: row.get::<_, String>("FIELD_NAME")?,
                    uss_id:     row.get::<_, i32>("USS_ID")?,
                });
            }
            Ok(list)
        }))
        .await
        .context("find_all_mappings timed out")?
        .context("spawn_blocking panicked")?
    }
}
