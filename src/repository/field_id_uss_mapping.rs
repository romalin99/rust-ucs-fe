/// Oracle repository for TCG_UCS.FIELD_ID_USS_MAPPING.
///
/// Mirrors Go's `internal/repository/field_id_uss_mapping.go`.
/// Provides full CRUD operations + cache-loader query.
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};

use crate::model::FieldIdUssMapping;
use crate::pkg::concurrency::lock_timeout;
use crate::repository::merchant_rule::OraclePool;

const COLUMNS: &str = "ID, MCS_ID, FIELD_ID, FIELD_NAME, USS_ID, CREATE_TIME, UPDATE_TIME";
//                      0   1       2         3            4       5            6

// ---------------------------------------------------------------------------
// Static SQL — built once at first use, never re-allocated.
// `find_one_for_update` is intentionally excluded because it embeds a
// runtime `lock_wait` value, making it impossible to hoist fully.
// ---------------------------------------------------------------------------

static SQL_FIND_ALL: once_cell::sync::Lazy<String> = once_cell::sync::Lazy::new(|| {
    format!(
        "SELECT {} FROM TCG_UCS.FIELD_ID_USS_MAPPING ORDER BY FIELD_ID, USS_ID",
        COLUMNS
    )
});

static SQL_FIND_ONE: once_cell::sync::Lazy<String> = once_cell::sync::Lazy::new(|| {
    format!(
        "SELECT {} FROM TCG_UCS.FIELD_ID_USS_MAPPING WHERE ID = :1",
        COLUMNS
    )
});

static SQL_FIND_BY_FIELD_MCS: once_cell::sync::Lazy<String> = once_cell::sync::Lazy::new(|| {
    format!(
        "SELECT {} FROM TCG_UCS.FIELD_ID_USS_MAPPING WHERE FIELD_ID = :1 AND MCS_ID = :2",
        COLUMNS
    )
});

static SQL_FIND_BY_FIELD_USS: once_cell::sync::Lazy<String> = once_cell::sync::Lazy::new(|| {
    format!(
        "SELECT {} FROM TCG_UCS.FIELD_ID_USS_MAPPING WHERE FIELD_ID = :1 AND USS_ID = :2",
        COLUMNS
    )
});

static SQL_FIND_LIST_BY_FIELD: once_cell::sync::Lazy<String> = once_cell::sync::Lazy::new(|| {
    format!(
        "SELECT {} FROM TCG_UCS.FIELD_ID_USS_MAPPING WHERE FIELD_ID = :1 ORDER BY USS_ID",
        COLUMNS
    )
});

// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct FieldIdUssMappingRepository {
    pool: Arc<OraclePool>,
    read_timeout: Duration,
    write_timeout: Duration,
}

impl FieldIdUssMappingRepository {
    pub fn new(pool: Arc<OraclePool>, read_timeout_secs: u64) -> Self {
        Self {
            pool,
            read_timeout: Duration::from_secs(if read_timeout_secs > 0 {
                read_timeout_secs
            } else {
                15
            }),
            write_timeout: Duration::from_secs(15),
        }
    }

    /// Map a query row to [`FieldIdUssMapping`] using positional indices.
    ///
    /// Positional (integer) lookup is O(1) array access; named lookup requires
    /// a string comparison per column.  Column order must match [`COLUMNS`].
    fn map_row(row: &oracle::Row) -> Result<FieldIdUssMapping> {
        Ok(FieldIdUssMapping {
            id: row.get::<_, i64>(0).context("ID")?,
            mcs_id: row.get::<_, i64>(1).context("MCS_ID")?,
            field_id: row.get::<_, String>(2).context("FIELD_ID")?,
            field_name: row.get::<_, String>(3).context("FIELD_NAME")?,
            uss_id: row.get::<_, i32>(4).context("USS_ID")?,
            create_time: row
                .get::<_, Option<chrono::NaiveDateTime>>(5)
                .unwrap_or(None),
            update_time: row
                .get::<_, Option<chrono::NaiveDateTime>>(6)
                .unwrap_or(None),
        })
    }

    /// Fetch all mapping rows ordered by `FIELD_ID, USS_ID`.
    ///
    /// Used by the startup cache loader and the periodic cron refresh.
    /// Mirrors Go's `FieldIdUssMappingRepo.FindAllMappings`.
    pub async fn find_all_mappings(&self) -> Result<Vec<FieldIdUssMapping>> {
        let pool = self.pool.clone();
        let timeout = self.read_timeout;

        tokio::time::timeout(
            timeout,
            tokio::task::spawn_blocking(move || {
                let conn = pool.get().context("Oracle pool: get connection")?;

                let mut stmt = conn
                    .statement(&*SQL_FIND_ALL)
                    .prefetch_rows(super::DEFAULT_PREFETCH_ROWS)
                    .fetch_array_size(super::DEFAULT_FETCH_ARRAY_SIZE)
                    .build()
                    .context("find_all_mappings prepare")?;
                let rows = stmt.query(&[]).context("query FIELD_ID_USS_MAPPING")?;

                let mut list = Vec::new();
                for row_result in rows {
                    let row = row_result.context("read row")?;
                    list.push(Self::map_row(&row)?);
                }
                Ok(list)
            }),
        )
        .await
        .context("find_all_mappings timed out")?
        .context("spawn_blocking panicked")?
    }

