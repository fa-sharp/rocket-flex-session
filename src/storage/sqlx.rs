//! Session storage in PostgreSQL via sqlx

use rocket::{async_trait, http::CookieJar};
use sqlx::{PgPool, Row};
use time::{Duration, OffsetDateTime};

use super::interface::{SessionError, SessionResult, SessionStorage};

/**
Session store using PostgreSQL via [sqlx](https://docs.rs/crate/sqlx). Stores the session data as a string, so you'll need
to implement `TryFrom<YourSession> for String` and `TryFrom<String> for YourSession`
for your session data type. Expects a table to already exist with the following columns:
| Name | Type |
|------|---------|
| id   | text PRIMARY KEY |
| data | text NOT NULL (or jsonb if using JSON) |
| expires | timestamptz NOT NULL |
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
    T: TryFrom<String> + TryInto<String> + Clone + Send + Sync + 'static,
    <T as TryFrom<String>>::Error: std::error::Error + Send + Sync + 'static,
    <T as TryInto<String>>::Error: std::error::Error + Send + Sync + 'static,
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
        let raw_str: String = data
            .try_into()
            .map_err(|e| SessionError::Serialization(Box::new(e)))?;
        let expires = OffsetDateTime::now_utc() + Duration::seconds(ttl.into());

        sqlx::query(&format!(
            r#"
            INSERT INTO "{}" (id, data, expires)
            VALUES ($1, $2, $3)
            ON CONFLICT (id) DO UPDATE SET
                data = EXCLUDED.data,
                expires = EXCLUDED.expires
            "#,
            self.table_name
        ))
        .bind(id)
        .bind(raw_str)
        .bind(expires)
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
