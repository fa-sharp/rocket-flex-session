//! Cookie-based session storage implementation

use rocket::{
    async_trait,
    http::{Cookie, CookieJar},
    serde::{de::DeserializeOwned, json::serde_json, Deserialize, Serialize},
    time::{Duration, OffsetDateTime},
};

use crate::error::{SessionError, SessionResult};

use super::interface::SessionStorage;

/**
Storage provider for sessions backed by cookies. All session data is serialized to JSON
and then encrypted into the cookie value. Keep in mind that cookies are limited to
4KB in size, and must be sent with every request, so session data should be kept as small as
possible.

This provider requires that your session data type
implements `serde::Serialize` and `serde::Deserialize`.

# Example

```
use rocket_flex_session::storage::cookie::{CookieStorage, CookieStorageOptions};

// Create with default options
let default_storage = CookieStorage::default();

// Create with custom options using builder pattern
let custom_storage = CookieStorage::builder()
    .with_options(|opt| {
        opt.cookie_name = "my_session".to_owned();
        opt.path = "/app".to_owned();
        opt.secure = true;
        opt.http_only = true;
    })
    .build();
```
*/
#[derive(Default)]
pub struct CookieStorage {
    options: CookieStorageOptions,
}
impl CookieStorage {
    pub fn builder() -> CookieStorageBuilder {
        CookieStorageBuilder::default()
    }
}

#[derive(Default)]
pub struct CookieStorageBuilder {
    options: CookieStorageOptions,
}
impl CookieStorageBuilder {
    /// Set the cookie options via a closure
    pub fn with_options<OptionsFn>(&mut self, options_fn: OptionsFn) -> &mut Self
    where
        OptionsFn: FnOnce(&mut CookieStorageOptions),
    {
        options_fn(&mut self.options);
        self
    }

    /// Build the cookie storage provider
    pub fn build(&self) -> CookieStorage {
        CookieStorage {
            options: self.options.clone(),
        }
    }
}
#[derive(Clone)]
pub struct CookieStorageOptions {
    /// Name of the cookie holding the encrypted session data. **This should be a different
    /// name from the main session cookie.**
    ///
    /// default: `"rocket_session"`
    pub cookie_name: String,
    /// default: `None`
    pub domain: Option<String>,
    /// default: `true`
    pub http_only: bool,
    /// default: `"/"`
    pub path: String,
    /// default: `SameSite::Lax`
    pub same_site: rocket::http::SameSite,
    /// default: `true`
    pub secure: bool,
}

impl Default for CookieStorageOptions {
    fn default() -> Self {
        Self {
            cookie_name: "rocket_session".to_owned(),
            domain: None,
            http_only: true,
            path: "/".to_owned(),
            same_site: rocket::http::SameSite::Lax,
            secure: true,
        }
    }
}

#[async_trait]
impl<T> SessionStorage<T> for CookieStorage
where
    T: Serialize + DeserializeOwned + Send + Sync + 'static,
{
    async fn load(
        &self,
        id: &str,
        ttl: Option<u32>,
        cookie_jar: &CookieJar,
    ) -> SessionResult<(T, u32)> {
        let cookie = cookie_jar
            .get_private(&self.options.cookie_name)
            .ok_or(SessionError::NotFound)?;
        let cookie_data = serde_json::from_str::<DeserializedCookieSession<T>>(cookie.value())
            .map_err(|e| SessionError::Serialization(Box::new(e)))?;
        if cookie_data.id != id || cookie_data.expires <= OffsetDateTime::now_utc() {
            return Err(SessionError::Expired);
        }

        if let Some(new_ttl) = ttl {
            let new_cookie = create_storage_cookie(
                SerializedCookieSession::<T> {
                    id,
                    data: &cookie_data.data,
                    expires: OffsetDateTime::now_utc() + Duration::seconds(new_ttl.into()),
                },
                &self.options,
            )?;
            cookie_jar.add_private(new_cookie);
        }

        Ok((
            cookie_data.data,
            ttl.unwrap_or((OffsetDateTime::now_utc() - cookie_data.expires).whole_seconds() as u32),
        ))
    }

    fn save_cookie(
        &self,
        id: &str,
        data: Option<&T>,
        ttl: u32,
        cookie_jar: &CookieJar,
    ) -> SessionResult<()> {
        if let Some(data) = data {
            // Save new data on cookie
            let new_cookie = create_storage_cookie(
                SerializedCookieSession {
                    id,
                    data,
                    expires: OffsetDateTime::now_utc() + Duration::seconds(ttl.into()),
                },
                &self.options,
            )?;
            cookie_jar.add_private(new_cookie);
            Ok(())
        } else {
            // Delete cookie
            cookie_jar.remove_private(
                Cookie::build(self.options.cookie_name.clone()).path(self.options.path.clone()),
            );
            Ok(())
        }
    }

    async fn save(&self, _id: &str, _data: T, _ttl: u32) -> SessionResult<()> {
        Ok(()) // no-op (cookie session should already be saved by `save_cookie`)
    }

    async fn delete(&self, _id: &str, _data: T) -> SessionResult<()> {
        Ok(()) // no-op (cookie session should already be deleted by `save_cookie`)
    }
}

/// Represents a session retrieved from the cookie
#[derive(Debug, Deserialize)]
#[serde(crate = "rocket::serde")]
struct DeserializedCookieSession<T> {
    pub id: String,
    pub data: T,
    pub expires: OffsetDateTime,
}

/// Represents data saved to the cookie. Structure should match [DeserializedCookieSession] - just
/// using references here so we don't have to clone.
#[derive(Debug, Serialize)]
#[serde(crate = "rocket::serde")]
struct SerializedCookieSession<'a, T> {
    pub id: &'a str,
    pub data: &'a T,
    pub expires: OffsetDateTime,
}

fn create_storage_cookie<'a, T>(
    data: SerializedCookieSession<T>,
    options: &CookieStorageOptions,
) -> SessionResult<Cookie<'a>>
where
    T: Serialize + DeserializeOwned + Send + Sync,
{
    let name = options.cookie_name.clone();
    let value =
        serde_json::to_string(&data).map_err(|e| SessionError::Serialization(Box::new(e)))?;
    let cookie = Cookie::build((name, value))
        .secure(options.secure)
        .http_only(options.http_only)
        .path(options.path.clone())
        .expires(data.expires)
        .build();

    Ok(cookie)
}