    /// Insert a mapping row and return the generated ID.
    /// Mirrors Go's `FieldIdUssMappingRepo.Insert`.
    pub async fn insert(&self, data: &FieldIdUssMapping) -> Result<i64> {
        let pool = self.pool.clone();
        let timeout = self.write_timeout;
        let mcs_id = data.mcs_id;
        let field_id = data.field_id.clone();
        let field_name = data.field_name.clone();
        let uss_id = data.uss_id;

        tokio::time::timeout(timeout, tokio::task::spawn_blocking(move || {
            let conn = pool.get().context("Oracle pool: get connection")?;
            let row = conn.query_row(
                "SELECT SEQ_FIELD_ID_USS_MAPPING.NEXTVAL FROM DUAL",
                &[],
            ).context("sequence nextval")?;
            let id: i64 = row.get(0)?;

            conn.execute(
                "INSERT INTO TCG_UCS.FIELD_ID_USS_MAPPING (ID, MCS_ID, FIELD_ID, FIELD_NAME, USS_ID) \
                 VALUES (:1, :2, :3, :4, :5)",
                &[&id, &mcs_id, &field_id, &field_name, &uss_id],
            ).context("insert field_id_uss_mapping")?;
            conn.commit().context("commit insert")?;
            Ok(id)
        }))
        .await
        .context("insert timed out")?
        .context("spawn_blocking panicked")?
    }

    /// Find one mapping by ID.
    /// Mirrors Go's `FieldIdUssMappingRepo.FindOne`.
    pub async fn find_one(&self, id: i64) -> Result<Option<FieldIdUssMapping>> {
        let pool = self.pool.clone();
        let timeout = self.read_timeout;

        tokio::time::timeout(
            timeout,
            tokio::task::spawn_blocking(move || {
                let conn = pool.get().context("Oracle pool: get connection")?;
                let rows = conn
                    .query(&*SQL_FIND_ONE, &[&id])
                    .context("find_one query")?;
                for row_result in rows {
                    let row = row_result.context("read row")?;
                    return Ok(Some(Self::map_row(&row)?));
                }
                Ok(None)
            }),
        )
        .await
        .context("find_one timed out")?
        .context("spawn_blocking panicked")?
    }

    /// Find and lock a mapping by ID (FOR UPDATE WAIT).
    /// Mirrors Go's `FieldIdUssMappingRepo.FindOneForUpdate`.
    /// Returns the mapping and the connection for caller-managed commit/rollback.
    pub async fn find_one_for_update(
        &self,
        id: i64,
    ) -> Result<
        Option<(
            FieldIdUssMapping,
            r2d2::PooledConnection<super::OracleConnectionManager>,
        )>,
    > {
        let pool = self.pool.clone();
        let timeout = self.read_timeout;
        let lock_wait = lock_timeout();

        tokio::time::timeout(
            timeout,
            tokio::task::spawn_blocking(move || {
                let conn = pool.get().context("Oracle pool: get connection")?;
                // lock_wait is a runtime value, so the SQL cannot be a static.
                let sql = format!(
                    "SELECT {} FROM TCG_UCS.FIELD_ID_USS_MAPPING WHERE ID = :1 FOR UPDATE WAIT {}",
                    COLUMNS, lock_wait
                );
                let rows = conn
                    .query(&sql, &[&id])
                    .context("find_one_for_update query")?;
                for row_result in rows {
                    let row = row_result.context("read row")?;
                    let m = Self::map_row(&row)?;
                    return Ok(Some((m, conn)));
                }
                Ok(None)
            }),
        )
        .await
        .context("find_one_for_update timed out")?
        .context("spawn_blocking panicked")?
    }

    /// Find by FIELD_ID + MCS_ID (unique index).
    /// Mirrors Go's `FieldIdUssMappingRepo.FindByFieldIDAndMcsID`.
    pub async fn find_by_field_id_and_mcs_id(
        &self,
        field_id: &str,
        mcs_id: i64,
    ) -> Result<Option<FieldIdUssMapping>> {
        let pool = self.pool.clone();
        let timeout = self.read_timeout;
        let fid = field_id.to_string();

        tokio::time::timeout(
            timeout,
            tokio::task::spawn_blocking(move || {
                let conn = pool.get().context("Oracle pool: get connection")?;
                let rows = conn
                    .query(&*SQL_FIND_BY_FIELD_MCS, &[&fid, &mcs_id])
                    .context("find_by_field_mcs query")?;
                for row_result in rows {
                    let row = row_result.context("read row")?;
                    return Ok(Some(Self::map_row(&row)?));
                }
                Ok(None)
            }),
        )
        .await
        .context("find_by_field_id_and_mcs_id timed out")?
        .context("spawn_blocking panicked")?
    }

