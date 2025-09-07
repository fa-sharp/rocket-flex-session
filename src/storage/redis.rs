//! Session storage with Redis (and Redis-compatible databases)

mod base;
mod storage;
mod storage_indexed;

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
    pool: fred::prelude::Pool,
    prefix: String,
    redis_type: RedisType,
}

/// Redis session storage using the [fred.rs](https://docs.rs/fred) crate. This is a wrapper around
/// [`RedisFredStorage`] that adds support for indexing sessions by an identifier (e.g. `user_id`).
///
/// In addition to the requirements for `RedisFredStorage`, your session data type must
/// implement [`SessionIdentifier`], and its [Id](`SessionIdentifier::Id`) type
/// must implement `ToString`. Sessions are tracked in Redis sets, with a key format of
/// `<key_prefix><identifier_name>:<id>`. e.g.: `sess:user_id:1`
pub struct RedisFredStorageIndexed {
    base_storage: RedisFredStorage,
    index_ttl: u32,
}
