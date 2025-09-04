//! Storage implementations for sessions

mod interface;
pub use interface::*;

pub mod memory;

#[cfg(feature = "cookie")]
pub mod cookie;

#[cfg(feature = "redis_fred")]
pub mod redis;

#[cfg(feature = "sqlx_postgres")]
pub mod sqlx;
