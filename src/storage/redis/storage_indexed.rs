use fred::prelude::{FromValue, HashesInterface, KeysInterface, SetsInterface, Value};
use rocket::http::CookieJar;

use crate::{
    error::{SessionError, SessionResult},
    storage::{SessionStorage, SessionStorageIndexed},
    SessionIdentifier,
};

use super::{RedisFredStorage, RedisFredStorageIndexed};

impl RedisFredStorageIndexed {
    pub fn new(base_storage: RedisFredStorage) -> Self {
        Self { base_storage }
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
            let session_idx_key = self
                .base_storage
                .session_index_key(T::IDENTIFIER, identifier);
            let pipeline = self.base_storage.pool.next().pipeline();
            let _: () = pipeline.sadd(&session_idx_key, id).await?;
            let _: () = pipeline.expire(&session_idx_key, ttl.into(), None).await?;
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
        let _: () = pipeline.del(self.base_storage.key(id)).await?;
        if let Some(identifier) = data.identifier() {
            let session_idx_key = self
                .base_storage
                .session_index_key(T::IDENTIFIER, identifier);
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
        let session_index_key = self.base_storage.session_index_key(T::IDENTIFIER, id);
        let session_ids: Vec<String> = self.base_storage.pool.smembers(&session_index_key).await?;

        let session_value_pipeline = self.base_storage.pool.next().pipeline();
        for session_id in &session_ids {
            let session_key = self.base_storage.key(&session_id);
            let _: () = match self.base_storage.redis_type {
                super::RedisType::String => session_value_pipeline.get(&session_key).await?,
                super::RedisType::Hash => session_value_pipeline.hgetall(&session_key).await?,
            };
        }
        let session_values: Vec<Option<Value>> = session_value_pipeline.all().await?;

        let sessions = session_values
            .into_iter()
            .enumerate()
            .filter_map(|(idx, value)| {
                value.and_then(|value| {
                    let session_id = session_ids.get(idx)?.clone();
                    let data = T::from_value(value).ok()?;
                    Some((session_id, data))
                })
            })
            .collect();
        Ok(sessions)
    }

    async fn get_session_ids_by_identifier(&self, id: &T::Id) -> SessionResult<Vec<String>> {
        let session_index_key = self.base_storage.session_index_key(T::IDENTIFIER, id);
        let session_ids: Vec<String> = self.base_storage.pool.smembers(&session_index_key).await?;

        let session_exist_pipeline = self.base_storage.pool.next().pipeline();
        for session_id in &session_ids {
            let session_key = self.base_storage.key(&session_id);
            let _: () = session_exist_pipeline.exists(&session_key).await?;
        }
        let session_exist_results: Vec<bool> = session_exist_pipeline.all().await?;

        let existing_sessions = session_ids
            .into_iter()
            .enumerate()
            .filter_map(|(idx, id)| session_exist_results.get(idx)?.then_some(id))
            .collect();
        Ok(existing_sessions)
    }

    async fn invalidate_sessions_by_identifier(
        &self,
        id: &T::Id,
        excluded_session_id: Option<&str>,
    ) -> SessionResult<u64> {
        let session_index_key = self.base_storage.session_index_key(T::IDENTIFIER, id);
        let mut session_ids: Vec<String> =
            self.base_storage.pool.smembers(&session_index_key).await?;
        if let Some(excluded_id) = excluded_session_id {
            session_ids.retain(|id| id != excluded_id);
        }
        if session_ids.is_empty() {
            return Ok(0);
        }

        let session_keys: Vec<_> = session_ids
            .iter()
            .map(|id| self.base_storage.key(id))
            .collect();
        let delete_pipeline = self.base_storage.pool.next().pipeline();
        let _: () = delete_pipeline.del(session_keys).await?;
        let _: () = delete_pipeline.srem(session_index_key, session_ids).await?;
        let (del_num, _srem_num): (u64, u64) = delete_pipeline.all().await?;

        Ok(del_num)
    }
}
