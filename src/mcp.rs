use anyhow::Result;
use axum::extract::State;
use rmcp::{
    ErrorData, RoleServer, handler::server::ServerHandler, model::*, service::RequestContext,
};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::future::Future;
use std::sync::Arc;

use crate::database;
use crate::mcp_types::*;

mod batch;
mod handlers;

#[derive(Clone)]
pub struct AppState {
    pub cfg: crate::config::Config,
    pub manager: database::Manager,
}

#[derive(Clone)]
pub struct DbMcpService {
    state: Arc<AppState>,
}

impl DbMcpService {
    pub fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }

    pub fn mcp_tools(&self) -> Vec<Tool> {
        use database::{query_annotations, read_only_annotations};
        vec![
            database::tool_schema::<ListConnectionsInput, ListConnectionsOutput>(
                "list_connections",
                "List configured database connections.",
                read_only_annotations(),
            ),
            database::tool_schema::<DiagnoseConnectionInput, DiagnoseConnectionOutput>(
                "diagnose_connection",
                "Run staged connection diagnostics for one or all configured databases.",
                read_only_annotations(),
            ),
            database::tool_schema::<ListInput, ListSchemasOutput>(
                "list_schemas",
                "List visible schemas for a named connection. Call list_connections first.",
                read_only_annotations(),
            ),
            database::tool_schema::<ListInput, ListTablesOutput>(
                "list_tables",
                "List tables and views in a schema for a named connection.",
                read_only_annotations(),
            ),
            database::tool_schema::<SearchColumnsInput, SearchColumnsOutput>(
                "search_columns",
                "Search visible columns by name within a schema for a named connection.",
                read_only_annotations(),
            ),
            database::tool_schema::<ListConstraintsInput, ListIndexesOutput>(
                "list_indexes",
                "List visible indexes for a schema or table on a named connection.",
                read_only_annotations(),
            ),
            database::tool_schema::<ListConstraintsInput, ListConstraintsOutput>(
                "list_constraints",
                "List visible table constraints for a schema or table on a named connection.",
                read_only_annotations(),
            ),
            database::tool_schema::<TableInput, GetTableCommentsOutput>(
                "get_table_comments",
                "Get table and column comments for a table or view on a named connection.",
                read_only_annotations(),
            ),
            database::tool_schema::<TableInput, DescribeTableOutput>(
                "describe_table",
                "Describe table or view columns on a named connection.",
                read_only_annotations(),
            ),
            database::tool_schema::<TableInput, GetTableDDLOutput>(
                "get_table_ddl",
                "Return a table or view DDL definition in a normalized read-only form for a named connection.",
                read_only_annotations(),
            ),
            database::tool_schema::<ExecuteQueryInput, ExecuteQueryOutput>(
                "execute_query",
                "Run one single-statement query on a named connection. A trailing semicolon is allowed; SQL scripts are not supported. Results are truncated to protect MCP context.",
                query_annotations(),
            ),
            database::tool_schema::<ExecuteQueriesInput, ExecuteQueriesOutput>(
                "execute_queries",
                "Run 1 to 8 read-only single-statement queries sequentially on a named connection. Each query may span multiple lines and end with a semicolon; SQL scripts are not supported. Results are returned separately and execution stops at the first failure.",
                query_annotations(),
            ),
            database::tool_schema::<ExplainQueryInput, ExplainQueryOutput>(
                "explain_query",
                "Explain a query on a named connection and return the execution plan.",
                read_only_annotations(),
            ),
        ]
    }

    pub fn mcp_prompts(&self) -> Vec<Prompt> {
        vec![
            database::prompt(
                "explore_database",
                "Suggested step-by-step workflow for exploring an unfamiliar database safely.",
            ),
            database::prompt_with_args(
                "inspect_table",
                "Suggested workflow for inspecting a table and writing a safe read-only query.",
                vec![
                    PromptArgument::new("table")
                        .with_description("table or view name to inspect")
                        .with_required(true),
                    PromptArgument::new("schema").with_description("schema name if known"),
                    PromptArgument::new("connection")
                        .with_description("connection name from list_connections")
                        .with_required(true),
                ],
            ),
        ]
    }

    pub fn mcp_resources(&self) -> Vec<Resource> {
        vec![database::to_resource(
            "db://connections",
            "connections",
            "Database Connections",
            "Configured database connections. Use one of these names as the connection argument in other tools.",
            "application/json",
        )]
    }

    pub fn mcp_resource_templates(&self) -> Vec<ResourceTemplate> {
        vec![database::to_resource_template(
            "db://table/{connection}/{schema}/{table}",
            "table_overview",
            "Table Overview",
            "Combined metadata for a table including columns, constraints, indexes, and comments.",
            "application/json",
        )]
    }

    fn parse_tool_input<T: for<'de> Deserialize<'de>>(
        args: Option<Map<String, serde_json::Value>>,
    ) -> Result<T> {
        let value = Value::Object(args.unwrap_or_default());
        Ok(serde_json::from_value(value)?)
    }

    async fn run_tool(&self, name: &str, arguments: Option<Map<String, Value>>) -> RunToolResult {
        let result = match name {
            "list_connections" => {
                self.list_connections(match Self::parse_tool_input(arguments) {
                    Ok(input) => input,
                    Err(error) => return RunToolResult::invalid_params(&error),
                })
                .await
            }
            "diagnose_connection" => {
                self.diagnose_connection(match Self::parse_tool_input(arguments) {
                    Ok(input) => input,
                    Err(error) => return RunToolResult::invalid_params(&error),
                })
                .await
            }
            "list_schemas" => {
                self.list_schemas(match Self::parse_tool_input(arguments) {
                    Ok(input) => input,
                    Err(error) => return RunToolResult::invalid_params(&error),
                })
                .await
            }
            "list_tables" => {
                self.list_tables(match Self::parse_tool_input(arguments) {
                    Ok(input) => input,
                    Err(error) => return RunToolResult::invalid_params(&error),
                })
                .await
            }
            "search_columns" => {
                self.search_columns(match Self::parse_tool_input(arguments) {
                    Ok(input) => input,
                    Err(error) => return RunToolResult::invalid_params(&error),
                })
                .await
            }
            "list_constraints" => {
                self.list_constraints(match Self::parse_tool_input(arguments) {
                    Ok(input) => input,
                    Err(error) => return RunToolResult::invalid_params(&error),
                })
                .await
            }
            "list_indexes" => {
                self.list_indexes(match Self::parse_tool_input(arguments) {
                    Ok(input) => input,
                    Err(error) => return RunToolResult::invalid_params(&error),
                })
                .await
            }
            "get_table_comments" => {
                self.get_table_comments(match Self::parse_tool_input(arguments) {
                    Ok(input) => input,
                    Err(error) => return RunToolResult::invalid_params(&error),
                })
                .await
            }
            "describe_table" => {
                self.describe_table(match Self::parse_tool_input(arguments) {
                    Ok(input) => input,
                    Err(error) => return RunToolResult::invalid_params(&error),
                })
                .await
            }
            "get_table_ddl" => {
                self.get_table_ddl(match Self::parse_tool_input(arguments) {
                    Ok(input) => input,
                    Err(error) => return RunToolResult::invalid_params(&error),
                })
                .await
            }
            "execute_query" => {
                self.execute_query(match Self::parse_tool_input(arguments) {
                    Ok(input) => input,
                    Err(error) => return RunToolResult::invalid_params(&error),
                })
                .await
            }
            "execute_queries" => {
                self.execute_queries(match Self::parse_tool_input(arguments) {
                    Ok(input) => input,
                    Err(error) => return RunToolResult::invalid_params(&error),
                })
                .await
            }
            "explain_query" => {
                self.explain_query(match Self::parse_tool_input(arguments) {
                    Ok(input) => input,
                    Err(error) => return RunToolResult::invalid_params(&error),
                })
                .await
            }
            other => {
                return RunToolResult::Protocol(ErrorData::invalid_params(
                    format!("tool {other:?} not found"),
                    None,
                ));
            }
        };
        RunToolResult::from_handler(result)
    }
}

