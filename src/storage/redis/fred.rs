use bon::Builder;
use fred::prelude::{HashesInterface, KeysInterface, SetsInterface, Value};
use rocket::http::CookieJar;

use crate::{
    error::{SessionError, SessionResult},
    storage::{SessionStorage, SessionStorageIndexed},
    SessionIdentifier,
};

use super::{SessionRedis, SessionRedisType, SessionRedisValue};

const TWO_WEEKS_TTL: u32 = 60 * 60 * 24 * 7 * 2;

/// Redis session storage using the [fred.rs](https://docs.rs/fred) crate.
///
/// # Requirements
/// - You must pass in an initialized fred.rs connection pool.
/// - Your session data type must implement [`SessionRedis`] to configure how to convert & store session data.
/// - Your session data type must implement [`SessionIdentifier`]. The
/// SessionIdentifier's [Id](`SessionIdentifier::Id`) type must be a string.
///
/// # Session keys and data
/// Sessions are stored using either Redis strings or hashes, depending on your [`SessionRedis`]
/// implementation. The key will be `<prefix>:<id>` (e.g.: `sess:abcdef...`)
///
/// # Indexing sessions
/// Sessions are indexed with the identifier retrieved from your [`SessionIdentifier`] implementation.
/// Session IDs are grouped together using Redis sets, with a key format of:
///
/// `<index_prefix>:<id>` (e.g.: `sess:user:1`)
///
/// # Example
/// A full Redis example can be found in the crate's examples directory.
#[derive(Builder)]
pub struct RedisFredStorage {
    /// The initialized fred.rs connection pool.
    pool: fred::prelude::Pool,
    /// The prefix to use for session keys.
    #[builder(into, default = "sess:")]
    prefix: String,
    /// The prefix to use for session index keys (e.g. to group sessions by user ID)
    #[builder(into, default = "sess:user:")]
    index_prefix: String,
    /// The TTL in seconds for the session index keys - should match your longest expected session duration (default: 2 weeks).
    #[builder(default = TWO_WEEKS_TTL)]
    index_ttl: u32,
}

impl RedisFredStorage {
    fn session_key(&self, id: &str) -> String {
        format!("{}{id}", self.prefix)
    }

    fn session_index_key(&self, identifier: &str) -> String {
        format!("{}{identifier}", self.index_prefix)
    }

    async fn fetch_session_index(&self, identifier: &str) -> SessionResult<(Vec<String>, String)> {
        let index_key = self.session_index_key(identifier);
        let session_ids = self.pool.smembers(&index_key).await?;
        Ok((session_ids, index_key))
    }

    fn to_typed_value(
        &self,
        redis_type: SessionRedisType,
        value: Value,
    ) -> SessionResult<SessionRedisValue> {
        match redis_type {
            SessionRedisType::String => value.into_string().map(SessionRedisValue::String),
            SessionRedisType::Bytes => value.into_owned_bytes().map(SessionRedisValue::Bytes),
            SessionRedisType::Map => value.convert().ok().map(SessionRedisValue::Map),
        }
        .ok_or(SessionError::InvalidData)
    }

    async fn cleanup_session_index(
        &self,
        index_key: &str,
        stale_ids: Vec<String>,
    ) -> SessionResult<()> {
        Ok(self.pool.srem(index_key, stale_ids).await?)
    }
}

#[rocket::async_trait]
impl<T> SessionStorage<T> for RedisFredStorage
where
    T: SessionRedis,
    <T as SessionIdentifier>::Id: AsRef<str>,
{
    fn as_indexed_storage(&self) -> Option<&dyn SessionStorageIndexed<T>> {
        Some(self)
    }

    async fn load(
        &self,
        id: &str,
        ttl: Option<u32>,
        _cookie_jar: &CookieJar,
    ) -> SessionResult<(T, u32)> {
        let key = self.session_key(id);
        let pipeline = self.pool.next().pipeline();
        let _: () = match T::REDIS_TYPE {
            SessionRedisType::String | SessionRedisType::Bytes => pipeline.get(&key).await?,
            SessionRedisType::Map => pipeline.hgetall(&key).await?,
        };
        let _: () = pipeline.ttl(&key).await?;

        let (value, orig_ttl): (Option<Value>, i64) = match ttl {
            None => pipeline.all().await?,
            Some(new_ttl) => {
                let _: () = pipeline.expire(&key, new_ttl.into(), None).await?;
                let (value, orig_ttl, _expire_result): (Option<Value>, i64, Option<u8>) =
                    pipeline.all().await?;
                (value, orig_ttl)
            }
        };

        let value = value.ok_or(SessionError::NotFound)?;
        let typed_value = self.to_typed_value(T::REDIS_TYPE, value)?;
        let data = T::from_redis(typed_value).map_err(|e| SessionError::Parsing(Box::new(e)))?;

        Ok((data, ttl.unwrap_or(orig_ttl.try_into().unwrap_or(0))))
    }

    async fn save(&self, id: &str, data: T, ttl: u32) -> SessionResult<()> {
        use fred::types::Expiration;

        if let Some(identifier) = data.identifier() {
            let index_key = self.session_index_key(identifier.as_ref());
            let pipeline = self.pool.next().pipeline();
            let _: () = pipeline.sadd(&index_key, id).await?;
            let _: () = pipeline
                .expire(&index_key, self.index_ttl.into(), None)
                .await?;
            let _: () = pipeline.all().await?;
        }

        let key = self.session_key(id);
        let value = data
            .into_redis()
            .map_err(|e| SessionError::Serialization(Box::new(e)))?;
        let _: () = match value {
            SessionRedisValue::String(val) => {
                self.pool
                    .set(&key, val, Some(Expiration::EX(ttl.into())), None, false)
                    .await?
            }
            SessionRedisValue::Bytes(val) => {
                self.pool
                    .set(&key, val, Some(Expiration::EX(ttl.into())), None, false)
                    .await?
            }
            SessionRedisValue::Map(map) => {
                let pipeline = self.pool.next().pipeline();
                let _: () = pipeline.hset(&key, map).await?;
                let _: () = pipeline.expire(&key, ttl.into(), None).await?;
                pipeline.all().await?
            }
        };
        Ok(())
    }

    async fn delete(&self, id: &str, data: T) -> SessionResult<()> {
        let pipeline = self.pool.next().pipeline();
        let _: () = pipeline.del(self.session_key(id)).await?;
        if let Some(identifier) = data.identifier() {
            let session_idx_key = self.session_index_key(identifier.as_ref());
            let _: () = pipeline.srem(&session_idx_key, id).await?;
        }
        Ok(pipeline.all().await?)
    }
}

