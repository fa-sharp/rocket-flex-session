//! Error types

/// Result type for session operations
pub type SessionResult<T> = Result<T, SessionError>;

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
    #[error("Failed to serialize/deserialize session: {0}")]
    Serialization(Box<dyn std::error::Error + Send + Sync>),
    /// An indexing operation failed because the storage provider doesn't
    /// implement [SessionStorageIndexed](crate::storage::SessionStorageIndexed)
    #[error("Storage doesn't support indexing")]
    NonIndexedStorage,
    /// A generic error from the storage backend. This error type can be
    /// used when implementing a custom session storage.
    #[error("Storage backend error: {0}")]
    Backend(Box<dyn std::error::Error + Send + Sync>),

    #[cfg(feature = "redis_fred")]
    #[error("fred.rs client error: {0}")]
    RedisFredError(#[from] fred::error::Error),

    #[cfg(feature = "sqlx_postgres")]
    #[error("Sqlx error: {0}")]
    SqlxError(#[from] sqlx::Error),
}
