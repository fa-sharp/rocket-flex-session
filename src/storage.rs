//! Storage implementations for sessions
//!
//! This module provides various storage backends for session data, with optional
//! support for session indexing by identifier.
//!
//! ## Session Indexing
//!
//! Some storage backends support indexing sessions by an identifier (like user ID).
//! This enables advanced features such as:
//!
//! - Finding all active sessions for a user
//! - Bulk invalidation of sessions (e.g., "log out everywhere")
//! - Security auditing and monitoring
//!
//! To use indexing, your session type must implement [`crate::SessionIdentifier`] and you
//! must use a storage backend that implements [`SessionStorageIndexed`].
//!
//! ## Custom Storage
//!
//! Implement [`SessionStorage`] to create custom storage backends. For indexing
//! support, also implement [`SessionStorageIndexed`].

mod interface;
pub use interface::*;

pub mod memory;

#[cfg(feature = "cookie")]
pub mod cookie;

#[cfg(feature = "redis_fred")]
pub mod redis;

#[cfg(feature = "sqlx_postgres")]
pub mod sqlx;
