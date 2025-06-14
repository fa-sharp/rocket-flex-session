//! Session storage with Redis (and Redis-compatible databases)

use fred::{
    prelude::{ClientLike, HashesInterface, KeysInterface, Pool, Value},
    types::Expiration,
};
use rocket::{async_trait, http::CookieJar};

use super::interface::{SessionError, SessionResult, SessionStorage};

#[derive(Debug)]
pub enum RedisType {
    String,
    Hash,
}

/**
Session storage with Redis (and Redis-compatible databases) using the [fred.rs](https://docs.rs/fred) crate.
You can store the data as a Redis string or hash. Your session data type must implement `TryFrom<Value>`
using the fred.rs [Value](https://docs.rs/fred/latest/fred/types/enum.Value.html) type, as well as the
inverse `TryFrom<MyData> for Value`, in order to dictate how the data will be converted to/from the Redis data type.
- For `RedisType::String`, convert to/from `Value::String`
- For `RedisType::Hash`, convert to/from `Value::Map`

```rust
use fred::prelude::{Builder, Config, Value};
use rocket_flex_session::storage::redis::{RedisFredStorage, RedisType};

fn setup_storage() -> RedisFredStorage {
    // Setup a fred.rs Redis pool. Don't initialize the pool - initialization and closing will be handled automatically
    let redis_pool = Builder::default_centralized()
        .set_config(Config::from_url("redis://localhost").expect("Valid Redis URL"))
        .build_pool(4)
        .expect("Should build Redis pool");
    let storage = RedisFredStorage::new(
        redis_pool,
        RedisType::String,  // or RedisType::Hash
        "sess:" // Prefix for Redis keys
    );

    storage
}

struct MySessionData {
    user_id: String,
}
// TryFrom<Value> to convert from the Redis value to your session data type
impl TryFrom<Value> for MySessionData {
    type Error = &'static str;
    fn try_from(value: Value) -> Result<Self, Self::Error> {
        match value {
            Value::String(id) => Ok(MySessionData {
                user_id: id.to_string(),
            }),
            _ => Err("Redis value had incorrect type"),
        }
    }
}
// ... and you'll need a `TryFrom<MySessionData> for Value` implementation too
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
    T: TryFrom<Value> + TryInto<Value> + Clone + Send + Sync + 'static,
    <T as TryFrom<Value>>::Error: std::error::Error + Send + Sync + 'static,
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
                let (value, orig_ttl, _expire_res): (Option<Value>, i64, Option<u8>) =
                    pipeline.all().await?;
                (value, orig_ttl)
            }
        };

        let found_value = value.ok_or(SessionError::NotFound)?;
        let data =
            T::try_from(found_value).map_err(|e| SessionError::Serialization(Box::new(e)))?;

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

    async fn delete(&self, id: &str) -> SessionResult<()> {
        let _: u8 = self.pool.del(self.key(id)).await?;
        Ok(())
    }

    async fn setup(&self) -> SessionResult<()> {
        rocket::debug!("Initializing fred.rs Redis pool...");
        self.pool.init().await?;
        rocket::info!("Initilaized fred.rs Redis pool");
        Ok(())
    }

    async fn shutdown(&self) -> SessionResult<()> {
        rocket::debug!("Shutting down fred.rs Redis pool...");
        self.pool.quit().await?;
        rocket::info!("Shut down fred.rs Redis pool");
        Ok(())
    }
}
