use crate::{error::SessionError, storage::SessionStorageIndexed, Session};

/// Optional trait for session data types that can be grouped by an identifier.
/// This enables features like retrieving all sessions for a user or invalidating
/// all sessions when a user's password changes.
///
/// The storage provider must support indexing sessions (check the docs for the
/// provider you're using).
///
/// # Example
/// ```rust
/// use rocket_flex_session::SessionIdentifier;
///
/// #[derive(Clone)]
/// struct MySession {
///     user_id: String,
///     role: String,
/// }
///
/// impl SessionIdentifier for MySession {
///     const NAME: &str = "user_id";
///     type Id = String;
///
///     fn identifier(&self) -> Option<&Self::Id> {
///         Some(&self.user_id)
///     }
/// }
/// ```
pub trait SessionIdentifier {
    /// The name of the identifier (default: `"user_id"`), that may be used as a field/key name by the storage backend.
    const IDENTIFIER: &str = "user_id";

    /// The type of the identifier
    type Id: Send + Sync + Clone;

    /// Extract the identifier from the session data.
    /// Returns `None` if the session doesn't have an identifier and/or
    /// shouldn't be indexed.
    fn identifier(&self) -> Option<&Self::Id>;
}

/// Session implementation block for indexing operations
impl<'a, T> Session<'a, T>
where
    T: SessionIdentifier + Send + Sync + Clone,
{
    /// Get all session IDs and data for the same identifier as the current session.
    /// Returns `None` if there's no session or the session isn't indexed.
    pub async fn get_all_sessions(&self) -> Result<Option<Vec<(String, T)>>, SessionError> {
        let Some(identifier) = self.get_identifier() else {
            return Ok(None);
        };
        let storage = self.get_indexed_storage()?;
        let sessions = storage.get_sessions_by_identifier(&identifier).await?;

        Ok(Some(sessions))
    }

    /// Get all session IDs for the same identifier as the current session.
    /// Returns `None` if there's no session or the session isn't indexed.
    pub async fn get_all_session_ids(&self) -> Result<Option<Vec<String>>, SessionError> {
        let Some(identifier) = self.get_identifier() else {
            return Ok(None);
        };
        let storage = self.get_indexed_storage()?;
        let session_ids = storage.get_session_ids_by_identifier(&identifier).await?;

        Ok(Some(session_ids))
    }

    /// Invalidate all sessions with the same identifier as the current session, optionally keeping the current session active.
    /// Returns the number of sessions invalidated, or `None` if there's no session or the session isn't indexed.
    pub async fn invalidate_all_sessions(
        &self,
        keep_current: bool,
    ) -> Result<Option<u64>, SessionError> {
        let Some((session_id, identifier)) = self.id().zip(self.get_identifier()) else {
            return Ok(None);
        };
        let storage = self.get_indexed_storage()?;
        let num_sessions = storage
            .invalidate_sessions_by_identifier(
                &identifier,
                keep_current.then_some(session_id.as_str()),
            )
            .await?;

        Ok(Some(num_sessions))
    }

    /// Get all session IDs and data for a specific identifier.
    pub async fn get_sessions_by_identifier(
        &self,
        identifier: &T::Id,
    ) -> Result<Vec<(String, T)>, SessionError> {
        let storage = self.get_indexed_storage()?;
        storage.get_sessions_by_identifier(identifier).await
    }

    /// Get all session IDs for a specific identifier.
    pub async fn get_session_ids_by_identifier(
        &self,
        identifier: &T::Id,
    ) -> Result<Vec<String>, SessionError> {
        let storage = self.get_indexed_storage()?;
        storage.get_session_ids_by_identifier(identifier).await
    }

    /// Invalidate all sessions for a specific identifier, returning the number of sessions invalidated.
    pub async fn invalidate_sessions_by_identifier(
        &self,
        identifier: &T::Id,
    ) -> Result<u64, SessionError> {
        let storage = self.get_indexed_storage()?;
        storage
            .invalidate_sessions_by_identifier(identifier, None)
            .await
    }

    /// Get the current session's identifier, if there is one.
    fn get_identifier(&self) -> Option<T::Id> {
        self.get_inner_lock().get_current_identifier().cloned()
    }

    /// Try to cast the storage as an indexed storage
    fn get_indexed_storage(&self) -> Result<&dyn SessionStorageIndexed<T>, SessionError> {
        let indexed_storage = self
            .storage
            .as_indexed_storage()
            .ok_or(SessionError::NonIndexedStorage)?;
        Ok(indexed_storage)
    }
}
