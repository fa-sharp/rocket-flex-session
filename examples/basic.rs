//! This is a basic example of how to use RocketFlexSession. To demonstrate the capabilities
//! of the library, this will store session data in memory for a debug build and in Redis in
//! the release build. In a real-world app using Redis, you should also use Redis locally
//! (e.g. with Docker) to match your production environment.

use rocket::{http::Status, routes, serde::json::Json};
use rocket_flex_session::{
    storage::{
        memory::MemoryStorageIndexed,
        redis::{RedisFredStorage, RedisFredStorageIndexed, RedisType},
    },
    RocketFlexSession, Session, SessionIdentifier,
};
use serde::{Deserialize, Serialize};

// Create a simple session data structure. Implement SessionIdentifier
// to enable grouping sessions by user ID.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct BasicSession {
    user_id: u32,
    name: String,
}

impl SessionIdentifier for BasicSession {
    type Id = u32;

    fn identifier(&self) -> Option<&Self::Id> {
        Some(&self.user_id)
    }
}

// Implement the Redis conversion traits for fred.rs (using JSON serialization here, but you
// can use any method you like)
impl fred::prelude::FromValue for BasicSession {
    fn from_value(value: fred::prelude::Value) -> Result<Self, fred::prelude::Error> {
        use fred::prelude::{Error, ErrorKind};

        let value_str: String = value.convert()?;
        serde_json::from_str(&value_str).map_err(|e| Error::new(ErrorKind::Parse, e.to_string()))
    }
}
impl TryFrom<BasicSession> for fred::prelude::Value {
    type Error = serde_json::Error;

    fn try_from(value: BasicSession) -> Result<Self, Self::Error> {
        serde_json::to_string(&value).map(fred::prelude::Value::from)
    }
}

#[rocket::launch]
async fn basic() -> _ {
    // Build the session fairing, passing in the session data type as the generic parameter
    let builder = RocketFlexSession::<BasicSession>::builder().with_options(|opt| {
        // customize the cookie name
        opt.cookie_name = "my-cookie-name".to_string();
        // more options available:
        // opt.ttl = 60 * 60 * 24 * 7; // session TTL in seconds
        // opt.domain = "example.com".to_string(); // cookie domain
        // opt.path = "/".to_string(); // cookie path
        // etc...
    });

    // Use an in-memory storage for development/debug mode, and Redis storage for production
    let session_fairing = {
        if cfg!(debug_assertions) {
            builder.storage(MemoryStorageIndexed::default()).build()
        } else {
            let config = fred::prelude::Config::from_url("redis://my-redis-server")
                .expect("Invalid Redis URL");
            let pool = fred::prelude::Builder::from_config(config)
                .build_pool(4)
                .expect("Failed to build Redis pool");
            let base_storage = RedisFredStorage::builder()
                .pool(pool.clone())
                .redis_type(RedisType::String) // Store session data as a Redis string
                .build();
            let storage = RedisFredStorageIndexed::from_storage(base_storage).build();
            builder.storage(storage).build()
        }
    };

    // Attach the session fairing and mount the routes
    rocket::build()
        .attach(session_fairing)
        .mount("/", routes![login, logout, user, logout_everywhere])
}

#[derive(Deserialize)]
struct LoginData {
    username: String,
    password: String,
}

#[rocket::post("/login", data = "<data>")]
async fn login(
    mut session: Session<'_, BasicSession>,
    data: Json<LoginData>,
) -> Result<&'static str, (Status, &'static str)> {
    if session.tap(|data| data.is_some()) {
        return Err((Status::BadRequest, "Already logged in"));
    }

    // Implement actual login logic here
    if data.username == "rossg" && data.password == "dinosaurs" {
        session.set(BasicSession {
            user_id: 1,
            name: "Ross".to_string(),
        });
        Ok("Logged in")
    } else {
        Err((Status::Unauthorized, "Invalid credentials"))
    }
}

#[rocket::get("/user")]
async fn user(session: Session<'_, BasicSession>) -> Result<String, (Status, &'static str)> {
    match session.tap(|data| data.map(|d| d.user_id)) {
        Some(user_id) => Ok(format!("User ID: {}", user_id)),
        None => Err((Status::Unauthorized, "Not logged in")),
    }
}

#[rocket::post("/logout")]
async fn logout(mut session: Session<'_, BasicSession>) -> &'static str {
    session.delete();
    "Logged out"
}

#[rocket::post("/logout-everywhere")]
async fn logout_everywhere(session: Session<'_, BasicSession>) -> Result<String, (Status, String)> {
    match session.invalidate_all_sessions(false).await {
        Ok(Some(n)) => Ok(format!("Logged out from {} sessions", n)),
        Ok(None) => Err((Status::Unauthorized, "Not logged in".to_string())),
        Err(err) => Err((Status::InternalServerError, err.to_string())),
    }
}