/// Distinguishes protocol-level failures (malformed arguments, unknown tools)
/// from tool-level failures the client should display as the tool result.
#[derive(Debug)]
pub enum RunToolResult {
    Ok(CallToolResult),
    Protocol(ErrorData),
    Tool(anyhow::Error),
}

impl RunToolResult {
    fn from_handler(result: Result<CallToolResult>) -> Self {
        match result {
            Ok(result) => Self::Ok(result),
            Err(error) => Self::Tool(error),
        }
    }

    fn invalid_params(error: &anyhow::Error) -> Self {
        Self::Protocol(ErrorData::invalid_params(error.to_string(), None))
    }

    fn into_response(self) -> Result<CallToolResponse, ErrorData> {
        match self {
            Self::Ok(result) => Ok(result.into()),
            Self::Protocol(error) => Err(error),
            // Connection, database, query, and diagnostic failures are
            // tool-level errors so the client displays the useful message.
            Self::Tool(error) => Ok(database::call_tool_error(format!("{error:#}")).into()),
        }
    }
}

impl ServerHandler for DbMcpService {
    fn get_info(&self) -> ServerInfo {
        database::build_server_info(&self.state.cfg.server.name, &self.state.cfg.server.version)
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> + Send + '_ {
        std::future::ready(Ok(database::list_tools_result(self.mcp_tools())))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.mcp_tools()
            .into_iter()
            .find(|tool| tool.name.as_ref() == name)
    }

    fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListPromptsResult, ErrorData>> + Send + '_ {
        std::future::ready(Ok(database::list_prompts_result(self.mcp_prompts())))
    }

    fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<GetPromptResponse, ErrorData>> + Send + '_ {
        std::future::ready(match request.name.as_str() {
            "explore_database" => Ok(database::get_prompt_result(
                "Suggested step-by-step workflow for exploring an unfamiliar database safely.",
                vec![PromptMessage::new_text(
                    Role::User,
                    "1. Start with list_connections or db://connections.\n2. Pick a connection name and pass it to every database tool.\n3. Inspect schemas, then tables.\n4. Use describe_table, list_constraints, and list_indexes before execute_query.\n5. Prefer small, scoped queries first.",
                )],
            )
            .into()),
            "inspect_table" => {
                let args = request.arguments.unwrap_or_default();
                let table = args.get("table").and_then(Value::as_str).unwrap_or("table");
                let schema = args
                    .get("schema")
                    .and_then(Value::as_str)
                    .unwrap_or("schema");
                let connection = args
                    .get("connection")
                    .and_then(Value::as_str)
                    .unwrap_or("connection");
                Ok(database::get_prompt_result(
                    "Suggested workflow for inspecting a table and writing a safe read-only query.",
                    vec![PromptMessage::new_text(
                        Role::User,
                        format!(
                            "Inspect {schema}.{table} on {connection}: describe the table, list constraints and indexes, review comments, then draft a small query."
                        ),
                    )],
                )
                .into())
            }
            other => Err(ErrorData::invalid_params(
                format!("prompt {other:?} not found"),
                None,
            )),
        })
    }

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourcesResult, ErrorData>> + Send + '_ {
        std::future::ready(Ok(database::list_resources_result(self.mcp_resources())))
    }

    fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourceTemplatesResult, ErrorData>> + Send + '_ {
        std::future::ready(Ok(database::list_resource_templates_result(
            self.mcp_resource_templates(),
        )))
    }

    fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ReadResourceResponse, ErrorData>> + Send + '_ {
        let state = self.state.clone();
        async move {
            if request.uri == "db://connections" {
                let health = state
                    .manager
                    .health(None, false)
                    .await
                    .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
                let body = serde_json::to_string_pretty(&health)
                    .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
                return Ok(database::read_resource_result(vec![ResourceContents::text(
                    body,
                    request.uri,
                )])
                .into());
            }
            if let Some(rest) = request.uri.strip_prefix("db://table/") {
                let parts = rest.split('/').collect::<Vec<_>>();
                if parts.len() != 3 {
                    return Err(ErrorData::invalid_params(
                        "invalid table resource uri",
                        None,
                    ));
                }
                let conn = state
                    .manager
                    .connection(Some(parts[0]))
                    .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
                let schema = parts[1];
                let table = parts[2];
                let desc = conn
                    .describe_table(Some(schema), table)
                    .await
                    .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
                let constraints = conn
                    .list_constraints(Some(schema), Some(table), 100)
                    .await
                    .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
                let indexes = conn
                    .list_indexes(Some(schema), Some(table), 100)
                    .await
                    .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
                let comments = conn
                    .get_table_comments(Some(schema), table)
                    .await
                    .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
                let body = serde_json::json!({
                    "connection": parts[0],
                    "schema": schema,
                    "table": table,
                    "columns": desc,
                    "constraints": constraints,
                    "indexes": indexes,
                    "comments": comments,
                });
                return Ok(database::read_resource_result(vec![ResourceContents::text(
                    serde_json::to_string_pretty(&body).unwrap_or_default(),
                    request.uri,
                )])
                .into());
            }
            Err(ErrorData::resource_not_found("resource not found", None))
        }
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResponse, ErrorData>> + Send + '_ {
        let state = self.state.clone();
        async move {
            let name = request.name.as_ref();
            let service = Self { state };
            let result = service.run_tool(name, request.arguments).await;
            result.into_response()
        }
    }
}

