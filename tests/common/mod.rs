use fred::prelude::{ClientLike, KeysInterface, ReconnectPolicy};
use sqlx::{Connection, PgPool};

pub const POSTGRES_URL: &str = "postgres://postgres:postgres@localhost";

fn random_string(n: usize) -> String {
    (0..n)
        .map(|_| (b'a' + (rand::random::<u8>() % 26)) as char)
        .collect()
}

/// Setup a test Postgres database
pub async fn setup_postgres(base_url: &str) -> (PgPool, String) {
    let db_name = format!("test_{}", random_string(6));
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

pub async fn setup_redis_fred() -> (fred::prelude::Pool, String) {
    let pool = fred::prelude::Builder::default_centralized()
        .set_policy(ReconnectPolicy::new_linear(3, 5, 1))
        .with_performance_config(|c| c.default_command_timeout = std::time::Duration::from_secs(5))
        .build_pool(3)
        .expect("Should build Redis pool");
    pool.init().await.expect("Should initialize Redis pool");
    let prefix = format!("test_{}:sess:", random_string(6));

    (pool, prefix)
}

pub async fn teardown_redis_fred(pool: fred::prelude::Pool, prefix: String) {
    let (_cursor, keys): (String, Vec<String>) = pool
        .scan_page("0", format!("{prefix}*"), Some(50), None)
        .await
        .expect("Should scan keys");
    if !keys.is_empty() {
        let _: () = pool.del(keys).await.expect("Should delete keys");
    }
    pool.quit().await.expect("Should quit Redis pool");
}
