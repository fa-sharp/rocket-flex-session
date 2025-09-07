//! Session storage with Redis (and Redis-compatible databases)

mod base;
mod storage;
mod storage_indexed;

pub use base::{RedisFredStorage, RedisType};
pub use storage_indexed::RedisFredStorageIndexed;
