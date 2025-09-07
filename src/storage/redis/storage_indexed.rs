use fred::prelude::{FromValue, HashesInterface, KeysInterface, SetsInterface, Value};
use rocket::http::CookieJar;

use crate::{
    error::{SessionError, SessionResult},
    storage::{SessionStorage, SessionStorageIndexed},
    SessionIdentifier,
};

use super::{RedisFredStorage, RedisFredStorageIndexed};

const DEFAULT_INDEX_TTL: u32 = 60 * 60 * 24 * 7 * 2; // 2 weeks

impl RedisFredStorageIndexed {
    /// Create the indexed storage.
    ///
    /// # Parameters:
    /// - `base_storage`: The base storage to use for session data.
    /// - `index_ttl`: The TTL for the session index - should match
    /// your longest expected session duration (default: 2 weeks).
    pub fn new(base_storage: RedisFredStorage, index_ttl: Option<u32>) -> Self {
        Self {
            base_storage,
            index_ttl: index_ttl.unwrap_or(DEFAULT_INDEX_TTL),
        }
    }

    fn session_index_key(&self, identifier_name: &str, identifier: &impl ToString) -> String {
        format!(
            "{}{identifier_name}:{}",
            self.base_storage.prefix,
            identifier.to_string()
        )
    }
}

#[rocket::async_trait]
impl<T> SessionStorage<T> for RedisFredStorageIndexed
where
    T: SessionIdentifier + FromValue + TryInto<Value> + Clone + Send + Sync + 'static,
    <T as TryInto<Value>>::Error: std::error::Error + Send + Sync + 'static,
    <T as SessionIdentifier>::Id: ToString,
{
    async fn load(
        &self,
        id: &str,
        ttl: Option<u32>,
        _cookie_jar: &CookieJar,
    ) -> SessionResult<(T, u32)> {
        let (value, ttl) = self.base_storage.fetch_session(id, ttl).await?;
        let data = T::from_value(value)?;
        Ok((data, ttl))
    }

    async fn save(&self, id: &str, data: T, ttl: u32) -> SessionResult<()> {
        if let Some(identifier) = data.identifier() {
            let index_key = self.session_index_key(T::IDENTIFIER, identifier);
            let pipeline = self.base_storage.pool.next().pipeline();
            let _: () = pipeline.sadd(&index_key, id).await?;
            let _: () = pipeline
                .expire(&index_key, self.index_ttl.into(), None)
                .await?;
            let _: () = pipeline.all().await?;
        }

        let value: Value = data
            .try_into()
            .map_err(|e| SessionError::Serialization(Box::new(e)))?;
        self.base_storage.save_session(id, value, ttl).await
    }

    async fn delete(&self, id: &str, _cookie_jar: &CookieJar) -> SessionResult<()> {
        let (value, _) = self.base_storage.fetch_session(id, None).await?;
        let data = T::from_value(value)?;

        let pipeline = self.base_storage.pool.next().pipeline();
        let _: () = pipeline.del(self.base_storage.session_key(id)).await?;
        if let Some(identifier) = data.identifier() {
            let session_idx_key = self.session_index_key(T::IDENTIFIER, identifier);
            let _: () = pipeline.srem(&session_idx_key, id).await?;
        }
        Ok(pipeline.all().await?)
    }
}

#[rocket::async_trait]
impl<T> SessionStorageIndexed<T> for RedisFredStorageIndexed
where
    T: SessionIdentifier + FromValue + TryInto<Value> + Clone + Send + Sync + 'static,
    <T as TryInto<Value>>::Error: std::error::Error + Send + Sync + 'static,
    <T as SessionIdentifier>::Id: ToString,
{
    async fn get_sessions_by_identifier(&self, id: &T::Id) -> SessionResult<Vec<(String, T)>> {
        let index_key = self.session_index_key(T::IDENTIFIER, id);
        let session_ids: Vec<String> = self.base_storage.pool.smembers(&index_key).await?;

        let session_value_pipeline = self.base_storage.pool.next().pipeline();
        for session_id in &session_ids {
            let session_key = self.base_storage.session_key(&session_id);
            let _: () = match self.base_storage.redis_type {
                super::RedisType::String => session_value_pipeline.get(&session_key).await?,
                super::RedisType::Hash => session_value_pipeline.hgetall(&session_key).await?,
            };
        }
        let session_values: Vec<Option<Value>> = session_value_pipeline.all().await?;

        let (existing_sessions, stale_sessions): (Vec<_>, Vec<_>) = session_ids
            .into_iter()
            .zip(session_values.into_iter())
            .map(|(id, value)| (id, value.and_then(|v| T::from_value(v).ok())))
            .partition(|(_, data)| data.is_some());
        if !stale_sessions.is_empty() {
            let stale_ids: Vec<_> = stale_sessions.into_iter().map(|(id, _)| id).collect();
            let _: () = self.base_storage.pool.srem(&index_key, stale_ids).await?;
        }

        let sessions = existing_sessions
            .into_iter()
            .map(|(id, data)| (id, data.expect("already checked by partition")))
            .collect();
        Ok(sessions)
    }

    async fn get_session_ids_by_identifier(&self, id: &T::Id) -> SessionResult<Vec<String>> {
        let index_key = self.session_index_key(T::IDENTIFIER, id);
        let session_ids: Vec<String> = self.base_storage.pool.smembers(&index_key).await?;

        let session_exist_pipeline = self.base_storage.pool.next().pipeline();
        for session_id in &session_ids {
            let session_key = self.base_storage.session_key(&session_id);
            let _: () = session_exist_pipeline.exists(&session_key).await?;
        }
        let session_exist_results: Vec<bool> = session_exist_pipeline.all().await?;

        let (existing_sessions, stale_sessions): (Vec<_>, Vec<_>) = session_ids
            .into_iter()
            .zip(session_exist_results.into_iter())
            .partition(|(_, exists)| *exists);
        if !stale_sessions.is_empty() {
            let stale_ids: Vec<_> = stale_sessions.into_iter().map(|(id, _)| id).collect();
            let _: () = self.base_storage.pool.srem(&index_key, stale_ids).await?;
        }

        let sessions = existing_sessions.into_iter().map(|(id, _)| id).collect();
        Ok(sessions)
    }

    async fn invalidate_sessions_by_identifier(
        &self,
        id: &T::Id,
        excluded_session_id: Option<&str>,
    ) -> SessionResult<u64> {
        let index_key = self.session_index_key(T::IDENTIFIER, id);
        let mut session_ids: Vec<String> = self.base_storage.pool.smembers(&index_key).await?;

        if let Some(excluded_id) = excluded_session_id {
            session_ids.retain(|id| id != excluded_id);
        }
        if session_ids.is_empty() {
            return Ok(0);
        }

        let session_keys: Vec<_> = session_ids
            .iter()
            .map(|id| self.base_storage.session_key(id))
            .collect();
        let delete_pipeline = self.base_storage.pool.next().pipeline();
        let _: () = delete_pipeline.del(session_keys).await?;
        let _: () = delete_pipeline.srem(index_key, session_ids).await?;
        let (del_num, _srem_num): (u64, u64) = delete_pipeline.all().await?;

        Ok(del_num)
    }
}
