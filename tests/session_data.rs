#[macro_use]
extern crate rocket;

use rocket::{
    http::Status,
    local::blocking::Client,
    serde::{Deserialize, Serialize},
    time::Duration,
    {routes, Build, Rocket},
};
use rocket_flex_session::{storage::cookie::CookieStorage, RocketFlexSession, Session};
use serde_json::json;
use std::collections::HashMap;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
struct LargeSession {
    id: String,
    data: Vec<String>,
    nested: HashMap<String, Vec<i32>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
struct InvalidSession {
    #[serde(deserialize_with = "deserialize_fail")]
    invalid_data: i32,
}

// Deserializer that immediately fails
fn deserialize_fail<'de, D>(_deserializer: D) -> Result<i32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Err(serde::de::Error::custom("Testing deserialize fail"))
}

#[get("/get_invalid_session")]
fn get_invalid_session(session: Session<InvalidSession>) -> String {
    match session.get() {
        Some(data) => format!("Shouldn't get this: {:?}", data),
        None => "No session".to_string(),
    }
}

#[get("/get_large_session")]
fn get_large_session(session: Session<LargeSession>) -> String {
    match session.get() {
        Some(data) => format!("Session size: {}", data.data.len()),
        None => "No session".to_string(),
    }
}

#[post("/set_large_session")]
fn set_large_session(mut session: Session<LargeSession>) -> &'static str {
    let mut large_data = Vec::new();
    for i in 0..100 {
        large_data.push(format!("Data entry {}", i));
    }

    let mut nested = HashMap::new();
    nested.insert("numbers".to_string(), (0..100).collect());

    session.set(LargeSession {
        id: "large_session".to_string(),
        data: large_data,
        nested,
    });
    "Large session set"
}

fn create_rocket() -> Rocket<Build> {
    rocket::build()
        .attach(
            RocketFlexSession::<InvalidSession>::builder()
                .with_options(|opt| opt.cookie_name = "invalid_session".to_owned())
                .build(),
        )
        .attach(
            RocketFlexSession::<LargeSession>::builder()
                .with_options(|opt| opt.cookie_name = "large_session".to_owned())
                .storage(
                    CookieStorage::builder()
                        .with_options(|opt| opt.cookie_name = "large_session_data".to_owned())
                        .build(),
                )
                .build(),
        )
        .mount(
            "/",
            routes![get_invalid_session, get_large_session, set_large_session,],
        )
}

#[test]
fn test_large_session_data() {
    let client = Client::tracked(create_rocket()).unwrap();

    // Set large session
    let set_response = client.post("/set_large_session").dispatch();
    assert_eq!(set_response.status(), Status::Ok);

    // Verify cookie size
    let cookie = set_response
        .cookies()
        .get_private("large_session_data")
        .expect("should have session data cookie");
    println!("Large session cookie size: {} bytes", cookie.value().len());
    assert!(cookie.value().len() > 2000); // Sanity check that it's actually large

    // Verify large session was stored and can be retrieved
    let get_response = client.get("/get_large_session").dispatch();
    assert_eq!(get_response.status(), Status::Ok);
    assert_eq!(get_response.into_string().unwrap(), "Session size: 100");
}

#[test]
fn test_invalid_session_data() {
    use rocket::http::Cookie;

    let client = Client::tracked(create_rocket()).unwrap();

    // Manually create an invalid session cookie
    let invalid_data = json!({
        "id": "test_id",
        "data": InvalidSession {
            invalid_data: 0,
        },
        "expires": time::OffsetDateTime::now_utc() + Duration::hours(1),
    });

    let cookie_value = serde_json::to_string(&invalid_data).unwrap();
    let cookie = Cookie::new("invalid_session", cookie_value);

    // The session should be treated as empty when invalid data is encountered
    let response = client
        .get("/get_invalid_session")
        .private_cookie(cookie)
        .dispatch();
    assert_eq!(response.into_string().unwrap(), "No session");
}

#[test]
fn test_malformed_cookie() {
    use rocket::http::Cookie;

    let client = Client::tracked(create_rocket()).unwrap();

    // Create a malformed cookie
    let cookie = Cookie::new("invalid_session", "not valid cookie data");
    client.cookies().add_private(cookie);

    // The session should be treated as empty when the cookie is malformed
    let response = client.get("/get_large_session").dispatch();
    assert_eq!(response.into_string().unwrap(), "No session");
}
