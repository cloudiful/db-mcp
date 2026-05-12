use anyhow::Result;
use axum::extract::State;
use rmcp::{
    ErrorData, RoleServer, handler::server::ServerHandler, model::*, service::RequestContext,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value};
use std::future::Future;
use std::sync::Arc;

use crate::database;

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
        vec![
            tool_schema::<ListConnectionsInput>(
                "list_connections",
                "List configured database connections.",
            ),
            tool_schema::<ListInput>("list_schemas", "List visible schemas for a connection."),
            tool_schema::<ListInput>("list_tables", "List tables and views in a schema."),
            tool_schema::<SearchColumnsInput>(
                "search_columns",
                "Search visible columns by name within a schema.",
            ),
            tool_schema::<ListInput>(
                "list_indexes",
                "List visible indexes for a schema or table.",
            ),
            tool_schema::<ListConstraintsInput>(
                "list_constraints",
                "List visible table constraints for a schema or table.",
            ),
            tool_schema::<TableInput>(
                "get_table_comments",
                "Get table and column comments for a table or view.",
            ),
            tool_schema::<TableInput>("describe_table", "Describe table or view columns."),
            tool_schema::<TableInput>(
                "get_table_ddl",
                "Return a table or view DDL definition in a normalized read-only form.",
            ),
            tool_schema::<ExecuteQueryInput>(
                "execute_query",
                "Run a read-only SELECT or WITH query. Results are truncated to protect MCP context.",
            ),
            tool_schema::<ExplainQueryInput>(
                "explain_query",
                "Explain a read-only SELECT or WITH query without executing data changes.",
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
                        .with_description("connection name if multiple databases are configured"),
                ],
            ),
        ]
    }

    pub fn mcp_resources(&self) -> Vec<Resource> {
        vec![database::to_resource(
            "db://connections",
            "connections",
            "Database Connections",
            "Configured database connections and the default connection for this server.",
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

    async fn list_connections(&self, input: ListConnectionsInput) -> Result<CallToolResult> {
        let refresh = input.refresh.unwrap_or(true);
        let health = self.state.manager.health(None, refresh).await?;
        let output = ListConnectionsOutput {
            default_connection: health.default_connection.clone(),
            connections: health.connections,
        };
        database::call_tool_json(
            format!(
                "Configured {} connection(s). Default is {}.",
                output.connections.len(),
                output.default_connection
            ),
            &output,
        )
    }

    async fn list_schemas(&self, input: ListInput) -> Result<CallToolResult> {
        let conn = self.state.manager.connection(input.connection.as_deref())?;
        let schemas = conn
            .list_schemas(input.filter.as_deref(), input.limit.unwrap_or(0))
            .await?;
        let output = ListSchemasOutput {
            connection: conn.info().name,
            schemas,
        };
        database::call_tool_json(
            format!("Returned {} schema(s).", output.schemas.len()),
            &output,
        )
    }

    async fn list_tables(&self, input: ListInput) -> Result<CallToolResult> {
        let conn = self.state.manager.connection(input.connection.as_deref())?;
        let schema = input.schema.as_deref();
        let tables = conn
            .list_tables(schema, input.filter.as_deref(), input.limit.unwrap_or(0))
            .await?;
        let output = ListTablesOutput {
            connection: conn.info().name,
            tables,
        };
        database::call_tool_json(
            format!("Returned {} table/view entries.", output.tables.len()),
            &output,
        )
    }

    async fn search_columns(&self, input: SearchColumnsInput) -> Result<CallToolResult> {
        let conn = self.state.manager.connection(input.connection.as_deref())?;
        let schema = input.schema.as_deref();
        let matches = conn
            .search_columns(schema, input.filter.as_deref(), input.limit.unwrap_or(0))
            .await?;
        let output = SearchColumnsOutput {
            connection: conn.info().name,
            matches,
        };
        database::call_tool_json(
            format!("Returned {} column match(es).", output.matches.len()),
            &output,
        )
    }

    async fn list_constraints(&self, input: ListConstraintsInput) -> Result<CallToolResult> {
        let conn = self.state.manager.connection(input.connection.as_deref())?;
        let schema = input.schema.as_deref();
        let constraints = conn
            .list_constraints(schema, input.table.as_deref(), input.limit.unwrap_or(0))
            .await?;
        let output = ListConstraintsOutput {
            connection: conn.info().name,
            constraints,
        };
        database::call_tool_json(
            format!("Returned {} constraint row(s).", output.constraints.len()),
            &output,
        )
    }

    async fn list_indexes(&self, input: ListConstraintsInput) -> Result<CallToolResult> {
        let conn = self.state.manager.connection(input.connection.as_deref())?;
        let schema = input.schema.as_deref();
        let indexes = conn
            .list_indexes(schema, input.table.as_deref(), input.limit.unwrap_or(0))
            .await?;
        let output = ListIndexesOutput {
            connection: conn.info().name,
            indexes,
        };
        database::call_tool_json(
            format!("Returned {} index row(s).", output.indexes.len()),
            &output,
        )
    }

    async fn get_table_comments(&self, input: TableInput) -> Result<CallToolResult> {
        let conn = self.state.manager.connection(input.connection.as_deref())?;
        let comments = conn
            .get_table_comments(input.schema.as_deref(), &input.table)
            .await?;
        database::call_tool_json(
            format!("Loaded comments for {}.{}", comments.schema, comments.table),
            &GetTableCommentsOutput {
                connection: conn.info().name,
                comments,
            },
        )
    }

    async fn describe_table(&self, input: TableInput) -> Result<CallToolResult> {
        let conn = self.state.manager.connection(input.connection.as_deref())?;
        let columns = conn
            .describe_table(input.schema.as_deref(), &input.table)
            .await?;
        let output = DescribeTableOutput {
            connection: conn.info().name,
            schema: input
                .schema
                .clone()
                .unwrap_or_else(|| conn.info().default_schema),
            table: input.table,
            columns,
        };
        database::call_tool_json(
            format!("Described {}.{}.", output.schema, output.table),
            &output,
        )
    }

    async fn get_table_ddl(&self, input: TableInput) -> Result<CallToolResult> {
        let conn = self.state.manager.connection(input.connection.as_deref())?;
        let ddl = conn
            .get_table_ddl(input.schema.as_deref(), &input.table)
            .await?;
        database::call_tool_json(
            format!(
                "Loaded {} DDL for {}.{}.",
                ddl.object_type, ddl.schema, ddl.table
            ),
            &GetTableDDLOutput {
                connection: conn.info().name,
                ddl,
            },
        )
    }

    async fn execute_query(&self, input: ExecuteQueryInput) -> Result<CallToolResult> {
        let conn = self.state.manager.connection(input.connection.as_deref())?;
        let result = conn
            .execute_query(
                &input.sql,
                input.max_rows.unwrap_or(0),
                self.state.manager.limits(),
            )
            .await?;
        let text = format!(
            "Returned {} row(s) with {} column(s) from {} in {}.",
            result.row_count,
            result.columns.len(),
            conn.info().name,
            result.execution_time
        );
        database::call_tool_json(
            text,
            &ExecuteQueryOutput {
                connection: conn.info().name,
                result,
            },
        )
    }

    async fn explain_query(&self, input: ExplainQueryInput) -> Result<CallToolResult> {
        let conn = self.state.manager.connection(input.connection.as_deref())?;
        let explain = conn
            .explain_query(&input.sql, self.state.manager.limits())
            .await?;
        database::call_tool_json(
            format!(
                "Explained query on {} with {} plan line(s).",
                conn.info().name,
                explain.plan_lines.len()
            ),
            &ExplainQueryOutput {
                connection: conn.info().name,
                explain,
            },
        )
    }
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListConnectionsInput {
    pub refresh: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListInput {
    pub connection: Option<String>,
    pub schema: Option<String>,
    pub filter: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchColumnsInput {
    pub connection: Option<String>,
    pub schema: Option<String>,
    pub filter: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListConstraintsInput {
    pub connection: Option<String>,
    pub schema: Option<String>,
    pub table: Option<String>,
    pub filter: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TableInput {
    pub connection: Option<String>,
    pub schema: Option<String>,
    pub table: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteQueryInput {
    pub connection: Option<String>,
    pub sql: String,
    pub max_rows: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExplainQueryInput {
    pub connection: Option<String>,
    pub sql: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListConnectionsOutput {
    pub default_connection: String,
    pub connections: Vec<database::ConnectionHealth>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListSchemasOutput {
    pub connection: String,
    pub schemas: Vec<database::SchemaInfo>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListTablesOutput {
    pub connection: String,
    pub tables: Vec<database::TableInfo>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchColumnsOutput {
    pub connection: String,
    pub matches: Vec<database::ColumnMatchInfo>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListConstraintsOutput {
    pub connection: String,
    pub constraints: Vec<database::ConstraintInfo>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListIndexesOutput {
    pub connection: String,
    pub indexes: Vec<database::IndexInfo>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetTableCommentsOutput {
    pub connection: String,
    pub comments: database::TableCommentInfo,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DescribeTableOutput {
    pub connection: String,
    pub schema: String,
    pub table: String,
    pub columns: Vec<database::ColumnInfo>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetTableDDLOutput {
    pub connection: String,
    pub ddl: database::TableDDLInfo,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteQueryOutput {
    pub connection: String,
    pub result: database::QueryResult,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExplainQueryOutput {
    pub connection: String,
    pub explain: database::ExplainResultData,
}

pub fn tool_schema<T: JsonSchema>(name: &'static str, description: &'static str) -> Tool {
    Tool::new(name, description, database::to_json_object::<T>())
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
    ) -> impl Future<Output = Result<GetPromptResult, ErrorData>> + Send + '_ {
        std::future::ready(match request.name.as_str() {
            "explore_database" => Ok(database::get_prompt_result(
                "Suggested step-by-step workflow for exploring an unfamiliar database safely.",
                vec![PromptMessage::new_text(
                    PromptMessageRole::User,
                    "1. Start with list_connections or db://connections.\n2. Inspect schemas, then tables.\n3. Use describe_table, list_constraints, and list_indexes before execute_query.\n4. Prefer read-only queries only.",
                )],
            )),
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
                        PromptMessageRole::User,
                        format!(
                            "Inspect {schema}.{table} on {connection}: describe the table, list constraints and indexes, review comments, then draft a small read-only SELECT."
                        ),
                    )],
                ))
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
    ) -> impl Future<Output = Result<ReadResourceResult, ErrorData>> + Send + '_ {
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
                return Ok(database::read_resource_result(vec![
                    ResourceContents::text(body, request.uri),
                ]));
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
                return Ok(database::read_resource_result(vec![
                    ResourceContents::text(
                        serde_json::to_string_pretty(&body).unwrap_or_default(),
                        request.uri,
                    ),
                ]));
            }
            Err(ErrorData::resource_not_found("resource not found", None))
        }
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResult, ErrorData>> + Send + '_ {
        async move {
            let name = request.name.as_ref();
            let result = match name {
                "list_connections" => {
                    self.list_connections(
                        Self::parse_tool_input(request.arguments)
                            .map_err(|e| ErrorData::invalid_params(e.to_string(), None))?,
                    )
                    .await
                }
                "list_schemas" => {
                    self.list_schemas(
                        Self::parse_tool_input(request.arguments)
                            .map_err(|e| ErrorData::invalid_params(e.to_string(), None))?,
                    )
                    .await
                }
                "list_tables" => {
                    self.list_tables(
                        Self::parse_tool_input(request.arguments)
                            .map_err(|e| ErrorData::invalid_params(e.to_string(), None))?,
                    )
                    .await
                }
                "search_columns" => {
                    self.search_columns(
                        Self::parse_tool_input(request.arguments)
                            .map_err(|e| ErrorData::invalid_params(e.to_string(), None))?,
                    )
                    .await
                }
                "list_constraints" => {
                    self.list_constraints(
                        Self::parse_tool_input(request.arguments)
                            .map_err(|e| ErrorData::invalid_params(e.to_string(), None))?,
                    )
                    .await
                }
                "list_indexes" => {
                    self.list_indexes(
                        Self::parse_tool_input(request.arguments)
                            .map_err(|e| ErrorData::invalid_params(e.to_string(), None))?,
                    )
                    .await
                }
                "get_table_comments" => {
                    self.get_table_comments(
                        Self::parse_tool_input(request.arguments)
                            .map_err(|e| ErrorData::invalid_params(e.to_string(), None))?,
                    )
                    .await
                }
                "describe_table" => {
                    self.describe_table(
                        Self::parse_tool_input(request.arguments)
                            .map_err(|e| ErrorData::invalid_params(e.to_string(), None))?,
                    )
                    .await
                }
                "get_table_ddl" => {
                    self.get_table_ddl(
                        Self::parse_tool_input(request.arguments)
                            .map_err(|e| ErrorData::invalid_params(e.to_string(), None))?,
                    )
                    .await
                }
                "execute_query" => {
                    self.execute_query(
                        Self::parse_tool_input(request.arguments)
                            .map_err(|e| ErrorData::invalid_params(e.to_string(), None))?,
                    )
                    .await
                }
                "explain_query" => {
                    self.explain_query(
                        Self::parse_tool_input(request.arguments)
                            .map_err(|e| ErrorData::invalid_params(e.to_string(), None))?,
                    )
                    .await
                }
                other => Err(anyhow::anyhow!("tool {other:?} not found")),
            };
            result.map_err(|e| ErrorData::invalid_params(e.to_string(), None))
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
        Some(value) if value.trim() == expected => next.run(request).await,
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
