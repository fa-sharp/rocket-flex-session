use fred::{
    prelude::{HashesInterface, KeysInterface, Pool, Value},
    types::Expiration,
};

use crate::error::{SessionError, SessionResult};

use super::{RedisFredStorage, RedisType};

impl RedisFredStorage {
    /// Create the storage instance.
    /// # Parameters
    /// * `pool` - The initialized fred.rs connection pool.
    /// * `redis_type` - The Redis data type to use for storing sessions.
    /// * `key_prefix` - The prefix to use for session keys. (e.g. "sess:")
    pub fn new(pool: Pool, redis_type: RedisType, key_prefix: &str) -> Self {
        Self {
            pool,
            prefix: key_prefix.to_owned(),
            redis_type,
        }
    }

    pub(super) fn session_key(&self, id: &str) -> String {
        format!("{}{id}", self.prefix)
    }

    pub(super) fn session_index_key(
        &self,
        identifier_name: &str,
        identifier: &impl ToString,
    ) -> String {
        format!(
            "{}{identifier_name}:{}",
            self.prefix,
            identifier.to_string()
        )
    }

    pub(super) async fn fetch_session(
        &self,
        id: &str,
        ttl: Option<u32>,
    ) -> SessionResult<(Value, u32)> {
        let key = self.session_key(id);
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

    pub(super) async fn save_session(&self, id: &str, value: Value, ttl: u32) -> SessionResult<()> {
        let key = self.session_key(id);
        let _: () = match self.redis_type {
            RedisType::String => {
                self.pool
                    .set(&key, value, Some(Expiration::EX(ttl.into())), None, false)
                    .await?
            }
            RedisType::Hash => {
                let pipeline = self.pool.next().pipeline();
                let _: () = pipeline.hset(&key, value.into_map()?).await?;
                let _: () = pipeline.expire(&key, ttl.into(), None).await?;
                pipeline.all().await?
            }
        };
        Ok(())
    }
}
