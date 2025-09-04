#![deny(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_auto_cfg))]

/*!
# Overview
Simple, extensible session library for Rocket applications.

- Session cookies are securely stored and encrypted using Rocket's
  built-in [private cookies](https://rocket.rs/guide/v0.5/requests/#private-cookies)
- Session guard can be used multiple times during a request, enabling various layers
  of authentication and authorization through Rocket's [request guard](https://rocket.rs/guide/v0.5/requests/#custom-guards)
  system.
- Makes use of Rocket's request-local cache to ensure that only one backend
  call will be made to get the session data, and if the session is updated multiple times
  during the request, only one call will be made at the end of the request to save the session.
- Multiple storage providers available, or you can
  use your own session storage by implementing the (`SessionStorage`)[crate::storage::SessionStorage] trait.
- Optional session indexing support for advanced features like multi-device login tracking,
  bulk session invalidation, and security auditing.

# Usage
While technically not needed for development, it is highly recommended to
[set the secret key](https://rocket.rs/guide/v0.5/requests/#secret-key) in Rocket. That way the sessions
will stay valid after reloading your code if you're using a persistent storage provider. The secret key is
_required_ for release mode.

## Basic setup

```rust
use rocket::routes;
use rocket_flex_session::{Session, RocketFlexSession};

// Create a session data type (this type must be thread-safe and Clone)
#[derive(Clone)]
struct MySession {
    user_id: String,
    // ..other session fields
}

#[rocket::launch]
fn rocket() -> _ {
    rocket::build()
        // attach the `RocketFlexSession` fairing, passing in your session data type
        .attach(RocketFlexSession::<MySession>::default())
        .mount("/", routes![login])
}

// use the `Session` request guard in a route handler
#[rocket::post("/login")]
fn login(mut session: Session<MySession>) {
    session.set(MySession { user_id: "123".to_owned() });
}

```

## Request guard auth

If a valid session isn't found, the [Session] request guard will still succeed, but calling [Session.get()](Session#method.get)
or [Session.tap()](Session#method.tap) will yield `None` - indicating an empty/uninitialized session.
This primitive is designed for you to be able to add your authentication and authorization layer on
top of it using Rocket's flexible request guard system.

For example, we can write a request guard for our `MySession` type, that will attempt to retrieve the
session data and verify whether there is an active session:
```
use rocket::{
    http::Status,
    request::{FromRequest, Outcome},
    Request,
};
use rocket_flex_session::Session;

#[derive(Clone)]
struct MySession {
    user_id: String,
    // ..other session fields
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for MySession {
   type Error = &'r str; // or your custom error type

   async fn from_request(req: &'r Request<'_>) -> Outcome<Self, Self::Error> {
       // Run the Session request guard (this guard should always succeed)
       let session = req.guard::<Session<MySession>>().await.expect("should not fail");

       // Get the `MySession` session data, or if it's `None`, send an Unauthorized error
       match session.get() {
           Some(my_session) => Outcome::Success(my_session),
           None => Outcome::Error((Status::Unauthorized, "Not logged in")),
       }
    }
 }

 // Use our new `MySession` request guard in a route handler
 #[rocket::get("/user")]
 fn get_user(session: MySession) -> String {
    return format!("Logged in as user {}!", session.user_id);
 }
```

For more info and examples of this powerful pattern, please see Rocket's documentation on
[request guards](https://api.rocket.rs/v0.5/rocket/request/trait.FromRequest).

## HashMap session data

Instead of a custom struct, you can use a [HashMap](std::collections::HashMap) as your Session data type. This is
particularly useful if you expect your session data structure to be inconsistent and/or change frequently.
When using a HashMap, there are [some additional helper functions](file:///Users/farshad/Projects/pg-user-manager/api/target/doc/rocket_flex_session/struct.Session.html#method.get_key)
to read and set keys.

```
use rocket_flex_session::Session;
use std::collections::HashMap;

type MySessionData = HashMap<String, String>;

#[rocket::post("/login")]
fn login(mut session: Session<MySessionData>) {
    let user_id: Option<String> = session.get_key("user_id");
    session.set_key("name".to_owned(), "Bob".to_owned());
}
```

## Session Indexing

For use cases like multi-device login tracking or other security features, you can use a storage
provider that supports indexing, and then group sessions by an identifier (such as a user ID) using the [`SessionIdentifier`] trait:

```rust
use rocket::routes;
use rocket_flex_session::{Session, SessionIdentifier, RocketFlexSession};
use rocket_flex_session::storage::memory::IndexedMemoryStorage;

#[derive(Clone)]
struct UserSession {
    user_id: String,
    device_name: String,
}

impl SessionIdentifier for UserSession {
    type Id = String;

    fn identifier(&self) -> Option<&Self::Id> {
        Some(&self.user_id) // Group sessions by user_id
    }
}

#[rocket::get("/user/sessions")]
async fn get_all_user_sessions(session: Session<'_, UserSession>) -> String {
    match session.get_all_sessions().await {
        Ok(Some(sessions)) => format!("Found {} active sessions", sessions.len()),
        Ok(None) => "No active session".to_string(),
        Err(e) => format!("Error: {}", e),
    }
}

#[rocket::get("/user/logout-everywhere")]
async fn logout_everywhere(session: Session<'_, UserSession>) -> String {
    match session.invalidate_all_sessions().await {
        Ok(Some(())) => "Logged out from all devices".to_string(),
        Ok(None) => "No active session".to_string(),
        Err(e) => format!("Error: {}", e),
    }
}

#[rocket::launch]
fn rocket() -> _ {
    rocket::build()
        .attach(
            RocketFlexSession::<UserSession>::builder()
                .storage(IndexedMemoryStorage::default())
                .build()
        )
        .mount("/", routes![get_all_user_sessions, logout_everywhere])
}
```

# Storage Providers

This crate supports multiple storage backends with different capabilities:

## Available Storage Providers

| Storage | Feature Flag | Indexing Support | Use Case |
|---------|-------------|------------------|----------|
| [`storage::memory::MemoryStorage`] | Built-in | ❌ | Development, testing |
| [`storage::memory::IndexedMemoryStorage`] | Built-in | ✅ | Development with indexing features |
| [`storage::cookie::CookieStorage`] | `cookie` | ❌ | Client-side storage, stateless servers |
| [`storage::redis::RedisFredStorage`] | `redis_fred` | ❌ | Production, distributed systems |
| [`storage::sqlx::SqlxPostgresStorage`] | `sqlx_postgres` | ✅* | Production, existing database |

*Support planned - see [Custom Storage](#custom-storage) section for implementation details.

## Custom Storage

To implement a custom storage provider, implement the [`SessionStorage`](crate::storage::SessionStorage) trait:

```rust
use rocket_flex_session::{error::SessionResult, storage::SessionStorage};
use rocket::{async_trait, http::CookieJar};

pub struct MyCustomStorage {}

#[async_trait]
impl<T> SessionStorage<T> for MyCustomStorage
where
    T: Send + Sync + Clone + 'static,
{
    async fn load(&self, id: &str, ttl: Option<u32>, cookie_jar: &CookieJar) -> SessionResult<(T, u32)> {
        // Load session from your storage
        todo!()
    }

    async fn save(&self, id: &str, data: T, ttl: u32) -> SessionResult<()> {
        // Save session to your storage
        todo!()
    }

    async fn delete(&self, id: &str, cookie_jar: &CookieJar) -> SessionResult<()> {
        // Delete session from your storage
        todo!()
    }
}
```

### Adding Indexing Support

To support session indexing, also implement [`SessionStorageIndexed`](crate::storage::SessionStorageIndexed) and add the `as_indexed_storage` method
to the [`SessionStorage`](crate::storage::SessionStorage) trait:


```rust,ignore
use rocket_flex_session::{error::SessionResult, storage::{SessionStorage, SessionStorageIndexed, SessionIdentifier}};

struct MyCustomStorage;

#[async_trait]
impl<T> SessionStorageIndexed<T> for MyCustomStorage
where
    T: SessionIdentifier + Send + Sync + Clone + 'static,
{
    async fn get_sessions_by_identifier(&self, id: &T::Id) -> SessionResult<Vec<(String, T)>> {
        // Return all (session_id, session_data) pairs for the identifier
        todo!()
    }
    // etc...
}

// Make sure to also add this to the `SessionStorage` trait to enable indexing support
#[async_trait]
impl<T> SessionStorage<T> for MyCustomStorage
where
    T: Send + Sync + Clone + 'static,
{
    // ... other methods ...

    fn as_indexed_storage(&self) -> Option<&dyn SessionStorageIndexed<T>> {
        Some(self) // Enable indexing support
    }
}
```

### Implementation Tips

1. **Thread Safety**: All storage implementations must be `Send + Sync`
2. **Trait bounds**: Add additional trait bounds to the session data type as needed
3. **Error Handling**: Use [`error::SessionError::Backend`] for custom errors
4. **TTL Handling**: Respect the TTL parameters in `load` and `save` for session expiration
5. **Indexing Consistency**: Keep identifier indexes in sync with session data
6. **Cleanup**: Implement proper cleanup in `shutdown()` if needed

# Feature flags

These features can be enabled as shown
[in Cargo's documentation](https://doc.rust-lang.org/cargo/reference/features.html).

| Name    | Description    |
|---------|----------------|
| `cookie` | A cookie-based session store. Data is serialized using serde_json and then encrypted into the value of a cookie. |
| `redis_fred`  | A session store for Redis (and Redis-compatible databases), using the [fred.rs](https://docs.rs/crate/fred) crate. |
| `sqlx_postgres`  | A session store using PostgreSQL via the [sqlx](https://docs.rs/crate/sqlx) crate. |
| `rocket_okapi`  | Enables support for the [rocket_okapi](https://docs.rs/crate/rocket_okapi) crate if needed. |
*/

mod fairing;
mod guard;
mod options;
mod session;
mod session_index;
mod session_inner;

pub mod error;
pub mod storage;
pub use fairing::{RocketFlexSession, RocketFlexSessionBuilder};
pub use options::RocketFlexSessionOptions;
pub use session::Session;
pub use session_index::SessionIdentifier;
