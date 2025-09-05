mod common;

#[macro_use]
extern crate rocket;

use std::{future::Future, pin::Pin};

use fred::prelude::{ClientLike, ReconnectPolicy};
use rocket::{http::Status, local::asynchronous::Client, tokio::time::sleep, Build, Rocket};
use rocket_flex_session::{
    error::SessionError,
    storage::{
        cookie::CookieStorage,
        redis::{RedisFredStorage, RedisType},
        sqlx::SqlxPostgresStorage,
    },
    RocketFlexSession, Session, SessionIdentifier,
};
use serde::{Deserialize, Serialize};
use sqlx::Connection;
use test_case::test_case;

use crate::common::{setup_postgres, POSTGRES_URL};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct SessionData {
    user_id: String,
}
impl TryFrom<String> for SessionData {
    type Error = SessionError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Ok(Self { user_id: value })
    }
}
impl fred::types::FromValue for SessionData {
    fn from_value(value: fred::prelude::Value) -> Result<Self, fred::prelude::Error> {
        Ok(Self {
            user_id: value.convert()?,
        })
    }
}
impl std::fmt::Display for SessionData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.user_id)
    }
}
impl TryFrom<fred::types::Value> for SessionData {
    type Error = SessionError;
    fn try_from(value: fred::types::Value) -> Result<Self, Self::Error> {
        let user_id = value.as_string().ok_or(SessionError::NotFound)?;
        Ok(Self { user_id })
    }
}
impl From<SessionData> for fred::types::Value {
    fn from(value: SessionData) -> Self {
        Self::String(value.user_id.into())
    }
}
impl SessionIdentifier for SessionData {
    const NAME: &str = "user_id";
    type Id = String;
    fn identifier(&self) -> Option<&Self::Id> {
        Some(&self.user_id)
    }
}

#[get("/get_session")]
fn get_session(session: Session<SessionData>) -> String {
    match session.get() {
        Some(data) => format!("User: {}", data.user_id),
        None => "No session".to_string(),
    }
}

#[post("/set_session")]
fn set_session(mut session: Session<SessionData>) -> String {
    session.set(SessionData {
        user_id: "123".to_string(),
    });
    session.id().unwrap().to_owned()
}

#[post("/delete_session")]
fn delete_session(mut session: Session<SessionData>) -> &'static str {
    session.delete();
    "Session deleted"
}

#[post("/expire_session")]
fn expire_session(mut session: Session<SessionData>) {
    session.set_ttl(1);
}

async fn create_rocket(
    storage_case: &str,
) -> (Rocket<Build>, Option<Pin<Box<dyn Future<Output = ()>>>>) {
    let (fairing, cleanup_task) = match storage_case {
        "cookie" => (
            RocketFlexSession::<SessionData>::builder()
                .storage(CookieStorage::default())
                .build(),
            None,
        ),
        "redis" => {
            let pool = fred::prelude::Builder::default_centralized()
                .set_policy(ReconnectPolicy::new_linear(3, 5, 1))
                .with_performance_config(|c| {
                    c.default_command_timeout = std::time::Duration::from_secs(5)
                })
                .build_pool(3)
                .expect("Should build Redis pool");
            pool.init().await.expect("Should initialize Redis pool");
            let storage = RedisFredStorage::new(pool.clone(), RedisType::String, "sess:");
            let fairing = RocketFlexSession::<SessionData>::builder()
                .storage(storage)
                .build();

            let cleanup_task: Pin<Box<dyn Future<Output = ()>>> = Box::pin(async move {
                pool.quit().await.ok();
                drop(pool);
            });
            (fairing, Some(cleanup_task))
        }
        "sqlx" => {
            let (pool, db_name) = setup_postgres(POSTGRES_URL).await;
            let storage = SqlxPostgresStorage::new(pool.clone(), "sessions");
            let fairing = RocketFlexSession::<SessionData>::builder()
                .storage(storage)
                .build();

            let cleanup_task: Pin<Box<dyn Future<Output = ()>>> = Box::pin(async move {
                pool.close().await;
                drop(pool);
                let mut cxn = sqlx::PgConnection::connect(POSTGRES_URL).await.unwrap();
                sqlx::query(&format!("DROP DATABASE {} WITH (FORCE)", db_name))
                    .execute(&mut cxn)
                    .await
                    .expect("Should drop test database");
            });
            (fairing, Some(cleanup_task))
        }
        _ => unimplemented!(),
    };

    let rocket = rocket::build().attach(fairing).mount(
        "/",
        routes![get_session, set_session, delete_session, expire_session],
    );

    (rocket, cleanup_task)
}

#[test_case("cookie"; "Cookie")]
#[test_case("redis"; "Fred Redis")]
#[test_case("sqlx"; "Sqlx Postgres")]
#[rocket::async_test]
async fn test_storages(storage_case: &str) {
    let (rocket, cleanup_task) = create_rocket(storage_case).await;
    let client = Client::tracked(rocket).await.unwrap();

    let response = client.get("/get_session").dispatch().await;
    assert_eq!(response.status(), Status::Ok);
    assert_eq!(
        response.into_string().await.unwrap(),
        "No session",
        "Is empty session"
    );

    let set_response = client.post("/set_session").dispatch().await;
    let cookie = set_response
        .cookies()
        .get_private("rocket")
        .expect("should have session cookie");
    assert_eq!(set_response.status(), Status::Ok);
    assert_eq!(
        cookie.value(),
        set_response.into_string().await.unwrap(),
        "Session ID set properly"
    );

    let authed_response = client.get("/get_session").dispatch().await;
    assert_eq!(authed_response.status(), Status::Ok);
    assert_eq!(
        authed_response.into_string().await.unwrap(),
        "User: 123",
        "Session is active"
    );

    client.post("/expire_session").dispatch().await;
    sleep(std::time::Duration::from_secs(2)).await;
    let expired_response = client.get("/get_session").dispatch().await;
    assert_eq!(
        expired_response.into_string().await.unwrap(),
        "No session",
        "Session is expired"
    );

    client.cookies().remove_private("rocket");
    client.post("/set_session").dispatch().await;
    let authed_response = client.get("/get_session").dispatch().await;
    assert_eq!(
        authed_response.into_string().await.unwrap(),
        "User: 123",
        "Session is active"
    );
    let delete_response = client.post("/delete_session").dispatch().await;
    assert_eq!(delete_response.status(), Status::Ok);
    let should_be_deleted = client.get("/get_session").dispatch().await;
    assert_eq!(
        should_be_deleted.into_string().await.unwrap(),
        "No session",
        "Session is deleted"
    );

    if let Some(task) = cleanup_task {
        task.await
    }
}
