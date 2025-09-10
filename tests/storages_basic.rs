mod common;

#[macro_use]
extern crate rocket;

use std::{future::Future, pin::Pin};

use rocket::{
    futures::FutureExt, http::Status, local::asynchronous::Client, tokio::time::sleep, Build,
    Rocket,
};
use rocket_flex_session::{
    error::SessionError,
    storage::{
        cookie::CookieStorage,
        redis::{RedisFormat, RedisFredStorage, RedisValue, SessionRedis},
        sqlx::{SessionSqlx, SqlxPostgresStorage, SqlxSqliteStorage},
    },
    RocketFlexSession, Session, SessionIdentifier,
};
use serde::{Deserialize, Serialize};
use test_case::test_case;

use crate::common::{
    setup_postgres, setup_redis_fred, setup_sqlite, teardown_postgres, teardown_redis_fred,
    teardown_sqlite, POSTGRES_URL,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct SessionData {
    user_id: String,
}

impl SessionIdentifier for SessionData {
    type Id = String;

    fn identifier(&self) -> Option<Self::Id> {
        Some(self.user_id.clone())
    }
}

impl SessionSqlx<sqlx::Postgres> for SessionData {
    type Data = String;
    type Error = SessionError;
    fn into_sql(self) -> Result<Self::Data, Self::Error> {
        Ok(self.user_id)
    }
    fn from_sql(value: Self::Data) -> Result<Self, Self::Error> {
        Ok(Self { user_id: value })
    }
}

impl SessionSqlx<sqlx::Sqlite> for SessionData {
    type Data = String;
    type Error = SessionError;
    fn into_sql(self) -> Result<Self::Data, Self::Error> {
        Ok(self.user_id)
    }
    fn from_sql(value: Self::Data) -> Result<Self, Self::Error> {
        Ok(Self { user_id: value })
    }
}

impl SessionRedis for SessionData {
    const REDIS_FORMAT: RedisFormat = RedisFormat::Bytes;
    type Error = SessionError;

    fn into_redis(self) -> Result<RedisValue, Self::Error> {
        Ok(RedisValue::Bytes(self.user_id.into_bytes()))
    }

    fn from_redis(value: RedisValue) -> Result<Self, Self::Error> {
        let bytes = value.into_bytes().expect("should be bytes");
        let user_id = String::from_utf8(bytes).map_err(|e| SessionError::Parsing(e.into()))?;
        Ok(Self { user_id })
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
) -> (
    Rocket<Build>,
    Option<Pin<Box<dyn Future<Output = ()> + Send>>>,
) {
    let (fairing, cleanup_task) = match storage_case {
        "cookie" => (
            RocketFlexSession::<SessionData>::builder()
                .storage(CookieStorage::default())
                .build(),
            None,
        ),
        "redis" => {
            let (pool, prefix) = setup_redis_fred().await;
            let storage = RedisFredStorage::builder()
                .pool(pool.clone())
                .prefix(&prefix)
                .build();
            let fairing = RocketFlexSession::<SessionData>::builder()
                .storage(storage)
                .build();
            let cleanup_task = teardown_redis_fred(pool, prefix).boxed();
            (fairing, Some(cleanup_task))
        }
        "sqlx_postgres" => {
            let (pool, db_name) = setup_postgres(POSTGRES_URL).await;
            let storage = SqlxPostgresStorage::builder()
                .pool(pool.clone())
                .table_name("sessions")
                .build();
            let fairing = RocketFlexSession::<SessionData>::builder()
                .storage(storage)
                .build();
            let cleanup_task = teardown_postgres(pool, db_name).boxed();
            (fairing, Some(cleanup_task))
        }
        "sqlx_sqlite" => {
            let pool = setup_sqlite().await;
            let storage = SqlxSqliteStorage::builder()
                .pool(pool.clone())
                .table_name("sessions")
                .build();
            let fairing = RocketFlexSession::<SessionData>::builder()
                .storage(storage)
                .build();
            let cleanup_task = teardown_sqlite(pool).boxed();
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
#[test_case("redis"; "Redis Fred")]
#[test_case("sqlx_postgres"; "Sqlx Postgres")]
#[test_case("sqlx_sqlite"; "Sqlx SQLite")]
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
