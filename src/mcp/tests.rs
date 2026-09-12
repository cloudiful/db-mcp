use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use rmcp::model::{CallToolResult, ErrorCode, ErrorData, Tool, ToolAnnotations};
use serde_json::{Map, Value, json};

use crate::config::{
    Config, ConnectionConfig, LimitsConfig, SQLiteConfig, ServerConfig, ToolProfile,
};
use crate::database;

use super::auth::constant_time_eq;
use super::{AppState, DbMcpService, RunToolResult};

const CANONICAL_TOOLS: &[&str] = &["db_connections", "db_introspect", "db_query", "db_explain"];
const LEGACY_READ_ONLY_TOOLS: &[&str] = &[
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
    "explain_query",
];
const LEGACY_QUERY_TOOLS: &[&str] = &["execute_query", "execute_queries"];

static DB_COUNTER: AtomicU64 = AtomicU64::new(0);

fn sqlite_config(profile: ToolProfile, dsn: String) -> Config {
    Config {
        server: ServerConfig {
            tool_profile: profile,
            ..ServerConfig::default()
        },
        limits: LimitsConfig::default(),
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

async fn service(profile: ToolProfile) -> DbMcpService {
    let cfg = sqlite_config(profile, "sqlite::memory:".to_string());
    let manager = database::Manager::new(&cfg).await.expect("manager");
    DbMcpService::new(Arc::new(AppState { cfg, manager }))
}

fn blocking_service(profile: ToolProfile) -> DbMcpService {
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    runtime.block_on(service(profile))
}

fn tool_names(service: &DbMcpService) -> Vec<String> {
    service
        .build_tools()
        .iter()
        .map(|tool| tool.name.to_string())
        .collect()
}

fn tool_by_name(service: &DbMcpService, name: &str) -> Tool {
    service
        .build_tools()
        .into_iter()
        .find(|tool| tool.name.as_ref() == name)
        .unwrap_or_else(|| panic!("tool {name} not found"))
}

fn annotations(tool: &Tool) -> ToolAnnotations {
    tool.annotations.clone().unwrap_or_default()
}

fn args(value: Value) -> Map<String, Value> {
    value.as_object().cloned().expect("object arguments")
}

fn expect_protocol(result: RunToolResult) -> ErrorData {
    match result {
        RunToolResult::Protocol(error) => error,
        other => panic!("expected protocol error, got {other:?}"),
    }
}

fn structured(result: &CallToolResult) -> Value {
    result
        .structured_content
        .clone()
        .expect("structured content")
}

async fn call(service: &DbMcpService, name: &str, payload: Value) -> CallToolResult {
    match service.dispatch_tool(name, Some(args(payload))).await {
        RunToolResult::Ok(result) => result,
        other => panic!("expected tool success, got {other:?}"),
    }
}

async fn protocol_error(service: &DbMcpService, name: &str, payload: Value) -> ErrorData {
    expect_protocol(service.dispatch_tool(name, Some(args(payload))).await)
}

async fn tool_error(service: &DbMcpService, name: &str, payload: Value) -> String {
    match service.dispatch_tool(name, Some(args(payload))).await {
        RunToolResult::Tool(error) => format!("{error:#}"),
        other => panic!("expected tool-level error, got {other:?}"),
    }
}

/// Every tool must carry a description, an object input schema with properties,
/// and a non-empty output schema.
fn assert_tool_contracts(tools: &[Tool]) {
    for tool in tools {
        assert!(
            tool.description.as_deref().is_some_and(|d| !d.is_empty()),
            "{} has no description",
            tool.name
        );
        assert_eq!(tool.input_schema["type"], "object", "{}", tool.name);
        assert!(
            tool.input_schema["properties"]
                .as_object()
                .is_some_and(|p| !p.is_empty()),
            "{} input schema has no properties",
            tool.name
        );
        let output = tool.output_schema.clone().expect("output schema");
        assert!(!output.is_empty(), "{} output schema is empty", tool.name);
    }
}

/// A canonical service backed by a file SQLite database opened with a read-only
/// DSN, matching the server-side gate canonical `db_query` enforces.
async fn read_only_service() -> (DbMcpService, PathBuf) {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    let sequence = DB_COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "dbmcp-canonical-{}-{sequence}-{suffix}.db",
        std::process::id()
    ));
    let writable_dsn = format!("sqlite://{}?mode=rwc", path.display());
    let pool = sqlx::SqlitePool::connect(&writable_dsn)
        .await
        .expect("create sqlite database");
    sqlx::query("create table if not exists probe (value integer)")
        .execute(&pool)
        .await
        .expect("initialize sqlite database");
    pool.close().await;

    let cfg = sqlite_config(
        ToolProfile::Canonical,
        format!("sqlite://{}?mode=ro", path.display()),
    );
    let manager = database::Manager::new(&cfg).await.expect("manager");
    (DbMcpService::new(Arc::new(AppState { cfg, manager })), path)
}

