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
  use your own session storage by implementing the [SessionStorage] trait.

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
pub use options::SessionOptions;
pub use session::Session;
pub use session_index::SessionIdentifier;

use crate::storage::{memory::MemoryStorage, SessionStorage};
use std::sync::Arc;

/**
A Rocket fairing that enables sessions.

# Type Parameters
* `T` - The type of your session data. Must be thread-safe and
   implement Clone. The storage provider you use may have additional
   trait bounds as well.

# Example
```rust
use rocket_flex_session::{RocketFlexSession, SessionOptions, storage::cookie::CookieStorage};
use rocket::time::Duration;
use rocket::serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
struct MySession {
    user_id: String,
    role: String,
}

#[rocket::launch]
fn rocket() -> _ {
    // Use default settings
    let session_fairing = RocketFlexSession::<MySession>::default();

    // Or customize settings with the builder
    let custom_session = RocketFlexSession::<MySession>::builder()
        .storage(CookieStorage::default()) // or a custom storage provider
        .with_options(|opt| {
            opt.cookie_name = "my_cookie".to_string();
            opt.path = "/app".to_string();
            opt.max_age = 7 * 24 * 60 * 60; // 7 days
        })
        .build();

    rocket::build()
        .attach(session_fairing)
        // ... other configuration ...
}
```
*/
#[derive(Clone)]
pub struct RocketFlexSession<T> {
    pub(crate) options: SessionOptions,
    pub(crate) storage: Arc<dyn SessionStorage<T>>,
}
impl<T> RocketFlexSession<T>
where
    T: Send + Sync + Clone + 'static,
{
    /// Build a session configuration
    pub fn builder() -> RocketFlexSessionBuilder<T> {
        RocketFlexSessionBuilder::default()
    }
}
impl<T> Default for RocketFlexSession<T>
where
    T: Send + Sync + Clone + 'static,
{
    fn default() -> Self {
        Self {
            options: Default::default(),
            storage: Arc::new(MemoryStorage::default()),
        }
    }
}

/// Builder to configure the [RocketFlexSession] fairing
pub struct RocketFlexSessionBuilder<T>
where
    T: Send + Sync + Clone + 'static,
{
    fairing: RocketFlexSession<T>,
}
impl<T> Default for RocketFlexSessionBuilder<T>
where
    T: Send + Sync + Clone + 'static,
{
    fn default() -> Self {
        Self {
            fairing: Default::default(),
        }
    }
}
impl<T> RocketFlexSessionBuilder<T>
where
    T: Send + Sync + Clone + 'static,
{
    /// Set the session options via a closure. If you're using a cookie-based storage
    /// provider, make sure to set the corresponding cookie settings
    /// in the storage configuration as well.
    pub fn with_options<OptionsFn>(&mut self, options_fn: OptionsFn) -> &mut Self
    where
        OptionsFn: FnOnce(&mut SessionOptions),
    {
        options_fn(&mut self.fairing.options);
        self
    }

    /// Set the session storage provider
    pub fn storage<S>(&mut self, storage: S) -> &mut Self
    where
        S: SessionStorage<T> + 'static,
    {
        self.fairing.storage = Arc::new(storage);
        self
    }

    /// Build the fairing
    pub fn build(&self) -> RocketFlexSession<T> {
        self.fairing.clone()
    }
}