pub async fn bearer_auth(
    State(state): State<Arc<AppState>>,
    request: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> axum::response::Response {
    if state.cfg.server.http_auth.mode == crate::config::HttpAuthMode::None {
        return next.run(request).await;
    }
    let expected = format!("Bearer {}", state.cfg.server.http_auth.token);
    match request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        Some(value) if constant_time_eq(value.trim().as_bytes(), expected.as_bytes()) => {
            next.run(request).await
        }
        _ => {
            let mut response =
                axum::response::Response::new(axum::body::Body::from("unauthorized"));
            *response.status_mut() = axum::http::StatusCode::UNAUTHORIZED;
            response.headers_mut().insert(
                axum::http::header::WWW_AUTHENTICATE,
                axum::http::HeaderValue::from_static("Bearer"),
            );
            response
        }
    }
}

/// Content comparison that does not short-circuit on the first differing byte,
/// avoiding token timing side channels; length differences fold into the diff.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let max_len = a.len().max(b.len());
    let mut diff = a.len() ^ b.len();
    for i in 0..max_len {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        diff |= (x ^ y) as usize;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::config::{Config, ConnectionConfig, LimitsConfig, SQLiteConfig, ServerConfig};
    use rmcp::model::ToolAnnotations;

    const READ_ONLY_TOOLS: &[&str] = &[
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
    const QUERY_TOOLS: &[&str] = &["execute_query", "execute_queries"];

    fn tool_by_name(service: &DbMcpService, name: &str) -> Tool {
        service
            .mcp_tools()
            .into_iter()
            .find(|tool| tool.name.as_ref() == name)
            .unwrap_or_else(|| panic!("tool {name} not found"))
    }

    fn annotations(tool: &Tool) -> ToolAnnotations {
        tool.annotations.clone().unwrap_or_default()
    }

    fn input_schema(tool: &Tool) -> Arc<Map<String, Value>> {
        tool.input_schema.clone()
    }

    fn output_schema(tool: &Tool) -> Arc<Map<String, Value>> {
        tool.output_schema
            .clone()
            .unwrap_or_else(|| panic!("tool {} has no output schema", tool.name))
    }

    #[test]
    fn mcp_tools_expose_thirteen_contracts() {
        let service = test_service_sync();
        let tools = service.mcp_tools();
        let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_ref()).collect();
        assert_eq!(names.len(), 13);
        for expected in READ_ONLY_TOOLS.iter().chain(QUERY_TOOLS) {
            assert!(names.contains(expected), "missing tool {expected}");
        }
        for tool in &tools {
            assert!(
                tool.description.as_deref().is_some_and(|d| !d.is_empty()),
                "{} has no description",
                tool.name
            );
            let input = input_schema(tool);
            assert_eq!(input["type"], "object", "{} input schema", tool.name);
            assert!(
                input["properties"]
                    .as_object()
                    .is_some_and(|p| !p.is_empty()),
                "{} input schema has no properties",
                tool.name
            );
            let output = output_schema(tool);
            assert_eq!(output["type"], "object", "{} output schema", tool.name);
            assert!(
                output["properties"]
                    .as_object()
                    .is_some_and(|p| !p.is_empty()),
                "{} output schema has no properties",
                tool.name
            );
        }
    }

    #[test]
    fn metadata_tools_are_read_only_annotated() {
        let service = test_service_sync();
        for name in READ_ONLY_TOOLS {
            let tool = tool_by_name(&service, name);
            let annotations = annotations(&tool);
            assert_eq!(
                annotations.read_only_hint,
                Some(true),
                "{name} should be read-only"
            );
            assert_eq!(
                annotations.idempotent_hint,
                Some(true),
                "{name} should be idempotent"
            );
            assert_eq!(
                annotations.open_world_hint,
                Some(false),
                "{name} should be closed-world"
            );
            assert_eq!(
                annotations.destructive_hint, None,
                "{name} should not declare destructive"
            );
        }
        for name in QUERY_TOOLS {
            let tool = tool_by_name(&service, name);
            let annotations = annotations(&tool);
            assert_eq!(annotations.read_only_hint, Some(false), "{name}");
            assert_eq!(annotations.destructive_hint, Some(true), "{name}");
            assert_eq!(annotations.idempotent_hint, Some(false), "{name}");
            assert_eq!(annotations.open_world_hint, Some(false), "{name}");
        }
    }

    #[test]
    fn list_indexes_accepts_table_argument() {
        let service = test_service_sync();
        let tool = tool_by_name(&service, "list_indexes");
        let input = input_schema(&tool);
        let properties = input["properties"].as_object().expect("input properties");
        assert!(
            properties.contains_key("table"),
            "list_indexes must accept table"
        );
        assert!(properties.contains_key("schema"));
        assert!(properties.contains_key("limit"));
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

    async fn test_state() -> Arc<AppState> {
        let cfg = Config {
            server: ServerConfig::default(),
            limits: LimitsConfig::default(),
            connections: vec![ConnectionConfig {
                name: "sqlite".to_string(),
                driver: "sqlite".to_string(),
                sqlite: Some(SQLiteConfig {
                    dsn: "sqlite::memory:".to_string(),
                    read_only: true,
                }),
                postgres: None,
                oracle: None,
            }],
        };
        let manager = database::Manager::new(&cfg).await.expect("manager");
        Arc::new(AppState { cfg, manager })
    }

    fn test_service_sync() -> DbMcpService {
        let cfg = Config::default();
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let manager = rt.block_on(database::Manager::new(&cfg)).expect("manager");
        DbMcpService::new(Arc::new(AppState { cfg, manager }))
    }

    #[tokio::test]
    async fn malformed_arguments_return_protocol_invalid_params() {
        let service = DbMcpService::new(test_state().await);
        let result = service.run_tool("describe_table", None).await;
        let error = match result {
            RunToolResult::Protocol(error) => error,
            other => panic!("expected protocol error, got {other:?}"),
        };
        assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
        assert!(!error.message.is_empty());
    }

    #[tokio::test]
    async fn unknown_tool_returns_protocol_invalid_params() {
        let service = DbMcpService::new(test_state().await);
        let result = service.run_tool("does_not_exist", Some(Map::new())).await;
        let error = match result {
            RunToolResult::Protocol(error) => error,
            other => panic!("expected protocol error, got {other:?}"),
        };
        assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
        assert!(error.message.contains("does_not_exist"));
    }

    #[tokio::test]
    async fn missing_connection_failure_is_tool_level_error() {
        let service = DbMcpService::new(test_state().await);
        let result = service
            .run_tool(
                "execute_query",
                Some(
                    serde_json::from_value(serde_json::json!({
                        "connection": "missing-conn",
                        "sql": "select 1",
                    }))
                    .expect("arguments"),
                ),
            )
            .await;
        let result = match result {
            RunToolResult::Tool(error) => database::call_tool_error(format!("{error:#}")),
            other => panic!("expected tool-level error, got {other:?}"),
        };
        assert_eq!(result.is_error, Some(true));
        let text = result
            .content
            .first()
            .and_then(|block| block.as_text())
            .expect("error text");
        assert!(text.text.contains("missing-conn"));
    }

    #[tokio::test]
    async fn structured_success_is_visible_json_matching_structured_content() {
        let path = {
            let suffix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock")
                .as_nanos();
            std::env::temp_dir().join(format!("dbmcp-mcp-test-{}-{suffix}.db", std::process::id()))
        };
        let writable_dsn = format!("sqlite://{}?mode=rwc", path.display());
        let pool = sqlx::SqlitePool::connect(&writable_dsn)
            .await
            .expect("create sqlite database");
        sqlx::query("create table if not exists probe (value integer)")
            .execute(&pool)
            .await
            .expect("initialize sqlite database");
        pool.close().await;

        let cfg = Config {
            server: ServerConfig::default(),
            limits: LimitsConfig::default(),
            connections: vec![ConnectionConfig {
                name: "sqlite".to_string(),
                driver: "sqlite".to_string(),
                sqlite: Some(SQLiteConfig {
                    dsn: format!("sqlite://{}?mode=ro", path.display()),
                    read_only: true,
                }),
                postgres: None,
                oracle: None,
            }],
        };
        let manager = database::Manager::new(&cfg).await.expect("manager");
        let service = DbMcpService::new(Arc::new(AppState { cfg, manager }));

        let result = service
            .run_tool(
                "execute_query",
                Some(
                    serde_json::from_value(serde_json::json!({
                        "connection": "sqlite",
                        "sql": "select 1 as value",
                    }))
                    .expect("arguments"),
                ),
            )
            .await;
        let result = match result {
            RunToolResult::Ok(result) => result,
            other => panic!("expected structured success, got {other:?}"),
        };
        let visible = result
            .content
            .first()
            .and_then(|block| block.as_text())
            .map(|text| text.text.clone())
            .expect("visible text content");
        let visible_json: Value = serde_json::from_str(&visible).expect("visible text is JSON");
        let structured = result
            .structured_content
            .expect("structured content present");
        assert_eq!(
            visible_json, structured,
            "visible text and structured content must match"
        );
        let result = structured["result"].as_object().expect("result object");
        let rows = result["rows"].as_array().expect("rows array");
        assert_eq!(rows[0]["value"].as_i64(), Some(1));
        assert_eq!(structured["connection"], "sqlite");

        std::fs::remove_file(path).expect("remove test database");
    }
}
