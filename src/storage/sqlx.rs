//! Session storage via sqlx

mod base;
use base::*;

#[cfg(feature = "sqlx_postgres")]
mod postgres;
#[cfg(feature = "sqlx_postgres")]
pub use postgres::SqlxPostgresStorage;

#[cfg(feature = "sqlx_sqlite")]
mod sqlite;
#[cfg(feature = "sqlx_sqlite")]
pub use sqlite::SqlxSqliteStorage;

use crate::SessionIdentifier;

/**
Trait for session data types that can be stored using sqlx.
The generic parameter `Database` represents the sqlx database type.
# Example

```
use rocket_flex_session::error::SessionError;
use rocket_flex_session::storage::sqlx::SessionSqlx;
use rocket_flex_session::SessionIdentifier;

#[derive(Clone)]
struct SessionData {
    user_id: i32,
    data: String,
}

// Implement SessionIdentifier to define how to group/index sessions
impl SessionIdentifier for SessionData {
    type Id = i32; // must be a type supported by sqlx
    fn identifier(&self) -> Option<Self::Id> {
        Some(self.user_id) // this will typically be the user ID
    }
}

impl SessionSqlx<sqlx::Postgres> for SessionData {
    type Error = SessionError; // or a custom error
    type Data = String; // the data type to be stored

    fn into_sql(self) -> Result<Self::Data, Self::Error> {
        Ok(format!("{}:{}", self.user_id, self.data))
    }

    fn from_sql(value: Self::Data) -> Result<Self, Self::Error> {
        let (user_id, data) = value.split_once(':').ok_or(SessionError::InvalidData)?;
        Ok(SessionData {
            user_id: user_id.parse().map_err(|e| SessionError::Parsing(Box::new(e)))?,
            data: data.to_owned(),
        })
    }
}
```
*/
pub trait SessionSqlx<Database>
where
    Self: SessionIdentifier + 'static,
    <Self as SessionIdentifier>::Id: for<'q> sqlx::Encode<'q, Database> + sqlx::Type<Database>,
    Database: sqlx::Database,
{
    /// The error that can occur when converting to/from the SQL value.
    type Error: std::error::Error + Send + Sync + 'static;

    /// The data type that can be stored in the SQL database. Must be a type supported by sqlx.
    type Data: for<'q> sqlx::Encode<'q, Database>
        + for<'q> sqlx::Decode<'q, Database>
        + sqlx::Type<Database>
        + Send
        + Sync;

    /// Convert this session into a SQL value.
    fn into_sql(self) -> Result<Self::Data, Self::Error>;

    /// Convert a SQL value into the session data type.
    fn from_sql(value: Self::Data) -> Result<Self, Self::Error>;
}
