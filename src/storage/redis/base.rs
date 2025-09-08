use bon::Builder;
use fred::{
    prelude::{HashesInterface, KeysInterface, Value},
    types::Expiration,
};

use crate::error::{SessionError, SessionResult};

/// The Redis type to use for the session data
#[derive(Debug)]
pub enum RedisType {
    String,
    Hash,
}

/**
Redis session storage using the [fred.rs](https://docs.rs/fred) crate.

# Requirements
You can store the data as a Redis string or hash. Your session data type must implement [`FromValue`](https://docs.rs/fred/latest/fred/types/trait.FromValue.html)
from the fred.rs crate, as well as the inverse `From<MyData>` or `TryFrom<MyData>` for [`Value`](https://docs.rs/fred/latest/fred/types/enum.Value.html) in order
to dictate how the data will be converted to/from the Redis data type.
- For Redis string types, convert to/from `Value::String`
- For Redis hash types, convert to/from `Value::Map`

# Setup
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
    let storage = RedisFredStorage::builder()
        .pool(redis_pool)
        .prefix("sess:") // Prefix for Redis keys
        .redis_type(RedisType::String) // or RedisType::Hash
        .build();

    storage
}

// Implement the following for your session data type...
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
#[derive(Builder)]
pub struct RedisFredStorage {
    /// The initialized fred.rs connection pool.
    pub(super) pool: fred::prelude::Pool,
    /// The prefix to use for session keys.
    #[builder(into, default = "sess:")]
    pub(super) prefix: String,
    /// The Redis data type to use for storing sessions.
    pub(super) redis_type: RedisType,
}

impl RedisFredStorage {
    pub(super) fn session_key(&self, id: &str) -> String {
        format!("{}{id}", self.prefix)
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
