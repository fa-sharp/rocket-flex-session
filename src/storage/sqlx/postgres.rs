use bon::bon;
use rocket::{async_trait, http::CookieJar, time::OffsetDateTime};
use sqlx::{postgres::PgRow, PgPool, Postgres, Row};

use crate::{
    error::{SessionError, SessionResult},
    storage::{SessionStorage, SessionStorageIndexed},
};

use super::*;

/**
Session store using PostgreSQL via [sqlx](https://docs.rs/crate/sqlx).

# Requirements
- You must pass in an initialized sqlx Postgres connection pool.
- Your session data type must implement [`SessionSqlx`] to configure how to convert & store session data.
- Your session data type must implement [`SessionIdentifier`]. The SessionIdentifier's
[Id](`SessionIdentifier::Id`) type must be a type supported by sqlx.
- Expects a table to already exist with the following columns:

| Name | Type |
|------|---------|
| id   | `text` PRIMARY KEY |
| data | `text` NOT NULL (or `jsonb`)  |
| user_id | SQL type of `SessionIdentifier::Id` |
| expires | `timestamptz` NOT NULL |

The name of the session index column ("user_id") can be customized when building the storage.

# Session storage
Sessions are stored in the table specified by `table_name`, along with the optional identifier
(typically a user ID) and the session's expiration time. You can enable automatic deletion of
expired sessions by setting the `cleanup_interval` option. This storage provider does not
create any table or index for you, so you'll need to do that in your existing migration flow.

# Example
Initialize the sqlx pool, then use the builder pattern to create a new instance of `SqlxPostgresStorage`:
```
use rocket_flex_session::storage::sqlx::SqlxPostgresStorage;
use std::time::Duration;

async fn create_storage() -> SqlxPostgresStorage {
    let url = "postgres://...";
    let pool = sqlx::PgPool::connect(url).await.unwrap();
    SqlxPostgresStorage::builder()
        .pool(pool.clone())
        .table_name("sessions")
        // name of the column used to group sessions
        .index_column("user_id")
        // optional auto-deletion of expired sessions
        .cleanup_interval(Duration::from_secs(600))
        .build()
}
```
*/
pub struct SqlxPostgresStorage {
    pool: PgPool,
    base: SqlxBase<Postgres>,
    table_name: String,
    index_column: String,
    cleanup_task: SqlxCleanupTask,
}

#[bon]
impl SqlxPostgresStorage {
    #[builder]
    pub fn new(
        /// An initialized Postgres connection pool.
        pool: PgPool,
        /// The name of the table to use for storing sessions.
        #[builder(into)]
        table_name: String,
        /// The name of the column used to index/group sessions (default: `"user_id"`)
        #[builder(into, default = "user_id")]
        index_column: String,
        /// Interval to check for and delete expired sessions. If not set,
        /// expired sessions will not be cleaned up automatically.
        cleanup_interval: Option<std::time::Duration>,
    ) -> Self {
        Self {
            base: SqlxBase::new(pool.clone(), table_name.clone(), index_column.clone()),
            cleanup_task: SqlxCleanupTask::new(cleanup_interval, &table_name),
            pool,
            table_name,
            index_column,
        }
    }

    fn ttl_from_row(&self, row: &PgRow) -> sqlx::Result<u32> {
        let expires: OffsetDateTime = row.try_get(EXPIRES_COLUMN)?;
        Ok(expires_to_ttl(&expires))
    }
}

#[async_trait]
impl<T> SessionStorage<T> for SqlxPostgresStorage
where
    T: SessionSqlx<Postgres>,
    <T as SessionIdentifier>::Id: for<'q> sqlx::Encode<'q, Postgres> + sqlx::Type<Postgres>,
{
    fn as_indexed_storage(&self) -> Option<&dyn SessionStorageIndexed<T>> {
        Some(self)
    }

    async fn load(
        &self,
        id: &str,
        ttl: Option<u32>,
        _cookie_jar: &CookieJar,
    ) -> SessionResult<(T, u32)> {
        let row: Option<PgRow> = self.base.load(id.into(), ttl).await?;
        let row = row.ok_or(SessionError::NotFound)?;

        let value = row.try_get(DATA_COLUMN)?;
        let data = T::from_sql(value).map_err(|e| SessionError::Parsing(Box::new(e)))?;
        let ttl = self.ttl_from_row(&row)?;

        Ok((data, ttl))
    }

    async fn save(&self, id: &str, data: T, ttl: u32) -> SessionResult<()> {
        let identifier = data.identifier();
        let value = data
            .into_sql()
            .map_err(|e| SessionError::Serialization(Box::new(e)))?;
        self.base.save(id.into(), value, identifier, ttl).await?;
        Ok(())
    }

    async fn delete(&self, id: &str, _data: T) -> SessionResult<()> {
        self.base.delete(id.into()).await?;
        Ok(())
    }

    async fn setup(&self) -> SessionResult<()> {
        self.cleanup_task.setup(&self.pool).await
    }

    async fn shutdown(&self) -> SessionResult<()> {
        self.cleanup_task.shutdown().await
    }
}

#[async_trait]
impl<T> SessionStorageIndexed<T> for SqlxPostgresStorage
where
    T: SessionSqlx<Postgres>,
    <T as SessionIdentifier>::Id: for<'q> sqlx::Encode<'q, Postgres> + sqlx::Type<Postgres>,
{
    async fn get_session_ids_by_identifier(&self, id: &T::Id) -> SessionResult<Vec<String>> {
        let sql = format!(
            "SELECT {ID_COLUMN} FROM \"{}\" \
            WHERE {} = $1 AND {EXPIRES_COLUMN} > CURRENT_TIMESTAMP",
            &self.table_name, self.index_column
        );
        let rows = sqlx::query(&sql).bind(id).fetch_all(&self.pool).await?;
        let parsed_rows = rows
            .into_iter()
            .filter_map(|row| row.try_get(ID_COLUMN).ok())
            .collect();

        Ok(parsed_rows)
    }

    async fn get_sessions_by_identifier(&self, id: &T::Id) -> SessionResult<Vec<(String, T, u32)>> {
        let sql = format!(
            "SELECT {ID_COLUMN}, {DATA_COLUMN}, {EXPIRES_COLUMN} FROM \"{}\" \
               WHERE {} = $1 AND {EXPIRES_COLUMN} > CURRENT_TIMESTAMP",
            self.table_name, self.index_column
        );
        let rows = sqlx::query(&sql).bind(id).fetch_all(&self.pool).await?;
        let parsed_rows = rows
            .into_iter()
            .filter_map(|row| {
                let id = row.try_get(ID_COLUMN).ok()?;
                let value = row.try_get(DATA_COLUMN).ok()?;
                let data = T::from_sql(value).ok()?;
                let ttl = self.ttl_from_row(&row).ok()?;
                Some((id, data, ttl))
            })
            .collect();

        Ok(parsed_rows)
    }

    async fn invalidate_sessions_by_identifier(
        &self,
        id: &T::Id,
        excluded_session_id: Option<&str>,
    ) -> SessionResult<u64> {
        let mut sql = format!(
            "DELETE FROM \"{}\" WHERE {} = $1",
            &self.table_name, self.index_column
        );
        if excluded_session_id.is_some() {
            sql.push_str(&format!(" AND {ID_COLUMN} != $2"));
        }

        let mut query = sqlx::query(&sql).bind(id);
        if let Some(excluded_id) = excluded_session_id {
            query = query.bind(excluded_id);
        }
        let rows = query.execute(&self.pool).await?;

        Ok(rows.rows_affected())
    }
}
