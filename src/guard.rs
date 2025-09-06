use std::{any::type_name, sync::Mutex};

use rocket::{
    http::CookieJar,
    request::{FromRequest, Outcome},
    Request,
};

use crate::{
    error::SessionError, session_inner::SessionInner, storage::SessionStorage, RocketFlexSession,
    Session,
};

/// Type of the cached inner session data in Rocket's request local cache
pub(crate) type LocalCachedSession<T> = (Mutex<SessionInner<T>>, Option<SessionError>);

#[rocket::async_trait]
impl<'r, T> FromRequest<'r> for Session<'r, T>
where
    T: Send + Sync + Clone + 'static,
{
    /// Unused outcome error type - this request guard shouldn't fail
    type Error = &'r str;

    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let fairing = get_fairing::<T>(req.rocket());
        let cookie_jar = req.cookies();

        // Use rocket's local cache so that the session data is only fetched once per request
        let (cached_inner, session_error): &LocalCachedSession<T> = req
            .local_cache_async(async {
                fetch_session_data(
                    cookie_jar,
                    &fairing.options.cookie_name,
                    fairing
                        .options
                        .rolling
                        .then(|| fairing.options.ttl.unwrap_or(fairing.options.max_age)),
                    fairing.storage.as_ref(),
                )
                .await
            })
            .await;

        Outcome::Success(Session::new(
            cached_inner,
            session_error.as_ref(),
            cookie_jar,
            &fairing.options,
            fairing.storage.as_ref(),
        ))
    }
}

/// Get session configuration from Rocket state
#[inline(always)]
fn get_fairing<T>(rocket: &rocket::Rocket<rocket::Orbit>) -> &RocketFlexSession<T>
where
    T: Send + Sync + Clone + 'static,
{
    rocket.state::<RocketFlexSession<T>>().unwrap_or_else(|| {
        panic!(
            "The RocketFlexSession<{}> fairing should be attached to the server",
            type_name::<T>()
        )
    })
}

/// Fetch session data from storage
#[inline(always)]
async fn fetch_session_data<'r, T: Send + Sync + Clone>(
    cookie_jar: &'r CookieJar<'_>,
    cookie_name: &str,
    rolling_ttl: Option<u32>,
    storage: &'r dyn SessionStorage<T>,
) -> LocalCachedSession<T> {
    let session_cookie = cookie_jar.get_private(cookie_name);
    if let Some(cookie) = session_cookie {
        let id = cookie.value();
        rocket::debug!("Got session id '{}' from cookie. Retrieving session...", id);
        match storage.load(id, rolling_ttl, cookie_jar).await {
            Ok((data, ttl)) => {
                rocket::debug!("Session found. Creating existing session...");
                let session_inner = SessionInner::new_existing(id, data, ttl);
                (Mutex::new(session_inner), None)
            }
            Err(e) => {
                rocket::debug!("Error from session storage, creating empty session: {}", e);
                (Mutex::default(), Some(e))
            }
        }
    } else {
        rocket::debug!("No valid session cookie found. Creating empty session...");
        (Mutex::default(), Some(SessionError::NoSessionCookie))
    }
}

/// If using rocket-okapi, this implements OpenApiFromRequest for Session to ignore the request guard
#[cfg(feature = "rocket_okapi")]
impl<'r, T> rocket_okapi::request::OpenApiFromRequest<'r> for Session<'r, T>
where
    T: Send + Sync + Clone + 'static,
{
    fn from_request_input(
        _gen: &mut rocket_okapi::gen::OpenApiGenerator,
        _name: String,
        _required: bool,
    ) -> rocket_okapi::Result<rocket_okapi::request::RequestHeaderInput> {
        Ok(rocket_okapi::request::RequestHeaderInput::None)
    }
}
