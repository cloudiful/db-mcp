use db_mcp::{
    config::{Config, ConnectionConfig, LimitsConfig, OracleConfig, ServerConfig},
    database::{Cancellation, Manager, cancellation_report},
};

fn oracle_config() -> Option<Config> {
    let jdbc_url = std::env::var("DBMCP_TEST_ORACLE_JDBC_URL").ok()?;
    let username = std::env::var("DBMCP_TEST_ORACLE_USERNAME").ok()?;
    let password = std::env::var("DBMCP_TEST_ORACLE_PASSWORD").ok()?;
    Some(oracle_config_with(jdbc_url, username, password))
}

fn oracle_config_with(jdbc_url: String, username: String, password: String) -> Config {
    Config {
        server: ServerConfig::default(),
        limits: LimitsConfig::default(),
        connections: vec![ConnectionConfig {
            name: "oracle".to_string(),
            driver: "oracle".to_string(),
            oracle: Some(OracleConfig {
                jdbc_url,
                username,
                password,
                read_only: true,
            }),
            postgres: None,
            sqlite: None,
        }],
    }
}

fn oracle_short_timeout_config() -> Option<Config> {
    let jdbc_url = std::env::var("DBMCP_TEST_ORACLE_JDBC_URL").ok()?;
    let username = std::env::var("DBMCP_TEST_ORACLE_USERNAME").ok()?;
    let password = std::env::var("DBMCP_TEST_ORACLE_PASSWORD").ok()?;
    let mut cfg = oracle_config_with(jdbc_url, username, password);
    cfg.limits.query_timeout = "1s".to_string();
    Some(cfg)
}

#[tokio::test]
async fn oracle_can_probe_and_query_when_configured() {
    let Some(cfg) = oracle_config() else {
        eprintln!("skipping oracle integration test; DBMCP_TEST_ORACLE_* not set");
        return;
    };
    let manager = Manager::new(&cfg).await.expect("manager init");
    let conn = manager.connection(Some("oracle")).expect("connection");
    assert!(conn.supports_read_only_batch());

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
        .execute_query("select\n1 as value from dual;", 10, manager.limits())
        .await
        .expect("execute_query should work");
    assert_eq!(result.row_count, 1);

    let result = conn
        .execute_query("select 1 as value from dual;", 10, manager.limits())
        .await
        .expect("execute_query should tolerate trailing semicolons");
    assert_eq!(result.row_count, 1);
}

/// Requires a reachable Oracle service. `oracle-rs` 0.1.7 exposes no public
/// BREAK/INTERRUPT, so a deadline expiry must be reported as an *unconfirmed*
/// cancellation after the dedicated session is hard-closed, never as a
/// confirmed stop. The heavy cross join is expected to outlive the one-second
/// budget on any real instance.
#[tokio::test]
async fn oracle_timeout_reports_unconfirmed_cancellation_when_configured() {
    let Some(cfg) = oracle_short_timeout_config() else {
        eprintln!("skipping oracle timeout test; DBMCP_TEST_ORACLE_* not set");
        return;
    };
    let manager = Manager::new(&cfg).await.expect("manager init");
    let conn = manager.connection(Some("oracle")).expect("connection");

    let err = conn
        .execute_query(
            "select count(*) from all_objects a, all_objects b",
            10,
            manager.limits(),
        )
        .await
        .expect_err("heavy oracle query should exceed the deadline");

    let report = cancellation_report(&err).expect("oracle timeout should record a cancellation");
    assert_eq!(
        report.cancellation(),
        Cancellation::Unconfirmed,
        "the driver exposes no confirmed interrupt: {err:#}"
    );
    assert!(
        format!("{err:#}").contains("unconfirmed"),
        "unexpected error: {err:#}"
    );
}
