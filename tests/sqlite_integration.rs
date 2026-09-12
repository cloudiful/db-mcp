use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use db_mcp::{
    config::{Config, ConnectionConfig, LimitsConfig, SQLiteConfig, ServerConfig},
    database::{Cancellation, Manager, Operation, cancellation_report},
};

/// CPU-bound query that always outlives a short test budget.
const SLOW_SQL: &str = "with recursive counter(value) as (\
    select 1 union all select value + 1 from counter where value < 5000000\
) select count(*) as value from counter";

fn read_only_config(dsn: String, query_timeout: &str) -> Config {
    let limits = LimitsConfig {
        query_timeout: query_timeout.to_string(),
        ..Default::default()
    };
    Config {
        server: ServerConfig::default(),
        limits,
        connections: vec![ConnectionConfig {
            name: "sqlite".to_string(),
            driver: "sqlite".to_string(),
            sqlite: Some(SQLiteConfig {
                dsn,
                read_only: true,
            }),
            postgres: None,
            oracle: None,
        }],
    }
}

fn test_db_path() -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    std::env::temp_dir().join(format!("dbmcp-sqlite-{suffix}.db"))
}

#[tokio::test]
async fn sqlite_read_only_dsn_supports_sequential_query_inputs() {
    let path = test_db_path();
    let writable_dsn = format!("sqlite://{}?mode=rwc", path.display());
    let pool = sqlx::SqlitePool::connect(&writable_dsn)
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

#[tokio::test]
async fn sqlite_empty_result_set_keeps_column_names() {
    let path = test_db_path();
    let writable_dsn = format!("sqlite://{}?mode=rwc", path.display());
    let pool = sqlx::SqlitePool::connect(&writable_dsn)
        .await
        .expect("create sqlite database");
    sqlx::query("create table if not exists probe (value integer)")
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

    let result = conn
        .execute_query("select value from probe where 1 = 0", 10, manager.limits())
        .await
        .expect("execute_query with empty result");

    assert_eq!(result.row_count, 0);
    assert_eq!(result.columns.len(), 1);
    assert_eq!(result.columns[0].name, "value");

    drop(conn);
    drop(manager);
    std::fs::remove_file(path).expect("remove test database");
}

#[tokio::test]
async fn sqlite_progress_handler_confirms_cancellation_and_the_pool_recovers() {
    let path = test_db_path();
    let writable_dsn = format!("sqlite://{}?mode=rwc", path.display());
    let pool = sqlx::SqlitePool::connect(&writable_dsn)
        .await
        .expect("create sqlite database");
    pool.close().await;

    let read_only_dsn = format!("sqlite://{}?mode=ro", path.display());
    let cfg = read_only_config(read_only_dsn, "300ms");
    let manager = Manager::new(&cfg).await.expect("manager init");
    let conn = manager.connection(Some("sqlite")).expect("connection");

    let err = conn
        .execute_query(SLOW_SQL, 5, manager.limits())
        .await
        .expect_err("slow query should be interrupted at the deadline");

    let report = cancellation_report(&err).expect("cancellation should be classified");
    assert_eq!(
        report.cancellation(),
        Cancellation::Confirmed,
        "the SQLite progress handler should confirm the interrupt: {err:#}"
    );
    assert!(
        matches!(
            report.operation(),
            Operation::Describe | Operation::Execute | Operation::Fetch
        ),
        "unexpected operation: {:?}",
        report.operation()
    );
    assert!(
        format!("{err:#}").contains("timeout"),
        "unexpected: {err:#}"
    );

    // The driver confirmed the interrupt and reset the statement, so the worker
    // is reusable and the next query succeeds on the same pool.
    let after = conn
        .execute_query("select 1 as value", 5, manager.limits())
        .await
        .expect("pool should recover after a confirmed interrupt");
    assert_eq!(
        after.rows[0].get("value").and_then(|value| value.as_i64()),
        Some(1)
    );

    drop(conn);
    drop(manager);
    std::fs::remove_file(path).expect("remove test database");
}
