use db_mcp::{
    config::{Config, ConnectionConfig, LimitsConfig, PostgresConfig, ServerConfig},
    database::Manager,
};

fn postgres_dsn() -> Option<String> {
    std::env::var("DBMCP_TEST_POSTGRES_DSN").ok()
}

fn test_config() -> Option<Config> {
    let dsn = postgres_dsn()?;
    Some(Config {
        server: ServerConfig::default(),
        limits: LimitsConfig::default(),
        connections: vec![ConnectionConfig {
            name: "postgres".to_string(),
            driver: "postgres".to_string(),
            postgres: Some(PostgresConfig {
                dsn,
                schema: "public".to_string(),
                read_only: true,
            }),
            oracle: None,
            sqlite: None,
        }],
    })
}

fn test_config_without_schema() -> Option<Config> {
    let mut cfg = test_config()?;
    cfg.connections[0].postgres.as_mut().unwrap().schema.clear();
    Some(cfg)
}

fn write_enabled_test_config() -> Option<Config> {
    let mut cfg = test_config()?;
    cfg.connections[0].postgres.as_mut().unwrap().read_only = false;
    Some(cfg)
}

fn short_timeout_test_config() -> Option<Config> {
    let mut cfg = test_config()?;
    cfg.limits.query_timeout = "1s".to_string();
    Some(cfg)
}

const SKIP_MESSAGE: &str = "skipping postgres integration test; DBMCP_TEST_POSTGRES_DSN not set";

#[tokio::test]
async fn postgres_manager_health_is_up() {
    let Some(cfg) = test_config() else {
        eprintln!("{SKIP_MESSAGE}");
        return;
    };
    let manager = Manager::new(&cfg).await.expect("manager init");
    let health = manager.health(None, false).await.expect("health");

    assert_eq!(health.connections.len(), 1);
    assert!(
        health.connections[0].available,
        "postgres should be reachable"
    );
    assert_eq!(health.connections[0].driver, "postgres");
}

#[tokio::test]
async fn postgres_manager_health_is_up_without_configured_schema() {
    let Some(cfg) = test_config_without_schema() else {
        eprintln!("{SKIP_MESSAGE}");
        return;
    };
    let manager = Manager::new(&cfg).await.expect("manager init");
    let health = manager.health(None, false).await.expect("health");

    assert!(
        health.connections[0].available,
        "postgres should refresh current_schema()::text"
    );
    assert_eq!(health.connections[0].default_schema, "public");
}

#[tokio::test]
async fn postgres_diagnostics_include_probe_checks() {
    let Some(cfg) = test_config() else {
        eprintln!("{SKIP_MESSAGE}");
        return;
    };
    let manager = Manager::new(&cfg).await.expect("manager init");
    let diagnostics = manager
        .diagnose(Some("postgres"), true)
        .await
        .expect("diagnostics");

    assert_eq!(diagnostics.len(), 1);
    assert!(diagnostics[0].available);
    assert!(
        diagnostics[0]
            .checks
            .iter()
            .any(|check| check.phase == "metadataSmokeTest")
    );
}

#[tokio::test]
async fn postgres_can_list_schemas_and_run_select() {
    let Some(cfg) = test_config() else {
        eprintln!("{SKIP_MESSAGE}");
        return;
    };
    let manager = Manager::new(&cfg).await.expect("manager init");
    let conn = manager.connection(Some("postgres")).expect("connection");

    let schemas = conn
        .list_schemas(Some("pub"), 10)
        .await
        .expect("list_schemas");
    assert!(
        schemas.iter().any(|schema| schema.name == "public"),
        "public schema should be visible"
    );

    let result = conn
        .execute_query("select 1 as value", 10, manager.limits())
        .await
        .expect("execute_query");
    assert_eq!(result.row_count, 1);
    assert_eq!(result.rows.len(), 1);
}

#[tokio::test]
async fn postgres_execute_query_accepts_trailing_semicolon() {
    let Some(cfg) = test_config() else {
        eprintln!("{SKIP_MESSAGE}");
        return;
    };
    let manager = Manager::new(&cfg).await.expect("manager init");
    let conn = manager.connection(Some("postgres")).expect("connection");

    let result = conn
        .execute_query("select 1 as value;", 10, manager.limits())
        .await
        .expect("execute_query should tolerate trailing semicolons");

    assert_eq!(result.row_count, 1);
    assert_eq!(
        result.rows[0].get("value").and_then(|value| value.as_i64()),
        Some(1)
    );
}

#[tokio::test]
async fn postgres_execute_query_accepts_trailing_comment() {
    let Some(cfg) = test_config() else {
        eprintln!("{SKIP_MESSAGE}");
        return;
    };
    let manager = Manager::new(&cfg).await.expect("manager init");
    let conn = manager.connection(Some("postgres")).expect("connection");

    let result = conn
        .execute_query(
            "select 1 as value -- trailing comment",
            10,
            manager.limits(),
        )
        .await
        .expect("execute_query should tolerate a trailing comment");

    assert_eq!(result.row_count, 1);
    assert_eq!(
        result.rows[0].get("value").and_then(|value| value.as_i64()),
        Some(1)
    );
}

