#[macro_use]
extern crate rocket;

use rocket::{
    form::validate::Contains,
    http::Status,
    outcome::try_outcome,
    request::{FromRequest, Outcome},
    serde::{Deserialize, Serialize},
    Build, Request, Rocket,
};
use rocket_flex_session::{RocketFlexSession, Session};
use std::collections::HashMap;

#[derive(Clone, Debug, FromFormField, Serialize, Deserialize, PartialEq)]
enum UserRole {
    User,
    Admin,
}
impl std::fmt::Display for UserRole {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            UserRole::User => write!(f, "user"),
            UserRole::Admin => write!(f, "admin"),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
struct User {
    role: UserRole,
}

struct Admin {
    user: User,
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for User {
    type Error = &'r str;

    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        // Run the Session request guard (this guard should always succeed)
        let session = req.guard::<Session<User>>().await.expect("should not fail");

        // Return the `User` session data, or if it's `None`, send an Unauthorized error
        match session.get() {
            Some(user) => Outcome::Success(user.to_owned()),
            None => Outcome::Error((Status::Unauthorized, "Not logged in")),
        }
    }
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for Admin {
    type Error = &'r str;

    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        // Run the User request guard to ensure there's a user
        let user = try_outcome!(req.guard::<User>().await);

        // Check user for admin role
        if user.role == UserRole::Admin {
            Outcome::Success(Admin { user })
        } else {
            Outcome::Forward(Status::Forbidden)
        }
    }
}

#[post("/login?<role>")]
fn login(role: UserRole, mut session: Session<User>) -> &'static str {
    session.set(User { role });
    "Logged in"
}

#[post("/logout")]
fn logout(mut session: Session<User>) -> &'static str {
    session.delete();
    "Logged out"
}

#[get("/user")]
fn get_user(user: User) -> String {
    format!("Logged in as {}", user.role)
}

#[get("/admin")]
fn admin_only_route(admin: Admin) -> String {
    format!("Admin access granted to {:?}", admin.user)
}

fn create_rocket() -> Rocket<Build> {
    rocket::build()
        .attach(RocketFlexSession::<User>::default())
        .attach(RocketFlexSession::<HashMap<String, String>>::default())
        .mount("/", routes![get_user, admin_only_route, login, logout])
}

use rocket::local::blocking::Client;

#[test]
fn test_unauthorized_access() {
    let client = Client::tracked(create_rocket()).unwrap();
    let response = client.get("/user").dispatch();
    assert_eq!(response.status(), Status::Unauthorized);
}

#[test]
fn test_login_logout_flow() {
    let client = Client::tracked(create_rocket()).unwrap();

    // Test login
    let login_response = client.post("/login?role=user").dispatch();
    assert_eq!(login_response.status(), Status::Ok);

    // Test accessing protected route after login
    let user_response = client.get("/user").dispatch();
    assert_eq!(user_response.status(), Status::Ok);
    assert_eq!(
        user_response.into_string(),
        Some("Logged in as user".into())
    );

    // Test logout
    let logout_response = client.post("/logout").dispatch();
    assert_eq!(logout_response.status(), Status::Ok);
    assert_eq!(logout_response.into_string(), Some("Logged out".into()));

    // Verify can't access protected route after logout
    let final_response = client.get("/user").dispatch();
    assert_eq!(final_response.status(), Status::Unauthorized);
}

#[test]
fn test_admin_access() {
    let client = Client::tracked(create_rocket()).unwrap();

    // Login as admin
    client.post("/login?role=admin").dispatch();

    // Test admin route access
    let response = client.get("/admin").dispatch();
    assert_eq!(response.status(), Status::Ok);
    assert!(response.into_string().contains("Admin access granted"));
}

#[test]
fn test_non_admin_access() {
    let client = Client::tracked(create_rocket()).unwrap();

    // Login as regular user
    client.post("/login?role=user").dispatch();

    // Try to access admin route
    let response = client.get("/admin").dispatch();
    assert_eq!(response.status(), Status::Forbidden);
}
