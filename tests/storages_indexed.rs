mod common;

use std::{future::Future, pin::Pin};

use rocket::local::asynchronous::Client;
use rocket_flex_session::{
    storage::{memory::IndexedMemoryStorage, sqlx::SqlxPostgresStorage, SessionStorageIndexed},
    SessionIdentifier,
};
use sqlx::Connection;
use test_case::test_case;

use crate::common::{setup_postgres, POSTGRES_URL};

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
impl ToString for TestSession {
    fn to_string(&self) -> String {
        format!("{}:{}", self.user_id, self.data)
    }
}
impl TryFrom<String> for TestSession {
    type Error = std::io::Error;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let (user_id, data) = value.split_once(':').ok_or(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Invalid session format",
        ))?;
        Ok(TestSession {
            user_id: user_id.to_string(),
            data: data.to_string(),
        })
    }
}

async fn create_storage(
    storage_case: &str,
) -> (
    Box<dyn SessionStorageIndexed<TestSession>>,
    Option<Pin<Box<dyn Future<Output = ()>>>>,
) {
    match storage_case {
        "memory" => {
            let storage = IndexedMemoryStorage::<TestSession>::default();
            (Box::new(storage), None)
        }
        "sqlx" => {
            let (pool, db_name) = setup_postgres(POSTGRES_URL).await;
            let storage = SqlxPostgresStorage::new(pool.clone(), "sessions");

            let cleanup_task: Pin<Box<dyn Future<Output = ()>>> = Box::pin(async move {
                pool.close().await;
                drop(pool);
                let mut cxn = sqlx::PgConnection::connect(POSTGRES_URL).await.unwrap();
                sqlx::query(&format!("DROP DATABASE {} WITH (FORCE)", db_name))
                    .execute(&mut cxn)
                    .await
                    .expect("Should drop test database");
            });
            (Box::new(storage), Some(cleanup_task))
        }
        _ => unimplemented!(),
    }
}

#[test_case("memory")]
#[test_case("sqlx")]
#[rocket::async_test]
async fn basic_operations(storage_case: &str) {
    let (storage, cleanup_task) = create_storage(storage_case).await;
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
    if let Some(task) = cleanup_task {
        task.await
    }
}

#[test_case("memory")]
#[test_case("sqlx")]
#[rocket::async_test]
async fn invalidate_by_identifier(storage_case: &str) {
    let (storage, cleanup_task) = create_storage(storage_case).await;
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
    if let Some(task) = cleanup_task {
        task.await
    }
}

#[test_case("memory")]
#[test_case("sqlx")]
#[rocket::async_test]
async fn delete_single_session(storage_case: &str) {
    let client = Client::tracked(rocket::build()).await.unwrap();
    let (storage, cleanup_task) = create_storage(storage_case).await;
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
    if let Some(task) = cleanup_task {
        task.await
    }
}

#[test_case("memory")]
#[test_case("sqlx")]
#[rocket::async_test]
async fn nonexistent_identifier(storage_case: &str) {
    let (storage, cleanup_task) = create_storage(storage_case).await;
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
    if let Some(task) = cleanup_task {
        task.await
    }
}
