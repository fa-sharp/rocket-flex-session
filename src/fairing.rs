use std::{
    marker::{Send, Sync},
    sync::{Arc, Mutex},
};

use bon::Builder;
use rocket::{fairing::Fairing, Build, Orbit, Request, Response, Rocket};

use crate::{
    guard::LocalCachedSession,
    storage::{memory::MemoryStorage, SessionStorage},
    RocketFlexSessionOptions,
};

/**
A Rocket fairing that enables sessions.

# Type Parameters
* `T` - The type of your session data. Must be thread-safe and
   implement Clone. The storage provider you use may have additional
   trait bounds as well.

# Example
```rust
use rocket_flex_session::{RocketFlexSession, storage::cookie::CookieStorage};
use rocket::time::Duration;
use rocket::serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
struct MySession {
    user_id: String,
    role: String,
}

#[rocket::launch]
fn rocket() -> _ {
    // Use default settings with in-memory storage
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
#[derive(Builder)]
pub struct RocketFlexSession<T: Send + Sync + Clone + 'static> {
    /// Set the options directly. Alternatively, use `with_options` to customize the default options via a closure.
    #[builder(default)]
    pub(crate) options: RocketFlexSessionOptions,
    #[builder(default = Arc::new(MemoryStorage::default()), with = |storage: impl SessionStorage<T> + 'static| Arc::new(storage))]
    /// Set the session storage provider. The default is an in-memory storage.
    pub(crate) storage: Arc<dyn SessionStorage<T>>,
}

impl<T> Default for RocketFlexSession<T>
where
    T: Send + Sync + Clone + 'static,
{
    /// Create a new instance with default options and an in-memory storage.
    fn default() -> Self {
        Self {
            options: Default::default(),
            storage: Arc::new(MemoryStorage::default()),
        }
    }
}

use rocket_flex_session_builder::{IsUnset, SetOptions, State};
impl<T, S> RocketFlexSessionBuilder<T, S>
where
    T: Send + Sync + Clone + 'static,
    S: State,
{
    /// Customize the [options](RocketFlexSessionOptions) via a closure. Any options that are not set will retain their default values.
    pub fn with_options<OptionsFn>(
        self,
        options_fn: OptionsFn,
    ) -> RocketFlexSessionBuilder<T, SetOptions<S>>
    where
        S::Options: IsUnset,
        OptionsFn: FnOnce(&mut RocketFlexSessionOptions),
    {
        let mut options = RocketFlexSessionOptions::default();
        options_fn(&mut options);
        self.options(options)
    }
}

#[rocket::async_trait]
impl<T> Fairing for RocketFlexSession<T>
where
    T: Send + Sync + Clone + 'static,
{
    fn info(&self) -> rocket::fairing::Info {
        use rocket::fairing::Kind;
        rocket::fairing::Info {
            name: "Rocket Flex Session",
            kind: Kind::Ignite | Kind::Response | Kind::Shutdown | Kind::Singleton,
        }
    }

    async fn on_ignite(&self, rocket: Rocket<Build>) -> Result<Rocket<Build>, Rocket<Build>> {
        rocket::debug!("Setting up session resources...");
        if let Err(e) = self.storage.setup().await {
            rocket::warn!("Error during session storage setup: {}", e);
        }

        Ok(rocket.manage::<RocketFlexSession<T>>(RocketFlexSession {
            options: self.options.clone(),
            storage: self.storage.clone(),
        }))
    }

    async fn on_response<'r>(&self, req: &'r Request<'_>, _res: &mut Response<'r>) {
        // Get session data from request local cache, or generate a default empty one
        let (session_inner, _): &LocalCachedSession<T> =
            req.local_cache(|| (Mutex::default(), None));

        // Take inner session data
        let (updated, deleted) = session_inner.lock().unwrap().take_for_storage();

        // Handle deleted session
        if let Some(deleted_id) = deleted {
            rocket::debug!("Found deleted session. Deleting session '{deleted_id}'...");
            if let Err(e) = self.storage.delete(&deleted_id, req.cookies()).await {
                rocket::warn!("Error while deleting session '{deleted_id}': {e}");
            } else {
                rocket::debug!("Deleted session '{deleted_id}' successfully");
            }
        }

        // Handle updated session
        if let Some((id, pending_data, ttl)) = updated {
            rocket::debug!("Found updated session. Saving session '{id}'...");
            if let Err(e) = self.storage.save(&id, pending_data, ttl).await {
                rocket::error!("Error while saving session '{id}': {e}");
            } else {
                rocket::debug!("Saved session '{id}' successfully");
            }
        }
    }

    async fn on_shutdown(&self, _rocket: &Rocket<Orbit>) {
        rocket::debug!("Shutting down session resources...");
        if let Err(e) = self.storage.shutdown().await {
            rocket::warn!("Error during session storage shutdown: {e}");
        }
    }
}
