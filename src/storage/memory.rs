//! In-memory session storage implementation

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Duration,
};

use retainer::Cache;
use rocket::{
    async_trait,
    http::CookieJar,
    tokio::{select, spawn, sync::oneshot},
};

use crate::{
    error::{SessionError, SessionResult},
    SessionIdentifier,
};

use super::interface::{SessionStorage, SessionStorageIndexed};

/// In-memory storage provider for sessions. This is designed mostly for local
/// development, and not for production use. It uses the [retainer] crate to
/// create an async cache.
///
/// For session indexing support, see [`MemoryStorageIndexed`].
pub struct MemoryStorage<T> {
    shutdown_tx: Mutex<Option<oneshot::Sender<()>>>,
    cache: Arc<Cache<String, T>>,
}

impl<T> Default for MemoryStorage<T> {
    fn default() -> Self {
        Self {
            shutdown_tx: Mutex::default(),
            cache: Default::default(),
        }
    }
}

#[async_trait]
impl<T> SessionStorage<T> for MemoryStorage<T>
where
    T: Clone + Send + Sync + 'static,
{
    async fn load(
        &self,
        id: &str,
        ttl: Option<u32>,
        _cookie_jar: &CookieJar,
    ) -> SessionResult<(T, u32)> {
        let Some(data) = self.cache.get(&id.to_owned()).await else {
            return Err(SessionError::NotFound);
        };
        if let Some(new_ttl) = ttl {
            self.cache
                .insert(
                    id.to_owned(),
                    data.to_owned(),
                    Duration::from_secs(new_ttl.into()),
                )
                .await;
        }
        let ttl = ttl.unwrap_or(data.expiration().remaining().unwrap().as_secs() as u32);
        Ok((data.to_owned(), ttl))
    }

    async fn save(&self, id: &str, data: T, ttl: u32) -> SessionResult<()> {
        self.cache
            .insert(id.to_owned(), data, Duration::from_secs(ttl.into()))
            .await;
        Ok(())
    }

    async fn delete(&self, id: &str, _cookie_jar: &CookieJar) -> SessionResult<()> {
        self.cache.remove(&id.to_owned()).await;
        Ok(())
    }

    async fn setup(&self) -> SessionResult<()> {
        let cache = self.cache.clone();
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        spawn(async move {
            select! {
                _ = cache.monitor(10, 0.25, Duration::from_secs(5 * 60)) => (),
                _ = shutdown_rx => {
                    rocket::debug!("Session cache monitor shutdown");
                }
            }
        });
        self.shutdown_tx.lock().unwrap().replace(shutdown_tx);
        Ok(())
    }

    async fn shutdown(&self) -> SessionResult<()> {
        if let Some(tx) = self.shutdown_tx.lock().unwrap().take() {
            let _ = tx.send(());
        }
        Ok(())
    }
}

impl<T> MemoryStorage<T> {
    /// Get access to the underlying cache for indexed operations
    pub(crate) fn cache(&self) -> &Cache<String, T> {
        &self.cache
    }
}

/// Extended in-memory storage that supports session indexing by identifier.
/// This allows for operations like retrieving all sessions for a user or
/// bulk invalidation of sessions.
///
/// You must implement the [`SessionIdentifier`] trait for your session type,
/// and the [`SessionIdentifier::Id`] type must implement [`ToString`].
///
/// # Example
/// ```rust
/// use rocket_flex_session::storage::memory::MemoryStorageIndexed;
/// use rocket_flex_session::{SessionIdentifier, RocketFlexSession};
///
/// #[derive(Clone)]
/// struct UserSession {
///     user_id: String,
///     data: String,
/// }
///
/// impl SessionIdentifier for UserSession {
///     type Id = String;
///     fn identifier(&self) -> Option<&Self::Id> {
///         Some(&self.user_id)
///     }
/// }
///
/// let storage = MemoryStorageIndexed::<UserSession>::default();
/// let fairing = RocketFlexSession::builder()
///     .storage(storage)
///     .build();
/// ```
pub struct MemoryStorageIndexed<T>
where
    T: SessionIdentifier,
{
    base_storage: MemoryStorage<T>,
    // Index from identifier to set of session IDs
    identifier_index: Arc<Mutex<HashMap<String, HashSet<String>>>>,
}

