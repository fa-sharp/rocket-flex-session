use bon::bon;
use rocket::{async_trait, http::CookieJar};
use sqlx::{sqlite::SqliteRow, Row, Sqlite, SqlitePool};

use crate::{
    error::{SessionError, SessionResult},
    storage::{SessionStorage, SessionStorageIndexed},
};

use super::*;

/** Session store using SQLite via [sqlx](https://docs.rs/crate/sqlx).

# Requirements
- You must pass in an initialized sqlx SQLite connection pool.
- Your session data type must implement [`SessionSqlx`] to configure how to convert & store session data.
- Your session data type must implement [`SessionIdentifier`]. The SessionIdentifier's
[Id](`SessionIdentifier::Id`) type must be a type supported by sqlx.
- Expects a table to already exist with the following columns:

| Name | Type |
|------|---------|
| id   | TEXT NOT NULL PRIMARY KEY |
| data | TEXT NOT NULL  |
| user_id | TEXT |
| expires | TEXT NOT NULL |

The name of the session index column ("user_id") can be customized when building the storage.

 */
pub struct SqlxSqliteStorage {
    pool: SqlitePool,
    base: SqlxBase<Sqlite>,
    cleanup_task: SqlxCleanupTask,
}

#[bon]
impl SqlxSqliteStorage {
    #[builder]
    pub fn new(
        /// An initialized SQLite connection pool.
        pool: SqlitePool,
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
            cleanup_task: SqlxCleanupTask::new(cleanup_interval, &table_name),
            base: SqlxBase::new(pool.clone(), table_name, index_column),
            pool,
        }
    }
}

#[async_trait]
impl<T> SessionStorage<T> for SqlxSqliteStorage
where
    T: SessionSqlx<Sqlite>,
    <T as SessionIdentifier>::Id: for<'q> sqlx::Encode<'q, Sqlite> + sqlx::Type<Sqlite>,
{
    // fn as_indexed_storage(&self) -> Option<&dyn SessionStorageIndexed<T>> {
    //     Some(self)
    // }

    async fn load(
        &self,
        id: &str,
        ttl: Option<u32>,
        _cookie_jar: &CookieJar,
    ) -> SessionResult<(T, u32)> {
        let row: Option<SqliteRow> = self.base.load(id, ttl).await?;
        let row = row.ok_or(SessionError::NotFound)?;

        let value = row.try_get(DATA_COLUMN)?;
        let data = T::from_sql(value).map_err(|e| SessionError::Parsing(Box::new(e)))?;
        let expires = row.try_get(EXPIRES_COLUMN)?;

        Ok((data, expires_to_ttl(&expires)))
    }

    async fn save(&self, id: &str, data: T, ttl: u32) -> SessionResult<()> {
        let identifier = data.identifier();
        let value = data
            .into_sql()
            .map_err(|e| SessionError::Serialization(Box::new(e)))?;
        self.base.save(id, value, identifier, ttl).await?;
        Ok(())
    }

    async fn delete(&self, id: &str, _data: T) -> SessionResult<()> {
        self.base.delete(id).await?;
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
impl<T> SessionStorageIndexed<T> for SqlxSqliteStorage
where
    T: SessionSqlx<Sqlite>,
    <T as SessionIdentifier>::Id: for<'q> sqlx::Encode<'q, Sqlite> + sqlx::Type<Sqlite>,
{
    async fn get_session_ids_by_identifier(&self, id: &T::Id) -> SessionResult<Vec<String>> {
        let rows = self.base.session_ids_belonging_to(id).await?;
        let session_ids = rows
            .into_iter()
            .filter_map(|row| row.try_get(ID_COLUMN).ok())
            .collect();

        Ok(session_ids)
    }

    async fn get_sessions_by_identifier(&self, id: &T::Id) -> SessionResult<Vec<(String, T, u32)>> {
        let rows = self.base.sessions_belonging_to(id).await?;
        let parsed_rows = rows
            .into_iter()
            .filter_map(|row| {
                let id = row.try_get(ID_COLUMN).ok()?;
                let value = row.try_get(DATA_COLUMN).ok()?;
                let data = T::from_sql(value).ok()?;
                let expires = row.try_get(EXPIRES_COLUMN).ok()?;
                Some((id, data, expires_to_ttl(&expires)))
            })
            .collect();

        Ok(parsed_rows)
    }

    async fn invalidate_sessions_by_identifier(
        &self,
        id: &T::Id,
        excluded_session_id: Option<&str>,
    ) -> SessionResult<u64> {
        let rows = self
            .base
            .invalidate_belonging_to(id, excluded_session_id)
            .await?;

        Ok(rows.rows_affected())
    }
}
