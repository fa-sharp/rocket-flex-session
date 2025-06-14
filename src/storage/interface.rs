//! Shared interface for session storage

use std::fmt::Debug;

use rocket::{async_trait, http::CookieJar};

/// Errors that can happen during session retrieval/handling
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    /// There was no session cookie, or decryption of the cookie failed
    #[error("No session cookie")]
    NoSessionCookie,
    /// Session wasn't found in storage
    #[error("Session not found")]
    NotFound,
    /// Session was found but it was expired
    #[error("Session expired")]
    Expired,
    /// Error serializing or deserializing the session data
    #[error("Failed to serialize/deserialize session")]
    Serialization(Box<dyn std::error::Error + Send + Sync>),
    /// An unexpected error from the storage backend
    #[error("Storage backend error: {0}")]
    Backend(Box<dyn std::error::Error + Send + Sync>),

    #[cfg(feature = "redis_fred")]
    #[error("fred.rs client error: {0}")]
    RedisFredError(fred::error::Error),

    #[cfg(feature = "sqlx_postgres")]
    #[error("Sqlx error: {0}")]
    SqlxError(sqlx::Error),
}

#[cfg(feature = "redis_fred")]
impl From<fred::error::Error> for SessionError {
    fn from(value: fred::error::Error) -> Self {
        SessionError::RedisFredError(value)
    }
}

#[cfg(feature = "sqlx_postgres")]
impl From<sqlx::Error> for SessionError {
    fn from(value: sqlx::Error) -> Self {
        SessionError::SqlxError(value)
    }
}

pub type SessionResult<T> = Result<T, SessionError>;

/// Trait representing a session backend storage. You can use your own session storage
/// by implementing this trait.
#[async_trait]
pub trait SessionStorage<T>: Send + Sync
where
    T: Send + Sync,
{
    /// Load session data and TTL (time-to-live in seconds) from storage. If a TTL value is provided,
    /// it should be set upon retreiving the session. If session is already expired
    /// or otherwise invalid, a [SessionError] should be returned instead.
    async fn load(
        &self,
        id: &str,
        ttl: Option<u32>,
        cookie_jar: &CookieJar,
    ) -> SessionResult<(T, u32)>;

    /// Save or update a session in storage. This will be performed at the end of the request lifecycle.
    async fn save(&self, id: &str, data: T, ttl: u32) -> SessionResult<()>;

    /// Delete a session in storage. This will be performed at the end of the request lifecycle.
    async fn delete(&self, id: &str) -> SessionResult<()>;

    /// Optional callback when there's a pending change to the session data. A `data` value
    /// of `None` indicates a deleted session. This callback can be used by cookie-based
    /// session stores to update the cookie jar during the request.
    #[allow(unused_variables, reason = "Public trait function with default no-op")]
    fn save_cookie(
        &self,
        id: &str,
        data: Option<&T>,
        ttl: u32,
        cookie_jar: &CookieJar,
    ) -> SessionResult<()> {
        Ok(()) // Default no-op
    }

    /// Optional setup of resources that will be called on server startup
    async fn setup(&self) -> SessionResult<()> {
        Ok(()) // Default no-op
    }

    /// Optional teardown of resources that will be called on server shutdown
    async fn shutdown(&self) -> SessionResult<()> {
        Ok(()) // Default no-op
    }
}