impl<T> Default for MemoryStorageIndexed<T>
where
    T: SessionIdentifier,
    <T as SessionIdentifier>::Id: ToString,
{
    fn default() -> Self {
        Self {
            base_storage: MemoryStorage::default(),
            identifier_index: Arc::default(),
        }
    }
}

impl<T> MemoryStorageIndexed<T>
where
    T: SessionIdentifier,
    T::Id: ToString,
{
    /// Update the identifier index when session data is saved
    fn update_identifier_index(&self, session_id: &str, data: &T) {
        if let Some(id) = data.identifier() {
            let mut index = self.identifier_index.lock().unwrap();
            index
                .entry(id.to_string())
                .or_insert_with(HashSet::new)
                .insert(session_id.to_owned());
        }
    }

    /// Remove from identifier index when session is deleted
    fn remove_from_identifier_index(&self, session_id: &str, data: &T) {
        if let Some(id) = data.identifier() {
            let mut index = self.identifier_index.lock().unwrap();
            let key = id.to_string();
            if let Some(session_ids) = index.get_mut(&key) {
                session_ids.remove(session_id);
                if session_ids.is_empty() {
                    index.remove(&key);
                }
            }
        }
    }
}

#[async_trait]
impl<T> SessionStorage<T> for MemoryStorageIndexed<T>
where
    T: SessionIdentifier + Clone + Send + Sync + 'static,
    T::Id: ToString,
{
    async fn load(
        &self,
        id: &str,
        ttl: Option<u32>,
        cookie_jar: &CookieJar,
    ) -> SessionResult<(T, u32)> {
        self.base_storage.load(id, ttl, cookie_jar).await
    }

    async fn save(&self, id: &str, data: T, ttl: u32) -> SessionResult<()> {
        // Update identifier index before saving
        self.update_identifier_index(id, &data);

        // Save using base storage
        self.base_storage.save(id, data, ttl).await
    }

    async fn delete(&self, id: &str, cookie_jar: &CookieJar) -> SessionResult<()> {
        // Get the data first so we can update the index
        if let Ok((data, _)) = self.base_storage.load(id, None, cookie_jar).await {
            self.remove_from_identifier_index(id, &data);
        }

        // Delete using base storage
        self.base_storage.delete(id, cookie_jar).await
    }

    fn as_indexed_storage(&self) -> Option<&dyn SessionStorageIndexed<T>> {
        Some(self)
    }

    async fn setup(&self) -> SessionResult<()> {
        self.base_storage.setup().await
    }

    async fn shutdown(&self) -> SessionResult<()> {
        self.base_storage.shutdown().await
    }
}

#[async_trait]
impl<T> SessionStorageIndexed<T> for MemoryStorageIndexed<T>
where
    Self: SessionStorage<T>,
    T: SessionIdentifier + Clone + Send + Sync,
    T::Id: ToString,
{
    async fn get_sessions_by_identifier(&self, id: &T::Id) -> SessionResult<Vec<(String, T)>> {
        let session_ids = {
            let index = self.identifier_index.lock().unwrap();
            index.get(&id.to_string()).cloned().unwrap_or_default()
        };

        let mut sessions: Vec<(String, T)> = Vec::new();
        for session_id in session_ids {
            if let Some(data) = self.base_storage.cache().get(&session_id).await {
                sessions.push((session_id, data.value().to_owned()));
            }
        }

        Ok(sessions)
    }

    async fn get_session_ids_by_identifier(&self, id: &T::Id) -> SessionResult<Vec<String>> {
        let id_str = id.to_string();
        let session_ids = {
            let index = self.identifier_index.lock().unwrap();
            index.get(&id_str).cloned().unwrap_or_default()
        };

        Ok(session_ids.into_iter().collect())
    }

    async fn invalidate_sessions_by_identifier(&self, id: &T::Id) -> SessionResult<u64> {
        let id_str = id.to_string();
        let session_ids = {
            let mut index = self.identifier_index.lock().unwrap();
            index.remove(&id_str).unwrap_or_default()
        };

        // Remove all sessions from cache
        for session_id in &session_ids {
            self.base_storage.cache().remove(session_id).await;
        }

        Ok(session_ids.len() as u64)
    }
}
