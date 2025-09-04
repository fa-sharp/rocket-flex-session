use rocket::{
    http::{Cookie, CookieJar},
    time::{Duration, OffsetDateTime},
};
use std::{
    collections::HashMap,
    fmt::Display,
    hash::Hash,
    marker::{Send, Sync},
    sync::{Mutex, MutexGuard},
};

use crate::{
    error::SessionError, options::RocketFlexSessionOptions, session_inner::SessionInner,
    storage::SessionStorage,
};

/**
Represents the current session state. When used as a request guard, it will
attempt to retrieve the session. The request guard will always succeed - if a
valid session wasn't found, `session.get()` will return `None` indicating an
inactive session.

# Type Parameters
* `T` - The session data type

# Example
```rust
use rocket_flex_session::Session;
use rocket::serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
struct UserSession {
    user_id: String,
    login_time: String
}

#[rocket::get("/profile")]
fn profile(session: Session<UserSession>) -> String {
    match session.get() {
        Some(data) => format!("User {} logged in at {}", data.user_id, data.login_time),
        None => "No active session".to_string()
    }
}
```
*/
pub struct Session<'a, T>
where
    T: Send + Sync + Clone,
{
    /// Internal mutable state of the session
    inner: &'a Mutex<SessionInner<T>>,
    /// Error (if any) when retrieving from storage
    error: Option<&'a SessionError>,
    /// Rocket's cookie jar for managing cookies
    cookie_jar: &'a CookieJar<'a>,
    /// User's session options
    options: &'a RocketFlexSessionOptions,
    /// Configured storage provider for sessions
    pub(crate) storage: &'a dyn SessionStorage<T>,
}

impl<T> Display for Session<'_, T>
where
    T: Send + Sync + Clone,
{
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "Session(id: {:?})", self.get_inner_lock().get_id())
    }
}

