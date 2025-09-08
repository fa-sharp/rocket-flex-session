use bon::Builder;
use rocket::{
    async_trait,
    http::CookieJar,
    time::{Duration, OffsetDateTime},
    tokio::{
        self,
        sync::{oneshot, Mutex},
        time::interval,
    },
};
use sqlx::{postgres::PgRow, PgPool, Row};

use crate::{
    error::{SessionError, SessionResult},
    storage::{SessionStorage, SessionStorageIndexed},
    SessionIdentifier,
};

const ID_COLUMN: &str = "id";
const DATA_COLUMN: &str = "data";
const EXPIRES_COLUMN: &str = "expires";

/**
Session store using PostgreSQL via [sqlx](https://docs.rs/crate/sqlx) that stores session data as a string, and supports session indexing.

# Requirements
You'll need to implement `TryInto<String>` and `TryFrom<String>` for your session data type. You'll also need to implement [`SessionIdentifier`],
and its [`Id`](crate::SessionIdentifier::Id) type must be a [type supported by sqlx](https://docs.rs/sqlx/latest/sqlx/postgres/types/index.html).
Expects a table to already exist with the following columns:

| Name | Type |
|------|---------|
| id   | `text` PRIMARY KEY |
| data | `text` NOT NULL (or `jsonb` if using JSON) |
| `<identifier>` | `<type>` (this identifier and type should match your [`SessionIdentifier`] impl) |
| expires | `timestamptz` NOT NULL |

# Creating the storage
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
        // optional auto-deletion of expired sessions
        .cleanup_interval(Duration::from_secs(600))
        .build()
}
```
*/
#[derive(Builder)]
pub struct SqlxPostgresStorage {
    /// An initialized Postgres connection pool.
    pool: PgPool,
    /// The name of the table to use for storing sessions.
    #[builder(into)]
    table_name: String,
    /// Interval to check for and delete expired sessions. If not set,
    /// expired sessions won't be cleaned up automatically.
    cleanup_interval: Option<std::time::Duration>,
    #[builder(skip)]
    shutdown_tx: Mutex<Option<oneshot::Sender<()>>>,
}

impl SqlxPostgresStorage {
    fn id_from_row(&self, row: &PgRow) -> sqlx::Result<String> {
        row.try_get(ID_COLUMN)
    }

    fn raw_data_from_row(&self, row: &PgRow) -> sqlx::Result<String> {
        row.try_get(DATA_COLUMN)
    }

    fn ttl_from_row(&self, row: &PgRow) -> sqlx::Result<u32> {
        let expires: OffsetDateTime = row.try_get(EXPIRES_COLUMN)?;
        let ttl = (expires - OffsetDateTime::now_utc())
            .whole_seconds()
            .try_into()
            .unwrap_or(0);
        Ok(ttl)
    }
}

