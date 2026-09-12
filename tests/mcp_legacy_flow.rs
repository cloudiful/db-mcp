//! Legacy and both profile flow tests. `DBMCP_TOOL_PROFILE` is fixed at server
//! startup, so each profile needs its own child process.

use rmcp::{
    ServiceExt,
    model::{CallToolRequestParams, ClientCapabilities, ClientInfo, Implementation},
    transport::{ConfigureCommandExt, TokioChildProcess},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::process::Command;

const CANONICAL_TOOLS: &[&str] = &["db_connections", "db_introspect", "db_query", "db_explain"];
const LEGACY_TOOLS: &[&str] = &[
    "list_connections",
    "diagnose_connection",
    "list_schemas",
    "list_tables",
    "search_columns",
    "list_indexes",
    "list_constraints",
    "get_table_comments",
    "describe_table",
    "get_table_ddl",
    "execute_query",
    "execute_queries",
    "explain_query",
];

static DB_COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_db_path(label: &str) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    let sequence = DB_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "dbmcp-legacy-flow-{label}-{}-{sequence}-{suffix}.db",
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
        Implementation::new("dbmcp-legacy-flow-test", "0.0.1"),
    )
}

async fn connect(
    dsn: &str,
    profile: &str,
) -> rmcp::service::RunningService<rmcp::RoleClient, ClientInfo> {
    let command = Command::new(env!("CARGO_BIN_EXE_dbmcp")).configure(|cmd| {
        cmd.env("DBMCP_TRANSPORT", "stdio")
            .env("DBMCP_TOOL_PROFILE", profile)
            .env("DBMCP_SQLITE_DSN", dsn)
            .env("DBMCP_SQLITE_NAME", "sqlite");
    });
    let transport = TokioChildProcess::new(command).expect("spawn stdio server");
    client_info()
        .serve(transport)
        .await
        .expect("stdio client initialize")
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

#[tokio::test]
async fn legacy_profile_exposes_and_runs_all_thirteen_tools() {
    let (path, dsn) = read_only_sqlite_dsn("legacy").await;
    let client = connect(&dsn, "legacy").await;

    let tools = client.list_all_tools().await.expect("list tools");
    let names: Vec<String> = tools.iter().map(|tool| tool.name.to_string()).collect();
    assert_eq!(
        names.len(),
        13,
        "legacy profile must expose 13 tools: {names:?}"
    );
    let unique: BTreeSet<&String> = names.iter().collect();
    assert_eq!(unique.len(), 13, "legacy tool names must be unique");
    for expected in LEGACY_TOOLS {
        assert!(names.contains(&expected.to_string()), "missing {expected}");
    }

    // Every legacy tool must remain callable on a read-only SQLite connection.
    let calls = [
        ("list_connections", json!({})),
        ("diagnose_connection", json!({ "connection": "sqlite" })),
        ("list_schemas", json!({ "connection": "sqlite" })),
        ("list_tables", json!({ "connection": "sqlite" })),
        (
            "search_columns",
            json!({ "connection": "sqlite", "filter": "value" }),
        ),
        (
            "list_indexes",
            json!({ "connection": "sqlite", "table": "probe" }),
        ),
        (
            "list_constraints",
            json!({ "connection": "sqlite", "table": "probe" }),
        ),
        (
            "get_table_comments",
            json!({ "connection": "sqlite", "table": "probe" }),
        ),
        (
            "describe_table",
            json!({ "connection": "sqlite", "table": "probe" }),
        ),
        (
            "get_table_ddl",
            json!({ "connection": "sqlite", "table": "probe" }),
        ),
        (
            "execute_query",
            json!({ "connection": "sqlite", "sql": "select 1 as value" }),
        ),
        (
            "execute_queries",
            json!({ "connection": "sqlite", "queries": [{ "sql": "select 1 as value" }] }),
        ),
        (
            "explain_query",
            json!({ "connection": "sqlite", "sql": "select * from probe" }),
        ),
    ];
    for (name, arguments) in calls {
        let result = client
            .call_tool(call(name, arguments))
            .await
            .unwrap_or_else(|error| panic!("{name} failed: {error:#}"));
        assert_eq!(result.is_error, Some(false), "{name} returned an error");
        assert!(!structured(&result).is_null(), "{name} has no payload");
    }

    let query = client
        .call_tool(call(
            "execute_query",
            json!({ "connection": "sqlite", "sql": "select 7 as value" }),
        ))
        .await
        .expect("execute_query");
    assert_eq!(structured(&query)["result"]["rows"][0]["value"], json!(7));

    drop(client);
    std::fs::remove_file(path).expect("remove test database");
}

#[tokio::test]
async fn both_profile_exposes_seventeen_unique_tools() {
    let (path, dsn) = read_only_sqlite_dsn("both").await;
    let client = connect(&dsn, "both").await;

    let tools = client.list_all_tools().await.expect("list tools");
    let names: Vec<String> = tools.iter().map(|tool| tool.name.to_string()).collect();
    assert_eq!(
        names.len(),
        17,
        "both profile must expose 17 tools: {names:?}"
    );
    let unique: BTreeSet<&String> = names.iter().collect();
    assert_eq!(unique.len(), 17, "both profile must not repeat names");
    for expected in CANONICAL_TOOLS.iter().chain(LEGACY_TOOLS) {
        assert!(names.contains(&expected.to_string()), "missing {expected}");
    }

    // A canonical and a legacy name are both callable under `both`.
    let canonical = client
        .call_tool(call("db_connections", json!({ "action": "list" })))
        .await
        .expect("db_connections");
    assert_eq!(canonical.is_error, Some(false));
    let legacy = client
        .call_tool(call("list_connections", json!({ "connection": "sqlite" })))
        .await
        .expect("list_connections");
    assert_eq!(legacy.is_error, Some(false));

    drop(client);
    std::fs::remove_file(path).expect("remove test database");
}
