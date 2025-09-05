//! Session storage in PostgreSQL via sqlx

use rocket::{
    async_trait,
    http::CookieJar,
    tokio::{
        self,
        sync::{oneshot, Mutex},
        time::interval,
    },
};
use sqlx::{PgPool, Row};
use time::{Duration, OffsetDateTime};

use crate::{
    error::{SessionError, SessionResult},
    storage::SessionStorageIndexed,
    SessionIdentifier,
};

use super::interface::SessionStorage;

/**
Session store using PostgreSQL via [sqlx](https://docs.rs/crate/sqlx) that stores session data as a string, and supports session indexing.

You'll need to implement `ToString` (or Display) and `TryFrom<String>` for your session data type. You'll also need to implement [`SessionIdentifier`],
and its [`Id`](crate::SessionIdentifier::Id) must be a [type supported by sqlx](https://docs.rs/sqlx/latest/sqlx/postgres/types/index.html).
Expects a table to already exist with the following columns:

| Name | Type |
|------|---------|
| id   | `text` PRIMARY KEY |
| data | `text` NOT NULL (or `jsonb` if using JSON) |
| `<session identifier name>` | `<type>` (the name and type should match your [`SessionIdentifier`] impl) |
| expires | `timestamptz` NOT NULL |
*/
pub struct SqlxPostgresStorage {
    pool: PgPool,
    table_name: String,
    cleanup_interval: Option<std::time::Duration>,
    shutdown_tx: Mutex<Option<oneshot::Sender<()>>>,
}

impl SqlxPostgresStorage {
    /// Creates a new [`SqlxPostgresStorage`].
    ///
    /// Parameters:
    /// - `pool`: An initialized Postgres connection pool.
    /// - `table_name`: The name of the table to use for storing sessions.
    /// - `cleanup_interval`: Interval to check for and clean up expired sessions. If `None`,
    ///    expired sessions won't be cleaned up automatically.
    pub fn new(
        pool: PgPool,
        table_name: &str,
        cleanup_interval: Option<std::time::Duration>,
    ) -> SqlxPostgresStorage {
        Self {
            pool,
            table_name: table_name.to_owned(),
            cleanup_interval,
            shutdown_tx: Mutex::default(),
        }
    }
}

const ID_COLUMN: &str = "id";
const DATA_COLUMN: &str = "data";
const EXPIRES_COLUMN: &str = "expires";