impl<'a, T> Session<'a, T>
where
    T: Send + Sync + Clone,
{
    /// Create a new session instance to keep track of the session state in a request
    pub(crate) fn new(
        inner: &'a Mutex<SessionInner<T>>,
        error: Option<&'a SessionError>,
        cookie_jar: &'a CookieJar<'a>,
        options: &'a RocketFlexSessionOptions,
        storage: &'a dyn SessionStorage<T>,
    ) -> Self {
        Self {
            inner,
            error,
            cookie_jar,
            options,
            storage,
        }
    }

    /// Get the session ID (alphanumeric string). Will be `None` if there's no active session.
    pub fn id(&self) -> Option<String> {
        self.get_inner_lock().get_id().map(|s| s.to_owned())
    }

    /// Get the current session data via cloning. Will be `None` if there's no active session.
    pub fn get(&self) -> Option<T> {
        self.get_inner_lock()
            .get_current_data()
            .map(|d| d.to_owned())
    }

    /// Get a reference to the current session data via a closure.
    /// Data will be `None` if there's no active session.
    pub fn tap<F, R>(&self, f: F) -> R
    where
        F: FnOnce(Option<&T>) -> R,
    {
        f(self.get_inner_lock().get_current_data())
    }

    /// Get a mutable reference to the current session data via a closure.
    /// The function's argument will be `None` if there's no active session.
    pub fn tap_mut<UpdateFn, R>(&mut self, f: UpdateFn) -> R
    where
        UpdateFn: FnOnce(&mut Option<T>) -> R,
    {
        let (response, is_deleted) = self
            .get_inner_lock()
            .tap_data_mut(f, self.get_default_ttl());
        if is_deleted {
            self.delete();
        } else {
            self.update_cookies();
        }

        response
    }

    /// Set/update the session data. Will create a new active session if needed.
    pub fn set(&mut self, new_data: T) {
        self.get_inner_lock()
            .set_data(new_data, self.get_default_ttl());
        self.update_cookies();
    }

    /// Set the TTL of the session in seconds. This can be used to extend the length
    /// of the session if needed. This has no effect if there is no active session.
    pub fn set_ttl(&mut self, new_ttl: u32) {
        self.get_inner_lock().set_ttl(new_ttl);
        self.update_cookies();
    }

    /// Get the session TTL in seconds.
    pub fn ttl(&self) -> u32 {
        self.get_inner_lock()
            .get_current_ttl()
            .unwrap_or(self.get_default_ttl())
    }

    /// Get the session expiration.
    pub fn expires(&self) -> OffsetDateTime {
        OffsetDateTime::now_utc().saturating_add(Duration::seconds(self.ttl().into()))
    }

    /// Delete the session.
    pub fn delete(&mut self) {
        // Delete inner session data
        let mut inner = self.get_inner_lock();
        inner.delete();

        // Remove the session cookie
        let mut remove_cookie =
            Cookie::build(self.options.cookie_name.to_owned()).path(self.options.path.to_owned());
        if let Some(domain) = &self.options.domain {
            remove_cookie = remove_cookie.domain(domain.to_owned());
        }
        self.cookie_jar.remove_private(remove_cookie);

        // Notify any cookie-based storage
        if let Some(deleted_id) = inner.get_deleted_id() {
            let delete_result = self
                .storage
                .save_cookie(deleted_id, None, 0, self.cookie_jar);
            if let Err(e) = delete_result {
                rocket::error!("Error while deleting session {:?}: {}", deleted_id, e);
            }
        }
    }

    /// Get the error (if any) during session retrieval.
    /// Note that this 'error' could be completely expected - e.g. a
    /// `SessionError::NoSessionCookie` if the user hasn't authenticated.
    pub fn error(&self) -> Option<&SessionError> {
        self.error
    }

    pub(crate) fn get_inner_lock(&self) -> MutexGuard<'_, SessionInner<T>> {
        self.inner.lock().expect("Failed to get session data lock")
    }

    fn get_default_ttl(&self) -> u32 {
        self.options.ttl.unwrap_or(self.options.max_age)
    }

    fn update_cookies(&self) {
        let inner = self.get_inner_lock();
        let Some(id) = inner.get_id() else {
            rocket::info!("Cookies not updated: no active session");
            return;
        };

        // Generate new cookie
        self.cookie_jar
            .add_private(create_session_cookie(id, self.options));

        // Notify any cookie-based storage
        let save_result = self.storage.save_cookie(
            id,
            inner.get_current_data(),
            inner.get_current_ttl().unwrap_or(self.get_default_ttl()),
            self.cookie_jar,
        );
        if let Err(e) = save_result {
            rocket::error!("Error while saving session {:?}: {}", id, e);
        };
    }
}

impl<K, V> Session<'_, HashMap<K, V>>
where
    K: Eq + Hash + Send + Sync + Clone,
    V: Send + Sync + Clone,
{
    /// Get the value of a key in the session data via cloning
    pub fn get_key<Q>(&self, key: &Q) -> Option<V>
    where
        Q: ?Sized + Eq + Hash,
        K: std::borrow::Borrow<Q>,
    {
        self.get_inner_lock()
            .get_current_data()
            .and_then(|h| h.get(key).cloned())
    }

    /// Set the value of a key in the session data. Will create
    /// a new session if needed.
    pub fn set_key(&mut self, key: K, value: V) {
        let mut new_data = self
            .get_inner_lock()
            .get_current_data()
            .cloned()
            .unwrap_or_default();
        new_data.insert(key, value);
        self.set(new_data);
    }

    /// Set multiple keys and values in the session data. Will create
    /// a new session if needed.
    pub fn set_keys<I>(&mut self, kv_iter: I)
    where
        I: IntoIterator<Item = (K, V)>,
    {
        let mut new_data = self
            .get_inner_lock()
            .get_current_data()
            .cloned()
            .unwrap_or_default();
        new_data.extend(kv_iter);
        self.set(new_data);
    }
}

/// Create the session cookie
fn create_session_cookie(id: &str, options: &RocketFlexSessionOptions) -> Cookie<'static> {
    let mut cookie = Cookie::build((options.cookie_name.to_owned(), id.to_owned()))
        .http_only(options.http_only)
        .max_age(Duration::seconds(options.max_age.into()))
        .path(options.path.clone())
        .same_site(options.same_site)
        .secure(options.secure);

    if let Some(domain) = &options.domain {
        cookie = cookie.domain(domain.clone());
    }

    cookie.build()
}
