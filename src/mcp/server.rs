use anyhow::Result;
use rmcp::{
    ErrorData, RoleServer, handler::server::ServerHandler, model::*, service::RequestContext,
};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use std::future::Future;

use crate::config::ToolProfile;
use crate::database;

use super::DbMcpService;
use super::canonical::build_canonical_tools;
use super::legacy::build_legacy_tools;

impl DbMcpService {
    /// Tools exposed for the configured profile: canonical four, the original
    /// thirteen legacy tools, or the union when both are requested.
    pub fn build_tools(&self) -> Vec<Tool> {
        match self.state.cfg.server.tool_profile {
            ToolProfile::Canonical => build_canonical_tools(),
            ToolProfile::Legacy => build_legacy_tools(),
            ToolProfile::Both => {
                let mut tools = build_canonical_tools();
                tools.extend(build_legacy_tools());
                tools
            }
        }
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

    /// Dispatch a tool call against the active profile. Canonical and legacy
    /// tool names are disjoint, so each surface is only consulted when enabled;
    /// a name outside the active profile is reported as an unknown tool.
    pub async fn dispatch_tool(
        &self,
        name: &str,
        arguments: Option<Map<String, Value>>,
    ) -> RunToolResult {
        let profile = self.state.cfg.server.tool_profile;
        if matches!(profile, ToolProfile::Canonical | ToolProfile::Both)
            && let Some(result) =
                super::canonical::dispatch_canonical(self, name, arguments.clone()).await
        {
            return result;
        }
        if matches!(profile, ToolProfile::Legacy | ToolProfile::Both)
            && let Some(result) = super::legacy::dispatch_legacy(self, name, arguments).await
        {
            return result;
        }
        RunToolResult::unknown_tool(name)
    }
}

pub(super) fn parse_tool_input<T: DeserializeOwned>(args: Option<Map<String, Value>>) -> Result<T> {
    let value = Value::Object(args.unwrap_or_default());
    Ok(serde_json::from_value(value)?)
}

/// Parse tool arguments, converting malformed input into an invalid-params
/// protocol error so dispatch can return it without an extra match.
pub(super) fn parse_or_error<T: DeserializeOwned>(
    args: Option<Map<String, Value>>,
) -> std::result::Result<T, RunToolResult> {
    parse_tool_input(args).map_err(|error| RunToolResult::invalid_params(&error))
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
    pub(super) fn from_handler(result: Result<CallToolResult>) -> Self {
        match result {
            Ok(result) => Self::Ok(result),
            Err(error) => Self::Tool(error),
        }
    }

    pub(super) fn invalid_params(error: &anyhow::Error) -> Self {
        Self::Protocol(ErrorData::invalid_params(error.to_string(), None))
    }

    pub(super) fn unknown_tool(name: &str) -> Self {
        Self::Protocol(ErrorData::invalid_params(
            format!("tool {name:?} not found"),
            None,
        ))
    }

    fn into_response(self) -> std::result::Result<CallToolResponse, ErrorData> {
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

    async fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> std::result::Result<InitializeResult, ErrorData> {
        context.peer.set_peer_info(request.clone());
        self.negotiate_initialize(&request)
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> + Send + '_ {
        std::future::ready(Ok(database::list_tools_result(self.build_tools())))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.build_tools()
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
            let service = Self { state };
            service
                .dispatch_tool(request.name.as_ref(), request.arguments)
                .await
                .into_response()
        }
    }
}