#[tokio::test]
async fn postgres_empty_result_set_keeps_column_names() {
    let Some(cfg) = test_config() else {
        eprintln!("{SKIP_MESSAGE}");
        return;
    };
    let manager = Manager::new(&cfg).await.expect("manager init");
    let conn = manager.connection(Some("postgres")).expect("connection");

    let result = conn
        .execute_query("select 1 as value where false", 10, manager.limits())
        .await
        .expect("execute_query");

    assert_eq!(result.row_count, 0);
    assert_eq!(result.columns.len(), 1);
    assert_eq!(result.columns[0].name, "value");
}

#[tokio::test]
async fn postgres_read_only_connection_supports_sequential_batch_queries() {
    let Some(cfg) = test_config() else {
        eprintln!("{SKIP_MESSAGE}");
        return;
    };
    let manager = Manager::new(&cfg).await.expect("manager init");
    let conn = manager.connection(Some("postgres")).expect("connection");

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
}

#[tokio::test]
async fn postgres_explain_query_accepts_trailing_semicolon() {
    let Some(cfg) = test_config() else {
        eprintln!("{SKIP_MESSAGE}");
        return;
    };
    let manager = Manager::new(&cfg).await.expect("manager init");
    let conn = manager.connection(Some("postgres")).expect("connection");

    let result = conn
        .explain_query("select 1 as value;", manager.limits())
        .await
        .expect("explain_query should tolerate trailing semicolons");

    assert!(!result.plan_lines.is_empty(), "plan should not be empty");
}

#[tokio::test]
async fn postgres_execute_query_serializes_uuid_columns() {
    let Some(cfg) = test_config() else {
        eprintln!("{SKIP_MESSAGE}");
        return;
    };
    let manager = Manager::new(&cfg).await.expect("manager init");
    let conn = manager.connection(Some("postgres")).expect("connection");

    let result = conn
        .execute_query(
            "select '00000000-0000-0000-0000-000000000001'::uuid as request_id",
            10,
            manager.limits(),
        )
        .await
        .expect("execute_query should serialize uuid columns");

    assert_eq!(result.row_count, 1);
    assert_eq!(
        result.rows[0]
            .get("request_id")
            .and_then(|value| value.as_str()),
        Some("00000000-0000-0000-0000-000000000001")
    );
}

#[tokio::test]
async fn postgres_read_only_transactions_reject_writes() {
    let Some(cfg) = test_config() else {
        eprintln!("{SKIP_MESSAGE}");
        return;
    };
    let manager = Manager::new(&cfg).await.expect("manager init");
    let conn = manager.connection(Some("postgres")).expect("connection");

    let err = conn
        .execute_query(
            "create temporary table __dbmcp_ro_test(id int)",
            10,
            manager.limits(),
        )
        .await
        .expect_err("read-only transaction should reject writes");

    let message = format!("{err:#}");
    assert!(
        message.contains("read-only")
            || message.contains("read only")
            || message.contains("cannot execute"),
        "unexpected error: {message}"
    );
}

#[tokio::test]
async fn postgres_timeout_is_bounded_and_the_pool_recovers() {
    let Some(cfg) = short_timeout_test_config() else {
        eprintln!("{SKIP_MESSAGE}");
        return;
    };
    let manager = Manager::new(&cfg).await.expect("manager init");
    let conn = manager.connection(Some("postgres")).expect("connection");

    let err = conn
        .execute_query("select pg_sleep(5)", 10, manager.limits())
        .await
        .expect_err("pg_sleep should exceed the logical deadline");
    assert!(
        format!("{err:#}").contains("timeout"),
        "unexpected error: {err:#}"
    );

    // The timed-out transaction is never committed; the unconfirmed in-flight
    // connection is evicted instead of being returned to the pool.
    let after = conn
        .execute_query("select 1 as value", 10, manager.limits())
        .await
        .expect("pool should recover with a fresh connection");
    assert_eq!(
        after.rows[0].get("value").and_then(|value| value.as_i64()),
        Some(1)
    );
}

#[tokio::test]
async fn postgres_can_disable_read_only_transactions() {
    let Some(cfg) = write_enabled_test_config() else {
        eprintln!("{SKIP_MESSAGE}");
        return;
    };
    let manager = Manager::new(&cfg).await.expect("manager init");
    let conn = manager.connection(Some("postgres")).expect("connection");

    conn.execute_query(
        "create temporary table __dbmcp_rw_test(id int)",
        10,
        manager.limits(),
    )
    .await
    .expect("write-enabled config should allow temp table creation");
}