#[test]
fn profiles_expose_canonical_legacy_and_both_tools() {
    let canonical = tool_names(&blocking_service(ToolProfile::Canonical));
    let legacy = tool_names(&blocking_service(ToolProfile::Legacy));
    let both = tool_names(&blocking_service(ToolProfile::Both));

    assert_eq!(canonical.len(), 4);
    assert_eq!(legacy.len(), 13);
    assert_eq!(both.len(), 17);
    assert_eq!(both.iter().collect::<BTreeSet<_>>().len(), 17);

    for name in CANONICAL_TOOLS {
        assert!(canonical.contains(&name.to_string()), "missing {name}");
    }
    for name in LEGACY_READ_ONLY_TOOLS.iter().chain(LEGACY_QUERY_TOOLS) {
        assert!(legacy.contains(&name.to_string()), "missing {name}");
    }
    for name in CANONICAL_TOOLS
        .iter()
        .chain(LEGACY_READ_ONLY_TOOLS)
        .chain(LEGACY_QUERY_TOOLS)
    {
        assert!(both.contains(&name.to_string()), "missing {name}");
    }

    for profile in [
        ToolProfile::Canonical,
        ToolProfile::Legacy,
        ToolProfile::Both,
    ] {
        assert_tool_contracts(&blocking_service(profile).build_tools());
    }
}

#[test]
fn canonical_tools_are_read_only_annotated() {
    for tool in blocking_service(ToolProfile::Canonical).build_tools() {
        let annotations = annotations(&tool);
        assert_eq!(annotations.read_only_hint, Some(true), "{}", tool.name);
        assert_eq!(annotations.idempotent_hint, Some(true), "{}", tool.name);
        assert_eq!(annotations.open_world_hint, Some(false), "{}", tool.name);
        assert_eq!(annotations.destructive_hint, None, "{}", tool.name);
    }
}

#[test]
fn legacy_annotations_distinguish_metadata_and_query_tools() {
    let service = blocking_service(ToolProfile::Legacy);
    for name in LEGACY_READ_ONLY_TOOLS {
        let annotations = annotations(&tool_by_name(&service, name));
        assert_eq!(annotations.read_only_hint, Some(true), "{name}");
        assert_eq!(annotations.idempotent_hint, Some(true), "{name}");
        assert_eq!(annotations.open_world_hint, Some(false), "{name}");
        assert_eq!(annotations.destructive_hint, None, "{name}");
    }
    for name in LEGACY_QUERY_TOOLS {
        let annotations = annotations(&tool_by_name(&service, name));
        assert_eq!(annotations.read_only_hint, Some(false), "{name}");
        assert_eq!(annotations.destructive_hint, Some(true), "{name}");
        assert_eq!(annotations.idempotent_hint, Some(false), "{name}");
        assert_eq!(annotations.open_world_hint, Some(false), "{name}");
    }
}

#[test]
fn list_indexes_accepts_table_argument() {
    let input = tool_by_name(&blocking_service(ToolProfile::Legacy), "list_indexes").input_schema;
    let properties = input["properties"].as_object().expect("input properties");
    for key in ["table", "schema", "limit"] {
        assert!(
            properties.contains_key(key),
            "list_indexes must accept {key}"
        );
    }
}

#[test]
fn constant_time_eq_distinguishes_equal_and_unequal_bytes() {
    assert!(constant_time_eq(b"secret", b"secret"));
    assert!(constant_time_eq(b"", b""));
    assert!(!constant_time_eq(b"secret", b"secrex"));
    assert!(!constant_time_eq(b"secret", b"secret-longer"));
    assert!(!constant_time_eq(b"", b"a"));
    assert!(!constant_time_eq(b"Bearer x", b"Bearer y"));
}

#[tokio::test]
async fn malformed_unknown_and_disabled_tools_are_invalid_params() {
    let legacy = service(ToolProfile::Legacy).await;
    let error = expect_protocol(legacy.dispatch_tool("describe_table", None).await);
    assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
    assert!(!error.message.is_empty());

    let canonical = service(ToolProfile::Canonical).await;
    let error = expect_protocol(
        canonical
            .dispatch_tool("does_not_exist", Some(Map::new()))
            .await,
    );
    assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
    assert!(error.message.contains("does_not_exist"));

    let error = protocol_error(&legacy, "db_connections", json!({"action": "list"})).await;
    assert!(error.message.contains("db_connections"));
}

#[tokio::test]
async fn canonical_dispatch_rejects_invalid_parameters() {
    let service = service(ToolProfile::Canonical).await;
    let oversized: Vec<Value> = (0..9).map(|_| json!({"sql": "select 1"})).collect();
    let cases = [
        ("db_connections", json!({"action": "ping"})),
        ("db_introspect", json!({"action": "procedures"})),
        (
            "db_introspect",
            json!({"action": "columns", "connection": "sqlite"}),
        ),
        (
            "db_introspect",
            json!({"action": "comments", "connection": "sqlite"}),
        ),
        (
            "db_introspect",
            json!({"action": "ddl", "connection": "sqlite"}),
        ),
        (
            "db_query",
            json!({"connection": "sqlite", "sql": "select 1", "queries": [{"sql": "select 1"}]}),
        ),
        ("db_query", json!({"connection": "sqlite"})),
        ("db_query", json!({"connection": "sqlite", "queries": []})),
        (
            "db_query",
            json!({"connection": "sqlite", "queries": oversized}),
        ),
    ];
    for (name, payload) in cases {
        let error = protocol_error(&service, name, payload).await;
        assert_eq!(error.code, ErrorCode::INVALID_PARAMS, "{name}");
    }
}

