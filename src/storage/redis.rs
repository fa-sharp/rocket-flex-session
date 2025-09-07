//! Session storage with Redis (and Redis-compatible databases)

use fred::{
    prelude::{FromValue, HashesInterface, KeysInterface, Pool, SetsInterface, Value},
    types::Expiration,
};
use rocket::{async_trait, http::CookieJar};

use crate::{
    error::{SessionError, SessionResult},
    storage::SessionStorageIndexed,
    SessionIdentifier,
};

use super::interface::SessionStorage;

/// The Redis type to use for the session data
#[derive(Debug)]
pub enum RedisType {
    String,
    Hash,
}

/**
Redis session storage using the [fred.rs](https://docs.rs/fred) crate.

You can store the data as a Redis string or hash. Your session data type must implement [`FromValue`](https://docs.rs/fred/latest/fred/types/trait.FromValue.html)
from the fred.rs crate, as well as the inverse `From<MyData>` or `TryFrom<MyData>` for [`Value`](https://docs.rs/fred/latest/fred/types/enum.Value.html) in order
to dictate how the data will be converted to/from the Redis data type.
- For Redis string types, convert to/from `Value::String`
- For Redis hash types, convert to/from `Value::Map`

💡 Common hashmap types like `HashMap<String, String>` are automatically supported - make sure to use `RedisType::Hash`
when constructing the storage to ensure they are properly converted and stored as Redis hashes.

```rust
use fred::prelude::{Builder, ClientLike, Config, FromValue, Value};
use rocket_flex_session::{error::SessionError, storage::{redis::{RedisFredStorage, RedisType}}};

async fn setup_storage() -> RedisFredStorage {
    // Setup and initialize a fred.rs Redis pool.
    let redis_pool = Builder::default_centralized()
        .set_config(Config::from_url("redis://localhost").expect("Valid Redis URL"))
        .build_pool(4)
        .expect("Should build Redis pool");
    redis_pool.init().await.expect("Should initialize Redis pool");

    // Construct the storage
    let storage = RedisFredStorage::new(
        redis_pool,
        RedisType::String,  // or RedisType::Hash
        "sess:" // Prefix for Redis keys
    );

    storage
}

// If using a custom struct for your session data, implement the following...
struct MySessionData {
    user_id: String,
}
// Implement `FromValue` to convert from the Redis value to your session data type
impl FromValue for MySessionData {
    fn from_value(value: Value) -> Result<Self, fred::error::Error> {
        let data: String = value.convert()?; // fred.rs provides several conversion methods on the Value type
        Ok(MySessionData {
            user_id: data,
        })
    }
}
// Implement the inverse conversion
impl From<MySessionData> for Value {
    fn from(data: MySessionData) -> Self {
        Value::String(data.user_id.into())
    }
}
```
*/
pub struct RedisFredStorage {
    pool: Pool,
    prefix: String,
    redis_type: RedisType,
}

impl RedisFredStorage {
    pub fn new(pool: Pool, redis_type: RedisType, key_prefix: &str) -> Self {
        Self {
            pool,
            prefix: key_prefix.to_owned(),
            redis_type,
        }
    }

    fn key(&self, id: &str) -> String {
        format!("{}{id}", self.prefix)
    }

    fn session_index_key(&self, identifier_name: &str, identifier: &impl ToString) -> String {
        format!(
            "{}{identifier_name}:{}",
            self.prefix,
            identifier.to_string()
        )
    }

    async fn fetch_session(&self, id: &str, ttl: Option<u32>) -> SessionResult<(Value, u32)> {
        let key = self.key(id);
        let pipeline = self.pool.next().pipeline();
        let _: () = match self.redis_type {
            RedisType::String => pipeline.get(&key).await?,
            RedisType::Hash => pipeline.hgetall(&key).await?,
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

        let found_value = value.ok_or(SessionError::NotFound)?;
        Ok((found_value, ttl.unwrap_or(orig_ttl.try_into().unwrap_or(0))))
    }

    async fn save_session(&self, id: &str, value: Value, ttl: u32) -> SessionResult<()> {
        let key = self.key(id);
        let _: () = match self.redis_type {
            RedisType::String => {
                self.pool
                    .set(&key, value, Some(Expiration::EX(ttl.into())), None, false)
                    .await?
            }
            RedisType::Hash => {
                let Value::Map(map) = value else {
                    return Err(SessionError::Serialization(Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("Converted Redis value wasn't a Map: {:?}", value),
                    ))));
                };
                let pipeline = self.pool.next().pipeline();
                let _: () = pipeline.hset(&key, map).await?;
                let _: () = pipeline.expire(&key, ttl.into(), None).await?;
                pipeline.all().await?
            }
        };
        Ok(())
    }
}

#[async_trait]
impl<T> SessionStorage<T> for RedisFredStorage
where
    T: FromValue + TryInto<Value> + Clone + Send + Sync + 'static,
    <T as TryInto<Value>>::Error: std::error::Error + Send + Sync + 'static,
{
    async fn load(
        &self,
        id: &str,
        ttl: Option<u32>,
        _cookie_jar: &CookieJar,
    ) -> SessionResult<(T, u32)> {
        let (value, ttl) = self.fetch_session(id, ttl).await?;
        let data = T::from_value(value)?;
        Ok((data, ttl))
    }

    async fn save(&self, id: &str, data: T, ttl: u32) -> SessionResult<()> {
        let value: Value = data
            .try_into()
            .map_err(|e| SessionError::Serialization(Box::new(e)))?;
        self.save_session(id, value, ttl).await?;
        Ok(())
    }

    async fn delete(&self, id: &str, _cookie_jar: &CookieJar) -> SessionResult<()> {
        let _: u8 = self.pool.del(self.key(id)).await?;
        Ok(())
    }
}

/// Redis session storage using the [fred.rs](https://docs.rs/fred) crate. This is a wrapper around
/// [`RedisFredStorage`] that adds support for indexing sessions by an identifier (e.g. `user_id`).
///
/// In addition to the requirements for [`RedisFredStorage`], your session data type must
/// implement [`SessionIdentifier`], and its [Id](`SessionIdentifier::Id`) type
/// must implement [`ToString`]. Sessions are tracked in Redis sets, with a key format of
/// `<key_prefix><identifier_name>:<id>`. e.g.: `sess:user_id:1`
pub struct RedisFredStorageIndexed {
    base_storage: RedisFredStorage,
}

impl RedisFredStorageIndexed {
    pub fn new(base_storage: RedisFredStorage) -> Self {
        Self { base_storage }
    }
}

#[async_trait]
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

#[async_trait]
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
                RedisType::String => session_value_pipeline.get(&session_key).await?,
                RedisType::Hash => session_value_pipeline.hgetall(&session_key).await?,
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
