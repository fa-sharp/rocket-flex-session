# Rocket Flex Session

Simple, extensible session library for Rocket applications.

## Features

- **Secure**: Session cookies are encrypted using Rocket's built-in private cookies
- **Flexible**: Use a custom struct or HashMap as your session data. Multiple storage providers available, as well as support for custom storage implementations.
- **Efficient**: Uses Rocket's request-local cache to minimize backend calls

## Quick Start

Add to your `Cargo.toml`:

```toml
[dependencies]
rocket = "0.5"
rocket-flex-session = { version = "0.1" }
```

Basic usage:

```rust
use rocket::routes;
use rocket_flex_session::{Session, RocketFlexSession};

#[derive(Clone)]
struct MySession {
    user_id: String,
}

#[rocket::launch]
fn rocket() -> _ {
    rocket::build()
        .attach(RocketFlexSession::<MySession>::default())
        .mount("/", routes![login])
}

#[rocket::post("/login")]
async fn login(mut session: Session<MySession>) {
    session.set(MySession { user_id: "123".to_owned() });
}
```

## Storage Options

- **Memory** (default) - In-memory storage, for local development
- **Cookie** - Client-side encrypted cookies, serialized using [serde](https://serde.rs/) (`cookie` feature)
- **Redis** - Redis-backed sessions via the [fred](https://docs.rs/fred) crate (`redis_fred` feature)
- **PostgreSQL** - Postgres-backed sessions via sqlx (`sqlx_postgres` feature)
- **Custom** - Implement the `SessionStorage` trait


## Request Guard Pattern

Build authentication and authorization layers using Rocket's request guard system:

```rust
#[rocket::async_trait]
impl<'r> FromRequest<'r> for MySession {
    type Error = &'r str;

    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let session = req.guard::<Session<MySession>>().await.expect("should not fail");
        match session.get() {
            Some(data) => Outcome::Success(data),
            None => Outcome::Error((Status::Unauthorized, "Not logged in")),
        }
    }
}
```

## Documentation

See the [full documentation](https://docs.rs/rocket-flex-session) for detailed usage examples, configuration options, and common patterns.

## License

This project is licensed under the MIT license.
