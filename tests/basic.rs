#[macro_use]
extern crate rocket;

use rocket::{
    http::Status,
    local::blocking::Client,
    {routes, Build, Rocket},
};
use rocket_flex_session::{
    storage::cookie::CookieStorage, RocketFlexSession, Session, SessionHashMap,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
struct User {
    id: String,
    name: String,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct SessionHash(HashMap<String, String>);

impl SessionHashMap for SessionHash {
    type Value = String;

    fn get(&self, key: &str) -> Option<&Self::Value> {
        self.0.get(key)
    }
    fn insert(&mut self, key: String, value: Self::Value) {
        self.0.insert(key, value);
    }
    fn remove(&mut self, key: &str) {
        self.0.remove(key);
    }
}

#[get("/get_session")]
fn get_session(session: Session<User>) -> String {
    match session.get() {
        Some(user) => format!("User: {} ({})", user.name, user.id),
        None => "No session".to_string(),
    }
}

#[post("/set_session")]
fn set_session(mut session: Session<User>) -> String {
    session.set(User {
        id: "123".to_string(),
        name: "Test User".to_string(),
    });
    session.id().unwrap().to_owned()
}

#[post("/delete_session")]
fn delete_session(mut session: Session<User>) -> &'static str {
    session.delete();
    "Session deleted"
}

#[get("/get_hash_session/<key>")]
fn get_hash_session(session: Session<SessionHash>, key: &str) -> String {
    match session.get_key(key) {
        Some(value) => value,
        None => "No value".to_string(),
    }
}

#[post("/set_hash_session/<key>/<value>")]
fn set_hash_session(mut session: Session<SessionHash>, key: &str, value: &str) -> &'static str {
    session.set_key(key.to_owned(), value.to_owned());
    "Hash session value set"
}

fn create_rocket() -> Rocket<Build> {
    rocket::build()
        .attach(RocketFlexSession::<User>::default())
        .attach(
            RocketFlexSession::<SessionHash>::builder()
                .with_options(|opt| opt.cookie_name = "hash_session".to_owned())
                .storage(
                    CookieStorage::builder()
                        .with_options(|opt| opt.cookie_name = "hash_session_data".to_owned())
                        .build(),
                )
                .build(),
        )
        .mount(
            "/",
            routes![
                get_session,
                set_session,
                delete_session,
                get_hash_session,
                set_hash_session,
            ],
        )
}

#[test]
fn test_empty_session() {
    let client = Client::tracked(create_rocket()).unwrap();
    let response = client.get("/get_session").dispatch();

    assert_eq!(response.status(), Status::Ok);
    assert_eq!(response.into_string().unwrap(), "No session");
}

#[test]
fn test_set_and_get_session() {
    let client = Client::tracked(create_rocket()).unwrap();

    // Set session
    let set_response = client.post("/set_session").dispatch();

    // Verify cookie was set
    let cookie = set_response
        .cookies()
        .get_private("rocket")
        .expect("should have session cookie");
    let cookie_value = cookie.value().to_owned();

    assert_eq!(set_response.status(), Status::Ok);
    assert_eq!(cookie_value, set_response.into_string().unwrap());

    // Get session
    let get_response = client.get("/get_session").dispatch();
    assert_eq!(get_response.status(), Status::Ok);
    assert_eq!(get_response.into_string().unwrap(), "User: Test User (123)");

    // Update session
    let set_response = client.post("/set_session").dispatch();

    // Verify cookie was not changed
    assert_eq!(set_response.cookies().get_private("rocket"), None);
    assert_eq!(set_response.status(), Status::Ok);
}

#[test]
fn test_delete_session() {
    let client = Client::tracked(create_rocket()).unwrap();

    // Set then delete session
    client.post("/set_session").dispatch();
    let response = client.post("/delete_session").dispatch();
    assert_eq!(response.status(), Status::Ok);

    // Verify session was deleted
    let response = client.get("/get_session").dispatch();
    assert_eq!(response.into_string().unwrap(), "No session");
}

#[test]
fn test_hashmap_session() {
    let client = Client::tracked(create_rocket()).unwrap();

    // Set hash value
    let response = client
        .post("/set_hash_session/test_key/test_value")
        .dispatch();
    assert_eq!(response.status(), Status::Ok);

    // Verify session cookie was set
    response
        .cookies()
        .get_private("hash_session")
        .expect("should have session cookie");

    // Set another hash value, and verify session cookie was not changed
    let response = client
        .post("/set_hash_session/test_key_2/test_value")
        .dispatch();
    assert_eq!(response.status(), Status::Ok);
    assert_eq!(response.cookies().get_private("hash_session"), None);

    // Get hash values
    let response = client.get("/get_hash_session/test_key").dispatch();
    assert_eq!(response.status(), Status::Ok);
    assert_eq!(response.into_string().unwrap(), "test_value");

    let response = client.get("/get_hash_session/test_key_2").dispatch();
    assert_eq!(response.status(), Status::Ok);
    assert_eq!(response.into_string().unwrap(), "test_value");

    // Get non-existent key
    let response = client.get("/get_hash_session/invalid_key").dispatch();
    assert_eq!(response.into_string().unwrap(), "No value");
}

#[test]
fn test_session_persistence() {
    let client = Client::tracked(create_rocket()).unwrap();

    // Set session
    client.post("/set_session").dispatch();

    // Make multiple requests - session should persist
    for _ in 0..3 {
        let response = client.get("/get_session").dispatch();
        assert_eq!(response.into_string().unwrap(), "User: Test User (123)");
    }
}