#[tokio::test]
async fn canonical_db_query_rejects_non_read_only_connections_on_both_paths() {
    // `sqlite::memory:` has read_only=true config but a writable DSN, so the
    // driver reports it as not read-only and canonical db_query must refuse.
    let service = service(ToolProfile::Canonical).await;
    for payload in [
        json!({"connection": "sqlite", "sql": "select 1"}),
        json!({"connection": "sqlite", "queries": [{"sql": "select 1"}]}),
    ] {
        let message = tool_error(&service, "db_query", payload).await;
        assert!(message.contains("read-only"), "{message}");
    }
}

#[tokio::test]
async fn missing_connection_failure_is_tool_level_error() {
    let service = service(ToolProfile::Canonical).await;
    let message = tool_error(
        &service,
        "db_query",
        json!({"connection": "missing-conn", "sql": "select 1"}),
    )
    .await;
    assert!(message.contains("missing-conn"));
}

#[tokio::test]
async fn canonical_db_query_runs_single_and_batch_on_read_only_sqlite() {
    let (service, path) = read_only_service().await;

    let single = structured(
        &call(
            &service,
            "db_query",
            json!({"connection": "sqlite", "sql": "select 1 as value"}),
        )
        .await,
    );
    assert_eq!(single["kind"], "single");
    assert_eq!(single["data"]["connection"], "sqlite");
    assert_eq!(
        single["data"]["result"]["rows"][0]["value"].as_i64(),
        Some(1)
    );

    let batch = structured(
        &call(
            &service,
            "db_query",
            json!({
                "connection": "sqlite",
                "queries": [{"sql": "select 1 as value"}, {"sql": "select 2 as value"}]
            }),
        )
        .await,
    );
    assert_eq!(batch["kind"], "batch");
    assert_eq!(batch["data"]["status"], "completed");
    assert_eq!(
        batch["data"]["results"].as_array().expect("results").len(),
        2
    );

    std::fs::remove_file(path).expect("remove test database");
}

#[tokio::test]
async fn canonical_db_introspect_returns_typed_kinds() {
    let (service, path) = read_only_service().await;

    let tables = structured(
        &call(
            &service,
            "db_introspect",
            json!({"action": "tables", "connection": "sqlite"}),
        )
        .await,
    );
    assert_eq!(tables["kind"], "tables");
    assert!(
        tables["data"]["tables"]
            .as_array()
            .is_some_and(|tables| !tables.is_empty())
    );

    let columns = structured(
        &call(
            &service,
            "db_introspect",
            json!({"action": "columns", "connection": "sqlite", "table": "probe"}),
        )
        .await,
    );
    assert_eq!(columns["kind"], "columns");
    assert_eq!(columns["data"]["table"], "probe");
    assert!(
        columns["data"]["columns"]
            .as_array()
            .is_some_and(|columns| !columns.is_empty())
    );

    std::fs::remove_file(path).expect("remove test database");
}

#[tokio::test]
async fn canonical_db_connections_and_explain_round_trip() {
    let (service, path) = read_only_service().await;

    let list = structured(&call(&service, "db_connections", json!({"action": "list"})).await);
    assert_eq!(list["kind"], "list");
    assert_eq!(list["data"]["connections"][0]["name"], "sqlite");

    let diagnose = structured(
        &call(
            &service,
            "db_connections",
            json!({"action": "diagnose", "connection": "sqlite"}),
        )
        .await,
    );
    assert_eq!(diagnose["kind"], "diagnose");
    assert_eq!(
        diagnose["data"]["diagnostics"]
            .as_array()
            .expect("diagnostics")
            .len(),
        1
    );

    let explain = structured(
        &call(
            &service,
            "db_explain",
            json!({"connection": "sqlite", "sql": "select * from probe"}),
        )
        .await,
    );
    assert_eq!(explain["connection"], "sqlite");
    assert!(
        explain["explain"]["planLines"]
            .as_array()
            .is_some_and(|lines| !lines.is_empty())
    );

    std::fs::remove_file(path).expect("remove test database");
}

#[tokio::test]
async fn structured_success_is_visible_json_matching_structured_content() {
    let (service, path) = read_only_service().await;
    let result = call(
        &service,
        "db_query",
        json!({"connection": "sqlite", "sql": "select 1 as value"}),
    )
    .await;
    let visible = result
        .content
        .first()
        .and_then(|block| block.as_text())
        .map(|text| text.text.clone())
        .expect("visible text content");
    let visible_json: Value = serde_json::from_str(&visible).expect("visible text is JSON");
    assert_eq!(
        visible_json,
        result
            .structured_content
            .clone()
            .expect("structured content")
    );

    std::fs::remove_file(path).expect("remove test database");
}
