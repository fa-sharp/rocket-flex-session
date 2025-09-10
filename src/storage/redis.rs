//! Session storage with Redis (and Redis-compatible databases)

#[cfg(feature = "redis_fred")]
mod fred;
#[cfg(feature = "redis_fred")]
pub use fred::RedisFredStorage;

use crate::SessionIdentifier;

/// The format used to store the session in Redis.
pub enum RedisFormat {
    String,
    Bytes,
    Map,
}

/// The raw data value saved to or retrieved from Redis.
#[derive(Debug)]
pub enum RedisValue {
    String(String),
    Bytes(Vec<u8>),
    Map(Vec<(String, String)>),
}
impl RedisValue {
    pub fn into_string(self) -> Result<String, Self> {
        match self {
            RedisValue::String(s) => Ok(s),
            _ => Err(self),
        }
    }
    pub fn into_bytes(self) -> Result<Vec<u8>, Self> {
        match self {
            RedisValue::Bytes(b) => Ok(b),
            _ => Err(self),
        }
    }
    pub fn into_map(self) -> Result<Vec<(String, String)>, Self> {
        match self {
            RedisValue::Map(map) => Ok(map),
            _ => Err(self),
        }
    }
}

/**
Trait for session data types to enable storage in Redis.
# Example

```
use rocket_flex_session::{
    error::SessionError,
    storage::redis::{SessionRedis, RedisFormat, RedisValue},
    SessionIdentifier,
};

#[derive(Debug, Clone)]
struct MySession {
    user_id: String,
    data: String,
}

// Implement SessionIdentifier to define how to group/index sessions
impl SessionIdentifier for MySession {
    type Id = String; // must be String for Redis storage

    fn identifier(&self) -> Option<Self::Id> {
        Some(self.user_id.clone())
    }
}

impl SessionRedis for MySession {
    const REDIS_FORMAT: RedisFormat = RedisFormat::Map;

    type Error = SessionError; // or a custom error type

    fn into_redis(self) -> Result<RedisValue, Self::Error> {
        Ok(RedisValue::Map(vec![
            ("user_id".to_string(), self.user_id),
            ("data".to_string(), self.data),
        ]))
    }

    fn from_redis(value: RedisValue) -> Result<Self, Self::Error> {
        let mut user_id = None;
        let mut data = None;

        // The `value` should always be the type you specify in REDIS_FORMAT,
        // so you can safely unwrap/expect it.
        let map = value.into_map().expect("should be a map");
        for (key, value) in map {
            match key.as_str() {
                "user_id" => user_id = Some(value),
                "data" => data = Some(value),
                _ => {}
            }
        }
        match (user_id, data) {
            (Some(user_id), Some(data)) => Ok(Self { user_id, data }),
            _ => Err(SessionError::InvalidData),
        }
    }
}
```
*/
pub trait SessionRedis
where
    Self: SessionIdentifier + 'static,
    <Self as SessionIdentifier>::Id: AsRef<str>,
{
    /// The format used to store the session in Redis.
    const REDIS_FORMAT: RedisFormat;

    /// The error that can occur when converting to/from the Redis value.
    type Error: std::error::Error + Send + Sync;

    /// Convert this session into a Redis value.
    fn into_redis(self) -> Result<RedisValue, Self::Error>;

    /// Convert a Redis value into the session data type.
    fn from_redis(value: RedisValue) -> Result<Self, Self::Error>;
}
