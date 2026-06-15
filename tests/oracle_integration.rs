use db_mcp::{
    config::{Config, ConnectionConfig, LimitsConfig, OracleConfig, ServerConfig},
    database::Manager,
};

fn oracle_config() -> Option<Config> {
    let jdbc_url = std::env::var("DBMCP_TEST_ORACLE_JDBC_URL").ok()?;
    let username = std::env::var("DBMCP_TEST_ORACLE_USERNAME").ok()?;
    let password = std::env::var("DBMCP_TEST_ORACLE_PASSWORD").ok()?;
    Some(Config {
        server: ServerConfig::default(),
        limits: LimitsConfig::default(),
        connections: vec![ConnectionConfig {
            name: "oracle".to_string(),
            driver: "oracle".to_string(),
            oracle: Some(OracleConfig {
                jdbc_url,
                username,
                password,
            }),
            postgres: None,
            sqlite: None,
        }],
    })
}

#[tokio::test]
async fn oracle_can_probe_and_query_when_configured() {
    let Some(cfg) = oracle_config() else {
        eprintln!("skipping oracle integration test; DBMCP_TEST_ORACLE_* not set");
        return;
    };
    let manager = Manager::new(&cfg).await.expect("manager init");
    let conn = manager.connection(Some("oracle")).expect("connection");

    let health = manager.health(Some("oracle"), true).await.expect("health");
    assert!(
        health.connections[0].available,
        "oracle should be reachable"
    );

    let schemas = conn
        .list_schemas(None, 5)
        .await
        .expect("list_schemas should work");
    assert!(!schemas.is_empty());

    let result = conn
        .execute_query("select 1 as value from dual", 10, manager.limits())
        .await
        .expect("execute_query should work");
    assert_eq!(result.row_count, 1);
}
