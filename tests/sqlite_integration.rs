use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use db_mcp::{
    config::{Config, ConnectionConfig, LimitsConfig, SQLiteConfig, ServerConfig},
    database::Manager,
};

fn test_db_path() -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    std::env::temp_dir().join(format!("dbmcp-sqlite-{suffix}.db"))
}

#[tokio::test]
async fn sqlite_read_only_dsn_supports_sequential_query_inputs() {
    sqlx::any::install_default_drivers();
    let path = test_db_path();
    let writable_dsn = format!("sqlite://{}?mode=rwc", path.display());
    let pool = sqlx::AnyPool::connect(&writable_dsn)
        .await
        .expect("create sqlite database");
    sqlx::query("create table if not exists batch_test (value integer)")
        .execute(&pool)
        .await
        .expect("initialize sqlite database");
    pool.close().await;

    let read_only_dsn = format!("sqlite://{}?mode=ro", path.display());
    let cfg = Config {
        server: ServerConfig::default(),
        limits: LimitsConfig::default(),
        connections: vec![ConnectionConfig {
            name: "sqlite".to_string(),
            driver: "sqlite".to_string(),
            sqlite: Some(SQLiteConfig {
                dsn: read_only_dsn,
                read_only: true,
            }),
            postgres: None,
            oracle: None,
        }],
    };
    let manager = Manager::new(&cfg).await.expect("manager init");
    let conn = manager.connection(Some("sqlite")).expect("connection");

    assert!(conn.supports_read_only_batch());
    let first = conn
        .execute_query("select\n1 as value;", 10, manager.limits())
        .await
        .expect("first batch statement");
    let second = conn
        .execute_query("select 2 as value;", 10, manager.limits())
        .await
        .expect("second batch statement");

    assert_eq!(
        first.rows[0].get("value").and_then(|value| value.as_i64()),
        Some(1)
    );
    assert_eq!(
        second.rows[0].get("value").and_then(|value| value.as_i64()),
        Some(2)
    );

    drop(conn);
    drop(manager);
    std::fs::remove_file(path).expect("remove test database");
}
