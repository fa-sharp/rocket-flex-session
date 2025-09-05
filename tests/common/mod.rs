use sqlx::{Connection, PgPool};

pub const POSTGRES_URL: &str = "postgres://postgres:postgres@localhost";

/// Setup a test Postgres database
pub async fn setup_postgres(base_url: &str) -> (PgPool, String) {
    let db_name = format!(
        "test_{}",
        (0..6)
            .map(|_| (b'a' + (rand::random::<u8>() % 26)) as char)
            .collect::<String>()
    );
    let mut cxn = sqlx::PgConnection::connect(base_url).await.unwrap();
    sqlx::query(&format!("CREATE DATABASE {}", db_name))
        .execute(&mut cxn)
        .await
        .expect("Should create test database");
    let _ = cxn.close().await;

    let db_url = format!("{}/{}", base_url, db_name);
    let pool = sqlx::PgPool::connect(&db_url).await.unwrap();
    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS sessions (
          id      TEXT PRIMARY KEY,
          data    TEXT NOT NULL,
          user_id TEXT,
          expires TIMESTAMPTZ NOT NULL
      )"#,
    )
    .execute(&pool)
    .await
    .expect("Should create sessions table");

    (pool, db_name)
}

pub async fn teardown_postgres(pool: sqlx::Pool<sqlx::Postgres>, db_name: String) {
    pool.close().await;
    drop(pool);
    let mut cxn = sqlx::PgConnection::connect(POSTGRES_URL).await.unwrap();
    sqlx::query(&format!("DROP DATABASE {} WITH (FORCE)", db_name))
        .execute(&mut cxn)
        .await
        .expect("Should drop test database");
}