#[async_trait]
impl<T> SessionStorage<T> for SqlxPostgresStorage
where
    T: SessionIdentifier + TryFrom<String> + TryInto<String> + Clone + Send + Sync + 'static,
    <T as SessionIdentifier>::Id:
        for<'q> sqlx::Encode<'q, sqlx::Postgres> + sqlx::Type<sqlx::Postgres>,
    <T as TryInto<String>>::Error: std::error::Error + Send + Sync + 'static,
    <T as TryFrom<String>>::Error: std::error::Error + Send + Sync + 'static,
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
        let row = match ttl {
            Some(new_ttl) => {
                let sql = format!(
                    "UPDATE \"{}\" SET {EXPIRES_COLUMN} = $1 \
                    WHERE {ID_COLUMN} = $2 AND {EXPIRES_COLUMN} > CURRENT_TIMESTAMP \
                    RETURNING {DATA_COLUMN}, {EXPIRES_COLUMN}",
                    &self.table_name,
                );
                sqlx::query(&sql)
                    .bind(OffsetDateTime::now_utc() + Duration::seconds(new_ttl.into()))
                    .bind(id)
                    .fetch_optional(&self.pool)
                    .await?
            }
            None => {
                let sql = format!(
                    "SELECT {DATA_COLUMN}, {EXPIRES_COLUMN} FROM \"{}\" \
                    WHERE {ID_COLUMN} = $1 AND {EXPIRES_COLUMN} > CURRENT_TIMESTAMP",
                    &self.table_name,
                );
                sqlx::query(&sql)
                    .bind(id)
                    .fetch_optional(&self.pool)
                    .await?
            }
        };

        let (raw_str, ttl) = match row {
            Some(row) => {
                let data = self.raw_data_from_row(&row)?;
                let ttl = self.ttl_from_row(&row)?;
                (data, ttl)
            }
            None => return Err(SessionError::NotFound),
        };
        let data = T::try_from(raw_str).map_err(|e| SessionError::Serialization(Box::new(e)))?;
        Ok((data, ttl))
    }

    async fn save(&self, id: &str, data: T, ttl: u32) -> SessionResult<()> {
        let sql = format!(
            "INSERT INTO \"{}\" ({ID_COLUMN}, {}, {DATA_COLUMN}, {EXPIRES_COLUMN}) \
            VALUES ($1, $2, $3, $4) \
            ON CONFLICT ({ID_COLUMN}) DO UPDATE SET \
                {DATA_COLUMN} = EXCLUDED.{DATA_COLUMN}, \
                {EXPIRES_COLUMN} = EXCLUDED.{EXPIRES_COLUMN}",
            self.table_name,
            T::IDENTIFIER
        );
        let identifier = data.identifier().cloned();
        let data_str: String = data
            .try_into()
            .map_err(|e| SessionError::Serialization(Box::new(e)))?;
        sqlx::query(&sql)
            .bind(id)
            .bind(identifier)
            .bind(data_str)
            .bind(OffsetDateTime::now_utc() + Duration::seconds(ttl.into()))
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn delete(&self, id: &str, _cookie_jar: &CookieJar) -> SessionResult<()> {
        let sql = format!("DELETE FROM {} WHERE {ID_COLUMN} = $1", &self.table_name);
        sqlx::query(&sql).bind(id).execute(&self.pool).await?;

        Ok(())
    }

    async fn setup(&self) -> SessionResult<()> {
        let Some(cleanup_interval) = self.cleanup_interval else {
            return Ok(());
        };
        let (tx, mut rx) = oneshot::channel();
        let pool = self.pool.clone();
        let table_name = self.table_name.clone();
        tokio::spawn(async move {
            rocket::info!("Starting session cleanup monitor");
            let mut interval = interval(cleanup_interval);
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        rocket::debug!("Cleaning up expired sessions");
                        if let Err(e) = cleanup_expired_sessions(&table_name, &pool).await {
                            rocket::error!("Error deleting expired sessions: {e}");
                        }
                    }
                    _ = &mut rx => {
                        rocket::info!("Session cleanup monitor shutdown");
                    }
                }
            }
        });
        self.shutdown_tx.lock().await.replace(tx);

        Ok(())
    }

    async fn shutdown(&self) -> SessionResult<()> {
        if let Some(tx) = self.shutdown_tx.lock().await.take() {
            tx.send(()).map_err(|_| {
                SessionError::SetupTeardown("Failed to send shutdown signal".to_string())
            })?;
        }
        Ok(())
    }
}

async fn cleanup_expired_sessions(table_name: &str, pool: &PgPool) -> Result<u64, sqlx::Error> {
    rocket::debug!("Cleaning up expired sessions");
    let sql = format!("DELETE FROM \"{table_name}\" WHERE {EXPIRES_COLUMN} < $1");
    let rows = sqlx::query(&sql)
        .bind(OffsetDateTime::now_utc())
        .execute(pool)
        .await?;
    Ok(rows.rows_affected())
}

#[async_trait]
impl<T> SessionStorageIndexed<T> for SqlxPostgresStorage
where
    T: SessionIdentifier + TryFrom<String> + TryInto<String> + Clone + Send + Sync + 'static,
    <T as SessionIdentifier>::Id:
        for<'q> sqlx::Encode<'q, sqlx::Postgres> + sqlx::Type<sqlx::Postgres>,
    <T as TryInto<String>>::Error: std::error::Error + Send + Sync + 'static,
    <T as TryFrom<String>>::Error: std::error::Error + Send + Sync + 'static,
{
    async fn get_session_ids_by_identifier(&self, id: &T::Id) -> SessionResult<Vec<String>> {
        let sql = format!(
            "SELECT {ID_COLUMN} FROM \"{}\" \
            WHERE {} = $1 AND {EXPIRES_COLUMN} > CURRENT_TIMESTAMP",
            &self.table_name,
            T::IDENTIFIER
        );
        let rows = sqlx::query(&sql).bind(id).fetch_all(&self.pool).await?;
        let parsed_rows = rows
            .into_iter()
            .filter_map(|row| self.id_from_row(&row).ok())
            .collect();

        Ok(parsed_rows)
    }

    async fn get_sessions_by_identifier(&self, id: &T::Id) -> SessionResult<Vec<(String, T, u32)>> {
        let sql = format!(
            "SELECT {ID_COLUMN}, {DATA_COLUMN}, {EXPIRES_COLUMN} FROM \"{}\" \
               WHERE {} = $1 AND {EXPIRES_COLUMN} > CURRENT_TIMESTAMP",
            &self.table_name,
            T::IDENTIFIER
        );
        let rows = sqlx::query(&sql).bind(id).fetch_all(&self.pool).await?;
        let parsed_rows = rows
            .into_iter()
            .filter_map(|row| {
                let id = self.id_from_row(&row).ok()?;
                let raw_data = self.raw_data_from_row(&row).ok()?;
                let data = T::try_from(raw_data).ok()?;
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
            &self.table_name,
            T::IDENTIFIER
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
