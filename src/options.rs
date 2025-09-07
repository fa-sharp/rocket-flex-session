/// Options for configuring the session.
#[derive(Clone, Debug)]
pub struct RocketFlexSessionOptions {
    /// The name of the cookie used to store the session ID (default: `"rocket"`)
    pub cookie_name: String,
    /// The session cookie's `Domain` attribute (default: `None`)
    pub domain: Option<String>,
    /// The session cookie's `HttpOnly` attribute (default: `true`)
    pub http_only: bool,
    /// The session cookie's `Max-Age` attribute, in seconds. This also determines
    /// the session storage TTL, unless you specify a different `ttl` setting. (default: 2 weeks)
    pub max_age: u32,
    /// The session cookie's `Path` attribute (default: `"/"`)
    pub path: String,
    /// Enable 'rolling' sessions where the TTL is extended every time the session is accessed.
    /// This should be used in combination with a shorter `ttl` setting to enable short-lived
    /// sessions that are automatically extended for active users. (default: `false`)
    pub rolling: bool,
    /// The session cookie's `SameSite` attribute (default: `SameSite::Lax`)
    pub same_site: rocket::http::SameSite,
    /// The session cookie's `Secure` attribute (default: `true`).
    /// When developing on localhost, you may need to set this to `false` on some browsers.
    pub secure: bool,
    /// The default TTL (time-to-live) for sessions, in seconds. This value is passed to the
    /// configured session storage. If not set, this defaults to the `max_age` setting.
    pub ttl: Option<u32>,
}

impl Default for RocketFlexSessionOptions {
    fn default() -> Self {
        Self {
            cookie_name: "rocket".to_owned(),
            domain: None,
            http_only: true,
            max_age: 14 * 24 * 60 * 60, // 14 days
            path: "/".to_owned(),
            rolling: false,
            same_site: rocket::http::SameSite::Lax,
            secure: true,
            ttl: None,
        }
    }
}
