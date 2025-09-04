use std::{
    marker::{Send, Sync},
    sync::{Arc, Mutex},
};

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
    pub(crate) options: RocketFlexSessionOptions,
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
        OptionsFn: FnOnce(&mut RocketFlexSessionOptions),
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
            let delete_result = self.storage.delete(&deleted_id, req.cookies()).await;
            if let Err(e) = delete_result {
                rocket::error!("Error while deleting session '{}': {}", deleted_id, e);
            }
        }

        // Handle updated session
        if let Some((id, pending_data, ttl)) = updated {
            let save_result = self.storage.save(&id, pending_data, ttl).await;
            if let Err(e) = save_result {
                rocket::error!("Error while saving session '{}': {}", &id, e);
            }
        }
    }

    async fn on_shutdown(&self, _rocket: &Rocket<Orbit>) {
        rocket::debug!("Shutting down session resources...");
        if let Err(e) = self.storage.shutdown().await {
            rocket::warn!("Error during session storage shutdown: {}", e);
        }
    }
}