    /// Find by FIELD_ID + USS_ID (unique index).
    /// Mirrors Go's `FieldIdUssMappingRepo.FindByFieldIDAndUssID`.
    pub async fn find_by_field_id_and_uss_id(
        &self,
        field_id: &str,
        uss_id: i32,
    ) -> Result<Option<FieldIdUssMapping>> {
        let pool = self.pool.clone();
        let timeout = self.read_timeout;
        let fid = field_id.to_string();

        tokio::time::timeout(
            timeout,
            tokio::task::spawn_blocking(move || {
                let conn = pool.get().context("Oracle pool: get connection")?;
                let rows = conn
                    .query(&*SQL_FIND_BY_FIELD_USS, &[&fid, &uss_id])
                    .context("find_by_field_uss query")?;
                for row_result in rows {
                    let row = row_result.context("read row")?;
                    return Ok(Some(Self::map_row(&row)?));
                }
                Ok(None)
            }),
        )
        .await
        .context("find_by_field_id_and_uss_id timed out")?
        .context("spawn_blocking panicked")?
    }

    /// Find all mappings for a given FIELD_ID, ordered by USS_ID.
    /// Mirrors Go's `FieldIdUssMappingRepo.FindListByFieldID`.
    pub async fn find_list_by_field_id(&self, field_id: &str) -> Result<Vec<FieldIdUssMapping>> {
        let pool = self.pool.clone();
        let timeout = self.read_timeout;
        let fid = field_id.to_string();

        tokio::time::timeout(
            timeout,
            tokio::task::spawn_blocking(move || {
                let conn = pool.get().context("Oracle pool: get connection")?;
                let rows = conn
                    .query(&*SQL_FIND_LIST_BY_FIELD, &[&fid])
                    .context("find_list_by_field_id query")?;
                let mut list = Vec::new();
                for row_result in rows {
                    let row = row_result.context("read row")?;
                    list.push(Self::map_row(&row)?);
                }
                Ok(list)
            }),
        )
        .await
        .context("find_list_by_field_id timed out")?
        .context("spawn_blocking panicked")?
    }

    /// Update a mapping row by ID.
    /// Mirrors Go's `FieldIdUssMappingRepo.Update`.
    pub async fn update(&self, data: &FieldIdUssMapping) -> Result<u64> {
        let pool = self.pool.clone();
        let timeout = self.write_timeout;
        let id = data.id;
        let mcs_id = data.mcs_id;
        let field_id = data.field_id.clone();
        let field_name = data.field_name.clone();
        let uss_id = data.uss_id;

        tokio::time::timeout(
            timeout,
            tokio::task::spawn_blocking(move || {
                let conn = pool.get().context("Oracle pool: get connection")?;
                let stmt = conn
                    .execute(
                        "UPDATE TCG_UCS.FIELD_ID_USS_MAPPING SET \
                    MCS_ID = :1, FIELD_ID = :2, FIELD_NAME = :3, USS_ID = :4, \
                    UPDATE_TIME = CURRENT_TIMESTAMP \
                 WHERE ID = :5",
                        &[&mcs_id, &field_id, &field_name, &uss_id, &id],
                    )
                    .context("update field_id_uss_mapping")?;
                let affected = stmt.row_count().context("row_count")?;
                conn.commit().context("commit update")?;
                if affected == 0 {
                    return Err(anyhow!(
                        "update field_id_uss_mapping: no rows affected for ID {}",
                        id
                    ));
                }
                Ok(affected)
            }),
        )
        .await
        .context("update timed out")?
        .context("spawn_blocking panicked")?
    }

    /// Delete a mapping row by ID.
    /// Mirrors Go's `FieldIdUssMappingRepo.Delete`.
    pub async fn delete(&self, id: i64) -> Result<u64> {
        let pool = self.pool.clone();
        let timeout = self.write_timeout;

        tokio::time::timeout(
            timeout,
            tokio::task::spawn_blocking(move || {
                let conn = pool.get().context("Oracle pool: get connection")?;
                let stmt = conn
                    .execute(
                        "DELETE FROM TCG_UCS.FIELD_ID_USS_MAPPING WHERE ID = :1",
                        &[&id],
                    )
                    .context("delete field_id_uss_mapping")?;
                let affected = stmt.row_count().context("row_count")?;
                conn.commit().context("commit delete")?;
                if affected == 0 {
                    return Err(anyhow!(
                        "delete field_id_uss_mapping: no rows affected for ID {}",
                        id
                    ));
                }
                Ok(affected)
            }),
        )
        .await
        .context("delete timed out")?
        .context("spawn_blocking panicked")?
    }
}
