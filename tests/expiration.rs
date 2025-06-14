#[macro_use]
extern crate rocket;

use rocket::{http::Status, local::blocking::Client, routes, Build, Rocket};
use rocket_flex_session::{RocketFlexSession, Session};

#[post("/set_session")]
fn set_session(mut session: Session<String>) -> &'static str {
    session.set("active".to_owned());
    "Session set"
}

#[get("/get_session")]
fn get_session(session: Session<String>) -> Result<String, Status> {
    match session.get() {
        Some(session) => Ok(format!("Session: {}", session)),
        None => Err(Status::Unauthorized),
    }
}

// Create rocket instance with custom expiration
fn create_rocket_with_expiration(max_age: u32) -> Rocket<Build> {
    rocket::build()
        .attach(
            RocketFlexSession::<String>::builder()
                .with_options(|opt| opt.max_age = max_age)
                .build(),
        )
        .mount("/", routes![get_session, set_session,])
}

#[test]
fn test_session_expiry() {
    // Create a rocket instance with 3 second expiration
    let client = Client::tracked(create_rocket_with_expiration(1)).unwrap();

    // Set session
    client.post("/set_session").dispatch();

    // Verify session exists
    assert_eq!(client.get("/get_session").dispatch().status(), Status::Ok);

    // Wait 0.5 seconds
    std::thread::sleep(std::time::Duration::from_secs_f32(0.5));

    // Verify session still valid
    assert_eq!(client.get("/get_session").dispatch().status(), Status::Ok);

    // Wait another 0.5 seconds
    std::thread::sleep(std::time::Duration::from_secs_f32(0.5));

    // Session should now be invalid even if sending expired cookie
    let mut expired_cookie = client.cookies().get_private("rocket").unwrap();
    assert_eq!(expired_cookie.max_age(), Some(time::Duration::seconds(1)));
    expired_cookie.set_max_age(time::Duration::seconds(100));
    expired_cookie.set_expires(Some(
        time::OffsetDateTime::now_utc() + time::Duration::seconds(100),
    ));
    let response = client
        .get("/get_session")
        .private_cookie(expired_cookie)
        .dispatch();
    assert_eq!(response.status(), Status::Unauthorized);
}