#[async_trait]
impl<T> SessionStorage<T> for SqlxPostgresStorage
where
    T: SessionIdentifier + TryFrom<String> + ToString + Clone + Send + Sync + 'static,
    <T as SessionIdentifier>::Id:
        for<'q> sqlx::Encode<'q, sqlx::Postgres> + sqlx::Type<sqlx::Postgres>,
    <T as TryFrom<String>>::Error: std::error::Error + Send + Sync + 'static,
{
    async fn load(
        &self,
        id: &str,
        ttl: Option<u32>,
        _cookie_jar: &CookieJar,
    ) -> SessionResult<(T, u32)> {
        let row = match ttl {
            Some(new_ttl) => {
                sqlx::query(&format!(
                    r#"
                    UPDATE "{}" SET {EXPIRES_COLUMN} = $1
                    WHERE {ID_COLUMN} = $2 AND {EXPIRES_COLUMN} > CURRENT_TIMESTAMP
                    RETURNING {DATA_COLUMN}, {EXPIRES_COLUMN}"#,
                    &self.table_name,
                ))
                .bind(OffsetDateTime::now_utc() + Duration::seconds(new_ttl.into()))
                .bind(id)
                .fetch_optional(&self.pool)
                .await?
            }
            None => {
                sqlx::query(&format!(
                    r#"
                    SELECT {DATA_COLUMN}, {EXPIRES_COLUMN} FROM "{}"
                    WHERE {ID_COLUMN} = $1 AND {EXPIRES_COLUMN} > CURRENT_TIMESTAMP"#,
                    &self.table_name,
                ))
                .bind(id)
                .fetch_optional(&self.pool)
                .await?
            }
        };

        let (raw_str, expires) = match row {
            Some(row) => {
                let data: String = row.try_get(DATA_COLUMN)?;
                let expires: OffsetDateTime = row.try_get(EXPIRES_COLUMN)?;
                (data, expires)
            }
            None => return Err(SessionError::NotFound),
        };
        let data = T::try_from(raw_str).map_err(|e| SessionError::Serialization(Box::new(e)))?;
        let ttl = (expires - OffsetDateTime::now_utc()).whole_seconds();

        Ok((data, ttl.try_into().unwrap_or(0)))
    }

    async fn save(&self, id: &str, data: T, ttl: u32) -> SessionResult<()> {
        sqlx::query(&format!(
            r#"
            INSERT INTO "{}" ({ID_COLUMN}, {}, {DATA_COLUMN}, {EXPIRES_COLUMN})
            VALUES ($1, $2, $3, $4)
            ON CONFLICT ({ID_COLUMN}) DO UPDATE SET
                {DATA_COLUMN} = EXCLUDED.{DATA_COLUMN},
                {EXPIRES_COLUMN} = EXCLUDED.{EXPIRES_COLUMN}
            "#,
            self.table_name,
            T::NAME
        ))
        .bind(id)
        .bind(data.identifier())
        .bind(data.to_string())
        .bind(OffsetDateTime::now_utc() + Duration::seconds(ttl.into()))
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn delete(&self, id: &str, _cookie_jar: &CookieJar) -> SessionResult<()> {
        sqlx::query(&format!(
            "DELETE FROM {} WHERE {ID_COLUMN} = $1",
            &self.table_name
        ))
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn setup(&self) -> SessionResult<()> {
        let Some(cleanup_interval) = self.cleanup_interval else {
            return Ok(());
        };
        let (tx, rx) = oneshot::channel();
        let pool = self.pool.clone();
        let table_name = self.table_name.clone();
        tokio::spawn(async move {
            rocket::info!("Starting session cleanup monitor");
            let mut interval = interval(cleanup_interval);
            tokio::select! {
                _ = async {
                    loop {
                        interval.tick().await;
                        rocket::debug!("Cleaning up expired sessions");
                        if let Err(e) = cleanup_expired_sessions(&table_name, &pool).await {
                            rocket::error!("Error deleting expired sessions: {e}");
                        }
                    }
                } => (),
                _ = rx => {
                    rocket::info!("Session cleanup monitor shutdown");
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
    let rows = sqlx::query(&format!(
        "DELETE FROM {table_name} WHERE {EXPIRES_COLUMN} < $1"
    ))
    .bind(OffsetDateTime::now_utc())
    .execute(pool)
    .await?;
    Ok(rows.rows_affected())
}

#[async_trait]
impl<T> SessionStorageIndexed<T> for SqlxPostgresStorage
where
    T: SessionIdentifier + TryFrom<String> + ToString + Clone + Send + Sync + 'static,
    <T as SessionIdentifier>::Id:
        for<'q> sqlx::Encode<'q, sqlx::Postgres> + sqlx::Type<sqlx::Postgres>,
    <T as TryFrom<String>>::Error: std::error::Error + Send + Sync + 'static,
{
    async fn get_sessions_by_identifier(&self, id: &T::Id) -> SessionResult<Vec<(String, T)>> {
        let rows = sqlx::query(&format!(
            r#"
            SELECT id, data FROM "{}"
            WHERE {} = $1 AND expires > CURRENT_TIMESTAMP"#,
            &self.table_name,
            T::NAME
        ))
        .bind(id)
        .fetch_all(&self.pool)
        .await?;

        let parsed_rows = rows
            .into_iter()
            .filter_map(|row| {
                let id: String = row.try_get(0).ok()?;
                let raw_data: String = row.try_get(1).ok()?;
                let data = T::try_from(raw_data).ok()?;
                Some((id, data))
            })
            .collect();
        Ok(parsed_rows)
    }

    async fn get_session_ids_by_identifier(&self, id: &T::Id) -> SessionResult<Vec<String>> {
        let rows = sqlx::query(&format!(
            r#"
            SELECT id FROM "{}"
            WHERE {} = $1 AND expires > CURRENT_TIMESTAMP"#,
            &self.table_name,
            T::NAME
        ))
        .bind(id)
        .fetch_all(&self.pool)
        .await?;

        let parsed_rows = rows
            .into_iter()
            .filter_map(|row| row.try_get(0).ok())
            .collect();
        Ok(parsed_rows)
    }

    async fn invalidate_sessions_by_identifier(&self, id: &T::Id) -> SessionResult<()> {
        let _rows = sqlx::query(&format!(
            r#"
            DELETE FROM "{}"
            WHERE {} = $1"#,
            &self.table_name,
            T::NAME
        ))
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }
}
