//! Session storage in PostgreSQL via sqlx

use rocket::{async_trait, http::CookieJar};
use sqlx::{PgPool, Row};
use time::{Duration, OffsetDateTime};

use crate::{
    error::{SessionError, SessionResult},
    storage::SessionStorageIndexed,
    SessionIdentifier,
};

use super::interface::SessionStorage;

/**
Session store using PostgreSQL via [sqlx](https://docs.rs/crate/sqlx).

Stores the session data as a string, so you'll need to implement `ToString` (or Display)
and `TryFrom<String>` for your session data type. This storage providers supports session
indexing, so you'll also need to implement [`SessionIdentifier`](crate::SessionIdentifier),
and its [`Id`](crate::SessionIdentifier::Id) must be a [type supported by sqlx](https://docs.rs/sqlx/latest/sqlx/postgres/types/index.html).
Expects a table to already exist with the following columns:

| Name | Type |
|------|---------|
| id   | `text` PRIMARY KEY |
| data | `text` NOT NULL (or `jsonb` if using JSON) |
| `<session identifier name>` | `<type>` (this should match the [`SessionIdentifier`](crate::SessionIdentifier) |
| expires | `timestamptz` NOT NULL |
*/
pub struct SqlxPostgresStorage {
    pool: PgPool,
    table_name: String,
}

impl SqlxPostgresStorage {
    pub fn new(pool: PgPool, table_name: &str) -> SqlxPostgresStorage {
        Self {
            pool,
            table_name: table_name.to_owned(),
        }
    }
}

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
                    UPDATE "{}" SET expires = $1
                    WHERE id = $2 AND expires > CURRENT_TIMESTAMP
                    RETURNING data, expires"#,
                    &self.table_name
                ))
                .bind(OffsetDateTime::now_utc() + Duration::seconds(new_ttl.into()))
                .bind(id)
                .fetch_optional(&self.pool)
                .await?
            }
            None => {
                sqlx::query(&format!(
                    r#"
                    SELECT data, expires FROM "{}"
                    WHERE id = $1 AND expires > CURRENT_TIMESTAMP"#,
                    &self.table_name
                ))
                .bind(id)
                .fetch_optional(&self.pool)
                .await?
            }
        };

        let (raw_str, expires) = match row {
            Some(row) => {
                let data: String = row.try_get("data")?;
                let expires: OffsetDateTime = row.try_get("expires")?;
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
            INSERT INTO "{}" (id, {}, data, expires)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (id) DO UPDATE SET
                data = EXCLUDED.data,
                expires = EXCLUDED.expires
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
        sqlx::query(&format!("DELETE FROM {} WHERE id = $1", &self.table_name))
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(())
    }
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
