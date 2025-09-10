//! This is an example of using Redis sessions.

use rocket::{fairing::AdHoc, futures::FutureExt, http::Status, routes, serde::json::Json};
use rocket_flex_session::{
    storage::redis::{RedisFormat, RedisFredStorage, RedisValue, SessionRedis},
    RocketFlexSession, Session, SessionIdentifier,
};
use serde::{Deserialize, Serialize};

// Create a simple session data structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct BasicSession {
    user_id: u32,
    name: String,
}

// Implement SessionIdentifier to group sessions by user ID
impl SessionIdentifier for BasicSession {
    type Id = String;

    fn identifier(&self) -> Option<Self::Id> {
        Some(self.user_id.to_string())
    }
}

// Implement the Redis session trait (using JSON serialization here, but you
// can customize the conversion however you like)
impl SessionRedis for BasicSession {
    // Storing session as a string in Redis
    const REDIS_TYPE: RedisFormat = RedisFormat::String;

    // Conversion error type
    type Error = serde_json::Error;

    // Convert to Redis value
    fn into_redis(self) -> Result<RedisValue, Self::Error> {
        let value = serde_json::to_string(&self)?;
        Ok(RedisValue::String(value))
    }

    // Convert from Redis value
    fn from_redis(value: RedisValue) -> Result<Self, Self::Error> {
        // Can safely assume the value is a string according to the REDIS_TYPE above
        let value = value.into_string().expect("Should be a string type");
        serde_json::from_str(&value)
    }
}

#[rocket::launch]
async fn basic() -> _ {
    // We'll create an AdHoc fairing to setup the Redis pool and the session fairing,
    // and another one that handles disconnecting the Redis pool on shutdown. You may want to
    // create a separate fairing for Redis to keep things organized.

    let setup_session = AdHoc::on_ignite("Sessions", |rocket| async {
        use fred::prelude::*;

        // Build and initialize the Redis pool
        let config = Config::from_url("redis://my-redis-server").expect("should parse Redis URL");
        let pool = Builder::from_config(config)
            .build_pool(4)
            .expect("should build Redis pool");
        pool.init().await.expect("should initialize Redis pool");

        // Build the session storage
        let storage = RedisFredStorage::builder()
            .pool(pool.clone())
            .prefix("sess:")
            .index_prefix("sess:user:")
            .build();

        // Build the session fairing, passing in the session data type as the generic parameter
        let session_fairing = RocketFlexSession::<BasicSession>::builder()
            .storage(storage)
            .with_options(|opt| {
                // customize the cookie name
                opt.cookie_name = "my-cookie-name".to_string();
                // more options available:
                // opt.ttl = 60 * 60 * 24 * 7; // session TTL in seconds
                // opt.domain = "example.com".to_string(); // cookie domain
                // opt.path = "/".to_string(); // cookie path
                // etc...
            })
            .build();

        // Attach the session fairing, and add the Redis pool to Rocket state
        rocket.attach(session_fairing).manage(pool)
    });

    let shutdown_session = AdHoc::on_shutdown("Shutdown", |rocket| {
        async {
            use fred::prelude::{ClientLike, Pool};

            // Get the Redis pool from Rocket state, and quit the connection
            let pool = rocket.state::<Pool>().expect("should be in Rocket state");
            if let Err(e) = pool.quit().await {
                eprintln!("Failed to quit Redis connection: {e}");
            }
        }
        .boxed()
    });

    // Attach the fairings and mount the routes
    rocket::build()
        .attach(setup_session)
        .attach(shutdown_session)
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
    let _ = session.get_session_ids_by_identifier(&"foo".into()).await;

    match session.invalidate_all_sessions(false).await {
        Ok(Some(n)) => Ok(format!("Logged out from {} sessions", n)),
        Ok(None) => Err((Status::Unauthorized, "Not logged in".to_string())),
        Err(err) => Err((Status::InternalServerError, err.to_string())),
    }
}
