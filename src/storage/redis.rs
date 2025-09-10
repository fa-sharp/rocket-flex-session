//! Session storage with Redis (and Redis-compatible databases)

#[cfg(feature = "redis_fred")]
mod fred;
#[cfg(feature = "redis_fred")]
pub use fred::RedisFredStorage;

use crate::SessionIdentifier;

/// The Redis type used to store the session.
pub enum SessionRedisType {
    String,
    Bytes,
    Map,
}

/// The data saved to or retrieved from Redis.
#[derive(Debug)]
pub enum SessionRedisValue {
    String(String),
    Bytes(Vec<u8>),
    Map(Vec<(String, String)>),
}

/**
Trait for session data types to enable storage in Redis.
# Example

```
use rocket_flex_session::{
    error::SessionError,
    storage::redis::{SessionRedis, SessionRedisType, SessionRedisValue},
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
    const REDIS_TYPE: SessionRedisType = SessionRedisType::Map;

    type Error = SessionError; // You can also use a custom error type here

    fn into_redis(self) -> Result<SessionRedisValue, Self::Error> {
        Ok(SessionRedisValue::Map(vec![
            ("user_id".to_string(), self.user_id),
            ("data".to_string(), self.data),
        ]))
    }

    fn from_redis(value: SessionRedisValue) -> Result<Self, Self::Error> {
        let mut user_id = None;
        let mut data = None;

        match value {
            SessionRedisValue::Map(map) => {
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
    /// The Redis data type used to store the session.
    const REDIS_TYPE: SessionRedisType;

    /// The error that can occur when converting to/from the Redis value.
    type Error: std::error::Error + Send + Sync;

    /// Convert this session into a Redis value.
    fn into_redis(self) -> Result<SessionRedisValue, Self::Error>;

    /// Convert a Redis value into the session data type.
    fn from_redis(value: SessionRedisValue) -> Result<Self, Self::Error>;
}
