//! End-to-end flow tests: drive the real `dbmcp` binary with an rmcp client
//! over both stdio and streamable HTTP transports. The default server profile
//! is the canonical four-tool surface, so these tests only reference the
//! canonical names and never the legacy thirteen-tool list.

use rmcp::{
    ServiceExt,
    model::{CallToolRequestParams, ClientCapabilities, ClientInfo, Implementation},
    transport::{ConfigureCommandExt, StreamableHttpClientTransport, TokioChildProcess},
};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
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
        "dbmcp-flow-{label}-{}-{sequence}-{suffix}.db",
        std::process::id()
    ))
}

/// Create a temp SQLite database with one table and return a read-only DSN.
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
        Implementation::new("dbmcp-flow-test", "0.0.1"),
    )
}

fn call(name: &str, arguments: Value) -> CallToolRequestParams {
    serde_json::from_value(json!({
        "name": name,
        "arguments": arguments,
    }))
    .expect("valid tool call params")
}

fn tool_names(tools: &[rmcp::model::Tool]) -> Vec<String> {
    tools.iter().map(|tool| tool.name.to_string()).collect()
}

fn assert_canonical_surface(names: &[String]) {
    assert_eq!(
        names.len(),
        CANONICAL_TOOLS.len(),
        "expected canonical 4 tools, got {names:?}"
    );
    for expected in CANONICAL_TOOLS {
        assert!(
            names.iter().any(|name| name == expected),
            "missing canonical tool {expected}: {names:?}"
        );
    }
}

fn visible_json(result: &rmcp::model::CallToolResult) -> Value {
    let visible = result
        .content
        .first()
        .expect("text content")
        .as_text()
        .unwrap();
    serde_json::from_str(&visible.text).expect("visible text is JSON")
}

#[tokio::test]
async fn stdio_flow_lists_canonical_tools_and_runs_queries() {
    let (path, dsn) = read_only_sqlite_dsn("stdio").await;
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
    assert_canonical_surface(&tool_names(&tools));

    let result = client
        .call_tool(call(
            "db_query",
            json!({ "connection": "sqlite", "sql": "select 1 as value" }),
        ))
        .await
        .expect("call db_query");
    assert_eq!(result.is_error, Some(false));
    let structured = result
        .structured_content
        .clone()
        .expect("structured content");
    assert_eq!(structured["kind"], "single");
    assert_eq!(structured["data"]["connection"], "sqlite");
    assert_eq!(structured["data"]["result"]["rows"][0]["value"], json!(1));
    assert_eq!(
        visible_json(&result),
        structured,
        "visible text must match structured content"
    );

    let result = client
        .call_tool(call("db_connections", json!({ "action": "list" })))
        .await
        .expect("call db_connections");
    assert_eq!(result.is_error, Some(false));
    let structured = result.structured_content.expect("structured content");
    assert_eq!(structured["kind"], "list");
    let connections = structured["data"]["connections"]
        .as_array()
        .expect("connections array");
    assert!(
        connections.iter().any(|conn| conn["name"] == "sqlite"),
        "connections should include sqlite"
    );

    drop(client);
    std::fs::remove_file(path).expect("remove test database");
}

