use db_mcp::{
    config::{Config, ConnectionConfig, LimitsConfig, PostgresConfig, ServerConfig},
    database::Manager,
};

fn postgres_dsn() -> String {
    std::env::var("DBMCP_TEST_POSTGRES_DSN").unwrap_or_else(|_| {
        "postgres://postgres:wEMSQSDn4sVGEZT3KSv6@localhost:5432/postgres".to_string()
    })
}

fn test_config() -> Config {
    Config {
        server: ServerConfig::default(),
        limits: LimitsConfig::default(),
        connections: vec![ConnectionConfig {
            name: "postgres".to_string(),
            driver: "postgres".to_string(),
            postgres: Some(PostgresConfig {
                dsn: postgres_dsn(),
                schema: "public".to_string(),
            }),
            oracle: None,
            sqlite: None,
        }],
    }
}

fn test_config_without_schema() -> Config {
    let mut cfg = test_config();
    cfg.connections[0].postgres.as_mut().unwrap().schema.clear();
    cfg
}

#[tokio::test]
async fn postgres_manager_health_is_up() {
    let cfg = test_config();
    let manager = Manager::new(&cfg).await.expect("manager init");
    let health = manager.health(None, true).await.expect("health");

    assert_eq!(health.default_connection, "postgres");
    assert_eq!(health.connections.len(), 1);
    assert!(
        health.connections[0].available,
        "postgres should be reachable"
    );
    assert_eq!(health.connections[0].driver, "postgres");
}

#[tokio::test]
async fn postgres_manager_health_is_up_without_configured_schema() {
    let cfg = test_config_without_schema();
    let manager = Manager::new(&cfg).await.expect("manager init");
    let health = manager.health(None, true).await.expect("health");

    assert!(
        health.connections[0].available,
        "postgres should refresh current_schema()::text"
    );
    assert_eq!(health.connections[0].default_schema, "public");
}

#[tokio::test]
async fn postgres_diagnostics_include_probe_checks() {
    let cfg = test_config();
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
    let cfg = test_config();
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
