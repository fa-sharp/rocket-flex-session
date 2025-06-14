//! In-memory session storage implementation

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use retainer::Cache;
use rocket::{
    async_trait,
    http::CookieJar,
    tokio::{select, spawn, sync::oneshot},
};

use super::interface::{SessionError, SessionResult, SessionStorage};

/// In-memory storage provider for sessions. This is designed mostly for local
/// development, and not for production use. It uses the [retainer] crate to
/// create an async cache.
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
        Ok((
            data.to_owned(),
            ttl.unwrap_or(data.expiration().remaining().unwrap().as_secs() as u32),
        ))
    }

    async fn save(&self, id: &str, data: T, ttl: u32) -> SessionResult<()> {
        self.cache
            .insert(id.to_owned(), data, Duration::from_secs(ttl.into()))
            .await;
        Ok(())
    }

    async fn delete(&self, id: &str) -> SessionResult<()> {
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