#[tokio::test]
async fn stdio_flow_surfaces_tool_and_protocol_errors() {
    let (path, dsn) = read_only_sqlite_dsn("stdio-err").await;
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

    // Unknown connection is a structured tool-level error carried in the result.
    let result = client
        .call_tool(call(
            "db_query",
            json!({ "connection": "missing-conn", "sql": "select 1" }),
        ))
        .await
        .expect("call with unknown connection");
    assert_eq!(result.is_error, Some(true));
    let value = visible_json(&result);
    assert_eq!(value["code"], "missing_connection");
    assert_eq!(value["kind"], "missing_connection");
    assert_eq!(value["connection"], "missing-conn");
    assert!(
        value["message"]
            .as_str()
            .is_some_and(|message| message.contains("missing-conn")),
        "unexpected error: {value}"
    );
    assert!(value.get("timeout").is_none());
    assert_eq!(
        value,
        result.structured_content.clone().expect("structured error"),
        "visible error text must match structured content"
    );

    // A batch failure keeps the failing index and the stable error fields.
    let result = client
        .call_tool(call(
            "db_query",
            json!({
                "connection": "sqlite",
                "queries": [
                    { "sql": "select 1 as value" },
                    { "sql": "select * from missing_flow_table" }
                ]
            }),
        ))
        .await
        .expect("call batch with a failing statement");
    assert_eq!(
        result.is_error,
        Some(false),
        "a partially successful batch is result data"
    );
    let batch = result.structured_content.expect("structured batch content");
    assert_eq!(batch["kind"], "batch");
    assert_eq!(batch["data"]["status"], "failed");
    let batch_error = &batch["data"]["error"];
    assert_eq!(batch_error["index"], 1);
    assert_eq!(batch_error["code"], "backend_error");
    assert_eq!(batch_error["kind"], "backend");
    assert!(
        batch_error["message"]
            .as_str()
            .is_some_and(|message| message.contains("missing_flow_table")),
        "unexpected batch error: {batch_error}"
    );

    // Malformed arguments are a protocol-level error.
    let error = client
        .call_tool(call("db_introspect", json!({})))
        .await
        .expect_err("missing required argument should fail");
    let error = format!("{error:#}");
    assert!(
        error.contains("-32602") || error.contains("missing field"),
        "unexpected error: {error}"
    );

    drop(client);
    std::fs::remove_file(path).expect("remove test database");
}

async fn free_port() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let port = listener.local_addr().expect("local address").port();
    drop(listener);
    port
}

/// Kills and reaps the HTTP server on drop. `TokioChildProcess` schedules its
/// kill on the current tokio runtime, which may already be shutting down when a
/// test returns, so an explicit guard keeps the child from leaking.
struct ServerChild(std::process::Child);

impl Drop for ServerChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

async fn spawn_http_server(dsn: &str) -> (u16, ServerChild) {
    let port = free_port().await;
    let child = std::process::Command::new(env!("CARGO_BIN_EXE_dbmcp"))
        .env("DBMCP_TRANSPORT", "http")
        .env("DBMCP_HTTP_ADDR", format!("127.0.0.1:{port}"))
        .env("DBMCP_SQLITE_DSN", dsn)
        .env("DBMCP_SQLITE_NAME", "sqlite")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn http server");
    (port, ServerChild(child))
}

#[tokio::test]
async fn http_flow_lists_canonical_tools_and_runs_queries() {
    let (path, dsn) = read_only_sqlite_dsn("http").await;
    let (port, child) = spawn_http_server(&dsn).await;
    let uri = format!("http://127.0.0.1:{port}/mcp");
    let mut client = None;
    for attempt in 0..20 {
        let transport = StreamableHttpClientTransport::from_uri(uri.as_str());
        match client_info().serve(transport).await {
            Ok(service) => {
                client = Some(service);
                break;
            }
            Err(error) => {
                assert!(attempt < 19, "http client initialize failed: {error:#}");
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    }
    let client = client.expect("http client");

    let tools = client.list_all_tools().await.expect("list tools");
    assert_canonical_surface(&tool_names(&tools));

    let result = client
        .call_tool(call(
            "db_query",
            json!({ "connection": "sqlite", "sql": "select value from probe" }),
        ))
        .await
        .expect("call db_query");
    assert_eq!(result.is_error, Some(false));
    let structured = result.structured_content.expect("structured content");
    assert_eq!(structured["kind"], "single");
    assert_eq!(structured["data"]["result"]["rows"][0]["value"], json!(42));

    drop(client);
    drop(child);
    std::fs::remove_file(path).expect("remove test database");
}
