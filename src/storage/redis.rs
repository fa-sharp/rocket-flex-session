//! Session storage with Redis (and Redis-compatible databases)

use fred::{
    prelude::{FromValue, HashesInterface, KeysInterface, Pool, Value},
    types::Expiration,
};
use rocket::{async_trait, http::CookieJar};

use crate::error::{SessionError, SessionResult};

use super::interface::SessionStorage;

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
        let data = T::from_value(found_value)?;

        Ok((data, ttl.unwrap_or(orig_ttl.try_into().unwrap_or(0))))
    }

    async fn save(&self, id: &str, data: T, ttl: u32) -> SessionResult<()> {
        let key = self.key(id);
        let value: Value = data
            .try_into()
            .map_err(|e| SessionError::Serialization(Box::new(e)))?;
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

    async fn delete(&self, id: &str, _cookie_jar: &CookieJar) -> SessionResult<()> {
        let _: u8 = self.pool.del(self.key(id)).await?;
        Ok(())
    }
}
