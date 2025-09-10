use rocket::{
    time::{Duration, OffsetDateTime},
    tokio::{
        sync::{oneshot, Mutex},
        time::interval,
    },
};

use crate::error::{SessionError, SessionResult};

pub(super) const ID_COLUMN: &str = "id";
pub(super) const DATA_COLUMN: &str = "data";
pub(super) const EXPIRES_COLUMN: &str = "expires";

/// Convert expiration time to TTL
pub(super) fn expires_to_ttl(expires: &OffsetDateTime) -> u32 {
    (*expires - OffsetDateTime::now_utc())
        .whole_seconds()
        .try_into()
        .unwrap_or(0)
}

/// Base struct for SQLx storage
pub(super) struct SqlxBase<DB: sqlx::Database> {
    pool: sqlx::Pool<DB>,
    table_name: String,
    index_column: String,
}

impl<DB> SqlxBase<DB>
where
    DB: sqlx::Database,
    for<'q> <DB as sqlx::Database>::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
    for<'c> &'c mut <DB as sqlx::Database>::Connection: sqlx::Executor<'c, Database = DB>,
    OffsetDateTime: for<'q> sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    String: for<'q> sqlx::Encode<'q, DB> + sqlx::Type<DB>,
{
    pub fn new(pool: sqlx::Pool<DB>, table_name: String, index_column: String) -> Self {
        SqlxBase {
            pool,
            table_name,
            index_column,
        }
    }

    pub async fn load(&self, id: &str, ttl: Option<u32>) -> Result<Option<DB::Row>, sqlx::Error> {
        match ttl {
            Some(new_ttl) => {
                sqlx::query(&sql::load_and_update_ttl(&self.table_name))
                    .bind(OffsetDateTime::now_utc() + Duration::seconds(new_ttl.into()))
                    .bind(id.to_owned())
                    .bind(OffsetDateTime::now_utc())
                    .fetch_optional(&self.pool)
                    .await
            }
            None => {
                sqlx::query(&sql::load(&self.table_name))
                    .bind(id.to_owned())
                    .bind(OffsetDateTime::now_utc())
                    .fetch_optional(&self.pool)
                    .await
            }
        }
    }

    pub async fn save<V, I>(
        &self,
        id: &str,
        value: V,
        index: Option<I>,
        ttl: u32,
    ) -> Result<DB::QueryResult, sqlx::Error>
    where
        V: for<'q> sqlx::Encode<'q, DB> + sqlx::Type<DB>,
        Option<I>: for<'q> sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    {
        sqlx::query(&sql::save(&self.table_name, &self.index_column))
            .bind(id.to_owned())
            .bind(index)
            .bind(value)
            .bind(OffsetDateTime::now_utc() + Duration::seconds(ttl.into()))
            .execute(&self.pool)
            .await
    }

    pub async fn delete(&self, id: &str) -> Result<DB::QueryResult, sqlx::Error> {
        sqlx::query(&sql::delete(&self.table_name))
            .bind(id.to_owned())
            .execute(&self.pool)
            .await
    }

    pub async fn session_ids_belonging_to<I>(
        &self,
        identifier: &I,
    ) -> Result<Vec<DB::Row>, sqlx::Error>
    where
        I: for<'q> sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    {
        sqlx::query(&sql::all_session_ids(&self.table_name, &self.index_column))
            .bind(identifier)
            .bind(OffsetDateTime::now_utc())
            .fetch_all(&self.pool)
            .await
    }

    pub async fn sessions_belonging_to<I>(
        &self,
        identifier: &I,
    ) -> Result<Vec<DB::Row>, sqlx::Error>
    where
        I: for<'q> sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    {
        sqlx::query(&sql::all_session_data(&self.table_name, &self.index_column))
            .bind(identifier)
            .bind(OffsetDateTime::now_utc())
            .fetch_all(&self.pool)
            .await
    }

    pub async fn invalidate_belonging_to<I>(
        &self,
        identifier: &I,
        excluded_id: Option<&str>,
    ) -> Result<DB::QueryResult, sqlx::Error>
    where
        I: for<'q> sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    {
        let sql = sql::invalidate_all(&self.table_name, &self.index_column, excluded_id.is_some());

        let mut query = sqlx::query(&sql).bind(identifier);
        if let Some(session_id) = excluded_id {
            query = query.bind(session_id.to_owned());
        }
        query.execute(&self.pool).await
    }
}

/// SQL queries
mod sql {
    use super::*;

    /// Load session data. Bind session ID and current time
    pub fn load(table_name: &str) -> String {
        format!(
            "SELECT {DATA_COLUMN}, {EXPIRES_COLUMN} FROM \"{table_name}\" \
            WHERE {ID_COLUMN} = $1 AND {EXPIRES_COLUMN} > $2"
        )
    }

