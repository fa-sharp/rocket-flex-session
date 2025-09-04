use rocket::local::asynchronous::Client;
use rocket_flex_session::{
    storage::{memory::IndexedMemoryStorage, SessionStorage, SessionStorageIndexed},
    SessionIdentifier,
};

#[derive(Clone, Debug, PartialEq)]
struct TestSession {
    user_id: String,
    data: String,
}

impl SessionIdentifier for TestSession {
    type Id = String;

    fn identifier(&self) -> Option<&Self::Id> {
        Some(&self.user_id)
    }
}

#[derive(Clone, Debug, PartialEq)]
struct SessionWithoutId {
    data: String,
}

impl SessionIdentifier for SessionWithoutId {
    type Id = String;

    fn identifier(&self) -> Option<&Self::Id> {
        None // This session type doesn't have an identifier
    }
}

#[rocket::async_test]
async fn indexed_memory_storage_basic_operations() {
    let storage = IndexedMemoryStorage::<TestSession>::default();
    storage.setup().await.unwrap();

    let session1 = TestSession {
        user_id: "user1".to_string(),
        data: "session1_data".to_string(),
    };
    let session2 = TestSession {
        user_id: "user1".to_string(),
        data: "session2_data".to_string(),
    };
    let session3 = TestSession {
        user_id: "user2".to_string(),
        data: "session3_data".to_string(),
    };

    // Save sessions
    storage.save("sid1", session1.clone(), 3600).await.unwrap();
    storage.save("sid2", session2.clone(), 3600).await.unwrap();
    storage.save("sid3", session3.clone(), 3600).await.unwrap();

    // Test get_sessions_by_identifier
    let user1_sessions = storage
        .get_sessions_by_identifier(&"user1".to_string())
        .await
        .unwrap();
    assert_eq!(user1_sessions.len(), 2);
    assert!(user1_sessions
        .iter()
        .any(|(id, data)| id == "sid1" && data == &session1));
    assert!(user1_sessions
        .iter()
        .any(|(id, data)| id == "sid2" && data == &session2));

    let user2_sessions = storage
        .get_sessions_by_identifier(&"user2".to_string())
        .await
        .unwrap();
    assert_eq!(user2_sessions.len(), 1);
    assert!(user2_sessions
        .iter()
        .any(|(id, data)| id == "sid3" && data == &session3));

    // Test get_session_ids_by_identifier
    let user1_session_ids = storage
        .get_session_ids_by_identifier(&"user1".to_string())
        .await
        .unwrap();
    assert_eq!(user1_session_ids.len(), 2);
    assert!(user1_session_ids.contains(&"sid1".to_string()));
    assert!(user1_session_ids.contains(&"sid2".to_string()));

    storage.shutdown().await.unwrap();
}

#[rocket::async_test]
async fn indexed_memory_storage_invalidate_by_identifier() {
    let storage = IndexedMemoryStorage::<TestSession>::default();
    storage.setup().await.unwrap();

    let session1 = TestSession {
        user_id: "user1".to_string(),
        data: "session1_data".to_string(),
    };
    let session2 = TestSession {
        user_id: "user1".to_string(),
        data: "session2_data".to_string(),
    };
    let session3 = TestSession {
        user_id: "user2".to_string(),
        data: "session3_data".to_string(),
    };

    // Save sessions
    storage.save("sid1", session1, 3600).await.unwrap();
    storage.save("sid2", session2, 3600).await.unwrap();
    storage.save("sid3", session3.clone(), 3600).await.unwrap();

    // Verify sessions exist
    assert_eq!(
        storage
            .get_sessions_by_identifier(&"user1".to_string())
            .await
            .unwrap()
            .len(),
        2
    );

    // Invalidate all sessions for user1
    storage
        .invalidate_sessions_by_identifier(&"user1".to_string())
        .await
        .unwrap();

    // Verify user1 sessions are gone
    assert_eq!(
        storage
            .get_sessions_by_identifier(&"user1".to_string())
            .await
            .unwrap()
            .len(),
        0
    );

    // Verify user2 session still exists
    let user2_sessions = storage
        .get_sessions_by_identifier(&"user2".to_string())
        .await
        .unwrap();
    assert_eq!(user2_sessions.len(), 1);
    assert!(user2_sessions
        .iter()
        .any(|(id, data)| id == "sid3" && data == &session3));

    storage.shutdown().await.unwrap();
}

#[rocket::async_test]
async fn indexed_memory_storage_delete_single_session() {
    let client = Client::tracked(rocket::build()).await.unwrap();
    let storage = IndexedMemoryStorage::<TestSession>::default();
    storage.setup().await.unwrap();

    let session1 = TestSession {
        user_id: "user1".to_string(),
        data: "session1_data".to_string(),
    };
    let session2 = TestSession {
        user_id: "user1".to_string(),
        data: "session2_data".to_string(),
    };

    // Save sessions
    storage.save("sid1", session1.clone(), 3600).await.unwrap();
    storage.save("sid2", session2.clone(), 3600).await.unwrap();

    // Verify both sessions exist
    assert_eq!(
        storage
            .get_sessions_by_identifier(&"user1".to_string())
            .await
            .unwrap()
            .len(),
        2
    );

    // Delete one session
    storage.delete("sid1", &client.cookies()).await.unwrap();

    // Verify only one session remains
    let remaining_sessions = storage
        .get_sessions_by_identifier(&"user1".to_string())
        .await
        .unwrap();
    assert_eq!(remaining_sessions.len(), 1);
    assert!(remaining_sessions
        .iter()
        .any(|(id, data)| id == "sid2" && data == &session2));

    storage.shutdown().await.unwrap();
}

#[rocket::async_test]
async fn indexed_memory_storage_session_without_identifier() {
    let client = Client::tracked(rocket::build()).await.unwrap();
    let storage = IndexedMemoryStorage::<SessionWithoutId>::default();
    storage.setup().await.unwrap();

    let session = SessionWithoutId {
        data: "test_data".to_string(),
    };

    // Save session (should not be indexed)
    storage.save("sid1", session.clone(), 3600).await.unwrap();

    // Try to get sessions by identifier (should return empty)
    let sessions = storage
        .get_sessions_by_identifier(&"any_id".to_string())
        .await
        .unwrap();
    assert_eq!(sessions.len(), 0);

    // Regular session operations should still work
    let (loaded_session, _ttl) = storage.load("sid1", None, &client.cookies()).await.unwrap();
    assert_eq!(loaded_session, session);

    storage.shutdown().await.unwrap();
}

#[rocket::async_test]
async fn indexed_memory_storage_nonexistent_identifier() {
    let storage = IndexedMemoryStorage::<TestSession>::default();
    storage.setup().await.unwrap();

    // Try to get sessions for non-existent identifier
    let sessions = storage
        .get_sessions_by_identifier(&"nonexistent".to_string())
        .await
        .unwrap();
    assert_eq!(sessions.len(), 0);

    // Try to get session IDs for non-existent identifier
    let session_ids = storage
        .get_session_ids_by_identifier(&"nonexistent".to_string())
        .await
        .unwrap();
    assert_eq!(session_ids.len(), 0);

    // Try to invalidate sessions for non-existent identifier (should not error)
    storage
        .invalidate_sessions_by_identifier(&"nonexistent".to_string())
        .await
        .unwrap();

    storage.shutdown().await.unwrap();
}
