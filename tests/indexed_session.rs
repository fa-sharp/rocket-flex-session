use rocket::{
    get, launch, routes,
    serde::{Deserialize, Serialize},
};
use rocket_flex_session::{
    storage::memory::IndexedMemoryStorage, RocketFlexSession, Session, SessionIdentifier,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct UserSession {
    user_id: String,
    username: String,
    login_time: u64,
}

impl SessionIdentifier for UserSession {
    type Id = String;

    fn identifier(&self) -> Option<&Self::Id> {
        Some(&self.user_id)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct AdminSession {
    admin_id: String,
    role: String,
    permissions: Vec<String>,
}

impl SessionIdentifier for AdminSession {
    type Id = String;

    fn identifier(&self) -> Option<&Self::Id> {
        Some(&self.admin_id)
    }
}

// Routes for testing user sessions
#[get("/user/login/<user_id>/<username>")]
async fn user_login(
    mut session: Session<'_, UserSession>,
    user_id: String,
    username: String,
) -> String {
    let user_session = UserSession {
        user_id: user_id.clone(),
        username: username.clone(),
        login_time: 1234567890,
    };

    session.set(user_session);
    format!("User {} logged in", username)
}

#[get("/user/sessions")]
async fn get_user_sessions(session: Session<'_, UserSession>) -> String {
    match session.get_all_sessions().await {
        Ok(Some(sessions)) => {
            format!("Found {} sessions for current user", sessions.len())
        }
        Ok(None) => "No current session".to_string(),
        Err(e) => format!("Error getting sessions: {}", e),
    }
}

#[get("/user/sessions/<user_id>")]
async fn get_sessions_for_user(session: Session<'_, UserSession>, user_id: String) -> String {
    match session.get_sessions_by_identifier(&user_id).await {
        Ok(sessions) => {
            format!("Sessions for user {}: {:?}", user_id, sessions)
        }
        Err(e) => format!("Error getting sessions: {}", e),
    }
}

#[get("/user/invalidate-all")]
async fn invalidate_all_user_sessions(session: Session<'_, UserSession>) -> String {
    match session.invalidate_all_sessions().await {
        Ok(Some(())) => "All sessions for current user invalidated".to_string(),
        Ok(None) => "No current session".to_string(),
        Err(e) => format!("Error invalidating sessions: {}", e),
    }
}

#[get("/user/invalidate-all/<user_id>")]
async fn invalidate_sessions_for_user(
    session: Session<'_, UserSession>,
    user_id: String,
) -> String {
    match session.invalidate_sessions_by_identifier(&user_id).await {
        Ok(()) => format!("All sessions for user {} invalidated", user_id),
        Err(e) => format!("Error invalidating sessions: {}", e),
    }
}

#[get("/user/session-ids")]
async fn get_user_session_ids(session: Session<'_, UserSession>) -> String {
    match session.get_all_session_ids().await {
        Ok(Some(session_ids)) => {
            format!("Session IDs for current user: {:?}", session_ids)
        }
        Ok(None) => "No current session".to_string(),
        Err(e) => format!("Error getting session IDs: {}", e),
    }
}

#[get("/user/profile")]
async fn user_profile(session: Session<'_, UserSession>) -> String {
    match session.get() {
        Some(user_session) => {
            format!(
                "Profile for {}: logged in at {}",
                user_session.username, user_session.login_time
            )
        }
        None => "No active session".to_string(),
    }
}

#[launch]
fn rocket() -> _ {
    let user_storage = IndexedMemoryStorage::<UserSession>::default();

    rocket::build()
        .attach(
            RocketFlexSession::<UserSession>::builder()
                .storage(user_storage)
                .build(),
        )
        .mount(
            "/",
            routes![
                user_login,
                get_user_sessions,
                get_sessions_for_user,
                invalidate_all_user_sessions,
                invalidate_sessions_for_user,
                get_user_session_ids,
                user_profile,
            ],
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rocket::http::Status;
    use rocket::local::blocking::Client;

    fn create_test_client() -> Client {
        Client::tracked(rocket()).expect("valid rocket instance")
    }

    #[test]
    fn user_login_and_profile() {
        let client = create_test_client();

        // Login user
        let response = client.get("/user/login/user1/alice").dispatch();
        assert_eq!(response.status(), Status::Ok);
        assert_eq!(response.into_string().unwrap(), "User alice logged in");

        // Check profile
        let response = client.get("/user/profile").dispatch();
        assert_eq!(response.status(), Status::Ok);
        assert!(response
            .into_string()
            .unwrap()
            .contains("Profile for alice"));
    }

    #[test]
    fn multiple_sessions_same_user() {
        let client = create_test_client();

        // First session for user1
        let response = client.get("/user/login/user1/alice").dispatch();
        assert_eq!(response.status(), Status::Ok);

        // Check that we can see current user's sessions
        let response = client.get("/user/sessions").dispatch();
        assert_eq!(response.status(), Status::Ok);
        // Note: This might show 0 or 1 sessions depending on whether the current
        // session cookie is being tracked properly in the test
    }

    #[test]
    fn get_sessions_by_user_id() {
        let client = create_test_client();

        // Login user
        let response = client.get("/user/login/user1/alice").dispatch();
        assert_eq!(response.status(), Status::Ok);

        // Get sessions for specific user ID
        let response = client.get("/user/sessions/user1").dispatch();
        assert_eq!(response.status(), Status::Ok);
        let body = response.into_string().unwrap();
        println!("{body}");
        assert!(body.contains("Sessions for user user1"));
    }

    #[test]
    fn test_session_ids_retrieval() {
        let client = create_test_client();

        // Login user
        let response = client.get("/user/login/user1/alice").dispatch();
        assert_eq!(response.status(), Status::Ok);

        // Get session IDs
        let response = client.get("/user/session-ids").dispatch();
        assert_eq!(response.status(), Status::Ok);
        let body = response.into_string().unwrap();
        assert!(body.contains("Session IDs for current user"));
    }

    #[test]
    fn test_invalidate_sessions() {
        let client = create_test_client();

        // Login user
        let response = client.get("/user/login/user1/alice").dispatch();
        assert_eq!(response.status(), Status::Ok);

        // Invalidate all sessions for current user
        let response = client.get("/user/invalidate-all").dispatch();
        assert_eq!(response.status(), Status::Ok);
        assert!(response
            .into_string()
            .unwrap()
            .contains("All sessions for current user invalidated"));

        // Profile should now show no session
        let response = client.get("/user/profile").dispatch();
        assert_eq!(response.status(), Status::Ok);
        assert_eq!(response.into_string().unwrap(), "No active session");
    }

    #[test]
    fn test_invalidate_sessions_by_user_id() {
        let client = create_test_client();

        // Login user
        let response = client.get("/user/login/user2/bob").dispatch();
        assert_eq!(response.status(), Status::Ok);

        // Invalidate sessions for specific user
        let response = client.get("/user/invalidate-all/user2").dispatch();
        assert_eq!(response.status(), Status::Ok);
        assert!(response
            .into_string()
            .unwrap()
            .contains("All sessions for user user2 invalidated"));
    }

    #[test]
    fn test_no_session_scenarios() {
        let client = create_test_client();

        // Try to get sessions without being logged in
        let response = client.get("/user/sessions").dispatch();
        assert_eq!(response.status(), Status::Ok);
        assert!(response
            .into_string()
            .unwrap()
            .contains("No current session"));

        // Try to get session IDs without being logged in
        let response = client.get("/user/session-ids").dispatch();
        assert_eq!(response.status(), Status::Ok);
        assert!(response
            .into_string()
            .unwrap()
            .contains("No current session"));

        // Try to invalidate sessions without being logged in
        let response = client.get("/user/invalidate-all").dispatch();
        assert_eq!(response.status(), Status::Ok);
        assert!(response
            .into_string()
            .unwrap()
            .contains("No current session"));
    }
}