    /// Load session data and update TTL. Bind expiration, session ID, and current time
    pub fn load_and_update_ttl(table_name: &str) -> String {
        format!(
            "UPDATE \"{table_name}\" SET {EXPIRES_COLUMN} = $1 \
            WHERE {ID_COLUMN} = $2 AND {EXPIRES_COLUMN} > $3 \
            RETURNING {DATA_COLUMN}, {EXPIRES_COLUMN}",
        )
    }

    /// Save session data. Bind the session ID, index, data, and expiration
    pub fn save(table_name: &str, index_column: &str) -> String {
        format!(
        "INSERT INTO \"{table_name}\" ({ID_COLUMN}, {index_column}, {DATA_COLUMN}, {EXPIRES_COLUMN}) \
        VALUES ($1, $2, $3, $4) \
        ON CONFLICT ({ID_COLUMN}) DO UPDATE SET \
            {DATA_COLUMN} = EXCLUDED.{DATA_COLUMN}, \
            {EXPIRES_COLUMN} = EXCLUDED.{EXPIRES_COLUMN}"
    )
    }

    /// Delete session data. Bind the session ID
    pub fn delete(table_name: &str) -> String {
        format!("DELETE FROM \"{table_name}\" WHERE {ID_COLUMN} = $1")
    }

    /// Get session IDs belonging to a user/identifier. Bind the identifier and current time
    pub fn all_session_ids(table_name: &str, index_column: &str) -> String {
        format!(
            "SELECT {ID_COLUMN} FROM \"{table_name}\" \
            WHERE {index_column} = $1 AND {EXPIRES_COLUMN} > $2"
        )
    }

    /// Get session data belonging to a user/identifier. Bind the identifier and current time
    pub fn all_session_data(table_name: &str, index_column: &str) -> String {
        format!(
            "SELECT {ID_COLUMN}, {DATA_COLUMN}, {EXPIRES_COLUMN} FROM \"{table_name}\" \
            WHERE {index_column} = $1 AND {EXPIRES_COLUMN} > $2"
        )
    }

    /// Invalidate all sessions belonging to a user/identifier. Bind the identifier and the optional session ID to exclude
    pub fn invalidate_all(table_name: &str, index_column: &str, excluded_id: bool) -> String {
        let mut sql = format!("DELETE FROM \"{table_name}\" WHERE {index_column} = $1");
        if excluded_id {
            sql.push_str(&format!(" AND {ID_COLUMN} != $2"));
        }
        sql
    }
}

/// Session cleanup task
#[derive(Default)]
pub(super) struct SqlxCleanupTask {
    interval: Option<std::time::Duration>,
    shutdown_tx: Mutex<Option<oneshot::Sender<u8>>>,
    table_name: String,
}

impl SqlxCleanupTask {
    pub fn new(cleanup_interval: Option<std::time::Duration>, table_name: &str) -> Self {
        Self {
            interval: cleanup_interval,
            shutdown_tx: Mutex::default(),
            table_name: table_name.to_string(),
        }
    }

    pub async fn setup<DB>(&self, pool: &sqlx::Pool<DB>) -> SessionResult<()>
    where
        DB: sqlx::Database,
        for<'q> <DB as sqlx::Database>::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
        for<'c> &'c mut <DB as sqlx::Database>::Connection: sqlx::Executor<'c, Database = DB>,
        OffsetDateTime: for<'q> sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    {
        let Some(cleanup_interval) = self.interval else {
            return Ok(());
        };

        let (tx, mut rx) = oneshot::channel();
        self.shutdown_tx.lock().await.replace(tx);

        let pool = pool.clone();
        let table_name = self.table_name.clone();
        rocket::tokio::spawn(async move {
            rocket::info!("Starting session cleanup monitor");
            let mut interval = interval(cleanup_interval);
            loop {
                rocket::tokio::select! {
                    _ = interval.tick() => {
                        rocket::debug!("Cleaning up expired sessions");
                        if let Err(e) = sqlx::query(&format!(
                            "DELETE FROM \"{table_name}\" WHERE {EXPIRES_COLUMN} < $1"
                            ))
                            .bind(OffsetDateTime::now_utc())
                            .execute(&pool)
                            .await
                        {
                            rocket::error!("Error deleting expired sessions: {e}");
                        }
                    }
                    _ = &mut rx => {
                        rocket::info!("Session cleanup monitor shutdown");
                    }
                }
            }
        });

        Ok(())
    }

    pub async fn shutdown(&self) -> SessionResult<()> {
        if let Some(tx) = self.shutdown_tx.lock().await.take() {
            tx.send(0).map_err(|_| {
                SessionError::SetupTeardown("Failed to send shutdown signal".to_string())
            })?;
        }
        Ok(())
    }
}
