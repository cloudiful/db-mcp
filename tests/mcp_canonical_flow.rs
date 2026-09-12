//! Canonical-profile flow test: start `dbmcp` with the default profile over
//! stdio and exercise the full four-tool canonical contract
//! (`db_connections`, `db_introspect`, `db_query`, `db_explain`) against a
//! read-only SQLite fixture.

use rmcp::{
    ServiceExt,
    model::{CallToolRequestParams, ClientCapabilities, ClientInfo, Implementation},
    transport::{ConfigureCommandExt, TokioChildProcess},
};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::process::Command;

const CANONICAL_TOOLS: &[&str] = &["db_connections", "db_introspect", "db_query", "db_explain"];

static DB_COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_db_path(label: &str) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    let sequence = DB_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "dbmcp-canonical-flow-{label}-{}-{sequence}-{suffix}.db",
        std::process::id()
    ))
}

async fn read_only_sqlite_dsn(label: &str) -> (PathBuf, String) {
    let path = temp_db_path(label);
    let writable_dsn = format!("sqlite://{}?mode=rwc", path.display());
    let pool = sqlx::SqlitePool::connect(&writable_dsn)
        .await
        .expect("create sqlite database");
    sqlx::query("create table probe (value integer)")
        .execute(&pool)
        .await
        .expect("initialize sqlite database");
    sqlx::query("insert into probe (value) values (42)")
        .execute(&pool)
        .await
        .expect("seed sqlite database");
    pool.close().await;
    let read_only_dsn = format!("sqlite://{}?mode=ro", path.display());
    (path, read_only_dsn)
}

fn client_info() -> ClientInfo {
    ClientInfo::new(
        ClientCapabilities::default(),
        Implementation::new("dbmcp-canonical-flow-test", "0.0.1"),
    )
}

fn call(name: &str, arguments: Value) -> CallToolRequestParams {
    serde_json::from_value(json!({ "name": name, "arguments": arguments }))
        .expect("valid tool call params")
}

fn structured(result: &rmcp::model::CallToolResult) -> Value {
    result
        .structured_content
        .clone()
        .expect("structured content")
}

fn visible(result: &rmcp::model::CallToolResult) -> Value {
    let text = result
        .content
        .first()
        .expect("text content")
        .as_text()
        .expect("text content block");
    serde_json::from_str(&text.text).expect("visible text is JSON")
}

#[tokio::test]
async fn canonical_profile_exposes_four_tools_and_runs_full_flow() {
    let (path, dsn) = read_only_sqlite_dsn("flow").await;
    let command = Command::new(env!("CARGO_BIN_EXE_dbmcp")).configure(|cmd| {
        cmd.env("DBMCP_TRANSPORT", "stdio")
            .env("DBMCP_SQLITE_DSN", &dsn)
            .env("DBMCP_SQLITE_NAME", "sqlite");
    });
    let transport = TokioChildProcess::new(command).expect("spawn stdio server");
    let client = client_info()
        .serve(transport)
        .await
        .expect("stdio client initialize");

    let tools = client.list_all_tools().await.expect("list tools");
    let names: Vec<String> = tools.iter().map(|tool| tool.name.to_string()).collect();
    assert_eq!(
        names.len(),
        4,
        "default profile must expose 4 tools: {names:?}"
    );
    for expected in CANONICAL_TOOLS {
        assert!(
            names.contains(&expected.to_string()),
            "missing {expected}: {names:?}"
        );
    }

    // db_connections: list and diagnose.
    let list = client
        .call_tool(call("db_connections", json!({ "action": "list" })))
        .await
        .expect("db_connections list");
    assert_eq!(list.is_error, Some(false));
    let list = structured(&list);
    assert_eq!(list["kind"], "list");
    assert_eq!(list["data"]["connections"][0]["name"], "sqlite");

    let diagnose = client
        .call_tool(call(
            "db_connections",
            json!({ "action": "diagnose", "connection": "sqlite" }),
        ))
        .await
        .expect("db_connections diagnose");
    assert_eq!(diagnose.is_error, Some(false));
    let diagnose = structured(&diagnose);
    assert_eq!(diagnose["kind"], "diagnose");
    assert_eq!(diagnose["data"]["diagnostics"].as_array().unwrap().len(), 1);

    // db_introspect: every action maps to its typed kind.
    for (action, kind) in [
        ("schemas", "schemas"),
        ("tables", "tables"),
        ("columns", "columns"),
        ("constraints", "constraints"),
        ("indexes", "indexes"),
        ("comments", "comments"),
        ("ddl", "ddl"),
    ] {
        let result = client
            .call_tool(call(
                "db_introspect",
                json!({ "action": action, "connection": "sqlite", "table": "probe" }),
            ))
            .await
            .unwrap_or_else(|error| panic!("db_introspect {action} failed: {error:#}"));
        assert_eq!(result.is_error, Some(false), "db_introspect {action}");
        let value = structured(&result);
        assert_eq!(value["kind"], kind, "db_introspect {action}");
        assert_eq!(
            value["data"]["connection"], "sqlite",
            "db_introspect {action}"
        );
    }

    // db_query: single and batch on the read-only connection.
    let single = client
        .call_tool(call(
            "db_query",
            json!({ "connection": "sqlite", "sql": "select 1 as value" }),
        ))
        .await
        .expect("db_query single");
    assert_eq!(single.is_error, Some(false));
    let single_value = structured(&single);
    assert_eq!(single_value["kind"], "single");
    assert_eq!(single_value["data"]["result"]["rows"][0]["value"], json!(1));
    assert_eq!(
        visible(&single),
        single_value,
        "visible text must match structured content"
    );

    let batch = client
        .call_tool(call(
            "db_query",
            json!({
                "connection": "sqlite",
                "queries": [{ "sql": "select 1 as value" }, { "sql": "select 2 as value" }]
            }),
        ))
        .await
        .expect("db_query batch");
    assert_eq!(batch.is_error, Some(false));
    let batch_value = structured(&batch);
    assert_eq!(batch_value["kind"], "batch");
    assert_eq!(batch_value["data"]["status"], "completed");
    assert_eq!(batch_value["data"]["results"].as_array().unwrap().len(), 2);

    // db_explain.
    let explain = client
        .call_tool(call(
            "db_explain",
            json!({ "connection": "sqlite", "sql": "select * from probe" }),
        ))
        .await
        .expect("db_explain");
    assert_eq!(explain.is_error, Some(false));
    let explain = structured(&explain);
    assert_eq!(explain["connection"], "sqlite");
    assert!(
        explain["explain"]["planLines"]
            .as_array()
            .is_some_and(|lines| !lines.is_empty()),
        "explain must return plan lines: {explain}"
    );

    drop(client);
    std::fs::remove_file(path).expect("remove test database");
}
