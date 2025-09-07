//! Shared interface for session storage

use rocket::{async_trait, http::CookieJar};

use crate::{error::SessionResult, SessionIdentifier};

/// Trait representing a session backend storage. You can use your own session storage
/// by implementing this trait.
#[async_trait]
pub trait SessionStorage<T>: Send + Sync
where
    T: Send + Sync,
{
    /// Load session data and TTL (time-to-live in seconds) from storage. If a TTL value is provided,
    /// it should be set upon retreiving the session. If session is already expired
    /// or otherwise invalid, a [`SessionError`](crate::error::SessionError) should be returned instead.
    async fn load(
        &self,
        id: &str,
        ttl: Option<u32>,
        cookie_jar: &CookieJar,
    ) -> SessionResult<(T, u32)>;

    /// Save or update a session in storage. This will be performed at the end of the request lifecycle.
    async fn save(&self, id: &str, data: T, ttl: u32) -> SessionResult<()>;

    /// Delete a session in storage. This will be performed at the end of the request lifecycle.
    async fn delete(&self, id: &str, cookie_jar: &CookieJar) -> SessionResult<()>;

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

    /// Storages that support indexing (by implementing [`SessionStorageIndexed`]) must
    /// also implement this. Implementation should be trivial: `Some(self)`
    fn as_indexed_storage(&self) -> Option<&dyn SessionStorageIndexed<T>> {
        None // Default not supported
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

/// Extended trait for storage backends that support session indexing by identifier.
/// This allows operations like finding all sessions for a user or bulk invalidation.
///
/// Not all storage backends can support this - for example, cookie-based storage
/// cannot implement this trait since cookies are only persisted on the client-side.
#[async_trait]
pub trait SessionStorageIndexed<T>: SessionStorage<T>
where
    T: SessionIdentifier + Send + Sync,
{
    /// Retrieve all tracked session IDs and data for the given identifier.
    async fn get_sessions_by_identifier(&self, id: &T::Id) -> SessionResult<Vec<(String, T)>>;

    /// Get all tracked session IDs associated with the given identifier.
    async fn get_session_ids_by_identifier(&self, id: &T::Id) -> SessionResult<Vec<String>>;

    /// Invalidate all tracked sessions associated with the given identifier, optionally excluding one session ID.
    /// Returns the number of sessions invalidated.
    async fn invalidate_sessions_by_identifier(
        &self,
        id: &T::Id,
        excluded_session_id: Option<&str>,
    ) -> SessionResult<u64>;
}