#[rocket::async_trait]
impl<T> SessionStorageIndexed<T> for RedisFredStorage
where
    T: SessionRedis,
    <T as SessionIdentifier>::Id: AsRef<str>,
{
    async fn get_session_ids_by_identifier(&self, id: &T::Id) -> SessionResult<Vec<String>> {
        let (session_ids, index_key) = self.fetch_session_index(id.as_ref()).await?;

        let session_exist_pipeline = self.pool.next().pipeline();
        for session_id in &session_ids {
            let session_key = self.session_key(&session_id);
            let _: () = session_exist_pipeline.exists(&session_key).await?;
        }
        let session_exist_results: Vec<bool> = session_exist_pipeline.all().await?;

        let (existing_sessions, stale_sessions): (Vec<_>, Vec<_>) = session_ids
            .into_iter()
            .zip(session_exist_results.into_iter())
            .partition(|(_, exists)| *exists);
        if !stale_sessions.is_empty() {
            let stale_ids: Vec<_> = stale_sessions.into_iter().map(|(id, _)| id).collect();
            self.cleanup_session_index(&index_key, stale_ids).await?;
        }

        let sessions = existing_sessions.into_iter().map(|(id, _)| id).collect();
        Ok(sessions)
    }

    async fn get_sessions_by_identifier(&self, id: &T::Id) -> SessionResult<Vec<(String, T, u32)>> {
        let (session_ids, index_key) = self.fetch_session_index(id.as_ref()).await?;

        let session_value_pipeline = self.pool.next().pipeline();
        for session_id in &session_ids {
            let session_key = self.session_key(&session_id);
            let _: () = match T::REDIS_TYPE {
                SessionRedisType::String | SessionRedisType::Bytes => {
                    session_value_pipeline.get(&session_key).await?
                }
                SessionRedisType::Map => session_value_pipeline.hgetall(&session_key).await?,
            };
            let _: () = session_value_pipeline.ttl(&session_key).await?;
        }
        let mut raw_values_and_ttls: Vec<Option<Value>> = session_value_pipeline.all().await?;

        let (existing_sessions, stale_sessions): (Vec<_>, Vec<_>) = session_ids
            .into_iter()
            .zip(raw_values_and_ttls.chunks_exact_mut(2))
            .map(|(id, raw)| {
                let data_and_ttl = raw[0].take().and_then(|val| {
                    let typed_value = self.to_typed_value(T::REDIS_TYPE, val).ok()?;
                    let data = T::from_redis(typed_value).ok()?;
                    let ttl = raw[1].as_ref().and_then(Value::as_i64)?;
                    Some((data, ttl))
                });
                (id, data_and_ttl)
            })
            .partition(|(_, data_and_ttl)| data_and_ttl.is_some());
        if !stale_sessions.is_empty() {
            let stale_ids: Vec<_> = stale_sessions.into_iter().map(|(id, _)| id).collect();
            self.cleanup_session_index(&index_key, stale_ids).await?;
        }

        let sessions = existing_sessions
            .into_iter()
            .map(|(id, data_and_ttl)| {
                let (data, ttl) = data_and_ttl.expect("already checked by partition");
                (id, data, ttl.try_into().unwrap_or(0))
            })
            .collect();
        Ok(sessions)
    }

    async fn invalidate_sessions_by_identifier(
        &self,
        id: &T::Id,
        excluded_session_id: Option<&str>,
    ) -> SessionResult<u64> {
        let (mut session_ids, index_key) = self.fetch_session_index(id.as_ref()).await?;
        if let Some(excluded_id) = excluded_session_id {
            session_ids.retain(|id| id != excluded_id);
        }
        if session_ids.is_empty() {
            return Ok(0);
        }

        let session_keys: Vec<_> = session_ids.iter().map(|id| self.session_key(id)).collect();
        let delete_pipeline = self.pool.next().pipeline();
        let _: () = delete_pipeline.del(session_keys).await?;
        let _: () = delete_pipeline.srem(index_key, session_ids).await?;
        let (del_num, _srem_num): (u64, u64) = delete_pipeline.all().await?;

        Ok(del_num)
    }
}
